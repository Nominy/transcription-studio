//! Gold inference for Studio: one [`InferenceBackend`] trait implemented by the local
//! (Gold host page in WebView2, crate `studio-inference`) and remote (coordinator) backends.

mod remote;
mod types;

pub use remote::{
    EnhancementModel, GOLD_E2E_RELEASE, GOLD_ENHANCEMENT_MODEL, GOLD_REMOTE_RELEASE, RemoteBackend,
    RemoteConfig, TimingTokenStore,
};
pub use types::*;

use async_trait::async_trait;

// async_trait marks its boxed-future methods `#[must_use]`; the lint fires on the macro output.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait InferenceBackend: Send + Sync {
    /// Readiness and per-operation availability; never fails, unusable states are `Unavailable`.
    async fn status(&self) -> BackendStatus;

    /// Word timings + segments for the task's lanes (1–2 Original WAVs).
    async fn timing(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        progress: ProgressSink,
    ) -> InferenceResult<TimingResponse>;

    /// LLM row text from `timing`. `preserve_rows` keeps those rows' bounds and drafts only
    /// their text (single-segment transcription); the response then holds those row ids.
    async fn draft(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        timing: &TimingResponse,
        preserve_rows: Option<Vec<PreservedRow>>,
        progress: ProgressSink,
    ) -> InferenceResult<DraftResponse>;

    /// Denoised lanes (exactly two Original lanes in, same order out).
    async fn enhance(
        &self,
        task_id: &str,
        lanes: &[LaneAudio],
        progress: ProgressSink,
    ) -> InferenceResult<Vec<EnhancedLane>>;

    /// Error-task clip ASR with word timings.
    async fn align(
        &self,
        request: AlignRequest,
        progress: ProgressSink,
    ) -> InferenceResult<AlignResponse>;
}
