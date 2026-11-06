//! HTTP client for the Reviewer/Gold Drafting backend (`reviewgen.ovh`).
//!
//! Requests never carry Babel credentials. Paid review routes (generate, session create,
//! template suggestions, grade) carry the user's own OpenRouter key in `X-OpenRouter-Key`
//! (plus `X-OpenRouter-Model` when set), only to `https://reviewgen.ovh` or a localhost
//! backend, without following redirects.

use std::fmt;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Value, json};
use url::form_urlencoded::byte_serialize;

use super::model::{
    FinalizeOutcome, GeneratedReview, GradeResult, ReviewerSession, ReviewerTemplate,
    parse_finalize_outcome, parse_generated_review, parse_grade_result, parse_reviewer_session,
    parse_template_search,
};
use super::snapshot::ReviewSnapshot;
use exaudio_babel_domain::i18n::localized;
use exaudio_gold::polish::{
    AudioTrack, BackendHealth, DraftResult, DraftStreamEvent, DraftStreamStep,
    RedistributionPreview, SseParser, draft_stream_step, parse_backend_health, parse_draft_result,
    parse_redistribution,
};

pub const REVIEW_BACKEND_BASE_URL_ENV: &str = "STUDIO_REVIEW_BACKEND_BASE_URL";
pub const DEFAULT_REVIEW_BACKEND_BASE_URL: &str = "https://reviewgen.ovh";
pub const OPENROUTER_KEY_HEADER: &str = "X-OpenRouter-Key";
pub const OPENROUTER_MODEL_HEADER: &str = "X-OpenRouter-Model";
/// Keyed (paid) requests time out after 180 s, like the Review Helper.
const KEYED_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_KEY_CHARS: usize = 512;
const MAX_MODEL_CHARS: usize = 128;
const DRAFT_GENERATE_PATH: &str = "/api/draft/generate";
const DRAFT_GENERATE_STREAM_PATH: &str = "/api/draft/generate/stream";

/// The user's OpenRouter key and optional model for paid review routes. `Debug` redacts the key.
#[derive(Clone, PartialEq, Eq)]
pub struct ReviewCredentials {
    api_key: String,
    model: Option<String>,
}

impl ReviewCredentials {
    /// Validates the Review settings: key `sk-or-…` (≤512 chars, no whitespace); model blank
    /// (server default) or `author/model` (≤128 chars).
    pub fn new(api_key: &str, model: &str) -> Result<Self> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            bail!("{}", missing_key_message());
        }
        if !api_key.starts_with("sk-or-")
            || api_key.chars().count() > MAX_KEY_CHARS
            || api_key.chars().any(char::is_whitespace)
        {
            bail!(
                "{}",
                localized(
                    "The key must start with sk-or-.",
                    "Ключ должен начинаться с sk-or-."
                )
            );
        }
        let model = model.trim();
        if !model.is_empty() && !is_valid_model_id(model) {
            bail!(
                "{}",
                localized(
                    "The model must look like author/model.",
                    "Модель должна выглядеть как author/model."
                )
            );
        }
        Ok(Self {
            api_key: api_key.to_string(),
            model: (!model.is_empty()).then(|| model.to_string()),
        })
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }
}

fn missing_key_message() -> &'static str {
    localized(
        "Add an OpenRouter key in Settings › Babel.",
        "Добавьте ключ OpenRouter в Настройки › Babel.",
    )
}

impl fmt::Debug for ReviewCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReviewCredentials")
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .finish()
    }
}

/// `^[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._:-]*$` (case-insensitive), ≤128 chars.
fn is_valid_model_id(model: &str) -> bool {
    let segment = |part: &str, extra: &[char]| {
        let mut chars = part.chars();
        chars
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
            && chars.all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
    };
    model.len() <= MAX_MODEL_CHARS
        && model.split_once('/').is_some_and(|(provider, name)| {
            segment(provider, &['.', '_', '-']) && segment(name, &['.', '_', ':', '-'])
        })
}

