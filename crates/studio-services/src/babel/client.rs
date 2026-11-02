//! Typed async Babel operations.

use std::path::PathBuf;

use studio_editor::editor_model::feedback_assessment::{FeedbackDraftSaveRequest, FeedbackInput};
use studio_editor::project_lint_tag_config::babel_project_lint_tag_metadata;
use studio_editor::workflow::StudioWorkflow;

use super::BabelError;
use super::dashboard::{Dashboard, parse_dashboard};
use super::payload::{SaveAnnotationsRequest, SubmitReviewActionRequest};
use super::request::BabelRequest;
use super::response::{
    FeedbackDraftBootstrap, SaveOutcome, merge_review_action_annotations, parse_feedback_draft,
    parse_feedback_draft_save, parse_feedback_form_step_id, parse_feedback_inputs,
    parse_feedback_rubric_flags, parse_save, parse_simple_mutation, parse_submit,
    review_action_missing_annotations,
};
use super::transport::{BabelConfig, TransportError, payload_for};
use crate::audio_fetch;

/// Which task payload to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRequest {
    /// Claim the next chunk from a chunked review queue.
    ClaimNext { queue_id: String },
    /// Claim a stitched transcription by processed-transcription id.
    ClaimByProcessedTranscription {
        processed_transcription_id: String,
        queue_id: String,
    },
    /// Read-only view of a review action.
    ReviewAction { review_action_id: String },
    /// Read-only view of the latest review action of a processed transcription.
    LatestReviewAction {
        processed_transcription_id: String,
        queue_id: String,
    },
}

impl TaskRequest {
    /// Status shown after the task opened.
    pub fn success_status(&self) -> &'static str {
        match self {
            Self::ReviewAction { .. } => "Review action loaded.",
            Self::LatestReviewAction { .. } => "Processed transcription loaded.",
            Self::ClaimByProcessedTranscription { .. } => "Stitched transcription claimed.",
            Self::ClaimNext { .. } => "Queue claim loaded.",
        }
    }

    /// Prefix for `"<label> failed: <error>"`.
    pub fn failure_label(&self) -> &'static str {
        match self {
            Self::ReviewAction { .. } => "Review action load",
            Self::LatestReviewAction { .. } => "Processed transcription load",
            Self::ClaimByProcessedTranscription { .. } => "Stitched transcription claim",
            Self::ClaimNext { .. } => "Queue claim load",
        }
    }

    fn babel_request(&self) -> BabelRequest {
        match self.clone() {
            Self::ClaimNext { queue_id } => BabelRequest::ClaimNextFromQueue { queue_id },
            Self::ClaimByProcessedTranscription {
                processed_transcription_id,
                queue_id,
            } => BabelRequest::ClaimByProcessedTranscription {
                processed_transcription_id,
                queue_id,
            },
            Self::ReviewAction { review_action_id } => {
                BabelRequest::LoadReviewAction { review_action_id }
            }
            Self::LatestReviewAction {
                processed_transcription_id,
                queue_id,
            } => BabelRequest::LoadLatestReviewAction {
                processed_transcription_id,
                queue_id,
            },
        }
    }
}

/// An opened task: imported workflow plus where its audio lives.
#[derive(Debug)]
pub struct LoadedTask {
    pub workflow: StudioWorkflow,
    /// Base directory for resolving lane audio paths.
    pub audio_base_dir: PathBuf,
    /// Remote audio references downloaded for this task (if > 0 and no lane
    /// audio decodes, the old app treated the load as failed).
    pub remote_audio_count: usize,
}

/// Babel service client. Cheap to clone; every method runs on the services runtime.
#[derive(Debug, Clone)]
pub struct BabelClient {
    config: BabelConfig,
}

impl BabelClient {
    pub fn new(config: BabelConfig) -> Self {
        Self { config }
    }

    /// Client configured from the `STUDIO_BABEL_SERVICE_*` env.
    pub fn from_env() -> Self {
        Self::new(BabelConfig::from_env())
    }

    pub fn config(&self) -> &BabelConfig {
        &self.config
    }

    /// Whether saves/submits should go to Babel (command bridge, explicit live
    /// mode, or an active sub-account session). Otherwise saves stay local.
    pub fn remote_mutations_configured(&self) -> bool {
        self.config.command.is_some()
            || self.config.live_mode
            || (self.config.subaccount_relay && crate::subaccounts::session_is_active())
    }

    /// Raw normalized response for `request` (also used by the review/subaccount services).
    pub async fn raw(&self, request: &BabelRequest) -> Result<String, BabelError> {
        payload_for(&self.config, request)
            .await
            .map_err(|error| transport_error(String::new(), error))
    }

    async fn raw_prefixed(
        &self,
        request: &BabelRequest,
        prefix: &str,
    ) -> Result<String, BabelError> {
        payload_for(&self.config, request)
            .await
            .map_err(|error| transport_error(format!("{prefix}: "), error))
    }

    /// Worker id + active projects with queue ids and daily progress.
    pub async fn load_dashboard(&self) -> Result<Dashboard, BabelError> {
        let payload = self.raw(&BabelRequest::LoadWorkerProjects).await?;
        parse_dashboard(&payload).map_err(BabelError::new)
    }

    /// Raw task payload (Babel review-action JSON accepted by `StudioWorkflow::from_babel_json`).
    /// A payload without annotations gets them from `transcriptions.getAnnotationsByReviewActionId`
    /// (with that answer's `lintErrors`), as Babel's editor does.
    pub async fn fetch_task_payload(&self, request: &TaskRequest) -> Result<String, BabelError> {
        let payload = self.raw(&request.babel_request()).await?;
        let Some(review_action_id) = review_action_missing_annotations(&payload) else {
            return Ok(payload);
        };
        let annotations = self
            .raw_prefixed(
                &BabelRequest::LoadAnnotationsByReviewAction { review_action_id },
                "Review action annotations load failed",
            )
            .await?;
        merge_review_action_annotations(&payload, &annotations)
    }

