use anyhow::{Context, Result, bail};
use serde_json::json;
use studio_babel::META_REVIEW_ACTION_DECISION;
use studio_core::EditorCommand;

use super::{StudioWorkflow, current_babel_timestamp};

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
            stamp_babel_mutation(segment);
            return Ok(());
        }

        self.apply(EditorCommand::ApplyTextEdit {
            segment_id: segment_id.to_string(),
            text,
        })
    }

    pub fn resolve_low_confidence(&mut self, segment_id: &str) -> Result<()> {
        self.apply(EditorCommand::ResolveLowConfidence {
            segment_id: segment_id.to_string(),
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
            stamp_babel_mutation(segment);
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

    pub fn mark_review_passed(&mut self) {
        self.clear_pending_boundary_edit();
        self.project
            .metadata
            .insert(META_REVIEW_ACTION_DECISION.to_string(), "pass".to_string());
    }

    pub(super) fn apply(&mut self, command: EditorCommand) -> Result<()> {
        self.pending_boundary_edit_segment_id.get_mut().take();
        self.history
            .apply_with_timestamp(&mut self.project, command, Some(current_babel_timestamp()))
            .context("failed to apply editor command")
    }

    pub(super) fn clear_pending_boundary_edit(&self) {
        self.pending_boundary_edit_segment_id.borrow_mut().take();
    }
}

fn stamp_babel_mutation(segment: &mut studio_core::Segment) {
    let updated_at = current_babel_timestamp();
    segment
        .metadata
        .insert("lowConfidenceResolved".to_string(), json!(true));
    segment
        .metadata
        .insert("updatedAt".to_string(), json!(updated_at));
}
