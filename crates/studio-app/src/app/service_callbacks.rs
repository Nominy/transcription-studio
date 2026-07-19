use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use studio_core::{Project, Segment};

use studio_app::{
    AdminFlagPayload, FeedbackDraftSaveRequest, FeedbackInput, FeedbackInputResponse,
    StudioWorkflow, TaskLookupEffect, TaskLookupState,
};

pub(super) const META_REVIEW_ACTION_ID: &str = "studio.babel.reviewActionId";
pub(super) const META_ACTIVE_QUEUE_ID: &str = "studio.babel.activeQueueId";
pub(super) const META_QUEUE_ID: &str = "studio.babel.queueId";
pub(super) const META_CHUNKED_QUEUE_ID: &str = "studio.babel.chunkedQueueId";
pub(super) const META_STITCHED_QUEUE_ID: &str = "studio.babel.stitchedQueueId";
pub(super) const META_IS_STITCHED_QUEUE: &str = "studio.babel.isStitchedQueue";
pub(super) const META_PROJECT_ID: &str = "studio.babel.projectId";
const META_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";
const META_EXTRA_PACKAGE: &str = "babel.extra.package";

#[derive(Debug, Clone, PartialEq)]
pub(super) enum BabelServiceRequest {
    LoadWorkerProjects,
    SaveAnnotations(SaveAnnotationsRequest),
    LintAnnotations(LintAnnotationsRequest),
    SubmitReviewAction(SubmitReviewActionRequest),
    DropWorkerAction(DropWorkerActionRequest),
    SkipReviewAction(SkipReviewActionRequest),
    AdminRateReviewAction(AdminRateReviewActionRequest),
    ClaimNextFromQueue(ClaimNextFromQueueRequest),
    ClaimByProcessedTranscription(ClaimByProcessedTranscriptionRequest),
    LoadReviewAction(LoadReviewActionRequest),
    LoadLatestReviewAction(LoadLatestReviewActionRequest),
    LoadFeedbackRubricFlags(LoadFeedbackRubricFlagsRequest),
    LoadFeedbackForm(LoadFeedbackFormRequest),
    LoadFeedbackInputs(LoadFeedbackInputsRequest),
    LoadFeedbackDraft(LoadFeedbackDraftRequest),
    SaveFeedbackDraft(FeedbackDraftSaveServiceRequest),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SaveAnnotationsRequest {
    pub(super) review_action_id: String,
    pub(super) annotations: Vec<Value>,
    pub(super) token: SaveRequestToken,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SaveRequestToken {
    pub(super) workspace_generation: u64,
    pub(super) revision: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SubmitRequestToken {
    pub(super) workspace_generation: u64,
    pub(super) revision: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct FeedbackBootstrapToken {
    pub(super) workspace_generation: u64,
    pub(super) attempt: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct FeedbackDraftSaveToken {
    pub(super) workspace_generation: u64,
    pub(super) revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FeedbackDraftSaveServiceRequest {
    pub(super) payload: FeedbackDraftSaveRequest,
    pub(super) token: FeedbackDraftSaveToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoadFeedbackRubricFlagsRequest {
    pub(super) project_id: String,
    pub(super) token: FeedbackBootstrapToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoadFeedbackFormRequest {
    pub(super) project_id: String,
    pub(super) token: FeedbackBootstrapToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoadFeedbackInputsRequest {
    pub(super) step_id: String,
    pub(super) token: FeedbackBootstrapToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoadFeedbackDraftRequest {
    pub(super) form_step_id: String,
    pub(super) review_action_id: String,
    pub(super) worker_id: String,
    pub(super) token: FeedbackBootstrapToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FeedbackDraftBootstrap {
    pub(super) form_step_response_id: String,
    pub(super) input_responses: Vec<FeedbackInputResponse>,
    pub(super) updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FeedbackBootstrapInputs {
    pub(super) inputs: Vec<FeedbackInput>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LintAnnotationsRequest {
    pub(super) annotations: Vec<Value>,
    pub(super) project_id: String,
    pub(super) consensus_data: Value,
    pub(super) token: SubmitRequestToken,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SubmitReviewActionRequest {
    pub(super) review_action_id: String,
    pub(super) annotations: Vec<Value>,
    pub(super) is_stitched_queue: bool,
    pub(super) token: SubmitRequestToken,
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

pub(super) fn build_save_annotations_request_with_asserted_warnings(
    workflow: &StudioWorkflow,
    asserted_warning_keys: &BTreeSet<String>,
) -> Result<SaveAnnotationsRequest, String> {
    let project = workflow.project();
    let review_action_id = review_action_id(project);
    let annotations = exported_annotations(workflow, "save", Some(asserted_warning_keys))?;
    if review_action_id.is_empty() {
        return Err("missing review action id".to_string());
    }

    Ok(SaveAnnotationsRequest {
        review_action_id,
        annotations,
        token: SaveRequestToken::default(),
    })
}

#[cfg(test)]
pub(super) fn build_save_annotations_request(
    workflow: &StudioWorkflow,
) -> Result<SaveAnnotationsRequest, String> {
    let project = workflow.project();
    let review_action_id = review_action_id(project);
    let annotations = exported_annotations(workflow, "save", None)?;
    if review_action_id.is_empty() {
        return Err("missing review action id".to_string());
    }

    Ok(SaveAnnotationsRequest {
        review_action_id,
        annotations,
        token: SaveRequestToken::default(),
    })
}

pub(super) fn build_submit_review_action_request_with_asserted_warnings(
    workflow: &StudioWorkflow,
    asserted_warning_keys: &BTreeSet<String>,
    token: SubmitRequestToken,
) -> Result<SubmitReviewActionRequest, String> {
    let project = workflow.project();
    let review_action_id = review_action_id(project);
    let annotations = exported_annotations(workflow, "submit", Some(asserted_warning_keys))?;

    if review_action_id.is_empty() {
        return Err("missing review action id".to_string());
    }
    Ok(SubmitReviewActionRequest {
        review_action_id,
        annotations,
        is_stitched_queue: metadata_bool(project, META_IS_STITCHED_QUEUE),
        token,
    })
}

pub(super) fn build_lint_annotations_request(
    workflow: &StudioWorkflow,
    asserted_warning_keys: &BTreeSet<String>,
    token: SubmitRequestToken,
) -> Result<LintAnnotationsRequest, String> {
    let project = workflow.project();
    let project_id = first_nonempty_string([
        trimmed_project_metadata(project, META_PROJECT_ID),
        Some(project.id.trim().to_string()).filter(|id| !id.is_empty()),
    ])
    .unwrap_or_default();
    if project_id.is_empty() {
        return Err("missing Babel project id".to_string());
    }

    Ok(LintAnnotationsRequest {
        annotations: exported_annotations(workflow, "lint", Some(asserted_warning_keys))?,
        project_id,
        consensus_data: Value::Null,
        token,
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

pub(super) fn review_action_id(project: &Project) -> String {
    first_nonempty_string([
        trimmed_project_metadata(project, META_REVIEW_ACTION_ID),
        review_action_id_from_segments(project),
        review_action_id_from_package(project),
        review_action_id_from_envelope(project),
    ])
    .unwrap_or_default()
}

pub(super) fn feedback_project_id(project: &Project) -> String {
    first_nonempty_string([
        trimmed_project_metadata(project, META_PROJECT_ID),
        Some(project.id.trim().to_string()).filter(|value| !value.is_empty()),
    ])
    .unwrap_or_default()
}

fn review_action_id_from_segments(project: &Project) -> Option<String> {
    project.segments.iter().find_map(|segment| {
        segment
            .metadata
            .get("reviewActionId")
            .and_then(metadata_string)
            .or_else(|| {
                segment
                    .metadata
                    .get("review_action_id")
                    .and_then(metadata_string)
            })
    })
}

fn review_action_id_from_package(project: &Project) -> Option<String> {
    let package = project.metadata.get(META_EXTRA_PACKAGE)?;
    let package = serde_json::from_str::<Value>(package).ok()?;
    string_from_map(
        package.as_object()?,
        &[
            "reviewActionId",
            "review_action_id",
            "actionId",
            "action_id",
        ],
    )
}

fn review_action_id_from_envelope(project: &Project) -> Option<String> {
    let envelope = project.metadata.get(META_REVIEW_ACTION_ENVELOPE)?;
    let value: Value = serde_json::from_str(envelope).ok()?;
    find_string_by_key(&value, "actionId").or_else(|| find_string_by_key(&value, "reviewActionId"))
}

fn metadata_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        }
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn string_from_map(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(metadata_string))
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

fn exported_annotations(
    workflow: &StudioWorkflow,
    operation: &str,
    asserted_warning_keys: Option<&BTreeSet<String>>,
) -> Result<Vec<Value>, String> {
    let exported = workflow
        .export_babel_json()
        .map_err(|error| format!("could not build Babel {operation} payload: {error}"))?;
    let value: Value = serde_json::from_str(&exported)
        .map_err(|error| format!("could not decode Babel {operation} payload: {error}"))?;
    let mut annotations = annotations_from_babel_export(&value)?;
    if let Some(asserted_warning_keys) = asserted_warning_keys {
        merge_asserted_warnings(&mut annotations, workflow.project(), asserted_warning_keys);
    }
    Ok(annotations)
}

fn merge_asserted_warnings(
    annotations: &mut [Value],
    project: &Project,
    asserted_warning_keys: &BTreeSet<String>,
) {
    let warnings_by_annotation = asserted_warnings_by_annotation(project, asserted_warning_keys);
    for annotation in annotations {
        let Some(annotation_object) = annotation.as_object_mut() else {
            continue;
        };
        let Some(annotation_id) = annotation_object
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Some(asserted_warnings) = warnings_by_annotation.get(&annotation_id) else {
            continue;
        };
        let metadata = annotation_object
            .entry("metadata")
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(metadata) = metadata.as_object_mut() else {
            continue;
        };
        metadata.insert(
            "assertedWarnings".to_string(),
            Value::Array(
                asserted_warnings
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
}

fn asserted_warnings_by_annotation(
    project: &Project,
    asserted_warning_keys: &BTreeSet<String>,
) -> BTreeMap<String, Vec<String>> {
    let mut warnings = BTreeMap::<String, Vec<String>>::new();
    for segment in &project.segments {
        let prefix = format!("{}::", segment.id);
        let reasons = asserted_warning_keys
            .iter()
            .filter_map(|key| key.strip_prefix(&prefix))
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        for annotation_id in segment_annotation_ids(segment) {
            let entry = warnings.entry(annotation_id).or_default();
            for reason in &reasons {
                if !entry.contains(reason) {
                    entry.push(reason.clone());
                }
            }
        }
    }
    warnings
}

fn segment_annotation_ids(segment: &Segment) -> impl Iterator<Item = String> + '_ {
    std::iter::once(segment.id.clone()).chain(
        segment
            .source_annotation_id
            .iter()
            .filter(|source_id| *source_id != &segment.id)
            .cloned(),
    )
}

fn annotations_from_babel_export(value: &Value) -> Result<Vec<Value>, String> {
    find_array_by_key(value, "annotations")
        .cloned()
        .ok_or_else(|| "could not find annotations array in Babel export".to_string())
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
