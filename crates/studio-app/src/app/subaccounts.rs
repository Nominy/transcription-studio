use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use iced::Task;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::Message;
use super::native_auth::{
    default_native_auth_store_path, mint_native_babel_authorization_from_store, native_clerk_url,
};
use super::native_auth_cli::native_babel_url;
#[cfg(not(test))]
use super::platform_paths::application_support_dir_named;
use super::service_callbacks::BabelServiceRequest;

const SUBACCOUNT_SESSION_STORE_ENV: &str = "STUDIO_SUBACCOUNT_SESSION_STORE";
const SUBACCOUNT_RELAY_SETTINGS_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_SETTINGS";
pub(super) const SUBACCOUNT_RELAY_URL_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_URL";
#[cfg(not(test))]
const DEFAULT_SESSION_FILE_NAME: &str = "subaccount-session.json";
#[cfg(not(test))]
const DEFAULT_RELAY_SETTINGS_FILE_NAME: &str = "subaccount-relay-settings.json";
const LOGIN_PATH: &str = "/api/subaccounts/login";
const BABEL_SERVICE_PATH: &str = "/api/subaccounts/babel-service";
const AUDIO_PRESIGN_PATH: &str = "/api/subaccounts/audio/presigned-urls";
const OWNER_DASHBOARD_PATH: &str = "/api/subaccounts/owner/dashboard";
const OWNER_SUBACCOUNTS_PATH: &str = "/api/subaccounts/owner/subaccounts";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SubaccountSession {
    pub(super) host_url: String,
    pub(super) token: String,
    pub(super) subaccount_id: String,
    pub(super) display_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SubaccountOwnerDashboard {
    pub(super) subaccounts: Vec<ManagedSubaccount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManagedSubaccount {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) code: Option<String>,
    pub(super) revoked: bool,
    pub(super) allowed_project_ids: Vec<String>,
    pub(super) completed_tasks: u64,
    pub(super) worked_seconds: u64,
    pub(super) earnings_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SubaccountOwnerRequest {
    LoadDashboard {
        host_url: String,
    },
    Create {
        host_url: String,
        label: String,
    },
    Revoke {
        host_url: String,
        subaccount_id: String,
    },
    AssignProjects {
        host_url: String,
        subaccount_id: String,
        project_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelayAuth {
    host_url: String,
    authorization: String,
}

pub(super) fn default_subaccount_session_store_path() -> PathBuf {
    env::var_os(SUBACCOUNT_SESSION_STORE_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(default_subaccount_session_store_path_without_env)
}

fn default_subaccount_session_store_path_without_env() -> PathBuf {
    #[cfg(test)]
    {
        env::temp_dir().join(format!(
            "transcription-studio-test-subaccount-session-{}.json",
            std::process::id()
        ))
    }

    #[cfg(not(test))]
    {
        application_support_dir_named("Transcription Studio").join(DEFAULT_SESSION_FILE_NAME)
    }
}

pub(super) fn default_subaccount_relay_settings_path() -> PathBuf {
    env::var_os(SUBACCOUNT_RELAY_SETTINGS_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(default_subaccount_relay_settings_path_without_env)
}

fn default_subaccount_relay_settings_path_without_env() -> PathBuf {
    #[cfg(test)]
    {
        env::temp_dir().join(format!(
            "transcription-studio-test-subaccount-relay-settings-{}.json",
            std::process::id()
        ))
    }

    #[cfg(not(test))]
    {
        application_support_dir_named("Transcription Studio").join(DEFAULT_RELAY_SETTINGS_FILE_NAME)
    }
}

pub(super) fn read_subaccount_relay_host_url(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read Sub-account relay settings {}: {error}",
            path.display()
        )
    })?;
    if source.trim().is_empty() {
        return Ok(None);
    }
    subaccount_relay_host_url_from_str(&source).map(Some)
}

pub(super) fn read_default_subaccount_relay_host_url() -> Result<Option<String>, String> {
    read_subaccount_relay_host_url(&default_subaccount_relay_settings_path())
}

pub(super) fn save_subaccount_relay_host_url(
    path: &Path,
    host_url: &str,
) -> Result<String, String> {
    let normalized = normalize_relay_host_url(host_url)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create Sub-account relay settings directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let payload = json!({
        "version": 1,
        "relayHostUrl": normalized,
    });
    fs::write(path, payload.to_string()).map_err(|error| {
        format!(
            "failed to save Sub-account relay settings {}: {error}",
            path.display()
        )
    })?;
    Ok(normalized)
}

pub(super) fn save_default_subaccount_relay_host_url(host_url: &str) -> Result<String, String> {
    save_subaccount_relay_host_url(&default_subaccount_relay_settings_path(), host_url)
}

pub(super) fn subaccount_relay_host_url_from_str(source: &str) -> Result<String, String> {
    let value = serde_json::from_str::<Value>(source)
        .map_err(|error| format!("failed to decode Sub-account relay settings: {error}"))?;
    let host_url = string_by_keys(
        &value,
        &["relayHostUrl", "relay_host_url", "hostUrl", "host"],
    )
    .ok_or_else(|| "Sub-account relay settings are missing relayHostUrl.".to_string())?;
    normalize_relay_host_url(&host_url)
}

pub(super) fn default_subaccount_relay_host_url() -> String {
    env::var(SUBACCOUNT_RELAY_URL_ENV)
        .ok()
        .and_then(|value| normalize_relay_host_url(&value).ok())
        .or_else(|| read_default_subaccount_relay_host_url().ok().flatten())
        .unwrap_or_default()
}

pub(super) fn normalize_relay_host_url(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Sub-account host URL is required.".to_string());
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("{}://{trimmed}", default_relay_url_scheme(trimmed))
    };
    let parsed = url::Url::parse(&candidate)
        .map_err(|error| format!("Sub-account host URL is invalid: {error}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!(
                "Sub-account host URL must use http or https, not {scheme}."
            ));
        }
    }
    parsed
        .host_str()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| "Sub-account host URL must include a host.".to_string())?;
    let mut normalized = parsed.to_string();
    while normalized.ends_with('/') {
        normalized.pop();
    }
    Ok(normalized)
}