    /// Fetches the task, stages remote audio locally, imports it and merges
    /// `route_metadata` plus the project lint/tag profile metadata.
    pub async fn open_task(
        &self,
        request: &TaskRequest,
        route_metadata: Vec<(String, String)>,
    ) -> Result<LoadedTask, BabelError> {
        let payload = self.fetch_task_payload(request).await?;
        let prepared = if audio_fetch::payload_needs_audio(&payload) {
            audio_fetch::prepare_payload_audio(payload, &self.config)
                .await
                .map_err(|error| BabelError::new(format!("{error:#}")))?
        } else {
            audio_fetch::PreparedPayload::local(payload)
        };
        let mut workflow = StudioWorkflow::from_babel_json(&prepared.source)
            .map_err(|error| BabelError::new(format!("{error:#}")))?;
        workflow.merge_project_metadata(route_metadata);
        let lint_tag_metadata = babel_project_lint_tag_metadata(workflow.project());
        workflow.merge_project_metadata(lint_tag_metadata);
        Ok(LoadedTask {
            workflow,
            audio_base_dir: prepared.audio_base_dir,
            remote_audio_count: prepared.remote_audio_count,
        })
    }

    /// Saves progress. Errors carry server lint issues when Babel returned them.
    pub async fn save_annotations(
        &self,
        request: SaveAnnotationsRequest,
    ) -> Result<SaveOutcome, BabelError> {
        parse_save(&self.raw(&BabelRequest::SaveAnnotations(request)).await?)
    }

    /// Submits the review action; incomplete feedback or any returned lint issue fails it.
    pub async fn submit_review_action(
        &self,
        request: SubmitReviewActionRequest,
    ) -> Result<(), BabelError> {
        parse_submit(&self.raw(&BabelRequest::SubmitReviewAction(request)).await?)
    }

    /// Releases the claim on `review_action_id` in `queue_id` (saved work stays with the task).
    pub async fn drop_claim(
        &self,
        queue_id: String,
        review_action_id: String,
    ) -> Result<(), BabelError> {
        let payload = self
            .raw(&BabelRequest::DropWorkerAction {
                queue_id,
                review_action_id,
            })
            .await?;
        parse_simple_mutation(&payload, "Failed to drop claim. Please try again.")
    }

    /// Skips the current chunk.
    pub async fn skip_chunk(&self, review_action_id: String) -> Result<(), BabelError> {
        let payload = self
            .raw(&BabelRequest::SkipReviewAction { review_action_id })
            .await?;
        parse_simple_mutation(&payload, "Please try again.")
    }

    /// Admin flag (`transcriptions.adminRateTtsTranscription`).
    pub async fn admin_flag(
        &self,
        review_action_id: String,
        rating: u8,
        rating_reason: Option<String>,
    ) -> Result<(), BabelError> {
        let payload = self
            .raw(&BabelRequest::AdminRateReviewAction {
                review_action_id,
                rating,
                rating_reason,
            })
            .await?;
        parse_simple_mutation(&payload, "Please try again.")
    }

    pub async fn feedback_rubric_flags(&self, project_id: &str) -> Result<Vec<String>, BabelError> {
        let request = BabelRequest::LoadFeedbackRubricFlags {
            project_id: project_id.to_string(),
        };
        parse_feedback_rubric_flags(
            &self
                .raw_prefixed(&request, "Feedback rubric flags load failed")
                .await?,
        )
    }

    /// Id of the feedback form's first step.
    pub async fn feedback_form_step_id(&self, project_id: &str) -> Result<String, BabelError> {
        let request = BabelRequest::LoadFeedbackForm {
            project_id: project_id.to_string(),
        };
        parse_feedback_form_step_id(
            &self
                .raw_prefixed(&request, "Feedback form load failed")
                .await?,
        )
    }

    pub async fn feedback_inputs(&self, step_id: &str) -> Result<Vec<FeedbackInput>, BabelError> {
        let request = BabelRequest::LoadFeedbackInputs {
            step_id: step_id.to_string(),
        };
        parse_feedback_inputs(
            &self
                .raw_prefixed(&request, "Feedback form inputs load failed")
                .await?,
        )
    }

    /// Gets or creates the reviewer's feedback draft.
    pub async fn feedback_draft(
        &self,
        form_step_id: &str,
        review_action_id: &str,
        worker_id: &str,
    ) -> Result<FeedbackDraftBootstrap, BabelError> {
        let request = BabelRequest::LoadFeedbackDraft {
            form_step_id: form_step_id.to_string(),
            review_action_id: review_action_id.to_string(),
            worker_id: worker_id.to_string(),
        };
        parse_feedback_draft(
            &self
                .raw_prefixed(&request, "Feedback draft bootstrap failed")
                .await?,
        )
    }

    /// Saves the feedback draft; returns `last_saved`.
    pub async fn save_feedback_draft(
        &self,
        request: &FeedbackDraftSaveRequest,
    ) -> Result<String, BabelError> {
        parse_feedback_draft_save(
            &self
                .raw(&BabelRequest::SaveFeedbackDraft(request.clone()))
                .await?,
        )
    }
}

fn transport_error(prefix: String, error: TransportError) -> BabelError {
    BabelError {
        message: prefix + &error.message,
        lint_issues: None,
        signed_out: error.signed_out,
    }
}
