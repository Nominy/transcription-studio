//! Word-level timestamps for one audio lane (L0 "timing" in Babel Helper / Gold Drafting).
//!
//! Producers: the Gold inference backends (local WebView2 host or the remote coordinator),
//! converted from their `TimingResponse` tracks. Consumers (in `studio-editor::timing`): smart
//! split text allocation, auto-segmentation text placement, the ghost cursor and Alt+Click word
//! seek. Times are seconds on the lane's own source clock (the same clock as
//! `Segment::start_seconds`/`end_seconds`).

use serde::{Deserialize, Serialize};

/// One recognized token (Gold `L0WordTimingToken`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingToken {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// Every token recognized on one lane, plus the identity of the audio it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneTimings {
    pub lane_id: String,
    /// Identity of the audio the tokens were computed from (Gold's `pcmSha256`). Timings are
    /// dropped when the lane's audio changes.
    pub source_id: String,
    pub tokens: Vec<TimingToken>,
}
