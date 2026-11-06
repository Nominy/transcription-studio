//! Sub-accounts: code-login sessions routed through a `studio-relay` host, the remembered relay
//! URL, Babel-request relaying for active sessions, and owner management APIs ([`owner`]).
//! A session lives in its account directory (`subaccount.json`, see [`crate::account`]).
//!
//! Env: `STUDIO_SUBACCOUNT_RELAY_SETTINGS` (remembered relay URL file),
//! `STUDIO_SUBACCOUNT_RELAY_URL` (relay URL override).

pub mod owner;

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};

pub const SUBACCOUNT_RELAY_SETTINGS_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_SETTINGS";
pub const SUBACCOUNT_RELAY_URL_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_URL";

const APP_SUPPORT_DIR_NAME: &str = "Exaudio";
/// File name of a relay session inside an account directory.
pub const SESSION_FILE_NAME: &str = "subaccount.json";
const RELAY_SETTINGS_FILE_NAME: &str = "subaccount-relay-settings.json";
const LOGIN_PATH: &str = "/api/subaccounts/login";
const BABEL_SERVICE_PATH: &str = "/api/subaccounts/babel-service";
const AUDIO_PRESIGN_PATH: &str = "/api/subaccounts/audio/presigned-urls";

/// Persisted login, explicitly supplied to a client when its account is opened.
#[derive(Clone, PartialEq, Eq)]
pub struct SubaccountSession {
    /// Normalized relay host URL (no trailing `/`).
    pub host_url: String,
    pub token: String,
    pub subaccount_id: String,
    pub display_name: String,
}

impl std::fmt::Debug for SubaccountSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubaccountSession")
            .field("host_url", &self.host_url)
            .field("token", &"[redacted]")
            .field("subaccount_id", &self.subaccount_id)
            .field("display_name", &self.display_name)
            .finish()
    }
}

/// `<account dir>/subaccount.json`.
pub fn session_store_path(account_dir: &Path) -> PathBuf {
    account_dir.join(SESSION_FILE_NAME)
}

pub fn relay_settings_path() -> PathBuf {
    env::var_os(SUBACCOUNT_RELAY_SETTINGS_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            studio_services::paths::app_support_dir_named(APP_SUPPORT_DIR_NAME)
                .join(RELAY_SETTINGS_FILE_NAME)
        })
}

/// `Ok(None)` when the file is missing or blank.
pub fn read_session(path: &Path) -> Result<Option<SubaccountSession>> {
    let Some(source) = read_optional(path, "Sub-account session store")? else {
        return Ok(None);
    };
    parse_session(&source).map(Some)
}

/// Accepts `hostUrl|host_url|host`, `token|accessToken|sessionToken`,
/// `subaccountId|subAccountId|subaccount_id|id`, `displayName|display_name|name|label`.
pub fn parse_session(source: &str) -> Result<SubaccountSession> {
    let value = serde_json::from_str::<Value>(source)
        .context("failed to decode Sub-account session store")?;
    let host_url = string_by_keys(&value, &["hostUrl", "host_url", "host"])
        .ok_or_else(|| anyhow!("Sub-account session store is missing hostUrl."))?;
    let token = string_by_keys(&value, &["token", "accessToken", "sessionToken"])
        .ok_or_else(|| anyhow!("Sub-account session store is missing token."))?;
    Ok(SubaccountSession {
        host_url: normalize_relay_host_url(&host_url)?,
        token,
        subaccount_id: string_by_keys(
            &value,
            &["subaccountId", "subAccountId", "subaccount_id", "id"],
        )
        .unwrap_or_default(),
        display_name: string_by_keys(&value, &["displayName", "display_name", "name", "label"])
            .unwrap_or_default(),
    })
}

pub fn save_session(path: &Path, session: &SubaccountSession) -> Result<()> {
    write_json(
        path,
        &json!({
            "version": 1,
            "hostUrl": session.host_url,
            "token": session.token,
            "subaccountId": session.subaccount_id,
            "displayName": session.display_name,
        }),
        "Sub-account session store",
    )
}

/// Deletes a session file; a missing file is not an error.
pub fn delete_session(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to clear Sub-account session store {}",
                path.display()
            )
        }),
    }
}

/// Remembered relay host URL (`Ok(None)` when unset).
pub fn read_relay_host_url(path: &Path) -> Result<Option<String>> {
    let Some(source) = read_optional(path, "Sub-account relay settings")? else {
        return Ok(None);
    };
    parse_relay_settings(&source).map(Some)
}

