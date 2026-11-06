//! Provider-neutral drafting results, audio capture and undoable text application.
//! Platform request construction and network policy remain in the supplying adapter.

mod apply;
mod audio;
mod config;
mod draft_stream;
mod model;

pub use apply::{AppliedRows, apply_draft_rows, apply_redistribution, apply_redistribution_moves};
pub use audio::{
    DraftAudioCueStatus, DraftAudioGuard, DraftAudioMode, broker_audio_tracks,
    draft_audio_cue_status, draft_audio_guard, encode_pcm16_wav, resolve_draft_audio,
};
pub use config::{DRAFT_OPENROUTER_API_KEY_ENV, DraftingConfig, OPENROUTER_API_KEY_ENV};
pub use draft_stream::{DraftStreamEvent, DraftStreamStep, SseEvent, SseParser, draft_stream_step};
pub use model::{
    AudioTrack, BackendHealth, DraftResult, DraftRow, DraftRowPreview, DraftSummary,
    RedistributionAllocation, RedistributionMove, RedistributionPreview, draft_row_previews,
    parse_backend_health, parse_draft_result, parse_redistribution,
};
