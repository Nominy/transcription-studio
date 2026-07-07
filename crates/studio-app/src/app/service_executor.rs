use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    process::{Command, Stdio},
};

use iced::Task;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::Message;
use super::native_auth::{
    default_native_auth_store_path, mint_native_babel_authorization_from_store, native_clerk_url,
};
use super::service_callbacks::{
    AdminRateReviewActionRequest, BabelServiceRequest, ClaimByProcessedTranscriptionRequest,
    ClaimNextFromQueueRequest, DropWorkerActionRequest, LoadLatestReviewActionRequest,
    LoadReviewActionRequest, SaveAnnotationsRequest, SkipReviewActionRequest,
    SubmitReviewActionRequest,
};
use super::subaccounts::{subaccount_relay_payload_for_request, subaccount_session_is_active};

const STUDIO_BABEL_SERVICE_FIXTURE_JSON: &str = "STUDIO_BABEL_SERVICE_FIXTURE_JSON";
const STUDIO_BABEL_SERVICE_FIXTURE_PATH: &str = "STUDIO_BABEL_SERVICE_FIXTURE_PATH";
const STUDIO_BABEL_SERVICE_COMMAND: &str = "STUDIO_BABEL_SERVICE_COMMAND";
const STUDIO_BABEL_SERVICE_MODE: &str = "STUDIO_BABEL_SERVICE_MODE";
const STUDIO_BABEL_SERVICE_BASE_URL: &str = "STUDIO_BABEL_SERVICE_BASE_URL";
const STUDIO_BABEL_SERVICE_COOKIE: &str = "STUDIO_BABEL_SERVICE_COOKIE";
const STUDIO_BABEL_SERVICE_AUTHORIZATION: &str = "STUDIO_BABEL_SERVICE_AUTHORIZATION";
const STUDIO_BABEL_SERVICE_AUTH_COMMAND: &str = "STUDIO_BABEL_SERVICE_AUTH_COMMAND";
const DEFAULT_BABEL_SERVICE_BASE_URL: &str = "https://dashboard.babel.audio";
const BABEL_DASHBOARD_PROJECTS_OPERATIONS: &str = concat!(
    "worker.get,",
    "worker.queryWorkerMetadata,",
    "worker.getProjects,",
    "qa.isQaWorker,",
    "worker.getOnboardingProject,",
    "actionItems.getWorkerActionItems,",
    "conferencing.getMyCallCount,",
    "actionItems.getWorkerActionItems,",
    "milestones.getAllProjectMilestones,",
    "worker.getActiveQuests,",
    "timer.getActiveTemporaryRates"
);

pub(super) fn is_dispatchable_babel_service_request(request: &BabelServiceRequest) -> bool {
    matches!(
        request,
        BabelServiceRequest::LoadWorkerProjects
            | BabelServiceRequest::SaveAnnotations(_)
            | BabelServiceRequest::ClaimNextFromQueue(_)
            | BabelServiceRequest::ClaimByProcessedTranscription(_)
            | BabelServiceRequest::LoadReviewAction(_)
            | BabelServiceRequest::LoadLatestReviewAction(_)
            | BabelServiceRequest::SubmitReviewAction(_)
            | BabelServiceRequest::DropWorkerAction(_)
            | BabelServiceRequest::SkipReviewAction(_)
            | BabelServiceRequest::AdminRateReviewAction(_)
            | BabelServiceRequest::SaveFeedbackDraft(_)
    )
}

pub(super) fn dispatch_babel_service_requests(requests: Vec<BabelServiceRequest>) -> Task<Message> {
    Task::batch(
        requests
            .into_iter()
            .filter(is_dispatchable_babel_service_request)
            .map(|request| {
                Task::perform(execute_babel_service_request(request), |message| message)
            }),
    )
}

async fn execute_babel_service_request(request: BabelServiceRequest) -> Message {
    match service_payload_for_request(&request).await {
        Ok(payload) => fixture_payload_message_for_request(request.clone(), payload)
            .unwrap_or_else(|| {
                failure_message_for_request(request, "unsupported native service request")
            }),
        Err(error) => failure_message_for_request(request, &error),
    }
}

async fn service_payload_for_request(request: &BabelServiceRequest) -> Result<String, String> {
    if let Some(payload) = fixture_payload_for_request()? {
        return Ok(payload);
    }

    if let Some(payload) = command_payload_for_request(request)? {
        return Ok(payload);
    }

    let envelope = request_envelope_for_service_command(request);
    if let Some(payload) = subaccount_relay_payload_for_request(request, envelope).await? {
        return Ok(normalize_service_response_payload_for_request(
            request, &payload,
        ));
    }

    if let Some(payload) = live_payload_for_request(request).await? {
        return Ok(payload);
    }

    Err(format!(
        "native Babel service is not configured; set {STUDIO_BABEL_SERVICE_FIXTURE_PATH}, {STUDIO_BABEL_SERVICE_FIXTURE_JSON}, {STUDIO_BABEL_SERVICE_COMMAND}, or {STUDIO_BABEL_SERVICE_MODE}=live"
    ))
}

pub(super) fn fixture_payload_message_for_request(
    request: BabelServiceRequest,
    payload: String,
) -> Option<Message> {
    match request {
        BabelServiceRequest::LoadWorkerProjects => Some(Message::DashboardProjectsLoaded(payload)),
        BabelServiceRequest::SaveAnnotations(_) => {
            Some(save_annotations_message_from_payload(&payload))
        }
        BabelServiceRequest::LoadReviewAction(_) => {
            Some(Message::LoadReviewActionSucceeded(payload))
        }
        BabelServiceRequest::LoadLatestReviewAction(_) => {
            Some(Message::LoadLatestReviewActionSucceeded(payload))
        }
        BabelServiceRequest::ClaimByProcessedTranscription(_) => {
            Some(Message::ClaimByProcessedTranscriptionSucceeded(payload))
        }
        BabelServiceRequest::ClaimNextFromQueue(_) => {
            Some(Message::ClaimNextFromQueueSucceeded(payload))
        }
        BabelServiceRequest::SubmitReviewAction(_) => {
            Some(submit_review_message_from_payload(&payload))
        }
        BabelServiceRequest::DropWorkerAction(_) => Some(simple_mutation_message_from_payload(
            &payload,
            Message::DropClaimSucceeded,
            Message::DropClaimFailed,
            "Failed to drop claim. Please try again.",
        )),
        BabelServiceRequest::SkipReviewAction(_) => Some(simple_mutation_message_from_payload(
            &payload,
            Message::SkipChunkSucceeded,
            Message::SkipChunkFailed,
            "Please try again.",
        )),
        BabelServiceRequest::AdminRateReviewAction(_) => {
            Some(simple_mutation_message_from_payload(
                &payload,
                Message::AdminFlagSucceeded,
                Message::AdminFlagFailed,
                "Please try again.",
            ))
        }
        BabelServiceRequest::SaveFeedbackDraft(_) => {
            Some(feedback_draft_message_from_payload(&payload))
        }
    }
}

