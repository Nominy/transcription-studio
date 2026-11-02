//! Babel tRPC operations and their request envelopes
//! (`{"operation": ..., "payload": ...}` / `{"operation": ..., "batchInput": ...}`),
//! the exact JSON the command bridge receives on stdin and the relay forwards.

use serde_json::{Value, json};
use studio_editor::editor_model::feedback_assessment::FeedbackDraftSaveRequest;

use super::payload::{SaveAnnotationsRequest, SubmitReviewActionRequest};

/// Worker (for its id) and the worker's projects; everything the dashboard consumes.
const DASHBOARD_PROJECTS_OPERATIONS: &str = "worker.get,worker.getProjects";

/// One Babel service operation.
#[derive(Debug, Clone, PartialEq)]
pub enum BabelRequest {
    LoadWorkerProjects,
    SaveAnnotations(SaveAnnotationsRequest),
    SubmitReviewAction(SubmitReviewActionRequest),
    DropWorkerAction {
        queue_id: String,
        review_action_id: String,
    },
    SkipReviewAction {
        review_action_id: String,
    },
    AdminRateReviewAction {
        review_action_id: String,
        rating: u8,
        rating_reason: Option<String>,
    },
    ClaimNextFromQueue {
        queue_id: String,
    },
    ClaimByProcessedTranscription {
        processed_transcription_id: String,
        queue_id: String,
    },
    LoadReviewAction {
        review_action_id: String,
    },
    /// Saved annotations of a review action, for claims whose payload carries none.
    LoadAnnotationsByReviewAction {
        review_action_id: String,
    },
    LoadLatestReviewAction {
        processed_transcription_id: String,
        queue_id: String,
    },
    LoadFeedbackRubricFlags {
        project_id: String,
    },
    LoadFeedbackForm {
        project_id: String,
    },
    LoadFeedbackInputs {
        step_id: String,
    },
    LoadFeedbackDraft {
        form_step_id: String,
        review_action_id: String,
        worker_id: String,
    },
    SaveFeedbackDraft(FeedbackDraftSaveRequest),
}

/// How a raw response is normalized before parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponseShape {
    Dashboard,
    ReviewAction,
    Generic,
}

