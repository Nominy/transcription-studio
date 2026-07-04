use std::{
    env, fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, SET_COOKIE};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::platform_paths::application_support_dir_named;

const CLERK_INSTANCE_SUFFIX: &str = "v1aVht_F";
const CLERK_API_VERSION: &str = "2025-11-10";
const CLERK_JS_VERSION: &str = "5.127.0";
const DEFAULT_CLERK_URL: &str = "https://clerk.babel.audio";
const STUDIO_BABEL_AUTH_STORE: &str = "STUDIO_BABEL_AUTH_STORE";
const STUDIO_BABEL_CLERK_URL: &str = "STUDIO_BABEL_CLERK_URL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeAuthCookie {
    pub(super) name: String,
    pub(super) value: String,
    pub(super) domain: String,
    pub(super) path: String,
    pub(super) secure: bool,
    pub(super) http_only: bool,
    pub(super) same_site: String,
    pub(super) expires_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct NativeAuthStore {
    pub(super) cookies: Vec<NativeAuthCookie>,
    pub(super) pending_sign_in_id: Option<String>,
    pub(super) pending_phone_number_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PhoneCodeLoginStart {
    pub(super) sign_in_id: String,
    pub(super) phone_number_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PhoneCodeLoginComplete {
    pub(super) session_id: String,
    pub(super) cookie: String,
}

pub(super) fn native_auth_store_from_str(source: &str) -> Result<NativeAuthStore, String> {
    let value: Value = serde_json::from_str(source)
        .map_err(|error| format!("failed to decode native Babel auth store: {error}"))?;
    let cookies = value
        .get("cookies")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(cookie_from_value)
        .collect();
    let pending_sign_in_id = value
        .pointer("/pendingSignIn/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty());
    let pending_phone_number_id = value
        .pointer("/pendingSignIn/phoneNumberId")
        .or_else(|| value.pointer("/pendingSignIn/phone_number_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty());
    Ok(NativeAuthStore {
        cookies,
        pending_sign_in_id,
        pending_phone_number_id,
    })
}

pub(super) fn read_native_auth_store(path: &Path) -> Result<NativeAuthStore, String> {
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read native Babel auth store {}: {error}",
            path.display()
        )
    })?;
    native_auth_store_from_str(&source)
}

fn read_native_auth_store_or_default(path: &Path) -> Result<NativeAuthStore, String> {
    match fs::read_to_string(path) {
        Ok(source) => native_auth_store_from_str(&source),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(NativeAuthStore::default())
        }
        Err(error) => Err(format!(
            "failed to read native Babel auth store {}: {error}",
            path.display()
        )),
    }
}

pub(super) fn save_native_auth_store(path: &Path, store: &NativeAuthStore) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create native Babel auth store directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let mut root = serde_json::Map::new();
    root.insert("version".to_string(), Value::from(1));
    root.insert(
        "updatedAt".to_string(),
        Value::from(
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_else(|_| "now".to_string()),
        ),
    );
    root.insert(
        "cookies".to_string(),
        Value::Array(store.cookies.iter().map(cookie_to_value).collect()),
    );
    if let Some(id) = store
        .pending_sign_in_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let mut pending = serde_json::Map::new();
        pending.insert("id".to_string(), Value::from(id.to_string()));
        if let Some(phone_number_id) = store
            .pending_phone_number_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            pending.insert(
                "phoneNumberId".to_string(),
                Value::from(phone_number_id.to_string()),
            );
        }
        root.insert("pendingSignIn".to_string(), Value::Object(pending));
    }

    let payload = serde_json::to_string_pretty(&Value::Object(root))
        .map_err(|error| format!("failed to encode native Babel auth store: {error}"))?;
    fs::write(path, payload).map_err(|error| {
        format!(
            "failed to write native Babel auth store {}: {error}",
            path.display()
        )
    })
}

pub(super) fn default_native_auth_store_path() -> PathBuf {
    if let Ok(path) = env::var(STUDIO_BABEL_AUTH_STORE)
        && !path.trim().is_empty()
    {
        return PathBuf::from(path.trim());
    }
    application_support_dir_named("TranscriptionStudio").join("babel-auth.json")
}

pub(super) fn native_clerk_url() -> String {
    env::var(STUDIO_BABEL_CLERK_URL)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CLERK_URL.to_string())
}

