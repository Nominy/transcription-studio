//! User-supplied drafting request settings.

use anyhow::{Result, bail};
use serde_json::{Value, json};
pub const DRAFT_OPENROUTER_API_KEY_ENV: &str = "STUDIO_DRAFT_OPENROUTER_API_KEY";
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";
/// Drafting inputs from the Gold Drafting tab. Empty `openrouter_api_key` falls back to
/// `STUDIO_DRAFT_OPENROUTER_API_KEY`, then `OPENROUTER_API_KEY`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftingConfig {
    pub openrouter_api_key: String,
    pub model: String,
    /// Omitted from payloads when empty or `default`.
    pub service_tier: String,
    /// Omitted from payloads when empty or `default`.
    pub reasoning_effort: String,
    pub draft_session_id: String,
}

impl DraftingConfig {
    /// Trims every field.
    pub fn from_inputs(
        openrouter_api_key: &str,
        model: &str,
        service_tier: &str,
        reasoning_effort: &str,
        draft_session_id: &str,
    ) -> Self {
        Self {
            openrouter_api_key: openrouter_api_key.trim().to_string(),
            model: model.trim().to_string(),
            service_tier: service_tier.trim().to_string(),
            reasoning_effort: reasoning_effort.trim().to_string(),
            draft_session_id: draft_session_id.trim().to_string(),
        }
    }

    pub fn resolved_api_key(&self) -> Result<String> {
        let key = if self.openrouter_api_key.is_empty() {
            std::env::var(DRAFT_OPENROUTER_API_KEY_ENV)
                .or_else(|_| std::env::var(OPENROUTER_API_KEY_ENV))
                .unwrap_or_default()
                .trim()
                .to_string()
        } else {
            self.openrouter_api_key.clone()
        };
        if key.is_empty() {
            bail!(
                "Drafting needs an OpenRouter API key. (set {DRAFT_OPENROUTER_API_KEY_ENV} or {OPENROUTER_API_KEY_ENV})"
            );
        }
        Ok(key)
    }

    pub fn add_optional_fields(&self, payload: &mut Value) {
        let Some(object) = payload.as_object_mut() else {
            return;
        };
        if !self.model.is_empty() {
            object.insert("model".to_string(), json!(self.model));
        }
        if !self.service_tier.is_empty() && self.service_tier != "default" {
            object.insert("serviceTier".to_string(), json!(self.service_tier));
        }
        if !self.reasoning_effort.is_empty() && self.reasoning_effort != "default" {
            object.insert("reasoningEffort".to_string(), json!(self.reasoning_effort));
        }
        if !self.draft_session_id.is_empty() {
            object.insert("draftSessionId".to_string(), json!(self.draft_session_id));
        }
    }
}
