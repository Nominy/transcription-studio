//! Add-region and edit-annotation dialog lifecycles.

use super::{EditError, EditResult, EditorDoc, Effect, Outcome};
use crate::edit::annotation::EditAnnotationDialog;
use crate::edit::create_region::{CreateRegionDialog, CreateRegionMode, CreateRegionTimeField};

impl EditorDoc {
    // ---- add region -------------------------------------------------------------------

    /// Opens the add-region dialog for a range (mode follows the project).
    pub fn open_create_region(&mut self, lane_id: &str, start_seconds: f64, end_seconds: f64) {
        self.create_region = Some(CreateRegionDialog::new_for_project(
            self.workflow.project(),
            lane_id.to_string(),
            start_seconds,
            end_seconds,
        ));
    }

    /// Plain field edits (track, start/end drafts, categories, intensity, description).
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

        if dialog.mode == CreateRegionMode::Transcription
            && let Some(segment_id) = dialog.created_segment_id.clone()
        {
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
            workflow.create_segment_in_lane_with_text_and_metadata(
                &payload.lane_id,
                payload.start_seconds,
                payload.end_seconds,
                payload.text,
                payload.metadata,
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

    // ---- edit annotation --------------------------------------------------------------

    /// Opens the annotation editor for a row (selects it without seeking).
    pub fn open_edit_annotation(&mut self, segment_id: &str) -> EditResult {
        if let Err(error) = self.guard("Edit") {
            self.edit_annotation = None;
            return Err(error);
        }
        let segment = match self.segment(segment_id) {
            Ok(segment) => segment,
            Err(error) => {
                self.edit_annotation = None;
                return Err(error);
            }
        };
        let outcome = self.select_segment_without_seek(segment_id);
        self.edit_annotation = Some(EditAnnotationDialog::for_project_segment(
            self.workflow.project(),
            &segment,
        ));
        Ok(outcome)
    }

    /// Category/intensity/description setters live on the dialog.
    pub fn edit_annotation_mut(&mut self) -> Option<&mut EditAnnotationDialog> {
        self.edit_annotation.as_mut()
    }

    pub fn cancel_edit_annotation(&mut self) {
        self.edit_annotation = None;
    }

    /// Saves intensity/content/label text (one undo step). A pending text draft of the row is
    /// kept; otherwise the row draft takes the new label text.
    pub fn submit_edit_annotation(&mut self) -> EditResult {
        self.guard("Annotation edit")?;
        let mut dialog = self.edit_annotation.take().ok_or_else(|| {
            EditError::Unavailable("Edit unavailable: no annotation edit is open.".into())
        })?;
        let payload = match dialog.save_payload() {
            Ok(payload) => payload,
            Err(error) => {
                dialog.error = Some(error);
                self.edit_annotation = Some(dialog);
                return Err(error.into());
            }
        };
        let segment_id = payload.segment_id.clone();
        let has_pending_text = self
            .workflow
            .project()
            .segment(&segment_id)
            .zip(self.drafts.get(&segment_id))
            .is_some_and(|(segment, draft)| segment.text != draft.text());
        if let Err(error) = self.apply("Annotation edit", |workflow| {
            workflow.update_annotation_metadata(
                &payload.segment_id,
                payload.text,
                payload.intensity,
                payload.content,
            )
        }) {
            self.edit_annotation = Some(dialog);
            return Err(error);
        }
        let mut outcome = Outcome::status(format!(
            "Saved annotation {}.",
            self.segment_label(&segment_id)
        ));
        if !has_pending_text
            && let Some(text) = self
                .workflow
                .project()
                .segment(&segment_id)
                .map(|segment| segment.text.clone())
            && let Some(draft) = self.drafts.get_mut(&segment_id)
            && draft.text() != text
        {
            draft.replace_text(text);
            outcome = outcome.with(Effect::DraftChanged { segment_id });
        }
        self.mark_dirty();
        self.reconcile(false);
        Ok(outcome)
    }
}
