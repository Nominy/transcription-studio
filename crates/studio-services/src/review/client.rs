//! HTTP client for the Reviewer/Gold Drafting backend (`reviewgen.ovh`).
//!
//! The backend has no authentication on its review/draft routes, so requests never carry an
//! `Authorization` header (also when a Sub-account session is active).

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Value, json};
use url::form_urlencoded::byte_serialize;

use super::model::{
    BackendHealth, DraftResult, DraftTranscription, FinalizeOutcome, GeneratedReview,
    RedistributionPreview, ReviewerSession, ReviewerTemplate, parse_backend_health,
    parse_draft_result, parse_finalize_outcome, parse_generated_review, parse_redistribution,
    parse_reviewer_session, parse_template_search, parse_transcription,
};

pub const REVIEW_BACKEND_BASE_URL_ENV: &str = "STUDIO_REVIEW_BACKEND_BASE_URL";
pub const DEFAULT_REVIEW_BACKEND_BASE_URL: &str = "https://reviewgen.ovh";
const DRAFT_GENERATE_PATH: &str = "/api/draft/generate";
const DRAFT_GENERATE_STREAM_PATH: &str = "/api/draft/generate/stream";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendHttpMethod {
    Get,
    Post,
}

/// One audio lane attached to a drafting request as multipart parts
/// `audioTrackMeta:<track_id>` (JSON) + `audioTrack:<track_id>` (file).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrack {
    pub track_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub metadata: Value,
    /// Shared bytes: cloning a request never copies the encoded audio.
    pub bytes: Bytes,
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
    ReviewGetSession {
        session_id: String,
    },
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
    ReviewHistory {
        query: Vec<(String, String)>,
    },
    ReviewHistoryItem {
        history_id: String,
    },
    DraftGenerate {
        payload: Value,
        audio_tracks: Vec<AudioTrack>,
    },
    DraftGenerateStream {
        payload: Value,
        audio_tracks: Vec<AudioTrack>,
    },
    DraftTranscribeSegment {
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
            Self::ReviewGetSession { session_id } => get(format!(
                "/api/review/sessions/{}",
                encode_path_segment(session_id)
            )),
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
            Self::ReviewHistory { query } => {
                let suffix = if query.is_empty() {
                    String::new()
                } else {
                    format!("?{}", encode_query_params(query))
                };
                get(format!("/api/review-history{suffix}"))
            }
            Self::ReviewHistoryItem { history_id } => get(format!(
                "/api/review-history/{}",
                encode_path_segment(history_id)
            )),
            Self::DraftGenerate {
                payload,
                audio_tracks,
            } => post_with_audio(DRAFT_GENERATE_PATH, payload, audio_tracks),
            Self::DraftGenerateStream {
                payload,
                audio_tracks,
            } => post_with_audio(DRAFT_GENERATE_STREAM_PATH, payload, audio_tracks),
            Self::DraftTranscribeSegment {
                payload,
                audio_tracks,
            } => post_with_audio("/api/broker/transcribe-segment", payload, audio_tracks),
            Self::DraftRedistributeText(body) => {
                post("/api/broker/redistribute-text", Some(body.clone()))
            }
        }
    }

    /// Stream requests ask for `text/event-stream` and resolve to the `done` event payload.
    pub fn is_stream(&self) -> bool {
        matches!(self, Self::DraftGenerateStream { .. })
    }
}

/// Reviewer/Drafting backend client. Cheap to clone; all methods are `Send` futures that need
/// the services tokio runtime (wrap in [`crate::runtime::spawn`] from a gpui executor).
#[derive(Debug, Clone)]
pub struct ReviewBackend {
    base_url: String,
    client: reqwest::Client,
}