fn save_annotations_message_from_payload(payload: &str) -> Message {
    let value = match decode_service_payload(payload) {
        Ok(value) => value,
        Err(error) => return Message::SaveProgressFailed(error),
    };

    if response_success(&value) {
        return Message::SaveProgressSucceeded(
            first_string_by_key(
                &value,
                &["updatedAt", "updated_at", "lastSaved", "last_saved"],
            )
            .unwrap_or_else(current_timestamp),
        );
    }

    Message::SaveProgressFailed(response_error(
        &value,
        "Failed to save progress. Please try again.",
    ))
}

fn submit_review_message_from_payload(payload: &str) -> Message {
    let value = match decode_service_payload(payload) {
        Ok(value) => value,
        Err(error) => return Message::SubmitReviewFailed(error),
    };

    if linter_error_count(&value) > 0 {
        return Message::SubmitReviewFailed(
            "Failed to pass linting. Please fix the errors and try again.".to_string(),
        );
    }

    if response_success(&value) {
        Message::SubmitReviewSucceeded
    } else {
        Message::SubmitReviewFailed(response_error(
            &value,
            "Failed to submit annotations. Please try again.",
        ))
    }
}

pub(super) fn babel_remote_mutations_configured() -> bool {
    command_configured() || live_service_explicitly_enabled() || subaccount_session_is_active()
}

fn simple_mutation_message_from_payload(
    payload: &str,
    success_message: Message,
    failure_message: impl FnOnce(String) -> Message,
    fallback: &str,
) -> Message {
    match decode_service_payload(payload) {
        Ok(value) if response_success(&value) => success_message,
        Ok(value) => failure_message(response_error(&value, fallback)),
        Err(error) => failure_message(error),
    }
}

fn feedback_draft_message_from_payload(payload: &str) -> Message {
    let value = match decode_service_payload(payload) {
        Ok(value) => value,
        Err(error) => return Message::FeedbackDraftSaveFailed(error),
    };

    if !response_success(&value) {
        return Message::FeedbackDraftSaveFailed(response_error(
            &value,
            "Failed to save feedback draft. Please try again.",
        ));
    }

    Message::FeedbackDraftSaved(
        first_string_by_key(
            &value,
            &["updatedAt", "updated_at", "lastSaved", "last_saved"],
        )
        .unwrap_or_else(current_timestamp),
    )
}

fn decode_service_payload(payload: &str) -> Result<Value, String> {
    if payload.trim().is_empty() {
        return Ok(Value::Null);
    }

    serde_json::from_str(payload)
        .map_err(|error| format!("failed to decode native Babel service response: {error}"))
}

fn response_success(value: &Value) -> bool {
    first_bool_by_key(value, &["success", "ok"]).unwrap_or(false)
}

fn response_error(value: &Value, fallback: &str) -> String {
    first_string_by_key(value, &["error", "message"])
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn linter_error_count(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            if let Some(errors) = map.get("linterErrors").and_then(Value::as_array) {
                return errors.len();
            }
            map.values().map(linter_error_count).sum()
        }
        Value::Array(items) => items.iter().map(linter_error_count).sum(),
        _ => 0,
    }
}

fn first_bool_by_key(value: &Value, keys: &[&str]) -> Option<bool> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(Value::as_bool) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|child| first_bool_by_key(child, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| first_bool_by_key(child, keys)),
        _ => None,
    }
}

fn first_string_by_key(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(Value::as_str) {
                    return Some(found.to_string());
                }
            }
            map.values()
                .find_map(|child| first_string_by_key(child, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| first_string_by_key(child, keys)),
        _ => None,
    }
}

fn current_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "now".to_string())
}

fn fixture_payload_for_request() -> Result<Option<String>, String> {
    if let Ok(source) = env::var(STUDIO_BABEL_SERVICE_FIXTURE_JSON)
        && !source.trim().is_empty()
    {
        return Ok(Some(source));
    }

    if let Ok(path) = env::var(STUDIO_BABEL_SERVICE_FIXTURE_PATH)
        && !path.trim().is_empty()
    {
        return fs::read_to_string(path.trim())
            .map(Some)
            .map_err(|error| format!("failed to read native Babel service fixture: {error}"));
    }

    Ok(None)
}

fn command_payload_for_request(request: &BabelServiceRequest) -> Result<Option<String>, String> {
    let Some(command_line) = command_line_from_env(STUDIO_BABEL_SERVICE_COMMAND) else {
        return Ok(None);
    };
    command_payload_for_command_line(request, &command_line)
}

fn command_payload_for_command_line(
    request: &BabelServiceRequest,
    command_line: &str,
) -> Result<Option<String>, String> {
    let command_line = command_line.trim();
    if command_line.is_empty() {
        return Ok(None);
    }
    let envelope = request_envelope_for_service_command(request);
    if env::var_os("STUDIO_LOG_BABEL_SERVICE_DISPATCH").is_some() {
        let operation = envelope
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        eprintln!("Transcription Studio Babel service command: operation={operation}");
    }

    let mut child = service_command(command_line)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start native Babel service command: {error}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(envelope.to_string().as_bytes())
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

    let response = String::from_utf8(output.stdout)
        .map_err(|error| format!("native Babel service response was not UTF-8: {error}"))?;
    Ok(Some(normalize_service_response_payload_for_request(
        request, &response,
    )))
}

fn command_configured() -> bool {
    command_line_from_env(STUDIO_BABEL_SERVICE_COMMAND).is_some()
}

fn command_line_from_env(name: &str) -> Option<String> {
    let value = env::var(name).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

async fn live_payload_for_request(request: &BabelServiceRequest) -> Result<Option<String>, String> {
    if !live_service_configured() {
        return Ok(None);
    }

    let envelope = request_envelope_for_service_command(request);
    let auth = live_auth_headers_for_request().await?;
    let parts = live_trpc_request_parts(&envelope, &live_base_url(), auth)?;

    if env::var_os("STUDIO_LOG_BABEL_SERVICE_DISPATCH").is_some() {
        eprintln!(
            "Transcription Studio Babel live service: {}",
            parts.redacted_summary()
        );
    }

    let payload = execute_live_trpc_request(parts.clone()).await?;
    let mut payload = normalize_service_response_payload_for_request(request, &payload);
    if matches!(request, BabelServiceRequest::LoadWorkerProjects) {
        let configs = discover_live_dashboard_project_configs(&payload, &parts)
            .await
            .unwrap_or_else(|_| Vec::new());
        payload = attach_dashboard_project_configs(&payload, configs);
    }
    Ok(Some(payload))
}

fn live_service_configured() -> bool {
    true
}

fn live_service_explicitly_enabled() -> bool {
    env::var(STUDIO_BABEL_SERVICE_MODE)
        .ok()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("live"))
}

