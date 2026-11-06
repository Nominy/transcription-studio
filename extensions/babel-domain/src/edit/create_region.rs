//! "Add region" dialog state: transcription regions (text) and annotation regions (intensity +
//! optional hierarchical categories), time-field drafts, validation and the create payload.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use studio_core::Project;

use crate::i18n::{is_russian, localized};
use studio_editor::edit::format::{format_time, parse_time};
use studio_editor::edit::time::TimeEditError;

const BABEL_TRANSCRIPTION_TYPE: &str = "transcription";

const BACKGROUND_NOISE_TYPE: &str = "background_noise";
/// Projects whose annotation regions use primary/secondary category labels.
pub const BACKGROUND_NOISE_ANNOTATION_PROJECT_ID: &str = "4c0db8c4-699e-4014-9dbf-27f178c3da8e";
pub const BACKGROUND_NOISE_ANNOTATION_FTE_PROJECT_ID: &str = "7cb04a85-c9f3-4d73-afb9-34156777948f";
/// Category value that requires a free-text description.
pub const OTHER_CATEGORY: &str = "other";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateRegionTimeField {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateRegionMode {
    Transcription,
    Annotation,
}

/// The open "add region" dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateRegionDialog {
    pub mode: CreateRegionMode,
    /// Set when the waveform drag already created the (transcription) segment; the dialog then
    /// only edits that segment's text.
    pub created_segment_id: Option<String>,
    pub selected_lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    pub intensity: Option<u8>,
    pub uses_hierarchical_labels: bool,
    pub primary_selection: String,
    pub secondary_selection: String,
    pub custom_description: String,
    pub active_time_field: Option<CreateRegionTimeField>,
    pub start_draft: String,
    pub end_draft: String,
    pub start_error: Option<TimeEditError>,
    pub end_error: Option<TimeEditError>,
}

/// Validated "create" request.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateRegionPayload {
    pub lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    pub metadata: BTreeMap<String, Value>,
}

/// Why the dialog cannot be submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateRegionError {
    Time(TimeEditError),
    UnknownLane,
    IncompleteAnnotation,
}

impl std::fmt::Display for CreateRegionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Time(error) => error.fmt(f),
            Self::UnknownLane => f.write_str(localized("Choose a speaker.", "Выберите спикера.")),
            Self::IncompleteAnnotation => f.write_str(localized(
                "Choose an intensity and both categories.",
                "Выберите интенсивность и обе категории.",
            )),
        }
    }
}

impl std::error::Error for CreateRegionError {}

impl CreateRegionDialog {
    /// Transcription-mode dialog.
    pub fn new(selected_lane_id: String, start_seconds: f64, end_seconds: f64) -> Self {
        Self::with_mode(
            CreateRegionMode::Transcription,
            selected_lane_id,
            start_seconds,
            end_seconds,
            false,
        )
    }

    /// Annotation-mode dialog (hierarchical labels on).
    pub fn new_annotation(selected_lane_id: String, start_seconds: f64, end_seconds: f64) -> Self {
        Self::with_mode(
            CreateRegionMode::Annotation,
            selected_lane_id,
            start_seconds,
            end_seconds,
            true,
        )
    }

