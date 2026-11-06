//! Persisted native auth store (`auth.json` inside an account directory): Clerk cookies +
//! pending sign-in. Callers always pass the path; no process-wide default exists.

use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::clerk;

/// File name of the cookie store inside an account directory.
pub const AUTH_FILE_NAME: &str = "auth.json";

/// `<account dir>/auth.json`.
pub fn auth_store_path(account_dir: &Path) -> PathBuf {
    account_dir.join(AUTH_FILE_NAME)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCookie {
    pub name: String,
    pub value: String,
    /// Lowercase, without a leading dot.
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: String,
    /// Expiry in Unix milliseconds.
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthStore {
    pub cookies: Vec<AuthCookie>,
    pub pending_sign_in_id: Option<String>,
    pub pending_phone_number_id: Option<String>,
}

pub fn store_from_str(source: &str) -> Result<AuthStore> {
    let value: Value =
        serde_json::from_str(source).context("failed to decode native Babel auth store")?;
    let cookies = value
        .get("cookies")
        .and_then(Value::as_array)
        .map(|cookies| cookies.iter().filter_map(cookie_from_value).collect())
        .unwrap_or_default();
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
    Ok(AuthStore {
        cookies,
        pending_sign_in_id,
        pending_phone_number_id,
    })
}

pub fn read_store(path: &Path) -> Result<AuthStore> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read native Babel auth store {}", path.display()))?;
    store_from_str(&source)
}

pub(super) fn read_store_or_default(path: &Path) -> Result<AuthStore> {
    match fs::read_to_string(path) {
        Ok(source) => store_from_str(&source),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AuthStore::default()),
        Err(error) => Err(anyhow!(
            "failed to read native Babel auth store {}: {error}",
            path.display()
        )),
    }
}

pub fn save_store(path: &Path, store: &AuthStore) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create native Babel auth store directory {}",
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
    if let Some(id) = trimmed(store.pending_sign_in_id.as_deref()) {
        let mut pending = serde_json::Map::new();
        pending.insert("id".to_string(), Value::from(id));
        if let Some(phone_number_id) = trimmed(store.pending_phone_number_id.as_deref()) {
            pending.insert("phoneNumberId".to_string(), Value::from(phone_number_id));
        }
        root.insert("pendingSignIn".to_string(), Value::Object(pending));
    }

    let payload = serde_json::to_string_pretty(&Value::Object(root))
        .context("failed to encode native Babel auth store")?;
    fs::write(path, payload)
        .with_context(|| format!("failed to write native Babel auth store {}", path.display()))
}

/// Clears the store at `path` (sign out / "back to login"); also drops the cached JWT.
pub fn reset_store(path: &Path) -> Result<()> {
    clerk::clear_jwt_cache();
    save_store(path, &AuthStore::default())
}

/// Deletes the store file; a missing file is not an error. Also drops the cached JWT.
pub fn delete_store(path: &Path) -> Result<()> {
    clerk::clear_jwt_cache();
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to delete native Babel auth store {}",
                path.display()
            )
        }),
    }
}

/// `name=value; ...` of the cookies matching `url`'s host/path and not expired at `now`.
pub fn cookie_header_for_url(
    store: &AuthStore,
    url: &str,
    now_unix_seconds: i64,
) -> Result<String> {
    let parsed = url::Url::parse(url).context("native auth cookie URL is invalid")?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("native auth cookie URL has no host"))?
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

pub(super) fn merge_response_cookies(
    store: &mut AuthStore,
    cookies: Vec<AuthCookie>,
    now_unix_seconds: i64,
) -> bool {
    let mut changed = false;
    for cookie in cookies {
        changed |= upsert_cookie(store, cookie, now_unix_seconds);
    }
    changed
}

fn upsert_cookie(store: &mut AuthStore, cookie: AuthCookie, now_unix_seconds: i64) -> bool {
    let before = store.cookies.len();
    store.cookies.retain(|existing| {
        existing.name != cookie.name
            || existing.domain != cookie.domain
            || existing.path != cookie.path
    });
    let removed = store.cookies.len() != before;
    let path = if cookie.path.is_empty() {
        "/"
    } else {
        cookie.path.as_str()
    };
    if cookie_matches(&cookie, &cookie.domain, path, now_unix_seconds) {
        store.cookies.push(cookie);
        true
    } else {
        removed
    }
}

fn cookie_matches(cookie: &AuthCookie, host: &str, path: &str, now_unix_seconds: i64) -> bool {
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

pub(super) fn parse_set_cookie(
    header: &str,
    url: &str,
    now_unix_seconds: i64,
) -> Option<AuthCookie> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let mut parts = header.split(';').map(str::trim);
    let first = parts.next()?;
    let equals = first.find('=')?;
    if equals == 0 {
        return None;
    }

    let mut cookie = AuthCookie {
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
            "path" if !value.is_empty() => cookie.path = value.to_string(),
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
    Some(cookie)
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

fn cookie_from_value(value: &Value) -> Option<AuthCookie> {
    Some(AuthCookie {
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
        expires_at: value
            .get("expiresAt")
            .or_else(|| value.get("expires_at"))
            .and_then(Value::as_i64)
            .map(normalized_expires_at_millis),
    })
}

fn cookie_to_value(cookie: &AuthCookie) -> Value {
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

/// Stores may carry seconds or milliseconds; values below 1e11 are seconds.
fn normalized_expires_at_millis(expires_at: i64) -> i64 {
    if expires_at > 100_000_000_000 {
        expires_at
    } else {
        expires_at.saturating_mul(1_000)
    }
}

pub(super) fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
