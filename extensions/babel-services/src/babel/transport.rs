//! Where Babel requests go: offline fixture, external command bridge,
//! sub-account relay, or the live Babel tRPC API.

use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue};
use serde_json::{Value, json};

use super::dashboard::{
    attach_project_configs, dashboard_project_ids, project_configs_from_queues,
    project_queues_envelope,
};
use super::request::{BabelRequest, ResponseShape, is_query_operation};
use super::response::{first_string_by_key, normalize_for_shape};
use crate::auth;
use studio_services::http;

const ENV_FIXTURE_JSON: &str = "STUDIO_BABEL_SERVICE_FIXTURE_JSON";
const ENV_FIXTURE_PATH: &str = "STUDIO_BABEL_SERVICE_FIXTURE_PATH";
const ENV_COMMAND: &str = "STUDIO_BABEL_SERVICE_COMMAND";
const ENV_COOKIE: &str = "STUDIO_BABEL_SERVICE_COOKIE";
const ENV_AUTHORIZATION: &str = "STUDIO_BABEL_SERVICE_AUTHORIZATION";
const ENV_AUTH_COMMAND: &str = "STUDIO_BABEL_SERVICE_AUTH_COMMAND";

/// Babel tRPC query that presigns a review action's track audio.
pub(crate) const PRESIGNED_CHUNK_URLS_OPERATION: &str = "transcriptions.getPresignedChunkUrls";

/// A failed transport step. `signed_out` marks failures the user fixes by signing in again:
/// the native auth store has no usable Clerk session, or Babel answered HTTP 401.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TransportError {
    pub(super) message: String,
    pub(super) signed_out: bool,
}

impl TransportError {
    fn signed_out(message: String) -> Self {
        Self {
            message,
            signed_out: true,
        }
    }
}

impl From<String> for TransportError {
    fn from(message: String) -> Self {
        Self {
            message,
            signed_out: false,
        }
    }
}

/// Offline fixture returned verbatim for every request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fixture {
    Json(String),
    /// Read on every request.
    Path(PathBuf),
}

/// Transport configuration; see [`BabelConfig::from_env`] for the env mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BabelConfig {
    pub fixture: Option<Fixture>,
    /// Shell command line receiving the request envelope on stdin and printing the response.
    pub command: Option<String>,
    /// Babel dashboard base URL (no trailing `/`).
    pub base_url: String,
    /// Fixed `Cookie` header for live requests.
    pub cookie: Option<String>,
    /// Fixed `Authorization` header for live requests.
    pub authorization: Option<String>,
    /// Command printing `{"cookie"|"authorization": ...}` JSON or a raw cookie, run per request.
    pub auth_command: Option<String>,
    /// Native auth store (`auth.json` of the account). `None` with no other auth source
    /// means the client cannot authenticate: live requests report "signed out".
    pub auth_store: Option<PathBuf>,
    /// Per-account directory for caches (remote audio). `None` → a temp directory.
    pub data_dir: Option<PathBuf>,
    pub clerk_url: String,
    /// Explicit relay origin captured when the document/session is opened.
    /// `None` always means direct Babel, regardless of persisted subaccount login.
    pub relay_session: Option<crate::subaccounts::SubaccountSession>,
}

impl BabelConfig {
    /// Direct Babel configuration from `STUDIO_BABEL_SERVICE_FIXTURE_JSON` /
    /// `_FIXTURE_PATH`, `_COMMAND`, `_BASE_URL`, `_COOKIE`, `_AUTHORIZATION`,
    /// `_AUTH_COMMAND`, and `STUDIO_BABEL_CLERK_URL`.
    pub fn from_env() -> Self {
        let fixture = std::env::var(ENV_FIXTURE_JSON)
            .ok()
            .filter(|source| !source.trim().is_empty())
            .map(Fixture::Json)
            .or_else(|| env_nonblank(ENV_FIXTURE_PATH).map(|path| Fixture::Path(path.into())));
        Self {
            fixture,
            command: env_nonblank(ENV_COMMAND),
            base_url: auth::babel_base_url(),
            cookie: env_nonblank(ENV_COOKIE),
            authorization: env_nonblank(ENV_AUTHORIZATION),
            auth_command: env_nonblank(ENV_AUTH_COMMAND),
            auth_store: None,
            data_dir: None,
            clerk_url: auth::clerk_url(),
            relay_session: None,
        }
    }

