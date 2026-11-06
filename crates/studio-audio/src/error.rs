use std::{io, path::PathBuf};

/// Format-independent failures from the audio decoder.
#[derive(Debug, thiserror::Error)]
pub enum AudioDecodeError {
    #[error("unsupported container, codec, or encryption: {0}")]
    Unsupported(String),
    #[error("invalid or corrupt audio: {0}")]
    Invalid(String),
    #[error("truncated audio: {0}")]
    Truncated(String),
    #[error("audio stream I/O failed: {0}")]
    Io(#[source] io::Error),
}

/// Errors reported by decoding helpers and the playback engine.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("failed to read audio file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to decode audio: {0}")]
    Decode(#[source] AudioDecodeError),
    #[error("no audio file found for lane `{lane_id}`")]
    LaneAudioNotFound { lane_id: String },
    #[error("lane `{0}` has no loaded audio")]
    LaneNotLoaded(String),
    #[error("enhanced audio for lane `{lane_id}` does not match its original: {reason}")]
    EnhancedAudioMismatch { lane_id: String, reason: String },
    #[error("unsupported channel count: {0}")]
    UnsupportedChannelCount(usize),
    #[error("audio output unavailable: {0}")]
    OutputUnavailable(String),
}
