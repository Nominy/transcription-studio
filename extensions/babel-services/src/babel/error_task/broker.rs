//! Provider calls of the error-task assistant (Gold `error-task-broker.ts`):
//!
//! - **align**: clip ASR through the selected [`InferenceBackend`] (local: Russian only; no
//!   remote fallback), timings repaired into the clip. Cancelling aborts the request.
//! - **decide**: one paid OpenRouter decisions call per input signature. A durable checkpoint
//!   is written before dispatch; a paid result is persisted before it is adapted, so a cached
//!   result is reused instead of paying again. Pending/uncertain/rejected outcomes are never
//!   silently reissued. Once dispatched, cancelling detaches the caller only: the request
//!   finishes into the cache.
//!
//! Progress is phase + elapsed time with a five-second heartbeat; neither provider streams.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::choices::ErrorLabelDraft;
use super::clef::{
    CLEF_MODEL, DecisionComparison, SCHEMA_VERSION, decision_body, parse_decision_response,
};
use super::diff_align::{AudioInterval, TimedWord};
use super::{ErrorTaskError, ErrorTaskIdentity};
use exaudio_gold::prefs::ExecutionLocation;
use studio_inference::{AlignRequest, InferenceBackend, InferenceErrorKind, ProgressSink};

/// Provider timeout of one ASR or decisions request (body read included).
pub const PROVIDER_TIMEOUT: Duration = Duration::from_millis(240_000);
const HEARTBEAT: Duration = Duration::from_secs(5);
/// Most ASR words used from one clip.
const MAX_CLIP_WORDS: usize = 1000;

/// Cooperative cancellation shared between the UI and a running assistant step.
#[derive(Debug, Clone)]
pub struct CancelToken(watch::Sender<bool>);

impl Default for CancelToken {
    fn default() -> Self {
        Self(watch::channel(false).0)
    }
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolves once cancelled.
    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        // `wait_for` only fails when the sender is gone, which `self` prevents.
        let _ = receiver.wait_for(|cancelled| *cancelled).await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressPhase {
    Validating,
    Cache,
    WaitingLocal,
    CheckingRemote,
    WaitingRemote,
    WaitingProvider,
    Persisting,
    Adapting,
}

/// Provider progress: phase and time since the request started (no token streaming).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorTaskProgress {
    pub phase: ProgressPhase,
    pub elapsed: Duration,
}

/// What a provider call cost and who served it (no secrets, no owner tokens).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    /// `local` | `remote` | `openrouter`.
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub usage: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

fn cancelled(message: &str) -> ErrorTaskError {
    ErrorTaskError::new("cancelled", message)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn safe_token(value: &str, secret: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
        && (secret.is_empty() || !value.contains(secret))
        && !value.starts_with("sk-")
}

/// A PCM16 mono clip of the target speaker, with its absolute bounds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClipWav {
    pub wav: Bytes,
    pub interval: AudioInterval,
    pub sample_rate: u32,
    pub frames: usize,
}

impl ClipWav {
    fn duration(&self) -> f64 {
        self.frames as f64 / f64::from(self.sample_rate)
    }

    fn pcm_sha256(&self) -> String {
        sha256_hex(&self.wav[44..])
    }

    /// Gold `validateErrorTaskWav`: mono, (0, 10] s, duration matching the bounds.
    fn validate(&self) -> Result<(), ErrorTaskError> {
        let duration = self.duration();
        if self.frames == 0
            || duration > 10.0
            || !self.interval.start_seconds.is_finite()
            || !self.interval.end_seconds.is_finite()
            || self.interval.start_seconds < 0.0
            || self.interval.end_seconds <= self.interval.start_seconds
            || self.interval.duration() > 10.0
            || (duration - self.interval.duration()).abs()
                > 1.0 / f64::from(self.sample_rate) + 1e-9
        {
            return Err(ErrorTaskError::new(
                "invalid-audio",
                "Supply only the target-speaker mono crop, at most ten seconds, with matching clip boundaries.",
            ));
        }
        Ok(())
    }
}

