//! Clerk phone-code login and session-JWT minting.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, SET_COOKIE};
use serde_json::Value;

use super::store::{
    AuthCookie, AuthStore, cookie_header_for_url, merge_response_cookies, parse_set_cookie,
    read_store, read_store_or_default, save_store, trimmed,
};
use super::{babel_base_url, clerk_url, now_unix_seconds};
use studio_services::http;

const CLERK_INSTANCE_SUFFIX: &str = "v1aVht_F";
const CLERK_API_VERSION: &str = "2025-11-10";
const CLERK_JS_VERSION: &str = "5.127.0";

/// Stop reusing a cached Clerk session JWT this long before its `exp` so an
/// in-flight request never carries a token that expires mid-flight. Clerk
/// session tokens live ~60s, so the margin is small.
const JWT_REUSE_MARGIN_SECONDS: i64 = 10;
/// Reuse window for a JWT without a parseable `exp` claim.
const JWT_FALLBACK_TTL_SECONDS: i64 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneCodeLoginStart {
    pub sign_in_id: String,
    pub phone_number_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneCodeLoginComplete {
    pub session_id: String,
    /// Babel `Cookie` header for the new session.
    pub cookie: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedJwt {
    store_path: PathBuf,
    clerk_url: String,
    token: String,
    reuse_until_unix_seconds: i64,
}

/// Process-wide cache of the last minted session JWT (minting costs two
/// Clerk round-trips and every Babel request needs one).
#[derive(Debug, Default)]
struct JwtCache {
    entry: Option<CachedJwt>,
}

impl JwtCache {
    fn token_for(&self, store_path: &Path, clerk_url: &str, now: i64) -> Option<String> {
        let entry = self.entry.as_ref()?;
        (entry.store_path == store_path
            && entry.clerk_url == clerk_url
            && now < entry.reuse_until_unix_seconds)
            .then(|| entry.token.clone())
    }

    fn store_token(&mut self, store_path: &Path, clerk_url: &str, token: String, now: i64) {
        let reuse_until_unix_seconds = jwt_exp_unix_seconds(&token)
            .map(|exp| exp - JWT_REUSE_MARGIN_SECONDS)
            .unwrap_or(now + JWT_FALLBACK_TTL_SECONDS);
        self.entry = Some(CachedJwt {
            store_path: store_path.to_path_buf(),
            clerk_url: clerk_url.to_string(),
            token,
            reuse_until_unix_seconds,
        });
    }
}

static JWT_CACHE: Mutex<JwtCache> = Mutex::new(JwtCache { entry: None });

fn jwt_cache() -> MutexGuard<'static, JwtCache> {
    JWT_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Dropped on every auth transition so a previous session's token is never reused.
pub fn clear_jwt_cache() {
    jwt_cache().entry = None;
}

/// `exp` claim of a JWT, read without signature verification.
fn jwt_exp_unix_seconds(jwt: &str) -> Option<i64> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value.get("exp").and_then(Value::as_i64)
}

/// Starts a phone-code login into `store_path` with the default URLs (resetting any
/// previous store, as the login screen does).
pub async fn start_login(store_path: &Path, identifier: &str) -> Result<PhoneCodeLoginStart> {
    start_phone_code_login(
        store_path,
        identifier,
        &babel_base_url(),
        &clerk_url(),
        true,
        now_unix_seconds(),
    )
    .await
}

/// Verifies the SMS code for the pending login in `store_path`.
pub async fn verify_login(store_path: &Path, code: &str) -> Result<PhoneCodeLoginComplete> {
    verify_phone_code_login(
        store_path,
        code,
        &babel_base_url(),
        &clerk_url(),
        now_unix_seconds(),
    )
    .await
}

/// Creates a Clerk sign-in for `identifier`, prepares the `phone_code` factor
/// and records the pending sign-in in the store. `reset` starts from an empty store.
pub async fn start_phone_code_login(
    store_path: &Path,
    identifier: &str,
    babel_url: &str,
    clerk_url: &str,
    reset: bool,
    now_unix_seconds: i64,
) -> Result<PhoneCodeLoginStart> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        bail!("phone-code login requires an identifier");
    }
    clear_jwt_cache();
    let mut store = if reset {
        AuthStore::default()
    } else {
        read_store_or_default(store_path)?
    };
    let (babel_url, clerk_url) = validated_urls(babel_url, clerk_url)?;

    let client = http::client();
    let sign_in_url = clerk_api_url(clerk_url, "/v1/client/sign_ins");
    let sign_in = request_clerk_json(
        &client,
        &mut store,
        ClerkRequest::post(&sign_in_url, &[("identifier", identifier)]),
        babel_url,
        now_unix_seconds,
    )
    .await?;
    let sign_in = sign_in.get("response").unwrap_or(&sign_in);
    let sign_in_id = string_field(sign_in, "id")
        .ok_or_else(|| anyhow!("Clerk sign-in response did not include id"))?;
    let phone_number_id = phone_code_factor_id(sign_in)?;

    let prepare_url = clerk_api_url(
        clerk_url,
        &format!(
            "/v1/client/sign_ins/{}/prepare_first_factor",
            encode_path_segment(&sign_in_id)
        ),
    );
    request_clerk_json(
        &client,
        &mut store,
        ClerkRequest::post(
            &prepare_url,
            &[
                ("strategy", "phone_code"),
                ("phone_number_id", &phone_number_id),
            ],
        ),
        babel_url,
        now_unix_seconds,
    )
    .await?;
    store.pending_sign_in_id = Some(sign_in_id.clone());
    store.pending_phone_number_id = Some(phone_number_id.clone());
    save_store(store_path, &store)?;

    Ok(PhoneCodeLoginStart {
        sign_in_id,
        phone_number_id,
    })
}