fn live_base_url() -> String {
    env::var(STUDIO_BABEL_SERVICE_BASE_URL)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BABEL_SERVICE_BASE_URL.to_string())
}

async fn live_auth_headers_for_request() -> Result<LiveAuthHeaders, String> {
    let mut headers = live_auth_headers_from_env()?;
    if headers.is_empty() {
        let store_path = default_native_auth_store_path();
        let authorization = mint_native_babel_authorization_from_store(
            &store_path,
            &live_base_url(),
            &native_clerk_url(),
            OffsetDateTime::now_utc().unix_timestamp(),
        )
        .await
        .map_err(|error| live_auth_configuration_error(Some(error)))?;
        headers.authorization = Some(authorization);
    }

    if headers.is_empty() {
        return Err(live_auth_configuration_error(None));
    }

    Ok(headers)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LiveAuthHeaders {
    cookie: Option<String>,
    authorization: Option<String>,
}

impl LiveAuthHeaders {
    fn is_empty(&self) -> bool {
        self.cookie.as_deref().is_none_or(str::is_empty)
            && self.authorization.as_deref().is_none_or(str::is_empty)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LiveTrpcMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveTrpcRequestParts {
    method: LiveTrpcMethod,
    operation: String,
    url: String,
    body: Option<String>,
    cookie_header: Option<String>,
    authorization_header: Option<String>,
}

impl LiveTrpcRequestParts {
    fn redacted_summary(&self) -> String {
        let mut auth = Vec::new();
        if self
            .cookie_header
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            auth.push("cookie");
        }
        if self
            .authorization_header
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            auth.push("authorization");
        }
        if auth.is_empty() {
            auth.push("none");
        }
        format!(
            "method={:?} operation={} url={} auth={}",
            self.method,
            self.operation,
            self.url,
            auth.join("+")
        )
    }
}

fn live_auth_headers_from_env() -> Result<LiveAuthHeaders, String> {
    let mut headers =
        if let Some(command_line) = command_line_from_env(STUDIO_BABEL_SERVICE_AUTH_COMMAND) {
            live_auth_headers_from_command(&command_line)?
        } else {
            LiveAuthHeaders::default()
        };

    if let Ok(cookie) = env::var(STUDIO_BABEL_SERVICE_COOKIE) {
        let cookie = cookie.trim();
        if !cookie.is_empty() {
            headers.cookie = Some(cookie.to_string());
        }
    }
    if let Ok(authorization) = env::var(STUDIO_BABEL_SERVICE_AUTHORIZATION) {
        let authorization = authorization.trim();
        if !authorization.is_empty() {
            headers.authorization = Some(authorization.to_string());
        }
    }

    Ok(headers)
}

fn live_auth_configuration_error(native_error: Option<String>) -> String {
    let help = format!(
        "set {STUDIO_BABEL_SERVICE_COOKIE}, {STUDIO_BABEL_SERVICE_AUTHORIZATION}, {STUDIO_BABEL_SERVICE_AUTH_COMMAND}, or complete native Babel auth with `studio-app --babel-auth-verify-login <code>`"
    );
    match native_error {
        Some(error) if !error.trim().is_empty() => {
            format!("Babel live service auth is not configured from native store: {error}; {help}")
        }
        _ => format!("Babel live service auth is not configured; {help}"),
    }
}

fn live_auth_headers_from_command(command_line: &str) -> Result<LiveAuthHeaders, String> {
    let output = service_command(command_line)
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

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("Babel live auth command output was not UTF-8: {error}"))?;
    parse_live_auth_command_output(&stdout)
}

fn parse_live_auth_command_output(output: &str) -> Result<LiveAuthHeaders, String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(LiveAuthHeaders::default());
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let cookie = first_string_by_key(&value, &["cookie", "cookieHeader", "Cookie"])
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let authorization = first_string_by_key(
            &value,
            &["authorization", "authorizationHeader", "Authorization"],
        )
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
        return Ok(LiveAuthHeaders {
            cookie,
            authorization,
        });
    }

    Ok(LiveAuthHeaders {
        cookie: Some(trimmed.to_string()),
        authorization: None,
    })
}

fn live_trpc_request_parts(
    envelope: &Value,
    base_url: &str,
    auth: LiveAuthHeaders,
) -> Result<LiveTrpcRequestParts, String> {
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

    let payload = envelope.get("payload").cloned().unwrap_or(Value::Null);
    let batch_input = envelope.get("batchInput").cloned();
    let method = live_trpc_method_for_operation(operation);
    let post_body = batch_input
        .clone()
        .unwrap_or_else(|| json!({ "0": { "json": payload } }))
        .to_string();
    let (url, body) = match method {
        LiveTrpcMethod::Post => (
            format!("{base_url}/api/trpc/{operation}?batch=1"),
            Some(post_body),
        ),
        LiveTrpcMethod::Get => {
            let input = batch_input
                .unwrap_or_else(|| json!({ "0": { "json": payload } }))
                .to_string();
            let encoded_input: String =
                url::form_urlencoded::byte_serialize(input.as_bytes()).collect();
            (
                format!("{base_url}/api/trpc/{operation}?batch=1&input={encoded_input}"),
                None,
            )
        }
    };
    Ok(LiveTrpcRequestParts {
        method,
        operation: operation.to_string(),
        url,
        body,
        cookie_header: auth.cookie,
        authorization_header: auth.authorization,
    })
}

fn live_trpc_method_for_operation(operation: &str) -> LiveTrpcMethod {
    match operation {
        "worker.getProjects"
        | "transcriptions.getReviewActionDataById"
        | "transcriptions.getLatestReviewActionByProcessedTranscriptionId" => LiveTrpcMethod::Get,
        BABEL_DASHBOARD_PROJECTS_OPERATIONS => LiveTrpcMethod::Get,
        _ => LiveTrpcMethod::Post,
    }
}

async fn execute_live_trpc_request(parts: LiveTrpcRequestParts) -> Result<String, String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(cookie) = parts
        .cookie_header
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        headers.insert(
            COOKIE,
            HeaderValue::from_str(cookie)
                .map_err(|error| format!("Babel live cookie header is invalid: {error}"))?,
        );
    }
    if let Some(authorization) = parts
        .authorization_header
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(authorization)
                .map_err(|error| format!("Babel live authorization header is invalid: {error}"))?,
        );
    }

    let client = reqwest::Client::new();
    let request = match parts.method {
        LiveTrpcMethod::Get => client.get(parts.url).headers(headers),
        LiveTrpcMethod::Post => client
            .post(parts.url)
            .headers(headers)
            .body(parts.body.unwrap_or_default()),
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("Babel live service request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read Babel live service response: {error}"))?;

    if !status.is_success() {
        return Err(format!(
            "Babel live service HTTP {status}: {}",
            response_body_snippet(&body)
        ));
    }

    Ok(body)
}