fn default_relay_url_scheme(value_without_scheme: &str) -> &'static str {
    let authority = value_without_scheme
        .split('/')
        .next()
        .unwrap_or(value_without_scheme)
        .trim();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(ipv6) = authority.strip_prefix('[') {
        ipv6.split(']').next().unwrap_or(ipv6)
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    if host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.") {
        "http"
    } else {
        "https"
    }
}

pub(super) fn read_subaccount_session(path: &Path) -> Result<Option<SubaccountSession>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read Sub-account session store {}: {error}",
            path.display()
        )
    })?;
    if source.trim().is_empty() {
        return Ok(None);
    }
    subaccount_session_from_str(&source).map(Some)
}

pub(super) fn read_default_subaccount_session() -> Result<Option<SubaccountSession>, String> {
    read_subaccount_session(&default_subaccount_session_store_path())
}

pub(super) fn subaccount_session_is_active() -> bool {
    read_default_subaccount_session().ok().flatten().is_some()
}

pub(super) fn subaccount_session_from_str(source: &str) -> Result<SubaccountSession, String> {
    let value = serde_json::from_str::<Value>(source)
        .map_err(|error| format!("failed to decode Sub-account session store: {error}"))?;
    let host_url = string_by_keys(&value, &["hostUrl", "host_url", "host"])
        .ok_or_else(|| "Sub-account session store is missing hostUrl.".to_string())?;
    let token = string_by_keys(&value, &["token", "accessToken", "sessionToken"])
        .ok_or_else(|| "Sub-account session store is missing token.".to_string())?;
    let subaccount_id = string_by_keys(
        &value,
        &["subaccountId", "subAccountId", "subaccount_id", "id"],
    )
    .unwrap_or_default();
    let display_name = string_by_keys(&value, &["displayName", "display_name", "name", "label"])
        .unwrap_or_default();
    Ok(SubaccountSession {
        host_url: normalize_relay_host_url(&host_url)?,
        token,
        subaccount_id,
        display_name,
    })
}

pub(super) fn save_subaccount_session(
    path: &Path,
    session: &SubaccountSession,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create Sub-account session directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let payload = json!({
        "version": 1,
        "hostUrl": session.host_url,
        "token": session.token,
        "subaccountId": session.subaccount_id,
        "displayName": session.display_name,
    });
    fs::write(path, payload.to_string()).map_err(|error| {
        format!(
            "failed to save Sub-account session store {}: {error}",
            path.display()
        )
    })
}

