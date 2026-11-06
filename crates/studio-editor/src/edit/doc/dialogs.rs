//! Plain-text add-region dialog lifecycle.

use super::{EditError, EditResult, EditorDoc, Effect, Outcome};
use crate::edit::create_region::{CreateRegionDialog, CreateRegionTimeField};

impl EditorDoc {
    // ---- add region -------------------------------------------------------------------

    /// Opens the add-region dialog for a plain-text range.
    pub fn open_create_region(&mut self, lane_id: &str, start_seconds: f64, end_seconds: f64) {
        self.create_region = Some(CreateRegionDialog::new(
            lane_id.to_string(),
            start_seconds,
            end_seconds,
        ));
    }

    /// Plain field edits (track and start/end drafts).
    /// Use [`EditorDoc::set_create_region_text`] for the text field.
    pub fn create_region_mut(&mut self) -> Option<&mut CreateRegionDialog> {
        self.create_region.as_mut()
    }

    /// Text of the region; for a drag-created row the row draft follows immediately.
    pub fn set_create_region_text(&mut self, value: String) -> Outcome {
        let Some(dialog) = self.create_region.as_mut() else {
            return Outcome::none();
        };
        dialog.set_text(value.clone());
        let Some(segment_id) = dialog.created_segment_id.clone() else {
            return Outcome::none();
        };
        let changed = self
            .drafts
            .get_mut(&segment_id)
            .is_some_and(|draft| draft.replace_text_from_user_edit(value));
        if !changed {
            return Outcome::none();
        }
        self.touch();
        Outcome::none().with(Effect::DraftChanged { segment_id })
    }

    /// Enter in a start/end field.
    pub fn commit_create_region_time(&mut self, field: CreateRegionTimeField) -> EditResult {
        let dialog = self
            .create_region
            .as_mut()
            .ok_or_else(|| EditError::Unavailable("No region draft is open.".into()))?;
        if dialog.commit_time_field(field) {
            return Ok(Outcome::status("Region time updated."));
        }
        let error = dialog
            .start_error
            .clone()
            .or_else(|| dialog.end_error.clone())
            .unwrap_or(crate::edit::time::TimeEditError::InvalidFormat);
        Err(EditError::Time(error))
    }

    /// Escape inside a time field (or other dialog input): restores the active time field.
    pub fn reset_create_region_time(&mut self) -> bool {
        self.create_region
            .as_mut()
            .is_some_and(CreateRegionDialog::reset_active_time_field_draft)
    }

    pub fn cancel_create_region(&mut self) {
        self.create_region = None;
    }

    /// Create (Ctrl/Cmd+Enter): a drag-created transcription row saves its text; otherwise the
    /// dialog is validated and a new row is created (one undo step) and selected. The dialog
    /// stays open with errors when invalid.
    pub fn submit_create_region(&mut self) -> EditResult {
        self.guard("Create")?;
        let mut dialog = self.create_region.take().ok_or_else(|| {
            EditError::Unavailable("Create unavailable: no region draft is open.".into())
        })?;

        if let Some(segment_id) = dialog.created_segment_id.clone() {
            let segment = match self.segment(&segment_id) {
                Ok(segment) => segment,
                Err(error) => {
                    self.create_region = Some(dialog);
                    return Err(error);
                }
            };
            if segment.text != dialog.text {
                let text = dialog.text.clone();
                if let Err(error) = self.apply("Create", |workflow| {
                    workflow.apply_text_edit(&segment_id, text)
                }) {
                    self.create_region = Some(dialog);
                    return Err(error);
                }
            }
            self.mark_dirty();
            self.drafts.refresh(self.workflow.project(), &segment_id);
            self.reconcile(true);
            let outcome = self.select_segment_without_seek(&segment_id);
            let status = format!("Saved text for {}.", self.segment_label(&segment_id));
            return Ok(outcome
                .with_status(status)
                .with(Effect::DraftChanged { segment_id }));
        }

        let payload = match dialog.validate_for_submit(self.workflow.project()) {
            Ok(payload) => payload,
            Err(error) => {
                self.create_region = Some(dialog);
                return Err(error.into());
            }
        };
        let (start, end) = (payload.start_seconds, payload.end_seconds);
        let created = match self.apply("Create", |workflow| {
            workflow.create_segment_in_lane_with_text(
                &payload.lane_id,
                payload.start_seconds,
                payload.end_seconds,
                payload.text,
            )
        }) {
            Ok(created) => created,
            Err(error) => {
                self.create_region = Some(dialog);
                return Err(error);
            }
        };
        self.mark_dirty();
        self.reconcile(true);
        let outcome = self.select_segment_without_seek(&created);
        let status = format!(
            "Created {} at {start:.3}s-{end:.3}s.",
            self.segment_label(&created)
        );
        Ok(outcome.with_status(status))
    }
}
