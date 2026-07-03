use std::{collections::BTreeMap, fmt};

use serde_json::{Value, json};
use studio_core::Project;

use super::browser_input::OutsideInteractionContext;
use super::formatting::{format_seconds_for_edit, parse_seconds};

const BABEL_TRANSCRIPTION_TYPE: &str = "transcription";
const BACKGROUND_NOISE_TYPE: &str = "background_noise";
pub(super) const BACKGROUND_NOISE_ANNOTATION_PROJECT_ID: &str =
    "4c0db8c4-699e-4014-9dbf-27f178c3da8e";
const BACKGROUND_NOISE_ANNOTATION_FTE_PROJECT_ID: &str = "7cb04a85-c9f3-4d73-afb9-34156777948f";
const CREATE_REGION_TRACE_ENV: &str = "STUDIO_TRACE_CREATE_REGION";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CreateRegionTimeField {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CreateRegionMode {
    Transcription,
    Annotation,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct CreateRegionDialog {
    pub(super) mode: CreateRegionMode,
    pub(super) created_segment_id: Option<String>,
    pub(super) selected_lane_id: String,
    pub(super) start_seconds: f64,
    pub(super) end_seconds: f64,
    pub(super) text: String,
    pub(super) intensity: Option<u8>,
    pub(super) uses_hierarchical_labels: bool,
    pub(super) primary_selection: String,
    pub(super) secondary_selection: String,
    pub(super) custom_description: String,
    pub(super) select_open: bool,
    pub(super) active_time_field: Option<CreateRegionTimeField>,
    pub(super) start_draft: String,
    pub(super) end_draft: String,
    pub(super) start_error: Option<String>,
    pub(super) end_error: Option<String>,
}

impl CreateRegionDialog {
    pub(super) fn new(selected_lane_id: String, start_seconds: f64, end_seconds: f64) -> Self {
        Self::with_mode(
            CreateRegionMode::Transcription,
            selected_lane_id,
            start_seconds,
            end_seconds,
            false,
        )
    }

    #[cfg(test)]
    pub(super) fn new_annotation(
        selected_lane_id: String,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Self {
        Self::with_mode(
            CreateRegionMode::Annotation,
            selected_lane_id,
            start_seconds,
            end_seconds,
            true,
        )
    }

    pub(super) fn new_for_project(
        project: &Project,
        selected_lane_id: String,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Self {
        if project_uses_annotation_mode(project) {
            Self::with_mode(
                CreateRegionMode::Annotation,
                selected_lane_id,
                start_seconds,
                end_seconds,
                project_uses_hierarchical_annotation_labels(project),
            )
        } else {
            Self::new(selected_lane_id, start_seconds, end_seconds)
        }
    }

    fn with_mode(
        mode: CreateRegionMode,
        selected_lane_id: String,
        start_seconds: f64,
        end_seconds: f64,
        uses_hierarchical_labels: bool,
    ) -> Self {
        Self {
            mode,
            created_segment_id: None,
            selected_lane_id,
            start_seconds,
            end_seconds,
            text: String::new(),
            intensity: None,
            uses_hierarchical_labels,
            primary_selection: String::new(),
            secondary_selection: String::new(),
            custom_description: String::new(),
            select_open: false,
            active_time_field: None,
            start_draft: format_seconds_for_edit(start_seconds),
            end_draft: format_seconds_for_edit(end_seconds),
            start_error: None,
            end_error: None,
        }
    }

    pub(super) fn with_created_segment_id(mut self, segment_id: String) -> Self {
        self.created_segment_id = Some(segment_id);
        self
    }

    pub(super) fn set_start_draft(&mut self, value: String) {
        self.start_draft = value;
        self.active_time_field = Some(CreateRegionTimeField::Start);
        self.start_error = None;
    }

    pub(super) fn set_end_draft(&mut self, value: String) {
        self.end_draft = value;
        self.active_time_field = Some(CreateRegionTimeField::End);
        self.end_error = None;
    }

    pub(super) fn commit_time_field(&mut self, field: CreateRegionTimeField) -> bool {
        match field {
            CreateRegionTimeField::Start => self.commit_start_draft(),
            CreateRegionTimeField::End => self.commit_end_draft(),
        }
    }

    #[cfg(test)]
    pub(super) fn blur_time_field(&mut self, field: CreateRegionTimeField) -> bool {
        let ok = self.commit_time_field(field);
        if !ok {
            self.reset_time_field_draft(field, false);
        }
        self.active_time_field = None;
        ok
    }

    pub(super) fn reset_active_time_field_draft(&mut self) -> bool {
        let Some(field) = self.active_time_field else {
            return false;
        };
        self.reset_time_field_draft(field, true);
        self.active_time_field = None;
        true
    }

    pub(super) fn is_create_enabled(&self) -> bool {
        !self.selected_lane_id.is_empty()
            && self.end_seconds > self.start_seconds
            && self.start_error.is_none()
            && self.end_error.is_none()
            && match self.mode {
                CreateRegionMode::Transcription => true,
                CreateRegionMode::Annotation => self.annotation_fields_are_complete(),
            }
    }

    pub(super) fn validate_for_submit(&mut self, project: &Project) -> Option<CreateRegionPayload> {
        let start_seconds = match parse_seconds("Start", &self.start_draft) {
            Ok(value) => value,
            Err(error) => {
                self.start_error = Some(error);
                return None;
            }
        };
        let end_seconds = match parse_seconds("End", &self.end_draft) {
            Ok(value) => value,
            Err(error) => {
                self.end_error = Some(error);
                return None;
            }
        };

        self.start_error = None;
        self.end_error = None;
        if start_seconds >= end_seconds {
            self.start_error = Some("Start must be earlier than end".to_string());
            return None;
        }

        if !project
            .lanes
            .iter()
            .any(|lane| lane.id == self.selected_lane_id)
        {
            return None;
        }

        self.start_seconds = start_seconds;
        self.end_seconds = end_seconds;
        self.start_draft = format_seconds_for_edit(start_seconds);
        self.end_draft = format_seconds_for_edit(end_seconds);
        if self.mode == CreateRegionMode::Annotation && !self.annotation_fields_are_complete() {
            return None;
        }
        let (text, metadata) = self.payload_text_and_metadata();
        Some(CreateRegionPayload {
            lane_id: self.selected_lane_id.clone(),
            start_seconds,
            end_seconds,
            text,
            metadata,
        })
    }

    fn commit_start_draft(&mut self) -> bool {
        let start_seconds = match parse_seconds("Start", &self.start_draft) {
            Ok(value) => value,
            Err(error) => {
                self.start_error = Some(error);
                return false;
            }
        };
        if start_seconds >= self.end_seconds {
            self.start_error = Some("Start must be earlier than end".to_string());
            return false;
        }

        self.start_seconds = start_seconds;
        self.start_draft = format_seconds_for_edit(start_seconds);
        self.end_draft = format_seconds_for_edit(self.end_seconds);
        self.start_error = None;
        self.end_error = None;
        true
    }

    fn commit_end_draft(&mut self) -> bool {
        let end_seconds = match parse_seconds("End", &self.end_draft) {
            Ok(value) => value,
            Err(error) => {
                self.end_error = Some(error);
                return false;
            }
        };
        if self.start_seconds >= end_seconds {
            self.end_error = Some("End must be later than start".to_string());
            return false;
        }

        self.end_seconds = end_seconds;
        self.start_draft = format_seconds_for_edit(self.start_seconds);
        self.end_draft = format_seconds_for_edit(end_seconds);
        self.start_error = None;
        self.end_error = None;
        true
    }

    fn reset_time_field_draft(&mut self, field: CreateRegionTimeField, clear_error: bool) {
        match field {
            CreateRegionTimeField::Start => {
                self.start_draft = format_seconds_for_edit(self.start_seconds);
                if clear_error {
                    self.start_error = None;
                }
            }
            CreateRegionTimeField::End => {
                self.end_draft = format_seconds_for_edit(self.end_seconds);
                if clear_error {
                    self.end_error = None;
                }
            }
        }
    }

    fn annotation_fields_are_complete(&self) -> bool {
        if self.intensity.is_none() {
            return false;
        }
        if !self.uses_hierarchical_labels {
            return true;
        }
        if self.primary_selection.is_empty() {
            return false;
        }

        if self.primary_selection == "other" {
            return !self.custom_description.trim().is_empty();
        }

        if self.secondary_selection.is_empty() {
            return false;
        }
        if self.secondary_selection == "other" {
            return !self.custom_description.trim().is_empty();
        }

        true
    }

    fn payload_text_and_metadata(&self) -> (String, BTreeMap<String, Value>) {
        if self.mode == CreateRegionMode::Transcription {
            return (self.text.clone(), BTreeMap::new());
        }

        if !self.uses_hierarchical_labels {
            let mut metadata = BTreeMap::new();
            metadata.insert("type".to_string(), json!(BACKGROUND_NOISE_TYPE));
            metadata.insert("intensity".to_string(), json!(self.intensity));
            return (String::new(), metadata);
        }

        let content = annotation_content(
            &self.primary_selection,
            &self.secondary_selection,
            &self.custom_description,
        );
        let mut metadata = BTreeMap::new();
        metadata.insert("type".to_string(), json!(BACKGROUND_NOISE_TYPE));
        metadata.insert("intensity".to_string(), json!(self.intensity));
        metadata.insert("content".to_string(), content.clone());

        (annotation_display_text(&content), metadata)
    }
}

pub(super) fn create_region_runtime_trace_line(
    action: &str,
    dialog: &CreateRegionDialog,
) -> String {
    format!(
        "Transcription Studio create region: action={action} mode={} lane={} start={:.3} end={:.3} enabled={} primary={} secondary={} intensity={} text_len={}",
        create_region_mode_label(dialog.mode),
        dialog.selected_lane_id,
        dialog.start_seconds,
        dialog.end_seconds,
        dialog.is_create_enabled(),
        dialog.primary_selection,
        dialog.secondary_selection,
        dialog
            .intensity
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        dialog.text.chars().count()
    )
}

pub(super) fn create_region_trace_enabled() -> bool {
    std::env::var(CREATE_REGION_TRACE_ENV).is_ok_and(|value| value != "0")
}

fn create_region_mode_label(mode: CreateRegionMode) -> &'static str {
    match mode {
        CreateRegionMode::Transcription => "transcription",
        CreateRegionMode::Annotation => "annotation",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct CreateRegionPayload {
    pub(super) lane_id: String,
    pub(super) start_seconds: f64,
    pub(super) end_seconds: f64,
    pub(super) text: String,
    pub(super) metadata: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CreateRegionTrackOption {
    pub(super) lane_id: String,
    pub(super) label: String,
}

impl fmt::Display for CreateRegionTrackOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

pub(super) fn create_region_track_options(project: &Project) -> Vec<CreateRegionTrackOption> {
    project
        .lanes
        .iter()
        .map(|lane| CreateRegionTrackOption {
            lane_id: lane.id.clone(),
            label: lane.label.clone(),
        })
        .collect()
}

pub(super) fn selected_create_region_track_option(
    options: &[CreateRegionTrackOption],
    lane_id: &str,
) -> Option<CreateRegionTrackOption> {
    options
        .iter()
        .find(|option| option.lane_id == lane_id)
        .cloned()
}

pub(super) fn create_region_primary_options() -> Vec<String> {
    [
        "speech-sounds (optional)",
        "non-speech-sounds",
        "distorted-speech",
        "background-noise",
        "background-speech",
        "plosive",
        "system-noise",
        "other",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub(super) fn create_region_secondary_options(primary: &str) -> Vec<String> {
    let mut options: Vec<String> = secondary_category_options_for(primary)
        .into_iter()
        .map(str::to_string)
        .collect();
    if !options.is_empty() {
        options.push("other".to_string());
    }
    options
}

pub(super) fn create_region_intensity_options() -> Vec<String> {
    [
        "1 - Barely noticeable",
        "2 - Noticeable but not disruptive",
        "3 - Disruptive",
        "4 - Severe",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub(super) fn intensity_value_from_label(value: &str) -> Option<u8> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(value) = value.parse::<u8>() {
        return Some(value);
    }

    value
        .chars()
        .next()
        .and_then(|character| character.to_digit(10))
        .and_then(|value| u8::try_from(value).ok())
}

pub(super) fn intensity_label(value: Option<u8>) -> Option<String> {
    let value = value?;
    create_region_intensity_options()
        .into_iter()
        .find(|option| option.starts_with(&value.to_string()))
}

fn project_uses_annotation_mode(project: &Project) -> bool {
    project
        .metadata
        .get("studio.babel.mode")
        .is_some_and(|mode| mode == "annotation")
        || project.segments.iter().any(|segment| {
            segment
                .metadata
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|annotation_type| annotation_type != BABEL_TRANSCRIPTION_TYPE)
        })
}

pub(super) fn project_uses_hierarchical_annotation_labels(project: &Project) -> bool {
    matches!(
        project.id.as_str(),
        BACKGROUND_NOISE_ANNOTATION_PROJECT_ID | BACKGROUND_NOISE_ANNOTATION_FTE_PROJECT_ID
    )
}

pub(super) fn secondary_category_options_for(primary: &str) -> Vec<&'static str> {
    match primary {
        "speech-sounds (optional)" => vec!["main speech", "interjection (um, uh, mhm, ah, ya)"],
        "non-speech-sounds" => vec![
            "cough",
            "sneeze",
            "sigh",
            "laugh",
            "breath",
            "throat clear",
            "mouth click",
        ],
        "distorted-speech" => vec![
            "audio smear",
            "muffle",
            "echo (channel bleed)",
            "dropout/network",
        ],
        "background-noise" => vec![
            "rustle",
            "mic bump",
            "tap",
            "click",
            "keyboard",
            "wind",
            "animal",
            "other environmental noise",
        ],
        "background-speech" => vec!["background speech"],
        "plosive" => vec!["plosive"],
        "system-noise" => vec!["static", "static-burst"],
        _ => Vec::new(),
    }
}

pub(super) fn annotation_content(
    primary: &str,
    secondary: &str,
    custom_description: &str,
) -> Value {
    let secondary_label = if primary == "other" || secondary == "other" {
        let custom = custom_description.trim();
        if custom.is_empty() {
            Value::Null
        } else {
            json!(custom)
        }
    } else if secondary.is_empty() {
        Value::Null
    } else {
        json!(secondary)
    };

    json!({
        "primaryLabel": primary,
        "secondaryLabel": secondary_label,
    })
}

pub(super) fn annotation_display_text(content: &Value) -> String {
    let primary = content.get("primaryLabel").and_then(Value::as_str);
    let secondary = content.get("secondaryLabel").and_then(Value::as_str);
    match (primary, secondary) {
        (Some(primary), Some(secondary)) if !secondary.is_empty() => {
            format!("{primary} / {secondary}")
        }
        (Some(primary), _) => primary.to_string(),
        _ => String::new(),
    }
}

pub(super) fn should_prevent_create_region_interact_outside(
    context: OutsideInteractionContext,
) -> bool {
    context.has_recovered_select_or_portal_target()
}