pub(super) fn clear_default_subaccount_session() -> Result<(), String> {
    let path = default_subaccount_session_store_path();
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).map_err(|error| {
        format!(
            "failed to clear Sub-account session store {}: {error}",
            path.display()
        )
    })
}

pub(super) async fn login_with_subaccount_code(
    host_url: String,
    code: String,
) -> Result<SubaccountSession, String> {
    let host_url = normalize_relay_host_url(&host_url)?;
    let code = code.trim().to_string();
    if code.is_empty() {
        return Err("Sub-account code is required.".to_string());
    }
    let response = reqwest::Client::new()
        .post(api_url(&host_url, LOGIN_PATH))
        .header(ACCEPT, "application/json")
        .json(&json!({ "code": code }))
        .send()
        .await
        .map_err(|error| format!("Sub-account login request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read Sub-account login response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Sub-account login HTTP {status}: {}",
            response_body_snippet(&body)
        ));
    }
    let session = session_from_login_response(&host_url, &body)?;
    save_subaccount_session(&default_subaccount_session_store_path(), &session)?;
    Ok(session)
}

pub(super) async fn subaccount_relay_payload_for_request(
    request: &BabelServiceRequest,
    envelope: Value,
) -> Result<Option<String>, String> {
    let Some(session) = read_default_subaccount_session()? else {
        return Ok(None);
    };
    let auth = relay_auth_from_session(&session)?;
    let body = json!({
        "requestKind": relay_request_kind(request),
        "envelope": envelope,
    });
    let response = reqwest::Client::new()
        .post(api_url(&auth.host_url, BABEL_SERVICE_PATH))
        .headers(relay_headers(&auth.authorization)?)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Sub-account relay request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read Sub-account relay response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Sub-account relay HTTP {status}: {}",
            response_body_snippet(&body)
        ));
    }
    Ok(Some(extract_relay_payload(&body)))
}

pub(super) async fn subaccount_audio_presigned_urls(
    s3_urls: &[String],
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(session) = read_default_subaccount_session()? else {
        return Ok(None);
    };
    if s3_urls.is_empty() {
        return Ok(Some(BTreeMap::new()));
    }
    let auth = relay_auth_from_session(&session)?;
    let response = reqwest::Client::new()
        .post(api_url(&auth.host_url, AUDIO_PRESIGN_PATH))
        .headers(relay_headers(&auth.authorization)?)
        .json(&json!({ "s3Urls": s3_urls }))
        .send()
        .await
        .map_err(|error| format!("Sub-account audio presign request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read Sub-account audio presign response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Sub-account audio presign HTTP {status}: {}",
            response_body_snippet(&body)
        ));
    }
    parse_presigned_map(&body).map(Some)
}

pub(super) fn dispatch_subaccount_owner_requests(
    requests: Vec<SubaccountOwnerRequest>,
) -> Task<Message> {
    Task::batch(
        requests.into_iter().map(|request| {
            Task::perform(execute_subaccount_owner_request(request), |message| message)
        }),
    )
}

async fn execute_subaccount_owner_request(request: SubaccountOwnerRequest) -> Message {
    match owner_dashboard_payload_for_request(request).await {
        Ok(dashboard) => Message::SubaccountOwnerDashboardLoaded(dashboard),
        Err(error) => Message::SubaccountOwnerRequestFailed(error),
    }
}

