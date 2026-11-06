//! Reviewer / Gold Drafting backend (native capability panel) and the required L2 feedback form.
//!
//! - [`ReviewBackend`]: async HTTP client (`STUDIO_REVIEW_BACKEND_BASE_URL`).
//! - Payload builders from the project ([`build_reviewer_payload`], drafting payloads).
//! - [`ReviewSnapshot`]: original (Babel L1) / current pair for Magic Review and the Grader.
//! - [`ReviewerWorkspace`]: interactive Magic Review session/card/comment/finalize state.
//! - Apply functions editing the project through `StudioWorkflow` (undoable).
//! - [`FeedbackForm`]: feedback drawer state, Babel bootstrap and autosave.

mod client;
mod feedback;
mod model;
mod payload;
mod snapshot;
mod workspace;

pub use client::{
    BackendHttpMethod, BackendRequestParts, DEFAULT_REVIEW_BACKEND_BASE_URL, OPENROUTER_KEY_HEADER,
    OPENROUTER_MODEL_HEADER, ProposalDecision, REVIEW_BACKEND_BASE_URL_ENV, ReviewBackend,
    ReviewBackendRequest, ReviewCredentials, draft_payload_has_session_id, parse_sse_done_payload,
    review_backend_base_url_from_env, review_backend_url,
};
pub use feedback::{
    FeedbackBootstrap, FeedbackBootstrapIds, FeedbackForm, feedback_initially_open,
    feedback_is_tts_project, field_for_category, load_feedback_bootstrap, reviewer_feedback_rating,
    save_feedback_draft,
};
pub use model::{
    CategoryGrade, EvidenceDetail, FinalizeOutcome, GRADE_CATEGORIES, GeneratedReview, GradeResult,
    GradeSample, ReviewerCard, ReviewerFeedback, ReviewerFinalize, ReviewerSession,
    ReviewerSuggestion, ReviewerTemplate, grade_category_title, parse_finalize_outcome,
    parse_generated_review, parse_grade_result, parse_reviewer_session, parse_template_search,
};
pub use payload::{
    build_draft_generate_payload, build_draft_redistribute_payload,
    build_reviewer_analytics_payload, build_reviewer_payload,
    draft_generate_payloads_are_equivalent, ensure_draft_session_id, project_review_action_level,
    review_action_id, reviewer_capability_available,
};
pub use snapshot::{
    ReviewSnapshot, capture_review_snapshot, current_review_state, current_transcript_key,
    l1_reference_id, load_l1_original, normalize_review_action,
};
pub use workspace::{
    CommentSave, ReviewerNext, ReviewerWorkspace, TEMPLATE_SEARCH_LIMIT, TemplateSearch,
    TemplateSearchRequest, required_session_id,
};
