use iced::Task;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::Value;
use url::form_urlencoded::byte_serialize;

use super::{Message, subaccounts::subaccount_relay_review_backend_payload_for_request};

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
    let parts = request.to_parts();
    let stream_request = is_draft_stream_request(request);

    if let Some(payload) = subaccount_relay_review_backend_payload_for_request(&parts).await? {
        return finalize_review_backend_payload(payload, stream_request);
    }

    let base_url = review_backend_base_url_from_env();
    let url = review_backend_url(&base_url, &parts.path);
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
        finalize_review_backend_payload(payload, stream_request)
    } else {
        Err(format!(
            "review backend returned HTTP {}: {}",
            status.as_u16(),
            payload
        ))
    }
}

fn finalize_review_backend_payload(
    payload: String,
    stream_request: bool,
) -> Result<String, String> {
    if stream_request && !payload.trim_start().starts_with('{') {
        final_payload_from_sse_stream(&payload)
    } else {
        Ok(payload)
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
    use std::{
        env,
        ffi::{OsStr, OsString},
        fs,
        io::{Read, Write as IoWrite},
        net::TcpListener,
        sync::MutexGuard,
        thread,
    };

    use serde_json::json;

    use super::{
        BackendHttpMethod, REVIEW_BACKEND_BASE_URL_ENV, ReviewBackendRequest,
        final_payload_from_sse_stream, review_backend_payload_for_request, review_backend_url,
    };

    struct MockHttpResponse {
        body: String,
        headers: Vec<String>,
    }

    struct MockHttpServer {
        base_url: String,
        handle: thread::JoinHandle<Vec<String>>,
    }

    impl MockHttpServer {
        fn start(responses: Vec<MockHttpResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
            let handle = thread::spawn(move || {
                let mut requests = Vec::new();
                for response in responses {
                    let (mut stream, _) = listener.accept().expect("accept request");
                    requests.push(read_http_request(&mut stream));
                    write_http_response(&mut stream, &response.body, &response.headers);
                }
                requests
            });
            Self { base_url, handle }
        }

        fn start_simple(responses: Vec<&str>) -> Self {
            Self::start(
                responses
                    .into_iter()
                    .map(|body| MockHttpResponse {
                        body: body.to_string(),
                        headers: Vec::new(),
                    })
                    .collect(),
            )
        }

        fn join(self) -> Vec<String> {
            self.handle.join().expect("mock server join")
        }
    }

    struct TestEnv {
        _guard: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl TestEnv {
        fn with_clean_review_backend_env() -> Self {
            let guard = super::super::subaccounts::SUBACCOUNT_SESSION_ENV_LOCK
                .lock()
                .expect("subaccount env lock");
            let names = vec![REVIEW_BACKEND_BASE_URL_ENV, "STUDIO_SUBACCOUNT_SESSION_STORE"];
            let saved = names
                .iter()
                .map(|name| (*name, env::var_os(name)))
                .collect::<Vec<_>>();
            for (name, _) in &saved {
                remove_env_var(name);
            }
            Self {
                _guard: guard,
                saved,
            }
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                match value {
                    Some(value) => set_env_var(name, value),
                    None => remove_env_var(name),
                }
            }
        }
    }

    fn set_env_var(name: &str, value: impl AsRef<OsStr>) {
        unsafe {
            env::set_var(name, value);
        }
    }

    fn remove_env_var(name: &str) {
        unsafe {
            env::remove_var(name);
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let read = stream.read(&mut chunk).expect("read request");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            let text = String::from_utf8_lossy(&buffer);
            if let Some(header_end) = text.find("\r\n\r\n") {
                let headers = &text[..header_end];
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length: ")
                            .or_else(|| line.strip_prefix("Content-Length: "))
                    })
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if buffer.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8(buffer).expect("utf8 request")
    }

    fn write_http_response(stream: &mut std::net::TcpStream, body: &str, headers: &[String]) {
        let extra_headers = if headers.is_empty() {
            String::new()
        } else {
            format!("{}\r\n", headers.join("\r\n"))
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\n{}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            extra_headers,
            body.len(),
            body
        );
    }

    fn request_contains(request: &str, expected: &str) -> bool {
        request
            .to_ascii_lowercase()
            .contains(&expected.to_ascii_lowercase())
    }

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

    #[tokio::test]
    async fn review_backend_request_for_subaccount_routes_through_relay() {
        let temp = tempfile::tempdir().expect("temp dir");
        let _env = TestEnv::with_clean_review_backend_env();
        let relay = MockHttpServer::start_simple(vec![r#"{"payload":{"result":"ok"}}"#]);

        let subaccount_session_path = temp.path().join("subaccount-session.json");
        fs::write(
            &subaccount_session_path,
            format!(
                r#"{{"version":1,"hostUrl":"{}","token":"relay-token","subaccountId":"sub-1","displayName":"Field Worker"}}"#,
                relay.base_url
            ),
        )
        .expect("write subaccount session");
        set_env_var(
            "STUDIO_SUBACCOUNT_SESSION_STORE",
            subaccount_session_path.as_os_str(),
        );

        let payload = review_backend_payload_for_request(&ReviewBackendRequest::ReviewGetSession {
            session_id: "session-1".to_string(),
        })
        .await
        .expect("review backend request should route through the sub-account relay");

        assert_eq!(payload, r#"{"result":"ok"}"#);

        let relay_requests = relay.join();
        assert!(request_contains(
            &relay_requests[0],
            "POST /api/subaccounts/babel-service"
        ));
        assert!(request_contains(
            &relay_requests[0],
            "Authorization: Bearer relay-token"
        ));
        assert!(request_contains(&relay_requests[0], "reviewBackend"));
        assert!(request_contains(
            &relay_requests[0],
            "/api/review/sessions/session-1"
        ));
    }
}
