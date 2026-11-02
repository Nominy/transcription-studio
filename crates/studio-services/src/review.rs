//! Reviewer / Gold Drafting backend (native capability panel) and the required L2 feedback form.
//!
//! - [`ReviewBackend`]: async HTTP client (`STUDIO_REVIEW_BACKEND_BASE_URL`).
//! - Payload builders from the project ([`build_reviewer_payload`], drafting payloads).
//! - [`ReviewerWorkspace`]: session/comment/finalize state machine.
//! - Apply functions editing the project through `StudioWorkflow` (undoable).
//! - [`FeedbackForm`]: feedback drawer state, Babel bootstrap and autosave.

mod apply;
mod audio;
mod client;
mod feedback;
mod model;
mod payload;
mod workspace;

pub use apply::{
    AppliedRows, apply_draft_row, apply_draft_rows, apply_redistribution,
    apply_redistribution_moves, apply_segment_text, persist_reviewer_session_id,
};
pub use audio::{
    DraftAudioCueStatus, DraftAudioGuard, DraftAudioMode, broker_audio_tracks,
    draft_audio_cue_status, draft_audio_guard, encode_pcm16_wav, resolve_draft_audio,
};
pub use client::{
    AudioTrack, BackendHttpMethod, BackendRequestParts, DEFAULT_REVIEW_BACKEND_BASE_URL,
    REVIEW_BACKEND_BASE_URL_ENV, ReviewBackend, ReviewBackendRequest, draft_payload_has_session_id,
    is_reconnectable_draft_stream_error, parse_history_query, parse_sse_done_payload,
    review_backend_base_url_from_env, review_backend_url,
};
pub use feedback::{
    FeedbackBootstrap, FeedbackBootstrapIds, FeedbackForm, feedback_initially_open,
    feedback_is_tts_project, field_for_category, load_feedback_bootstrap,
    reviewer_feedback_for_card, reviewer_feedback_rating, save_feedback_draft,
};
pub use model::{
    BackendHealth, DraftResult, DraftRow, DraftRowPreview, DraftSummary, DraftTranscription,
    FinalizeOutcome, GeneratedReview, RedistributionAllocation, RedistributionMove,
    RedistributionPreview, ReviewerCard, ReviewerFeedback, ReviewerFinalize, ReviewerSession,
    ReviewerSuggestion, ReviewerTemplate, draft_row_previews, parse_backend_health,
    parse_draft_result, parse_finalize_outcome, parse_generated_review, parse_redistribution,
    parse_reviewer_session, parse_template_search, parse_transcription,
};
pub use payload::{
    DRAFT_OPENROUTER_API_KEY_ENV, DraftingConfig, META_REVIEWER_SELECTED_SESSION_ID,
    OPENROUTER_API_KEY_ENV, build_draft_generate_payload, build_draft_redistribute_payload,
    build_draft_transcribe_segment_payload, build_reviewer_analytics_payload,
    build_reviewer_payload, draft_generate_payloads_are_equivalent, ensure_draft_session_id,
    project_review_action_level, review_action_id, reviewer_capability_available,
    reviewer_session_id_from_project,
};
pub use workspace::{CommentSave, ReviewerNext, ReviewerWorkspace, required_field};