/// Runs `work`, reporting `phase` with the elapsed time on every heartbeat.
async fn with_heartbeat<T>(
    work: impl Future<Output = T>,
    started: Instant,
    phase: ProgressPhase,
    progress: &(dyn Fn(ErrorTaskProgress) + Send + Sync),
) -> T {
    tokio::pin!(work);
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
    loop {
        tokio::select! {
            output = &mut work => return output,
            _ = ticker.tick() => progress(ErrorTaskProgress {
                phase,
                elapsed: started.elapsed(),
            }),
        }
    }
}

/// Gold `clipWords`: clamp provider words into the clip, keep starts nondecreasing and
/// intervals nonempty; fail only when no usable timed word remains.
pub(crate) fn clip_words(
    tokens: impl IntoIterator<Item = TimedWord>,
    duration: f64,
) -> Result<Vec<TimedWord>, ErrorTaskError> {
    let minimum = 0.001f64.min(duration / 2.0);
    let latest_start = duration - minimum;
    let mut words = Vec::new();
    let mut previous_start = 0.0f64;
    for word in tokens.into_iter().take(MAX_CLIP_WORDS) {
        if word.text.trim().is_empty()
            || !word.start_seconds.is_finite()
            || !word.end_seconds.is_finite()
        {
            continue;
        }
        let start_seconds = latest_start.min(previous_start.max(word.start_seconds));
        let end_seconds = duration.min((start_seconds + minimum).max(word.end_seconds));
        previous_start = start_seconds;
        words.push(TimedWord {
            text: word.text,
            start_seconds,
            end_seconds,
        });
    }
    if words.is_empty() {
        return Err(ErrorTaskError::new(
            "word-timestamps-unavailable",
            "ASR returned no timed words for this clip.",
        ));
    }
    Ok(words)
}

/// The ASR backend alignment uses (the "Where to run" setting) and what to redact from
/// local error details.
#[derive(Clone)]
pub struct AlignBackend {
    pub backend: Arc<dyn InferenceBackend>,
    pub location: ExecutionLocation,
    /// Remote base URL (only namespaces the clip task id).
    pub remote_base_url: String,
    /// Secrets (API keys) that must never appear in messages.
    pub secrets: Vec<String>,
}

/// Timed words of one clip plus the receipt.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AlignedWords {
    pub words: Vec<TimedWord>,
    pub receipt: Receipt,
}