/// Attempts the `phone_code` factor for the pending sign-in, mints the first
/// session token and clears the pending sign-in.
pub async fn verify_phone_code_login(
    store_path: &Path,
    code: &str,
    babel_url: &str,
    clerk_url: &str,
    now_unix_seconds: i64,
) -> Result<PhoneCodeLoginComplete> {
    let code = code.trim();
    if code.is_empty() {
        bail!("phone verification code is empty");
    }
    clear_jwt_cache();
    let mut store = read_store(store_path)?;
    let Some(sign_in_id) = trimmed(store.pending_sign_in_id.as_deref()) else {
        bail!(
            "native Babel auth store has no pending phone-code login; run `studio-app --babel-auth-start-login <phone>`"
        );
    };
    let (babel_url, clerk_url) = validated_urls(babel_url, clerk_url)?;

    let client = http::client();
    let attempt_url = clerk_api_url(
        clerk_url,
        &format!(
            "/v1/client/sign_ins/{}/attempt_first_factor",
            encode_path_segment(&sign_in_id)
        ),
    );
    let attempt = request_clerk_json(
        &client,
        &mut store,
        ClerkRequest::post(&attempt_url, &[("strategy", "phone_code"), ("code", code)]),
        babel_url,
        now_unix_seconds,
    )
    .await?;
    let Some(session_id) = active_session_id(&attempt) else {
        bail!("phone-code sign-in did not create an active session");
    };

    let jwt = mint_session_token(
        &client,
        &mut store,
        clerk_url,
        &encode_path_segment(&session_id),
        babel_url,
        now_unix_seconds,
    )
    .await?;
    store.pending_sign_in_id = None;
    store.pending_phone_number_id = None;
    save_store(store_path, &store)?;
    let jwt = jwt.ok_or_else(|| anyhow!("Clerk token response did not include jwt"))?;
    let cookie = babel_backend_cookie_header(&jwt, now_unix_seconds)?;
    Ok(PhoneCodeLoginComplete { session_id, cookie })
}

/// Minting failed because the stored login is missing, unfinished or no longer active:
/// only signing in again fixes it (network and server errors are not this).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NeedsSignIn(pub String);

/// `Bearer <jwt>` for Babel API calls, minted from the stored Clerk client
/// cookies (cached until shortly before the JWT expires).
pub async fn mint_babel_authorization(
    store_path: &Path,
    babel_url: &str,
    clerk_url: &str,
    now_unix_seconds: i64,
) -> Result<String> {
    let jwt = mint_babel_jwt(store_path, babel_url, clerk_url, now_unix_seconds).await?;
    Ok(format!("Bearer {jwt}"))
}

