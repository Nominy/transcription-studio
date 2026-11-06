//! Submit / skip-chunk / admin-flag eligibility and payloads.
//!
//! Dialog copy lives in the UI; this module only decides what is allowed and why not.

use studio_core::Project;

use crate::babel_metadata::{AssessmentPhase, metadata_bool};
use crate::i18n::localized;

use super::{EditorSession, EditorSessionView};

const ADMIN_FLAG_RATING: u8 = 1;

const META_DIFF_ASSESSMENT_PHASE: &str = "studio.babel.diff.assessmentPhase";
const META_ADMIN_FLAG_AVAILABLE: &str = "studio.babel.adminFlag.available";
const META_SKIP_AVAILABLE: &str = "studio.babel.skip.available";
const META_SKIP_CAN_SKIP: &str = "studio.babel.skip.canSkip";
const META_SUBMIT_ASSESSMENT_PHASE: &str = "studio.babel.submit.assessmentPhase";
const META_SUBMIT_MODE: &str = "studio.babel.submit.mode";
const META_SUBMIT_HAS_MISSING_INTENSITY: &str = "studio.babel.submit.hasMissingIntensity";
const META_SUBMIT_FEEDBACK_INCOMPLETE: &str = "studio.babel.submit.hasFeedbackIncomplete";
const META_SUBMIT_FEEDBACK_SAVING: &str = "studio.babel.submit.isFeedbackSaving";

/// What a submission means for this task (selects the confirmation copy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitKind {
    Review,
    Practice,
    FinalAssessment,
}

/// Why submission is currently not allowed, in the priority order they are checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitBlocker {
    /// Annotation tasks: some annotation has no intensity.
    MissingIntensity,
    LintErrors,
    UnacknowledgedWarnings,
    UnresolvedLowConfidence,
    /// Feedback form is still being saved (transient, shown as a warning).
    FeedbackSaving,
    FeedbackIncomplete,
}

impl SubmitBlocker {
    /// User-facing reason shown on the disabled submit control.
    pub fn reason(self) -> &'static str {
        match self {
            Self::MissingIntensity => localized(
                "Set an intensity for every annotation",
                "Укажите интенсивность для всех аннотаций",
            ),
            Self::LintErrors => localized("Fix the problems first", "Сначала исправьте проблемы"),
            Self::UnacknowledgedWarnings => localized(
                "Dismiss all warnings first",
                "Сначала скройте все предупреждения",
            ),
            Self::UnresolvedLowConfidence => localized(
                "Review all low-confidence words",
                "Проверьте все слова с низкой уверенностью",
            ),
            Self::FeedbackSaving => localized("Saving feedback…", "Сохранение отзыва…"),
            Self::FeedbackIncomplete => {
                localized("Finish the feedback form", "Заполните форму отзыва")
            }
        }
    }

    /// Whether the blocker concerns the feedback form (the UI opens the Feedback panel).
    pub fn is_feedback(self) -> bool {
        matches!(self, Self::FeedbackSaving | Self::FeedbackIncomplete)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmitEligibility {
    pub kind: SubmitKind,
    /// `None` = submit is allowed.
    pub blocker: Option<SubmitBlocker>,
}

impl SubmitEligibility {
    pub fn is_allowed(&self) -> bool {
        self.blocker.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminFlagPayload {
    pub rating: u8,
    pub rating_reason: Option<String>,
}

impl EditorSession<'_> {
    pub fn admin_flag_available(project: &Project) -> bool {
        metadata_bool(project, META_ADMIN_FLAG_AVAILABLE)
    }

    /// Payload for flagging the recording; blank reasons are omitted.
    pub fn admin_flag_payload(reason: &str) -> AdminFlagPayload {
        let reason = reason.trim();
        AdminFlagPayload {
            rating: ADMIN_FLAG_RATING,
            rating_reason: (!reason.is_empty()).then(|| reason.to_string()),
        }
    }

    pub fn skip_chunk_available(project: &Project) -> bool {
        project.metadata.contains_key(META_SKIP_CAN_SKIP)
            || metadata_bool(project, META_SKIP_AVAILABLE)
    }

    /// `false` while the backend says the chunk has not been listened to long enough.
    pub fn skip_chunk_allowed(project: &Project) -> bool {
        !project.metadata.contains_key(META_SKIP_CAN_SKIP)
            || metadata_bool(project, META_SKIP_CAN_SKIP)
    }
}

fn submit_kind(project: &Project) -> SubmitKind {
    match project
        .metadata
        .get(META_SUBMIT_ASSESSMENT_PHASE)
        .or_else(|| project.metadata.get(META_DIFF_ASSESSMENT_PHASE))
        .and_then(|phase| AssessmentPhase::parse(phase))
    {
        Some(AssessmentPhase::Assessment) => SubmitKind::FinalAssessment,
        Some(AssessmentPhase::Practice) => SubmitKind::Practice,
        None => SubmitKind::Review,
    }
}

impl EditorSessionView<'_> {
    pub fn submit_eligibility(&self, project: &Project) -> SubmitEligibility {
        SubmitEligibility {
            kind: submit_kind(project),
            blocker: self.submit_blocker(project),
        }
    }
    fn submit_blocker(&self, project: &Project) -> Option<SubmitBlocker> {
        if project
            .metadata
            .get(META_SUBMIT_MODE)
            .is_some_and(|mode| mode == "annotation")
        {
            return metadata_bool(project, META_SUBMIT_HAS_MISSING_INTENSITY)
                .then_some(SubmitBlocker::MissingIntensity);
        }
        if self.submission_blocking_lint_count(project) > 0 {
            return Some(SubmitBlocker::LintErrors);
        }
        if self.submission_unacknowledged_warning_count(project) > 0 {
            return Some(SubmitBlocker::UnacknowledgedWarnings);
        }
        if self.submission_unresolved_low_confidence_count(project) > 0 {
            return Some(SubmitBlocker::UnresolvedLowConfidence);
        }
        if metadata_bool(project, META_SUBMIT_FEEDBACK_SAVING) {
            return Some(SubmitBlocker::FeedbackSaving);
        }
        if metadata_bool(project, META_SUBMIT_FEEDBACK_INCOMPLETE) {
            return Some(SubmitBlocker::FeedbackIncomplete);
        }
        None
    }
}