/// Clip ASR (Gold `transcribeErrorTaskClip`). Cancelling aborts the request.
pub(crate) async fn align_clip(
    provider: &AlignBackend,
    identity: &ErrorTaskIdentity,
    language: &str,
    clip: &ClipWav,
    cancel: &CancelToken,
    progress: &(dyn Fn(ErrorTaskProgress) + Send + Sync),
) -> Result<AlignedWords, ErrorTaskError> {
    let started = Instant::now();
    progress(ErrorTaskProgress {
        phase: ProgressPhase::Validating,
        elapsed: Duration::ZERO,
    });
    clip.validate()?;
    let local = provider.location == ExecutionLocation::Local;
    if local && language.split('-').next() != Some("ru") {
        return Err(ErrorTaskError::new(
            "unsupported-language",
            "Local ASR alignment supports Russian only; no remote transcription fallback is used.",
        ));
    }
    let signature = serde_json::json!([
        "error-task-asr-v1",
        if local { "local" } else { "remote" },
        if local {
            ""
        } else {
            provider.remote_base_url.as_str()
        },
        identity.review_action_id,
        identity.diff_id,
        identity.annotation_id,
        identity.track_id,
        clip.pcm_sha256(),
        clip.sample_rate,
        1,
        clip.interval.start_seconds,
        clip.interval.end_seconds,
    ]);
    let key = format!(
        "error-task-asr-v1:{}",
        sha256_hex(signature.to_string().as_bytes())
    );
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = AlignRequest {
        task_id: format!("error-task-clip:{key}:{request_id}"),
        request_id,
        lane: identity.track_id.clone(),
        language: language.to_string(),
        wav: clip.wav.clone(),
    };
    let phase = if local {
        ProgressPhase::WaitingLocal
    } else {
        ProgressPhase::WaitingRemote
    };
    progress(ErrorTaskProgress {
        phase,
        elapsed: started.elapsed(),
    });
    let call = provider.backend.align(request, ProgressSink::none());
    let outcome = tokio::select! {
        outcome = tokio::time::timeout(PROVIDER_TIMEOUT, with_heartbeat(call, started, phase, progress)) => outcome,
        () = cancel.cancelled() => return Err(cancelled("ASR alignment cancelled.")),
    };
    let response = match outcome {
        Err(_) => {
            return Err(ErrorTaskError::new(
                "provider-timeout",
                "ASR alignment timed out. No provider fallback was used.",
            ));
        }
        Ok(Err(error)) if error.kind == InferenceErrorKind::Cancelled => {
            return Err(cancelled("ASR alignment cancelled."));
        }
        Ok(Err(error)) if local => {
            let mut detail = error.message;
            for secret in &provider.secrets {
                let secret = secret.trim();
                if !secret.is_empty() {
                    detail = detail.replace(secret, "[redacted]");
                }
            }
            let detail: String = detail
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(1000)
                .collect();
            return Err(ErrorTaskError::new(
                "local-model-unavailable",
                format!(
                    "Local ASR failed: {detail}. Check the installed local model in Settings → AI; no remote transcription fallback was used."
                ),
            ));
        }
        Ok(Err(error)) => return Err(ErrorTaskError::new("remote-unavailable", error.message)),
        Ok(Ok(response)) => response,
    };
    progress(ErrorTaskProgress {
        phase: ProgressPhase::Adapting,
        elapsed: started.elapsed(),
    });
    let duration = clip.duration();
    if local
        && response.duration_seconds.is_none_or(|reported| {
            !reported.is_finite()
                || (reported - duration).abs() > 1.0 / f64::from(clip.sample_rate) + 1e-7
        })
    {
        return Err(ErrorTaskError::new(
            "invalid-provider-response",
            "Local ASR returned a duration that does not match the captured clip.",
        ));
    }
    let words = clip_words(
        response.tokens.into_iter().map(|token| TimedWord {
            text: token.text,
            start_seconds: token.start_seconds,
            end_seconds: token.end_seconds,
        }),
        duration,
    )?;
    if words.iter().any(|word| {
        provider
            .secrets
            .iter()
            .any(|secret| !secret.trim().is_empty() && word.text.contains(secret.trim()))
    }) {
        return Err(ErrorTaskError::new(
            "invalid-provider-response",
            "Remote ASR returned unsafe transcription metadata.",
        ));
    }
    Ok(AlignedWords {
        words,
        receipt: Receipt {
            provider: if local { "local" } else { "remote" }.to_string(),
            model: response.model,
            id: None,
            usage: BTreeMap::new(),
            cost: None,
        },
    })
}

