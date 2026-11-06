//! Neutral audio, transcription and alignment provider contracts.
//! Implementations and runtime lifecycle belong to extensions.
mod types;

pub use types::*;

use async_trait::async_trait;

// async_trait marks its boxed-future methods `#[must_use]`; the lint fires on the macro output.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait InferenceBackend: Send + Sync {
    /// Readiness and per-operation availability; never fails, unusable states are `Unavailable`.
    async fn status(&self) -> BackendStatus;

    /// Word timings and speech segments for source lanes.
    async fn timing(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        progress: ProgressSink,
    ) -> InferenceResult<TimingResponse>;

    /// Draft text from `timing`. `preserve_rows` retains those rows' bounds and identifiers.
    async fn draft(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        timing: &TimingResponse,
        preserve_rows: Option<Vec<PreservedRow>>,
        progress: ProgressSink,
    ) -> InferenceResult<DraftResponse>;

    /// Processed lanes, preserving input identity and order.
    async fn enhance(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        progress: ProgressSink,
    ) -> InferenceResult<Vec<EnhancedLane>>;

    /// Clip transcription/alignment with word timings.
    async fn align(
        &self,
        request: AlignRequest,
        progress: ProgressSink,
    ) -> InferenceResult<AlignResponse>;
}
