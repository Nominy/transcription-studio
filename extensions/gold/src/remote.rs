//! Remote coordinator client (Gold `l0-client.ts`, `l0-timing-client.ts`, `inference-release.ts`,
//! `audio-enhancement-swarm.ts`, `background/error-task-asr.ts`, `background/l0-timing-token.ts`).
//!
//! Every operation first negotiates `/v1/e2e/capabilities` for the pinned model release, so no
//! audio is sent to a server that would time it with another model.

use std::{
    collections::BTreeMap,
    convert::Infallible,
    fmt, fs,
    io::{self, Error as IoError, ErrorKind},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use reqwest::{
    Body, Client, RequestBuilder, Response, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
    multipart::{Form, Part},
    redirect,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use url::Url;

use crate::prefs::AiSettings;
use studio_inference::{
    AlignRequest, AlignResponse, BackendStatus, DraftResponse, DraftRow, EnhancedLane,
    InferenceBackend, InferenceError, InferenceErrorKind, InferenceProgress, InferenceResult,
    LaneAudio, OperationAvailability, PreservedRow, ProgressSink, TimingResponse,
};
use studio_services::paths::app_support_dir;

/// Model release of the bundled Gold build (`model-release.json` `id`), sent as `X-Babel-E2E-Release`.
pub const GOLD_E2E_RELEASE: &str = "local-lfm3-llm-text-2026-10-31-r1";
/// Gold's legacy coordinator protocol release (`remoteReleaseId`), sent as `X-Babel-Inference-Release`
/// on enhancement requests.
pub const GOLD_REMOTE_RELEASE: &str = "c-denoise-v3-2026-10-03-r2";
const SHARED_E2E_PROTOCOL: &str = "babel-shared-e2e-webgpu-v1";

/// Enhancement model descriptor (Gold `getAudioEnhancementModel()`); compatibility is by `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhancementModel {
    pub id: &'static str,
    pub sha256: &'static str,
    pub source_graph_sha256: &'static str,
}

pub const GOLD_ENHANCEMENT_MODEL: EnhancementModel = EnhancementModel {
    id: "zipenhancer-webgpu-2026-10-09-r1",
    sha256: "e9d5433559cc2a82b2158045366300756716b7668249830c95c926da96acaec4",
    source_graph_sha256: "2f18c8f7ff10a2702d6243ce1230db9e73e6804dd6cd7b20d8e191ee06924016",
};

const CAPABILITIES_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_ENHANCE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const ENHANCE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const ENHANCE_STATUS_TIMEOUT: Duration = Duration::from_secs(15);
const ENHANCEMENT_MAX_TRACK_BYTES: u64 = 240 * 1024 * 1024;
const ENHANCEMENT_MAX_REQUEST_BYTES: u64 = 500 * 1024 * 1024;
const ENHANCEMENT_MAX_SECONDS: u64 = 4 * 60 * 60;
const ENHANCEMENT_CONTROL_BYTES: u64 = 64 * 1024;
const ENHANCEMENT_CHUNK_BYTES: u64 = 512 * 1024;

const NO_LLM_DRAFTING: &str =
    "The remote server can't draft right now. Draft on this computer or try again later.";
const DENOISE_UNAVAILABLE: &str = "The remote server can't denoise right now. Try again later.";

/// Remote server address and options; build from [`AiSettings`] with [`RemoteConfig::from_settings`].
#[derive(Clone)]
pub struct RemoteConfig {
    /// Coordinator base URL (`https://`, or `http://` on loopback).
    pub base_url: String,
    /// Optional `X-Babel-Remote-Key` (secret).
    pub access_key: Option<String>,
    /// Model release requested from the coordinator.
    pub release: String,
    pub queue_poll_interval: Duration,
    pub enhance_poll_interval: Duration,
}

impl RemoteConfig {
    pub fn new(base_url: impl Into<String>, access_key: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            access_key: access_key.filter(|key| !key.trim().is_empty()),
            release: GOLD_E2E_RELEASE.to_string(),
            queue_poll_interval: DEFAULT_QUEUE_POLL_INTERVAL,
            enhance_poll_interval: DEFAULT_ENHANCE_POLL_INTERVAL,
        }
    }

    pub fn from_settings(settings: &AiSettings) -> Self {
        Self::new(
            settings.remote_base_url.clone(),
            Some(settings.remote_access_key.trim().to_string()),
        )
    }
}

impl fmt::Debug for RemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteConfig")
            .field("base_url", &self.base_url)
            .field(
                "access_key",
                &self.access_key.as_ref().map(|_| "[redacted]"),
            )
            .field("release", &self.release)
            .finish_non_exhaustive()
    }
}

/// Per-task timing owner tokens (Gold `l0-timing-token.ts`): one random 32-byte base64url
/// bearer token per task id, persisted so the coordinator's cached timing/draft text stays
/// reachable across restarts. Stored in its own JSON file under Studio's app support dir.
#[derive(Debug)]
pub struct TimingTokenStore {
    path: PathBuf,
    lock: Mutex<()>,
}

const TIMING_TOKENS_FILE_NAME: &str = "inference-timing-tokens.json";

impl TimingTokenStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn load_default() -> Self {
        Self::new(app_support_dir().join(TIMING_TOKENS_FILE_NAME))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The task's persisted token, created on first use.
    pub async fn token(&self, task_id: &str) -> io::Result<String> {
        let _guard = self.lock.lock().await;
        let mut tokens: BTreeMap<String, String> = match fs::read_to_string(&self.path) {
            Ok(source) => serde_json::from_str(&source).map_err(|source| {
                IoError::new(
                    ErrorKind::InvalidData,
                    format!("failed to decode timing tokens: {source}"),
                )
            })?,
            Err(error) if error.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error),
        };
        if let Some(token) = tokens.get(task_id).filter(|token| is_access_token(token)) {
            return Ok(token.clone());
        }
        let token = random_token();
        tokens.insert(task_id.to_string(), token.clone());
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut file, &tokens).map_err(IoError::other)?;
        file.persist(&self.path).map_err(|error| error.error)?;
        Ok(token)
    }
}