fn response_body_snippet(body: &str) -> String {
    let snippet: String = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect();
    if snippet.is_empty() {
        "<empty response body>".to_string()
    } else {
        snippet
    }
}

#[cfg(windows)]
fn service_command(command_line: &str) -> Command {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new("cmd");
    command
        .args(["/D", "/S", "/C"])
        .raw_arg(format!("\"{command_line}\""));
    command
}

#[cfg(not(windows))]
fn service_command(command_line: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(command_line);
    command
}

pub(super) fn request_envelope_for_service_command(request: &BabelServiceRequest) -> Value {
    match request {
        BabelServiceRequest::LoadWorkerProjects => {
            json!({
                "operation": BABEL_DASHBOARD_PROJECTS_OPERATIONS,
                "batchInput": babel_dashboard_projects_batch_input(),
            })
        }
        BabelServiceRequest::SaveAnnotations(SaveAnnotationsRequest {
            review_action_id,
            annotations,
        }) => json!({
            "operation": "transcriptions.saveAnnotationsByReviewActionId",
            "payload": {
                "reviewActionId": review_action_id,
                "annotations": annotations,
            },
        }),
        BabelServiceRequest::ClaimNextFromQueue(ClaimNextFromQueueRequest { queue_id }) => json!({
            "operation": "transcriptions.claimNextReviewActionFromReviewQueue",
            "payload": { "queueId": queue_id },
        }),
        BabelServiceRequest::ClaimByProcessedTranscription(
            ClaimByProcessedTranscriptionRequest {
                processed_transcription_id,
                queue_id,
            },
        ) => json!({
            "operation": "transcriptions.claimReviewActionByProcessedTranscriptionId",
            "payload": {
                "processedTranscriptionId": processed_transcription_id,
                "queueId": queue_id,
            },
        }),
        BabelServiceRequest::LoadReviewAction(LoadReviewActionRequest { review_action_id }) => {
            json!({
                "operation": "transcriptions.getReviewActionDataById",
                "payload": { "reviewActionId": review_action_id },
            })
        }
        BabelServiceRequest::LoadLatestReviewAction(LoadLatestReviewActionRequest {
            processed_transcription_id,
            queue_id,
        }) => json!({
            "operation": "transcriptions.getLatestReviewActionByProcessedTranscriptionId",
            "payload": {
                "processedTranscriptionId": processed_transcription_id,
                "queueId": queue_id,
            },
        }),
        BabelServiceRequest::SubmitReviewAction(SubmitReviewActionRequest {
            review_action_id,
            annotations,
            is_stitched_queue,
        }) => json!({
            "operation": "transcriptions.submitTranscriptReviewAction",
            "payload": {
                "reviewActionId": review_action_id,
                "annotations": annotations,
                "isStitchedQueue": is_stitched_queue,
            },
        }),
        BabelServiceRequest::DropWorkerAction(DropWorkerActionRequest { queue_id }) => json!({
            "operation": "transcriptions.dropWorkerAction",
            "payload": { "queueId": queue_id },
        }),
        BabelServiceRequest::SkipReviewAction(SkipReviewActionRequest { review_action_id }) => {
            json!({
                "operation": "transcriptions.skipTranscriptReviewAction",
                "payload": { "reviewActionId": review_action_id },
            })
        }
        BabelServiceRequest::AdminRateReviewAction(AdminRateReviewActionRequest {
            review_action_id,
            rating,
            rating_reason,
        }) => json!({
            "operation": "transcriptions.adminRateTtsTranscription",
            "payload": {
                "reviewActionId": review_action_id,
                "rating": rating,
                "ratingReason": rating_reason,
            },
        }),
        BabelServiceRequest::SaveFeedbackDraft(request) => json!({
            "operation": "transcriptionFeedbackForm.saveDraft",
            "payload": {
                "formStepResponseId": request.form_step_response_id,
                "inputResponses": request.input_responses.iter().map(|response| {
                    json!({
                        "formInputId": response.form_input_id,
                        "value": response.value,
                    })
                }).collect::<Vec<_>>(),
            },
        }),
    }
}

fn babel_dashboard_projects_batch_input() -> Value {
    let undefined = json!({
        "json": null,
        "meta": { "values": ["undefined"] },
    });
    json!({
        "0": undefined,
        "1": undefined,
        "2": undefined,
        "3": undefined,
        "4": undefined,
        "5": {
            "json": {
                "names": [
                    "onboarding-welcome",
                    "onboarding-about",
                    "onboarding-quality-standards",
                    "onboarding-complete-profile",
                    "onboarding-agreement",
                    "onboarding-idv-explainer",
                    "onboarding-setup-payments",
                    "onboarding-pangram-info",
                    "onboarding-pangram",
                    "onboarding-start-project",
                    "onboarding-community"
                ]
            }
        },
        "6": undefined,
        "7": {
            "json": {
                "names": ["joyride", "getting-started-video"]
            }
        },
        "8": undefined,
        "9": undefined,
        "10": undefined,
    })
}

fn normalize_service_response_payload_for_request(
    request: &BabelServiceRequest,
    response: &str,
) -> String {
    match request {
        BabelServiceRequest::LoadWorkerProjects => {
            normalize_dashboard_projects_response_payload(response)
        }
        BabelServiceRequest::LoadReviewAction(_)
        | BabelServiceRequest::LoadLatestReviewAction(_)
        | BabelServiceRequest::ClaimByProcessedTranscription(_)
        | BabelServiceRequest::ClaimNextFromQueue(_) => {
            normalize_review_action_response_payload(response)
        }
        _ => normalize_service_response_payload(response),
    }
}

fn normalize_dashboard_projects_response_payload(response: &str) -> String {
    let trimmed = response.trim_start_matches('\u{feff}').trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return response.trim().to_string();
    };

    if let Value::Array(items) = &value
        && let Some(projects) = items
            .get(2)
            .and_then(normalize_trpc_response_value)
            .filter(Value::is_array)
    {
        return json!({ "projects": projects }).to_string();
    }

    let normalized = normalize_trpc_response_value(&value).unwrap_or(value);
    if normalized.is_array() {
        json!({ "projects": normalized }).to_string()
    } else {
        normalized.to_string()
    }
}

