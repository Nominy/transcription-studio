use super::super::StudioState;
use super::super::formatting::parse_draft_times;

impl StudioState {
    #[cfg(test)]
    pub(in crate::app) fn apply_segment(&mut self, segment_id: &str) {
        self.apply_segment_draft(segment_id, true);
    }

    #[allow(dead_code)]
    pub(in crate::app) fn apply_segment_draft(
        &mut self,
        segment_id: &str,
        report_no_change: bool,
    ) -> bool {
        if self.block_read_only_edit("Apply") {
            return false;
        }

        let Some(draft) = self.segment_drafts.get(segment_id) else {
            self.status = format!("Segment `{segment_id}` has no editable draft.");
            return false;
        };

        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .cloned()
        else {
            self.status = format!("Segment `{segment_id}` was not found.");
            return false;
        };

        let (start_seconds, end_seconds) = match parse_draft_times(&segment, draft) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        if start_seconds >= end_seconds {
            self.status = "Start must be before end.".to_string();
            return false;
        }

        let timing_changed =
            segment.start_seconds != start_seconds || segment.end_seconds != end_seconds;
        let text_changed = segment.text != draft.text;
        if !timing_changed && !text_changed {
            if report_no_change {
                self.status = format!("No changes to apply for `{segment_id}`.");
            }
            return true;
        }

        let text = draft.text.clone();
        let segment_id = segment_id.to_string();
        let applied = self.with_editable_workflow_mut("Apply", |workflow| {
            workflow
                .apply_segment_edit(&segment_id, start_seconds, end_seconds, text)
                .map_err(|error| error.to_string())?;
            Ok(format!("Applied edits to `{segment_id}`."))
        });
        if applied {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
        }
        applied
    }
}