    /// Dialog whose mode and labels follow the project (annotation projects get annotation mode).
    pub fn new_for_project(
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
            active_time_field: None,
            start_draft: format_time(start_seconds),
            end_draft: format_time(end_seconds),
            start_error: None,
            end_error: None,
        }
    }

    pub fn with_created_segment_id(mut self, segment_id: String) -> Self {
        self.created_segment_id = Some(segment_id);
        self
    }

    pub fn set_start_draft(&mut self, value: String) {
        self.start_draft = value;
        self.active_time_field = Some(CreateRegionTimeField::Start);
        self.start_error = None;
    }

    pub fn set_end_draft(&mut self, value: String) {
        self.end_draft = value;
        self.active_time_field = Some(CreateRegionTimeField::End);
        self.end_error = None;
    }

    pub fn set_text(&mut self, value: String) {
        self.active_time_field = None;
        self.text = value;
    }

    /// Changing the primary category clears the secondary choice and description.
    pub fn set_primary_category(&mut self, value: String) {
        self.active_time_field = None;
        self.primary_selection = value;
        self.secondary_selection.clear();
        self.custom_description.clear();
    }

    /// Choosing "other" clears the description; other choices keep it.
    pub fn set_secondary_category(&mut self, value: String) {
        self.active_time_field = None;
        if value == OTHER_CATEGORY {
            self.custom_description.clear();
        }
        self.secondary_selection = value;
    }

    pub fn set_custom_description(&mut self, value: String) {
        self.active_time_field = None;
        self.custom_description = value;
    }

    /// Accepts an intensity option label (`"2 - Noticeable..."`) or a bare number.
    pub fn set_intensity_label(&mut self, value: &str) {
        self.active_time_field = None;
        self.intensity = intensity_value_from_label(value);
    }

    /// Commits the typed start/end (Enter). Returns `false` and records the error when invalid.
    pub fn commit_time_field(&mut self, field: CreateRegionTimeField) -> bool {
        match field {
            CreateRegionTimeField::Start => {
                let parsed = parse_time(&self.start_draft).ok_or(TimeEditError::InvalidFormat);
                match parsed {
                    Ok(start) if start < self.end_seconds => {
                        self.start_seconds = start;
                    }
                    Ok(_) => {
                        self.start_error = Some(TimeEditError::StartNotBeforeEnd);
                        return false;
                    }
                    Err(error) => {
                        self.start_error = Some(error);
                        return false;
                    }
                }
            }
            CreateRegionTimeField::End => {
                let parsed = parse_time(&self.end_draft).ok_or(TimeEditError::InvalidFormat);
                match parsed {
                    Ok(end) if self.start_seconds < end => {
                        self.end_seconds = end;
                    }
                    Ok(_) => {
                        self.end_error = Some(TimeEditError::EndNotAfterStart);
                        return false;
                    }
                    Err(error) => {
                        self.end_error = Some(error);
                        return false;
                    }
                }
            }
        }
        self.start_draft = format_time(self.start_seconds);
        self.end_draft = format_time(self.end_seconds);
        self.start_error = None;
        self.end_error = None;
        true
    }

    /// Blur: commit, and on failure restore the stored value (keeping the error).
    pub fn blur_time_field(&mut self, field: CreateRegionTimeField) -> bool {
        let ok = self.commit_time_field(field);
        if !ok {
            self.reset_time_field_draft(field, false);
        }
        self.active_time_field = None;
        ok
    }

    /// Escape inside a time field: restore the stored value. Returns whether a field was active.
    pub fn reset_active_time_field_draft(&mut self) -> bool {
        let Some(field) = self.active_time_field.take() else {
            return false;
        };
        self.reset_time_field_draft(field, true);
        true
    }

    /// Whether the Create button is enabled.
    pub fn is_create_enabled(&self) -> bool {
        !self.selected_lane_id.is_empty()
            && self.end_seconds > self.start_seconds
            && self.start_error.is_none()
            && self.end_error.is_none()
            && match self.mode {
                CreateRegionMode::Transcription => true,
                CreateRegionMode::Annotation => self.annotation_fields_are_complete(),
            }
    }

    /// Validates the dialog against the project and builds the create payload.
    /// Hidden/uncommitted time drafts are parsed here too.
    pub fn validate_for_submit(
        &mut self,
        project: &Project,
    ) -> Result<CreateRegionPayload, CreateRegionError> {
        let start_seconds = parse_time(&self.start_draft).ok_or_else(|| {
            self.start_error = Some(TimeEditError::InvalidFormat);
            CreateRegionError::Time(TimeEditError::InvalidFormat)
        })?;
        let end_seconds = parse_time(&self.end_draft).ok_or_else(|| {
            self.end_error = Some(TimeEditError::InvalidFormat);
            CreateRegionError::Time(TimeEditError::InvalidFormat)
        })?;
        self.start_error = None;
        self.end_error = None;
        if start_seconds >= end_seconds {
            self.start_error = Some(TimeEditError::StartNotBeforeEnd);
            return Err(CreateRegionError::Time(TimeEditError::StartNotBeforeEnd));
        }
        if !project
            .lanes
            .iter()
            .any(|lane| lane.id == self.selected_lane_id)
        {
            return Err(CreateRegionError::UnknownLane);
        }

        self.start_seconds = start_seconds;
        self.end_seconds = end_seconds;
        self.start_draft = format_time(start_seconds);
        self.end_draft = format_time(end_seconds);
        if self.mode == CreateRegionMode::Annotation && !self.annotation_fields_are_complete() {
            return Err(CreateRegionError::IncompleteAnnotation);
        }
        let (text, metadata) = self.payload_text_and_metadata();
        Ok(CreateRegionPayload {
            lane_id: self.selected_lane_id.clone(),
            start_seconds,
            end_seconds,
            text,
            metadata,
        })
    }

    fn reset_time_field_draft(&mut self, field: CreateRegionTimeField, clear_error: bool) {
        match field {
            CreateRegionTimeField::Start => {
                self.start_draft = format_time(self.start_seconds);
                if clear_error {
                    self.start_error = None;
                }
            }
            CreateRegionTimeField::End => {
                self.end_draft = format_time(self.end_seconds);
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
        if self.primary_selection == OTHER_CATEGORY || self.secondary_selection == OTHER_CATEGORY {
            return !self.custom_description.trim().is_empty();
        }
        !self.secondary_selection.is_empty()
    }

    fn payload_text_and_metadata(&self) -> (String, BTreeMap<String, Value>) {
        if self.mode == CreateRegionMode::Transcription {
            return (self.text.clone(), BTreeMap::new());
        }
        let mut metadata = BTreeMap::new();
        metadata.insert("type".to_string(), json!(BACKGROUND_NOISE_TYPE));
        metadata.insert("intensity".to_string(), json!(self.intensity));
        if !self.uses_hierarchical_labels {
            return (String::new(), metadata);
        }
        let content = annotation_content(
            &self.primary_selection,
            &self.secondary_selection,
            &self.custom_description,
        );
        metadata.insert("content".to_string(), content.clone());
        (annotation_display_text(&content), metadata)
    }
}

/// Speaker-lane option for the dialog's track picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackOption {
    pub lane_id: String,
    pub label: String,
}

