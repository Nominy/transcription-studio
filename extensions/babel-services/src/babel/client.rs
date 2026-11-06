//! Typed async Babel operations.

use std::path::PathBuf;

use exaudio_babel_domain::editor_model::feedback_assessment::{
    FeedbackDraftSaveRequest, FeedbackInput,
};
use exaudio_babel_domain::i18n::{localized, with_detail};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use exaudio_babel_lint::project_lint_tag_config::babel_project_lint_tag_metadata;
use studio_editor::workflow::StudioWorkflow;

use super::BabelError;
use super::dashboard::{Dashboard, parse_dashboard};
use super::error_task::{
    ErrorLabelSubmission, ErrorTask, ErrorTaskError, StyleGuide, parse_error_task,
    parse_style_guide_document, payload_is_error_task, select_style_guide_document,
};
use super::payload::{SaveAnnotationsRequest, SubmitReviewActionRequest};
use super::request::BabelRequest;
use super::response::parse_error_label_submit;
use super::response::parse_mutation_value;
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
    /// Claim an error-category QA task by its transcription-error diff id.
    ClaimErrorQaTask {
        transcription_error_diff_id: String,
        queue_id: String,
    },
}

impl TaskRequest {
    /// Status shown after the task opened.
    pub fn success_status(&self) -> &'static str {
        match self {
            Self::ReviewAction { .. } | Self::LatestReviewAction { .. } => {
                localized("Task loaded", "Задача загружена")
            }
            Self::ClaimByProcessedTranscription { .. }
            | Self::ClaimNext { .. }
            | Self::ClaimErrorQaTask { .. } => localized("Task taken", "Задача взята"),
        }
    }

    /// Names the action in the UI's `{label} didn't work`.
    pub fn failure_label(&self) -> &'static str {
        match self {
            Self::ReviewAction { .. } | Self::LatestReviewAction { .. } => {
                localized("Loading the task", "Загрузка задачи")
            }
            Self::ClaimByProcessedTranscription { .. }
            | Self::ClaimNext { .. }
            | Self::ClaimErrorQaTask { .. } => localized("Taking the task", "Взятие задачи"),
        }
    }

    /// Babel's read-only rule for an error task opened by this request: claims are
    /// editable; a viewed action only while it is this worker's in-progress action; the
    /// latest action of a processed transcription never.
    pub fn error_task_read_only(&self, task: &ErrorTask, worker_id: Option<&str>) -> bool {
        match self {
            Self::ClaimNext { .. }
            | Self::ClaimByProcessedTranscription { .. }
            | Self::ClaimErrorQaTask { .. } => false,
            Self::ReviewAction { .. } => task.viewed_read_only(worker_id),
            Self::LatestReviewAction { .. } => true,
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
            Self::ClaimErrorQaTask {
                transcription_error_diff_id,
                queue_id,
            } => BabelRequest::ClaimTranscriptionErrorQaTask {
                transcription_error_diff_id,
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

/// An opened error-category task plus where its staged audio lives.
#[derive(Debug)]
pub struct LoadedErrorTask {
    pub task: ErrorTask,
    /// Base directory for resolving the focus recording's audio path.
    pub audio_base_dir: PathBuf,
}

/// What a task payload turned out to be.
#[derive(Debug)]
pub enum OpenedTask {
    /// An ordinary transcript (editor).
    Transcript(Box<LoadedTask>),
    /// A review action carrying a `transcriptionError` (error questionnaire).
    ErrorTask(Box<LoadedErrorTask>),
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

    /// Direct Babel mode; a persisted subaccount session cannot redirect this client.
    pub fn direct(mut config: BabelConfig) -> Self {
        config.relay_session = None;
        Self::new(config)
    }

    /// Relay mode with credentials owned by this session/document origin.
    pub fn relay(session: crate::subaccounts::SubaccountSession) -> Self {
        Self::new(BabelConfig::relay(session))
    }

    /// [`Self::relay`] with caches kept in `account_dir`.
    pub fn relay_in(
        session: crate::subaccounts::SubaccountSession,
        account_dir: &std::path::Path,
    ) -> Self {
        Self::new(BabelConfig::relay_in(session, account_dir))
    }

    /// Raw normalized response for `request` (also used by the review/subaccount services).
    pub async fn raw(&self, request: &BabelRequest) -> Result<String, BabelError> {
        payload_for(&self.config, request)
            .await
            .map_err(|error| transport_error("", error))
    }

    async fn raw_prefixed(
        &self,
        request: &BabelRequest,
        prefix: &str,
    ) -> Result<String, BabelError> {
        payload_for(&self.config, request)
            .await
            .map_err(|error| transport_error(prefix, error))
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
                "Task annotations load failed",
            )
            .await?;
        merge_review_action_annotations(&payload, &annotations)
    }

    /// Fetches the task and stages remote audio locally. A payload with a
    /// `transcriptionError` opens as an error task; anything else is imported as a transcript
    /// with `route_metadata` plus the project lint/tag profile metadata merged in.
    pub async fn open_task(
        &self,
        request: &TaskRequest,
        route_metadata: Vec<(String, String)>,
    ) -> Result<OpenedTask, BabelError> {
        let payload = self.fetch_task_payload(request).await?;
        let prepared = if audio_fetch::payload_needs_audio(&payload) {
            audio_fetch::prepare_payload_audio(payload, &self.config)
                .await
                .map_err(|error| BabelError::new(format!("{error:#}")))?
        } else {
            audio_fetch::PreparedPayload::local(payload)
        };
        if payload_is_error_task(&prepared.source) {
            return Ok(OpenedTask::ErrorTask(Box::new(LoadedErrorTask {
                task: parse_error_task(&prepared.source)?,
                audio_base_dir: prepared.audio_base_dir,
            })));
        }
        let mut workflow = StudioWorkflow::from_babel_json(&prepared.source)
            .map_err(|error| BabelError::new(format!("{error:#}")))?;
        workflow.merge_project_metadata(route_metadata);
        let lint_tag_metadata = babel_project_lint_tag_metadata(workflow.project());
        workflow.merge_project_metadata(lint_tag_metadata);
        Ok(OpenedTask::Transcript(Box::new(LoadedTask {
            workflow,
            audio_base_dir: prepared.audio_base_dir,
            remote_audio_count: prepared.remote_audio_count,
        })))
    }

    /// Submits an error-diff label (`transcriptions.submitTranscriptionErrorLabel`). Never
    /// saves annotations.
    pub async fn submit_error_label(
        &self,
        submission: ErrorLabelSubmission,
    ) -> Result<(), BabelError> {
        let payload = self
            .raw(&BabelRequest::SubmitTranscriptionErrorLabel(submission))
            .await?;
        parse_error_label_submit(&payload)
    }

    /// The project's complete Docs style guide for Initial fill (15 s budget).
    pub async fn error_task_style_guide(
        &self,
        project_id: &str,
    ) -> Result<StyleGuide, ErrorTaskError> {
        if !super::error_task::valid_project_id(project_id) {
            return Err(ErrorTaskError::new(
                "style-guide-project",
                "Couldn’t find this project’s style guide. Reopen the task from its project.",
            ));
        }
        let load = async {
            let read = |request: BabelRequest| async move {
                let payload = self.raw(&request).await.map_err(|error| {
                    ErrorTaskError::new(
                        "style-guide-unavailable",
                        format!("Couldn’t load the style guide: {}", error.message),
                    )
                })?;
                if payload.trim().is_empty() {
                    return Ok(serde_json::Value::Null);
                }
                serde_json::from_str::<serde_json::Value>(&payload).map_err(|_| {
                    ErrorTaskError::new(
                        "style-guide-invalid",
                        "Couldn’t read the style guide. Try again.",
                    )
                })
            };
            let documents = read(BabelRequest::LoadProjectDocuments {
                project_id: project_id.to_string(),
            })
            .await?;
            let document_id = select_style_guide_document(&documents)?;
            let document = read(BabelRequest::LoadProjectDocument {
                project_id: project_id.to_string(),
                document_id: document_id.clone(),
            })
            .await?;
            parse_style_guide_document(project_id, &document_id, &document)
        };
        tokio::time::timeout(std::time::Duration::from_secs(15), load)
            .await
            .unwrap_or_else(|_| {
                Err(ErrorTaskError::new(
                    "style-guide-unavailable",
                    "Couldn’t load the style guide. Check your connection and try again.",
                ))
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

    /// Submit a practice attempt through this document's captured session.
    /// The returned result is retained for the onboarding adapter.
    pub async fn submit_practice_attempt(
        &self,
        request: SaveAnnotationsRequest,
    ) -> Result<serde_json::Value, BabelError> {
        parse_mutation_value(
            &self
                .raw(&BabelRequest::SubmitPracticeAttempt(request))
                .await?,
            localized(
                "Couldn't submit the attempt. Try again.",
                "Не удалось отправить попытку. Попробуйте снова.",
            ),
        )
    }

    /// Submit an assessment, returning its pass/grant/attempt/error-rate result unchanged.
    pub async fn submit_assessment(
        &self,
        request: SaveAnnotationsRequest,
    ) -> Result<serde_json::Value, BabelError> {
        parse_mutation_value(
            &self.raw(&BabelRequest::SubmitAssessment(request)).await?,
            localized(
                "Couldn't submit the assessment. Try again.",
                "Не удалось отправить тест. Попробуйте снова.",
            ),
        )
    }

    /// Transition this queue from practice to assessment; the caller reloads its task.
    pub async fn transition_to_assessment(&self, queue_id: &str) -> Result<(), BabelError> {
        parse_mutation_value(
            &self
                .raw(&BabelRequest::TransitionToAssessment {
                    queue_id: queue_id.into(),
                })
                .await?,
            localized(
                "Couldn't open the assessment. Try again.",
                "Не удалось открыть тест. Попробуйте снова.",
            ),
        )
        .map(|_| ())
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
        parse_simple_mutation(
            &payload,
            localized(
                "Couldn't release the task. Try again.",
                "Не удалось отпустить задачу. Попробуйте снова.",
            ),
        )
    }

    /// Skips the current chunk.
    pub async fn skip_chunk(&self, review_action_id: String) -> Result<(), BabelError> {
        let payload = self
            .raw(&BabelRequest::SkipReviewAction { review_action_id })
            .await?;
        parse_simple_mutation(&payload, localized("Try again.", "Попробуйте снова."))
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
        parse_simple_mutation(&payload, localized("Try again.", "Попробуйте снова."))
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
                .raw_prefixed(&request, "Feedback draft load failed")
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

    /// Raw review actions of `review_action_id`'s chunk (`[{id, level, ...}]`).
    pub async fn review_actions_for_chunk(
        &self,
        review_action_id: &str,
    ) -> Result<String, BabelError> {
        self.raw_prefixed(
            &BabelRequest::LoadReviewActionsForChunk {
                review_action_id: review_action_id.to_string(),
            },
            "Chunk reviews load failed",
        )
        .await
    }
}

fn transport_error(prefix: &str, error: TransportError) -> BabelError {
    BabelError {
        message: with_detail(prefix, error.message),
        lint_issues: None,
        signed_out: error.signed_out,
    }
}
