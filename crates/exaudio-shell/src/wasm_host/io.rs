use super::*;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const VALUE_LIMIT: usize = 1024 * 1024;
const OWNER_QUOTA: u64 = 16 * 1024 * 1024;
const SECRETS_FILE: &str = "secrets.json";
/// Storage lives under `<root>/storage/<sha256(key)>`; `root` is the extension's data
/// directory or an instance namespace (`providers/<p>/<id>/wasm/<owner>`).
fn key_path(root: &Path, key: &str) -> Result<PathBuf, String> {
    if key.is_empty() || key.len() > 256 {
        return Err("storage key must contain 1..256 bytes".into());
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(key.as_bytes());
    Ok(root.join("storage").join(format!("{digest:x}")))
}
fn private_directory(path: &std::path::Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    if fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("extension data directory cannot be a symlink".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub(super) fn get(
    root: &Path,
    cx: &CallContext<'_>,
    key: &str,
    max: usize,
) -> Result<Option<Vec<u8>>, String> {
    let path = key_path(root, key)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > max.min(VALUE_LIMIT) as u64
    {
        return Err("storage value exceeds limit or is not a regular file".into());
    }
    cx.check()?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(max.min(VALUE_LIMIT) as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    cx.check()?;
    if bytes.len() > max.min(VALUE_LIMIT) {
        return Err("storage value exceeds limit".into());
    }
    Ok(Some(bytes))
}
pub(super) fn set(
    root: &Path,
    cx: &CallContext<'_>,
    key: &str,
    value: &[u8],
) -> Result<(), String> {
    if value.len() > VALUE_LIMIT {
        return Err("storage value exceeds limit".into());
    }
    let path = key_path(root, key)?;
    let parent = path.parent().ok_or("invalid storage path")?;
    private_directory(parent)?;
    let mut total = 0u64;
    for item in fs::read_dir(parent).map_err(|e| e.to_string())? {
        cx.check()?;
        let item = item.map_err(|e| e.to_string())?;
        if item.path() != path {
            let metadata = item.metadata().map_err(|e| e.to_string())?;
            total = total.saturating_add(metadata.len());
        }
    }
    if total.saturating_add(value.len() as u64) > OWNER_QUOTA {
        return Err("extension storage quota exceeded".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(value).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    cx.check()?;
    file.persist(&path).map_err(|e| e.error.to_string())?;
    Ok(())
}
pub(super) fn secret(
    host: &Bridge,
    cx: &CallContext<'_>,
    key: &str,
    max: usize,
) -> Result<Option<String>, String> {
    if key.is_empty() || key.len() > 256 {
        return Err("invalid secret key".into());
    }
    cx.check()?;
    let entry = keyring::Entry::new(&format!("Exaudio.Extension.{}", host.owner), key)
        .map_err(|e| e.to_string())?;
    let value = match entry.get_password() {
        Ok(value) => Some(value),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => return Err(e.to_string()),
    };
    cx.check()?;
    if value
        .as_ref()
        .is_some_and(|v| v.len() > max.min(VALUE_LIMIT))
    {
        return Err("secret exceeds limit".into());
    }
    Ok(value)
}
fn read_secrets(path: &Path, cx: &CallContext<'_>) -> Result<BTreeMap<String, String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > VALUE_LIMIT as u64
    {
        return Err("secret store exceeds limit or is not a regular file".into());
    }
    cx.check()?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(VALUE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > VALUE_LIMIT {
        return Err("secret store exceeds limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("corrupt secret store: {e}"))
}
/// Instance secrets are one private JSON object per (instance, extension), deleted with
/// the instance directory. Unlike `secret`, they are writable: provider types create them.
pub(super) fn instance_secret_get(
    root: &Path,
    cx: &CallContext<'_>,
    key: &str,
    max: usize,
) -> Result<Option<String>, String> {
    if key.is_empty() || key.len() > 256 {
        return Err("invalid secret key".into());
    }
    let value = read_secrets(&root.join(SECRETS_FILE), cx)?.remove(key);
    cx.check()?;
    if value
        .as_ref()
        .is_some_and(|v| v.len() > max.min(VALUE_LIMIT))
    {
        return Err("secret exceeds limit".into());
    }
    Ok(value)
}
pub(super) fn instance_secret_set(
    root: &Path,
    cx: &CallContext<'_>,
    key: &str,
    value: Option<&str>,
) -> Result<(), String> {
    if key.is_empty() || key.len() > 256 {
        return Err("invalid secret key".into());
    }
    if value.is_some_and(|v| v.len() > VALUE_LIMIT) {
        return Err("secret exceeds limit".into());
    }
    let path = root.join(SECRETS_FILE);
    let mut secrets = read_secrets(&path, cx)?;
    match value {
        Some(value) => {
            secrets.insert(key.into(), value.into());
        }
        None => {
            if secrets.remove(key).is_none() {
                return Ok(());
            }
        }
    }
    let bytes = serde_json::to_vec(&secrets).map_err(|e| e.to_string())?;
    if bytes.len() > VALUE_LIMIT {
        return Err("secret store exceeds limit".into());
    }
    private_directory(root)?;
    let mut file = tempfile::NamedTempFile::new_in(root).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    cx.check()?;
    file.persist(&path).map_err(|e| e.error.to_string())?;
    Ok(())
}
pub(super) fn http(
    host: &Bridge,
    cx: &CallContext<'_>,
    value: &w::HttpRequest,
    max: usize,
) -> Result<w::HttpResponse, String> {
    let url = reqwest::Url::parse(&value.url).map_err(|e| e.to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("HTTPS without embedded credentials is required".into());
    }
    let hostname = url.host_str().ok_or("HTTP URL has no hostname")?;
    host.permitted(cx, Permission::Network(hostname.to_owned()))?;
    let timeout = cx.deadline.saturating_duration_since(Instant::now());
    if timeout.is_zero() {
        return Err("HTTP deadline exceeded".into());
    }
    // Fresh client per exchange: no shared cookie jar, proxy discovery, redirect
    // credentials, default auth, referer, or connection cache shared with native clients.
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .timeout(timeout)
        .connect_timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?;
    let method = reqwest::Method::from_bytes(value.method.as_bytes()).map_err(|e| e.to_string())?;
    let mut request = client.request(method, url);
    for header in &value.headers {
        if [
            "host",
            "cookie",
            "proxy-authorization",
            "proxy-connection",
            "connection",
            "transfer-encoding",
            "content-length",
        ]
        .iter()
        .any(|name| header.name.eq_ignore_ascii_case(name))
        {
            return Err("ambient or hop-by-hop HTTP header is forbidden".into());
        }
        request = request.header(&header.name, &header.value);
    }
    let mut response = request
        .body(value.body.clone())
        .send()
        .map_err(|e| e.to_string())?;
    cx.check()?;
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err("HTTP response exceeds limit".into());
    }
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            Ok(w::Header {
                name: name.to_string(),
                value: value.to_str().map_err(|e| e.to_string())?.to_owned(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut body = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        cx.check()?;
        let read = response.read(&mut chunk).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        if body.len().checked_add(read).is_none_or(|size| size > max) {
            return Err("HTTP response exceeds limit".into());
        }
        body.extend_from_slice(&chunk[..read]);
    }
    cx.check()?;
    Ok(w::HttpResponse {
        status,
        headers,
        body,
    })
}