    /// The env configuration bound to one account directory: cookies are read from its
    /// `auth.json` and caches live beside it.
    pub fn for_account(account_dir: &std::path::Path) -> Self {
        Self {
            auth_store: Some(auth::auth_store_path(account_dir)),
            data_dir: Some(account_dir.to_path_buf()),
            ..Self::from_env()
        }
    }

    /// A live-only config against `base_url` (no fixture/command/relay).
    pub fn live(base_url: impl Into<String>) -> Self {
        Self {
            fixture: None,
            command: None,
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            cookie: None,
            authorization: None,
            auth_command: None,
            auth_store: None,
            data_dir: None,
            clerk_url: auth::clerk_url(),
            relay_session: None,
        }
    }

    /// A relay-only document session. Never falls back to a direct Babel account.
    pub fn relay(session: crate::subaccounts::SubaccountSession) -> Self {
        let mut config = Self::live(session.host_url.clone());
        config.relay_session = Some(session);
        config
    }

    /// [`Self::relay`] with caches kept in the account directory.
    pub fn relay_in(
        session: crate::subaccounts::SubaccountSession,
        account_dir: &std::path::Path,
    ) -> Self {
        let mut config = Self::relay(session);
        config.data_dir = Some(account_dir.to_path_buf());
        config
    }
}

fn env_nonblank(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Raw (normalized) response payload for `request`, or the failure.
pub(super) async fn payload_for(
    config: &BabelConfig,
    request: &BabelRequest,
) -> Result<String, TransportError> {
    match &config.fixture {
        Some(Fixture::Json(source)) => return Ok(source.clone()),
        Some(Fixture::Path(path)) => {
            return std::fs::read_to_string(path).map_err(|error| {
                format!("failed to read native Babel service fixture: {error}").into()
            });
        }
        None => {}
    }

    let envelope = request.envelope();
    let shape = request.response_shape();
    if let Some(command_line) = config.command.clone() {
        let input = envelope.to_string();
        let response =
            tokio::task::spawn_blocking(move || run_service_command(&command_line, &input))
                .await
                .map_err(|error| format!("native Babel service command task failed: {error}"))??;
        return Ok(normalize_for_shape(shape, &response));
    }

    if let Some(session) = &config.relay_session {
        let payload =
            crate::subaccounts::relay_babel_payload(session, request.relay_kind(), envelope)
                .await
                .map_err(|error| format!("{error:#}"))?;
        return Ok(normalize_for_shape(shape, &payload));
    }

    let auth = live_auth_headers(config).await?;
    let parts = LiveRequest::new(&envelope, &config.base_url, auth)?;
    let payload = parts.execute().await?;
    let payload = normalize_for_shape(shape, &payload);
    if shape == ResponseShape::Dashboard {
        let configs = project_queue_configs(&payload, &config.base_url, &parts).await?;
        return Ok(attach_project_configs(&payload, configs));
    }
    Ok(payload)
}

/// Raw `transcriptions.getPresignedChunkUrls` response (`{ <chunkedProcessedRecordingId>:
/// <https url> }`) for a claimed review action, from the live API with this config's auth.
/// This is what Babel's own editor calls to play a task's tracks.
pub(crate) async fn presigned_chunk_urls_payload(
    config: &BabelConfig,
    review_action_id: &str,
    chunked_processed_recording_ids: &[String],
) -> Result<String, String> {
    let envelope = json!({
        "operation": PRESIGNED_CHUNK_URLS_OPERATION,
        "payload": {
            "reviewActionId": review_action_id,
            "chunkedProcessedRecordingIds": chunked_processed_recording_ids,
        },
    });
    let auth = live_auth_headers(config)
        .await
        .map_err(|error| error.message)?;
    LiveRequest::new(&envelope, &config.base_url, auth)?
        .execute()
        .await
        .map_err(|error| error.message)
}

/// Runs the bridge command with `input` on stdin; non-zero exit → stderr (or status) as error.
fn run_service_command(command_line: &str, input: &str) -> Result<String, String> {
    let mut child = shell_command(command_line)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start native Babel service command: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| format!("failed to write native Babel service request: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to read native Babel service response: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!(
                "native Babel service command exited with status {}",
                output.status
            )
        } else {
            stderr
        });
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("native Babel service response was not UTF-8: {error}"))
}

