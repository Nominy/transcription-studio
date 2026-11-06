//! The project style guide Initial fill sends with every decision: the single Docs document
//! shown in the task tool whose label is "Style Guide" (or whose name contains it), read in
//! full (`project.listProjectDocuments` → `project.getProjectDocument`).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::ErrorTaskError;
use super::diff_align::{js_trim, utf16_len};

/// Longest style guide accepted (UTF-16 units).
pub const MAX_GUIDE_UNITS: usize = 64_000;

static LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^style\s+guide$").expect("valid label pattern"));
static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bstyle\s+guide\b").expect("valid name pattern"));

/// A complete, versioned project style guide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleGuide {
    pub project_id: String,
    /// Document id.
    pub id: String,
    pub version: i64,
    pub created_at: String,
    pub content: String,
}

/// Gold's identifier rule: 1–256 chars, no whitespace or control characters.
pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 256
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl StyleGuide {
    /// `isErrorTaskStyleGuide`.
    pub fn is_valid(&self) -> bool {
        valid_id(&self.project_id)
            && valid_id(&self.id)
            && (1..=9_007_199_254_740_991).contains(&self.version)
            && self.created_at.chars().count() <= 64
            && OffsetDateTime::parse(&self.created_at, &Rfc3339).is_ok()
            && !js_trim(&self.content).is_empty()
            && utf16_len(&self.content) <= MAX_GUIDE_UNITS
    }
}

fn invalid() -> ErrorTaskError {
    ErrorTaskError::new(
        "style-guide-invalid",
        "Couldn’t read the style guide. Try again.",
    )
}

/// Picks the style guide's document id from a `listProjectDocuments` result.
pub fn select_style_guide_document(documents: &Value) -> Result<String, ErrorTaskError> {
    if documents.is_null() {
        return Err(ErrorTaskError::new(
            "style-guide-unavailable",
            "This project has no style guide in Docs.",
        ));
    }
    let documents = documents.as_array().ok_or_else(invalid)?;
    let guides: Vec<&Value> = documents
        .iter()
        .filter(|document| {
            document.get("showInTaskTool").and_then(Value::as_bool) == Some(true)
                && document.get("kind").and_then(Value::as_str) == Some("markdown")
                && (document
                    .get("label")
                    .and_then(Value::as_str)
                    .is_some_and(|label| LABEL.is_match(js_trim(label)))
                    || document
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| NAME.is_match(name)))
        })
        .collect();
    match guides.as_slice() {
        [guide] => guide
            .get("documentId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                ErrorTaskError::new(
                    "style-guide-unavailable",
                    "This project has no style guide in Docs.",
                )
            }),
        [] => Err(ErrorTaskError::new(
            "style-guide-unavailable",
            "This project has no style guide in Docs.",
        )),
        _ => Err(ErrorTaskError::new(
            "style-guide-unavailable",
            "This project has more than one style guide in Docs.",
        )),
    }
}

/// Reads a `getProjectDocument` result into a validated guide.
pub fn parse_style_guide_document(
    project_id: &str,
    document_id: &str,
    document: &Value,
) -> Result<StyleGuide, ErrorTaskError> {
    if document.is_null() {
        return Err(ErrorTaskError::new(
            "style-guide-unavailable",
            "This project has no style guide in Docs.",
        ));
    }
    let latest = document.get("latest").filter(|value| value.is_object());
    let body = document.get("body").filter(|value| value.is_object());
    let (Some(latest), Some(body)) = (latest, body) else {
        return Err(invalid());
    };
    if document.get("documentId").and_then(Value::as_str) != Some(document_id)
        || document.get("kind").and_then(Value::as_str) != Some("markdown")
        || body.get("kind").and_then(Value::as_str) != Some("markdown")
    {
        return Err(invalid());
    }
    let incomplete = || {
        ErrorTaskError::new(
            "style-guide-invalid",
            "The style guide is incomplete or too long.",
        )
    };
    let version = latest
        .get("version")
        .and_then(|version| {
            version.as_i64().or_else(|| {
                version
                    .as_f64()
                    .filter(|v| v.fract() == 0.0)
                    .map(|v| v as i64)
            })
        })
        .ok_or_else(incomplete)?;
    let guide = StyleGuide {
        project_id: project_id.to_string(),
        id: document_id.to_string(),
        version,
        created_at: latest
            .get("createdAt")
            .and_then(Value::as_str)
            .ok_or_else(incomplete)?
            .to_string(),
        content: body
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(incomplete)?
            .to_string(),
    };
    if !guide.is_valid() {
        return Err(incomplete());
    }
    Ok(guide)
}
