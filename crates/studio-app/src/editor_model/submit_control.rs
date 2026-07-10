use studio_core::Project;

use super::EditorSession;

const ADMIN_FLAG_RATING: u8 = 1;
const ADMIN_FLAG_DESCRIPTION: &str = "Admin rating marks a recording as unusable and flags the transcription. Use this when the audio or performance has issues that make it unusable - such as poor audio quality, poor acting, removing tags, or a voice actor going fully off-script. You must still edit and submit an accurate transcription before admin rating.";
const ADMIN_FLAG_REASON_PLACEHOLDER: &str =
    "e.g. background noise, wrong emotion, mispronunciation...";

const SKIP_DISABLED_TOOLTIP: &str = "Listen to this chunk for a bit longer before skipping.";
const SKIP_CHUNK_DESCRIPTION: &str = "The chunk will be archived and you'll be returned to the project page. This counts against your daily skip cap (20% of your task limit). It will not count toward your task limits or quality metrics.";

const SUBMIT_ERROR_DESCRIPTION: &str =
    "Submission was rejected. See the status bar below for the exact reason.";
const SUBMIT_PENDING_OVERLAY_TEXT: &str = "Submitting your review...";

const META_ASSESSMENT_PHASE: &str = "studio.babel.diff.assessmentPhase";
const META_ADMIN_FLAG_AVAILABLE: &str = "studio.babel.adminFlag.available";
const META_SKIP_AVAILABLE: &str = "studio.babel.skip.available";
const META_SKIP_CAN_SKIP: &str = "studio.babel.skip.canSkip";
const META_SUBMIT_ASSESSMENT_PHASE: &str = "studio.babel.submit.assessmentPhase";
const META_SUBMIT_MODE: &str = "studio.babel.submit.mode";
const META_SUBMIT_HAS_MISSING_INTENSITY: &str = "studio.babel.submit.hasMissingIntensity";
const META_SUBMIT_FEEDBACK_INCOMPLETE: &str = "studio.babel.submit.hasFeedbackIncomplete";
const META_SUBMIT_FEEDBACK_SAVING: &str = "studio.babel.submit.isFeedbackSaving";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitControlView {
    pub open: bool,
    pub trigger_button: String,
    pub trigger_tooltip_variant: SubmitTooltipVariant,
    pub trigger_tooltip_content: String,
    pub trigger_disabled: bool,
    pub show_pending_overlay: bool,
    pub pending_overlay_text: String,
    pub dialog_title: String,
    pub dialog_description: String,
    pub dialog_description_tone: SubmitDialogDescriptionTone,
    pub close_label: String,
    pub cancel_label: String,
    pub cancel_disabled: bool,
    pub confirm_label: String,
    pub confirm_disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminFlagControlView {
    pub open: bool,
    pub trigger_label: String,
    pub title: String,
    pub description: String,
    pub reason_label: String,
    pub reason_placeholder: String,
    pub reason_value: String,
    pub reason_disabled: bool,
    pub cancel_label: String,
    pub cancel_disabled: bool,
    pub confirm_label: String,
    pub confirm_disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminFlagPayload {
    pub rating: u8,
    pub rating_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkipChunkControlView {
    pub open: bool,
    pub trigger_label: String,
    pub trigger_disabled: bool,
    pub trigger_tooltip: Option<String>,
    pub title: String,
    pub description: String,
    pub cancel_label: String,
    pub cancel_disabled: bool,
    pub confirm_label: String,
    pub confirm_disabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitTooltipVariant {
    Destructive,
    Success,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitDialogDescriptionTone {
    Default,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SubmitBlockState {
    variant: SubmitTooltipVariant,
    content: &'static str,
    is_blocked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SubmitDialogCopy {
    trigger_button: &'static str,
    dialog_title: &'static str,
    dialog_description: &'static str,
    confirm_button: &'static str,
}

const DEFAULT_SUBMIT_COPY: SubmitDialogCopy = SubmitDialogCopy {
    trigger_button: "Submit Review",
    dialog_title: "Confirm Submission",
    dialog_description: "Are you sure you want to submit?",
    confirm_button: "Submit",
};

const ASSESSMENT_SUBMIT_COPY: SubmitDialogCopy = SubmitDialogCopy {
    trigger_button: "Submit Assessment",
    dialog_title: "Submit Final Assessment",
    dialog_description: "This is your final assessment submission. Your transcription will be reviewed by our team. You will not be able to retry after submitting.",
    confirm_button: "Submit Assessment",
};

const PRACTICE_SUBMIT_COPY: SubmitDialogCopy = SubmitDialogCopy {
    trigger_button: "Submit Practice",
    dialog_title: "Submit Practice Attempt",
    dialog_description: "Your transcription will be compared to the answer key. You'll be able to review your results and retry as many times as you like. When you're ready, click \"Take Assessment\" to submit for real.",
    confirm_button: "Submit Practice",
};

impl EditorSession {
    pub fn admin_flag_control(
        open: bool,
        reason: impl Into<String>,
        is_submitting: bool,
    ) -> AdminFlagControlView {
        AdminFlagControlView {
            open,
            trigger_label: "Flag Recording".to_string(),
            title: "Flag this recording?".to_string(),
            description: ADMIN_FLAG_DESCRIPTION.to_string(),
            reason_label: "Reason (optional)".to_string(),
            reason_placeholder: ADMIN_FLAG_REASON_PLACEHOLDER.to_string(),
            reason_value: reason.into(),
            reason_disabled: is_submitting,
            cancel_label: "Cancel".to_string(),
            cancel_disabled: is_submitting,
            confirm_label: if is_submitting {
                "Flagging...".to_string()
            } else {
                "Confirm Flag".to_string()
            },
            confirm_disabled: is_submitting,
        }
    }

    pub fn admin_flag_control_available(project: &Project) -> bool {
        project_metadata_flag(project, META_ADMIN_FLAG_AVAILABLE)
    }

    pub fn admin_flag_payload(reason: &str) -> AdminFlagPayload {
        AdminFlagPayload {
            rating: ADMIN_FLAG_RATING,
            rating_reason: non_empty_trimmed(reason),
        }
    }

    pub fn skip_chunk_control_available(project: &Project) -> bool {
        project.metadata.contains_key(META_SKIP_CAN_SKIP)
            || project_metadata_flag(project, META_SKIP_AVAILABLE)
    }

    pub fn skip_chunk_control(
        project: &Project,
        open: bool,
        is_skipping: bool,
    ) -> SkipChunkControlView {
        let can_skip = project
            .metadata
            .get(META_SKIP_CAN_SKIP)
            .is_none_or(|value| metadata_value_is_truthy(value));

        SkipChunkControlView {
            open,
            trigger_label: "Skip Chunk".to_string(),
            trigger_disabled: !can_skip || is_skipping,
            trigger_tooltip: (!can_skip).then(|| SKIP_DISABLED_TOOLTIP.to_string()),
            title: "Skip this chunk?".to_string(),
            description: SKIP_CHUNK_DESCRIPTION.to_string(),
            cancel_label: "Cancel".to_string(),
            cancel_disabled: is_skipping,
            confirm_label: if is_skipping {
                "Skipping...".to_string()
            } else {
                "Confirm Skip".to_string()
            },
            confirm_disabled: is_skipping,
        }
    }

    pub fn submit_control(
        &self,
        project: &Project,
        open: bool,
        is_pending: bool,
        is_error: bool,
    ) -> SubmitControlView {
        let block_state = self.resolve_submit_block_state(project);
        let copy = submit_dialog_copy(project);

        SubmitControlView {
            open,
            trigger_button: copy.trigger_button.to_string(),
            trigger_tooltip_variant: block_state.variant,
            trigger_tooltip_content: block_state.content.to_string(),
            trigger_disabled: block_state.is_blocked,
            show_pending_overlay: is_pending,
            pending_overlay_text: SUBMIT_PENDING_OVERLAY_TEXT.to_string(),
            dialog_title: copy.dialog_title.to_string(),
            dialog_description: if is_error {
                SUBMIT_ERROR_DESCRIPTION.to_string()
            } else {
                copy.dialog_description.to_string()
            },
            dialog_description_tone: if is_error {
                SubmitDialogDescriptionTone::Error
            } else {
                SubmitDialogDescriptionTone::Default
            },
            close_label: "Close".to_string(),
            cancel_label: "Cancel".to_string(),
            cancel_disabled: is_pending,
            confirm_label: if is_pending {
                "Submitting...".to_string()
            } else {
                copy.confirm_button.to_string()
            },
            confirm_disabled: is_pending,
        }
    }

    fn resolve_submit_block_state(&self, project: &Project) -> SubmitBlockState {
        if project
            .metadata
            .get(META_SUBMIT_MODE)
            .is_some_and(|mode| mode == "annotation")
        {
            if project_metadata_flag(project, META_SUBMIT_HAS_MISSING_INTENSITY) {
                return SubmitBlockState {
                    variant: SubmitTooltipVariant::Destructive,
                    content: "Please set intensity for all annotations",
                    is_blocked: true,
                };
            }

            return SubmitBlockState {
                variant: SubmitTooltipVariant::Success,
                content: "Submit your review",
                is_blocked: false,
            };
        }

        if self.submission_blocking_lint_count(project) > 0 {
            return SubmitBlockState {
                variant: SubmitTooltipVariant::Destructive,
                content: "Please fix linter errors",
                is_blocked: true,
            };
        }

        if self.submission_unacknowledged_warning_count(project) > 0 {
            return SubmitBlockState {
                variant: SubmitTooltipVariant::Destructive,
                content: "Please acknowledge all warnings",
                is_blocked: true,
            };
        }

        if self.submission_unresolved_low_confidence_count(project) > 0 {
            return SubmitBlockState {
                variant: SubmitTooltipVariant::Destructive,
                content: "Please review all low-confidence words",
                is_blocked: true,
            };
        }

        if project_metadata_flag(project, META_SUBMIT_FEEDBACK_SAVING) {
            return SubmitBlockState {
                variant: SubmitTooltipVariant::Warning,
                content: "Saving feedback...",
                is_blocked: true,
            };
        }

        if project_metadata_flag(project, META_SUBMIT_FEEDBACK_INCOMPLETE) {
            return SubmitBlockState {
                variant: SubmitTooltipVariant::Destructive,
                content: "Please complete the feedback form",
                is_blocked: true,
            };
        }

        SubmitBlockState {
            variant: SubmitTooltipVariant::Success,
            content: "Submit your review",
            is_blocked: false,
        }
    }
}

fn submit_dialog_copy(project: &Project) -> SubmitDialogCopy {
    match project
        .metadata
        .get(META_SUBMIT_ASSESSMENT_PHASE)
        .or_else(|| project.metadata.get(META_ASSESSMENT_PHASE))
        .map(String::as_str)
    {
        Some("assessment") => ASSESSMENT_SUBMIT_COPY,
        Some("practice") => PRACTICE_SUBMIT_COPY,
        _ => DEFAULT_SUBMIT_COPY,
    }
}

fn project_metadata_flag(project: &Project, key: &str) -> bool {
    project
        .metadata
        .get(key)
        .is_some_and(|value| metadata_value_is_truthy(value))
}

fn metadata_value_is_truthy(value: &str) -> bool {
    matches!(
        value,
        "true" | "1" | "yes" | "on" | "True" | "TRUE" | "Yes" | "YES" | "On" | "ON"
    )
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