async fn mint_babel_jwt(
    store_path: &Path,
    babel_url: &str,
    clerk_url: &str,
    now_unix_seconds: i64,
) -> Result<String> {
    // No store file yet means the user never signed in: same as an empty store.
    let mut store = read_store_or_default(store_path)?;
    let (babel_url, clerk_url) = validated_urls(babel_url, clerk_url)?;

    if let Some(cached) = jwt_cache().token_for(store_path, clerk_url, now_unix_seconds) {
        return Ok(cached);
    }

    let client = http::client();
    let client_url = clerk_api_url(clerk_url, "/v1/client");
    if cookie_header_for_url(&store, &client_url, now_unix_seconds)?
        .trim()
        .is_empty()
    {
        return Err(NeedsSignIn(format!(
            "native Babel auth store {} has no Clerk cookies; run `studio-app --babel-auth-start-login <phone>`",
            store_path.display()
        ))
        .into());
    }

    let cookies_before = store.cookies.clone();
    let clerk_client = request_clerk_json(
        &client,
        &mut store,
        ClerkRequest::get(&client_url),
        babel_url,
        now_unix_seconds,
    )
    .await?;
    if store.cookies != cookies_before {
        save_store(store_path, &store)?;
    }
    let Some(session_id) = active_session_id(&clerk_client) else {
        if store.pending_sign_in_id.is_some() {
            return Err(NeedsSignIn(
                "native Babel auth store has a pending phone-code login; run `studio-app --babel-auth-verify-login <code>`"
                    .to_string(),
            )
            .into());
        }
        return Err(NeedsSignIn(
            "native Babel auth store has no active Clerk session; run `studio-app --babel-auth-start-login <phone>`"
                .to_string(),
        )
        .into());
    };

    let jwt = mint_session_token(
        &client,
        &mut store,
        clerk_url,
        &session_id,
        babel_url,
        now_unix_seconds,
    )
    .await?;
    store.pending_sign_in_id = None;
    store.pending_phone_number_id = None;
    save_store(store_path, &store)?;
    let jwt = jwt.ok_or_else(|| anyhow!("Clerk token response did not include jwt"))?;
    jwt_cache().store_token(store_path, clerk_url, jwt.clone(), now_unix_seconds);
    Ok(jwt)
}

