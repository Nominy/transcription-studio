//! Persistent UI settings: one flat string key/value JSON file
//! (`<app support>/ui-local-storage.json`) holding the preferences record,
//! hotkey bindings and context-panel collapse flags.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Error, ErrorKind},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::paths::app_support_dir;

/// Key of the serialized [`Preferences`] record.
pub const PREFERENCES_KEY: &str = "studio.preferences.v1";
/// Key of the serialized [`AiSettings`] record.
pub const AI_SETTINGS_KEY: &str = "studio.ai.settings.v1";
/// Key of the serialized hotkey bindings (format owned by the UI's keybind model).
pub const HOTKEY_BINDINGS_KEY: &str = "studio.native.hotkeyBindings.v1";
/// Collapse flag of the Script Context panel (Babel's localStorage key).
pub const SCRIPT_CONTEXT_HIDDEN_KEY: &str = "tx:hide-script-context";
/// Collapse flag of the Original Script panel (Babel's localStorage key).
pub const ORIGINAL_SCRIPT_HIDDEN_KEY: &str = "tx:hide-original-script";

const STORAGE_FILE_NAME: &str = "ui-local-storage.json";

/// String key/value store persisted as pretty JSON; every change is written through.
#[derive(Debug, Clone)]
pub struct UiStorage {
    path: PathBuf,
    values: BTreeMap<String, String>,
}

impl UiStorage {
    /// Loads the default file; a missing or unreadable file yields an empty store at that path.
    pub fn load_default() -> Self {
        let path = app_support_dir().join(STORAGE_FILE_NAME);
        Self::load(&path).unwrap_or_else(|_| Self {
            path,
            values: BTreeMap::new(),
        })
    }

    /// Loads `path`; a missing file is an empty store, malformed JSON is `InvalidData`.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let values = match fs::read_to_string(&path) {
            Ok(source) => serde_json::from_str(&source).map_err(|source| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("failed to decode UI storage: {source}"),
                )
            })?,
            Err(error) if error.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { path, values })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Stores `value` and writes the file if it changed.
    pub fn set_value(&mut self, key: &str, value: String) -> io::Result<()> {
        if self.values.get(key) == Some(&value) {
            return Ok(());
        }
        self.values.insert(key.to_string(), value);
        self.save()
    }

    /// Babel semantics: collapsed only when the stored value is exactly `"true"`.
    pub fn is_collapsed(&self, key: &str) -> bool {
        self.value(key) == Some("true")
    }

    pub fn set_collapsed(&mut self, key: &str, collapsed: bool) -> io::Result<()> {
        self.set_value(key, collapsed.to_string())
    }

    /// The stored preferences, or defaults when missing/undecodable.
    pub fn preferences(&self) -> Preferences {
        Preferences::from_storage(self.value(PREFERENCES_KEY))
    }

    pub fn set_preferences(&mut self, preferences: &Preferences) -> io::Result<()> {
        self.set_value(PREFERENCES_KEY, preferences.to_storage())
    }

    /// The stored AI settings, normalized; defaults when missing/undecodable.
    pub fn ai_settings(&self) -> AiSettings {
        AiSettings::from_storage(self.value(AI_SETTINGS_KEY))
    }

    pub fn set_ai_settings(&mut self, settings: &AiSettings) -> io::Result<()> {
        self.set_value(AI_SETTINGS_KEY, settings.to_storage())
    }

    fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let source = serde_json::to_string_pretty(&self.values).map_err(|source| {
            Error::new(
                ErrorKind::InvalidData,
                format!("failed to encode UI storage: {source}"),
            )
        })?;
        fs::write(&self.path, source)
    }
}

/// User preferences (mirrors Babel Helper's settings). Every field has a
/// default and `#[serde(default)]`, so older/newer records still load.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    /// Transcript layout ("Editor → Layout").
    pub editor_layout: EditorLayout,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditorLayout {
    /// Waveform card above one transcript table with every speaker.
    #[default]
    Classic,
    /// Waveform card above per-speaker transcript panes, one above the other.
    SplitStacked,
    /// Waveform card above per-speaker transcript panes, side by side.
    SplitSideBySide,
}

impl Preferences {
    pub fn from_storage(raw: Option<&str>) -> Self {
        raw.and_then(|source| serde_json::from_str(source).ok())
            .unwrap_or_default()
    }

    pub fn to_storage(&self) -> String {
        serde_json::to_string(self).expect("preferences always serialize")
    }
}

/// Public coordinator base used by Gold's "Remote GPU" setting (`PUBLIC_L0_BASE_URL`).
pub const PUBLIC_INFERENCE_BASE_URL: &str =
    "https://reviewgen.ovh/a3f73d6cf25fa138be653daaf2d7cd0702c0b2d69c40fb9eaee4e07d4b067dd5";

