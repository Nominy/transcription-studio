pub const STITCHED_SELECT_DESCRIPTION: &str =
    "Claim a specific stitched transcription or view a transcription in read-only mode.";
pub const READ_ONLY_SELECT_DESCRIPTION: &str = "View a transcription in read-only mode.";
pub const CONFIRM_EDIT_BODY: &str = "Are you sure you want to edit the stitched transcript? You will have to submit again to mark the transcript as verified. For audits, please use the read only flow.";
pub const PROCESSED_TRANSCRIPTION_PLACEHOLDER: &str =
    "Processed Transcription ID (For Stitched Queue)";
pub const REVIEW_ACTION_PLACEHOLDER: &str = "Review Action ID (For Read-Only)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLookupState {
    pub processed_transcription_id: String,
    pub review_action_id: String,
    pub show_stitched_option: bool,
    pub confirming_edit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLookupInitialInput {
    pub processed_transcription_id: String,
    pub review_action_id: String,
    pub show_stitched_option: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLookupSelectInput {
    pub processed_transcription_id: String,
    pub review_action_id: String,
    pub show_stitched_option: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectActionModel {
    pub branch: String,
    pub title: String,
    pub description: String,
    pub show_processed_transcription_input: bool,
    pub processed_transcription_placeholder: String,
    pub review_action_placeholder: String,
    pub processed_transcription_id: String,
    pub review_action_id: String,
    pub claim_stitched_disabled: bool,
    pub read_only_disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmEditModel {
    pub branch: String,
    pub title: String,
    pub body: String,
    pub cancel_label: String,
    pub confirm_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskLookupEffect {
    SetProcessedTranscriptionId { value: String },
    SetReviewActionId { value: String },
    ClaimById,
    ViewReadOnlyByReviewActionId,
    ViewReadOnlyByProcessedTranscriptionId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskLookupEvent {
    ChangeProcessedTranscriptionId { value: String },
    ChangeReviewActionId { value: String },
    ClickClaimStitched,
    CancelConfirmEdit,
    ConfirmEdit,
    ClickReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLookupUpdate {
    pub state: TaskLookupState,
    pub effects: Vec<TaskLookupEffect>,
}

pub fn trim_task_lookup_field_value(value: &str) -> String {
    value.trim().to_string()
}

pub fn create_initial_task_lookup_state(input: TaskLookupInitialInput) -> TaskLookupState {
    TaskLookupState {
        processed_transcription_id: input.processed_transcription_id,
        review_action_id: input.review_action_id,
        show_stitched_option: input.show_stitched_option.unwrap_or(false),
        confirming_edit: false,
    }
}

impl TaskLookupState {
    pub fn select_action_model(&self) -> SelectActionModel {
        build_select_action_model(TaskLookupSelectInput {
            processed_transcription_id: self.processed_transcription_id.clone(),
            review_action_id: self.review_action_id.clone(),
            show_stitched_option: Some(self.show_stitched_option),
        })
    }

    pub fn confirm_edit_model(&self) -> ConfirmEditModel {
        build_confirm_edit_model()
    }
}

pub fn build_select_action_model(input: TaskLookupSelectInput) -> SelectActionModel {
    let show_stitched_option = input.show_stitched_option.unwrap_or(false);
    let has_read_only_target =
        !input.review_action_id.is_empty() || !input.processed_transcription_id.is_empty();

    SelectActionModel {
        branch: "select-action".to_string(),
        title: "Select Action".to_string(),
        description: if show_stitched_option {
            STITCHED_SELECT_DESCRIPTION
        } else {
            READ_ONLY_SELECT_DESCRIPTION
        }
        .to_string(),
        show_processed_transcription_input: show_stitched_option,
        processed_transcription_placeholder: PROCESSED_TRANSCRIPTION_PLACEHOLDER.to_string(),
        review_action_placeholder: REVIEW_ACTION_PLACEHOLDER.to_string(),
        claim_stitched_disabled: !show_stitched_option
            || input.processed_transcription_id.is_empty(),
        read_only_disabled: !has_read_only_target,
        processed_transcription_id: input.processed_transcription_id,
        review_action_id: input.review_action_id,
    }
}

pub fn build_confirm_edit_model() -> ConfirmEditModel {
    ConfirmEditModel {
        branch: "confirm-edit".to_string(),
        title: "Confirm Edit".to_string(),
        body: CONFIRM_EDIT_BODY.to_string(),
        cancel_label: "Cancel".to_string(),
        confirm_label: "Confirm Edit".to_string(),
    }
}

pub fn reduce_task_lookup_event(
    state: TaskLookupState,
    event: TaskLookupEvent,
) -> TaskLookupUpdate {
    match event {
        TaskLookupEvent::ChangeProcessedTranscriptionId { value } => {
            let value = trim_task_lookup_field_value(&value);
            let mut state = state;
            state.processed_transcription_id = value.clone();
            TaskLookupUpdate {
                state,
                effects: vec![TaskLookupEffect::SetProcessedTranscriptionId { value }],
            }
        }
        TaskLookupEvent::ChangeReviewActionId { value } => {
            let value = trim_task_lookup_field_value(&value);
            let mut state = state;
            state.review_action_id = value.clone();
            TaskLookupUpdate {
                state,
                effects: vec![TaskLookupEffect::SetReviewActionId { value }],
            }
        }
        TaskLookupEvent::ClickClaimStitched => {
            if !state.show_stitched_option || state.processed_transcription_id.is_empty() {
                return TaskLookupUpdate {
                    state,
                    effects: Vec::new(),
                };
            }

            let mut state = state;
            state.confirming_edit = true;
            TaskLookupUpdate {
                state,
                effects: Vec::new(),
            }
        }
        TaskLookupEvent::CancelConfirmEdit => {
            let mut state = state;
            state.confirming_edit = false;
            TaskLookupUpdate {
                state,
                effects: Vec::new(),
            }
        }
        TaskLookupEvent::ConfirmEdit => {
            let mut state = state;
            state.confirming_edit = false;
            TaskLookupUpdate {
                state,
                effects: vec![TaskLookupEffect::ClaimById],
            }
        }
        TaskLookupEvent::ClickReadOnly => {
            if !state.review_action_id.is_empty() {
                return TaskLookupUpdate {
                    state,
                    effects: vec![TaskLookupEffect::ViewReadOnlyByReviewActionId],
                };
            }
            if !state.processed_transcription_id.is_empty() {
                return TaskLookupUpdate {
                    state,
                    effects: vec![TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId],
                };
            }
            TaskLookupUpdate {
                state,
                effects: Vec::new(),
            }
        }
    }
}
