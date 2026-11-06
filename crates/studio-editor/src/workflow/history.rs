use anyhow::{Context, Result, bail};
use studio_core::EditorCommand;

use super::StudioWorkflow;

impl StudioWorkflow {
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn apply_text_edit(&mut self, segment_id: &str, text: impl Into<String>) -> Result<()> {
        let text = text.into();
        let should_coalesce =
            self.pending_boundary_edit_segment_id.get_mut().as_deref() == Some(segment_id);
        if should_coalesce {
            self.pending_boundary_edit_segment_id.get_mut().take();
            let segment = self
                .project
                .segment_mut(segment_id)
                .with_context(|| format!("segment `{segment_id}` was not found"))?;
            segment.text = text;
            self.mutation_policy.updated(segment);
            return Ok(());
        }

        self.apply(EditorCommand::ApplyTextEdit {
            segment_id: segment_id.to_string(),
            text,
        })
    }

    pub fn move_segment_boundary(
        &mut self,
        segment_id: &str,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Result<()> {
        let should_coalesce =
            self.pending_boundary_edit_segment_id.get_mut().as_deref() == Some(segment_id);
        if should_coalesce {
            if !start_seconds.is_finite()
                || !end_seconds.is_finite()
                || start_seconds >= end_seconds
            {
                bail!("time range must have start before end");
            }
            let segment = self
                .project
                .segment_mut(segment_id)
                .with_context(|| format!("segment `{segment_id}` was not found"))?;
            segment.start_seconds = start_seconds;
            segment.end_seconds = end_seconds;
            self.mutation_policy.updated(segment);
            return Ok(());
        }

        self.apply(EditorCommand::MoveSegmentBoundary {
            segment_id: segment_id.to_string(),
            start_seconds,
            end_seconds,
        })?;
        *self.pending_boundary_edit_segment_id.get_mut() = Some(segment_id.to_string());
        Ok(())
    }

    pub fn undo(&mut self) -> Result<()> {
        self.pending_boundary_edit_segment_id.get_mut().take();
        self.history
            .undo(&mut self.project)
            .context("failed to undo command")
    }

    pub fn redo(&mut self) -> Result<()> {
        self.pending_boundary_edit_segment_id.get_mut().take();
        self.history
            .redo(&mut self.project)
            .context("failed to redo command")
    }

    pub fn finish_boundary_edit(&mut self) {
        self.pending_boundary_edit_segment_id.get_mut().take();
    }

    /// Drops every retained undo/redo snapshot, freeing the retained project
    /// clones (up to `CommandHistory::MAX_HISTORY_BYTES` per stack). Called
    /// when the workspace returns to the dashboard; reopening a project
    /// always builds a fresh workflow via `replace_workflow`, so no flow can
    /// observe the cleared history.
    pub fn clear_history(&mut self) {
        self.pending_boundary_edit_segment_id.get_mut().take();
        self.history.clear();
    }

    pub fn apply(&mut self, command: EditorCommand) -> Result<()> {
        self.pending_boundary_edit_segment_id.get_mut().take();
        self.history
            .apply_with_policy(&mut self.project, command, self.mutation_policy.as_ref())
            .context("failed to apply editor command")
    }

    /// Applies several commands as one undo snapshot.
    pub fn apply_batch(&mut self, commands: Vec<EditorCommand>) -> Result<()> {
        self.pending_boundary_edit_segment_id.get_mut().take();
        self.history
            .apply_batch_with_policy(&mut self.project, commands, self.mutation_policy.as_ref())
            .context("failed to apply editor commands")
    }

    /// Applies a validated domain transaction atomically as one undo snapshot.
    pub fn apply_project_transaction(
        &mut self,
        _label: &str,
        update: impl FnOnce(&mut studio_core::Project) -> Result<()>,
    ) -> Result<()> {
        self.history.apply_transaction(&mut self.project, update)?;
        self.pending_boundary_edit_segment_id.get_mut().take();
        Ok(())
    }

    pub fn clear_pending_boundary_edit(&self) {
        self.pending_boundary_edit_segment_id.borrow_mut().take();
    }
}