fn is_access_token(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

/// Server-owned pipeline snapshot for one operation (Gold `RemoteInferenceCapability`).
#[derive(Debug, Clone)]
struct Capability {
    base_url: String,
    release: String,
    drafting: bool,
}

/// One Original lane prepared for upload (Gold `PreparedL0Track`).
struct PreparedTrack {
    lane: String,
    field_name: &'static str,
    wav: Bytes,
}

pub struct RemoteBackend {
    client: Client,
    config: RemoteConfig,
    tokens: Arc<TimingTokenStore>,
    /// Gold `getEnhancementClientId()`: per instance, never persisted or used as authorization.
    enhancement_client_id: String,
}

impl RemoteBackend {
    pub fn new(config: RemoteConfig, tokens: Arc<TimingTokenStore>) -> Self {
        let client = Client::builder()
            .redirect(redirect::Policy::none())
            .build()
            .expect("reqwest client with default TLS always builds");
        Self {
            client,
            config,
            tokens,
            enhancement_client_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn config(&self) -> &RemoteConfig {
        &self.config
    }

    /// Gold `getRemoteInferenceBaseUrl`.
    fn base_url(&self) -> InferenceResult<String> {
        let value = self.config.base_url.trim();
        if value.is_empty() {
            return Err(InferenceError::other(
                "Add a remote server address in Settings › AI.",
            ));
        }
        let invalid = || {
            InferenceError::other(
                "The remote server address isn't valid. Check it in Settings › AI.",
            )
        };
        let url = Url::parse(value).map_err(|_| invalid())?;
        let host = url.host_str().unwrap_or_default();
        if !matches!(url.scheme(), "http" | "https")
            || host.is_empty()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        if url.scheme() != "https" && !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
            return Err(InferenceError::other(
                "The remote server address must start with https://.",
            ));
        }
        Ok(value.trim_end_matches('/').to_string())
    }

    fn with_remote_key(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.config.access_key {
            Some(key) => request.header("X-Babel-Remote-Key", key),
            None => request,
        }
    }

    /// Release + optional key + owner bearer token (Gold `remoteInferenceHeaders` + `timingAuthorization`).
    fn authorized(
        &self,
        request: RequestBuilder,
        capability: &Capability,
        token: &str,
    ) -> RequestBuilder {
        self.with_remote_key(request)
            .header("X-Babel-E2E-Release", &capability.release)
            .bearer_auth(token)
            .header("Accept", "application/json")
    }

    async fn timing_token(&self, task_id: &str) -> InferenceResult<String> {
        self.tokens.token(task_id).await.map_err(|error| {
            InferenceError::other(format!(
                "Couldn't start word alignment. Try again. (timing credential: {error})"
            ))
        })
    }

    /// Gold `assertRemoteInferenceCompatible`.
    async fn capability(&self) -> InferenceResult<Capability> {
        let base_url = self.base_url()?;
        let response = self
            .with_remote_key(self.client.get(format!("{base_url}/v1/e2e/capabilities")))
            .header("X-Babel-E2E-Release", &self.config.release)
            .header("Accept", "application/json")
            .timeout(CAPABILITIES_TIMEOUT)
            .send()
            .await
            .map_err(|_| {
                InferenceError::other(
                    "The remote server isn't reachable. Check its address in Settings › AI.",
                )
            })?;
        let status = response.status();
        if status == StatusCode::UPGRADE_REQUIRED {
            return Err(InferenceError::new(
                InferenceErrorKind::ReleaseMismatch,
                format!(
                    "The remote server doesn't have this version's model. Use this computer until the server is updated. (release {})",
                    self.config.release
                ),
            ));
        }
        if status == StatusCode::SERVICE_UNAVAILABLE {
            return Err(InferenceError::new(
                InferenceErrorKind::NoCapacity,
                "The remote server is busy right now. Try again later or use this computer.",
            ));
        }
        if !status.is_success() {
            return Err(InferenceError::other(format!(
                "The remote server isn't responding properly. Check its address in Settings › AI. (HTTP {})",
                status.as_u16()
            )));
        }
        let report: Value = response.json().await.map_err(|_| {
            InferenceError::other(
                "The remote server can't draft. Check its address in Settings › AI.",
            )
        })?;
        let not_ready = || InferenceError::other("The remote server isn't ready. Try again later.");
        let report = report.as_object().ok_or_else(not_ready)?;
        let operations = report.get("operations");
        let release = report.get("release").and_then(Value::as_str);
        let ready = report.get("ready") == Some(&Value::Bool(true))
            && report.get("protocol").and_then(Value::as_str) == Some(SHARED_E2E_PROTOCOL)
            && release.is_some_and(is_release_id)
            && report.get("provider").and_then(Value::as_str) == Some("webgpu")
            && report.get("audio").and_then(Value::as_str) == Some("original")
            && report.get("neuralCpuFallback") == Some(&Value::Bool(false))
            && report.get("llm") == Some(&Value::Bool(false))
            && operations.is_none_or(|operations| {
                operations.pointer("/e2e-transcribe/ready") == Some(&Value::Bool(true))
            });
        let Some(release) = release.filter(|_| ready) else {
            return Err(not_ready());
        };
        if release != self.config.release {
            return Err(InferenceError::new(
                InferenceErrorKind::ReleaseMismatch,
                format!(
                    "The remote server runs a different model release. Use this computer until the server is updated. (server {release}, expected {})",
                    self.config.release
                ),
            ));
        }
        let drafting = operations.is_some_and(|operations| {
            operations.pointer("/e2e-draft/ready") == Some(&Value::Bool(true))
                && operations.pointer("/e2e-draft/llm") == Some(&Value::Bool(true))
        });
        Ok(Capability {
            base_url,
            release: release.to_string(),
            drafting,
        })
    }

    /// Gold `lookupL0Timing`: the coordinator's cached timing for this owner, if any.
    async fn lookup_timing(
        &self,
        capability: &Capability,
        task_id: &str,
        token: &str,
    ) -> InferenceResult<Option<TimingResponse>> {
        let request = self
            .client
            .post(format!("{}/v1/e2e/timing/lookup", capability.base_url));
        let response = self
            .authorized(request, capability, token)
            .json(&json!({ "taskId": task_id }))
            .send()
            .await
            .map_err(|error| unreachable_error("Couldn't get word alignment.", &error))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let (status, payload) = read_payload(response).await?;
        if !status.is_success() {
            return Err(http_error(
                "Couldn't get word alignment. Try again.",
                status,
                &payload,
            ));
        }
        let timing = parse_timing_response(payload, task_id)?;
        assert_model_result(&timing.models, capability, false)?;
        Ok(Some(timing))
    }

    /// Gold `generateL0Timing`: multipart Original WAVs with queue polling while the POST runs.
    async fn transcribe(
        &self,
        capability: &Capability,
        task_id: &str,
        token: &str,
        tracks: &[PreparedTrack],
        progress: &ProgressSink,
    ) -> InferenceResult<TimingResponse> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let form = audio_form(
            json!({ "taskId": task_id, "tracks": track_declarations(tracks) }),
            tracks,
        );
        let request = self
            .client
            .post(format!("{}/v1/e2e/transcribe", capability.base_url));
        let post = self
            .authorized(request, capability, token)
            .header("X-Babel-Local-Engine", "1")
            .header("X-Babel-Request-Id", &request_id)
            .multipart(form)
            .send();
        let response = tokio::select! {
            biased;
            response = post => response,
            never = self.poll_e2e_queue(capability, &request_id, token, progress) => match never {},
        }
        .map_err(|error| unreachable_error("Drafting failed. Try again.", &error))?;
        let (status, payload) = read_payload(response).await?;
        if !status.is_success() {
            return Err(http_error("Drafting failed. Try again.", status, &payload));
        }
        if payload.get("accessToken").and_then(Value::as_str) != Some(token) {
            return Err(InferenceError::other(
                "The remote server returned an unexpected alignment result.",
            ));
        }
        let timing = parse_timing_response(payload, task_id)?;
        assert_model_result(&timing.models, capability, false)?;
        Ok(timing)
    }

    /// Gold `pollQueueStatus`: best-effort, runs until the caller drops it.
    async fn poll_e2e_queue(
        &self,
        capability: &Capability,
        request_id: &str,
        token: &str,
        progress: &ProgressSink,
    ) -> Infallible {
        let endpoint = format!("{}/v1/e2e/queue/{request_id}", capability.base_url);
        loop {
            let request = self.authorized(self.client.get(&endpoint), capability, token);
            if let Ok(response) = request.send().await {
                if response.status() == StatusCode::NOT_FOUND {
                    progress.emit(stage("preparing", 0, None, None));
                } else if response.status().is_success()
                    && let Ok(payload) = response.json::<Value>().await
                {
                    if let Some(stage) = payload.get("progress").and_then(parse_inference_progress)
                    {
                        progress.emit(stage);
                    }
                    if let Some(queued) = parse_queue_status(&payload, request_id) {
                        progress.emit(queued);
                    }
                }
            }
            tokio::time::sleep(self.config.queue_poll_interval).await;
        }
    }

    /// Gold `requestRemoteDraft`: JSON first; 428 `original-audio-required` re-sends the Original
    /// audio once as multipart.
    async fn request_draft(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        preserve_rows: Option<&[PreservedRow]>,
    ) -> InferenceResult<DraftResponse> {
        let capability = self.capability().await?;
        if !capability.drafting {
            return Err(InferenceError::new(
                InferenceErrorKind::NoCapacity,
                NO_LLM_DRAFTING,
            ));
        }
        let token = self.timing_token(task_id).await?;
        let endpoint = format!("{}/v1/e2e/draft", capability.base_url);
        let mut body = json!({ "taskId": task_id, "text": "llm" });
        if let Some(rows) = preserve_rows {
            body["options"] = json!({ "preserveRows": rows });
        }
        let send = |request: RequestBuilder| async move {
            request.send().await.map_err(|error| {
                InferenceError::other(format!(
                    "Couldn't reach the remote server. Check your connection and its address in Settings › AI. ({})",
                    error_chain(&error)
                ))
            })
        };
        let request = self.authorized(self.client.post(&endpoint), &capability, &token);
        let (mut status, mut payload) = read_payload(send(request.json(&body)).await?).await?;
        if status == StatusCode::PRECONDITION_REQUIRED
            && error_code(&payload) == Some("original-audio-required")
        {
            let tracks = prepare_tracks(lanes)?;
            body["tracks"] = track_declarations(&tracks);
            let request = self.authorized(self.client.post(&endpoint), &capability, &token);
            (status, payload) =
                read_payload(send(request.multipart(audio_form(body, &tracks))).await?).await?;
        }
        if !status.is_success() {
            return Err(http_error("Drafting failed. Try again.", status, &payload));
        }
        let draft = parse_draft_response(payload)?;
        assert_model_result(&draft.models, &capability, true)?;
        Ok(draft)
    }

    /// Gold `enhanceAudioThroughSwarm` after its input checks.
    async fn enhance_through_swarm(
        &self,
        base_url: &str,
        task_id: &str,
        payload: Value,
        sources: &[EnhancementSource],
        progress: &ProgressSink,
    ) -> InferenceResult<Vec<EnhancedLane>> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let owner = random_token();
        let mut form = Form::new().text("payload", payload.to_string());
        for (index, source) in sources.iter().enumerate() {
            form = form.part(
                source.field_name,
                wav_part(source.wav.clone(), format!("original-{}.wav", index + 1)),
            );
        }
        progress.emit(stage("uploading", 0, None, None));
        let post = self
            .client
            .post(format!("{base_url}/v1/enhance"))
            .header("X-Babel-Inference-Release", GOLD_REMOTE_RELEASE)
            .bearer_auth(&owner)
            .header("X-Babel-Local-Engine", "1")
            .header("X-Babel-Request-Id", &request_id)
            .multipart(form)
            .send();
        let response = tokio::select! {
            biased;
            response = post => response.map_err(|error| {
                InferenceError::other(format!("{DENOISE_UNAVAILABLE} ({})", error_chain(&error)))
            })?,
            failure = self.poll_enhancement(base_url, &request_id, &owner, sources, progress) => return Err(failure),
        };
        let status = response.status();
        if !status.is_success() {
            let error = read_bounded(response, ENHANCEMENT_CONTROL_BYTES)
                .await
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            let detail = error
                .as_ref()
                .and_then(|error| {
                    error
                        .get("detail")
                        .and_then(Value::as_str)
                        .or_else(|| error.get("error").and_then(Value::as_str))
                })
                .map(|detail| detail.chars().take(500).collect::<String>())
                .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
            let kind = if status == StatusCode::SERVICE_UNAVAILABLE {
                InferenceErrorKind::NoCapacity
            } else {
                InferenceErrorKind::Other
            };
            return Err(InferenceError::new(
                kind,
                format!("{DENOISE_UNAVAILABLE} ({detail})"),
            ));
        }
        progress.emit(stage("transferring", 0, None, None));
        receive_enhanced_tracks(response, task_id, sources).await
    }

    /// Enhancement queue polling; any authorization/protocol failure aborts the request.
    async fn poll_enhancement(
        &self,
        base_url: &str,
        request_id: &str,
        owner: &str,
        sources: &[EnhancementSource],
        progress: &ProgressSink,
    ) -> InferenceError {
        let endpoint = format!("{base_url}/v1/queue/{request_id}");
        let mut previous: Option<WorkerProgress> = None;
        loop {
            tokio::time::sleep(self.config.enhance_poll_interval).await;
            let response = match self
                .client
                .get(&endpoint)
                .header("X-Babel-Inference-Release", GOLD_REMOTE_RELEASE)
                .bearer_auth(owner)
                .timeout(ENHANCE_STATUS_TIMEOUT)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    return InferenceError::other(format!(
                        "{DENOISE_UNAVAILABLE} (status: {})",
                        error_chain(&error)
                    ));
                }
            };
            if response.status() == StatusCode::NOT_FOUND {
                continue;
            }
            if !response.status().is_success() {
                return InferenceError::other(format!(
                    "{DENOISE_UNAVAILABLE} (status: HTTP {})",
                    response.status().as_u16()
                ));
            }
            let status = match read_bounded(response, ENHANCEMENT_CONTROL_BYTES)
                .await
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            {
                Some(status)
                    if status.get("requestId").and_then(Value::as_str) == Some(request_id)
                        && matches!(
                            status.get("status").and_then(Value::as_str),
                            Some("queued" | "running" | "completed")
                        ) =>
                {
                    status
                }
                _ => {
                    return InferenceError::other(
                        "The remote server returned an invalid queue status.",
                    );
                }
            };
            match status.get("progress") {
                Some(value) => match parse_worker_progress(value, sources, previous.as_ref()) {
                    Ok(next) => {
                        progress.emit(stage(
                            next.phase,
                            next.completed_chunks,
                            Some(next.total_chunks),
                            Some(sources[next.track_index].track_id.clone()),
                        ));
                        previous = Some(next);
                    }
                    Err(error) => return error,
                },
                None if previous.is_none() => {
                    progress.emit(stage("queued", 0, None, None));
                }
                None => {}
            }
        }
    }
}

