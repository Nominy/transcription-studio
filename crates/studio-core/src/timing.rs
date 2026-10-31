//! Word-level timestamps for one audio lane (L0 "timing" in Babel Helper / Gold Drafting).
//!
//! Producers: the remote L0 service (`/v1/timing/lookup`, `/v1/transcribe`) and local
//! GPU inference. Consumers: smart split text allocation, auto segmentation, ghost
//! cursor projection, Alt+click word seek. Times are seconds on the lane's own
//! source clock (the same clock as `Segment::start`/`end`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// All recognised words for one lane, sorted by `start`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneWordTimings {
    pub lane_id: String,
    /// Identity of the audio the words were computed from (recording id / content hash).
    /// A consumer must ignore timings whose source no longer matches the lane's audio.
    pub source_id: String,
    pub words: Vec<TimedWord>,
}

impl LaneWordTimings {
    /// Words whose midpoint lies inside `[start, end)`.
    pub fn words_in(&self, start: f64, end: f64) -> impl Iterator<Item = &TimedWord> {
        self.words.iter().filter(move |word| {
            let middle = (word.start + word.end) / 2.0;
            middle >= start && middle < end
        })
    }
}
