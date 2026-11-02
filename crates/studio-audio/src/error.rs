use std::{io, path::PathBuf};

use studio_waveform::WavDecodeError;

/// Errors reported by decoding helpers and the playback engine.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("failed to read audio file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("audio is not a supported PCM16 WAV: {0:?}")]
    Decode(WavDecodeError),
    #[error("no audio file found for lane `{lane_id}`")]
    LaneAudioNotFound { lane_id: String },
    #[error("unsupported channel count: {0}")]
    UnsupportedChannelCount(usize),
    #[error("audio output unavailable: {0}")]
    OutputUnavailable(String),
}