#[async_trait]
impl InferenceBackend for RemoteBackend {
    async fn status(&self) -> BackendStatus {
        match self.capability().await {
            Ok(capability) => BackendStatus::Ready {
                release: capability.release,
                operations: OperationAvailability {
                    timing: true,
                    draft: capability.drafting,
                    enhance: true,
                    align: true,
                },
            },
            Err(error) => BackendStatus::Unavailable {
                kind: error.kind,
                message: error.message,
            },
        }
    }

    /// Timing lookup first (the coordinator caches timings per owner), then transcription.
    async fn timing(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        progress: ProgressSink,
    ) -> InferenceResult<TimingResponse> {
        let capability = self.capability().await?;
        let tracks = prepare_tracks(lanes)?;
        let token = self.timing_token(task_id).await?;
        if let Some(timing) = self.lookup_timing(&capability, task_id, &token).await? {
            return Ok(timing);
        }
        self.transcribe(&capability, task_id, &token, &tracks, &progress)
            .await
    }

    async fn draft(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        timing: &TimingResponse,
        preserve_rows: Option<Vec<PreservedRow>>,
        progress: ProgressSink,
    ) -> InferenceResult<DraftResponse> {
        if timing.task_id != task_id {
            return Err(InferenceError::other(
                "The word alignment belongs to a different task. Align words again.",
            ));
        }
        progress.emit(stage("drafting", 0, None, None));
        let draft = self
            .request_draft(task_id, lanes, preserve_rows.as_deref())
            .await?;
        if let Some(rows) = &preserve_rows {
            let missing = rows.iter().any(|row| {
                !draft.rows.iter().any(|candidate| {
                    candidate.id == row.row_id && !candidate.text.trim().is_empty()
                })
            });
            if missing {
                return Err(InferenceError::other(
                    "The remote server returned a draft without this segment.",
                ));
            }
        }
        Ok(draft)
    }

