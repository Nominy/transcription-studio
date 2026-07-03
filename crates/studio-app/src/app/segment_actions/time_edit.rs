use super::super::StudioState;
use super::super::formatting::format_seconds_for_edit;
use super::super::time_edit::{
    CommitTimeCellEditInput, CommitTimeCellEditResult, TimeEditState, TimeField,
    commit_time_cell_edit,
};

impl StudioState {
    pub(in crate::app) fn update_segment_time_draft(
        &mut self,
        segment_id: &str,
        field: TimeField,
        value: String,
    ) {
        if self.block_read_only_edit("Time edit") {
            return;
        }

        let current_value = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .map(|segment| field.value_from(segment.start_seconds, segment.end_seconds));

        let Some(draft) = self.segment_drafts.get_mut(segment_id) else {
            self.status = format!("Segment `{segment_id}` has no editable draft.");
            return;
        };

        if draft.time_edit.active_field != Some(field)
            && let Some(current_value) = current_value
        {
            draft.start_time_edit(field, current_value);
        }

        draft.set_time_value(field, value.clone());
        draft.time_edit.active_field = Some(field);
        draft.time_edit.draft_value = value;
        draft.time_edit.error = None;
        draft.time_edit.should_focus_and_select = false;
    }

    pub(in crate::app) fn commit_segment_time_edit(
        &mut self,
        segment_id: &str,
        field: TimeField,
        from_blur: bool,
    ) {
        if self.block_read_only_edit("Time edit") {
            return;
        }

        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .cloned()
        else {
            self.status = format!("Segment `{segment_id}` was not found.");
            return;
        };

        let Some(draft) = self.segment_drafts.get(segment_id) else {
            self.status = format!("Segment `{segment_id}` has no editable draft.");
            return;
        };

        let result = commit_time_cell_edit(CommitTimeCellEditInput {
            active_field: draft.time_edit.active_field,
            field,
            draft_value: draft.time_value(field).to_string(),
            value: field.value_from(segment.start_seconds, segment.end_seconds),
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
            annotation_id: segment_id.to_string(),
            from_blur,
        });

        let CommitTimeCellEditResult::Handled {
            active_field,
            draft_value,
            error,
            should_call_on_time_change,
            annotation_id: _,
            next_times,
        } = result
        else {
            return;
        };

        if let Some(draft) = self.segment_drafts.get_mut(segment_id) {
            draft.time_edit = TimeEditState {
                active_field,
                draft_value: draft_value.clone(),
                error: error.clone(),
                should_focus_and_select: false,
            };
            draft.set_time_value(field, draft_value);
        }

        if !should_call_on_time_change {
            if let Some(error) = error {
                self.status = error;
            }
            return;
        }

        let Some(next_times) = next_times else {
            return;
        };

        if next_times.start_seconds == segment.start_seconds
            && next_times.end_seconds == segment.end_seconds
        {
            if let Some(draft) = self.segment_drafts.get_mut(segment_id) {
                draft.replace_times(segment.start_seconds, segment.end_seconds);
            }
            self.status = format!("No timestamp changes to apply for `{segment_id}`.");
            return;
        }

        let applied = self.with_editable_workflow_mut("Time edit", |workflow| {
            workflow
                .move_segment_boundary(segment_id, next_times.start_seconds, next_times.end_seconds)
                .map_err(|error| error.to_string())?;
            workflow.finish_boundary_edit();
            Ok(format!(
                "Updated `{segment_id}` {} time to {}.",
                field.label(),
                format_seconds_for_edit(
                    field.value_from(next_times.start_seconds, next_times.end_seconds)
                )
            ))
        });

        if applied {
            if let Some(draft) = self.segment_drafts.get_mut(segment_id) {
                draft.replace_times(next_times.start_seconds, next_times.end_seconds);
                draft.time_edit = TimeEditState::default();
            }
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
        }
    }
}