async fn discover_live_dashboard_project_configs(
    normalized_payload: &str,
    parts: &LiveTrpcRequestParts,
) -> Result<Vec<Value>, String> {
    let routes = dashboard_project_config_routes(normalized_payload);
    if routes.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::new();
    for route in routes {
        let html = fetch_live_text(&client, &format!("{}{}", live_base_url(), route), parts).await;
        let Ok(html) = html else {
            continue;
        };
        let scripts = dashboard_script_urls(&html, &live_base_url());
        for script_url in scripts {
            let Ok(script) = fetch_live_text(&client, &script_url, parts).await else {
                continue;
            };
            let configs = dashboard_project_configs_from_script(&script);
            if let Some(items) = configs.as_array()
                && !items.is_empty()
            {
                return Ok(items.clone());
            }
        }
    }
    Ok(Vec::new())
}

async fn fetch_live_text(
    client: &reqwest::Client,
    url: &str,
    parts: &LiveTrpcRequestParts,
) -> Result<String, String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    if let Some(cookie) = parts
        .cookie_header
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        headers.insert(
            COOKIE,
            HeaderValue::from_str(cookie)
                .map_err(|error| format!("Babel live cookie header is invalid: {error}"))?,
        );
    }
    if let Some(authorization) = parts
        .authorization_header
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(authorization)
                .map_err(|error| format!("Babel live authorization header is invalid: {error}"))?,
        );
    }

    let response = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|error| format!("Babel dashboard config request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read Babel dashboard config response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Babel dashboard config HTTP {status}: {}",
            response_body_snippet(&body)
        ));
    }
    Ok(body)
}

fn dashboard_project_config_routes(normalized_payload: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(normalized_payload) else {
        return vec!["/transcription/RU-tx-gold".to_string()];
    };
    let projects = value
        .get("projects")
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    let mut routes = projects
        .into_iter()
        .flatten()
        .filter_map(|project| {
            first_string_by_key(project, &["name", "slug", "projectSlug"])
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .map(|name| format!("/transcription/{}", encode_path_segment(&name)))
        .collect::<Vec<_>>();
    routes.push("/transcription/RU-tx-gold".to_string());
    routes.sort();
    routes.dedup();
    routes
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn dashboard_script_urls(html: &str, base_url: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut remaining = html;
    while let Some(src_index) = remaining.find("src=") {
        remaining = &remaining[src_index + 4..];
        let mut chars = remaining.chars();
        let Some(quote) = chars.next() else {
            break;
        };
        if quote != '"' && quote != '\'' {
            continue;
        }
        let value_start = quote.len_utf8();
        let Some(value_end) = remaining[value_start..].find(quote) else {
            break;
        };
        let src = &remaining[value_start..value_start + value_end];
        if src.ends_with(".js") {
            urls.push(resolve_dashboard_script_url(base_url, src));
        }
        remaining = &remaining[value_start + value_end + quote.len_utf8()..];
    }
    urls.sort();
    urls.dedup();
    urls
}

fn resolve_dashboard_script_url(base_url: &str, src: &str) -> String {
    if src.starts_with("http://") || src.starts_with("https://") {
        return src.to_string();
    }
    if src.starts_with('/') {
        format!("{}{}", base_url.trim_end_matches('/'), src)
    } else {
        format!("{}/{}", base_url.trim_end_matches('/'), src)
    }
}

fn attach_dashboard_project_configs(payload: &str, configs: Vec<Value>) -> String {
    if configs.is_empty() {
        return payload.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return payload.to_string();
    };
    match &mut value {
        Value::Object(map) => {
            map.insert("projectConfigs".to_string(), Value::Array(configs));
            value.to_string()
        }
        Value::Array(_) => json!({
            "projects": value,
            "projectConfigs": configs,
        })
        .to_string(),
        _ => payload.to_string(),
    }
}

fn dashboard_project_configs_from_script(script: &str) -> Value {
    let project_ids = dashboard_project_ids_by_bundle_key(script);
    let queue_configs = dashboard_queue_configs_by_bundle_key(script);
    let mut configs_by_project_id = BTreeMap::new();

    for (key, (chunked_queue_id, stitched_queue_id)) in queue_configs {
        let Some(project_id) = project_ids.get(&key) else {
            continue;
        };
        configs_by_project_id.insert(
            project_id.clone(),
            json!({
                "projectId": project_id,
                "chunkedQueueId": chunked_queue_id,
                "stitchedTxQueueId": stitched_queue_id,
            }),
        );
    }

    Value::Array(configs_by_project_id.into_values().collect())
}

fn dashboard_project_ids_by_bundle_key(script: &str) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    let mut offset = 0;
    while let Some(marker_index) = script[offset..].find(":\"") {
        let marker_index = offset + marker_index;
        let value_start = marker_index + 2;
        let Some(value_end) = script[value_start..].find('"') else {
            break;
        };
        let value = &script[value_start..value_start + value_end];
        if is_uuid_like(value)
            && let Some(key) = uppercase_key_before(script, marker_index)
        {
            ids.insert(key, value.to_string());
        }
        offset = value_start + value_end + 1;
    }
    ids
}

fn dashboard_queue_configs_by_bundle_key(script: &str) -> BTreeMap<String, (String, String)> {
    let mut configs = BTreeMap::new();
    let mut offset = 0;
    const CHUNKED: &str = "chunkedQueueId:\"";
    const STITCHED: &str = "stitchedTxQueueId:\"";
    while let Some(chunked_index) = script[offset..].find(CHUNKED) {
        let chunked_index = offset + chunked_index;
        let Some(key) = bundle_key_before_queue_config(script, chunked_index) else {
            offset = chunked_index + CHUNKED.len();
            continue;
        };
        let chunked_start = chunked_index + CHUNKED.len();
        let Some(chunked_end) = script[chunked_start..].find('"') else {
            break;
        };
        let chunked_queue_id = &script[chunked_start..chunked_start + chunked_end];
        let stitched_search_start = chunked_start + chunked_end;
        let Some(stitched_index) = script[stitched_search_start..].find(STITCHED) else {
            offset = stitched_search_start;
            continue;
        };
        let stitched_start = stitched_search_start + stitched_index + STITCHED.len();
        let Some(stitched_end) = script[stitched_start..].find('"') else {
            break;
        };
        let stitched_queue_id = &script[stitched_start..stitched_start + stitched_end];
        if is_uuid_like(chunked_queue_id) && is_uuid_like(stitched_queue_id) {
            configs.insert(
                key,
                (chunked_queue_id.to_string(), stitched_queue_id.to_string()),
            );
        }
        offset = stitched_start + stitched_end + 1;
    }
    configs
}

fn uppercase_key_before(script: &str, marker_index: usize) -> Option<String> {
    let mut start = marker_index;
    while start > 0 {
        let Some(character) = script[..start].chars().next_back() else {
            break;
        };
        if character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_' {
            start -= character.len_utf8();
        } else {
            break;
        }
    }
    if start == marker_index {
        return None;
    }
    let key = &script[start..marker_index];
    key.contains('_').then(|| key.to_string())
}

fn bundle_key_before_queue_config(script: &str, chunked_index: usize) -> Option<String> {
    let search_start = chunked_index.saturating_sub(120);
    let prefix = &script[search_start..chunked_index];
    let bracket_start = prefix.rfind('[')?;
    let bracket_end = prefix[bracket_start..].find(']')? + bracket_start;
    let inside = &prefix[bracket_start + 1..bracket_end];
    let key = inside.rsplit('.').next()?.trim();
    (!key.is_empty()).then(|| key.to_string())
}

fn is_uuid_like(value: &str) -> bool {
    value.len() == 36
        && value
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
        && value.as_bytes().get(8) == Some(&b'-')
        && value.as_bytes().get(13) == Some(&b'-')
        && value.as_bytes().get(18) == Some(&b'-')
        && value.as_bytes().get(23) == Some(&b'-')
}

fn normalize_review_action_response_payload(response: &str) -> String {
    let trimmed = response.trim_start_matches('\u{feff}').trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return trimmed.to_string();
    };

    if review_action_data_value(&value).is_some() {
        return value.to_string();
    }

    if review_action_data_shape(&value) {
        return json!({ "json": value }).to_string();
    }

    if let Some(data) = normalize_trpc_response_value(&value)
        && review_action_data_shape(&data)
    {
        return json!({ "json": data }).to_string();
    }

    value.to_string()
}