    async fn enhance(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        progress: ProgressSink,
    ) -> InferenceResult<Vec<EnhancedLane>> {
        let base_url = self.base_url()?;
        let sources = prepare_enhancement_sources(task_id, lanes)?;
        let payload = json!({
            "taskId": task_id,
            "clientId": self.enhancement_client_id,
            "model": GOLD_ENHANCEMENT_MODEL,
            "tracks": sources.iter().map(|source| json!({
                "trackId": source.track_id,
                "speakerKey": source.speaker_key,
                "trackLabel": source.track_label,
                "fieldName": source.field_name,
                "sourceSha256": source.source_sha256,
                "sampleRate": source.sample_rate,
                "frameCount": source.frame_count,
            })).collect::<Vec<_>>(),
        });
        tokio::time::timeout(
            ENHANCE_REQUEST_TIMEOUT,
            self.enhance_through_swarm(&base_url, task_id, payload, &sources, &progress),
        )
        .await
        .unwrap_or_else(|_| {
            Err(InferenceError::other(
                "Remote denoise took too long. Try again later.",
            ))
        })
    }

    /// Gold `transcribeErrorTaskClip` (remote): fresh per-dispatch owner, single target lane.
    async fn align(
        &self,
        request: AlignRequest,
        progress: ProgressSink,
    ) -> InferenceResult<AlignResponse> {
        progress.emit(stage("checking-remote", 0, None, None));
        let capability = self.capability().await.map_err(|error| {
            InferenceError::new(
                error.kind,
                "Couldn't check what the remote server supports.",
            )
        })?;
        let owner = random_token();
        let form = Form::new()
            .text(
                "payload",
                json!({
                    "taskId": request.task_id,
                    "tracks": [{ "lane": request.lane, "fieldName": "audio:1" }],
                })
                .to_string(),
            )
            .part(
                "audio:1",
                wav_part(request.wav, "target-speaker-original.wav".to_string()),
            );
        progress.emit(stage("waiting-remote", 0, None, None));
        let http = self
            .client
            .post(format!("{}/v1/e2e/transcribe", capability.base_url));
        let response = self
            .authorized(http, &capability, &owner)
            .header("X-Babel-Local-Engine", "1")
            .header("X-Babel-Request-Id", &request.request_id)
            .multipart(form)
            .send()
            .await
            .map_err(|_| InferenceError::other("Couldn't connect to the remote server."))?;
        let status = response.status();
        if !status.is_success() {
            let kind = if status == StatusCode::UPGRADE_REQUIRED {
                InferenceErrorKind::ReleaseMismatch
            } else {
                InferenceErrorKind::Other
            };
            return Err(InferenceError::new(
                kind,
                format!(
                    "The remote server rejected the clip. (HTTP {})",
                    status.as_u16()
                ),
            ));
        }
        let payload: Value = response.json().await.map_err(|_| {
            InferenceError::other("The remote server returned an invalid response.")
        })?;
        progress.emit(stage("adapting", 0, None, None));
        if payload.get("accessToken").and_then(Value::as_str) != Some(owner.as_str()) {
            return Err(InferenceError::other(
                "The remote server returned a clip for a different request.",
            ));
        }
        let timing = parse_timing_response(payload, &request.task_id)
            .and_then(|timing| {
                assert_model_result(&timing.models, &capability, false).map(|()| timing)
            })
            .map_err(|_| {
                InferenceError::other("The remote server returned an invalid transcription.")
            })?;
        let [track] = <[_; 1]>::try_from(timing.tracks).map_err(|_| {
            InferenceError::other("The remote server returned a different speaker track.")
        })?;
        if track.lane != request.lane {
            return Err(InferenceError::other(
                "The remote server returned a different speaker track.",
            ));
        }
        let secret = self.config.access_key.as_deref();
        if track.tokens.iter().any(|token| {
            token.text.contains(owner.as_str())
                || secret.is_some_and(|key| token.text.contains(key))
        }) {
            return Err(InferenceError::other(
                "The remote server returned unsafe transcription text.",
            ));
        }
        Ok(AlignResponse {
            tokens: track.tokens,
            duration_seconds: None,
            model: capability.release,
        })
    }
}