#[cfg(windows)]
fn shell_command(command_line: &str) -> Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("cmd");
    command
        .args(["/D", "/S", "/C"])
        .raw_arg(format!("\"{command_line}\""));
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn shell_command(command_line: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(command_line);
    command
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct LiveAuth {
    pub(super) cookie: Option<String>,
    pub(super) authorization: Option<String>,
}

impl LiveAuth {
    fn is_empty(&self) -> bool {
        self.cookie.as_deref().is_none_or(str::is_empty)
            && self.authorization.as_deref().is_none_or(str::is_empty)
    }
}

/// Auth command output, then fixed cookie/authorization overrides; when still
/// empty, a `Bearer` JWT minted from the native auth store.
async fn live_auth_headers(config: &BabelConfig) -> Result<LiveAuth, TransportError> {
    let mut auth = match config.auth_command.clone() {
        Some(command_line) => tokio::task::spawn_blocking(move || {
            let output = shell_command(&command_line)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .map_err(|error| format!("failed to run Babel live auth command: {error}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if stderr.is_empty() {
                    format!(
                        "Babel live auth command exited with status {}",
                        output.status
                    )
                } else {
                    stderr
                });
            }
            String::from_utf8(output.stdout)
                .map(|stdout| parse_auth_command_output(&stdout))
                .map_err(|error| format!("Babel live auth command output was not UTF-8: {error}"))
        })
        .await
        .map_err(|error| format!("Babel live auth command task failed: {error}"))??,
        None => LiveAuth::default(),
    };
    if let Some(cookie) = &config.cookie {
        auth.cookie = Some(cookie.clone());
    }
    if let Some(authorization) = &config.authorization {
        auth.authorization = Some(authorization.clone());
    }
    if !auth.is_empty() {
        return Ok(auth);
    }

    let Some(store_path) = config.auth_store.as_deref() else {
        return Err(TransportError::signed_out(auth_configuration_error(
            "no Babel account is attached to this client; sign in first",
        )));
    };
    let authorization = auth::mint_babel_authorization(
        store_path,
        &config.base_url,
        &config.clerk_url,
        time::OffsetDateTime::now_utc().unix_timestamp(),
    )
    .await
    .map_err(|error| {
        let message = auth_configuration_error(&format!("{error:#}"));
        if error.downcast_ref::<auth::NeedsSignIn>().is_some() {
            TransportError::signed_out(message)
        } else {
            message.into()
        }
    })?;
    auth.authorization = Some(authorization);
    Ok(auth)
}

fn auth_configuration_error(native_error: &str) -> String {
    format!(
        "Babel live service auth is not configured from native store: {native_error}; set {ENV_COOKIE}, {ENV_AUTHORIZATION}, {ENV_AUTH_COMMAND}, or complete native Babel auth with `studio-app --babel-auth-verify-login <code>`"
    )
}

pub(super) fn parse_auth_command_output(output: &str) -> LiveAuth {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return LiveAuth::default();
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return LiveAuth {
            cookie: Some(trimmed.to_string()),
            authorization: None,
        };
    };
    let field = |keys: &[&str]| {
        first_string_by_key(&value, keys)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    LiveAuth {
        cookie: field(&["cookie", "cookieHeader", "Cookie"]),
        authorization: field(&["authorization", "authorizationHeader", "Authorization"]),
    }
}

/// A resolved live tRPC request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LiveRequest {
    pub(super) is_get: bool,
    pub(super) url: String,
    pub(super) body: Option<String>,
    pub(super) auth: LiveAuth,
}

