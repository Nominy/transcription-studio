//! One directory per configured Babel account (a shell provider instance). The directory
//! holds `auth.json` (direct Clerk cookies) **or** `subaccount.json` (relay session), plus
//! `account.json` (the instance record, so a moved install can rediscover its accounts) and
//! caches. Nothing here knows about the shell: callers hand in the directory.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::{self, AuthStore, auth_store_path, cookie_header_for_url};
use crate::babel::{BabelClient, BabelConfig};
use crate::subaccounts::{self, SubaccountSession, session_store_path};

/// File name of the instance record inside an account directory.
pub const RECORD_FILE_NAME: &str = "account.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountKind {
    /// Phone/e-mail (Clerk) login; cookies in `auth.json`.
    Direct,
    /// Sub-account code login; session in `subaccount.json`.
    Relay,
}

/// What the provider registry needs to re-create an instance, plus what the sign-in screen
/// needs to prefill a re-authentication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRecord {
    pub id: String,
    pub label: String,
    pub kind: AccountKind,
    /// Phone number / e-mail for direct accounts; relay host URL for sub-accounts.
    pub identifier: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    /// Usable credentials are present; `detail` describes them (label or host).
    Ready {
        detail: String,
    },
    NeedsSignIn,
}

/// Credentials found in an account directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountCredentials {
    Direct(AuthStore),
    Relay(SubaccountSession),
}

/// An account directory. Cheap to clone; nothing is read until asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountStore {
    dir: PathBuf,
}

impl AccountStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn auth_path(&self) -> PathBuf {
        auth_store_path(&self.dir)
    }

    pub fn session_path(&self) -> PathBuf {
        session_store_path(&self.dir)
    }

    pub fn record_path(&self) -> PathBuf {
        self.dir.join(RECORD_FILE_NAME)
    }

    /// `Ok(None)` when no record was written yet.
    pub fn read_record(&self) -> Result<Option<AccountRecord>> {
        let path = self.record_path();
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read account record {}", path.display()));
            }
        };
        serde_json::from_str(&source)
            .map(Some)
            .with_context(|| format!("failed to decode account record {}", path.display()))
    }

    pub fn save_record(&self, record: &AccountRecord) -> Result<()> {
        fs::create_dir_all(&self.dir).with_context(|| {
            format!("failed to create account directory {}", self.dir.display())
        })?;
        let path = self.record_path();
        let payload =
            serde_json::to_string_pretty(record).context("failed to encode account record")?;
        fs::write(&path, payload)
            .with_context(|| format!("failed to write account record {}", path.display()))
    }

    /// A relay session wins over cookies (a directory never legitimately holds both).
    pub fn credentials(&self) -> Result<Option<AccountCredentials>> {
        if let Some(session) = subaccounts::read_session(&self.session_path())? {
            return Ok(Some(AccountCredentials::Relay(session)));
        }
        let path = self.auth_path();
        if !path.is_file() {
            return Ok(None);
        }
        auth::read_store(&path).map(|store| Some(AccountCredentials::Direct(store)))
    }

    /// Ready when a relay session exists or an unexpired Clerk client cookie exists.
    /// Unreadable credentials count as signed out rather than as an error: the user can
    /// always fix them by signing in again.
    pub fn status(&self, clerk_url: &str, now_unix_seconds: i64) -> AccountStatus {
        match self.credentials() {
            Ok(Some(AccountCredentials::Relay(session))) => AccountStatus::Ready {
                detail: session.host_url,
            },
            Ok(Some(AccountCredentials::Direct(store))) => {
                let client_url = format!("{}/v1/client", clerk_url.trim_end_matches('/'));
                let has_cookie = cookie_header_for_url(&store, &client_url, now_unix_seconds)
                    .is_ok_and(|header| !header.trim().is_empty());
                if has_cookie && store.pending_sign_in_id.is_none() {
                    AccountStatus::Ready {
                        detail: "Signed in".to_string(),
                    }
                } else {
                    AccountStatus::NeedsSignIn
                }
            }
            Ok(None) | Err(_) => AccountStatus::NeedsSignIn,
        }
    }

    /// [`Self::status`] against the configured Clerk URL at the current time.
    pub fn current_status(&self) -> AccountStatus {
        self.status(
            &auth::clerk_url(),
            time::OffsetDateTime::now_utc().unix_timestamp(),
        )
    }

    /// The client that talks to Babel as this account, or `None` without credentials.
    pub fn client(&self) -> Result<Option<BabelClient>> {
        Ok(match self.credentials()? {
            Some(AccountCredentials::Relay(session)) => {
                Some(BabelClient::relay_in(session, &self.dir))
            }
            Some(AccountCredentials::Direct(_)) => {
                Some(BabelClient::direct(BabelConfig::for_account(&self.dir)))
            }
            None => None,
        })
    }

    /// Deletes the credential files (keeps the record and caches); also drops the cached JWT.
    pub fn clear_credentials(&self) -> Result<()> {
        auth::delete_store(&self.auth_path())?;
        subaccounts::delete_session(&self.session_path())
    }
}

