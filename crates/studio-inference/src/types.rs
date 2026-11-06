//! Shared audio-provider values. Serialized field names remain stable for cached results;
//! provider-specific records are opaque JSON owned by the producing extension.

use std::{fmt, sync::Arc};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One lane's source audio as RIFF WAV.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneAudio {
    /// Stable lane identity; processed-audio results are keyed by it.
    pub lane_id: String,
    /// The transcript speaker key whose rows this audio belongs to; becomes the timing lane name.
    pub speaker_key: String,
    /// Human track label; used as the lane name when `speaker_key` is blank.
    pub label: String,
    pub wav: Bytes,
}

/// One timed word, in absolute seconds within its lane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingToken {
    pub id: String,
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// One detected speech segment of a lane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingSegment {
    pub id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub start_sample: u64,
    pub end_sample: u64,
    pub sample_rate: u32,
}

/// Lane timings. `e2e` is an optional opaque completed decode record reusable by its provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingTrack {
    pub lane: String,
    pub pcm_sha256: String,
    pub sample_rate: u32,
    pub tokens: Vec<TimingToken>,
    pub segments: Vec<TimingSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e2e: Option<Value>,
}

/// Timings; `summary`/`models` are opaque JSON objects owned by the producer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingResponse {
    pub task_id: String,
    pub tracks: Vec<TimingTrack>,
    pub summary: Value,
    pub models: Value,
}

/// One drafted transcript row; `lane` is the timing lane (speaker key).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRow {
    pub id: String,
    pub lane: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

/// Transcription result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftResponse {
    pub rows: Vec<DraftRow>,
    pub summary: Value,
    pub models: Value,
}

/// A row whose bounds are retained while its text is drafted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreservedRow {
    pub row_id: String,
    pub speaker_key: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    pub index: u32,
}

/// Denoised lane: canonical mono PCM16 WAV at the Original rate with the exact Original frame count.
#[derive(Debug, Clone, PartialEq)]
pub struct EnhancedLane {
    pub lane_id: String,
    /// Enhancer model id that produced `wav` (e.g. `zipenhancer-webgpu-2026-10-09-r1`).
    pub model_id: String,
    pub wav: Bytes,
}

/// Word alignment request for one clip. Providers advertise their own size/language limits.
#[derive(Debug, Clone, PartialEq)]
pub struct AlignRequest {
    /// Stable clip/task namespace supplied by the caller.
    pub task_id: String,
    /// Unique dispatch identity, preserved in provider receipts.
    pub request_id: String,
    /// Target-speaker lane.
    pub lane: String,
    /// BCP-47 language of the task, e.g. `ru`.
    pub language: String,
    /// The clip as PCM16 mono RIFF WAV.
    pub wav: Bytes,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlignResponse {
    /// Raw clip-relative timed words; callers validate them against the clip.
    pub tokens: Vec<TimingToken>,
    /// Local: model-reported clip duration (checked against the PCM); remote: `None`.
    pub duration_seconds: Option<f64>,
    /// Model release that produced the words (receipt).
    pub model: String,
}

/// Observational progress; never affects the operation's outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum InferenceProgress {
    /// Waiting in a remote queue; `position` 0 = running, `count` = requests queued.
    Queued { position: u32, count: u32 },
    /// A pipeline stage (`LocalInferenceProgress.phase` / enhancement phase).
    Stage {
        stage: String,
        completed: u64,
        total: Option<u64>,
        detail: Option<String>,
        /// 1-based row (`LocalInferenceProgress.clip`) of per-row phases such as `row-llm`.
        clip: Option<u64>,
    },
    /// Model download bytes.
    Download { bytes: u64, total: u64 },
}

/// Progress callback; cheap to clone, must not block.
#[derive(Clone)]
pub struct ProgressSink(Arc<dyn Fn(InferenceProgress) + Send + Sync>);

impl ProgressSink {
    pub fn new(callback: impl Fn(InferenceProgress) + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }

    /// A sink that discards progress.
    pub fn none() -> Self {
        Self::new(|_| {})
    }

    pub fn emit(&self, progress: InferenceProgress) {
        (self.0)(progress);
    }
}

impl fmt::Debug for ProgressSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProgressSink")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InferenceErrorKind {
    /// No server/volunteer can run the operation now (503 `no-llm-drafting-capacity`, draft capability off).
    NoCapacity,
    /// The server serves another model release (426, or a different `release` in capabilities/results).
    ReleaseMismatch,
    /// Local models are not installed or outdated.
    NeedsModels,
    /// Local WebGPU (with shader-f16) is unavailable.
    NoWebGpu,
    /// The backend does not offer this operation at all.
    Unsupported,
    Cancelled,
    /// Server address invalid, unreachable, not ready, malformed result, HTTP failure.
    Other,
}

/// User-facing message plus a kind the UI branches on.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct InferenceError {
    pub kind: InferenceErrorKind,
    pub message: String,
}

impl InferenceError {
    pub fn new(kind: InferenceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(InferenceErrorKind::Other, message)
    }
}

pub type InferenceResult<T> = Result<T, InferenceError>;

/// Which operations the backend can run right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationAvailability {
    pub timing: bool,
    pub draft: bool,
    pub enhance: bool,
    pub align: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum BackendStatus {
    /// Usable for the listed operations with model release `release`.
    #[serde(rename_all = "camelCase")]
    Ready {
        release: String,
        operations: OperationAvailability,
    },
    /// Local models must be (re)installed; `outdated` when an older release is present.
    #[serde(rename_all = "camelCase")]
    NeedsModels {
        release: String,
        download_bytes: u64,
        outdated: bool,
    },
    /// Not usable; `kind`/`message` explain why (no WebGPU, release mismatch, unreachable…).
    Unavailable {
        kind: InferenceErrorKind,
        message: String,
    },
}