impl LiveRequest {
    /// `GET <base>/api/trpc/<op>?batch=1&input=<batch>` for queries,
    /// `POST <base>/api/trpc/<op>?batch=1` with the batch body otherwise.
    pub(super) fn new(envelope: &Value, base_url: &str, auth: LiveAuth) -> Result<Self, String> {
        let operation = envelope
            .get("operation")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Babel live service request is missing operation".to_string())?;
        if operation.contains(['?', '#']) {
            return Err("Babel live service operation contains invalid URL characters".to_string());
        }
        let base_url = base_url.trim().trim_end_matches('/');
        if base_url.is_empty() {
            return Err("Babel live service base URL is empty".to_string());
        }
        let batch = envelope
            .get("batchInput")
            .cloned()
            .unwrap_or_else(|| {
                json!({ "0": { "json": envelope.get("payload").cloned().unwrap_or(Value::Null) } })
            })
            .to_string();
        let is_get = is_query_operation(operation);
        let (url, body) = if is_get {
            let encoded: String = url::form_urlencoded::byte_serialize(batch.as_bytes()).collect();
            (
                format!("{base_url}/api/trpc/{operation}?batch=1&input={encoded}"),
                None,
            )
        } else {
            (
                format!("{base_url}/api/trpc/{operation}?batch=1"),
                Some(batch),
            )
        };
        Ok(Self {
            is_get,
            url,
            body,
            auth,
        })
    }