/// Keys only go to `https://reviewgen.ovh` or `http://localhost|127.0.0.1[:port]` (bare origin).
fn key_transport_allowed(base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(base_url) else {
        return false;
    };
    let bare = url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/");
    let host = url.host_str().unwrap_or_default();
    bare && match url.scheme() {
        "https" => host == "reviewgen.ovh" && url.port().is_none(),
        "http" => host == "localhost" || host == "127.0.0.1",
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendHttpMethod {
    Get,
    Post,
}

/// Method/path/body of a backend request, independent of the transport.
#[derive(Debug, Clone, PartialEq)]
pub struct BackendRequestParts {
    pub method: BackendHttpMethod,
    pub path: String,
    pub body: Option<Value>,
    pub audio_tracks: Vec<AudioTrack>,
}

/// Every route the old native capability panel used.
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewBackendRequest {
    BackendHealth,
    ReviewGenerate(Value),
    ReviewCreateSession(Value),
    /// `{ reviewActionId, original, current }`; paid.
    ReviewGrade(Value),
    ReviewSearchTemplates {
        query: String,
        limit: Option<usize>,
    },
    ReviewUpdateComments {
        session_id: String,
        body: Value,
    },
    ReviewSetTemplateMatch {
        session_id: String,
        card_id: String,
        body: Value,
    },
    ReviewClearTemplateMatch {
        session_id: String,
        card_id: String,
    },
    ReviewGenerateTemplateSuggestions {
        session_id: String,
    },
    ReviewDecideTemplateSuggestion {
        session_id: String,
        proposal_id: String,
        body: Value,
    },
    ReviewFinalizeSession {
        session_id: String,
        body: Value,
    },
    ReviewSubmitAnalytics(Value),
    DraftGenerate {
        payload: Value,
        audio_tracks: Vec<AudioTrack>,
    },
    DraftGenerateStream {
        payload: Value,
        audio_tracks: Vec<AudioTrack>,
    },
    DraftRedistributeText(Value),
}

impl ReviewBackendRequest {
    pub fn to_parts(&self) -> BackendRequestParts {
        match self {
            Self::BackendHealth => get("/health"),
            Self::ReviewGenerate(body) => post("/api/review/generate", Some(body.clone())),
            Self::ReviewCreateSession(body) => post("/api/review/sessions", Some(body.clone())),
            Self::ReviewGrade(body) => post("/api/review/grade", Some(body.clone())),
            Self::ReviewSearchTemplates { query, limit } => {
                let mut params = vec![("q".to_string(), query.clone())];
                if let Some(limit) = limit {
                    params.push(("limit".to_string(), limit.to_string()));
                }
                get(format!(
                    "/api/review/templates/search?{}",
                    encode_query_params(&params)
                ))
            }
            Self::ReviewUpdateComments { session_id, body } => post(
                format!(
                    "/api/review/sessions/{}/comments",
                    encode_path_segment(session_id)
                ),
                Some(body.clone()),
            ),
            Self::ReviewSetTemplateMatch {
                session_id,
                card_id,
                body,
            } => post(
                format!(
                    "/api/review/sessions/{}/cards/{}/template-match",
                    encode_path_segment(session_id),
                    encode_path_segment(card_id)
                ),
                Some(body.clone()),
            ),
            Self::ReviewClearTemplateMatch {
                session_id,
                card_id,
            } => post(
                format!(
                    "/api/review/sessions/{}/cards/{}/template-clear",
                    encode_path_segment(session_id),
                    encode_path_segment(card_id)
                ),
                None,
            ),
            Self::ReviewGenerateTemplateSuggestions { session_id } => post(
                format!(
                    "/api/review/sessions/{}/template-suggestions",
                    encode_path_segment(session_id)
                ),
                None,
            ),
            Self::ReviewDecideTemplateSuggestion {
                session_id,
                proposal_id,
                body,
            } => post(
                format!(
                    "/api/review/sessions/{}/template-suggestions/{}/decision",
                    encode_path_segment(session_id),
                    encode_path_segment(proposal_id)
                ),
                Some(body.clone()),
            ),
            Self::ReviewFinalizeSession { session_id, body } => post(
                format!(
                    "/api/review/sessions/{}/finalize",
                    encode_path_segment(session_id)
                ),
                Some(body.clone()),
            ),
            Self::ReviewSubmitAnalytics(body) => post(
                "/api/trpc/transcriptions.submitTranscriptReviewAction",
                Some(body.clone()),
            ),
            Self::DraftGenerate {
                payload,
                audio_tracks,
            } => post_with_audio(DRAFT_GENERATE_PATH, payload, audio_tracks),
            Self::DraftGenerateStream {
                payload,
                audio_tracks,
            } => post_with_audio(DRAFT_GENERATE_STREAM_PATH, payload, audio_tracks),
            Self::DraftRedistributeText(body) => {
                post("/api/broker/redistribute-text", Some(body.clone()))
            }
        }
    }

    /// Stream requests ask for `text/event-stream` and resolve to the `done` event payload.
    pub fn is_stream(&self) -> bool {
        matches!(self, Self::DraftGenerateStream { .. })
    }

    /// Paid routes that need the user's OpenRouter key (`X-OpenRouter-Key`).
    pub fn requires_openrouter_key(&self) -> bool {
        matches!(
            self,
            Self::ReviewGenerate(_)
                | Self::ReviewCreateSession(_)
                | Self::ReviewGenerateTemplateSuggestions { .. }
                | Self::ReviewGrade(_)
        )
    }
}

/// Template-suggestion decision (`approved` / `rejected`, the backend's values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalDecision {
    Approved,
    Rejected,
}

impl ProposalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

/// Reviewer/Drafting backend client. Cheap to clone; all methods are `Send` futures that need
/// the services tokio runtime (wrap in [`studio_services::runtime::spawn`] from a gpui executor).
#[derive(Debug, Clone)]
pub struct ReviewBackend {
    base_url: String,
    client: reqwest::Client,
    /// No redirects, 180 s timeout: used only for keyed requests.
    keyed_client: reqwest::Client,
    credentials: Option<ReviewCredentials>,
}

impl ReviewBackend {
    /// Uses `STUDIO_REVIEW_BACKEND_BASE_URL` (trimmed, no trailing `/`) or `https://reviewgen.ovh`.
    pub fn from_env() -> Self {
        Self::new(review_backend_base_url_from_env())
    }

    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            client: studio_services::http::client(),
            keyed_client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(KEYED_REQUEST_TIMEOUT)
                .build()
                .expect("keyed review HTTP client should build"),
            credentials: None,
        }
    }

    /// A client sending `credentials` on paid routes. Fails when this server's origin is not
    /// allowed to receive keys.
    pub fn with_credentials(&self, credentials: ReviewCredentials) -> Result<Self> {
        if !key_transport_allowed(&self.base_url) {
            bail!(
                "{}",
                localized(
                    "Your OpenRouter key is only sent to reviewgen.ovh or a server on this computer.",
                    "Ключ OpenRouter отправляется только на reviewgen.ovh или сервер на этом компьютере.",
                )
            );
        }
        Ok(Self {
            credentials: Some(credentials),
            ..self.clone()
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Sends one request and returns the raw success body (`done` payload for stream requests).
    /// Non-2xx → `review server returned HTTP <code>: <body>`. Paid routes without credentials
    /// fail before sending.
    pub async fn send(&self, request: &ReviewBackendRequest) -> Result<String> {
        let credentials = if request.requires_openrouter_key() {
            Some(
                self.credentials
                    .as_ref()
                    .ok_or_else(|| anyhow!("{}", missing_key_message()))?,
            )
        } else {
            None
        };
        self.send_parts(request.to_parts(), request.is_stream(), credentials)
            .await
    }

    async fn send_parts(
        &self,
        parts: BackendRequestParts,
        stream: bool,
        credentials: Option<&ReviewCredentials>,
    ) -> Result<String> {
        let response = self.start_request(parts, stream, credentials).await?;
        let status = response.status();
        let payload = response
            .text()
            .await
            .context("review server response read failed")?;
        if !status.is_success() {
            bail!(
                "review server returned HTTP {}: {}",
                status.as_u16(),
                payload
            );
        }
        if stream && !payload.trim_start().starts_with('{') {
            parse_sse_done_payload(&payload)
        } else {
            Ok(payload)
        }
    }

    /// Sends a request and returns the response with its status unchecked (3xx refused).
    async fn start_request(
        &self,
        parts: BackendRequestParts,
        stream: bool,
        credentials: Option<&ReviewCredentials>,
    ) -> Result<reqwest::Response> {
        let url = review_backend_url(&self.base_url, &parts.path);
        let client = if credentials.is_some() {
            &self.keyed_client
        } else {
            &self.client
        };
        let mut builder = match parts.method {
            BackendHttpMethod::Get => client.get(&url),
            BackendHttpMethod::Post => client.post(&url),
        }
        .header(
            ACCEPT,
            if stream {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
        if let Some(credentials) = credentials {
            builder = builder.header(OPENROUTER_KEY_HEADER, &credentials.api_key);
            if let Some(model) = &credentials.model {
                builder = builder.header(OPENROUTER_MODEL_HEADER, model);
            }
        }
        let builder = if !parts.audio_tracks.is_empty() {
            let payload = parts.body.unwrap_or(Value::Null);
            let mut form = reqwest::multipart::Form::new().text("payload", payload.to_string());
            for track in parts.audio_tracks {
                let byte_len = track.bytes.len() as u64;
                let part = reqwest::multipart::Part::stream_with_length(track.bytes, byte_len)
                    .file_name(track.file_name)
                    .mime_str(&track.mime_type)
                    .context("failed to attach broker audio")?;
                form = form
                    .text(
                        format!("audioTrackMeta:{}", track.track_id),
                        track.metadata.to_string(),
                    )
                    .part(format!("audioTrack:{}", track.track_id), part);
            }
            builder.multipart(form)
        } else if let Some(body) = parts.body {
            builder.header(CONTENT_TYPE, "application/json").json(&body)
        } else {
            builder
        };

        // `without_url`: request errors must not echo URLs; keys only travel in headers.
        let response = builder
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("review server request failed")?;
        let status = response.status();
        if status.is_redirection() {
            bail!(
                "review server returned HTTP {}: redirects are not followed",
                status.as_u16()
            );
        }
        Ok(response)
    }

    /// Native capability panel "Check backend".
    pub async fn health(&self) -> Result<BackendHealth> {
        parse_backend_health(&self.send(&ReviewBackendRequest::BackendHealth).await?)
    }

    /// Fast-mode Magic Review (paid).
    pub async fn generate_review(&self, snapshot: &ReviewSnapshot) -> Result<GeneratedReview> {
        parse_generated_review(
            &self
                .send(&ReviewBackendRequest::ReviewGenerate(
                    snapshot.prepare_body(),
                ))
                .await?,
        )
    }

    /// Interactive Magic Review / Refresh: a new session from `snapshot` (paid).
    pub async fn create_session(&self, snapshot: &ReviewSnapshot) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewCreateSession(
            snapshot.prepare_body(),
        ))
        .await
    }

    /// Grader: five category grades for `snapshot` (paid).
    pub async fn grade(&self, snapshot: &ReviewSnapshot) -> Result<GradeResult> {
        parse_grade_result(
            &self
                .send(&ReviewBackendRequest::ReviewGrade(snapshot.grade_body()))
                .await?,
        )
    }

    /// Saves session + card comments; `body` from [`super::ReviewerWorkspace::begin_comment_save`].
    pub async fn update_comments(&self, session_id: &str, body: Value) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewUpdateComments {
            session_id: session_id.to_string(),
            body,
        })
        .await
    }

    pub async fn set_template_match(
        &self,
        session_id: &str,
        card_id: &str,
        template_id: &str,
    ) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewSetTemplateMatch {
            session_id: session_id.to_string(),
            card_id: card_id.to_string(),
            body: json!({ "templateId": template_id }),
        })
        .await
    }

    pub async fn clear_template_match(
        &self,
        session_id: &str,
        card_id: &str,
    ) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewClearTemplateMatch {
            session_id: session_id.to_string(),
            card_id: card_id.to_string(),
        })
        .await
    }

    pub async fn generate_template_suggestions(&self, session_id: &str) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewGenerateTemplateSuggestions {
            session_id: session_id.to_string(),
        })
        .await
    }

    pub async fn decide_template_suggestion(
        &self,
        session_id: &str,
        proposal_id: &str,
        decision: ProposalDecision,
    ) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewDecideTemplateSuggestion {
            session_id: session_id.to_string(),
            proposal_id: proposal_id.to_string(),
            body: json!({ "decision": decision.as_str() }),
        })
        .await
    }

    /// `mode` is `"apply"` or `"skip"`.
    pub async fn finalize_session(&self, session_id: &str, mode: &str) -> Result<FinalizeOutcome> {
        let body = json!({ "mode": mode });
        let payload = self
            .send(&ReviewBackendRequest::ReviewFinalizeSession {
                session_id: session_id.to_string(),
                body: body.clone(),
            })
            .await?;
        parse_finalize_outcome(session_id, &body, &payload)
    }

    /// Per-card template search (unpaid).
    pub async fn search_templates(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ReviewerTemplate>> {
        parse_template_search(
            &self
                .send(&ReviewBackendRequest::ReviewSearchTemplates {
                    query: query.trim().to_string(),
                    limit,
                })
                .await?,
        )
    }

    /// Reviewer analytics submission (payload from [`super::build_reviewer_analytics_payload`]).
    pub async fn submit_analytics(&self, payload: Value) -> Result<String> {
        self.send(&ReviewBackendRequest::ReviewSubmitAnalytics(payload))
            .await
    }

    /// Gold Draft polish via `/api/draft/generate/stream`: `started` / `row` events reach
    /// `on_event` as they arrive; the `done` payload is the result. A `draftSessionId` is added
    /// when missing; if the connection drops (not an HTTP or `error` event failure), the same
    /// session is reconciled once through the non-stream route (`Reconnecting` is reported and
    /// the result is marked `reconciled_after_stream_loss`).
    pub async fn generate_drafts(
        &self,
        mut payload: Value,
        audio_tracks: Vec<AudioTrack>,
        on_event: impl Fn(DraftStreamEvent) + Send + Sync,
    ) -> Result<DraftResult> {
        super::payload::ensure_draft_session_id(&mut payload);
        match self.stream_drafts(&payload, &audio_tracks, &on_event).await {
            Ok(result) => Ok(result),
            Err(StreamFailure::Connection) => {
                on_event(DraftStreamEvent::Reconnecting);
                let reconcile = post_with_audio(DRAFT_GENERATE_PATH, &payload, &audio_tracks);
                let mut result =
                    parse_draft_result(&self.send_parts(reconcile, false, None).await?)?;
                result.reconciled_after_stream_loss = true;
                Ok(result)
            }
            Err(StreamFailure::Rejected(error)) => Err(error),
        }
    }

    async fn stream_drafts(
        &self,
        payload: &Value,
        audio_tracks: &[AudioTrack],
        on_event: &(impl Fn(DraftStreamEvent) + Send + Sync),
    ) -> Result<DraftResult, StreamFailure> {
        let parts = post_with_audio(DRAFT_GENERATE_STREAM_PATH, payload, audio_tracks);
        let mut response = self
            .start_request(parts, true, None)
            .await
            .map_err(|_| StreamFailure::Connection)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(StreamFailure::Rejected(anyhow!(
                "review server returned HTTP {}: {}",
                status.as_u16(),
                body
            )));
        }
        let mut parser = SseParser::default();
        let mut done = None;
        loop {
            let chunk = response
                .chunk()
                .await
                .map_err(|_| StreamFailure::Connection)?;
            let events = match &chunk {
                Some(bytes) => parser.push(bytes),
                None => parser.finish(),
            };
            for event in &events {
                match draft_stream_step(event).map_err(StreamFailure::Rejected)? {
                    DraftStreamStep::Progress(progress) => on_event(progress),
                    DraftStreamStep::Done(payload) => done = Some(payload),
                    DraftStreamStep::Ignored => {}
                }
            }
            if chunk.is_none() {
                break;
            }
        }
        // The stream ended without its required terminal event: treat it as a lost stream.
        let payload = done.ok_or(StreamFailure::Connection)?;
        parse_draft_result(&payload).map_err(StreamFailure::Rejected)
    }

    /// Broker text redistribution review (payload from [`super::build_draft_redistribute_payload`]).
    pub async fn redistribute_text(&self, payload: Value) -> Result<Vec<RedistributionPreview>> {
        let raw = self
            .send(&ReviewBackendRequest::DraftRedistributeText(
                payload.clone(),
            ))
            .await?;
        parse_redistribution(&payload, &raw)
    }

    async fn session_request(&self, request: ReviewBackendRequest) -> Result<ReviewerSession> {
        parse_reviewer_session(&self.send(&request).await?)
    }
}