/// One option per project lane, labelled with the lane label.
pub fn track_options(project: &Project) -> Vec<TrackOption> {
    project
        .lanes
        .iter()
        .map(|lane| TrackOption {
            lane_id: lane.id.clone(),
            label: lane.label.clone(),
        })
        .collect()
}

/// Primary annotation categories.
pub fn primary_category_options() -> &'static [&'static str] {
    &[
        "speech-sounds (optional)",
        "non-speech-sounds",
        "distorted-speech",
        "background-noise",
        "background-speech",
        "plosive",
        "system-noise",
        OTHER_CATEGORY,
    ]
}

/// Secondary options for `primary`, ending with "other" when non-empty.
pub fn secondary_category_options(primary: &str) -> Vec<&'static str> {
    let mut options = secondary_category_options_for(primary);
    if !options.is_empty() {
        options.push(OTHER_CATEGORY);
    }
    options
}

/// Intensity option labels (`1`..`4`) in the current UI language.
pub fn intensity_options() -> &'static [&'static str] {
    const EN: [&str; 4] = [
        "1 – Barely noticeable",
        "2 – Noticeable but not disruptive",
        "3 – Disruptive",
        "4 – Severe",
    ];
    const RU: [&str; 4] = [
        "1 – Едва заметно",
        "2 – Заметно, но не мешает",
        "3 – Мешает",
        "4 – Сильно мешает",
    ];
    if is_russian() { &RU } else { &EN }
}