/// Normalizes and remembers the relay host URL; returns the normalized URL.
pub fn save_relay_host_url(path: &Path, host_url: &str) -> Result<String> {
    let normalized = normalize_relay_host_url(host_url)?;
    write_json(
        path,
        &json!({ "version": 1, "relayHostUrl": normalized }),
        "Sub-account relay settings",
    )?;
    Ok(normalized)
}

pub fn save_default_relay_host_url(host_url: &str) -> Result<String> {
    save_relay_host_url(&relay_settings_path(), host_url)
}

/// Accepts `relayHostUrl|relay_host_url|hostUrl|host`.
pub fn parse_relay_settings(source: &str) -> Result<String> {
    let value = serde_json::from_str::<Value>(source)
        .context("failed to decode Sub-account relay settings")?;
    let host_url = string_by_keys(
        &value,
        &["relayHostUrl", "relay_host_url", "hostUrl", "host"],
    )
    .ok_or_else(|| anyhow!("Sub-account relay settings are missing relayHostUrl."))?;
    normalize_relay_host_url(&host_url)
}

/// Relay URL for login prompts and owner controls: `STUDIO_SUBACCOUNT_RELAY_URL`, else the
/// remembered settings, else empty.
pub fn default_relay_host_url() -> String {
    env::var(SUBACCOUNT_RELAY_URL_ENV)
        .ok()
        .and_then(|value| normalize_relay_host_url(&value).ok())
        .or_else(|| read_relay_host_url(&relay_settings_path()).ok().flatten())
        .unwrap_or_default()
}

/// Adds a scheme when missing (`http` for localhost/127.x/::1, else `https`), requires
/// http(s) and a host, strips trailing `/`.
pub fn normalize_relay_host_url(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("Sub-account host URL is required.");
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("{}://{trimmed}", default_scheme(trimmed))
    };
    let parsed = url::Url::parse(&candidate).context("Sub-account host URL is invalid")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!(
            "Sub-account host URL must use http or https, not {}.",
            parsed.scheme()
        );
    }
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        bail!("Sub-account host URL must include a host.");
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

/// Logs in with a one-time sub-account code. The caller persists the session.
pub async fn login_with_code(host_url: &str, code: &str) -> Result<SubaccountSession> {
    let host_url = normalize_relay_host_url(host_url)?;
    let code = code.trim();
    if code.is_empty() {
        bail!("Sub-account code is required.");
    }
    let response = studio_services::http::client()
        .post(api_url(&host_url, LOGIN_PATH))
        .header(ACCEPT, "application/json")
        .json(&json!({ "code": code }))
        .send()
        .await
        .context("Sub-account login request failed")?;
    let body = success_body(response, "Sub-account login").await?;
    parse_login_response(&host_url, &body)
}

/// Session from a login response (`session`/`subaccount`/`data` wrapper or flat).
pub fn parse_login_response(host_url: &str, body: &str) -> Result<SubaccountSession> {
    let value = serde_json::from_str::<Value>(body)
        .context("failed to decode Sub-account login response")?;
    let source = value
        .get("session")
        .or_else(|| value.get("subaccount"))
        .or_else(|| value.get("data"))
        .unwrap_or(&value);
    let token = string_by_keys(source, &["token", "accessToken", "sessionToken"])
        .or_else(|| string_by_keys(&value, &["token", "accessToken", "sessionToken"]))
        .ok_or_else(|| anyhow!("Sub-account login response did not include a session token."))?;
    Ok(SubaccountSession {
        host_url: host_url.to_string(),
        token,
        subaccount_id: string_by_keys(
            source,
            &["subaccountId", "subAccountId", "subaccount_id", "id"],
        )
        .unwrap_or_default(),
        display_name: string_by_keys(source, &["displayName", "display_name", "name", "label"])
            .unwrap_or_default(),
    })
}

/// Relay one envelope with the supplied document session; never select another account.
pub async fn relay_babel_payload(
    session: &SubaccountSession,
    request_kind: &str,
    envelope: Value,
) -> Result<String> {
    let response = studio_services::http::client()
        .post(api_url(&session.host_url, BABEL_SERVICE_PATH))
        .headers(session_headers(session)?)
        .json(&json!({ "requestKind": request_kind, "envelope": envelope }))
        .send()
        .await
        .context("Sub-account relay request failed")?;
    let body = success_body(response, "Sub-account relay").await?;
    Ok(parse_relay_payload(&body))
}