/// Durable state of one paid decision attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
enum Checkpoint {
    Pending(Attempt),
    Uncertain(Attempt),
    NotDispatched(Attempt),
    #[serde(rename_all = "camelCase")]
    Rejected {
        #[serde(flatten)]
        attempt: Attempt,
        http_status: u16,
    },
    #[serde(rename_all = "camelCase")]
    Completed {
        #[serde(flatten)]
        attempt: Attempt,
        payload: Value,
        receipt: Receipt,
        unsafe_output: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attempt {
    attempt_id: String,
    /// Unix milliseconds.
    started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_request_id: Option<String>,
}

/// Durable store of paid decision attempts and results (one JSON file per input signature).
#[derive(Debug, Clone)]
pub struct DecisionStore {
    dir: PathBuf,
}

impl DecisionStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<app support>/error-task-decisions`.
    pub fn default_location() -> Self {
        Self::new(studio_services::paths::app_support_dir().join("error-task-decisions"))
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.json", key.replace(':', "-")))
    }

    fn get(&self, key: &str) -> Result<Option<Checkpoint>, ErrorTaskError> {
        match std::fs::read(self.path(key)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| unavailable()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(unavailable()),
        }
    }

    fn set(&self, key: &str, checkpoint: &Checkpoint) -> Result<(), ErrorTaskError> {
        write_atomic(
            &self.path(key),
            &serde_json::to_vec(checkpoint).map_err(|_| unavailable())?,
        )
        .map_err(|_| unavailable())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut temp, bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn unavailable() -> ErrorTaskError {
    ErrorTaskError::new(
        "cache-unavailable",
        "Error-task durable storage is unavailable. No fallback or automatic paid retry was made.",
    )
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// A decision result plus its receipt.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Decision {
    pub draft: ErrorLabelDraft,
    pub receipt: Receipt,
}

type FlightResult = Option<Result<Decision, ErrorTaskError>>;

/// One running decision: shared by every caller with the same input signature.
struct Flight {
    result: watch::Receiver<FlightResult>,
    phase: watch::Receiver<ProgressPhase>,
    started: Instant,
    /// Callers still waiting; dispatch requires at least one.
    subscribers: Arc<AtomicUsize>,
}

static FLIGHTS: LazyLock<Mutex<HashMap<String, Flight>>> = LazyLock::new(Default::default);

/// Releases a flight subscription when the caller stops waiting.
struct Subscription(Arc<AtomicUsize>);

impl Drop for Subscription {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Everything a paid decision needs.
#[derive(Debug, Clone)]
pub(crate) struct DecisionInput {
    pub identity: ErrorTaskIdentity,
    pub comparison: DecisionComparison,
    pub clip: ClipWav,
    pub api_key: String,
    pub store: DecisionStore,
    pub endpoint: String,
}

/// Initial fill decision (Gold broker `draft`). See the module docs for paid semantics.
pub(crate) async fn decide(
    input: DecisionInput,
    cancel: &CancelToken,
    progress: &(dyn Fn(ErrorTaskProgress) + Send + Sync),
) -> Result<Decision, ErrorTaskError> {
    progress(ErrorTaskProgress {
        phase: ProgressPhase::Validating,
        elapsed: Duration::ZERO,
    });
    input.clip.validate()?;
    if !input.comparison.style_guide.is_valid() {
        return Err(ErrorTaskError::new(
            "style-guide-required",
            "Initial fill requires the current project’s complete style guide.",
        ));
    }
    let schema = decision_body(&input.comparison, "")?;
    let signature = serde_json::json!([
        SCHEMA_VERSION,
        schema,
        CLEF_MODEL,
        input.identity.review_action_id,
        input.identity.diff_id,
        input.identity.annotation_id,
        input.identity.track_id,
        input.clip.pcm_sha256(),
        input.clip.sample_rate,
        1,
        input.clip.interval.start_seconds,
        input.clip.interval.end_seconds,
    ]);
    let key = format!(
        "{SCHEMA_VERSION}:{}",
        sha256_hex(signature.to_string().as_bytes())
    );
    if cancel.is_cancelled() {
        return Err(cancelled("Cancelled."));
    }
    let (mut result, mut phase, started, subscription) = {
        let mut flights = FLIGHTS.lock().expect("flights lock");
        let mut execution = None;
        let flight = flights.entry(key.clone()).or_insert_with(|| {
            let (result_tx, result_rx) = watch::channel(None);
            let (phase_tx, phase_rx) = watch::channel(ProgressPhase::Cache);
            let subscribers = Arc::new(AtomicUsize::new(0));
            execution = Some((
                execute(input, key.clone(), phase_tx, Arc::clone(&subscribers)),
                result_tx,
            ));
            Flight {
                result: result_rx,
                phase: phase_rx,
                started: Instant::now(),
                subscribers,
            }
        });
        // Subscribe before the flight starts, so a new flight is always admitted.
        flight.subscribers.fetch_add(1, Ordering::SeqCst);
        let subscription = (
            flight.result.clone(),
            flight.phase.clone(),
            flight.started,
            Subscription(Arc::clone(&flight.subscribers)),
        );
        if let Some((execution, result_tx)) = execution {
            let flight_key = key.clone();
            tokio::spawn(async move {
                let outcome = execution.await;
                result_tx.send_replace(Some(outcome));
                FLIGHTS.lock().expect("flights lock").remove(&flight_key);
            });
        }
        subscription
    };
    let report = |phase: ProgressPhase| {
        progress(ErrorTaskProgress {
            phase,
            elapsed: started.elapsed(),
        });
    };
    report(*phase.borrow_and_update());
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
    let outcome = loop {
        if let Some(outcome) = result.borrow_and_update().clone() {
            break outcome;
        }
        tokio::select! {
            changed = result.changed() => if changed.is_err() {
                break Err(unavailable());
            },
            changed = phase.changed() => if changed.is_ok() {
                report(*phase.borrow_and_update());
            },
            _ = ticker.tick() => report(*phase.borrow()),
            () = cancel.cancelled() => {
                drop(subscription);
                return Err(cancelled(
                    "Request cancelled; a dispatched paid result will be retained but not applied.",
                ));
            }
        }
    };
    drop(subscription);
    if cancel.is_cancelled() {
        return Err(cancelled("Cancelled. Nothing was applied."));
    }
    outcome
}

/// Runs inside the spawned flight; the paid request survives caller cancellation.
async fn execute(
    input: DecisionInput,
    key: String,
    phase: watch::Sender<ProgressPhase>,
    subscribers: Arc<AtomicUsize>,
) -> Result<Decision, ErrorTaskError> {
    let admitted = || subscribers.load(Ordering::SeqCst) > 0;
    phase.send_replace(ProgressPhase::Cache);
    let store = &input.store;
    let mut record = store.get(&key)?;
    match &record {
        Some(Checkpoint::Rejected { http_status, .. }) => {
            return Err(ErrorTaskError::new(
                "provider-rejected",
                format!(
                    "The previous provider request was rejected (HTTP {http_status}). No automatic paid retry is permitted."
                ),
            ));
        }
        Some(Checkpoint::Pending(_) | Checkpoint::Uncertain(_)) => {
            return Err(ErrorTaskError::new(
                "outcome-uncertain",
                "A previous paid request has no durable result. Its outcome and charge are uncertain. Reconcile it before any new paid attempt.",
            ));
        }
        _ => {}
    }
    let secret = input.api_key.trim().to_string();
    if !matches!(record, Some(Checkpoint::Completed { .. })) {
        if !admitted() {
            return Err(cancelled("Cancelled."));
        }
        if secret.is_empty() {
            return Err(ErrorTaskError::new(
                "missing-key",
                "Add a Gold OpenRouter API key in Settings → AI first.",
            ));
        }
        if secret.len() > 1024 || secret.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(ErrorTaskError::new(
                "invalid-key",
                "Your OpenRouter API key looks invalid. Check it in Settings → AI.",
            ));
        }
        let body = serde_json::to_vec(&decision_body(
            &input.comparison,
            &BASE64.encode(&input.clip.wav),
        )?)
        .map_err(|_| unavailable())?;
        let mut attempt = Attempt {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            started_at: now_millis(),
            provider_request_id: None,
        };
        store.set(&key, &Checkpoint::Pending(attempt.clone()))?;
        if !admitted() {
            store.set(&key, &Checkpoint::NotDispatched(attempt))?;
            return Err(cancelled("Cancelled."));
        }
        phase.send_replace(ProgressPhase::WaitingProvider);
        let uncertain = |message: &str| ErrorTaskError::new("outcome-uncertain", message);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(PROVIDER_TIMEOUT)
            .build()
            .map_err(|_| unavailable())?;
        let response = client
            .post(&input.endpoint)
            .bearer_auth(&secret)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await;
        let response = match response {
            Ok(response) if !response.status().is_redirection() => response,
            _ => {
                store.set(&key, &Checkpoint::Uncertain(attempt))?;
                return Err(uncertain(
                    "Couldn’t reach the AI service. You may still be charged. Try again if needed.",
                ));
            }
        };
        let status = response.status();
        if !status.is_success() {
            // 5xx can follow a paid provider call: never erase the uncertainty.
            let server = status.is_server_error();
            store.set(
                &key,
                &if server {
                    Checkpoint::Uncertain(attempt)
                } else {
                    Checkpoint::Rejected {
                        attempt,
                        http_status: status.as_u16(),
                    }
                },
            )?;
            return Err(ErrorTaskError::new(
                if server {
                    "outcome-uncertain"
                } else {
                    "provider-rejected"
                },
                format!(
                    "OpenRouter rejected the request (HTTP {}). No fallback or automatic paid retry was made.",
                    status.as_u16()
                ),
            ));
        }
        if let Some(id) = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .filter(|id| safe_token(id, &secret))
        {
            attempt.provider_request_id = Some(id.to_string());
            store.set(&key, &Checkpoint::Pending(attempt.clone()))?;
        }
        let payload: Value = match response
            .bytes()
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(payload) => payload,
            None => {
                store.set(&key, &Checkpoint::Uncertain(attempt))?;
                return Err(uncertain(
                    "The AI service sent an unreadable reply. You may still be charged. Try again if needed.",
                ));
            }
        };
        phase.send_replace(ProgressPhase::Persisting);
        let unsafe_output = payload.to_string().contains(&secret);
        let mut receipt = provider_receipt(&payload, &secret);
        if receipt.id.is_none() {
            receipt.id = attempt.provider_request_id.clone();
        }
        let completed = Checkpoint::Completed {
            attempt,
            payload: if unsafe_output { Value::Null } else { payload },
            receipt,
            unsafe_output,
        };
        // The paid native response is stored BEFORE adaptation.
        store.set(&key, &completed)?;
        record = Some(completed);
    }
    phase.send_replace(ProgressPhase::Adapting);
    let Some(Checkpoint::Completed {
        payload,
        receipt,
        unsafe_output,
        ..
    }) = record
    else {
        return Err(ErrorTaskError::new(
            "cache-unavailable",
            "No durable provider result is available.",
        ));
    };
    if unsafe_output {
        return Err(ErrorTaskError::new(
            "invalid-provider-response",
            "The AI reply was unsafe and was discarded.",
        ));
    }
    Ok(Decision {
        draft: parse_decision_response(&payload)?,
        receipt,
    })
}

/// Gold `providerReceipt`: model/id/usage/cost, sanitized.
fn provider_receipt(payload: &Value, secret: &str) -> Receipt {
    let mut receipt = Receipt {
        provider: "openrouter".to_string(),
        model: CLEF_MODEL.to_string(),
        id: None,
        usage: BTreeMap::new(),
        cost: None,
    };
    if let Some(model) = payload.get("model").and_then(Value::as_str)
        && (model == CLEF_MODEL || model.starts_with(&format!("{CLEF_MODEL}-")))
        && model.len() <= 256
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '-'))
        && !model.contains(secret)
    {
        receipt.model = model.to_string();
    }
    if let Some(id) = payload.get("id").and_then(Value::as_str)
        && safe_token(id, secret)
    {
        receipt.id = Some(id.to_string());
    }
    if let Some(usage) = payload.get("usage").and_then(Value::as_object) {
        for key in [
            "input_tokens",
            "output_tokens",
            "prompt_tokens",
            "completion_tokens",
            "total_tokens",
            "cost",
            "total_cost",
        ] {
            if let Some(number) = usage.get(key).and_then(Value::as_f64)
                && number.is_finite()
                && number >= 0.0
            {
                receipt.usage.insert(key.to_string(), number);
            }
        }
        receipt.cost = receipt
            .usage
            .get("cost")
            .or_else(|| receipt.usage.get("total_cost"))
            .copied();
    }
    if receipt.cost.is_none()
        && let Some(cost) = payload.get("cost").and_then(Value::as_f64)
        && cost.is_finite()
        && cost >= 0.0
    {
        receipt.cost = Some(cost);
    }
    receipt
}
