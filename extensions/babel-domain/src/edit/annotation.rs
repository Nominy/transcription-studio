//! "Edit annotation" dialog: hydrates intensity/categories from segment metadata, validates the
//! category rules and builds the metadata update.

use serde_json::Value;
use studio_core::{Project, Segment};

use super::create_region::{
    OTHER_CATEGORY, annotation_content, annotation_display_text, intensity_value_from_label,
    project_uses_hierarchical_annotation_labels, secondary_category_options_for,
};
use crate::i18n::localized;

/// Why the annotation edit cannot be saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAnnotationError {
    IntensityRequired,
    CategoryRequired,
}

impl EditAnnotationError {
    /// Message shown in the dialog.
    pub fn message(self) -> &'static str {
        match self {
            Self::IntensityRequired => localized("Choose an intensity.", "Выберите интенсивность."),
            Self::CategoryRequired => {
                localized("Choose both categories.", "Выберите обе категории.")
            }
        }
    }
}

impl std::fmt::Display for EditAnnotationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for EditAnnotationError {}

/// The open "edit annotation" dialog for one segment.
#[derive(Debug, Clone, PartialEq)]
pub struct EditAnnotationDialog {
    pub segment_id: String,
    pub initial_text: String,
    pub original_content: Value,
    pub uses_hierarchical_labels: bool,
    pub intensity: Option<u8>,
    pub primary_selection: String,
    pub secondary_selection: String,
    pub custom_description: String,
    pub error: Option<EditAnnotationError>,
}

/// Validated annotation update.
#[derive(Debug, Clone, PartialEq)]
pub struct EditAnnotationPayload {
    pub segment_id: String,
    pub text: String,
    pub intensity: Option<u8>,
    pub content: Value,
}

impl EditAnnotationDialog {
    pub fn for_project_segment(project: &Project, segment: &Segment) -> Self {
        let original_content = segment
            .metadata
            .get("content")
            .cloned()
            .unwrap_or(Value::Null);
        let primary_selection = label(&original_content, "primaryLabel").unwrap_or_default();
        let (secondary_selection, custom_description) = hydrate_secondary_selection(
            &primary_selection,
            label(&original_content, "secondaryLabel"),
        );
        Self {
            segment_id: segment.id.clone(),
            initial_text: segment.text.clone(),
            original_content,
            uses_hierarchical_labels: project_uses_hierarchical_annotation_labels(project),
            intensity: segment
                .metadata
                .get("intensity")
                .and_then(Value::as_u64)
                .and_then(|value| u8::try_from(value).ok()),
            primary_selection,
            secondary_selection,
            custom_description,
            error: None,
        }
    }

    /// `true` when the segment had no text yet ("Add" vs "Edit" annotation).
    pub fn is_new(&self) -> bool {
        self.initial_text.trim().is_empty()
    }

    /// Choosing a primary category keeps the original secondary only if it is still valid.
    pub fn set_primary_selection(&mut self, value: String) {
        if !self.uses_hierarchical_labels {
            return;
        }
        self.primary_selection = value;
        self.error = None;
        if self.primary_selection == OTHER_CATEGORY {
            self.secondary_selection.clear();
            return;
        }
        if !self.primary_selection.is_empty() {
            let still_valid = label(&self.original_content, "secondaryLabel").is_some_and(|s| {
                secondary_category_options_for(&self.primary_selection).contains(&s.as_str())
            });
            if !still_valid {
                self.secondary_selection.clear();
            }
        }
    }

    pub fn set_secondary_selection(&mut self, value: String) {
        if !self.uses_hierarchical_labels {
            return;
        }
        if value == OTHER_CATEGORY {
            self.custom_description.clear();
        }
        self.secondary_selection = value;
        self.error = None;
    }

    pub fn set_custom_description(&mut self, value: String) {
        if !self.uses_hierarchical_labels {
            return;
        }
        self.custom_description = value;
        self.error = None;
    }

    pub fn set_intensity_label(&mut self, value: &str) {
        self.intensity = intensity_value_from_label(value);
        self.error = None;
    }

    pub fn is_save_enabled(&self) -> bool {
        self.validate().is_ok()
    }

    /// Builds the update; flat-label projects keep the original text/content.
    pub fn save_payload(&self) -> Result<EditAnnotationPayload, EditAnnotationError> {
        self.validate()?;
        if !self.uses_hierarchical_labels {
            return Ok(EditAnnotationPayload {
                segment_id: self.segment_id.clone(),
                text: self.initial_text.clone(),
                intensity: self.intensity,
                content: self.original_content.clone(),
            });
        }
        let content = annotation_content(
            &self.primary_selection,
            &self.secondary_selection,
            &self.custom_description,
        );
        Ok(EditAnnotationPayload {
            segment_id: self.segment_id.clone(),
            text: annotation_display_text(&content),
            intensity: self.intensity,
            content,
        })
    }

    fn validate(&self) -> Result<(), EditAnnotationError> {
        if self.intensity.is_none() {
            return Err(EditAnnotationError::IntensityRequired);
        }
        if !self.uses_hierarchical_labels {
            return Ok(());
        }
        let needs_description =
            self.primary_selection == OTHER_CATEGORY || self.secondary_selection == OTHER_CATEGORY;
        if self.primary_selection.is_empty()
            || (self.primary_selection != OTHER_CATEGORY && self.secondary_selection.is_empty())
            || (needs_description && self.custom_description.trim().is_empty())
        {
            return Err(EditAnnotationError::CategoryRequired);
        }
        Ok(())
    }
}

fn label(content: &Value, key: &str) -> Option<String> {
    content.get(key).and_then(Value::as_str).map(str::to_string)
}

fn hydrate_secondary_selection(primary: &str, secondary: Option<String>) -> (String, String) {
    let Some(secondary) = secondary else {
        return (String::new(), String::new());
    };
    if primary == OTHER_CATEGORY {
        return (secondary.clone(), secondary);
    }
    if secondary.is_empty() || secondary_category_options_for(primary).contains(&secondary.as_str())
    {
        return (secondary, String::new());
    }
    (OTHER_CATEGORY.to_string(), secondary)
}