fn review_action_data_value(value: &Value) -> Option<&Value> {
    value
        .pointer("/result/data/json")
        .or_else(|| value.pointer("/data/json"))
        .or_else(|| value.pointer("/json"))
        .or_else(|| {
            value
                .as_array()?
                .iter()
                .find_map(|entry| entry.pointer("/result/data/json"))
        })
}

fn review_action_data_shape(value: &Value) -> bool {
    value
        .get("actionId")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        && value
            .get("transcriptionChunkProcessedRecordings")
            .and_then(Value::as_array)
            .is_some()
        && value.get("annotations").and_then(Value::as_array).is_some()
}

fn normalize_service_response_payload(response: &str) -> String {
    let trimmed = response.trim_start_matches('\u{feff}').trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return trimmed.to_string();
    };

    normalize_trpc_response_value(&value)
        .unwrap_or(value)
        .to_string()
}

fn normalize_trpc_response_value(value: &Value) -> Option<Value> {
    if let Value::Array(items) = value {
        return items.first().and_then(normalize_trpc_response_value);
    }

    value
        .pointer("/result/data/json")
        .or_else(|| value.pointer("/data/json"))
        .or_else(|| value.pointer("/result/data"))
        .or_else(|| value.pointer("/data"))
        .cloned()
}

