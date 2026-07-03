use super::super::browser_input::OutsideInteractionContext;
use super::super::edit_annotation::{
    edit_annotation_runtime_trace_line, edit_annotation_trace_enabled,
    should_close_edit_annotation_on_outside_mouse_down,
};
use super::super::{EditAnnotationDialog, StudioState};

impl StudioState {
    pub(in crate::app) fn request_edit_annotation(&mut self, segment_id: &str) {
        if self.block_read_only_edit("Edit") {
            self.edit_annotation_dialog = None;
            return;
        }

        let Some(project) = self.project().cloned() else {
            self.edit_annotation_dialog = None;
            self.status = "Edit unavailable: no project loaded.".to_string();
            return;
        };
        let Some(segment) = project.segment(segment_id).cloned() else {
            self.edit_annotation_dialog = None;
            self.status = format!("Edit unavailable: segment `{segment_id}` was not found.");
            return;
        };

        self.select_segment_without_seek(segment_id);
        let dialog = EditAnnotationDialog::for_project_segment(&project, &segment);
        if edit_annotation_trace_enabled() {
            eprintln!("{}", edit_annotation_runtime_trace_line("open", &dialog));
        }
        self.edit_annotation_dialog = Some(dialog);
        self.status = "Edit Annotation opened.".to_string();
    }

    pub(in crate::app) fn change_edit_annotation_primary_category(&mut self, value: String) {
        if let Some(dialog) = &mut self.edit_annotation_dialog {
            dialog.set_primary_selection(value);
        }
    }

    pub(in crate::app) fn change_edit_annotation_secondary_category(&mut self, value: String) {
        if let Some(dialog) = &mut self.edit_annotation_dialog {
            dialog.set_secondary_selection(value);
        }
    }

    pub(in crate::app) fn change_edit_annotation_custom_description(&mut self, value: String) {
        if let Some(dialog) = &mut self.edit_annotation_dialog {
            dialog.set_custom_description(value);
        }
    }

    pub(in crate::app) fn change_edit_annotation_intensity(&mut self, value: String) {
        if let Some(dialog) = &mut self.edit_annotation_dialog {
            dialog.set_intensity_from_label(&value);
        }
    }

    pub(in crate::app) fn change_edit_annotation_select_open(&mut self, open: bool) {
        if let Some(dialog) = &mut self.edit_annotation_dialog {
            dialog.select_open = open;
        }
    }

    pub(in crate::app) fn mouse_down_outside_edit_annotation(
        &mut self,
        context: OutsideInteractionContext,
    ) {
        let should_close = self.edit_annotation_dialog.as_ref().is_some_and(|dialog| {
            should_close_edit_annotation_on_outside_mouse_down(dialog.select_open, context)
        });
        if should_close {
            self.cancel_edit_annotation();
        }
    }

    pub(in crate::app) fn cancel_edit_annotation(&mut self) {
        if edit_annotation_trace_enabled()
            && let Some(dialog) = &self.edit_annotation_dialog
        {
            eprintln!("{}", edit_annotation_runtime_trace_line("cancel", dialog));
        }
        self.edit_annotation_dialog = None;
    }

    pub(in crate::app) fn submit_edit_annotation(&mut self) {
        if self.block_read_only_edit("Annotation edit") {
            return;
        }

        let Some(mut dialog) = self.edit_annotation_dialog.take() else {
            self.status = "Edit unavailable: no annotation edit is open.".to_string();
            return;
        };

        let payload = match dialog.save_payload() {
            Ok(payload) => payload,
            Err(error) => {
                dialog.error = Some(error.clone());
                self.status = error;
                self.edit_annotation_dialog = Some(dialog);
                return;
            }
        };

        let segment_id = payload.segment_id.clone();
        if self.with_editable_workflow_mut("Annotation edit", |workflow| {
            workflow
                .update_annotation_metadata(
                    &payload.segment_id,
                    payload.text,
                    payload.intensity,
                    payload.content,
                )
                .map_err(|error| error.to_string())?;
            Ok(format!("Saved annotation `{}`.", payload.segment_id))
        }) {
            if edit_annotation_trace_enabled() {
                eprintln!("{}", edit_annotation_runtime_trace_line("submit", &dialog));
            }
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project();
        } else {
            self.edit_annotation_dialog = Some(dialog);
        }
    }
}