pub(super) async fn start_phone_code_login(
    store_path: &Path,
    identifier: &str,
    babel_url: &str,
    clerk_url: &str,
    reset: bool,
    now_unix_seconds: i64,
) -> Result<PhoneCodeLoginStart, String> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Err("phone-code login requires an identifier".to_string());
    }
    let mut store = if reset {
        NativeAuthStore::default()
    } else {
        read_native_auth_store_or_default(store_path)?
    };
    let clerk_url = clerk_url.trim().trim_end_matches('/');
    let babel_url = babel_url.trim().trim_end_matches('/');
    if clerk_url.is_empty() {
        return Err("native Babel auth Clerk URL is empty".to_string());
    }
    if babel_url.is_empty() {
        return Err("native Babel auth Babel URL is empty".to_string());
    }

    let client = reqwest::Client::new();
    let sign_in_url = clerk_api_url(clerk_url, "/v1/client/sign_ins");
    let clerk_cookie = cookie_header_for_url(&store, &sign_in_url, now_unix_seconds)?;
    let sign_in_response = request_clerk_json(
        &client,
        "POST",
        &sign_in_url,
        babel_url,
        &clerk_cookie,
        Some(form_body(&[("identifier", identifier)])),
        now_unix_seconds,
    )
    .await?;
    merge_response_cookies(&mut store, sign_in_response.set_cookies, now_unix_seconds);
    let sign_in = clerk_response_source(&sign_in_response.value);
    let sign_in_id = string_field(sign_in, &["id"])
        .ok_or_else(|| "Clerk sign-in response did not include id".to_string())?;
    let phone_number_id = phone_code_factor_id(sign_in)?;

    let prepare_url = clerk_api_url(
        clerk_url,
        &format!(
            "/v1/client/sign_ins/{}/prepare_first_factor",
            encode_path_segment(&sign_in_id)
        ),
    );
    let clerk_cookie = cookie_header_for_url(&store, &prepare_url, now_unix_seconds)?;
    let prepare_response = request_clerk_json(
        &client,
        "POST",
        &prepare_url,
        babel_url,
        &clerk_cookie,
        Some(form_body(&[
            ("strategy", "phone_code"),
            ("phone_number_id", &phone_number_id),
        ])),
        now_unix_seconds,
    )
    .await?;
    merge_response_cookies(&mut store, prepare_response.set_cookies, now_unix_seconds);
    store.pending_sign_in_id = Some(sign_in_id.clone());
    store.pending_phone_number_id = Some(phone_number_id.clone());
    save_native_auth_store(store_path, &store)?;

    Ok(PhoneCodeLoginStart {
        sign_in_id,
        phone_number_id,
    })
}

pub(super) async fn verify_phone_code_login(
    store_path: &Path,
    code: &str,
    babel_url: &str,
    clerk_url: &str,
    now_unix_seconds: i64,
) -> Result<PhoneCodeLoginComplete, String> {
    let code = code.trim();
    if code.is_empty() {
        return Err("phone verification code is empty".to_string());
    }
    let mut store = read_native_auth_store(store_path)?;
    let Some(sign_in_id) = store
        .pending_sign_in_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return Err(
            "native Babel auth store has no pending phone-code login; run `studio-app --babel-auth-start-login <phone>`"
                .to_string(),
        );
    };
    let clerk_url = clerk_url.trim().trim_end_matches('/');
    let babel_url = babel_url.trim().trim_end_matches('/');
    if clerk_url.is_empty() {
        return Err("native Babel auth Clerk URL is empty".to_string());
    }
    if babel_url.is_empty() {
        return Err("native Babel auth Babel URL is empty".to_string());
    }

    let client = reqwest::Client::new();
    let attempt_url = clerk_api_url(
        clerk_url,
        &format!(
            "/v1/client/sign_ins/{}/attempt_first_factor",
            encode_path_segment(&sign_in_id)
        ),
    );
    let clerk_cookie = cookie_header_for_url(&store, &attempt_url, now_unix_seconds)?;
    let attempt_response = request_clerk_json(
        &client,
        "POST",
        &attempt_url,
        babel_url,
        &clerk_cookie,
        Some(form_body(&[("strategy", "phone_code"), ("code", code)])),
        now_unix_seconds,
    )
    .await?;
    merge_response_cookies(&mut store, attempt_response.set_cookies, now_unix_seconds);
    let Some(session_id) = active_session_id(&attempt_response.value) else {
        return Err("phone-code sign-in did not create an active session".to_string());
    };

    let token_url = clerk_api_url(
        clerk_url,
        &format!(
            "/v1/client/sessions/{}/tokens",
            encode_path_segment(&session_id)
        ),
    );
    let clerk_cookie = cookie_header_for_url(&store, &token_url, now_unix_seconds)?;
    let token_response = request_clerk_json(
        &client,
        "POST",
        &token_url,
        babel_url,
        &clerk_cookie,
        Some(form_body(&[("organization_id", "")])),
        now_unix_seconds,
    )
    .await?;
    merge_response_cookies(&mut store, token_response.set_cookies, now_unix_seconds);
    store.pending_sign_in_id = None;
    store.pending_phone_number_id = None;
    save_native_auth_store(store_path, &store)?;
    let jwt = token_response
        .value
        .get("jwt")
        .and_then(Value::as_str)
        .ok_or_else(|| "Clerk token response did not include jwt".to_string())?;
    let cookie = babel_backend_cookie_header(jwt, now_unix_seconds)?;
    Ok(PhoneCodeLoginComplete { session_id, cookie })
}

