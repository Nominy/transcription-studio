use serde_json::Value;
use studio_core::{Project, Segment};

use super::browser_input::OutsideInteractionContext;
use super::create_region::{
    annotation_content, annotation_display_text, intensity_value_from_label,
    project_uses_hierarchical_annotation_labels, secondary_category_options_for,
};

const INTENSITY_REQUIRED_ERROR: &str = "Intensity is required";
const CATEGORY_REQUIRED_ERROR: &str = "Primary and secondary categories are required";
const EDIT_ANNOTATION_TRACE_ENV: &str = "STUDIO_TRACE_EDIT_ANNOTATION";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct EditAnnotationDialog {
    pub(super) segment_id: String,
    pub(super) initial_text: String,
    pub(super) original_content: Value,
    pub(super) uses_hierarchical_labels: bool,
    pub(super) intensity: Option<u8>,
    pub(super) primary_selection: String,
    pub(super) secondary_selection: String,
    pub(super) custom_description: String,
    pub(super) select_open: bool,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct EditAnnotationPayload {
    pub(super) segment_id: String,
    pub(super) text: String,
    pub(super) intensity: Option<u8>,
    pub(super) content: Value,
}

impl EditAnnotationDialog {
    pub(super) fn for_project_segment(project: &Project, segment: &Segment) -> Self {
        let original_content = segment
            .metadata
            .get("content")
            .cloned()
            .unwrap_or(Value::Null);
        let primary_selection = extract_primary_label(&original_content).unwrap_or_default();
        let (secondary_selection, custom_description) = hydrate_secondary_selection(
            &primary_selection,
            extract_secondary_label(&original_content),
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
            select_open: false,
            error: None,
        }
    }

    pub(super) fn title(&self) -> &'static str {
        if self.initial_text.trim().is_empty() {
            "Add Annotation"
        } else {
            "Edit Annotation"
        }
    }

    pub(super) fn set_primary_selection(&mut self, value: String) {
        if !self.uses_hierarchical_labels {
            return;
        }
        self.primary_selection = value;
        self.error = None;

        if self.primary_selection == "other" {
            self.secondary_selection.clear();
            return;
        }

        if !self.primary_selection.is_empty() {
            let original_secondary = extract_secondary_label(&self.original_content);
            let original_still_valid = original_secondary.as_deref().is_some_and(|secondary| {
                secondary_category_options_for(&self.primary_selection).contains(&secondary)
            });
            if !original_still_valid {
                self.secondary_selection.clear();
            }
        }
    }

    pub(super) fn set_secondary_selection(&mut self, value: String) {
        if !self.uses_hierarchical_labels {
            return;
        }
        if value == "other" {
            self.secondary_selection = "other".to_string();
            self.custom_description.clear();
        } else {
            self.secondary_selection = value;
        }
        self.error = None;
    }

    pub(super) fn set_custom_description(&mut self, value: String) {
        if !self.uses_hierarchical_labels {
            return;
        }
        self.custom_description = value;
        self.error = None;
    }

    pub(super) fn set_intensity_from_label(&mut self, value: &str) {
        self.intensity = intensity_value_from_label(value);
        self.error = None;
    }

    #[cfg(test)]
    pub(super) fn is_save_enabled(&self) -> bool {
        self.validate().is_ok()
    }

    pub(super) fn save_payload(&self) -> Result<EditAnnotationPayload, String> {
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

    fn validate(&self) -> Result<(), String> {
        if self.intensity.is_none() {
            return Err(INTENSITY_REQUIRED_ERROR.to_string());
        }
        if !self.uses_hierarchical_labels {
            return Ok(());
        }
        if self.primary_selection.is_empty()
            || (self.primary_selection != "other" && self.secondary_selection.is_empty())
            || (self.primary_selection == "other" && self.custom_description.trim().is_empty())
            || (self.secondary_selection == "other" && self.custom_description.trim().is_empty())
        {
            return Err(CATEGORY_REQUIRED_ERROR.to_string());
        }
        Ok(())
    }
}

pub(super) fn edit_annotation_runtime_trace_line(
    action: &str,
    dialog: &EditAnnotationDialog,
) -> String {
    format!(
        "Transcription Studio edit annotation: action={action} segment={} title={} hierarchical={} primary={} secondary={} intensity={} save_ready={} text_len={}",
        dialog.segment_id,
        dialog.title(),
        dialog.uses_hierarchical_labels,
        dialog.primary_selection,
        dialog.secondary_selection,
        dialog
            .intensity
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        dialog.validate().is_ok(),
        dialog.initial_text.chars().count()
    )
}

pub(super) fn edit_annotation_trace_enabled() -> bool {
    std::env::var(EDIT_ANNOTATION_TRACE_ENV).is_ok_and(|value| value != "0")
}

fn extract_primary_label(content: &Value) -> Option<String> {
    content
        .get("primaryLabel")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_secondary_label(content: &Value) -> Option<String> {
    content
        .get("secondaryLabel")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn hydrate_secondary_selection(
    primary_selection: &str,
    secondary_label: Option<String>,
) -> (String, String) {
    let Some(secondary_label) = secondary_label else {
        return (String::new(), String::new());
    };

    if primary_selection == "other" {
        return (secondary_label.clone(), secondary_label);
    }

    if secondary_label.is_empty()
        || secondary_category_options_for(primary_selection).contains(&secondary_label.as_str())
    {
        return (secondary_label, String::new());
    }

    ("other".to_string(), secondary_label)
}

pub(super) fn should_close_edit_annotation_on_outside_mouse_down(
    select_open: bool,
    context: OutsideInteractionContext,
) -> bool {
    !select_open && !context.inside_surface && !context.has_recovered_select_or_portal_target()
}