/// Where model inference runs (Gold's "Where to draft").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionLocation {
    /// This PC (Gold's runtime hosted in WebView2).
    #[default]
    Local,
    /// The remote coordinator GPU.
    Remote,
}

/// AI settings (Gold Draft, Review Helper, denoise). Stored in [`UiStorage`] under
/// [`AI_SETTINGS_KEY`]; keys are secrets: never log them or put them in process arguments.
/// Defaults mirror Gold's `DEFAULT_SETTINGS`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiSettings {
    /// Where word timings and draft text are computed.
    pub execution_location: ExecutionLocation,
    /// Remote coordinator base URL (no trailing slash).
    pub remote_base_url: String,
    /// Optional remote coordinator access key.
    pub remote_access_key: String,
    /// Gold OpenRouter key: draft polish and error-task Initial fill.
    pub gold_openrouter_key: String,
    /// Gold polish model (OpenRouter id).
    pub gold_model: String,
    /// `default` | `flex` | `priority`.
    pub gold_service_tier: String,
    /// `default` | `none` | `minimal` | `low` | `medium` | `high` | `xhigh`.
    pub gold_reasoning_effort: String,
    /// Gold's `llmRefinementEnabled`: polish the draft through OpenRouter after timestamps.
    pub gold_polish_enabled: bool,
    /// Review Helper OpenRouter key (`X-OpenRouter-Key`).
    pub review_openrouter_key: String,
    /// Review Helper decision model (`X-OpenRouter-Model`); empty = backend default.
    pub review_model: String,
    /// Magic Review mode: `false` = interactive workspace, `true` = fast (generate → apply).
    pub review_fast_mode: bool,
    /// Where speech enhancement (denoise) runs.
    pub denoise_location: ExecutionLocation,
    /// Generate enhanced audio automatically when a task opens.
    pub auto_denoise: bool,
}

pub const SERVICE_TIERS: [&str; 3] = ["default", "flex", "priority"];
pub const REASONING_EFFORTS: [&str; 7] = [
    "default", "none", "minimal", "low", "medium", "high", "xhigh",
];

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            execution_location: ExecutionLocation::Local,
            remote_base_url: PUBLIC_INFERENCE_BASE_URL.to_string(),
            remote_access_key: String::new(),
            gold_openrouter_key: String::new(),
            gold_model: "google/gemini-3-flash-preview".to_string(),
            gold_service_tier: "flex".to_string(),
            gold_reasoning_effort: "low".to_string(),
            gold_polish_enabled: false,
            review_openrouter_key: String::new(),
            review_model: String::new(),
            review_fast_mode: false,
            denoise_location: ExecutionLocation::Local,
            auto_denoise: false,
        }
    }
}

impl AiSettings {
    pub fn from_storage(raw: Option<&str>) -> Self {
        raw.and_then(|source| serde_json::from_str::<Self>(source).ok())
            .unwrap_or_default()
            .normalized()
    }

    pub fn to_storage(&self) -> String {
        serde_json::to_string(self).expect("AI settings always serialize")
    }

    /// Gold's `normalizeSettings`: trims values, restores defaults for unknown tiers/efforts,
    /// an empty model or an invalid remote URL.
    pub fn normalized(mut self) -> Self {
        let defaults = Self::default();
        for value in [
            &mut self.remote_access_key,
            &mut self.gold_openrouter_key,
            &mut self.gold_model,
            &mut self.review_openrouter_key,
            &mut self.review_model,
        ] {
            let trimmed = value.trim();
            if trimmed.len() != value.len() {
                *value = trimmed.to_string();
            }
        }
        if self.gold_model.is_empty() {
            self.gold_model = defaults.gold_model;
        }
        if !SERVICE_TIERS.contains(&self.gold_service_tier.as_str()) {
            self.gold_service_tier = defaults.gold_service_tier;
        }
        if !REASONING_EFFORTS.contains(&self.gold_reasoning_effort.as_str()) {
            self.gold_reasoning_effort = defaults.gold_reasoning_effort;
        }
        self.remote_base_url = normalize_base_url(&self.remote_base_url)
            .unwrap_or_else(|| PUBLIC_INFERENCE_BASE_URL.to_string());
        self
    }
}

/// Gold's `normalizeL0CustomBaseUrl`: http(s) only, no credentials/query/fragment, no trailing
/// slash. `None` for blank or invalid input.
pub fn normalize_base_url(input: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(input.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Some(url.as_str().trim_end_matches('/').to_string())
}