#[cfg(test)]
async fn mint_native_babel_cookie_from_store(
    store_path: &Path,
    babel_url: &str,
    clerk_url: &str,
    now_unix_seconds: i64,
) -> Result<String, String> {
    let jwt = mint_native_babel_jwt_from_store(store_path, babel_url, clerk_url, now_unix_seconds)
        .await?;
    babel_backend_cookie_header(&jwt, now_unix_seconds)
}

pub(super) async fn mint_native_babel_authorization_from_store(
    store_path: &Path,
    babel_url: &str,
    clerk_url: &str,
    now_unix_seconds: i64,
) -> Result<String, String> {
    let jwt = mint_native_babel_jwt_from_store(store_path, babel_url, clerk_url, now_unix_seconds)
        .await?;
    Ok(format!("Bearer {jwt}"))
}

async fn mint_native_babel_jwt_from_store(
    store_path: &Path,
    babel_url: &str,
    clerk_url: &str,
    now_unix_seconds: i64,
) -> Result<String, String> {
    let mut store = read_native_auth_store(store_path)?;
    let clerk_url = clerk_url.trim().trim_end_matches('/');
    let babel_url = babel_url.trim().trim_end_matches('/');
    if clerk_url.is_empty() {
        return Err("native Babel auth Clerk URL is empty".to_string());
    }
    if babel_url.is_empty() {
        return Err("native Babel auth Babel URL is empty".to_string());
    }

    let client = reqwest::Client::new();
    let client_url = clerk_api_url(clerk_url, "/v1/client");
    let clerk_cookie = cookie_header_for_url(&store, &client_url, now_unix_seconds)?;
    if clerk_cookie.trim().is_empty() {
        return Err(format!(
            "native Babel auth store {} has no Clerk cookies; run `studio-app --babel-auth-start-login <phone>`",
            store_path.display()
        ));
    }

    let clerk_client = request_clerk_json(
        &client,
        "GET",
        &client_url,
        babel_url,
        &clerk_cookie,
        None,
        now_unix_seconds,
    )
    .await?;
    if merge_response_cookies(&mut store, clerk_client.set_cookies, now_unix_seconds) {
        save_native_auth_store(store_path, &store)?;
    }
    let Some(session_id) = active_session_id(&clerk_client.value) else {
        if store.pending_sign_in_id.is_some() {
            return Err(
                "native Babel auth store has a pending phone-code login; run `studio-app --babel-auth-verify-login <code>`"
                    .to_string(),
            );
        }
        return Err(
            "native Babel auth store has no active Clerk session; run `studio-app --babel-auth-start-login <phone>`"
                .to_string(),
        );
    };

    let token_url = clerk_api_url(
        clerk_url,
        &format!("/v1/client/sessions/{session_id}/tokens"),
    );
    let clerk_cookie = cookie_header_for_url(&store, &token_url, now_unix_seconds)?;
    let token_response = request_clerk_json(
        &client,
        "POST",
        &token_url,
        babel_url,
        &clerk_cookie,
        Some(form_body(&[("organization_id", "")])),
        now_unix_seconds,
    )
    .await?;
    merge_response_cookies(&mut store, token_response.set_cookies, now_unix_seconds);
    store.pending_sign_in_id = None;
    store.pending_phone_number_id = None;
    save_native_auth_store(store_path, &store)?;
    let jwt = token_response
        .value
        .get("jwt")
        .and_then(Value::as_str)
        .ok_or_else(|| "Clerk token response did not include jwt".to_string())?;
    Ok(jwt.to_string())
}