fn stage(
    name: impl Into<String>,
    completed: u64,
    total: Option<u64>,
    detail: Option<String>,
) -> InferenceProgress {
    InferenceProgress::Stage {
        stage: name.into(),
        completed,
        total,
        detail,
        clip: None,
    }
}

fn is_release_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=200).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn error_chain(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

fn unreachable_error(prefix: &str, error: &reqwest::Error) -> InferenceError {
    InferenceError::other(format!("{prefix} ({})", error_chain(error)))
}

/// Gold `parseResponsePayload`: JSON when it parses, else the text, `null` when empty.
async fn read_payload(response: Response) -> InferenceResult<(StatusCode, Value)> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| unreachable_error("The server response was interrupted.", &error))?;
    let payload = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    };
    Ok((status, payload))
}

fn error_code(payload: &Value) -> Option<&str> {
    payload.get("code").and_then(Value::as_str)
}

/// Typed coordinator errors: 503 `no-llm-drafting-capacity` → NoCapacity, 426 (release
/// mismatch / `llm-drafting-required`) → ReleaseMismatch, everything else Other.
fn http_error(prefix: &str, status: StatusCode, payload: &Value) -> InferenceError {
    if error_code(payload) == Some("no-llm-drafting-capacity") {
        return InferenceError::new(InferenceErrorKind::NoCapacity, NO_LLM_DRAFTING);
    }
    let detail = match payload {
        Value::Object(object) => match (object.get("error"), object.get("detail")) {
            (Some(Value::String(error)), _) => error.clone(),
            (_, Some(Value::String(detail))) => detail.clone(),
            (_, Some(detail)) => detail.to_string(),
            _ => format!("HTTP {}", status.as_u16()),
        },
        Value::String(text) if !text.trim().is_empty() => format!(
            "HTTP {}: {}",
            status.as_u16(),
            text.chars().take(240).collect::<String>()
        ),
        _ => format!("HTTP {}", status.as_u16()),
    };
    if status == StatusCode::UPGRADE_REQUIRED {
        return InferenceError::new(
            InferenceErrorKind::ReleaseMismatch,
            format!(
                "The remote server needs a different model release. Use this computer until the server is updated. ({detail})"
            ),
        );
    }
    InferenceError::other(format!("{prefix} ({detail})"))
}

/// Gold `prepareL0TimingTracks` with `allowMissingPeer`: one or two lanes named by speaker key
/// (falling back to the label), case-insensitively distinct, each a RIFF WAV.
fn prepare_tracks(lanes: &[LaneAudio]) -> InferenceResult<Vec<PreparedTrack>> {
    let mut tracks: Vec<PreparedTrack> = Vec::with_capacity(2);
    for audio in lanes {
        let lane = [audio.speaker_key.trim(), audio.label.trim()]
            .into_iter()
            .find(|identity| {
                !identity.is_empty()
                    && !tracks
                        .iter()
                        .any(|track| track.lane.to_lowercase() == identity.to_lowercase())
            });
        let Some(lane) = lane else { continue };
        if audio.wav.is_empty() {
            return Err(InferenceError::other(format!(
                "The audio track for \"{lane}\" is empty."
            )));
        }
        if audio.wav.len() < 12 || &audio.wav[0..4] != b"RIFF" || &audio.wav[8..12] != b"WAVE" {
            return Err(InferenceError::other(format!(
                "The audio track for \"{lane}\" isn't a WAV file."
            )));
        }
        tracks.push(PreparedTrack {
            lane: lane.to_string(),
            field_name: if tracks.is_empty() {
                "audio:1"
            } else {
                "audio:2"
            },
            wav: audio.wav.clone(),
        });
        if tracks.len() == 2 {
            break;
        }
    }
    if tracks.is_empty() {
        return Err(InferenceError::other(
            "Drafting needs the speaker audio. Reload the audio and try again.",
        ));
    }
    Ok(tracks)
}

fn track_declarations(tracks: &[PreparedTrack]) -> Value {
    tracks
        .iter()
        .map(|track| json!({ "lane": track.lane, "fieldName": track.field_name }))
        .collect()
}

fn wav_part(wav: Bytes, file_name: String) -> Part {
    let length = wav.len() as u64;
    Part::stream_with_length(Body::from(wav), length)
        .file_name(file_name)
        .mime_str("audio/wav")
        .expect("audio/wav is a valid MIME type")
}

fn audio_form(payload: Value, tracks: &[PreparedTrack]) -> Form {
    tracks.iter().fold(
        Form::new().text("payload", payload.to_string()),
        |form, track| {
            form.part(
                track.field_name,
                wav_part(
                    track.wav.clone(),
                    format!("{}.wav", track.field_name.replace(':', "-")),
                ),
            )
        },
    )
}

fn invalid_timing() -> InferenceError {
    InferenceError::other("The remote server returned invalid word alignment.")
}

