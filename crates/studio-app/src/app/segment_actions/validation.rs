use studio_app::{LintSeverity, TranscriptRowView};
use studio_core::SegmentUpdate;

use super::super::formatting::parse_draft_times;
use super::super::{SegmentDraft, StudioState};

impl StudioState {
    pub(in crate::app) fn pending_draft_segment_ids(&self) -> Result<Vec<String>, String> {
        let Some(project) = self.project() else {
            return Ok(Vec::new());
        };

        project
            .segments
            .iter()
            .filter_map(|segment| match self.segment_draft_differs(segment) {
                Ok(true) => Some(Ok(segment.id.clone())),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    pub(in crate::app) fn draft_lint_status(
        &self,
        row_view: &TranscriptRowView,
        draft: &SegmentDraft,
    ) -> (LintSeverity, String) {
        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(&row_view.segment_id))
        else {
            return (LintSeverity::Error, "segment is missing".to_string());
        };

        let (start_seconds, end_seconds) = match parse_draft_times(segment, draft) {
            Ok(value) => value,
            Err(error) => return (LintSeverity::Error, error),
        };
        if start_seconds >= end_seconds {
            return (LintSeverity::Error, "Start must be before end".to_string());
        }
        if draft.text.trim().is_empty() {
            return (LintSeverity::Error, "text is empty".to_string());
        }
        if segment.start_seconds != start_seconds
            || segment.end_seconds != end_seconds
            || segment.text != draft.text
        {
            return (LintSeverity::Warning, "Draft pending".to_string());
        }

        (row_view.lint.severity, row_view.lint.message.clone())
    }

    pub(in crate::app) fn apply_pending_drafts(&mut self, action_name: &str) -> bool {
        let pending_ids = match self.pending_draft_segment_ids() {
            Ok(pending_ids) => pending_ids,
            Err(error) => {
                self.status = format!("{action_name} blocked: {error}");
                return false;
            }
        };

        if pending_ids.is_empty() {
            return true;
        }

        let Some(project) = self.project() else {
            return true;
        };
        let mut updates = Vec::with_capacity(pending_ids.len());
        for segment_id in pending_ids {
            let Some(segment) = project.segment(&segment_id) else {
                self.status =
                    format!("{action_name} blocked: segment `{segment_id}` was not found.");
                return false;
            };
            let Some(draft) = self.segment_drafts.get(&segment_id) else {
                self.status =
                    format!("{action_name} blocked: segment `{segment_id}` has no editable draft.");
                return false;
            };
            let (start_seconds, end_seconds) = match parse_draft_times(segment, draft) {
                Ok(value) => value,
                Err(error) => {
                    self.status = format!("{action_name} blocked: {error}");
                    return false;
                }
            };
            if start_seconds >= end_seconds {
                self.status = format!("{action_name} blocked: Start must be before end.");
                return false;
            }
            updates.push(SegmentUpdate {
                segment_id,
                start_seconds,
                end_seconds,
                text: draft.text.clone(),
            });
        }

        let update_count = updates.len();
        if self.with_editable_workflow_mut(action_name, |workflow| {
            workflow
                .apply_segment_edits(updates)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "{action_name}: applied {update_count} pending draft rows."
            ))
        }) {
            self.reset_segment_drafts();
            self.mark_dirty();
            self.invalidate_transcript_layout_cache();
            true
        } else {
            false
        }
    }
}
