use iced::Task;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::Value;
use url::form_urlencoded::byte_serialize;

use super::Message;

pub(super) const REVIEW_BACKEND_BASE_URL_ENV: &str = "STUDIO_REVIEW_BACKEND_BASE_URL";
pub(super) const DEFAULT_REVIEW_BACKEND_BASE_URL: &str = "https://reviewgen.ovh";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BackendHttpMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct BackendRequestParts {
    pub(super) method: BackendHttpMethod,
    pub(super) path: String,
    pub(super) body: Option<Value>,
    pub(super) audio_tracks: Vec<BackendMultipartAudioTrack>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct BackendMultipartAudioTrack {
    pub(super) track_id: String,
    pub(super) file_name: String,
    pub(super) mime_type: String,
    pub(super) metadata: Value,
    pub(super) bytes: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ReviewBackendRequest {
    BackendHealth,
    ReviewPrepare(Value),
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
    DraftGenerate(Value),
    DraftGenerateWithAudio {
        payload: Value,
        audio_tracks: Vec<BackendMultipartAudioTrack>,
    },
    DraftGenerateStream(Value),
    DraftGenerateStreamWithAudio {
        payload: Value,
        audio_tracks: Vec<BackendMultipartAudioTrack>,
    },
    DraftBrokerTranscribeSegment(Value),
    DraftBrokerTranscribeSegmentWithAudio {
        payload: Value,
        audio_tracks: Vec<BackendMultipartAudioTrack>,
    },
    DraftBrokerRedistributeText(Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReviewBackendRequestToken {
    pub(super) workspace_generation: u64,
    pub(super) sequence: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ReviewBackendServiceRequest {
    pub(super) request: ReviewBackendRequest,
    pub(super) token: ReviewBackendRequestToken,
}

impl ReviewBackendRequest {
    pub(super) fn to_parts(&self) -> BackendRequestParts {
        match self {
            Self::BackendHealth => get("/health"),
            Self::ReviewPrepare(body) => post("/api/review/prepare", body.clone()),
            Self::ReviewGenerate(body) => post("/api/review/generate", body.clone()),
            Self::ReviewCreateSession(body) => post("/api/review/sessions", body.clone()),
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
                body.clone(),
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
                body.clone(),
            ),
            Self::ReviewClearTemplateMatch {
                session_id,
                card_id,
            } => BackendRequestParts {
                method: BackendHttpMethod::Post,
                path: format!(
                    "/api/review/sessions/{}/cards/{}/template-clear",
                    encode_path_segment(session_id),
                    encode_path_segment(card_id)
                ),
                body: None,
                audio_tracks: Vec::new(),
            },
            Self::ReviewGenerateTemplateSuggestions { session_id } => BackendRequestParts {
                method: BackendHttpMethod::Post,
                path: format!(
                    "/api/review/sessions/{}/template-suggestions",
                    encode_path_segment(session_id)
                ),
                body: None,
                audio_tracks: Vec::new(),
            },
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
                body.clone(),
            ),
            Self::ReviewFinalizeSession { session_id, body } => post(
                format!(
                    "/api/review/sessions/{}/finalize",
                    encode_path_segment(session_id)
                ),
                body.clone(),
            ),
            Self::ReviewSubmitAnalytics(body) => post(
                "/api/trpc/transcriptions.submitTranscriptReviewAction",
                body.clone(),
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
            Self::DraftGenerate(body) => post("/api/draft/generate", body.clone()),
            Self::DraftGenerateWithAudio {
                payload,
                audio_tracks,
            } => post_with_audio("/api/draft/generate", payload.clone(), audio_tracks.clone()),
            Self::DraftGenerateStream(body) => post("/api/draft/generate/stream", body.clone()),
            Self::DraftGenerateStreamWithAudio {
                payload,
                audio_tracks,
            } => post_with_audio(
                "/api/draft/generate/stream",
                payload.clone(),
                audio_tracks.clone(),
            ),
            Self::DraftBrokerTranscribeSegment(body) => {
                post("/api/broker/transcribe-segment", body.clone())
            }
            Self::DraftBrokerTranscribeSegmentWithAudio {
                payload,
                audio_tracks,
            } => post_with_audio(
                "/api/broker/transcribe-segment",
                payload.clone(),
                audio_tracks.clone(),
            ),
            Self::DraftBrokerRedistributeText(body) => {
                post("/api/broker/redistribute-text", body.clone())
            }
        }
    }
}

pub(super) fn dispatch_review_backend_requests(
    requests: Vec<ReviewBackendServiceRequest>,
) -> Task<Message> {
    Task::batch(
        requests.into_iter().map(|request| {
            Task::perform(execute_review_backend_request(request), |message| message)
        }),
    )
}

async fn execute_review_backend_request(request: ReviewBackendServiceRequest) -> Message {
    match review_backend_payload_for_request(&request.request).await {
        Ok(payload) => Message::ReviewBackendRequestSucceeded(request, payload),
        Err(error) => Message::ReviewBackendRequestFailed(request, error),
    }
}

async fn review_backend_payload_for_request(
    request: &ReviewBackendRequest,
) -> Result<String, String> {
    let base_url = review_backend_base_url_from_env();
    let parts = request.to_parts();
    let url = review_backend_url(&base_url, &parts.path);
    let stream_request = is_draft_stream_request(request);
    let client = reqwest::Client::new();
    let builder = match parts.method {
        BackendHttpMethod::Get => client.get(&url),
        BackendHttpMethod::Post => client.post(&url),
    }
    .header(
        ACCEPT,
        if stream_request {
            "text/event-stream"
        } else {
            "application/json"
        },
    );

    let builder = if !parts.audio_tracks.is_empty() {
        let payload = parts.body.unwrap_or(Value::Null);
        let mut form = reqwest::multipart::Form::new().text("payload", payload.to_string());
        for track in parts.audio_tracks {
            let meta_name = format!("audioTrackMeta:{}", track.track_id);
            let audio_name = format!("audioTrack:{}", track.track_id);
            let part = reqwest::multipart::Part::bytes(track.bytes)
                .file_name(track.file_name)
                .mime_str(&track.mime_type)
                .map_err(|error| format!("failed to attach broker audio: {error}"))?;
            form = form
                .text(meta_name, track.metadata.to_string())
                .part(audio_name, part);
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
        .map_err(|error| format!("review backend request failed: {error}"))?;
    let status = response.status();
    let payload = response
        .text()
        .await
        .map_err(|error| format!("review backend response read failed: {error}"))?;
    if status.is_success() {
        if stream_request && !payload.trim_start().starts_with('{') {
            final_payload_from_sse_stream(&payload)
        } else {
            Ok(payload)
        }
    } else {
        Err(format!(
            "review backend returned HTTP {}: {}",
            status.as_u16(),
            payload
        ))
    }
}

fn is_draft_stream_request(request: &ReviewBackendRequest) -> bool {
    matches!(
        request,
        ReviewBackendRequest::DraftGenerateStream(_)
            | ReviewBackendRequest::DraftGenerateStreamWithAudio { .. }
    )
}

fn final_payload_from_sse_stream(stream: &str) -> Result<String, String> {
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
                continue;
            }
            if let Some(value) = line.strip_prefix("data:") {
                data_lines.push(value.trim_start());
            }
        }
        if data_lines.is_empty() {
            continue;
        }
        let payload = data_lines.join("\n");
        if event_name == "error" {
            return Err(format!("draft stream returned error event: {payload}"));
        }
        if event_name == "done" {
            final_payload = Some(payload);
        }
    }
    final_payload.ok_or_else(|| "draft stream finished without a done event".to_string())
}

pub(super) fn review_backend_base_url_from_env() -> String {
    std::env::var(REVIEW_BACKEND_BASE_URL_ENV)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REVIEW_BACKEND_BASE_URL.to_string())
}

pub(super) fn review_backend_url(base_url: &str, path: &str) -> String {
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

fn post(path: impl Into<String>, body: Value) -> BackendRequestParts {
    BackendRequestParts {
        method: BackendHttpMethod::Post,
        path: path.into(),
        body: Some(body),
        audio_tracks: Vec::new(),
    }
}

fn post_with_audio(
    path: impl Into<String>,
    body: Value,
    audio_tracks: Vec<BackendMultipartAudioTrack>,
) -> BackendRequestParts {
    BackendRequestParts {
        method: BackendHttpMethod::Post,
        path: path.into(),
        body: Some(body),
        audio_tracks,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        BackendHttpMethod, ReviewBackendRequest, final_payload_from_sse_stream, review_backend_url,
    };

    #[test]
    fn maps_reviewer_session_and_template_routes_to_backend_contract() {
        let parts = ReviewBackendRequest::ReviewSetTemplateMatch {
            session_id: "session/one".to_string(),
            card_id: "card A".to_string(),
            body: json!({ "templateId": "tpl-1" }),
        }
        .to_parts();

        assert_eq!(parts.method, BackendHttpMethod::Post);
        assert_eq!(
            parts.path,
            "/api/review/sessions/session%2Fone/cards/card%20A/template-match"
        );
        assert_eq!(parts.body, Some(json!({ "templateId": "tpl-1" })));

        let search = ReviewBackendRequest::ReviewSearchTemplates {
            query: "tone issue".to_string(),
            limit: Some(7),
        }
        .to_parts();
        assert_eq!(search.method, BackendHttpMethod::Get);
        assert_eq!(
            search.path,
            "/api/review/templates/search?q=tone+issue&limit=7"
        );
        assert_eq!(search.body, None);
    }

    #[test]
    fn maps_all_reviewer_mutation_endpoints_from_extension_client() {
        let cases = [
            (
                ReviewBackendRequest::ReviewPrepare(json!({ "reviewActionId": "ra" })),
                BackendHttpMethod::Post,
                "/api/review/prepare",
                true,
            ),
            (
                ReviewBackendRequest::ReviewGenerate(json!({ "reviewActionId": "ra" })),
                BackendHttpMethod::Post,
                "/api/review/generate",
                true,
            ),
            (
                ReviewBackendRequest::ReviewCreateSession(json!({ "reviewActionId": "ra" })),
                BackendHttpMethod::Post,
                "/api/review/sessions",
                true,
            ),
            (
                ReviewBackendRequest::ReviewGetSession {
                    session_id: "s1".to_string(),
                },
                BackendHttpMethod::Get,
                "/api/review/sessions/s1",
                false,
            ),
            (
                ReviewBackendRequest::ReviewUpdateComments {
                    session_id: "s1".to_string(),
                    body: json!({ "sessionComment": "ok", "cardComments": {} }),
                },
                BackendHttpMethod::Post,
                "/api/review/sessions/s1/comments",
                true,
            ),
            (
                ReviewBackendRequest::ReviewClearTemplateMatch {
                    session_id: "s1".to_string(),
                    card_id: "card".to_string(),
                },
                BackendHttpMethod::Post,
                "/api/review/sessions/s1/cards/card/template-clear",
                false,
            ),
            (
                ReviewBackendRequest::ReviewGenerateTemplateSuggestions {
                    session_id: "s1".to_string(),
                },
                BackendHttpMethod::Post,
                "/api/review/sessions/s1/template-suggestions",
                false,
            ),
            (
                ReviewBackendRequest::ReviewDecideTemplateSuggestion {
                    session_id: "s1".to_string(),
                    proposal_id: "p1".to_string(),
                    body: json!({ "decision": "accepted" }),
                },
                BackendHttpMethod::Post,
                "/api/review/sessions/s1/template-suggestions/p1/decision",
                true,
            ),
            (
                ReviewBackendRequest::ReviewFinalizeSession {
                    session_id: "s1".to_string(),
                    body: json!({ "mode": "apply" }),
                },
                BackendHttpMethod::Post,
                "/api/review/sessions/s1/finalize",
                true,
            ),
            (
                ReviewBackendRequest::ReviewHistory {
                    query: vec![("limit".to_string(), "10".to_string())],
                },
                BackendHttpMethod::Get,
                "/api/review-history?limit=10",
                false,
            ),
            (
                ReviewBackendRequest::ReviewHistoryItem {
                    history_id: "h1".to_string(),
                },
                BackendHttpMethod::Get,
                "/api/review-history/h1",
                false,
            ),
        ];

        for (request, method, path, has_body) in cases {
            let parts = request.to_parts();
            assert_eq!(parts.method, method);
            assert_eq!(parts.path, path);
            assert_eq!(parts.body.is_some(), has_body);
        }
    }

    #[test]
    fn maps_gold_drafting_backend_routes_from_extension_client() {
        let cases = [
            (
                ReviewBackendRequest::DraftGenerate(json!({ "jobId": "job" })),
                "/api/draft/generate",
            ),
            (
                ReviewBackendRequest::DraftGenerateStream(json!({ "jobId": "job" })),
                "/api/draft/generate/stream",
            ),
            (
                ReviewBackendRequest::DraftBrokerTranscribeSegment(json!({ "segmentId": "s" })),
                "/api/broker/transcribe-segment",
            ),
            (
                ReviewBackendRequest::DraftBrokerRedistributeText(json!({ "rows": [] })),
                "/api/broker/redistribute-text",
            ),
        ];

        for (request, path) in cases {
            let parts = request.to_parts();
            assert_eq!(parts.method, BackendHttpMethod::Post);
            assert_eq!(parts.path, path);
            assert!(parts.body.is_some());
        }
    }

    #[test]
    fn extracts_final_gold_drafting_payload_from_recovered_sse_stream() {
        let stream = concat!(
            "event: started\n",
            "data: {\"totalRows\":2}\n",
            "\n",
            "event: row\n",
            "data: {\"row\":{\"rowId\":\"r1\"},\"completedRows\":1,\"totalRows\":2}\n",
            "\n",
            "event: done\n",
            "data: {\"draftRows\":[{\"rowId\":\"r1\",\"rewrittenText\":\"Gold\",\"status\":\"rewritten\",\"warnings\":[]}],\"summary\":{\"totalRows\":1}}\n",
            "\n"
        );

        assert_eq!(
            final_payload_from_sse_stream(stream).expect("done payload should parse"),
            "{\"draftRows\":[{\"rowId\":\"r1\",\"rewrittenText\":\"Gold\",\"status\":\"rewritten\",\"warnings\":[]}],\"summary\":{\"totalRows\":1}}"
        );
    }

    #[test]
    fn builds_absolute_review_backend_urls_without_double_slashes() {
        assert_eq!(
            review_backend_url("https://reviewgen.ovh/", "/api/review/generate"),
            "https://reviewgen.ovh/api/review/generate"
        );
    }
}