/// Display label for a stored primary or secondary category value; unknown values (custom
/// descriptions) are shown as they are.
pub fn category_label(value: &str) -> &str {
    let (en, ru) = match value {
        "speech-sounds (optional)" => ("Speech sounds (optional)", "Речевые звуки (необязательно)"),
        "non-speech-sounds" => ("Non-speech sounds", "Неречевые звуки"),
        "distorted-speech" => ("Distorted speech", "Искажённая речь"),
        "background-noise" => ("Background noise", "Фоновый шум"),
        "background-speech" | "background speech" => ("Background speech", "Фоновая речь"),
        "plosive" => ("Plosive", "Взрывной звук"),
        "system-noise" => ("System noise", "Системный шум"),
        OTHER_CATEGORY => ("Other", "Другое"),
        "main speech" => ("Main speech", "Основная речь"),
        "interjection (um, uh, mhm, ah, ya)" => (
            "Interjection (um, uh, mhm, ah, ya)",
            "Междометие (эм, э-э, мгм, ах, ага)",
        ),
        "cough" => ("Cough", "Кашель"),
        "sneeze" => ("Sneeze", "Чихание"),
        "sigh" => ("Sigh", "Вздох"),
        "laugh" => ("Laugh", "Смех"),
        "breath" => ("Breath", "Дыхание"),
        "throat clear" => ("Throat clear", "Прочистка горла"),
        "mouth click" => ("Mouth click", "Щелчок губ"),
        "audio smear" => ("Audio smear", "Размытый звук"),
        "muffle" => ("Muffle", "Приглушение"),
        "echo (channel bleed)" => ("Echo (channel bleed)", "Эхо (просачивание канала)"),
        "dropout/network" => ("Dropout or network loss", "Пропадание звука (сеть)"),
        "rustle" => ("Rustle", "Шуршание"),
        "mic bump" => ("Mic bump", "Удар по микрофону"),
        "tap" => ("Tap", "Стук"),
        "click" => ("Click", "Щелчок"),
        "keyboard" => ("Keyboard", "Клавиатура"),
        "wind" => ("Wind", "Ветер"),
        "animal" => ("Animal", "Животное"),
        "other environmental noise" => ("Other environmental noise", "Другой шум окружения"),
        "static" => ("Static", "Статика"),
        "static-burst" => ("Static burst", "Всплеск статики"),
        _ => return value,
    };
    localized(en, ru)
}

/// Parses an intensity option label or bare number (`"0"` is accepted).
pub fn intensity_value_from_label(value: &str) -> Option<u8> {
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
        .and_then(|c| c.to_digit(10))
        .and_then(|v| u8::try_from(v).ok())
}

/// Option label for an intensity value.
pub fn intensity_label(value: Option<u8>) -> Option<&'static str> {
    let value = value?.to_string();
    intensity_options()
        .iter()
        .copied()
        .find(|option| option.starts_with(&value))
}

/// Annotation projects: `studio.babel.mode = annotation` or any non-transcription segment.
pub fn project_uses_annotation_mode(project: &Project) -> bool {
    project
        .metadata
        .get("studio.babel.mode")
        .is_some_and(|mode| mode == "annotation")
        || project.segments.iter().any(|segment| {
            segment
                .metadata
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != BABEL_TRANSCRIPTION_TYPE)
        })
}

pub fn project_uses_hierarchical_annotation_labels(project: &Project) -> bool {
    matches!(
        project.id.as_str(),
        BACKGROUND_NOISE_ANNOTATION_PROJECT_ID | BACKGROUND_NOISE_ANNOTATION_FTE_PROJECT_ID
    )
}

/// Secondary categories for `primary`, without the trailing "other".
pub fn secondary_category_options_for(primary: &str) -> Vec<&'static str> {
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

/// Babel `content` JSON (`primaryLabel`/`secondaryLabel`); "other" uses the description.
pub fn annotation_content(primary: &str, secondary: &str, custom_description: &str) -> Value {
    let secondary_label = if primary == OTHER_CATEGORY || secondary == OTHER_CATEGORY {
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

/// Row text for annotation content: `"primary / secondary"` or `"primary"`.
pub fn annotation_display_text(content: &Value) -> String {
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