async fn owner_dashboard_payload_for_request(
    request: SubaccountOwnerRequest,
) -> Result<SubaccountOwnerDashboard, String> {
    let host_url = match &request {
        SubaccountOwnerRequest::LoadDashboard { host_url }
        | SubaccountOwnerRequest::Create { host_url, .. }
        | SubaccountOwnerRequest::Revoke { host_url, .. }
        | SubaccountOwnerRequest::AssignProjects { host_url, .. } => {
            normalize_relay_host_url(host_url)?
        }
    };
    let authorization = mint_native_babel_authorization_from_store(
        &default_native_auth_store_path(),
        &native_babel_url(),
        &native_clerk_url(),
        OffsetDateTime::now_utc().unix_timestamp(),
    )
    .await
    .map_err(|error| {
        format!("Main Babel account auth is required for Sub-account controls: {error}")
    })?;
    let client = reqwest::Client::new();
    let response = match request {
        SubaccountOwnerRequest::LoadDashboard { .. } => {
            client
                .get(api_url(&host_url, OWNER_DASHBOARD_PATH))
                .headers(relay_headers(&authorization)?)
                .send()
                .await
        }
        SubaccountOwnerRequest::Create { label, .. } => {
            client
                .post(api_url(&host_url, OWNER_SUBACCOUNTS_PATH))
                .headers(relay_headers(&authorization)?)
                .json(&json!({ "label": label.trim() }))
                .send()
                .await
        }
        SubaccountOwnerRequest::Revoke { subaccount_id, .. } => {
            client
                .delete(api_url(
                    &host_url,
                    &format!(
                        "{}/{}",
                        OWNER_SUBACCOUNTS_PATH,
                        encode_path_segment(&subaccount_id)
                    ),
                ))
                .headers(relay_headers(&authorization)?)
                .send()
                .await
        }
        SubaccountOwnerRequest::AssignProjects {
            subaccount_id,
            project_ids,
            ..
        } => {
            client
                .put(api_url(
                    &host_url,
                    &format!(
                        "{}/{}/projects",
                        OWNER_SUBACCOUNTS_PATH,
                        encode_path_segment(&subaccount_id)
                    ),
                ))
                .headers(relay_headers(&authorization)?)
                .json(&json!({ "projectIds": project_ids }))
                .send()
                .await
        }
    }
    .map_err(|error| format!("Sub-account owner request failed: {error}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read Sub-account owner response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Sub-account owner HTTP {status}: {}",
            response_body_snippet(&body)
        ));
    }
    parse_owner_dashboard(&body)
}

fn session_from_login_response(host_url: &str, body: &str) -> Result<SubaccountSession, String> {
    let value = serde_json::from_str::<Value>(body)
        .map_err(|error| format!("failed to decode Sub-account login response: {error}"))?;
    let source = value
        .get("session")
        .or_else(|| value.get("subaccount"))
        .or_else(|| value.get("data"))
        .unwrap_or(&value);
    let token = string_by_keys(source, &["token", "accessToken", "sessionToken"])
        .or_else(|| string_by_keys(&value, &["token", "accessToken", "sessionToken"]))
        .ok_or_else(|| "Sub-account login response did not include a session token.".to_string())?;
    let subaccount_id = string_by_keys(
        source,
        &["subaccountId", "subAccountId", "subaccount_id", "id"],
    )
    .unwrap_or_default();
    let display_name = string_by_keys(source, &["displayName", "display_name", "name", "label"])
        .unwrap_or_default();
    Ok(SubaccountSession {
        host_url: host_url.to_string(),
        token,
        subaccount_id,
        display_name,
    })
}

fn relay_auth_from_session(session: &SubaccountSession) -> Result<RelayAuth, String> {
    if session.token.trim().is_empty() {
        return Err("Sub-account session token is empty.".to_string());
    }
    Ok(RelayAuth {
        host_url: normalize_relay_host_url(&session.host_url)?,
        authorization: format!("Bearer {}", session.token.trim()),
    })
}

fn relay_headers(authorization: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(authorization)
            .map_err(|error| format!("Sub-account authorization header is invalid: {error}"))?,
    );
    Ok(headers)
}

pub(super) fn relay_request_kind(request: &BabelServiceRequest) -> &'static str {
    match request {
        BabelServiceRequest::LoadWorkerProjects => "loadWorkerProjects",
        BabelServiceRequest::SaveAnnotations(_) => "saveAnnotations",
        BabelServiceRequest::LintAnnotations(_) => "lintAnnotations",
        BabelServiceRequest::SubmitReviewAction(_) => "submitReviewAction",
        BabelServiceRequest::DropWorkerAction(_) => "dropWorkerAction",
        BabelServiceRequest::SkipReviewAction(_) => "skipReviewAction",
        BabelServiceRequest::AdminRateReviewAction(_) => "adminRateReviewAction",
        BabelServiceRequest::ClaimNextFromQueue(_) => "claimNextFromQueue",
        BabelServiceRequest::ClaimByProcessedTranscription(_) => "claimByProcessedTranscription",
        BabelServiceRequest::LoadReviewAction(_) => "loadReviewAction",
        BabelServiceRequest::LoadLatestReviewAction(_) => "loadLatestReviewAction",
        BabelServiceRequest::LoadFeedbackRubricFlags(_) => "loadFeedbackRubricFlags",
        BabelServiceRequest::LoadFeedbackForm(_) => "loadFeedbackForm",
        BabelServiceRequest::LoadFeedbackInputs(_) => "loadFeedbackInputs",
        BabelServiceRequest::LoadFeedbackDraft(_) => "loadFeedbackDraft",
        BabelServiceRequest::SaveFeedbackDraft(_) => "saveFeedbackDraft",
    }
}