pub(super) fn cookie_header_for_url(
    store: &NativeAuthStore,
    url: &str,
    now_unix_seconds: i64,
) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| format!("native auth cookie URL is invalid: {error}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "native auth cookie URL has no host".to_string())?
        .to_ascii_lowercase();
    let path = parsed.path();
    Ok(store
        .cookies
        .iter()
        .filter(|cookie| cookie_matches(cookie, &host, path, now_unix_seconds))
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; "))
}

pub(super) fn babel_backend_cookie_header(
    jwt: &str,
    now_unix_seconds: i64,
) -> Result<String, String> {
    if jwt.trim().is_empty() {
        return Err("cannot build Babel cookie header without a Clerk session jwt".to_string());
    }
    Ok(format!(
        "__session={jwt}; __session_{CLERK_INSTANCE_SUFFIX}={jwt}; __client_uat={now_unix_seconds}; __client_uat_{CLERK_INSTANCE_SUFFIX}={now_unix_seconds}"
    ))
}

pub(super) fn active_session_id(value: &Value) -> Option<String> {
    let source = value.get("response").unwrap_or(value);
    source
        .get("last_active_session_id")
        .and_then(Value::as_str)
        .or_else(|| source.get("created_session_id").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            source
                .get("sessions")
                .and_then(Value::as_array)
                .and_then(|sessions| {
                    sessions
                        .iter()
                        .find(|session| {
                            session
                                .get("status")
                                .and_then(Value::as_str)
                                .is_some_and(|status| status == "active")
                        })
                        .or_else(|| sessions.first())
                })
                .and_then(|session| session.get("id").and_then(Value::as_str))
                .map(str::to_string)
        })
}

fn clerk_response_source(value: &Value) -> &Value {
    value.get("response").unwrap_or(value)
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
}

