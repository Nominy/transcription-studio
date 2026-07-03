use super::super::StudioState;

impl StudioState {
    pub(in crate::app) fn undo(&mut self) {
        if self.with_editable_workflow_mut("Undo", |workflow| {
            workflow.undo().map_err(|error| error.to_string())?;
            Ok("Undid last edit.".to_string())
        }) {
            self.reset_segment_drafts();
            self.mark_dirty();
        }
    }

    pub(in crate::app) fn redo(&mut self) {
        if self.with_editable_workflow_mut("Redo", |workflow| {
            workflow.redo().map_err(|error| error.to_string())?;
            Ok("Redid last edit.".to_string())
        }) {
            self.reset_segment_drafts();
            self.mark_dirty();
        }
    }
}
