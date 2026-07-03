use serde_json::Value;
use studio_core::Project;

use studio_app::{
    AdminFlagPayload, FeedbackDraftSaveRequest, StudioWorkflow, TaskLookupEffect, TaskLookupState,
};

pub(super) const META_REVIEW_ACTION_ID: &str = "studio.babel.reviewActionId";
pub(super) const META_ACTIVE_QUEUE_ID: &str = "studio.babel.activeQueueId";
pub(super) const META_QUEUE_ID: &str = "studio.babel.queueId";
pub(super) const META_CHUNKED_QUEUE_ID: &str = "studio.babel.chunkedQueueId";
pub(super) const META_STITCHED_QUEUE_ID: &str = "studio.babel.stitchedQueueId";
pub(super) const META_IS_STITCHED_QUEUE: &str = "studio.babel.isStitchedQueue";
const META_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";

#[derive(Debug, Clone, PartialEq)]
pub(super) enum BabelServiceRequest {
    LoadWorkerProjects,
    SaveAnnotations(SaveAnnotationsRequest),
    SubmitReviewAction(SubmitReviewActionRequest),
    DropWorkerAction(DropWorkerActionRequest),
    SkipReviewAction(SkipReviewActionRequest),
    AdminRateReviewAction(AdminRateReviewActionRequest),
    ClaimNextFromQueue(ClaimNextFromQueueRequest),
    ClaimByProcessedTranscription(ClaimByProcessedTranscriptionRequest),
    LoadReviewAction(LoadReviewActionRequest),
    LoadLatestReviewAction(LoadLatestReviewActionRequest),
    SaveFeedbackDraft(FeedbackDraftSaveRequest),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SaveAnnotationsRequest {
    pub(super) review_action_id: String,
    pub(super) annotations: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SubmitReviewActionRequest {
    pub(super) review_action_id: String,
    pub(super) annotations: Vec<Value>,
    pub(super) is_stitched_queue: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DropWorkerActionRequest {
    pub(super) queue_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkipReviewActionRequest {
    pub(super) review_action_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdminRateReviewActionRequest {
    pub(super) review_action_id: String,
    pub(super) rating: u8,
    pub(super) rating_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClaimNextFromQueueRequest {
    pub(super) queue_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClaimByProcessedTranscriptionRequest {
    pub(super) processed_transcription_id: String,
    pub(super) queue_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoadReviewActionRequest {
    pub(super) review_action_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoadLatestReviewActionRequest {
    pub(super) processed_transcription_id: String,
    pub(super) queue_id: String,
}

pub(super) fn build_save_annotations_request(
    workflow: &StudioWorkflow,
) -> Result<SaveAnnotationsRequest, String> {
    let project = workflow.project();
    let review_action_id = review_action_id(project);
    let exported = workflow
        .export_babel_json()
        .map_err(|error| format!("could not build Babel save payload: {error}"))?;
    let value: Value = serde_json::from_str(&exported)
        .map_err(|error| format!("could not decode Babel save payload: {error}"))?;
    let annotations = find_array_by_key(&value, "annotations")
        .cloned()
        .unwrap_or_default();

    if review_action_id.is_empty() {
        return Err("missing review action id".to_string());
    }

    Ok(SaveAnnotationsRequest {
        review_action_id,
        annotations,
    })
}

pub(super) fn build_submit_review_action_request(
    workflow: &StudioWorkflow,
) -> Result<SubmitReviewActionRequest, String> {
    let project = workflow.project();
    let review_action_id = review_action_id(project);
    let exported = workflow
        .export_babel_json()
        .map_err(|error| format!("could not build Babel submit payload: {error}"))?;
    let value: Value = serde_json::from_str(&exported)
        .map_err(|error| format!("could not decode Babel submit payload: {error}"))?;
    let annotations = find_array_by_key(&value, "annotations")
        .cloned()
        .unwrap_or_default();

    Ok(SubmitReviewActionRequest {
        review_action_id,
        annotations,
        is_stitched_queue: metadata_bool(project, META_IS_STITCHED_QUEUE),
    })
}

pub(super) fn build_drop_worker_action_request(project: &Project) -> DropWorkerActionRequest {
    DropWorkerActionRequest {
        queue_id: active_queue_id(project),
    }
}

pub(super) fn build_skip_review_action_request(project: &Project) -> SkipReviewActionRequest {
    SkipReviewActionRequest {
        review_action_id: review_action_id(project),
    }
}

pub(super) fn build_admin_rate_review_action_request(
    project: &Project,
    payload: AdminFlagPayload,
) -> AdminRateReviewActionRequest {
    AdminRateReviewActionRequest {
        review_action_id: review_action_id(project),
        rating: payload.rating,
        rating_reason: payload.rating_reason,
    }
}

pub(super) fn task_lookup_request(
    project: Option<&Project>,
    state: Option<&TaskLookupState>,
    effect: &TaskLookupEffect,
) -> Option<BabelServiceRequest> {
    match effect {
        TaskLookupEffect::SetProcessedTranscriptionId { .. }
        | TaskLookupEffect::SetReviewActionId { .. } => None,
        TaskLookupEffect::ClaimById => {
            let project = project?;
            let state = state?;
            Some(BabelServiceRequest::ClaimByProcessedTranscription(
                ClaimByProcessedTranscriptionRequest {
                    processed_transcription_id: state.processed_transcription_id.clone(),
                    queue_id: stitched_queue_id(project),
                },
            ))
        }
        TaskLookupEffect::ViewReadOnlyByReviewActionId => {
            let state = state?;
            Some(BabelServiceRequest::LoadReviewAction(
                LoadReviewActionRequest {
                    review_action_id: state.review_action_id.clone(),
                },
            ))
        }
        TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId => {
            let project = project?;
            let state = state?;
            Some(BabelServiceRequest::LoadLatestReviewAction(
                LoadLatestReviewActionRequest {
                    processed_transcription_id: state.processed_transcription_id.clone(),
                    queue_id: stitched_queue_id(project),
                },
            ))
        }
    }
}

fn active_queue_id(project: &Project) -> String {
    first_nonempty_string([
        trimmed_project_metadata(project, META_ACTIVE_QUEUE_ID),
        trimmed_project_metadata(project, META_QUEUE_ID),
        trimmed_project_metadata(project, META_CHUNKED_QUEUE_ID),
    ])
    .unwrap_or_default()
}

fn stitched_queue_id(project: &Project) -> String {
    first_nonempty_string([
        trimmed_project_metadata(project, META_STITCHED_QUEUE_ID),
        trimmed_project_metadata(project, META_ACTIVE_QUEUE_ID),
        trimmed_project_metadata(project, META_QUEUE_ID),
    ])
    .unwrap_or_default()
}

fn review_action_id(project: &Project) -> String {
    first_nonempty_string([
        trimmed_project_metadata(project, META_REVIEW_ACTION_ID),
        review_action_id_from_segments(project),
        review_action_id_from_envelope(project),
    ])
    .unwrap_or_default()
}

fn review_action_id_from_segments(project: &Project) -> Option<String> {
    project.segments.iter().find_map(|segment| {
        segment
            .metadata
            .get("reviewActionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn review_action_id_from_envelope(project: &Project) -> Option<String> {
    let envelope = project.metadata.get(META_REVIEW_ACTION_ENVELOPE)?;
    let value: Value = serde_json::from_str(envelope).ok()?;
    find_string_by_key(&value, "actionId")
}

fn trimmed_project_metadata(project: &Project, key: &str) -> Option<String> {
    project
        .metadata
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn metadata_bool(project: &Project, key: &str) -> bool {
    project.metadata.get(key).is_some_and(|value| {
        matches!(
            value.trim(),
            "true" | "1" | "yes" | "on" | "True" | "TRUE" | "Yes" | "YES" | "On" | "ON"
        )
    })
}

fn first_nonempty_string(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values.into_iter().flatten().find(|value| !value.is_empty())
}

fn find_string_by_key(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(found) = map
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(found.to_string());
            }
            map.values()
                .find_map(|child| find_string_by_key(child, key))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_by_key(child, key)),
        _ => None,
    }
}

fn find_array_by_key<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key).and_then(Value::as_array) {
                return Some(found);
            }
            map.values().find_map(|child| find_array_by_key(child, key))
        }
        Value::Array(items) => items.iter().find_map(|child| find_array_by_key(child, key)),
        _ => None,
    }
}