/// Presign track URLs using the supplied document's relay session.
pub async fn audio_presigned_urls(
    session: &SubaccountSession,
    review_action_id: &str,
    chunked_processed_recording_ids: &[String],
) -> Result<BTreeMap<String, String>> {
    if chunked_processed_recording_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let response = studio_services::http::client()
        .post(api_url(&session.host_url, AUDIO_PRESIGN_PATH))
        .headers(session_headers(session)?)
        .json(&json!({
            "reviewActionId": review_action_id,
            "chunkedProcessedRecordingIds": chunked_processed_recording_ids,
        }))
        .send()
        .await
        .context("Sub-account audio presign request failed")?;
    let body = success_body(response, "Sub-account audio presign").await?;
    parse_presigned_urls(&body)
}

/// Unwraps a relay response: `payload`/`response`/`body` (string verbatim, else JSON text),
/// then `data`, else the whole body.
pub fn parse_relay_payload(body: &str) -> String {
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
    value.get("data").unwrap_or(&value).to_string()
}

/// `{ <s3 url>: <signed url> }` from plain, wrapped, string-encoded, or tRPC batch responses.
pub fn parse_presigned_urls(body: &str) -> Result<BTreeMap<String, String>> {
    let value = serde_json::from_str::<Value>(body)
        .context("failed to decode Sub-account audio presign response")?;
    let source = value
        .get("payload")
        .or_else(|| value.get("response"))
        .or_else(|| value.get("data"))
        .unwrap_or(&value);
    let decoded;
    let source = match source.as_str() {
        Some(text) => {
            decoded = serde_json::from_str::<Value>(text)
                .context("failed to decode Sub-account audio presign payload")?;
            &decoded
        }
        None => source,
    };
    let data = source
        .get("urls")
        .or_else(|| source.get("presignedUrls"))
        .or_else(|| source.get("data"))
        .or_else(|| trpc_json_payload(source))
        .unwrap_or(source);
    let map = data
        .as_object()
        .ok_or_else(|| anyhow!("Sub-account audio presign response was not an object."))?;
    Ok(map
        .iter()
        .filter_map(|(source, signed)| {
            let signed = signed.as_str()?.trim();
            (!signed.is_empty()).then(|| (source.clone(), signed.to_string()))
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

fn session_headers(session: &SubaccountSession) -> Result<HeaderMap> {
    let token = session.token.trim();
    if token.is_empty() {
        bail!("Sub-account session token is empty.");
    }
    json_headers(&format!("Bearer {token}"))
}

pub(crate) fn json_headers(authorization: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(authorization)
            .context("Sub-account authorization header is invalid")?,
    );
    Ok(headers)
}

/// Body of a 2xx response; otherwise `<what> HTTP <status>: <first 300 chars>`.
pub(crate) async fn success_body(response: reqwest::Response, what: &str) -> Result<String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .with_context(|| format!("failed to read {what} response"))?;
    if !status.is_success() {
        bail!("{what} HTTP {status}: {}", body_snippet(&body));
    }
    Ok(body)
}

pub(crate) fn api_url(host_url: &str, path: &str) -> String {
    format!("{}{}", host_url.trim_end_matches('/'), path)
}

pub(crate) fn string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn body_snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() > 300 {
        trimmed.chars().take(300).collect::<String>() + "..."
    } else {
        trimmed.to_string()
    }
}

fn default_scheme(value_without_scheme: &str) -> &'static str {
    let authority = value_without_scheme
        .split('/')
        .next()
        .unwrap_or(value_without_scheme)
        .trim();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match authority.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next().unwrap_or(ipv6),
        None => authority.split(':').next().unwrap_or(authority),
    };
    if host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.") {
        "http"
    } else {
        "https"
    }
}

fn read_optional(path: &Path, what: &str) -> Result<Option<String>> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {what} {}", path.display()));
        }
    };
    Ok((!source.trim().is_empty()).then_some(source))
}

fn write_json(path: &Path, value: &Value, what: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {what} directory {}", parent.display()))?;
    }
    fs::write(path, value.to_string())
        .with_context(|| format!("failed to save {what} {}", path.display()))
}
