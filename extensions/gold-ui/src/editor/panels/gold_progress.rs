//! Gold's overall progress (`gold-drafting-extension/src/core/overall-progress.ts`): one bar per
//! operation that never goes back, sized by each stage's typical share, and 100% only once the
//! operation completes. Pipeline events (`LocalInferenceProgress`) are placed by lane and row
//! so long inner steps (frontend frames, kernel audits) cannot jump the bar.

use std::collections::HashMap;

use sonus_gold::bundle::{DOWNLOAD_STAGE, EXTRACT_STAGE};
use studio_inference::InferenceProgress;

/// Studio's first local call installs Gold's runtime before any model work.
fn is_runtime_install(phase: &str) -> bool {
    phase == DOWNLOAD_STAGE || phase == EXTRACT_STAGE
}

/// An unknown or smaller next value holds the previous one.
pub fn hold(previous: Option<f32>, next: Option<f32>) -> Option<f32> {
    let Some(next) = next.filter(|next| next.is_finite()) else {
        return previous;
    };
    let next = next.clamp(0.0, 1.0);
    Some(previous.map_or(next, |previous| previous.max(next)))
}

/// Whole percent; 100 only for a completed operation.
pub fn percent(fraction: f32) -> u32 {
    if fraction >= 1.0 {
        100
    } else {
        (fraction * 100.0).round().clamp(0.0, 99.0) as u32
    }
}

/// "Action… 45% · step"; without a value "Action… step".
pub fn label(action: &str, fraction: Option<f32>, step: &str) -> String {
    let mut line = format!("{action}…");
    if let Some(fraction) = fraction {
        line.push_str(&format!(" {}%", percent(fraction)));
        if !step.is_empty() {
            line.push_str(" · ");
            line.push_str(step);
        }
    } else if !step.is_empty() {
        line.push(' ');
        line.push_str(step);
    }
    line
}

/// Model loading/queueing; determinate only for a server-side bundle download.
const TIMING_SETUP_SHARE: f32 = 0.05;
/// The speaker lanes; the final 1% (assembling the result) reports no counts.
const TIMING_LANES_SHARE: f32 = 0.94;

#[derive(Debug, Clone, Copy, PartialEq)]
struct LaneShares {
    encode: f32,
    segment: f32,
    text: f32,
}

const TIMING_LANE_SHARES: LaneShares = LaneShares {
    encode: 0.5,
    segment: 0.3,
    text: 0.2,
};
/// LLM draft text that first repeats the encoder pass and segmentation.
const DRAFT_TEXT_REENCODE_SHARES: LaneShares = LaneShares {
    encode: 0.15,
    segment: 0.1,
    text: 0.75,
};
/// LLM draft text on the timing run's retained encoder state: row text only.
const DRAFT_TEXT_RETAINED_SHARES: LaneShares = LaneShares {
    encode: 0.0,
    segment: 0.0,
    text: 1.0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaneStage {
    Encode,
    Segment,
    Text,
}

fn lane_stage(phase: &str) -> Option<LaneStage> {
    match phase {
        "decoding-audio" | "shared-encoder" => Some(LaneStage::Encode),
        "learned-rows" | "edge-mel" | "edge-statistics" | "learned-edges" => {
            Some(LaneStage::Segment)
        }
        "row-text" | "row-llm" | "row-text-decode" => Some(LaneStage::Text),
        _ => None,
    }
}

/// `[start, share]` of a segmentation sub-phase within the segmentation stage.
fn segment_phase(phase: &str) -> (f32, f32) {
    match phase {
        "learned-rows" => (0.0, 0.35),
        "edge-mel" => (0.35, 0.4),
        "edge-statistics" => (0.75, 0.05),
        _ => (0.8, 0.2),
    }
}

/// One pipeline event as the bar reads it.
struct Event<'a> {
    phase: &'a str,
    completed: u64,
    total: u64,
    lane: &'a str,
    clip: Option<u64>,
}

impl<'a> Event<'a> {
    fn of(progress: &'a InferenceProgress) -> Option<Self> {
        match progress {
            InferenceProgress::Stage {
                stage,
                completed,
                total,
                detail,
                clip,
            } => Some(Self {
                phase: stage,
                completed: *completed,
                total: total.unwrap_or(0),
                lane: detail.as_deref().unwrap_or(""),
                clip: *clip,
            }),
            // Bundle bytes behind a run are its model loading.
            InferenceProgress::Download { bytes, total } => Some(Self {
                phase: "loading-model",
                completed: *bytes,
                total: *total,
                lane: "",
                clip: None,
            }),
            InferenceProgress::Queued { .. } => None,
        }
    }