impl ReviewBackend {
    /// Uses `STUDIO_REVIEW_BACKEND_BASE_URL` (trimmed, no trailing `/`) or `https://reviewgen.ovh`.
    pub fn from_env() -> Self {
        Self::new(review_backend_base_url_from_env())
    }

    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            client: crate::http::client(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Sends one request and returns the raw success body (`done` payload for stream requests).
    /// Non-2xx → `review backend returned HTTP <code>: <body>`.
    pub async fn send(&self, request: &ReviewBackendRequest) -> Result<String> {
        self.send_parts(request.to_parts(), request.is_stream())
            .await
    }

    async fn send_parts(&self, parts: BackendRequestParts, stream: bool) -> Result<String> {
        let url = review_backend_url(&self.base_url, &parts.path);
        let builder = match parts.method {
            BackendHttpMethod::Get => self.client.get(&url),
            BackendHttpMethod::Post => self.client.post(&url),
        }
        .header(
            ACCEPT,
            if stream {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
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

        let response = builder
            .send()
            .await
            .context("review backend request failed")?;
        let status = response.status();
        let payload = response
            .text()
            .await
            .context("review backend response read failed")?;
        if !status.is_success() {
            bail!(
                "review backend returned HTTP {}: {}",
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

    /// Native capability panel "Check backend".
    pub async fn health(&self) -> Result<BackendHealth> {
        parse_backend_health(&self.send(&ReviewBackendRequest::BackendHealth).await?)
    }

    /// Reviewer "Generate review" (payload from [`super::build_reviewer_payload`]).
    pub async fn generate_review(&self, payload: Value) -> Result<GeneratedReview> {
        parse_generated_review(
            &self
                .send(&ReviewBackendRequest::ReviewGenerate(payload))
                .await?,
        )
    }

    /// Reviewer "Create session" (payload from [`super::build_reviewer_payload`]).
    pub async fn create_session(&self, payload: Value) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewCreateSession(payload))
            .await
    }

    pub async fn get_session(&self, session_id: &str) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewGetSession {
            session_id: session_id.to_string(),
        })
        .await
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

    /// `decision` is `"accepted"` or `"rejected"`.
    pub async fn decide_template_suggestion(
        &self,
        session_id: &str,
        proposal_id: &str,
        decision: &str,
    ) -> Result<ReviewerSession> {
        self.session_request(ReviewBackendRequest::ReviewDecideTemplateSuggestion {
            session_id: session_id.to_string(),
            proposal_id: proposal_id.to_string(),
            body: json!({ "decision": decision }),
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

    /// Template search (old panel used `limit = Some(20)`).
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

    /// Review history list; raw JSON body (the old panel displayed it verbatim).
    pub async fn history(&self, query: Vec<(String, String)>) -> Result<String> {
        self.send(&ReviewBackendRequest::ReviewHistory { query })
            .await
    }

    pub async fn history_item(&self, history_id: &str) -> Result<String> {
        self.send(&ReviewBackendRequest::ReviewHistoryItem {
            history_id: history_id.to_string(),
        })
        .await
    }

    /// Gold Drafting row generation via the stream route. A `draftSessionId` is added when
    /// missing; if the stream connection drops, the final result is reconciled with one
    /// non-stream request for the same session (`reconciled_after_stream_loss`).
    pub async fn generate_drafts(
        &self,
        mut payload: Value,
        audio_tracks: Vec<AudioTrack>,
    ) -> Result<DraftResult> {
        super::payload::ensure_draft_session_id(&mut payload);
        let stream = post_with_audio(DRAFT_GENERATE_STREAM_PATH, &payload, &audio_tracks);
        match self.send_parts(stream, true).await {
            Ok(raw) => parse_draft_result(&raw),
            Err(error) if is_reconnectable_draft_stream_error(&format!("{error:#}")) => {
                let reconcile = post_with_audio(DRAFT_GENERATE_PATH, &payload, &audio_tracks);
                let mut result = parse_draft_result(&self.send_parts(reconcile, false).await?)?;
                result.reconciled_after_stream_loss = true;
                Ok(result)
            }
            Err(error) => Err(error),
        }
    }

    /// Broker transcription of one segment (payload from
    /// [`super::build_draft_transcribe_segment_payload`]). The old UI required lane audio.
    pub async fn transcribe_segment(
        &self,
        payload: Value,
        audio_tracks: Vec<AudioTrack>,
    ) -> Result<DraftTranscription> {
        let raw = self
            .send(&ReviewBackendRequest::DraftTranscribeSegment {
                payload: payload.clone(),
                audio_tracks,
            })
            .await?;
        parse_transcription(&payload, &raw)
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

/// Errors after which a stream with a `draftSessionId` is reconciled via the non-stream route.
pub fn is_reconnectable_draft_stream_error(error: &str) -> bool {
    let message = error.to_ascii_lowercase();
    [
        "could not reach backend",
        "failed to fetch",
        "networkerror",
        "load failed",
        "response read failed",
    ]
    .iter()
    .any(|needle| message.contains(needle))
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

/// Parses the panel's free-form history query (`a=1&b=2`; bare terms become `q=<term>`).
pub fn parse_history_query(input: &str) -> Vec<(String, String)> {
    input
        .split('&')
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (key, value) = trimmed.split_once('=').unwrap_or(("q", trimmed));
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
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
