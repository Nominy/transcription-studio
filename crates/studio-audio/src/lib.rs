//! UI-free playback engine for Transcription Studio.
//!
//! - Decoding ([`decode_audio_file`], [`load_lane_audio`], ...): PCM16 WAV and
//!   project lane file lookup. Blocking; run off the UI thread.
//! - [`AudioEngine`]: decoded lane storage, transport (play / pause / seek /
//!   speed / loop / audition), per-lane mute/solo/volume and rodio output.
//! - [`AudioEngine::offline`] + [`ManualClock`]: deterministic headless engine
//!   for tests (no audio device).

mod clock;
mod decode;
mod engine;
mod error;
mod source;
mod transport;

pub use clock::ManualClock;
pub use decode::{audio_source_candidates, decode_audio_bytes, decode_audio_file, load_lane_audio};
pub use engine::{AudioEngine, LaneMix, MAX_LANE_VOLUME, OfflineOutput};
pub use error::AudioError;
pub use studio_waveform::DecodedAudio;
pub use transport::{
    LoopRange, MIN_LOOP_DURATION_SECONDS, MIN_PLAYBACK_SPEED, PLAYBACK_SPEED_STEPS, PlaybackEvent,
    PlaybackState, adjacent_playback_speed,
};