fn finite_interval(start: f64, end: f64) -> bool {
    start.is_finite() && end.is_finite() && start >= 0.0 && end > start
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Gold `parseL0TimingResponse`.
fn parse_timing_response(payload: Value, task_id: &str) -> InferenceResult<TimingResponse> {
    let mut object = match payload {
        Value::Object(object) => object,
        _ => return Err(invalid_timing()),
    };
    object.remove("accessToken");
    let timing: TimingResponse =
        serde_json::from_value(Value::Object(object)).map_err(|_| invalid_timing())?;
    let valid = !timing.task_id.is_empty()
        && timing.task_id == task_id
        && (1..=2).contains(&timing.tracks.len())
        && timing.tracks.iter().all(|track| {
            !track.lane.is_empty()
                && is_sha256_hex(&track.pcm_sha256)
                && track.sample_rate > 0
                && track.tokens.iter().all(|token| {
                    !token.id.is_empty()
                        && !token.text.is_empty()
                        && finite_interval(token.start_seconds, token.end_seconds)
                })
                && track.segments.iter().all(|segment| {
                    !segment.id.is_empty()
                        && finite_interval(segment.start_seconds, segment.end_seconds)
                        && segment.end_sample > segment.start_sample
                        && segment.sample_rate == track.sample_rate
                })
        })
        && timing.summary.is_object()
        && timing.models.is_object();
    if valid {
        Ok(timing)
    } else {
        Err(invalid_timing())
    }
}

/// Gold `parseL0DraftResponse`.
fn parse_draft_response(payload: Value) -> InferenceResult<DraftResponse> {
    let invalid = || InferenceError::other("The remote server returned an invalid draft.");
    let draft: DraftResponse = serde_json::from_value(payload).map_err(|_| invalid())?;
    let canonical = |row: &DraftRow| {
        !row.id.is_empty()
            && !row.lane.is_empty()
            && row.start_seconds.is_finite()
            && row.end_seconds.is_finite()
            && row.end_seconds > row.start_seconds
            && !row.text.trim().is_empty()
    };
    if draft.rows.is_empty()
        || !draft.rows.iter().all(canonical)
        || !draft.summary.is_object()
        || !draft.models.is_object()
    {
        return Err(invalid());
    }
    Ok(draft)
}

/// Gold `assertRemoteModelResult`: this release on WebGPU, Original audio, CTC (`llm: false`)
/// timing or LLM draft text.
fn assert_model_result(models: &Value, capability: &Capability, llm: bool) -> InferenceResult<()> {
    let valid = models.get("release").and_then(Value::as_str) == Some(capability.release.as_str())
        && models
            .pointer("/execution/provider")
            .and_then(Value::as_str)
            == Some("webgpu")
        && models.pointer("/execution/observedNeuralCpuFallback") == Some(&Value::Bool(false))
        && models.pointer("/e2e/audio").and_then(Value::as_str) == Some("original")
        && models.pointer("/e2e/llm") == Some(&Value::Bool(llm));
    if valid {
        Ok(())
    } else {
        Err(InferenceError::other(
            "The remote server sent an unexpected result. Check its address in Settings › AI.",
        ))
    }
}

/// `LocalInferenceProgress` → stage progress.
fn parse_inference_progress(value: &Value) -> Option<InferenceProgress> {
    let phase = value.get("phase")?.as_str()?;
    let count = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_f64)
            .filter(|count| count.is_finite() && *count >= 0.0)
            .map(|count| count as u64)
    };
    Some(InferenceProgress::Stage {
        stage: phase.to_string(),
        completed: count("completed")?,
        total: count("total"),
        detail: value
            .get("lane")
            .and_then(Value::as_str)
            .map(str::to_string),
        clip: count("clip"),
    })
}

/// Gold `parseQueueStatus`: queued (position ≥ 1) or running/completed (position 0).
fn parse_queue_status(payload: &Value, request_id: &str) -> Option<InferenceProgress> {
    if payload.get("requestId")?.as_str()? != request_id {
        return None;
    }
    let position = u32::try_from(payload.get("position")?.as_u64()?).ok()?;
    let count = u32::try_from(payload.get("queuedCount")?.as_u64()?).ok()?;
    let valid = match payload.get("status")?.as_str()? {
        "queued" => position >= 1,
        "running" | "completed" => position == 0,
        _ => false,
    };
    valid.then_some(InferenceProgress::Queued { position, count })
}

/// One Original lane validated for `/v1/enhance` (Gold `PreparedEnhancementSource` + swarm track).
struct EnhancementSource {
    track_id: String,
    speaker_key: String,
    track_label: String,
    field_name: &'static str,
    source_sha256: String,
    sample_rate: u32,
    frame_count: u64,
    wav: Bytes,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Gold swarm `text()`: nonblank, ≤ 256 chars, no control characters.
fn is_swarm_text(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 256 && !value.chars().any(char::is_control)
}

/// Gold `parseEnhancementPayload` input rules plus `verifyEnhancementWav` on each Original.
fn prepare_enhancement_sources(
    task_id: &str,
    lanes: &[LaneAudio],
) -> InferenceResult<Vec<EnhancementSource>> {
    if lanes.len() != 2 {
        return Err(InferenceError::other(
            "Denoise needs the original audio of both speakers.",
        ));
    }
    if !is_swarm_text(task_id) {
        return Err(InferenceError::other(
            "This task can't be denoised remotely. (invalid task id)",
        ));
    }
    let total =
        lanes.iter().map(|lane| lane.wav.len() as u64).sum::<u64>() + ENHANCEMENT_CONTROL_BYTES;
    if total > ENHANCEMENT_MAX_REQUEST_BYTES {
        return Err(InferenceError::other(
            "This recording is too long to denoise remotely.",
        ));
    }
    let mut sources = Vec::with_capacity(2);
    for (index, lane) in lanes.iter().enumerate() {
        let speaker_key = Some(lane.speaker_key.as_str())
            .filter(|key| !key.is_empty())
            .unwrap_or(&lane.lane_id)
            .to_string();
        let track_label = [lane.label.as_str(), lane.speaker_key.as_str()]
            .into_iter()
            .find(|value| !value.is_empty())
            .unwrap_or(&lane.lane_id)
            .to_string();
        let header = verify_wav(&lane.wav, None, false).map_err(InferenceError::other)?;
        let frame_count = header.frame_count;
        let valid = is_swarm_text(&lane.lane_id)
            && is_swarm_text(&speaker_key)
            && is_swarm_text(&track_label)
            && frame_count <= 0x7fff_ffff
            && frame_count / u64::from(header.sample_rate) <= ENHANCEMENT_MAX_SECONDS
            && 44 + frame_count * 2 <= ENHANCEMENT_MAX_TRACK_BYTES;
        if !valid {
            return Err(InferenceError::other(
                "This audio can't be denoised remotely. (invalid track identity or clock)",
            ));
        }
        sources.push(EnhancementSource {
            track_id: lane.lane_id.clone(),
            speaker_key,
            track_label,
            field_name: if index == 0 { "audio:1" } else { "audio:2" },
            source_sha256: sha256_hex(&lane.wav),
            sample_rate: header.sample_rate,
            frame_count,
            wav: lane.wav.clone(),
        });
    }
    if sources[0].track_id == sources[1].track_id {
        return Err(InferenceError::other(
            "This audio can't be denoised remotely. (duplicate track id)",
        ));
    }
    Ok(sources)
}

#[derive(Debug, Clone, Copy)]
struct WavHeader {
    sample_rate: u32,
    frame_count: u64,
    channels: u16,
    format: u16,
    bits: u16,
    block_align: u64,
    data_offset: usize,
}

fn u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("4-byte slice"))
}

