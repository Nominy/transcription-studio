//! Babel error-category tasks (`error-categorization` projects such as RU-TX-Error-Cat):
//! the loaded `transcriptionError` comparison, the static questionnaire with Babel's pairing
//! rules and `transcriptions.submitTranscriptionErrorLabel` payload, and Gold's assistant —
//! audio alignment of the diff through [`InferenceBackend::align`] and Initial fill through
//! OpenRouter decisions (`cloudflare/clef-omni`). The assistant fills only unanswered
//! choices and never submits.
//!
//! [`InferenceBackend::align`]: studio_inference::InferenceBackend::align

mod assistant;
mod broker;
mod choices;
mod clef;
mod diff_align;
mod style_guide;
mod task;
mod wav;

pub use assistant::{
    AlignedDiff, AssistantPhase, AssistantStatus, ClipSource, ErrorTaskSnapshot,
    InitialFillRequest, find_alignment, initial_fill_decision, status_with_boundary_note,
};
pub use broker::{
    AlignBackend, CancelToken, DecisionStore, ErrorTaskProgress, ProgressPhase, Receipt,
};
pub use choices::{
    AUDIO_ONLY_CATEGORIES, ApplyDraftError, CATEGORY_GROUPS, CategoryGroup, ERROR_CATEGORIZATION,
    ErrorChoices, ErrorLabelDraft, ErrorLabelSubmission, LabelRow, MAX_NOTE_UNITS, apply_draft,
    category_name, filtered_groups, group_index, group_name, grouped_label_categories,
    is_known_category, label_rows, level_one_categories, missing_items, pairing_problem,
    picks_by_key, readiness, submission, valid_categories, validation_problem,
};
pub use clef::{
    CLEF_MODEL, DECISIONS_URL_ENV, DecisionComparison, decisions_url, decisions_url_with,
    parse_decision_response,
};
pub use diff_align::{
    AlignError, AudioInterval, BoundaryNote, ClipDirection, DiffAlignment, DiffSide,
    NormalizedDiff, TextWindow, TimedWord, align_error_diff_context,
    crop_aligned_error_diff_context, crop_error_diff_context, estimate_error_diff_clip,
    next_error_diff_clip, segment_error_diff_context, valid_alignment,
};
pub(crate) use style_guide::valid_id as valid_project_id;
pub use style_guide::{StyleGuide, parse_style_guide_document, select_style_guide_document};
pub use task::{
    ErrorLabel, ErrorSource, ErrorTask, FocusSegment, TranscriptionError, parse_error_task,
    payload_is_error_task,
};
pub use wav::encode_clip_wav;

/// Identity every assistant result is bound to (Gold `ErrorTaskIdentity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrorTaskIdentity {
    pub review_action_id: String,
    pub diff_id: String,
    pub annotation_id: String,
    /// The target speaker's processed recording id.
    pub track_id: String,
}

/// A failed assistant step: a stable `code` plus the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ErrorTaskError {
    pub code: &'static str,
    pub message: String,
}

impl ErrorTaskError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.code == "cancelled"
    }
}
