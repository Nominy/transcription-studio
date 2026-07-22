use studio_app::{
    TaskLookupEffect, TaskLookupEvent, TaskLookupInitialInput, TaskLookupState,
    create_initial_task_lookup_state, reduce_task_lookup_event,
};
use studio_core::Project;

use super::StudioState;
use super::service_callbacks::{BabelPayloadRequestToken, task_lookup_request};

const META_TASK_LOOKUP_OPEN: &str = "studio.babel.taskLookup.open";
const META_TASK_LOOKUP_PROCESSED_ID: &str = "studio.babel.taskLookup.processedTranscriptionId";
const META_TASK_LOOKUP_REVIEW_ACTION_ID: &str = "studio.babel.taskLookup.reviewActionId";
const META_TASK_LOOKUP_SHOW_STITCHED: &str = "studio.babel.taskLookup.showStitchedOption";

impl StudioState {
    pub(super) fn task_lookup_from_project(project: &Project) -> Option<TaskLookupState> {
        if !metadata_bool(project, META_TASK_LOOKUP_OPEN) {
            return None;
        }

        Some(create_initial_task_lookup_state(TaskLookupInitialInput {
            processed_transcription_id: project
                .metadata
                .get(META_TASK_LOOKUP_PROCESSED_ID)
                .cloned()
                .unwrap_or_default(),
            review_action_id: project
                .metadata
                .get(META_TASK_LOOKUP_REVIEW_ACTION_ID)
                .cloned()
                .unwrap_or_default(),
            show_stitched_option: Some(metadata_bool(project, META_TASK_LOOKUP_SHOW_STITCHED)),
        }))
    }

    pub(super) fn apply_task_lookup_event(&mut self, event: TaskLookupEvent) {
        let Some(current) = self.task_lookup.take() else {
            return;
        };
        let update = reduce_task_lookup_event(current, event);
        self.task_lookup = Some(update.state);
        for effect in update.effects {
            self.apply_task_lookup_effect(effect);
        }
    }

    fn apply_task_lookup_effect(&mut self, effect: TaskLookupEffect) {
        self.status = match &effect {
            TaskLookupEffect::SetProcessedTranscriptionId { value } => {
                format!("Processed transcription ID set: {value}")
            }
            TaskLookupEffect::SetReviewActionId { value } => {
                format!("Review action ID set: {value}")
            }
            TaskLookupEffect::ClaimById => "Task lookup claim requested.".to_string(),
            TaskLookupEffect::ViewReadOnlyByReviewActionId => {
                "Task lookup read-only review action requested.".to_string()
            }
            TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId => {
                "Task lookup read-only processed transcription requested.".to_string()
            }
        };
        let request = match effect {
            TaskLookupEffect::SetProcessedTranscriptionId { .. }
            | TaskLookupEffect::SetReviewActionId { .. } => None,
            _ => {
                let token: BabelPayloadRequestToken = self.issue_babel_payload_request_token();
                task_lookup_request(self.project(), self.task_lookup.as_ref(), &effect, token)
            }
        };
        if let Some(request) = request {
            self.babel_service_requests.push(request);
        }
        self.task_lookup_last_effect = Some(effect);
    }
}

fn metadata_bool(project: &Project, key: &str) -> bool {
    project.metadata.get(key).is_some_and(|value| {
        matches!(
            value.as_str(),
            "true" | "1" | "yes" | "on" | "True" | "TRUE" | "Yes" | "YES" | "On" | "ON"
        )
    })
}