    fn ratio(&self) -> f32 {
        if self.total > 0 {
            (self.completed as f32 / self.total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

fn speaker(action: &str, index: usize, count: usize) -> String {
    if count > 1 {
        format!("{action} speaker {}", index + 1)
    } else {
        action.to_string()
    }
}

/// Lanes in the order the pipeline first reported them, and each lane's learned row count.
#[derive(Debug, Clone, Default)]
struct LaneOrder {
    lanes: Vec<String>,
    row_totals: HashMap<String, u64>,
}

impl LaneOrder {
    /// Share (0..1) of all lane work done at one event: every encoder pass, then each lane's
    /// segmentation and text in turn.
    fn work(
        &mut self,
        event: &Event,
        stage: LaneStage,
        lane_count: usize,
        shares: LaneShares,
        text_action: &str,
    ) -> (f32, String) {
        let index = match self.lanes.iter().position(|lane| lane == event.lane) {
            Some(index) => index,
            None => {
                self.lanes.push(event.lane.to_string());
                self.lanes.len() - 1
            }
        };
        let count = lane_count.max(self.lanes.len()).max(1);
        let (index_f, count_f) = (index as f32, count as f32);
        let (rows_part, step) = match stage {
            LaneStage::Encode => {
                let part = if event.phase == "shared-encoder" {
                    event.ratio()
                } else {
                    0.0
                };
                return (
                    shares.encode * (index_f + part) / count_f,
                    speaker("encoding", index, count),
                );
            }
            LaneStage::Segment => {
                let (start, share) = segment_phase(event.phase);
                (
                    shares.segment * (start + share * event.ratio()),
                    speaker("segmenting", index, count),
                )
            }
            LaneStage::Text => {
                let text = if event.phase == "row-text" {
                    if event.total > 0 {
                        self.row_totals.insert(event.lane.to_string(), event.total);
                    }
                    event.ratio()
                } else {
                    // Per-row decoding (clip = 1-based row), placed once the row count is known.
                    let placed = match (self.row_totals.get(event.lane), event.clip) {
                        (Some(&rows), Some(clip)) if rows > 0 && clip >= 1 => {
                            ((clip - 1) as f32 + event.ratio()) / rows as f32
                        }
                        _ => 0.0,
                    };
                    placed.min(1.0)
                };
                (
                    shares.segment + shares.text * text,
                    speaker(text_action, index, count),
                )
            }
        };
        (
            shares.encode + ((shares.segment + shares.text) * index_f + rows_part) / count_f,
            step,
        )
    }
}

/// Overall progress of one timing run (Gold `advanceL0TimingOverall`).
#[derive(Debug, Clone)]
pub struct TimingOverall {
    lane_count: usize,
    order: LaneOrder,
    pub fraction: Option<f32>,
    pub step: String,
}

impl TimingOverall {
    /// `lane_count`: speaker lanes sent to the model (1 or 2).
    pub fn new(lane_count: usize) -> Self {
        Self {
            lane_count,
            order: LaneOrder::default(),
            fraction: None,
            step: String::new(),
        }
    }

    pub fn advance(&mut self, progress: &InferenceProgress) {
        if let InferenceProgress::Queued { position, .. } = progress {
            if *position > 0 && self.fraction.is_none() {
                self.step = format!("queued #{position}");
            }
            return;
        }
        let Some(event) = Event::of(progress) else {
            return;
        };
        let mut next = None;
        if event.phase == "waiting-local-pipeline" {
            if event.completed < event.total {
                self.step = "waiting for the GPU".to_string();
            }
        } else if is_runtime_install(event.phase) {
            self.step = "downloading the Gold runtime".to_string();
        } else if event.phase == "loading-model" {
            self.step = "loading models".to_string();
            if event.total > 0 {
                next = Some(TIMING_SETUP_SHARE * event.ratio());
            }
        } else if event.phase.starts_with("loading-") {
            if self.fraction.is_none() {
                self.step = "loading models".to_string();
            }
        } else if let Some(stage) = lane_stage(event.phase) {
            let (work, step) = self.order.work(
                &event,
                stage,
                self.lane_count,
                TIMING_LANE_SHARES,
                "decoding",
            );
            self.step = step;
            next = Some(TIMING_SETUP_SHARE + TIMING_LANES_SHARE * work);
        }
        self.fraction = hold(self.fraction, next);
    }
}

/// Overall progress of an LLM draft-text pass (Gold `advanceDraftTextOverall`).
#[derive(Debug, Clone)]
pub struct DraftTextOverall {
    lane_count: usize,
    order: LaneOrder,
    reencoding: bool,
    pub fraction: Option<f32>,
    pub step: String,
}

impl DraftTextOverall {
    pub fn new(lane_count: usize) -> Self {
        Self {
            lane_count,
            order: LaneOrder::default(),
            reencoding: false,
            fraction: None,
            step: String::new(),
        }
    }

    pub fn advance(&mut self, progress: &InferenceProgress) {
        if let InferenceProgress::Queued { position, .. } = progress {
            if *position > 0 && self.fraction.is_none() {
                self.step = format!("queued #{position}");
            }
            return;
        }
        let Some(event) = Event::of(progress) else {
            return;
        };
        let stage = lane_stage(event.phase);
        self.reencoding |= matches!(stage, Some(LaneStage::Encode | LaneStage::Segment));
        let mut next = None;
        if event.phase == "waiting-local-pipeline" {
            if event.completed < event.total {
                self.step = "waiting for the GPU".to_string();
            }
        } else if is_runtime_install(event.phase) {
            self.step = "downloading the Gold runtime".to_string();
        } else if event.phase.starts_with("loading-") {
            if self.fraction.is_none() {
                self.step = "loading the text model".to_string();
            }
        } else if let Some(stage) = stage {
            let shares = if self.reencoding {
                DRAFT_TEXT_REENCODE_SHARES
            } else {
                DRAFT_TEXT_RETAINED_SHARES
            };
            let (work, step) =
                self.order
                    .work(&event, stage, self.lane_count, shares, "writing text");
            self.step = step;
            next = Some(work);
        }
        self.fraction = hold(self.fraction, next);
    }
}

/// Draft stages after the user starts a draft, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftStage {
    Timestamps,
    Drafting,
    Writing,
    Refining,
}

/// Which typical stage shares size the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageWeights {
    /// Row text is ready or cheap (remote): the model run behind the timestamps dominates.
    TextReady,
    /// Local LLM text still to write: the draft-text pass dominates.
    WithText,
}

impl StageWeights {
    fn of(self, stage: DraftStage) -> f32 {
        match (self, stage) {
            (StageWeights::TextReady, DraftStage::Timestamps) => 0.55,
            (StageWeights::TextReady, DraftStage::Drafting) => 0.025,
            (StageWeights::TextReady, DraftStage::Writing) => 0.025,
            (StageWeights::TextReady, DraftStage::Refining) => 0.4,
            (StageWeights::WithText, DraftStage::Timestamps) => 0.15,
            (StageWeights::WithText, DraftStage::Drafting) => 0.55,
            (StageWeights::WithText, DraftStage::Writing) => 0.02,
            (StageWeights::WithText, DraftStage::Refining) => 0.28,
        }
    }
}

/// Largest in-progress share of one stage: a stage completes only when the next one does.
const STAGE_IN_PROGRESS_LIMIT: f32 = 0.99;

/// The draft operation's bar across its stages (Gold `draftOverallFraction` + `holdOverall`).
#[derive(Debug, Clone)]
pub struct DraftProgress {
    stages: Vec<DraftStage>,
    weights: StageWeights,
    /// `None` until the first stage reports a value (indeterminate bar).
    pub fraction: Option<f32>,
}

impl DraftProgress {
    pub fn new(stages: Vec<DraftStage>, weights: StageWeights) -> Self {
        Self {
            stages,
            weights,
            fraction: None,
        }
    }

    /// Moves the bar to `stage_fraction` of `stage`; `None` holds it.
    pub fn advance(&mut self, stage: DraftStage, stage_fraction: Option<f32>) {
        let next = stage_fraction.and_then(|inner| self.overall(stage, inner));
        self.fraction = hold(self.fraction, next);
    }

    pub fn complete(&mut self) {
        self.fraction = Some(1.0);
    }

    fn overall(&self, stage: DraftStage, inner: f32) -> Option<f32> {
        let index = self.stages.iter().position(|item| *item == stage)?;
        let weight =
            |items: &[DraftStage]| -> f32 { items.iter().map(|item| self.weights.of(*item)).sum() };
        let total = weight(&self.stages);
        if total <= 0.0 {
            return None;
        }
        let inner = if inner.is_finite() { inner } else { 0.0 };
        let inner = inner.clamp(0.0, STAGE_IN_PROGRESS_LIMIT);
        Some((weight(&self.stages[..index]) + self.weights.of(stage) * inner) / total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(phase: &str, completed: u64, total: u64, lane: &str) -> InferenceProgress {
        InferenceProgress::Stage {
            stage: phase.to_string(),
            completed,
            total: Some(total),
            detail: Some(lane.to_string()),
            clip: None,
        }
    }

    fn row_llm(lane: &str, clip: u64, completed: u64, total: u64) -> InferenceProgress {
        InferenceProgress::Stage {
            stage: "row-llm".to_string(),
            completed,
            total: Some(total),
            detail: Some(lane.to_string()),
            clip: Some(clip),
        }
    }

    /// The reported bug: a per-row frontend pass (`asr-frontend` 0/1741) during the draft-text
    /// pass sent the bar to 97% while the first row was still being written.
    #[test]
    fn inner_row_phases_do_not_jump_the_draft_bar() {
        let mut bar = DraftProgress::new(
            vec![
                DraftStage::Timestamps,
                DraftStage::Drafting,
                DraftStage::Writing,
            ],
            StageWeights::WithText,
        );
        let mut text = DraftTextOverall::new(2);
        bar.advance(DraftStage::Drafting, Some(0.0));
        let start = bar.fraction.unwrap();
        for event in [
            stage("asr-frontend", 1741, 1741, "speaker-1"),
            stage("asr-frontend", 0, 1741, "speaker-1"),
            row_llm("speaker-1", 1, 40, 40),
            stage("loading-text-decoder", 5, 5, ""),
        ] {
            text.advance(&event);
            bar.advance(DraftStage::Drafting, text.fraction.map(|f| 0.95 * f));
        }
        assert_eq!(bar.fraction, Some(start));

        // Row 1 of 10 of the first speaker done, then half of row 2.
        text.advance(&stage("row-text", 1, 10, "speaker-1"));
        bar.advance(DraftStage::Drafting, text.fraction.map(|f| 0.95 * f));
        let after_row = bar.fraction.unwrap();
        text.advance(&row_llm("speaker-1", 2, 20, 40));
        bar.advance(DraftStage::Drafting, text.fraction.map(|f| 0.95 * f));
        let mid_row = bar.fraction.unwrap();
        assert!(start < after_row && after_row < mid_row);
        // Two speakers, retained encoder: one row of ten of speaker 1 is 1/20 of the pass,
        // and a half row 1.5/20; drafting is 0.55 of 0.72 behind 0.15 of timestamps.
        let expect = |share: f32| (0.15 + 0.55 * 0.95 * share) / 0.72;
        assert!((after_row - expect(0.05)).abs() < 1e-4);
        assert!((mid_row - expect(0.075)).abs() < 1e-4);
    }

    #[test]
    fn timing_bar_follows_lanes_and_never_goes_back() {
        let mut timing = TimingOverall::new(2);
        let mut seen = Vec::new();
        for event in [
            stage("loading-model", 1, 2, ""),
            stage("shared-encoder", 2, 4, "a"),
            stage("shared-encoder", 4, 4, "a"),
            stage("shared-encoder", 4, 4, "b"),
            stage("learned-rows", 1, 1, "a"),
            stage("asr-frontend", 0, 900, "a"),
            stage("row-text", 5, 10, "a"),
            stage("row-text", 10, 10, "b"),
        ] {
            timing.advance(&event);
            seen.push(timing.fraction.unwrap());
        }
        let expected = [
            0.025,
            0.05 + 0.94 * 0.125,
            0.05 + 0.94 * 0.25,
            0.05 + 0.94 * 0.5,
            0.05 + 0.94 * (0.5 + 0.105 / 2.0),
            0.05 + 0.94 * (0.5 + 0.105 / 2.0),
            0.05 + 0.94 * (0.5 + 0.4 / 2.0),
            0.99,
        ];
        for (seen, expected) in seen.iter().zip(expected) {
            assert!((seen - expected).abs() < 1e-4, "{seen} vs {expected}");
        }
        assert_eq!(timing.step, "decoding speaker 2");
        assert_eq!(percent(timing.fraction.unwrap()), 99);
    }

    #[test]
    fn stage_shares_hold_until_completion() {
        let mut bar = DraftProgress::new(
            vec![DraftStage::Timestamps, DraftStage::Drafting],
            StageWeights::TextReady,
        );
        assert_eq!(bar.fraction, None);
        bar.advance(DraftStage::Timestamps, Some(1.0));
        assert!((bar.fraction.unwrap() - 0.99 * 0.55 / 0.575).abs() < 1e-5);
        bar.advance(DraftStage::Timestamps, Some(0.2));
        bar.advance(DraftStage::Refining, Some(1.0));
        assert!((bar.fraction.unwrap() - 0.99 * 0.55 / 0.575).abs() < 1e-5);
        bar.advance(DraftStage::Drafting, Some(1.0));
        assert_eq!(percent(bar.fraction.unwrap()), 99);
        bar.complete();
        assert_eq!(percent(bar.fraction.unwrap()), 100);
    }
}