    fn auth_headers(&self, accept: &'static str) -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static(accept));
        let nonblank = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        if let Some(cookie) = nonblank(&self.auth.cookie) {
            headers.insert(
                COOKIE,
                HeaderValue::from_str(&cookie)
                    .map_err(|error| format!("Babel live cookie header is invalid: {error}"))?,
            );
        }
        if let Some(authorization) = nonblank(&self.auth.authorization) {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&authorization).map_err(|error| {
                    format!("Babel live authorization header is invalid: {error}")
                })?,
            );
        }
        Ok(headers)
    }

    async fn execute(&self) -> Result<String, TransportError> {
        let mut headers = self.auth_headers("application/json")?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let client = http::client();
        let request = if self.is_get {
            client.get(&self.url)
        } else {
            client
                .post(&self.url)
                .body(self.body.clone().unwrap_or_default())
        };
        let response = request
            .headers(headers)
            .send()
            .await
            .map_err(|error| format!("Babel live service request failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("failed to read Babel live service response: {error}"))?;
        if !status.is_success() {
            let message = format!(
                "Babel live service HTTP {status}: {}",
                http::body_snippet(&body)
            );
            return Err(if status == reqwest::StatusCode::UNAUTHORIZED {
                TransportError::signed_out(message)
            } else {
                message.into()
            });
        }
        Ok(body)
    }
}

/// Queue ids for every dashboard project from one batched `getProjectQueues` query (the
/// project list itself carries none). Projects without review queues are skipped.
async fn project_queue_configs(
    payload: &str,
    base_url: &str,
    live: &LiveRequest,
) -> Result<Vec<Value>, TransportError> {
    let project_ids = dashboard_project_ids(payload);
    let Some(envelope) = project_queues_envelope(&project_ids) else {
        return Ok(Vec::new());
    };
    let response = LiveRequest::new(&envelope, base_url, live.auth.clone())?
        .execute()
        .await?;
    Ok(project_configs_from_queues(&project_ids, &response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::babel::payload::SaveAnnotationsRequest;

    fn auth() -> LiveAuth {
        LiveAuth {
            cookie: Some("babel_session=super-secret".to_string()),
            authorization: None,
        }
    }

    #[test]
    fn live_request_matches_babel_batch_post_contract() {
        let envelope = json!({
            "operation": "transcriptions.saveAnnotationsByReviewActionId",
            "payload": {
                "reviewActionId": "review-action-42",
                "annotations": [{ "id": "annotation-1" }],
            },
        });
        let live = LiveRequest::new(&envelope, "https://dashboard.babel.audio/", auth())
            .expect("live request");
        assert!(!live.is_get);
        assert_eq!(
            live.url,
            "https://dashboard.babel.audio/api/trpc/transcriptions.saveAnnotationsByReviewActionId?batch=1"
        );
        assert_eq!(
            serde_json::from_str::<Value>(live.body.as_deref().expect("body")).expect("json"),
            json!({ "0": { "json": {
                "reviewActionId": "review-action-42",
                "annotations": [{ "id": "annotation-1" }],
            } } })
        );
    }

    #[test]
    fn live_request_uses_get_for_dashboard_batch() {
        let live = LiveRequest::new(
            &BabelRequest::LoadWorkerProjects.envelope(),
            "https://dashboard.babel.audio/",
            auth(),
        )
        .expect("live request");
        assert!(live.is_get);
        assert!(live.body.is_none());
        assert!(live.url.starts_with(
            "https://dashboard.babel.audio/api/trpc/worker.get,worker.getProjects?batch=1&input="
        ));
    }

    #[test]
    fn every_operation_routes_to_the_expected_http_method() {
        let cases = [
            (BabelRequest::LoadWorkerProjects, true),
            (
                BabelRequest::SaveAnnotations(SaveAnnotationsRequest {
                    review_action_id: "r".to_string(),
                    annotations: vec![json!({ "id": "annotation-1" })],
                }),
                false,
            ),
            (
                BabelRequest::DropWorkerAction {
                    queue_id: "q".to_string(),
                    review_action_id: "r".to_string(),
                },
                false,
            ),
            (
                BabelRequest::ClaimNextFromQueue {
                    queue_id: "q".to_string(),
                },
                false,
            ),
            (
                BabelRequest::ClaimByProcessedTranscription {
                    processed_transcription_id: "pt".to_string(),
                    queue_id: "q".to_string(),
                },
                false,
            ),
            (
                BabelRequest::LoadReviewAction {
                    review_action_id: "r".to_string(),
                },
                true,
            ),
            (
                BabelRequest::LoadAnnotationsByReviewAction {
                    review_action_id: "r".to_string(),
                },
                true,
            ),
            (
                BabelRequest::LoadLatestReviewAction {
                    processed_transcription_id: "pt".to_string(),
                    queue_id: "q".to_string(),
                },
                true,
            ),
            (
                BabelRequest::LoadFeedbackRubricFlags {
                    project_id: "p".to_string(),
                },
                true,
            ),
            (
                BabelRequest::LoadFeedbackForm {
                    project_id: "p".to_string(),
                },
                true,
            ),
            (
                BabelRequest::LoadFeedbackInputs {
                    step_id: "s".to_string(),
                },
                true,
            ),
            (
                BabelRequest::LoadFeedbackDraft {
                    form_step_id: "s".to_string(),
                    review_action_id: "r".to_string(),
                    worker_id: "w".to_string(),
                },
                false,
            ),
            (
                BabelRequest::ClaimTranscriptionErrorQaTask {
                    transcription_error_diff_id: "d".to_string(),
                    queue_id: "q".to_string(),
                },
                false,
            ),
            (
                BabelRequest::SubmitTranscriptionErrorLabel(crate::babel::error_task::submission(
                    "r",
                    &crate::babel::error_task::ErrorChoices::default(),
                    crate::babel::error_task::ErrorSource::Real,
                )),
                false,
            ),
            (
                BabelRequest::LoadProjectDocuments {
                    project_id: "p".to_string(),
                },
                true,
            ),
            (
                BabelRequest::LoadProjectDocument {
                    project_id: "p".to_string(),
                    document_id: "d".to_string(),
                },
                true,
            ),
        ];
        for (request, expect_get) in cases {
            let envelope = request.envelope();
            let operation = envelope["operation"].as_str().expect("operation");
            let live = LiveRequest::new(&envelope, "https://dashboard.babel.audio", auth())
                .expect("live request");
            assert_eq!(live.is_get, expect_get, "{operation}");
            assert_eq!(live.body.is_none(), expect_get, "{operation}");
            assert!(live.url.contains(operation.split(',').next().unwrap()));
        }
        assert!(is_query_operation(PRESIGNED_CHUNK_URLS_OPERATION));
    }

    #[test]
    fn auth_command_output_accepts_json_or_raw_cookie() {
        assert_eq!(
            parse_auth_command_output(r#"{"cookieHeader":" c=1 ","Authorization":"Bearer x"}"#),
            LiveAuth {
                cookie: Some("c=1".to_string()),
                authorization: Some("Bearer x".to_string()),
            }
        );
        assert_eq!(
            parse_auth_command_output("session=abc\n"),
            LiveAuth {
                cookie: Some("session=abc".to_string()),
                authorization: None,
            }
        );
        assert_eq!(parse_auth_command_output("  "), LiveAuth::default());
    }
}