impl BabelRequest {
    /// Request kind understood by the sub-account relay.
    pub fn relay_kind(&self) -> &'static str {
        match self {
            Self::LoadWorkerProjects => "loadWorkerProjects",
            Self::SaveAnnotations(_) => "saveAnnotations",
            Self::SubmitReviewAction(_) => "submitReviewAction",
            Self::DropWorkerAction { .. } => "dropWorkerAction",
            Self::SkipReviewAction { .. } => "skipReviewAction",
            Self::AdminRateReviewAction { .. } => "adminRateReviewAction",
            Self::ClaimNextFromQueue { .. } => "claimNextFromQueue",
            Self::ClaimByProcessedTranscription { .. } => "claimByProcessedTranscription",
            Self::LoadReviewAction { .. } => "loadReviewAction",
            Self::LoadAnnotationsByReviewAction { .. } => "loadAnnotationsByReviewAction",
            Self::LoadLatestReviewAction { .. } => "loadLatestReviewAction",
            Self::LoadFeedbackRubricFlags { .. } => "loadFeedbackRubricFlags",
            Self::LoadFeedbackForm { .. } => "loadFeedbackForm",
            Self::LoadFeedbackInputs { .. } => "loadFeedbackInputs",
            Self::LoadFeedbackDraft { .. } => "loadFeedbackDraft",
            Self::SaveFeedbackDraft(_) => "saveFeedbackDraft",
        }
    }

    pub(super) fn response_shape(&self) -> ResponseShape {
        match self {
            Self::LoadWorkerProjects => ResponseShape::Dashboard,
            Self::LoadReviewAction { .. }
            | Self::LoadLatestReviewAction { .. }
            | Self::ClaimByProcessedTranscription { .. }
            | Self::ClaimNextFromQueue { .. } => ResponseShape::ReviewAction,
            _ => ResponseShape::Generic,
        }
    }

    /// The service envelope for this request.
    pub fn envelope(&self) -> Value {
        match self {
            Self::LoadWorkerProjects => json!({
                "operation": DASHBOARD_PROJECTS_OPERATIONS,
                "batchInput": dashboard_projects_batch_input(),
            }),
            Self::SaveAnnotations(request) => json!({
                "operation": "transcriptions.saveAnnotationsByReviewActionId",
                "payload": {
                    "reviewActionId": request.review_action_id,
                    "annotations": request.annotations,
                },
            }),
            Self::SubmitReviewAction(request) => json!({
                "operation": "transcriptions.submitTranscriptReviewAction",
                "payload": {
                    "reviewActionId": request.review_action_id,
                    "annotations": request.annotations,
                    "isStitchedQueue": request.is_stitched_queue,
                },
            }),
            Self::DropWorkerAction {
                queue_id,
                review_action_id,
            } => json!({
                "operation": "transcriptions.dropWorkerAction",
                "payload": { "queueId": queue_id, "reviewActionId": review_action_id },
            }),
            Self::SkipReviewAction { review_action_id } => json!({
                "operation": "transcriptions.skipTranscriptReviewAction",
                "payload": { "reviewActionId": review_action_id },
            }),
            Self::AdminRateReviewAction {
                review_action_id,
                rating,
                rating_reason,
            } => json!({
                "operation": "transcriptions.adminRateTtsTranscription",
                "payload": {
                    "reviewActionId": review_action_id,
                    "rating": rating,
                    "ratingReason": rating_reason,
                },
            }),
            Self::ClaimNextFromQueue { queue_id } => json!({
                "operation": "transcriptions.claimNextReviewActionFromReviewQueue",
                "payload": { "queueId": queue_id },
            }),
            Self::ClaimByProcessedTranscription {
                processed_transcription_id,
                queue_id,
            } => json!({
                "operation": "transcriptions.claimReviewActionByProcessedTranscriptionId",
                "payload": {
                    "processedTranscriptionId": processed_transcription_id,
                    "queueId": queue_id,
                },
            }),
            Self::LoadReviewAction { review_action_id } => json!({
                "operation": "transcriptions.getReviewActionDataById",
                "payload": { "reviewActionId": review_action_id },
            }),
            Self::LoadAnnotationsByReviewAction { review_action_id } => json!({
                "operation": "transcriptions.getAnnotationsByReviewActionId",
                "payload": { "reviewActionId": review_action_id },
            }),
            Self::LoadLatestReviewAction {
                processed_transcription_id,
                queue_id,
            } => json!({
                "operation": "transcriptions.getLatestReviewActionByProcessedTranscriptionId",
                "payload": {
                    "processedTranscriptionId": processed_transcription_id,
                    "queueId": queue_id,
                },
            }),
            Self::LoadFeedbackRubricFlags { project_id } => json!({
                "operation": "transcriptionFeedbackForm.getRubricFlagNames",
                "payload": { "projectId": project_id },
            }),
            Self::LoadFeedbackForm { project_id } => json!({
                "operation": "transcriptionFeedbackForm.getForm",
                "payload": { "projectId": project_id },
            }),
            Self::LoadFeedbackInputs { step_id } => json!({
                "operation": "forms.getFormInputsByStepId",
                "payload": { "stepId": step_id },
            }),
            Self::LoadFeedbackDraft {
                form_step_id,
                review_action_id,
                worker_id,
            } => json!({
                "operation": "transcriptionFeedbackForm.getOrCreateDraft",
                "payload": {
                    "formStepId": form_step_id,
                    "reviewActionId": review_action_id,
                    "workerId": worker_id,
                },
            }),
            Self::SaveFeedbackDraft(request) => json!({
                "operation": "transcriptionFeedbackForm.saveDraft",
                "payload": {
                    "formStepResponseId": request.form_step_response_id,
                    "inputResponses": request.input_responses.iter().map(|response| json!({
                        "formInputId": response.form_input_id,
                        "value": response.value,
                    })).collect::<Vec<_>>(),
                },
            }),
        }
    }
}

/// The dashboard batch input: both queries take no input (`undefined` via tRPC superjson meta).
fn dashboard_projects_batch_input() -> Value {
    let undefined = json!({
        "json": null,
        "meta": { "values": ["undefined"] },
    });
    json!({ "0": undefined, "1": undefined })
}

/// GET for read-only queries (every comma-separated operation is a query), else POST.
pub(super) fn is_query_operation(operation: &str) -> bool {
    operation.split(',').all(|name| {
        matches!(
            name.trim(),
            "worker.get"
                | "worker.getProjects"
                | "transcriptions.getProjectQueues"
                | "transcriptionFeedbackForm.getRubricFlagNames"
                | "transcriptionFeedbackForm.getForm"
                | "forms.getFormInputsByStepId"
                | "transcriptions.getReviewActionDataById"
                | "transcriptions.getLatestReviewActionByProcessedTranscriptionId"
                | "transcriptions.getAnnotationsByReviewActionId"
                | "transcriptions.getPresignedChunkUrls"
        )
    })
}
