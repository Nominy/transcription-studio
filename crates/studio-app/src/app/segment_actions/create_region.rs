use super::super::StudioState;
use super::super::browser_input::OutsideInteractionContext;
use super::super::create_region::{
    CreateRegionTimeField, create_region_runtime_trace_line, create_region_trace_enabled,
    intensity_value_from_label, should_prevent_create_region_interact_outside,
};

impl StudioState {
    pub(in crate::app) fn change_create_region_track(&mut self, lane_id: String) {
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.selected_lane_id = lane_id;
        }
    }

    pub(in crate::app) fn change_create_region_start(&mut self, value: String) {
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.set_start_draft(value);
        }
    }

    pub(in crate::app) fn change_create_region_end(&mut self, value: String) {
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.set_end_draft(value);
        }
    }

    pub(in crate::app) fn change_create_region_text(&mut self, value: String) {
        let created_segment_id = self
            .create_region_dialog
            .as_ref()
            .and_then(|dialog| dialog.created_segment_id.clone());
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.active_time_field = None;
            dialog.text = value.clone();
        }

        if let Some(segment_id) = created_segment_id
            && let Some(draft) = self.segment_drafts.get_mut(&segment_id)
        {
            draft.replace_text_from_user_edit(value);
            self.invalidate_transcript_layout_cache();
        }
    }

    pub(in crate::app) fn change_create_region_primary_category(&mut self, value: String) {
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.active_time_field = None;
            dialog.primary_selection = value;
            dialog.secondary_selection.clear();
            dialog.custom_description.clear();
        }
    }

    pub(in crate::app) fn change_create_region_secondary_category(&mut self, value: String) {
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.active_time_field = None;
            let is_other = value == "other";
            dialog.secondary_selection = value;
            if is_other {
                dialog.custom_description.clear();
            }
        }
    }

    pub(in crate::app) fn change_create_region_custom_description(&mut self, value: String) {
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.active_time_field = None;
            dialog.custom_description = value;
        }
    }

    pub(in crate::app) fn change_create_region_intensity(&mut self, value: String) {
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.active_time_field = None;
            dialog.intensity = intensity_value_from_label(&value);
        }
    }

    pub(in crate::app) fn change_create_region_select_open(&mut self, open: bool) {
        if let Some(dialog) = &mut self.create_region_dialog {
            dialog.select_open = open;
        }
    }

    pub(in crate::app) fn interact_outside_create_region(
        &mut self,
        context: OutsideInteractionContext,
    ) {
        let should_cancel = self.create_region_dialog.as_ref().is_some_and(|dialog| {
            !dialog.select_open && !should_prevent_create_region_interact_outside(context)
        });
        if should_cancel {
            self.cancel_create_region();
        }
    }

    pub(in crate::app) fn commit_create_region_start(&mut self) {
        self.commit_create_region_time(CreateRegionTimeField::Start);
    }

    pub(in crate::app) fn commit_create_region_end(&mut self) {
        self.commit_create_region_time(CreateRegionTimeField::End);
    }

    pub(in crate::app) fn reset_active_create_region_time_field(&mut self) {
        let Some(dialog) = &mut self.create_region_dialog else {
            return;
        };

        if dialog.reset_active_time_field_draft() {
            self.status = "Region time edit canceled.".to_string();
        }
    }

    pub(in crate::app) fn cancel_create_region(&mut self) {
        if create_region_trace_enabled()
            && let Some(dialog) = &self.create_region_dialog
        {
            eprintln!("{}", create_region_runtime_trace_line("cancel", dialog));
        }
        self.create_region_dialog = None;
    }

    pub(in crate::app) fn submit_create_region(&mut self) {
        if self.block_read_only_edit("Create") {
            return;
        }

        let Some(project) = self.project().cloned() else {
            self.status = "Create unavailable: no project loaded.".to_string();
            return;
        };
        let Some(mut dialog) = self.create_region_dialog.take() else {
            self.status = "Create unavailable: no region draft is open.".to_string();
            return;
        };

        if dialog.mode == super::super::create_region::CreateRegionMode::Transcription
            && let Some(segment_id) = dialog.created_segment_id.clone()
        {
            if project.segment(&segment_id).is_none() {
                self.status = format!("Create unavailable: segment `{segment_id}` was not found.");
                return;
            }

            let text = dialog.text.clone();
            let text_changed = project
                .segment(&segment_id)
                .is_some_and(|segment| segment.text != text);
            if text_changed
                && !self.with_editable_workflow_mut("Create", |workflow| {
                    workflow
                        .apply_text_edit(&segment_id, text)
                        .map_err(|error| error.to_string())?;
                    Ok(format!("Saved text for `{segment_id}`."))
                })
            {
                self.create_region_dialog = Some(dialog);
                return;
            }

            if create_region_trace_enabled() {
                eprintln!("{}", create_region_runtime_trace_line("save", &dialog));
            }
            self.mark_dirty();
            self.refresh_segment_draft(&segment_id);
            self.reconcile_editor_with_project_preserving_viewport();
            self.select_segment_without_seek(&segment_id);
            return;
        }

        let Some(payload) = dialog.validate_for_submit(&project) else {
            self.status = dialog
                .start_error
                .clone()
                .or(dialog.end_error.clone())
                .unwrap_or_else(|| "Create unavailable: select a valid speaker.".to_string());
            self.create_region_dialog = Some(dialog);
            return;
        };

        let segment_start = payload.start_seconds;
        let segment_end = payload.end_seconds;
        let mut created_segment_id = None;
        if self.with_editable_workflow_mut("Create", |workflow| {
            let new_segment_id = workflow
                .create_segment_in_lane_with_text_and_metadata(
                    &payload.lane_id,
                    payload.start_seconds,
                    payload.end_seconds,
                    payload.text,
                    payload.metadata,
                )
                .map_err(|error| error.to_string())?;
            created_segment_id = Some(new_segment_id.clone());
            Ok(format!(
                "Created `{new_segment_id}` at {segment_start:.3}s-{segment_end:.3}s."
            ))
        }) {
            if create_region_trace_enabled() {
                eprintln!("{}", create_region_runtime_trace_line("submit", &dialog));
            }
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
            if let Some(created_segment_id) = created_segment_id {
                self.select_segment_without_seek(&created_segment_id);
            }
        } else {
            self.create_region_dialog = Some(dialog);
        }
    }

    fn commit_create_region_time(&mut self, field: CreateRegionTimeField) {
        let Some(dialog) = &mut self.create_region_dialog else {
            return;
        };

        if dialog.commit_time_field(field) {
            self.status = "Region time updated.".to_string();
        } else {
            self.status = dialog
                .start_error
                .clone()
                .or(dialog.end_error.clone())
                .unwrap_or_else(|| "Invalid region time.".to_string());
        }
    }
}