/// Gold `readEnhancementWavHeader` + `verifyEnhancementWav`: full RIFF structure, matching clock
/// (`expected` = sample rate, frame count), and for outputs canonical PCM16 mono with a 44-byte header.
fn verify_wav(
    bytes: &[u8],
    expected: Option<(u32, u64)>,
    output: bool,
) -> Result<WavHeader, String> {
    let problem = |detail: &str| {
        if output {
            format!("The remote server returned damaged audio. ({detail})")
        } else {
            format!("This audio can't be denoised. ({detail})")
        }
    };
    if bytes.len() as u64 > ENHANCEMENT_MAX_TRACK_BYTES || bytes.len() < 44 {
        return Err(problem("WAV exceeds the bounded audio size"));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(problem("not a RIFF PCM16 or float32 WAV"));
    }
    let (mut rate, mut channels, mut format, mut bits) = (0u32, 0u16, 0u16, 0u16);
    let (mut data_offset, mut data_length) = (None, 0usize);
    let (mut formats, mut datas, mut byte_rate, mut fmt_align) = (0, 0, 0u32, 0u16);
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32_le(bytes, offset + 4) as usize;
        if offset + 8 + size > bytes.len() {
            return Err(problem("truncated WAV chunk"));
        }
        if id == b"fmt " {
            formats += 1;
            if size < 16 {
                return Err(problem("invalid WAV format chunk"));
            }
            format = u16_le(bytes, offset + 8);
            channels = u16_le(bytes, offset + 10);
            rate = u32_le(bytes, offset + 12);
            byte_rate = u32_le(bytes, offset + 16);
            fmt_align = u16_le(bytes, offset + 20);
            bits = u16_le(bytes, offset + 22);
        } else if id == b"data" {
            datas += 1;
            data_offset = Some(offset + 8);
            data_length = size;
        }
        offset += 8 + size + (size & 1);
    }
    let bytes_per_sample = match (format, bits) {
        (1, 16) => 2u64,
        (3, 32) => 4,
        _ => 0,
    };
    let block_align = u64::from(channels) * bytes_per_sample;
    let Some(data_offset) = data_offset.filter(|_| {
        bytes_per_sample != 0
            && channels != 0
            && rate != 0
            && data_length as u64 >= block_align
            && (data_length as u64).is_multiple_of(block_align)
    }) else {
        return Err("Denoise requires nonempty PCM16 or float32 WAV audio.".to_string());
    };
    let header = WavHeader {
        sample_rate: rate,
        frame_count: data_length as u64 / block_align,
        channels,
        format,
        bits,
        block_align,
        data_offset,
    };
    if u64::from(u32_le(bytes, 4)) != bytes.len() as u64 - 8
        || expected.is_some_and(|(rate, frames)| {
            header.sample_rate != rate || header.frame_count != frames
        })
    {
        return Err(problem("WAV clock or RIFF length mismatch"));
    }
    if u64::from(byte_rate) != u64::from(rate) * block_align || u64::from(fmt_align) != block_align
    {
        return Err(problem("invalid WAV byte clock"));
    }
    if offset != bytes.len() || formats != 1 || datas != 1 {
        return Err(problem("ambiguous or trailing WAV chunks"));
    }
    if header.format == 3 {
        let end = header.data_offset + (header.frame_count * header.block_align) as usize;
        if bytes[header.data_offset..end]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|sample| !f32::from_le_bytes(*sample).is_finite())
        {
            return Err(problem("nonfinite PCM"));
        }
    }
    if output
        && (header.format != 1
            || header.channels != 1
            || header.bits != 16
            || header.data_offset != 44
            || bytes.len() as u64 != 44 + header.frame_count * 2
            || u32_le(bytes, 16) != 16)
    {
        return Err(problem("not canonical PCM16 mono WAV"));
    }
    Ok(header)
}

/// Validated worker progress (Gold `EnhancementWorkerProgress`).
#[derive(Debug, Clone, Copy)]
struct WorkerProgress {
    phase: &'static str,
    track_index: usize,
    completed_chunks: u64,
    total_chunks: u64,
}

const WORKER_PHASES: [&str; 3] = ["loading-model", "enhancing", "encoding"];

/// Gold `parseEnhancementWorkerProgress`, including the monotonicity rules.
fn parse_worker_progress(
    value: &Value,
    sources: &[EnhancementSource],
    previous: Option<&WorkerProgress>,
) -> Result<WorkerProgress, InferenceError> {
    let invalid = || InferenceError::other("The remote server returned invalid progress.");
    let object = value.as_object().ok_or_else(invalid)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "phase" | "trackId" | "trackIndex" | "trackCount" | "completedChunks" | "totalChunks"
        )
    }) {
        return Err(invalid());
    }
    let phase_name = object
        .get("phase")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let phase_rank = WORKER_PHASES
        .iter()
        .position(|phase| *phase == phase_name)
        .ok_or_else(invalid)?;
    let track_index = object
        .get("trackIndex")
        .and_then(Value::as_u64)
        .filter(|index| *index <= 1)
        .ok_or_else(invalid)? as usize;
    let completed = object
        .get("completedChunks")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    let total = object
        .get("totalChunks")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    if object.get("trackCount").and_then(Value::as_u64) != Some(2)
        || object.get("trackId").and_then(Value::as_str)
            != Some(sources[track_index].track_id.as_str())
        || total < completed
        || (phase_name == "encoding" && completed != total)
    {
        return Err(invalid());
    }
    let progress = WorkerProgress {
        phase: WORKER_PHASES[phase_rank],
        track_index,
        completed_chunks: completed,
        total_chunks: total,
    };
    if let Some(previous) = previous {
        let previous_rank = WORKER_PHASES
            .iter()
            .position(|phase| *phase == previous.phase)
            .expect("previous phase was validated");
        let regressed = progress.track_index < previous.track_index
            || (progress.track_index == previous.track_index
                && (progress.completed_chunks < previous.completed_chunks
                    || (previous.total_chunks > 0
                        && progress.total_chunks != previous.total_chunks)
                    || phase_rank < previous_rank));
        if regressed {
            return Err(InferenceError::other(
                "The remote server returned progress that went backwards.",
            ));
        }
    }
    Ok(progress)
}