/// Extracts the `done` event payload from a buffered SSE body; an `error` event fails.
pub fn parse_sse_done_payload(stream: &str) -> Result<String> {
    let mut final_payload = None;
    for block in stream.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let mut event_name = "message";
        let mut data_lines = Vec::new();
        for line in block.lines().map(str::trim_end) {
            if let Some(value) = line.strip_prefix("event:") {
                event_name = value.trim();
            } else if let Some(value) = line.strip_prefix("data:") {
                data_lines.push(value.trim_start());
            }
        }
        if data_lines.is_empty() {
            continue;
        }
        let payload = data_lines.join("\n");
        match event_name {
            "error" => bail!("draft stream returned error event: {payload}"),
            "done" => final_payload = Some(payload),
            _ => {}
        }
    }
    final_payload.ok_or_else(|| anyhow!("draft stream finished without a done event"))
}

/// Why a draft stream ended without a result: a lost connection is reconciled for the same
/// session; an HTTP rejection, `error` event or malformed result is final.
enum StreamFailure {
    Connection,
    Rejected(anyhow::Error),
}

pub fn draft_payload_has_session_id(payload: &Value) -> bool {
    payload
        .get("draftSessionId")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

pub fn review_backend_base_url_from_env() -> String {
    std::env::var(REVIEW_BACKEND_BASE_URL_ENV)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REVIEW_BACKEND_BASE_URL.to_string())
}

pub fn review_backend_url(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

fn get(path: impl Into<String>) -> BackendRequestParts {
    BackendRequestParts {
        method: BackendHttpMethod::Get,
        path: path.into(),
        body: None,
        audio_tracks: Vec::new(),
    }
}

fn post(path: impl Into<String>, body: Option<Value>) -> BackendRequestParts {
    BackendRequestParts {
        method: BackendHttpMethod::Post,
        path: path.into(),
        body,
        audio_tracks: Vec::new(),
    }
}

fn post_with_audio(path: &str, body: &Value, audio_tracks: &[AudioTrack]) -> BackendRequestParts {
    BackendRequestParts {
        method: BackendHttpMethod::Post,
        path: path.to_string(),
        body: Some(body.clone()),
        audio_tracks: audio_tracks.to_vec(),
    }
}

fn encode_path_segment(value: &str) -> String {
    byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

fn encode_query_params(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                byte_serialize(key.as_bytes()).collect::<String>(),
                byte_serialize(value.as_bytes()).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
