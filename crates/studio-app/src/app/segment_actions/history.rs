use super::super::StudioState;

impl StudioState {
    fn block_history_for_pending_drafts(&mut self, action: &str) -> bool {
        let has_pending_drafts = match self.pending_draft_segment_ids() {
            Ok(segment_ids) => !segment_ids.is_empty(),
            Err(_) => true,
        };
        if !has_pending_drafts {
            return false;
        }

        self.status = format!(
            "{action} blocked: use the focused text editor's local undo or save transcript edits first."
        );
        true
    }

    pub(in crate::app) fn undo(&mut self) {
        if self.block_history_for_pending_drafts("Undo") {
            return;
        }

        if self.with_editable_workflow_mut("Undo", |workflow| {
            workflow.undo().map_err(|error| error.to_string())?;
            Ok("Undid last edit.".to_string())
        }) {
            self.reset_segment_drafts();
            self.mark_dirty();
        }
    }

    pub(in crate::app) fn redo(&mut self) {
        if self.block_history_for_pending_drafts("Redo") {
            return;
        }

        if self.with_editable_workflow_mut("Redo", |workflow| {
            workflow.redo().map_err(|error| error.to_string())?;
            Ok("Redid last edit.".to_string())
        }) {
            self.reset_segment_drafts();
            self.mark_dirty();
        }
    }
}