fn extract_relay_payload(body: &str) -> String {
    let trimmed = body.trim_start_matches('\u{feff}').trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return trimmed.to_string();
    };
    for key in ["payload", "response", "body"] {
        if let Some(payload) = value.get(key) {
            return payload
                .as_str()
                .map(ToString::to_string)
                .unwrap_or_else(|| payload.to_string());
        }
    }
    if let Some(data) = value.get("data") {
        return data.to_string();
    }
    value.to_string()
}

fn parse_presigned_map(body: &str) -> Result<BTreeMap<String, String>, String> {
    let value = serde_json::from_str::<Value>(body)
        .map_err(|error| format!("failed to decode Sub-account audio presign response: {error}"))?;
    let source = value
        .get("payload")
        .or_else(|| value.get("response"))
        .or_else(|| value.get("data"))
        .unwrap_or(&value);
    let owned;
    let source = if let Some(text) = source.as_str() {
        owned = serde_json::from_str::<Value>(text).map_err(|error| {
            format!("failed to decode Sub-account audio presign payload: {error}")
        })?;
        &owned
    } else {
        source
    };
    let data = source
        .get("urls")
        .or_else(|| source.get("presignedUrls"))
        .or_else(|| source.get("data"))
        .or_else(|| trpc_json_payload(source))
        .unwrap_or(source);
    let map = data
        .as_object()
        .ok_or_else(|| "Sub-account audio presign response was not an object.".to_string())?;
    Ok(map
        .iter()
        .filter_map(|(source, signed)| {
            signed
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|signed| (source.clone(), signed.to_string()))
        })
        .collect())
}

fn trpc_json_payload(value: &Value) -> Option<&Value> {
    if let Value::Array(items) = value {
        return items.first().and_then(trpc_json_payload);
    }
    value
        .pointer("/result/data/json")
        .or_else(|| value.pointer("/data/json"))
        .or_else(|| value.pointer("/result/data"))
        .or_else(|| value.pointer("/data"))
}

pub(super) fn parse_owner_dashboard(body: &str) -> Result<SubaccountOwnerDashboard, String> {
    let value = serde_json::from_str::<Value>(body)
        .map_err(|error| format!("failed to decode Sub-account owner response: {error}"))?;
    let source = value.get("data").unwrap_or(&value);
    let Some(items) = source
        .get("subaccounts")
        .or_else(|| source.get("subAccounts"))
        .or_else(|| source.get("items"))
        .and_then(Value::as_array)
    else {
        return Ok(SubaccountOwnerDashboard::default());
    };
    Ok(SubaccountOwnerDashboard {
        subaccounts: items
            .iter()
            .filter_map(parse_managed_subaccount)
            .filter(|subaccount| !subaccount.revoked)
            .collect(),
    })
}

fn parse_managed_subaccount(value: &Value) -> Option<ManagedSubaccount> {
    let id = string_by_keys(
        value,
        &["id", "subaccountId", "subAccountId", "subaccount_id"],
    )?;
    let label = string_by_keys(value, &["label", "name", "displayName", "display_name"])
        .unwrap_or_else(|| id.clone());
    let code = string_by_keys(value, &["code", "specialCode", "inviteCode", "loginCode"]);
    let revoked = bool_by_keys(value, &["revoked", "isRevoked", "disabled"]).unwrap_or(false);
    let allowed_project_ids = string_array_by_keys(
        value,
        &[
            "allowedProjectIds",
            "projectIds",
            "assignedProjectIds",
            "allowed_projects",
        ],
    );
    let analytics = value.get("analytics").unwrap_or(value);
    let completed_tasks = u64_by_keys(
        analytics,
        &["completedTasks", "taskCount", "tasksCompleted", "tasks"],
    )
    .unwrap_or(0);
    let worked_seconds = u64_by_keys(
        analytics,
        &[
            "workedSeconds",
            "timeSeconds",
            "workSeconds",
            "durationSeconds",
        ],
    )
    .unwrap_or(0);
    let earnings_cents = i64_by_keys(
        analytics,
        &["earningsCents", "moneyCents", "earnedCents", "payoutCents"],
    )
    .unwrap_or(0);
    Some(ManagedSubaccount {
        id,
        label,
        code,
        revoked,
        allowed_project_ids,
        completed_tasks,
        worked_seconds,
        earnings_cents,
    })
}

