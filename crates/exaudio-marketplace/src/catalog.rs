use std::collections::BTreeSet;

use exaudio_extension::{ExtensionManifest, Runtime};
use semver::Version;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Result, invalid};

pub const CATALOG_SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub owner: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub source_commit: String,
    pub manifest: ExtensionManifest,
    pub asset: ReleaseAsset,
    /// A withdrawn release remains in the index for immutable history and diagnostics.
    #[serde(default)]
    pub withdrawn: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogExtension {
    pub id: String,
    /// GitHub account owning the repository; reviewed by index maintainers, not a signature.
    pub publisher: String,
    pub repository: Repository,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub releases: Vec<Release>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: u32,
    pub extensions: Vec<CatalogExtension>,
}

/// Whether `name` ends in the package extension (`.exaudio`).
pub(crate) fn is_package_name(name: &str) -> bool {
    name.strip_suffix(crate::PACKAGE_EXTENSION)
        .is_some_and(|stem| stem.ends_with('.'))
}

pub(crate) fn safe_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub(crate) fn plain_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
}

impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != CATALOG_SCHEMA {
            return Err(invalid("unsupported catalog schema"));
        }
        let mut ids = BTreeSet::new();
        let mut repositories = BTreeSet::new();
        for entry in &self.extensions {
            if !exaudio_extension::manifest::valid_id(&entry.id) || !ids.insert(&entry.id) {
                return Err(invalid("invalid or duplicate extension identity"));
            }
            let repo = &entry.repository;
            if !safe_token(&repo.owner)
                || !safe_token(&repo.name)
                || repo.owner.contains('.')
                || repo.owner.contains('_')
                || repo.owner.starts_with('-')
                || repo.owner.ends_with('-')
                || repo.owner.len() > 39
                || repo.owner.contains("--")
                || repo.name.len() > 100
                || entry.publisher != repo.owner
                || !repositories.insert((
                    repo.owner.to_ascii_lowercase(),
                    repo.name.to_ascii_lowercase(),
                ))
            {
                return Err(invalid(
                    "invalid, shared, or publisher-mismatched GitHub repository",
                ));
            }
            let mut versions = BTreeSet::new();
            for release in &entry.releases {
                release
                    .manifest
                    .validate()
                    .map_err(|e| invalid(e.to_string()))?;
                if release.manifest.id != entry.id
                    || release.manifest.version != release.version
                    || release.manifest.runtime != Runtime::Wasm
                    || !versions.insert(&release.version)
                {
                    return Err(invalid(
                        "duplicate release, native runtime, or mismatched manifest identity",
                    ));
                }
                if !safe_token(&release.tag)
                    || !safe_token(&release.asset.name)
                    || !is_package_name(&release.asset.name)
                    || release.asset.bytes == 0
                    || !hex(&release.asset.sha256, 64)
                    || !(hex(&release.source_commit, 40) || hex(&release.source_commit, 64))
                    || release
                        .withdrawn
                        .as_ref()
                        .is_some_and(|reason| reason.trim().is_empty())
                {
                    return Err(invalid(
                        "invalid release tag, asset, hash, source commit or withdrawal",
                    ));
                }
                let expected = format!(
                    "https://github.com/{}/{}/releases/download/{}/{}",
                    repo.owner, repo.name, release.tag, release.asset.name
                );
                if release.asset.url != expected {
                    return Err(invalid(
                        "asset must be the exact HTTPS GitHub release URL in its author's repository",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Identity, provenance, version and bytes are immutable. Withdrawals may change;
    /// deletion is forbidden so cached clients cannot silently lose revocation history.
    pub fn validate_successor(&self, next: &Self) -> Result<()> {
        next.validate()?;
        for old in &self.extensions {
            let new = next
                .extensions
                .iter()
                .find(|entry| entry.id == old.id)
                .ok_or_else(|| invalid("index removed an extension; use withdrawals"))?;
            if old.repository != new.repository || old.publisher != new.publisher {
                return Err(invalid("index changed repository ownership"));
            }
            for release in &old.releases {
                let successor = new
                    .releases
                    .iter()
                    .find(|item| item.version == release.version)
                    .ok_or_else(|| invalid("index removed a release; use withdrawal"))?;
                let mut comparable = successor.clone();
                comparable.withdrawn = release.withdrawn.clone();
                if &comparable != release {
                    return Err(invalid("index mutated an immutable release"));
                }
            }
        }
        Ok(())
    }

    pub fn release(&self, id: &str, version: &Version) -> Option<&Release> {
        self.extensions
            .iter()
            .find(|entry| entry.id == id)?
            .releases
            .iter()
            .find(|release| &release.version == version)
    }

    pub fn search(&self, query: &str) -> Vec<&CatalogExtension> {
        let query = query.to_lowercase();
        self.extensions
            .iter()
            .filter(|entry| {
                entry.id.contains(&query)
                    || entry.publisher.to_lowercase().contains(&query)
                    || entry
                        .keywords
                        .iter()
                        .any(|word| word.to_lowercase().contains(&query))
                    || entry
                        .releases
                        .iter()
                        .filter(|r| r.withdrawn.is_none())
                        .any(|r| {
                            r.manifest.name.to_lowercase().contains(&query)
                                || r.manifest.description.to_lowercase().contains(&query)
                        })
            })
            .collect()
    }
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
