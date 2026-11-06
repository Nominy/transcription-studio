//! Serializable identity, dependency and permission contracts shared by native and WASM
//! extensions. These contracts contain no platform, AI or checking policy.

use std::collections::BTreeMap;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Host API version supported by this build (distinct from the application version).
pub const API_VERSION: Version = Version::new(0, 1, 0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Runtime {
    /// Compiled into the application; not eligible for downloaded native code loading.
    Native,
    Wasm,
}

/// Permissions are capabilities, not unrestricted access to the host OS. The host enforces
/// each grant again when its service is called; a declaration alone is never a grant.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "scope", rename_all = "kebab-case")]
pub enum Permission {
    DocumentRead,
    DocumentEdit,
    AudioRead,
    /// Control playback/seek; reading samples alone must not authorize audible playback.
    TransportControl,
    /// Change editor focus/selection without granting permission to rewrite the document.
    SelectionControl,
    FilePicker,
    /// Access to this extension's own persistent key/value and cache namespace.
    Storage,
    /// Exact HTTPS hostname; redirects must be checked against grants too.
    Network(String),
    /// Access to this extension's own secret namespace, never another extension's secrets.
    Secrets,
}

/// How a mode starts once its requirements and provider are ready.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModeLaunch {
    /// The mode shows its own launch window (task list, guides, metadata) and opens
    /// documents from there.
    #[default]
    Window,
    /// The mode opens a document immediately; no intermediate window.
    Direct,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeManifest {
    pub id: String,
    pub name: String,
    /// Hard requirements. Resolving a mode also resolves their transitive dependencies.
    #[serde(default)]
    pub requires: BTreeMap<String, VersionReq>,
    /// Recommendations never become hard requirements or silently acquire permissions.
    #[serde(default)]
    pub recommends: BTreeMap<String, VersionReq>,
    /// Provider type this mode signs in through (declared by this or a required extension).
    /// Opening the mode runs the provider's setup when no instance is configured yet.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub launch: ModeLaunch,
}

/// A provider type owns credentials and sessions for an external account (a task
/// platform, a review service). Modes reference it by id; users see it only as a sign-in
/// step while opening a mode, and as "Accounts" in advanced settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderManifest {
    pub id: String,
    pub name: String,
    /// Whether several configured instances (accounts) of this type may coexist.
    #[serde(default)]
    pub multiple: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub api: VersionReq,
    pub runtime: Runtime,
    pub description: String,
    pub license: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, VersionReq>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub modes: Vec<ModeManifest>,
    #[serde(default)]
    pub providers: Vec<ProviderManifest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ManifestError {
    #[error("invalid extension identifier `{0}` (expected dotted lowercase components)")]
    InvalidId(String),
    #[error("extension `{0}` has an empty name or license")]
    MissingIdentity(String),
    #[error("extension `{extension}` requires API {required}, host provides {available}")]
    IncompatibleApi {
        extension: String,
        required: VersionReq,
        available: Version,
    },
    #[error("invalid network permission hostname `{0}`")]
    InvalidNetworkHost(String),
    #[error("duplicate mode identifier `{0}`")]
    DuplicateMode(String),
    #[error("duplicate provider identifier `{0}`")]
    DuplicateProvider(String),
    #[error("mode `{mode}` references provider `{provider}` that is not declared")]
    UnknownProvider { mode: String, provider: String },
}

/// Portable, path-safe identity: never used as an unchecked filesystem path.
pub fn valid_id(id: &str) -> bool {
    id.len() <= 128
        && id.contains('.')
        && id.split('.').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_lowercase()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !part.ends_with('-')
        })
}

impl ExtensionManifest {
    /// Compatibility is checked when selecting a release, not while displaying a catalog:
    /// a well-formed marketplace may include releases targeting newer hosts.
    pub fn check_host_compatibility(&self) -> Result<(), ManifestError> {
        if self.api.matches(&API_VERSION) {
            Ok(())
        } else {
            Err(ManifestError::IncompatibleApi {
                extension: self.id.clone(),
                required: self.api.clone(),
                available: API_VERSION,
            })
        }
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if !valid_id(&self.id) {
            return Err(ManifestError::InvalidId(self.id.clone()));
        }
        if self.name.trim().is_empty() || self.license.trim().is_empty() {
            return Err(ManifestError::MissingIdentity(self.id.clone()));
        }
        for id in self.dependencies.keys().chain(
            self.modes
                .iter()
                .flat_map(|mode| mode.requires.keys().chain(mode.recommends.keys())),
        ) {
            if !valid_id(id) {
                return Err(ManifestError::InvalidId(id.clone()));
            }
        }
        let mut modes = std::collections::BTreeSet::new();
        for mode in &self.modes {
            if !valid_id(&mode.id) {
                return Err(ManifestError::InvalidId(mode.id.clone()));
            }
            if !modes.insert(&mode.id) {
                return Err(ManifestError::DuplicateMode(mode.id.clone()));
            }
        }
        let mut providers = std::collections::BTreeSet::new();
        for provider in &self.providers {
            if !valid_id(&provider.id) || provider.name.trim().is_empty() {
                return Err(ManifestError::InvalidId(provider.id.clone()));
            }
            if !providers.insert(&provider.id) {
                return Err(ManifestError::DuplicateProvider(provider.id.clone()));
            }
        }
        for mode in &self.modes {
            if let Some(provider) = &mode.provider
                && !valid_id(provider)
            {
                return Err(ManifestError::InvalidId(provider.clone()));
            }
        }
        for permission in &self.permissions {
            if let Permission::Network(host) = permission {
                // No URL syntax, wildcards, ports, credentials or empty DNS labels.
                let valid = !host.is_empty()
                    && host.len() <= 253
                    && host.split('.').all(|label| {
                        !label.is_empty()
                            && label.len() <= 63
                            && !label.starts_with('-')
                            && !label.ends_with('-')
                            && label
                                .bytes()
                                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                    });
                if !valid {
                    return Err(ManifestError::InvalidNetworkHost(host.clone()));
                }
            }
        }
        Ok(())
    }
}