fn api_url(host_url: &str, path: &str) -> String {
    format!("{}{}", host_url.trim_end_matches('/'), path)
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn string_array_by_keys(value: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(items) = value.get(*key).and_then(Value::as_array) {
            return items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect();
        }
    }
    Vec::new()
}

fn bool_by_keys(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn u64_by_keys(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
        })
    })
}

fn i64_by_keys(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_i64))
}

fn response_body_snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() > 300 {
        trimmed.chars().take(300).collect::<String>() + "..."
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_relay_host_url_with_default_scheme() {
        assert_eq!(
            normalize_relay_host_url("relay.example.test/").expect("normalized"),
            "https://relay.example.test"
        );
    }

    #[test]
    fn parses_trpc_audio_presign_response() {
        let urls = parse_presigned_map(
            r#"[{"result":{"data":{"json":{"s3://bucket/audio.wav":"https://signed.example/audio.wav"}}}}]"#,
        )
        .expect("presigned urls");

        assert_eq!(
            urls.get("s3://bucket/audio.wav").map(String::as_str),
            Some("https://signed.example/audio.wav")
        );
    }

    #[test]
    fn parses_session_store() {
        let session = subaccount_session_from_str(
            r#"{"hostUrl":"http://127.0.0.1:3000/","accessToken":"secret","subaccountId":"sub-1","displayName":"Desk"}"#,
        )
        .expect("session");

        assert_eq!(session.host_url, "http://127.0.0.1:3000");
        assert_eq!(session.token, "secret");
        assert_eq!(session.subaccount_id, "sub-1");
        assert_eq!(session.display_name, "Desk");
    }

    #[test]
    fn relay_settings_round_trip_normalized_host_url() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("relay-settings.json");

        let normalized =
            save_subaccount_relay_host_url(&path, "127.0.0.1:8787/").expect("saved settings");

        assert_eq!(normalized, "http://127.0.0.1:8787");
        assert_eq!(
            read_subaccount_relay_host_url(&path).expect("read settings"),
            Some("http://127.0.0.1:8787".to_string())
        );
    }

    #[test]
    fn relay_settings_parse_legacy_host_keys() {
        assert_eq!(
            subaccount_relay_host_url_from_str(r#"{"hostUrl":"http://localhost:8787/"}"#)
                .expect("parsed settings"),
            "http://localhost:8787"
        );
    }

    #[test]
    fn parses_owner_dashboard_analytics() {
        let dashboard = parse_owner_dashboard(
            r#"{"subaccounts":[{"id":"sub-1","label":"Laptop","code":"CODE1","allowedProjectIds":["p1","p2"],"analytics":{"completedTasks":3,"workedSeconds":4500,"earningsCents":1234}}]}"#,
        )
        .expect("dashboard");

        assert_eq!(dashboard.subaccounts.len(), 1);
        assert_eq!(
            dashboard.subaccounts[0].allowed_project_ids,
            vec!["p1", "p2"]
        );
        assert_eq!(dashboard.subaccounts[0].completed_tasks, 3);
        assert_eq!(dashboard.subaccounts[0].worked_seconds, 4500);
        assert_eq!(dashboard.subaccounts[0].earnings_cents, 1234);
    }
    #[test]
    fn owner_dashboard_drops_legacy_revoked_rows() {
        let dashboard = parse_owner_dashboard(
            r#"{"subaccounts":[
                {"id":"sub-active","label":"Active","code":"ACTIVE","revoked":false},
                {"id":"sub-revoked","label":"Revoked","code":"OLD-SECRET","revoked":true},
                {"id":"sub-is-revoked","label":"Legacy revoked","code":"OLD-SECRET-2","isRevoked":true},
                {"id":"sub-disabled","label":"Legacy disabled","code":"OLD-SECRET-3","disabled":true}
            ]}"#,
        )
        .expect("dashboard");

        assert_eq!(dashboard.subaccounts.len(), 1);
        assert_eq!(dashboard.subaccounts[0].id, "sub-active");
        assert_eq!(dashboard.subaccounts[0].code.as_deref(), Some("ACTIVE"));
        assert!(!dashboard.subaccounts[0].revoked);
    }
}
