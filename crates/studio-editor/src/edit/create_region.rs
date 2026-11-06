//! Plain-text region creation, time-field drafts, validation and the create payload.

use studio_core::Project;

use super::format::{format_time, parse_time};
use super::time::TimeEditError;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateRegionTimeField {
    Start,
    End,
}

/// The open "add region" dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateRegionDialog {
    pub selected_lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
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
}

/// Why the dialog cannot be submitted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CreateRegionError {
    #[error("{0}")]
    Time(TimeEditError),
    #[error("Select a valid speaker.")]
    UnknownLane,
}

impl CreateRegionDialog {
    /// Transcription-mode dialog.
    pub fn new(selected_lane_id: String, start_seconds: f64, end_seconds: f64) -> Self {
        Self {
            selected_lane_id,
            start_seconds,
            end_seconds,
            text: String::new(),
            active_time_field: None,
            start_draft: format_time(start_seconds),
            end_draft: format_time(end_seconds),
            start_error: None,
            end_error: None,
        }
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
        Ok(CreateRegionPayload {
            lane_id: self.selected_lane_id.clone(),
            start_seconds,
            end_seconds,
            text: self.text.clone(),
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