/// Instance id for a phone number / e-mail: a readable slug plus a hash suffix so two
/// identifiers that slug alike never share a directory.
pub fn direct_instance_id(identifier: &str) -> String {
    let normalized = identifier.trim().to_lowercase();
    instance_id(&slug(&normalized, 40), &normalized)
}

/// Instance id for a relay login: host + sub-account id.
pub fn relay_instance_id(host_url: &str, subaccount_id: &str) -> String {
    let host = host_url
        .trim()
        .to_lowercase()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    let key = format!("{host}|{}", subaccount_id.trim());
    let readable = format!("{}-{}", slug(&host, 24), slug(subaccount_id, 15));
    instance_id(readable.trim_matches('-'), &key)
}

/// Label for a relay login: display name (or "Sub-account") @ host.
pub fn relay_label(session: &SubaccountSession) -> String {
    let name = session.display_name.trim();
    let name = if name.is_empty() { "Sub-account" } else { name };
    let host = session
        .host_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    format!("{name} @ {host}")
}

fn instance_id(readable: &str, key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let suffix: String = digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if readable.is_empty() {
        suffix
    } else {
        format!("{readable}-{suffix}")
    }
}

/// `[a-z0-9]` runs joined by `-`, at most `max` characters.
fn slug(value: &str, max: usize) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(character.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
        if out.len() >= max {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

/// Records of every account directory under `provider_dir` (one subdirectory per instance),
/// sorted by id. Directories without a readable record are skipped.
pub fn discover_records(provider_dir: &Path) -> Vec<AccountRecord> {
    let Ok(entries) = fs::read_dir(provider_dir) else {
        return Vec::new();
    };
    let mut records: Vec<AccountRecord> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let store = AccountStore::new(entry.path());
            let record = store.read_record().ok().flatten()?;
            let dir_name = entry.file_name();
            (dir_name.to_string_lossy() == record.id).then_some(record)
        })
        .collect();
    records.sort_by(|a, b| a.id.cmp(&b.id));
    records
}

/// Moves the whole providers root to `target` (portable install): copies recursively, then
/// removes the source. A missing source leaves an empty `target`. Refuses when `target` is
/// inside `source` or already holds files.
pub fn move_providers_root(source: &Path, target: &Path) -> Result<()> {
    if source == target {
        fs::create_dir_all(target)
            .with_context(|| format!("failed to create {}", target.display()))?;
        return Ok(());
    }
    if target.starts_with(source) {
        bail!(
            "{} is inside {}; choose a location outside the current providers folder",
            target.display(),
            source.display()
        );
    }
    if target.is_dir()
        && fs::read_dir(target)
            .with_context(|| format!("failed to read {}", target.display()))?
            .next()
            .is_some()
    {
        bail!("{} already contains files", target.display());
    }
    // The JWT cache is keyed by store path; paths change now.
    auth::clear_jwt_cache();
    if !source.exists() {
        fs::create_dir_all(target)
            .with_context(|| format!("failed to create {}", target.display()))?;
        return Ok(());
    }
    copy_dir(source, target)?;
    fs::remove_dir_all(source)
        .with_context(|| format!("failed to remove {} after moving it", source.display()))
}

fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target).with_context(|| format!("failed to create {}", target.display()))?;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to).with_context(|| {
                format!("failed to copy {} to {}", from.display(), to.display())
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_ids_are_path_safe_and_stable() {
        let a = direct_instance_id("+7 (999) 123-45-67");
        let b = direct_instance_id(" +7 (999) 123-45-67 ");
        assert_eq!(a, b);
        assert!(a.starts_with("7-999-123-45-67-"));
        assert!(
            a.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
        assert_ne!(
            direct_instance_id("User@Example.com"),
            direct_instance_id("user@example.org")
        );
        assert_eq!(
            direct_instance_id("User@Example.com"),
            direct_instance_id("user@example.com")
        );
    }

    #[test]
    fn relay_ids_depend_on_host_and_subaccount() {
        let a = relay_instance_id("https://relay.example.com/", "sub-1");
        let b = relay_instance_id("relay.example.com", "sub-1");
        assert_eq!(a, b);
        assert_ne!(a, relay_instance_id("https://relay.example.com", "sub-2"));
        assert!(a.starts_with("relay-example-com-sub-1-"));
    }

    #[test]
    fn empty_identifier_still_yields_an_id() {
        assert_eq!(direct_instance_id("***").len(), 8);
    }
}
