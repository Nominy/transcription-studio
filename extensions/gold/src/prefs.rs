//! Gold settings preserve the existing shared storage key and serialized fields.

use serde::{Deserialize, Serialize};
use std::io;
use studio_services::prefs::UiStorage;

pub const AI_SETTINGS_KEY: &str = "studio.ai.settings.v1";

/// Typed access to Gold settings without coupling the generic store to AI.
pub trait AiPreferences {
    fn ai_settings(&self) -> AiSettings;
    fn set_ai_settings(&mut self, settings: &AiSettings) -> io::Result<()>;
}

impl AiPreferences for UiStorage {
    fn ai_settings(&self) -> AiSettings {
        AiSettings::from_storage(self.value(AI_SETTINGS_KEY))
    }

    fn set_ai_settings(&mut self, settings: &AiSettings) -> io::Result<()> {
        self.set_value(AI_SETTINGS_KEY, settings.to_storage())
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