/// POST `/v1/client/sessions/<id>/tokens`; returns the JWT if present (cookies are merged into `store`).
async fn mint_session_token(
    client: &reqwest::Client,
    store: &mut AuthStore,
    clerk_url: &str,
    session_path_segment: &str,
    babel_url: &str,
    now_unix_seconds: i64,
) -> Result<Option<String>> {
    let token_url = clerk_api_url(
        clerk_url,
        &format!("/v1/client/sessions/{session_path_segment}/tokens"),
    );
    let response = request_clerk_json(
        client,
        store,
        ClerkRequest::post(&token_url, &[("organization_id", "")]),
        babel_url,
        now_unix_seconds,
    )
    .await?;
    Ok(response
        .get("jwt")
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// Babel's Clerk session cookie family for `jwt`.
pub fn babel_backend_cookie_header(jwt: &str, now_unix_seconds: i64) -> Result<String> {
    if jwt.trim().is_empty() {
        bail!("cannot build Babel cookie header without a Clerk session jwt");
    }
    Ok(format!(
        "__session={jwt}; __session_{CLERK_INSTANCE_SUFFIX}={jwt}; __client_uat={now_unix_seconds}; __client_uat_{CLERK_INSTANCE_SUFFIX}={now_unix_seconds}"
    ))
}

/// Session id from a Clerk client / sign-in attempt response.
pub fn active_session_id(value: &Value) -> Option<String> {
    let source = value.get("response").unwrap_or(value);
    source
        .get("last_active_session_id")
        .and_then(Value::as_str)
        .or_else(|| source.get("created_session_id").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let sessions = source.get("sessions")?.as_array()?;
            sessions
                .iter()
                .find(|session| session.get("status").and_then(Value::as_str) == Some("active"))
                .or_else(|| sessions.first())?
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn validated_urls<'a>(babel_url: &'a str, clerk_url: &'a str) -> Result<(&'a str, &'a str)> {
    let clerk_url = clerk_url.trim().trim_end_matches('/');
    let babel_url = babel_url.trim().trim_end_matches('/');
    if clerk_url.is_empty() {
        bail!("native Babel auth Clerk URL is empty");
    }
    if babel_url.is_empty() {
        bail!("native Babel auth Babel URL is empty");
    }
    Ok((babel_url, clerk_url))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
}

fn phone_code_factor_id(sign_in: &Value) -> Result<String> {
    let factors = sign_in
        .get("supported_first_factors")
        .or_else(|| sign_in.get("supportedFirstFactors"))
        .or_else(|| sign_in.get("first_factors"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Clerk sign-in response did not include first factors"))?;

    factors
        .iter()
        .find(|factor| factor.get("strategy").and_then(Value::as_str) == Some("phone_code"))
        .and_then(|factor| {
            factor
                .get("phone_number_id")
                .or_else(|| factor.get("phoneNumberId"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            let strategies = factors
                .iter()
                .filter_map(|factor| factor.get("strategy").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(",");
            anyhow!(
                "phone_code is not an available first factor; available={}",
                if strategies.is_empty() {
                    "none"
                } else {
                    strategies.as_str()
                }
            )
        })
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn clerk_api_url(base_url: &str, path: &str) -> String {
    format!(
        "{base_url}{path}?__clerk_api_version={CLERK_API_VERSION}&_clerk_js_version={CLERK_JS_VERSION}"
    )
}

struct ClerkRequest<'a> {
    url: &'a str,
    form: Option<String>,
}

impl<'a> ClerkRequest<'a> {
    fn get(url: &'a str) -> Self {
        Self { url, form: None }
    }

    fn post(url: &'a str, pairs: &[(&str, &str)]) -> Self {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs.iter().copied())
            .finish();
        Self {
            url,
            form: Some(form),
        }
    }
}

/// Sends a Clerk frontend-API request with the store's cookies for that URL,
/// merges `Set-Cookie` responses into `store`, and decodes the JSON body.
async fn request_clerk_json(
    client: &reqwest::Client,
    store: &mut AuthStore,
    request: ClerkRequest<'_>,
    babel_url: &str,
    now_unix_seconds: i64,
) -> Result<Value> {
    let cookie_header = cookie_header_for_url(store, request.url, now_unix_seconds)?;
    let builder = match request.form {
        Some(form) => client
            .post(request.url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form),
        None => client.get(request.url),
    }
    .header(ACCEPT, "*/*")
    .header("origin", babel_url)
    .header("referer", format!("{babel_url}/"))
    .header(COOKIE, cookie_header);

    let response = builder
        .send()
        .await
        .context("native Babel auth Clerk request failed")?;
    let status = response.status();
    let set_cookies: Vec<AuthCookie> = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .filter_map(|header| parse_set_cookie(header, request.url, now_unix_seconds))
        .collect();
    let text = response
        .text()
        .await
        .context("failed to read native Babel auth Clerk response")?;
    let value: Value =
        serde_json::from_str(&text).context("failed to decode native Babel auth Clerk response")?;
    if !status.is_success() {
        bail!(
            "native Babel auth Clerk HTTP {status}: {}",
            http::body_snippet(&text)
        );
    }
    merge_response_cookies(store, set_cookies, now_unix_seconds);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample_jwt_with_exp(exp: i64) -> String {
        let payload = URL_SAFE_NO_PAD.encode(json!({ "exp": exp }).to_string());
        format!("header.{payload}.signature")
    }

    #[test]
    fn jwt_exp_unix_seconds_reads_exp_claim() {
        assert_eq!(
            jwt_exp_unix_seconds(&sample_jwt_with_exp(1_900_000_000)),
            Some(1_900_000_000)
        );
        assert_eq!(jwt_exp_unix_seconds("not-a-jwt"), None);
        assert_eq!(jwt_exp_unix_seconds("header.!!!.signature"), None);
        assert_eq!(
            jwt_exp_unix_seconds(&format!(
                "header.{}.signature",
                URL_SAFE_NO_PAD.encode(json!({ "sub": "user" }).to_string())
            )),
            None
        );
    }

    #[test]
    fn jwt_cache_reuses_token_until_margin_before_expiry() {
        let mut cache = JwtCache::default();
        let store = Path::new("store.json");
        cache.store_token(
            store,
            "https://clerk.example",
            sample_jwt_with_exp(1_000),
            900,
        );

        assert_eq!(
            cache.token_for(store, "https://clerk.example", 989),
            Some(sample_jwt_with_exp(1_000))
        );
        assert!(
            cache
                .token_for(store, "https://clerk.example", 990)
                .is_none()
        );
        assert!(
            cache
                .token_for(store, "https://clerk.example", 1_500)
                .is_none()
        );
    }

    #[test]
    fn jwt_cache_falls_back_to_conservative_ttl_without_exp() {
        let mut cache = JwtCache::default();
        let store = Path::new("store.json");
        cache.store_token(store, "https://clerk.example", "opaque".to_string(), 1_000);

        assert_eq!(
            cache.token_for(store, "https://clerk.example", 1_049),
            Some("opaque".to_string())
        );
        assert!(
            cache
                .token_for(store, "https://clerk.example", 1_050)
                .is_none()
        );
    }

    #[test]
    fn jwt_cache_is_scoped_to_store_path_and_clerk_url() {
        let mut cache = JwtCache::default();
        cache.store_token(
            Path::new("a.json"),
            "https://clerk-a.example",
            sample_jwt_with_exp(1_000),
            900,
        );
        assert!(
            cache
                .token_for(Path::new("b.json"), "https://clerk-a.example", 900)
                .is_none()
        );
        assert!(
            cache
                .token_for(Path::new("a.json"), "https://clerk-b.example", 900)
                .is_none()
        );
    }
}