fn phone_code_factor_id(sign_in: &Value) -> Result<String, String> {
    let factors = sign_in
        .get("supported_first_factors")
        .or_else(|| sign_in.get("supportedFirstFactors"))
        .or_else(|| sign_in.get("first_factors"))
        .and_then(Value::as_array)
        .ok_or_else(|| "Clerk sign-in response did not include first factors".to_string())?;

    let strategies = factors
        .iter()
        .filter_map(|factor| factor.get("strategy").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(",");
    factors
        .iter()
        .find(|factor| {
            factor
                .get("strategy")
                .and_then(Value::as_str)
                .is_some_and(|strategy| strategy == "phone_code")
        })
        .and_then(|factor| {
            factor
                .get("phone_number_id")
                .or_else(|| factor.get("phoneNumberId"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "phone_code is not an available first factor; available={}",
                if strategies.is_empty() {
                    "none"
                } else {
                    strategies.as_str()
                }
            )
        })
}

fn form_body(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish()
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn cookie_from_value(value: &Value) -> Option<NativeAuthCookie> {
    Some(NativeAuthCookie {
        name: value.get("name")?.as_str()?.to_string(),
        value: value.get("value")?.as_str()?.to_string(),
        domain: value
            .get("domain")?
            .as_str()?
            .trim_start_matches('.')
            .to_ascii_lowercase(),
        path: value
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("/")
            .to_string(),
        secure: value
            .get("secure")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        http_only: value
            .get("httpOnly")
            .or_else(|| value.get("http_only"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        same_site: value
            .get("sameSite")
            .or_else(|| value.get("same_site"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        expires_at: normalized_expires_at_millis(
            value
                .get("expiresAt")
                .and_then(Value::as_i64)
                .or_else(|| value.get("expires_at").and_then(Value::as_i64)),
        ),
    })
}

fn cookie_to_value(cookie: &NativeAuthCookie) -> Value {
    let mut value = serde_json::Map::new();
    value.insert("name".to_string(), Value::from(cookie.name.clone()));
    value.insert("value".to_string(), Value::from(cookie.value.clone()));
    value.insert("domain".to_string(), Value::from(cookie.domain.clone()));
    value.insert("path".to_string(), Value::from(cookie.path.clone()));
    value.insert("secure".to_string(), Value::from(cookie.secure));
    value.insert("httpOnly".to_string(), Value::from(cookie.http_only));
    if !cookie.same_site.is_empty() {
        value.insert(
            "sameSite".to_string(),
            Value::from(cookie.same_site.clone()),
        );
    }
    if let Some(expires_at) = cookie.expires_at {
        value.insert("expiresAt".to_string(), Value::from(expires_at));
    }
    Value::Object(value)
}

fn clerk_api_url(base_url: &str, path: &str) -> String {
    format!(
        "{base_url}{path}?__clerk_api_version={CLERK_API_VERSION}&_clerk_js_version={CLERK_JS_VERSION}"
    )
}

struct ClerkJsonResponse {
    value: Value,
    set_cookies: Vec<NativeAuthCookie>,
}

async fn request_clerk_json(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    babel_url: &str,
    cookie_header: &str,
    body: Option<String>,
    now_unix_seconds: i64,
) -> Result<ClerkJsonResponse, String> {
    let mut request = match method {
        "POST" => client.post(url),
        _ => client.get(url),
    }
    .header(ACCEPT, "*/*")
    .header("origin", babel_url)
    .header("referer", format!("{babel_url}/"))
    .header(COOKIE, cookie_header);
    if let Some(body) = body {
        request = request
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("native Babel auth Clerk request failed: {error}"))?;
    let status = response.status();
    let set_cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .filter_map(|header| {
            parse_set_cookie(header, url, now_unix_seconds)
                .ok()
                .flatten()
        })
        .collect::<Vec<_>>();
    let text = response
        .text()
        .await
        .map_err(|error| format!("failed to read native Babel auth Clerk response: {error}"))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("failed to decode native Babel auth Clerk response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "native Babel auth Clerk HTTP {status}: {}",
            response_body_snippet(&text)
        ));
    }
    Ok(ClerkJsonResponse { value, set_cookies })
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

fn cookie_matches(
    cookie: &NativeAuthCookie,
    host: &str,
    path: &str,
    now_unix_seconds: i64,
) -> bool {
    let now_millis = now_unix_seconds.saturating_mul(1_000);
    if cookie
        .expires_at
        .is_some_and(|expires_at| expires_at <= now_millis)
    {
        return false;
    }
    let domain = cookie.domain.trim_start_matches('.').to_ascii_lowercase();
    if host != domain && !host.ends_with(&format!(".{domain}")) {
        return false;
    }
    path.starts_with(if cookie.path.is_empty() {
        "/"
    } else {
        cookie.path.as_str()
    })
}

fn merge_response_cookies(
    store: &mut NativeAuthStore,
    cookies: Vec<NativeAuthCookie>,
    now_unix_seconds: i64,
) -> bool {
    let mut changed = false;
    for cookie in cookies {
        changed |= upsert_cookie(store, cookie, now_unix_seconds);
    }
    changed
}

fn upsert_cookie(
    store: &mut NativeAuthStore,
    cookie: NativeAuthCookie,
    now_unix_seconds: i64,
) -> bool {
    let before = store.cookies.len();
    store.cookies.retain(|existing| {
        existing.name != cookie.name
            || existing.domain != cookie.domain
            || existing.path != cookie.path
    });
    let removed = store.cookies.len() != before;
    if cookie_matches(
        &cookie,
        &cookie.domain,
        if cookie.path.is_empty() {
            "/"
        } else {
            &cookie.path
        },
        now_unix_seconds,
    ) {
        store.cookies.push(cookie);
        true
    } else {
        removed
    }
}

fn parse_set_cookie(
    header: &str,
    url: &str,
    now_unix_seconds: i64,
) -> Result<Option<NativeAuthCookie>, String> {
    let parsed =
        reqwest::Url::parse(url).map_err(|error| format!("Set-Cookie URL is invalid: {error}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "Set-Cookie URL has no host".to_string())?
        .to_ascii_lowercase();
    let mut parts = header.split(';').map(str::trim);
    let Some(first) = parts.next() else {
        return Ok(None);
    };
    let Some(equals) = first.find('=') else {
        return Ok(None);
    };
    if equals == 0 {
        return Ok(None);
    }

    let mut cookie = NativeAuthCookie {
        name: first[..equals].to_string(),
        value: first[equals + 1..].to_string(),
        domain: host,
        path: default_cookie_path(parsed.path()),
        secure: false,
        http_only: false,
        same_site: String::new(),
        expires_at: None,
    };

    for part in parts {
        let (key, value) = part
            .split_once('=')
            .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim()))
            .unwrap_or_else(|| (part.trim().to_ascii_lowercase(), ""));
        match key.as_str() {
            "domain" if !value.is_empty() => {
                cookie.domain = value.trim_start_matches('.').to_ascii_lowercase();
            }
            "path" if !value.is_empty() => {
                cookie.path = value.to_string();
            }
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            "samesite" => cookie.same_site = value.to_string(),
            "expires" if !value.is_empty() => {
                cookie.expires_at = httpdate::parse_http_date(value)
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as i64);
            }
            "max-age" if !value.is_empty() => {
                if let Ok(seconds) = value.parse::<i64>() {
                    cookie.expires_at = Some(
                        now_unix_seconds
                            .saturating_mul(1_000)
                            .saturating_add(seconds.saturating_mul(1_000)),
                    );
                }
            }
            _ => {}
        }
    }

    Ok(Some(cookie))
}

fn default_cookie_path(path: &str) -> String {
    if !path.starts_with('/') || path == "/" {
        return "/".to_string();
    }
    path.rsplit_once('/')
        .map(|(prefix, _)| {
            if prefix.is_empty() {
                "/".to_string()
            } else {
                format!("{prefix}/")
            }
        })
        .unwrap_or_else(|| "/".to_string())
}

fn normalized_expires_at_millis(value: Option<i64>) -> Option<i64> {
    value.map(|expires_at| {
        if expires_at > 100_000_000_000 {
            expires_at
        } else {
            expires_at.saturating_mul(1_000)
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use serde_json::json;

    use super::*;

    #[test]
    fn native_auth_store_parses_clerk_cookies_and_pending_login() {
        let store = native_auth_store_from_str(
            r#"{
                "version": 1,
                "cookies": [
                    {
                        "name": "__client",
                        "value": "client-cookie",
                        "domain": "clerk.babel.audio",
                        "path": "/",
                        "expiresAt": 1814289611
                    }
                ],
                "pendingSignIn": {
                    "id": "si_native",
                    "phoneNumberId": "idn_phone"
                }
            }"#,
        )
        .expect("store");

        assert_eq!(store.pending_sign_in_id.as_deref(), Some("si_native"));
        assert_eq!(
            store.cookies,
            vec![NativeAuthCookie {
                name: "__client".to_string(),
                value: "client-cookie".to_string(),
                domain: "clerk.babel.audio".to_string(),
                path: "/".to_string(),
                secure: false,
                http_only: false,
                same_site: String::new(),
                expires_at: Some(1_814_289_611_000),
            }]
        );
    }

    #[test]
    fn cookie_header_for_url_filters_by_domain_path_and_expiry() {
        let store = NativeAuthStore {
            cookies: vec![
                NativeAuthCookie {
                    name: "__client".to_string(),
                    value: "client-cookie".to_string(),
                    domain: "clerk.babel.audio".to_string(),
                    path: "/".to_string(),
                    secure: false,
                    http_only: false,
                    same_site: String::new(),
                    expires_at: None,
                },
                NativeAuthCookie {
                    name: "__session".to_string(),
                    value: "session-cookie".to_string(),
                    domain: "dashboard.babel.audio".to_string(),
                    path: "/".to_string(),
                    secure: false,
                    http_only: false,
                    same_site: String::new(),
                    expires_at: None,
                },
                NativeAuthCookie {
                    name: "expired".to_string(),
                    value: "old".to_string(),
                    domain: "clerk.babel.audio".to_string(),
                    path: "/".to_string(),
                    secure: false,
                    http_only: false,
                    same_site: String::new(),
                    expires_at: Some(10_000),
                },
            ],
            pending_sign_in_id: None,
            pending_phone_number_id: None,
        };

        assert_eq!(
            cookie_header_for_url(&store, "https://clerk.babel.audio/v1/client", 20)
                .expect("cookie header"),
            "__client=client-cookie"
        );
        assert_eq!(
            cookie_header_for_url(
                &store,
                "https://dashboard.babel.audio/api/trpc/worker.get",
                20
            )
            .expect("cookie header"),
            "__session=session-cookie"
        );
    }

    #[test]
    fn babel_backend_cookie_header_matches_babel_required_session_family() {
        assert_eq!(
            babel_backend_cookie_header("jwt-value", 1782754800).expect("header"),
            "__session=jwt-value; __session_v1aVht_F=jwt-value; __client_uat=1782754800; __client_uat_v1aVht_F=1782754800"
        );
    }

    #[test]
    fn active_session_id_reads_clerk_client_shapes() {
        assert_eq!(
            active_session_id(&json!({
                "response": {
                    "object": "client",
                    "sessions": [{ "id": "sess_native", "status": "active" }]
                }
            }))
            .as_deref(),
            Some("sess_native")
        );
        assert_eq!(
            active_session_id(&json!({
                "response": {
                    "object": "sign_in_attempt",
                    "created_session_id": "sess_created"
                }
            }))
            .as_deref(),
            Some("sess_created")
        );
    }

    #[tokio::test]
    async fn start_phone_code_login_stores_pending_sign_in_and_refreshed_clerk_cookies() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("babel-auth.json");
        let server = MockClerkServer::with_responses(vec![
            MockClerkResponse {
                body: r#"{"response":{"object":"sign_in_attempt","id":"si_native","supported_first_factors":[{"strategy":"phone_code","phone_number_id":"idn_phone"}]}}"#,
                headers: vec![
                    "set-cookie: __client=start-client; Max-Age=3600; Path=/; SameSite=None; Secure",
                ],
            },
            MockClerkResponse {
                body: r#"{"response":{"object":"sign_in_attempt","id":"si_native","status":"needs_first_factor"}}"#,
                headers: vec![
                    "set-cookie: __client=prepared-client; Max-Age=3600; Path=/; SameSite=None; Secure",
                ],
            },
        ]);

        let result = start_phone_code_login(
            &store_path,
            "tester@example.com",
            "https://dashboard.babel.audio",
            &server.base_url,
            true,
            1_782_754_800,
        )
        .await
        .expect("start native phone-code login");

        assert_eq!(result.sign_in_id, "si_native");
        assert_eq!(result.phone_number_id, "idn_phone");
        let store = read_native_auth_store(&store_path).expect("read auth store");
        assert_eq!(store.pending_sign_in_id.as_deref(), Some("si_native"));
        assert_eq!(store.pending_phone_number_id.as_deref(), Some("idn_phone"));
        assert_eq!(
            cookie_header_for_url(
                &store,
                &format!("{}/v1/client", server.base_url),
                1_782_754_801
            )
            .expect("cookie header"),
            "__client=prepared-client"
        );
        let requests = server.join();
        assert!(requests[0].contains("post /v1/client/sign_ins?"));
        assert!(requests[0].contains("identifier=tester%40example.com"));
        assert!(requests[1].contains("post /v1/client/sign_ins/si_native/prepare_first_factor?"));
        assert!(requests[1].contains("strategy=phone_code"));
        assert!(requests[1].contains("phone_number_id=idn_phone"));
    }

    #[tokio::test]
    async fn verify_phone_code_login_completes_session_clears_pending_and_returns_babel_cookie() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("babel-auth.json");
        std::fs::write(
            &store_path,
            r#"{
                "version": 1,
                "cookies": [
                    {
                        "name": "__client",
                        "value": "prepared-client",
                        "domain": "127.0.0.1",
                        "path": "/",
                        "expiresAt": 4102444800000
                    }
                ],
                "pendingSignIn": {
                    "id": "si_native",
                    "phoneNumberId": "idn_phone"
                }
            }"#,
        )
        .expect("write auth store");
        let server = MockClerkServer::with_responses(vec![
            MockClerkResponse {
                body: r#"{"response":{"object":"sign_in_attempt","id":"si_native","status":"complete","created_session_id":"sess_native"}}"#,
                headers: vec![
                    "set-cookie: __client=verified-client; Max-Age=3600; Path=/; SameSite=None; Secure",
                ],
            },
            MockClerkResponse {
                body: r#"{"object":"token","jwt":"jwt-from-clerk"}"#,
                headers: vec![],
            },
        ]);

        let result = verify_phone_code_login(
            &store_path,
            "123456",
            "https://dashboard.babel.audio",
            &server.base_url,
            1_782_754_800,
        )
        .await
        .expect("verify native phone-code login");

        assert_eq!(result.session_id, "sess_native");
        assert_eq!(
            result.cookie,
            "__session=jwt-from-clerk; __session_v1aVht_F=jwt-from-clerk; __client_uat=1782754800; __client_uat_v1aVht_F=1782754800"
        );
        let store = read_native_auth_store(&store_path).expect("read auth store");
        assert_eq!(store.pending_sign_in_id, None);
        assert_eq!(store.pending_phone_number_id, None);
        assert_eq!(
            cookie_header_for_url(
                &store,
                &format!("{}/v1/client", server.base_url),
                1_782_754_801
            )
            .expect("cookie header"),
            "__client=verified-client"
        );
        let requests = server.join();
        assert!(requests[0].contains("post /v1/client/sign_ins/si_native/attempt_first_factor?"));
        assert!(requests[0].contains("strategy=phone_code"));
        assert!(requests[0].contains("code=123456"));
        assert!(requests[1].contains("post /v1/client/sessions/sess_native/tokens?"));
        assert!(requests[1].contains("cookie: __client=verified-client"));
    }

    #[tokio::test]
    async fn mint_native_babel_cookie_reads_store_and_mints_fresh_session_cookie() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("babel-auth.json");
        std::fs::write(
            &store_path,
            r#"{
                "version": 1,
                "cookies": [
                    {
                        "name": "__client",
                        "value": "client-cookie",
                        "domain": "127.0.0.1",
                        "path": "/",
                        "expiresAt": 4102444800000
                    }
                ]
            }"#,
        )
        .expect("write auth store");
        let server = MockClerkServer::start();

        let cookie = mint_native_babel_cookie_from_store(
            &store_path,
            "https://dashboard.babel.audio",
            &server.base_url,
            1_782_754_800,
        )
        .await
        .expect("native cookie");

        assert_eq!(
            cookie,
            "__session=jwt-from-clerk; __session_v1aVht_F=jwt-from-clerk; __client_uat=1782754800; __client_uat_v1aVht_F=1782754800"
        );
        let requests = server.join();
        assert!(requests[0].contains("get /v1/client?"));
        assert!(requests[0].contains("cookie: __client=client-cookie"));
        assert!(requests[1].contains("post /v1/client/sessions/sess_native/tokens?"));
        assert!(requests[1].contains("cookie: __client=refreshed-client"));
        assert!(requests[1].contains("organization_id="));
        let saved = std::fs::read_to_string(&store_path).expect("read refreshed auth store");
        assert!(saved.contains("\"value\": \"refreshed-client\""));
        assert!(!saved.contains("\"value\": \"client-cookie\""));
    }

    struct MockClerkServer {
        base_url: String,
        handle: thread::JoinHandle<Vec<String>>,
    }

    struct MockClerkResponse {
        body: &'static str,
        headers: Vec<&'static str>,
    }

    impl MockClerkServer {
        fn start() -> Self {
            Self::with_responses(vec![
                MockClerkResponse {
                    body: r#"{"response":{"object":"client","last_active_session_id":"sess_native","sessions":[{"id":"sess_native","status":"active"}]}}"#,
                    headers: vec![
                        "set-cookie: __client=refreshed-client; Max-Age=3600; Path=/; SameSite=None; Secure",
                    ],
                },
                MockClerkResponse {
                    body: r#"{"object":"token","jwt":"jwt-from-clerk"}"#,
                    headers: vec![],
                },
            ])
        }

        fn with_responses(responses: Vec<MockClerkResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clerk");
            let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
            let handle = thread::spawn(move || {
                let mut requests = Vec::new();
                for response in responses {
                    let (mut stream, _) = listener.accept().expect("accept request");
                    let request = read_http_request(&mut stream);
                    requests.push(request.clone());
                    write_http_response(&mut stream, response.body, &response.headers);
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
        String::from_utf8(buffer)
            .expect("utf8 request")
            .to_ascii_lowercase()
    }

    fn write_http_response(stream: &mut std::net::TcpStream, body: &str, headers: &[&str]) {
        let extra_headers = if headers.is_empty() {
            String::new()
        } else {
            format!("{}\r\n", headers.join("\r\n"))
        };
        write!(
            stream,
            "HTTP/1.1 200 OK\r\n{}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            extra_headers,
            body.len(),
            body
        )
        .expect("write response");
    }
}
