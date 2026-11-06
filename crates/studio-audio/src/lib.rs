//! UI-free audio decoding and playback engine for Exaudio.
//!
//! - Decoding ([`decode_audio_file`], [`load_lane_audio`], ...): common audio
//!   containers and project lane file lookup. Blocking; run off the UI thread.
//! - [`AudioEngine`]: decoded lane storage, transport (play / pause / seek /
//!   speed / loop / audition), per-lane mute/solo/volume, per-lane enhanced
//!   (denoised) PCM blended with the original at a live ratio, and rodio
//!   output.
//! - [`blend_lane_audio`]: the PCM one lane plays at an enhanced ratio, for
//!   waveforms of what is heard.
//! - [`EnhancedAudioCache`]: enhanced lane WAVs on disk, keyed by task,
//!   enhancer model and original source digest.
//! - [`AudioEngine::offline`] + [`ManualClock`]: deterministic headless engine
//!   for tests (no audio device).

mod blend;
mod clock;
mod decode;
mod engine;
mod enhancement_cache;
mod error;
mod source;
mod transport;

pub use blend::blend_lane_audio;
pub use clock::ManualClock;
pub use decode::{
    LaneAudioFile, decode_audio_bytes, decode_audio_file, load_lane_audio,
    load_lane_audio_candidates,
};
pub use engine::{AudioEngine, LaneMix, MAX_LANE_VOLUME, OfflineOutput};
pub use enhancement_cache::{EnhancedAudioCache, MAX_CACHED_ENHANCEMENT_TASKS, sha256_hex};
pub use error::{AudioDecodeError, AudioError};
pub use studio_waveform::DecodedAudio;
pub use transport::{
    LoopRange, MIN_LOOP_DURATION_SECONDS, MIN_PLAYBACK_SPEED, PLAYBACK_SPEED_STEPS, PlaybackEvent,
    PlaybackState, adjacent_playback_speed,
};