fn failure_message_for_request(request: BabelServiceRequest, error: &str) -> Message {
    match request {
        BabelServiceRequest::LoadWorkerProjects => {
            Message::DashboardProjectsLoadFailed(error.to_string())
        }
        BabelServiceRequest::SaveAnnotations(_) => Message::SaveProgressFailed(error.to_string()),
        BabelServiceRequest::LoadReviewAction(_) => {
            Message::LoadReviewActionFailed(error.to_string())
        }
        BabelServiceRequest::LoadLatestReviewAction(_) => {
            Message::LoadLatestReviewActionFailed(error.to_string())
        }
        BabelServiceRequest::ClaimByProcessedTranscription(_) => {
            Message::ClaimByProcessedTranscriptionFailed(error.to_string())
        }
        BabelServiceRequest::ClaimNextFromQueue(_) => {
            Message::ClaimNextFromQueueFailed(error.to_string())
        }
        BabelServiceRequest::SubmitReviewAction(_) => {
            Message::SubmitReviewFailed(error.to_string())
        }
        BabelServiceRequest::DropWorkerAction(_) => Message::DropClaimFailed(error.to_string()),
        BabelServiceRequest::SkipReviewAction(_) => Message::SkipChunkFailed(error.to_string()),
        BabelServiceRequest::AdminRateReviewAction(_) => {
            Message::AdminFlagFailed(error.to_string())
        }
        BabelServiceRequest::SaveFeedbackDraft(_) => {
            Message::FeedbackDraftSaveFailed(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::{OsStr, OsString},
        fs,
        io::{Read, Write as IoWrite},
        net::TcpListener,
        path::Path,
        sync::{Mutex, MutexGuard},
        thread,
    };

    use studio_app::{FeedbackDraftSaveRequest, FeedbackInputResponse};

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn remote_mutations_require_command_or_explicit_live_mode() {
        let _env = TestEnv::with_live_native_auth();

        assert!(!babel_remote_mutations_configured());

        set_env_var(STUDIO_BABEL_SERVICE_MODE, "live");

        assert!(babel_remote_mutations_configured());
    }

    #[tokio::test]
    async fn dashboard_project_load_uses_live_service_by_default_with_native_auth() {
        let _env = TestEnv::with_live_native_auth();
        let babel = MockJsonServer::start(vec![
            r#"[null,null,{"result":{"data":{"json":[{"id":"project-1","name":"Project 1"}]}}}]"#,
        ]);
        set_env_var(STUDIO_BABEL_SERVICE_BASE_URL, babel.base_url.as_str());
        set_env_var(STUDIO_BABEL_SERVICE_COOKIE, "__session=session-cookie");

        let payload = live_payload_for_request(&BabelServiceRequest::LoadWorkerProjects)
            .await
            .expect("live request should be attempted")
            .expect("live response");

        assert!(payload.contains("project-1"));
        let babel_requests = babel.join();
        assert!(request_contains(
            &babel_requests[0],
            "GET /api/trpc/worker.get,"
        ));
        assert!(request_contains(
            &babel_requests[0],
            "Cookie: __session=session-cookie"
        ));
    }

    #[test]
    fn service_command_line_runs_with_arguments_and_receives_recovered_envelope() {
        let temp = tempfile::tempdir().expect("temp dir");
        let request_log = temp.path().join("request.json");
        let script_path = temp.path().join(if cfg!(windows) {
            "service adapter.cmd"
        } else {
            "service adapter.sh"
        });
        write_service_adapter_script(&script_path);
        let command_line = format!(
            "\"{}\" \"{}\"",
            script_path.display(),
            request_log.display()
        );
        let request = BabelServiceRequest::SaveFeedbackDraft(FeedbackDraftSaveRequest {
            form_step_response_id: "form-step-response-1".to_string(),
            input_responses: vec![FeedbackInputResponse {
                form_input_id: "comment".to_string(),
                value: "Looks good".to_string(),
            }],
        });

        let payload = command_payload_for_command_line(&request, &command_line)
            .expect("command should run")
            .expect("command response");

        assert_eq!(
            payload,
            r#"{"success":true,"updatedAt":"2026-06-23T12:00:00Z"}"#
        );
        let envelope: Value = serde_json::from_str(
            &fs::read_to_string(request_log).expect("adapter should receive stdin"),
        )
        .expect("request envelope should be json");
        assert_eq!(
            envelope,
            json!({
                "operation": "transcriptionFeedbackForm.saveDraft",
                "payload": {
                    "formStepResponseId": "form-step-response-1",
                    "inputResponses": [
                        { "formInputId": "comment", "value": "Looks good" },
                    ],
                },
            })
        );
    }

    #[test]
    fn normalizes_babel_trpc_batch_json_results() {
        assert_eq!(
            normalize_service_response_payload(
                r#"[{"result":{"data":{"json":{"success":true,"updatedAt":"2026-06-28T10:00:00Z"}}}}]"#
            ),
            r#"{"success":true,"updatedAt":"2026-06-28T10:00:00Z"}"#
        );
    }

    #[test]
    fn live_trpc_request_parts_match_babel_batch_post_contract() {
        let envelope = json!({
            "operation": "transcriptions.saveAnnotationsByReviewActionId",
            "payload": {
                "reviewActionId": "review-action-42",
                "annotations": [{ "id": "annotation-1" }],
            },
        });
        let auth = LiveAuthHeaders {
            cookie: Some("babel_session=super-secret".to_string()),
            authorization: None,
        };

        let parts = live_trpc_request_parts(&envelope, "https://dashboard.babel.audio/", auth)
            .expect("live request parts");

        assert_eq!(parts.method, LiveTrpcMethod::Post);
        assert_eq!(
            parts.url,
            "https://dashboard.babel.audio/api/trpc/transcriptions.saveAnnotationsByReviewActionId?batch=1"
        );
        assert_eq!(
            serde_json::from_str::<Value>(parts.body.as_deref().expect("post body"))
                .expect("batch body"),
            json!({
                "0": {
                    "json": {
                        "reviewActionId": "review-action-42",
                        "annotations": [{ "id": "annotation-1" }],
                    },
                },
            })
        );
        assert_eq!(
            parts.cookie_header.as_deref(),
            Some("babel_session=super-secret")
        );
        assert!(!parts.redacted_summary().contains("super-secret"));
        assert!(parts.redacted_summary().contains("auth=cookie"));
    }

    #[test]
    fn live_trpc_request_parts_use_get_for_worker_project_dashboard_batch() {
        let envelope =
            request_envelope_for_service_command(&BabelServiceRequest::LoadWorkerProjects);
        let auth = LiveAuthHeaders {
            cookie: Some("babel_session=super-secret".to_string()),
            authorization: None,
        };

        let parts = live_trpc_request_parts(&envelope, "https://dashboard.babel.audio/", auth)
            .expect("live request parts");

        assert_eq!(parts.method, LiveTrpcMethod::Get);
        assert!(parts.body.is_none());
        assert!(parts.url.starts_with(
            "https://dashboard.babel.audio/api/trpc/worker.get,worker.queryWorkerMetadata,worker.getProjects"
        ));
        assert!(parts.url.contains("timer.getActiveTemporaryRates"));
        assert!(parts.url.contains("%2210%22"));
        assert!(parts.redacted_summary().contains("method=Get"));
        assert!(!parts.redacted_summary().contains("super-secret"));
    }

    #[test]
    fn dashboard_project_batch_normalization_keeps_project_batch_for_queue_config_merge() {
        let payload = normalize_service_response_payload_for_request(
            &BabelServiceRequest::LoadWorkerProjects,
            r#"[
                {"result":{"data":{"json":{"id":"worker-1"}}}},
                {"result":{"data":{"json":{"status":"active"}}}},
                {"result":{"data":{"json":[{"projectId":"project-1","name":"Project One"}]}}}
            ]"#,
        );

        assert_eq!(
            serde_json::from_str::<Value>(&payload).expect("normalized projects"),
            json!({
                "projects": [{ "projectId": "project-1", "name": "Project One" }]
            })
        );
    }

    #[test]
    fn dashboard_project_configs_are_discovered_from_babel_bundle_script() {
        let script = r#"
            let E={RU_TX_GOLD_DG:"1c97f55a-3868-4861-9778-d39ee80cda3d"};
            let H={[E.RU_TX_GOLD_DG]:{chunkedQueueId:"a1111111-1111-4111-8111-111111111111",stitchedTxQueueId:"b2222222-2222-4222-8222-222222222222"}};
        "#;

        assert_eq!(
            dashboard_project_configs_from_script(script),
            json!([
                {
                    "projectId": "1c97f55a-3868-4861-9778-d39ee80cda3d",
                    "chunkedQueueId": "a1111111-1111-4111-8111-111111111111",
                    "stitchedTxQueueId": "b2222222-2222-4222-8222-222222222222"
                }
            ])
        );
    }

    #[test]
    fn review_action_normalization_wraps_bare_claim_payload_for_importer() {
        let payload = normalize_service_response_payload_for_request(
            &BabelServiceRequest::ClaimNextFromQueue(ClaimNextFromQueueRequest {
                queue_id: "queue-1".to_string(),
            }),
            r#"{
                "actionId": "review-action-1",
                "actionLevel": 1,
                "success": true,
                "isWorkerResumingClaim": true,
                "processedRecordingUriMap": {},
                "transcriptionChunkProcessedRecordings": [],
                "annotations": [],
                "lintErrors": []
            }"#,
        );
        let value = serde_json::from_str::<Value>(&payload).expect("wrapped review action");

        assert_eq!(
            value.pointer("/json/actionId").and_then(Value::as_str),
            Some("review-action-1")
        );
        assert!(value.pointer("/json/success").is_some());
    }

    #[tokio::test]
    async fn live_payload_uses_native_auth_store_without_auth_command() {
        let _env = TestEnv::with_live_native_auth();
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("babel-auth.json");
        fs::write(
            &store_path,
            r#"{
                "version": 1,
                "cookies": [
                    {
                        "name": "__client",
                        "value": "client-cookie",
                        "domain": "127.0.0.1",
                        "path": "/",
                        "expiresAt": 4102444800
                    }
                ]
            }"#,
        )
        .expect("write auth store");
        let clerk = MockJsonServer::start(vec![
            r#"{"response":{"object":"client","last_active_session_id":"sess_native","sessions":[{"id":"sess_native","status":"active"}]}}"#,
            r#"{"object":"token","jwt":"jwt-from-clerk"}"#,
        ]);
        let babel = MockJsonServer::start(vec![
            r#"[{"result":{"data":{"json":{"success":true,"updatedAt":"2026-06-29T12:00:00Z"}}}}]"#,
        ]);
        set_env_var(STUDIO_BABEL_SERVICE_MODE, "live");
        set_env_var("STUDIO_BABEL_AUTH_STORE", store_path.as_os_str());
        set_env_var("STUDIO_BABEL_CLERK_URL", clerk.base_url.as_str());
        set_env_var(STUDIO_BABEL_SERVICE_BASE_URL, babel.base_url.as_str());

        let request = BabelServiceRequest::SaveFeedbackDraft(FeedbackDraftSaveRequest {
            form_step_response_id: "form-step-response-1".to_string(),
            input_responses: vec![FeedbackInputResponse {
                form_input_id: "comment".to_string(),
                value: "Looks good".to_string(),
            }],
        });

        let payload = live_payload_for_request(&request)
            .await
            .expect("live request should succeed")
            .expect("live response");

        assert_eq!(
            payload,
            r#"{"success":true,"updatedAt":"2026-06-29T12:00:00Z"}"#
        );
        let clerk_requests = clerk.join();
        assert!(request_contains(&clerk_requests[0], "GET /v1/client?"));
        assert!(request_contains(
            &clerk_requests[0],
            "Cookie: __client=client-cookie"
        ));
        assert!(request_contains(
            &clerk_requests[1],
            "POST /v1/client/sessions/sess_native/tokens?"
        ));
        assert!(request_contains(&clerk_requests[1], "organization_id="));
        let babel_requests = babel.join();
        assert!(request_contains(
            &babel_requests[0],
            "POST /api/trpc/transcriptionFeedbackForm.saveDraft?batch=1"
        ));
        assert!(request_contains(
            &babel_requests[0],
            "Authorization: Bearer jwt-from-clerk"
        ));
        assert!(!request_contains(&babel_requests[0], "Cookie: __session="));
        assert!(request_contains(&babel_requests[0], "formStepResponseId"));
    }

    #[tokio::test]
    async fn subaccount_session_routes_babel_service_envelope_to_relay() {
        let _env = TestEnv::with_live_native_auth();
        let temp = tempfile::tempdir().expect("temp dir");
        let relay = MockJsonServer::start(vec![
            r#"{"payload":{"success":true,"updatedAt":"2026-07-07T12:00:00Z"}}"#,
        ]);
        let store_path = temp.path().join("subaccount-session.json");
        fs::write(
            &store_path,
            format!(
                r#"{{"version":1,"hostUrl":"{}","token":"relay-token","subaccountId":"sub-1"}}"#,
                relay.base_url
            ),
        )
        .expect("write subaccount store");
        set_env_var("STUDIO_SUBACCOUNT_SESSION_STORE", store_path.as_os_str());
        let request = BabelServiceRequest::SaveAnnotations(SaveAnnotationsRequest {
            review_action_id: "review-action-1".to_string(),
            annotations: vec![json!({ "id": "annotation-1" })],
        });

        let payload = service_payload_for_request(&request)
            .await
            .expect("relay payload");

        assert_eq!(
            payload,
            r#"{"success":true,"updatedAt":"2026-07-07T12:00:00Z"}"#
        );
        let relay_requests = relay.join();
        assert!(request_contains(
            &relay_requests[0],
            "POST /api/subaccounts/babel-service"
        ));
        assert!(request_contains(
            &relay_requests[0],
            "Authorization: Bearer relay-token"
        ));
        assert!(request_contains(
            &relay_requests[0],
            "transcriptions.saveAnnotationsByReviewActionId"
        ));
        assert!(request_contains(&relay_requests[0], "saveAnnotations"));
    }

    fn write_service_adapter_script(path: &Path) {
        #[cfg(windows)]
        {
            fs::write(
                path,
                concat!(
                    "@echo off\r\n",
                    "more > \"%~1\"\r\n",
                    "echo {\"result\":{\"data\":{\"success\":true,\"updatedAt\":\"2026-06-23T12:00:00Z\"}}}\r\n",
                ),
            )
            .expect("write windows adapter script");
        }

        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::write(
                path,
                concat!(
                    "#!/bin/sh\n",
                    "cat > \"$1\"\n",
                    "printf '%s\\n' '{\"result\":{\"data\":{\"success\":true,\"updatedAt\":\"2026-06-23T12:00:00Z\"}}}'\n",
                ),
            )
            .expect("write unix adapter script");
            let mut permissions = fs::metadata(path).expect("script metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("mark script executable");
        }
    }

    fn request_contains(request: &str, expected: &str) -> bool {
        request
            .to_ascii_lowercase()
            .contains(&expected.to_ascii_lowercase())
    }

    struct TestEnv {
        _guard: MutexGuard<'static, ()>,
        _temp_dir: tempfile::TempDir,
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl TestEnv {
        fn with_live_native_auth() -> Self {
            let guard = ENV_LOCK.lock().expect("env lock");
            let names = vec![
                STUDIO_BABEL_SERVICE_MODE,
                STUDIO_BABEL_SERVICE_COMMAND,
                STUDIO_BABEL_SERVICE_AUTH_COMMAND,
                STUDIO_BABEL_SERVICE_COOKIE,
                STUDIO_BABEL_SERVICE_AUTHORIZATION,
                STUDIO_BABEL_SERVICE_BASE_URL,
                "STUDIO_BABEL_AUTH_STORE",
                "STUDIO_BABEL_CLERK_URL",
                "STUDIO_SUBACCOUNT_SESSION_STORE",
                "STUDIO_SUBACCOUNT_RELAY_URL",
            ];
            let saved = names
                .into_iter()
                .map(|name| (name, env::var_os(name)))
                .collect::<Vec<_>>();
            for (name, _) in &saved {
                remove_env_var(name);
            }
            let temp_dir = tempfile::tempdir().expect("temp dir");
            set_env_var(
                "STUDIO_SUBACCOUNT_SESSION_STORE",
                temp_dir
                    .path()
                    .join("missing-subaccount-session.json")
                    .as_os_str(),
            );
            Self {
                _guard: guard,
                _temp_dir: temp_dir,
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

    struct MockJsonServer {
        base_url: String,
        handle: thread::JoinHandle<Vec<String>>,
    }

    impl MockJsonServer {
        fn start(responses: Vec<&'static str>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
            let handle = thread::spawn(move || {
                let mut requests = Vec::new();
                for response in responses {
                    let (mut stream, _) = listener.accept().expect("accept request");
                    requests.push(read_http_request(&mut stream));
                    write_json_response(&mut stream, response);
                }
                requests
            });
            Self { base_url, handle }
        }

        fn join(self) -> Vec<String> {
            self.handle.join().expect("mock server join")
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

    fn write_json_response(stream: &mut std::net::TcpStream, body: &str) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write response");
    }
}
