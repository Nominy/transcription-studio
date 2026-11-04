//! Reviewer / Gold Drafting backend (native capability panel) and the required L2 feedback form.
//!
//! - [`ReviewBackend`]: async HTTP client (`STUDIO_REVIEW_BACKEND_BASE_URL`).
//! - Payload builders from the project ([`build_reviewer_payload`], drafting payloads).
//! - [`ReviewSnapshot`]: original (Babel L1) / current pair for Magic Review and the Grader.
//! - [`ReviewerWorkspace`]: interactive Magic Review session/card/comment/finalize state.
//! - Apply functions editing the project through `StudioWorkflow` (undoable).
//! - [`FeedbackForm`]: feedback drawer state, Babel bootstrap and autosave.

mod apply;
mod audio;
mod client;
mod draft_stream;
mod feedback;
mod model;
mod payload;
mod snapshot;
mod workspace;

pub use apply::{AppliedRows, apply_draft_rows, apply_redistribution, apply_redistribution_moves};
pub use audio::{
    DraftAudioCueStatus, DraftAudioGuard, DraftAudioMode, broker_audio_tracks,
    draft_audio_cue_status, draft_audio_guard, encode_pcm16_wav, resolve_draft_audio,
};
pub use client::{
    AudioTrack, BackendHttpMethod, BackendRequestParts, DEFAULT_REVIEW_BACKEND_BASE_URL,
    OPENROUTER_KEY_HEADER, OPENROUTER_MODEL_HEADER, ProposalDecision, REVIEW_BACKEND_BASE_URL_ENV,
    ReviewBackend, ReviewBackendRequest, ReviewCredentials, draft_payload_has_session_id,
    parse_sse_done_payload, review_backend_base_url_from_env, review_backend_url,
};
pub use draft_stream::{DraftStreamEvent, DraftStreamStep, SseEvent, SseParser, draft_stream_step};
pub use feedback::{
    FeedbackBootstrap, FeedbackBootstrapIds, FeedbackForm, feedback_initially_open,
    feedback_is_tts_project, field_for_category, load_feedback_bootstrap, reviewer_feedback_rating,
    save_feedback_draft,
};
pub use model::{
    BackendHealth, CategoryGrade, DraftResult, DraftRow, DraftRowPreview, DraftSummary,
    EvidenceDetail, FinalizeOutcome, GRADE_CATEGORIES, GeneratedReview, GradeResult, GradeSample,
    RedistributionAllocation, RedistributionMove, RedistributionPreview, ReviewerCard,
    ReviewerFeedback, ReviewerFinalize, ReviewerSession, ReviewerSuggestion, ReviewerTemplate,
    draft_row_previews, parse_backend_health, parse_draft_result, parse_finalize_outcome,
    parse_generated_review, parse_grade_result, parse_redistribution, parse_reviewer_session,
    parse_template_search,
};
pub use payload::{
    DRAFT_OPENROUTER_API_KEY_ENV, DraftingConfig, OPENROUTER_API_KEY_ENV,
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
    TemplateSearchRequest, required_field,
};
