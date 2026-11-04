//! Task-lookup dialog state: look up a stitched transcription to claim, or open a review
//! action / processed transcription read-only.

use studio_core::Project;

use crate::babel_metadata::metadata_bool;

const META_TASK_LOOKUP_OPEN: &str = "studio.babel.taskLookup.open";
const META_TASK_LOOKUP_PROCESSED_ID: &str = "studio.babel.taskLookup.processedTranscriptionId";
const META_TASK_LOOKUP_REVIEW_ACTION_ID: &str = "studio.babel.taskLookup.reviewActionId";
const META_TASK_LOOKUP_SHOW_STITCHED: &str = "studio.babel.taskLookup.showStitchedOption";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskLookupState {
    pub processed_transcription_id: String,
    pub review_action_id: String,
    /// The worker may claim stitched transcriptions (enables the processed-id input).
    pub show_stitched_option: bool,
    /// The "confirm edit of a stitched transcript" step is showing.
    pub confirming_edit: bool,
    /// Error-categorization projects claim QA tasks by diff id instead of stitched
    /// transcriptions.
    pub claims_qa_tasks_by_id: bool,
    pub qa_diff_id: String,
}

/// Request the lookup dialog asks the app to perform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskLookupEffect {
    ClaimById,
    /// Claim the QA task of `qa_diff_id` (error-categorization projects).
    ClaimQaTask,
    ViewReadOnlyByReviewActionId,
    ViewReadOnlyByProcessedTranscriptionId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskLookupEvent {
    ChangeProcessedTranscriptionId { value: String },
    ChangeReviewActionId { value: String },
    ChangeQaDiffId { value: String },
    ClickClaimStitched,
    ClickClaimQa,
    CancelConfirmEdit,
    ConfirmEdit,
    ClickReadOnly,
}

impl TaskLookupState {
    pub fn new(
        processed_transcription_id: impl Into<String>,
        review_action_id: impl Into<String>,
        show_stitched_option: bool,
    ) -> Self {
        Self {
            processed_transcription_id: processed_transcription_id.into(),
            review_action_id: review_action_id.into(),
            show_stitched_option,
            confirming_edit: false,
            claims_qa_tasks_by_id: false,
            qa_diff_id: String::new(),
        }
    }

    /// For error-categorization projects: the claim field takes a diff id (QA task).
    pub fn claiming_qa_tasks(mut self, claims_qa_tasks_by_id: bool) -> Self {
        self.claims_qa_tasks_by_id = claims_qa_tasks_by_id;
        self
    }

    /// Lookup state requested by the startup route (`studio.babel.taskLookup.*` metadata);
    /// `None` unless the lookup dialog was asked to open.
    pub fn from_project(project: &Project) -> Option<Self> {
        if !metadata_bool(project, META_TASK_LOOKUP_OPEN) {
            return None;
        }
        let field = |key: &str| project.metadata.get(key).cloned().unwrap_or_default();
        Some(Self::new(
            field(META_TASK_LOOKUP_PROCESSED_ID),
            field(META_TASK_LOOKUP_REVIEW_ACTION_ID),
            metadata_bool(project, META_TASK_LOOKUP_SHOW_STITCHED),
        ))
    }

    /// The processed-transcription claim field is shown.
    pub fn shows_stitched_claim(&self) -> bool {
        self.show_stitched_option && !self.claims_qa_tasks_by_id
    }

    /// The QA diff-id claim field is shown.
    pub fn shows_qa_claim(&self) -> bool {
        self.show_stitched_option && self.claims_qa_tasks_by_id
    }

    pub fn can_claim_stitched(&self) -> bool {
        self.shows_stitched_claim() && !self.processed_transcription_id.is_empty()
    }

    pub fn can_claim_qa(&self) -> bool {
        self.shows_qa_claim() && !self.qa_diff_id.is_empty()
    }

    pub fn can_view_read_only(&self) -> bool {
        !self.review_action_id.is_empty() || !self.processed_transcription_id.is_empty()
    }

    /// Applies a UI event (field values are trimmed) and returns the request to perform, if any.
    pub fn apply(&mut self, event: TaskLookupEvent) -> Option<TaskLookupEffect> {
        match event {
            TaskLookupEvent::ChangeProcessedTranscriptionId { value } => {
                self.processed_transcription_id = value.trim().to_string();
                None
            }
            TaskLookupEvent::ChangeReviewActionId { value } => {
                self.review_action_id = value.trim().to_string();
                None
            }
            TaskLookupEvent::ChangeQaDiffId { value } => {
                self.qa_diff_id = value.trim().to_string();
                None
            }
            TaskLookupEvent::ClickClaimQa => {
                self.can_claim_qa().then_some(TaskLookupEffect::ClaimQaTask)
            }
            TaskLookupEvent::ClickClaimStitched => {
                if self.can_claim_stitched() {
                    self.confirming_edit = true;
                }
                None
            }
            TaskLookupEvent::CancelConfirmEdit => {
                self.confirming_edit = false;
                None
            }
            TaskLookupEvent::ConfirmEdit => {
                self.confirming_edit = false;
                Some(TaskLookupEffect::ClaimById)
            }
            TaskLookupEvent::ClickReadOnly => {
                if !self.review_action_id.is_empty() {
                    Some(TaskLookupEffect::ViewReadOnlyByReviewActionId)
                } else if !self.processed_transcription_id.is_empty() {
                    Some(TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId)
                } else {
                    None
                }
            }
        }
    }
}