/// Gold `readBoundedSwarmBlob`: enforces the bound while streaming and the declared length.
async fn read_bounded(mut response: Response, maximum: u64) -> InferenceResult<Bytes> {
    let too_large = || InferenceError::other("The remote server sent too much data.");
    let declared = match response.headers().get(CONTENT_LENGTH) {
        Some(value) => {
            let length = value
                .to_str()
                .ok()
                .filter(|text| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
                .and_then(|text| text.parse::<u64>().ok())
                .filter(|length| *length <= maximum)
                .ok_or_else(too_large)?;
            Some(length)
        }
        None => None,
    };
    let mut body = Vec::with_capacity(declared.unwrap_or(0) as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| unreachable_error(DENOISE_UNAVAILABLE, &error))?
    {
        if (body.len() + chunk.len()) as u64 > maximum {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    if declared.is_some_and(|length| length != body.len() as u64) {
        return Err(InferenceError::other(
            "The remote server sent an incomplete response.",
        ));
    }
    Ok(Bytes::from(body))
}

/// Gold `receiveTracks`: multipart `result` + `audio:1` + `audio:2`, every lane verified
/// against its Original clock and the announced SHA-256.
async fn receive_enhanced_tracks(
    response: Response,
    task_id: &str,
    sources: &[EnhancementSource],
) -> InferenceResult<Vec<EnhancedLane>> {
    let invalid = |message: &str| InferenceError::other(message.to_string());
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let boundary = multer::parse_boundary(&content_type)
        .ok()
        .filter(|_| {
            content_type
                .to_ascii_lowercase()
                .starts_with("multipart/form-data")
        })
        .ok_or_else(|| invalid("The remote server returned no audio."))?;
    let maximum = sources
        .iter()
        .map(|source| 44 + source.frame_count * 2)
        .sum::<u64>()
        + ENHANCEMENT_CONTROL_BYTES;
    let body = read_bounded(response, maximum).await?;
    let mut multipart = multer::Multipart::new(
        futures_util::stream::once(async move { Ok::<Bytes, Infallible>(body) }),
        boundary,
    );
    let mut fields: BTreeMap<String, Bytes> = BTreeMap::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| invalid("The remote server returned damaged audio. (malformed multipart)"))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if !matches!(name.as_str(), "result" | "audio:1" | "audio:2") || fields.contains_key(&name)
        {
            return Err(invalid(
                "The remote server returned damaged audio. (unexpected audio fields)",
            ));
        }
        let bytes = field.bytes().await.map_err(|_| {
            invalid("The remote server returned damaged audio. (malformed multipart)")
        })?;
        fields.insert(name, bytes);
    }
    if fields.len() != 3 {
        return Err(invalid(
            "The remote server returned damaged audio. (unexpected audio fields)",
        ));
    }
    let result_bytes = &fields["result"];
    if result_bytes.len() as u64 > ENHANCEMENT_CONTROL_BYTES {
        return Err(invalid(
            "The remote server returned damaged audio. (invalid result metadata)",
        ));
    }
    let result: Map<String, Value> = std::str::from_utf8(result_bytes)
        .ok()
        .and_then(|text| serde_json::from_str(text).ok())
        .ok_or_else(|| {
            invalid("The remote server returned damaged audio. (invalid result metadata)")
        })?;
    let tracks = result
        .get("tracks")
        .and_then(Value::as_array)
        .filter(|tracks| tracks.len() == 2);
    let (Some(tracks), true) = (
        tracks,
        result.get("ok") == Some(&Value::Bool(true))
            && result.get("provider").and_then(Value::as_str) == Some("swarm")
            && result.get("taskId").and_then(Value::as_str) == Some(task_id)
            && result.get("model").and_then(Value::as_str) == Some(GOLD_ENHANCEMENT_MODEL.id),
    ) else {
        return Err(invalid(
            "The remote server returned audio for a different task or model.",
        ));
    };
    let mut lanes = Vec::with_capacity(2);
    for (source, metadata) in sources.iter().zip(tracks) {
        let wav_sha256 = parse_track_metadata(metadata, source)?;
        let wav = fields[source.field_name].clone();
        if wav.len() as u64 != 44 + source.frame_count * 2 {
            return Err(invalid(
                "The remote server returned damaged audio. (WAV length mismatch)",
            ));
        }
        verify_wav(&wav, Some((source.sample_rate, source.frame_count)), true)
            .map_err(InferenceError::other)?;
        if sha256_hex(&wav) != wav_sha256 {
            return Err(invalid(
                "The remote server returned damaged audio. (SHA-256 mismatch)",
            ));
        }
        lanes.push(EnhancedLane {
            lane_id: source.track_id.clone(),
            model_id: GOLD_ENHANCEMENT_MODEL.id.to_string(),
            wav,
        });
    }
    Ok(lanes)
}

/// Gold `parseEnhancementTrackMetadata`; returns the announced `wavSha256`.
fn parse_track_metadata(value: &Value, source: &EnhancementSource) -> InferenceResult<String> {
    let invalid = || {
        InferenceError::other(
            "The remote server returned audio that doesn't match the original recording.",
        )
    };
    let object = value.as_object().ok_or_else(invalid)?;
    let allowed = [
        "trackId",
        "speakerKey",
        "trackLabel",
        "mimeType",
        "sampleRate",
        "frameCount",
        "sourceSha256",
        "wavSha256",
        "totalBytes",
        "chunkCount",
    ];
    let total_bytes = 44 + source.frame_count * 2;
    let text = |key: &str| object.get(key).and_then(Value::as_str);
    let number = |key: &str| object.get(key).and_then(Value::as_u64);
    let wav_sha256 = text("wavSha256").filter(|sha| is_sha256_hex(sha));
    let valid = object.keys().all(|key| allowed.contains(&key.as_str()))
        && text("trackId") == Some(source.track_id.as_str())
        && text("speakerKey") == Some(source.speaker_key.as_str())
        && text("trackLabel") == Some(source.track_label.as_str())
        && text("sourceSha256") == Some(source.source_sha256.as_str())
        && number("sampleRate") == Some(u64::from(source.sample_rate))
        && number("frameCount") == Some(source.frame_count)
        && text("mimeType") == Some("audio/wav")
        && number("totalBytes") == Some(total_bytes)
        && number("chunkCount") == Some(total_bytes.div_ceil(ENHANCEMENT_CHUNK_BYTES));
    match wav_sha256 {
        Some(sha) if valid => Ok(sha.to_string()),
        _ => Err(invalid()),
    }
}
