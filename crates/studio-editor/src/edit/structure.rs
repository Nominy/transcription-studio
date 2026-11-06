//! Structural editing rules: split/merge neighbours, waveform region drags (boundary resize,
//! create-by-drag, cut-preview overlay create/move/resize), middle-button autoscroll and
//! audition ranges.

use std::cmp::Ordering;

use studio_core::{Project, Segment};
use studio_waveform::WaveformHitTarget;

use crate::timeline_model::{TimeRange, TimelineSegmentPreview};

/// Regions shorter than this are not created from a waveform drag.
pub const MIN_CREATED_REGION_DURATION_SECONDS: f64 = 0.001;
/// Pointer travel (fraction of the viewport) before a press counts as a drag.
pub const MIN_CREATE_DRAG_DISTANCE_FRACTION: f32 = 0.006;
/// Window auditioned before/after a boundary while Q/W/E/R is held.
pub const AUDITION_BUFFER_SECONDS: f64 = 0.5;
/// Splits must leave at least this much of the segment after the split point.
pub const SPLIT_TAIL_GUARD_SECONDS: f64 = 0.1;
/// Grab distance of a cut-preview edge, in CSS px on either side of it (Helper's
/// `CUT_PREVIEW_HANDLE_HIT_WIDTH`); screen px scale with the interface zoom.
pub const CUT_PREVIEW_HANDLE_HIT_PX: f64 = 12.0;
/// Pointer travel (CSS px) before an Alt-press draws a cut preview (Helper's
/// `CUT_PREVIEW_DRAG_THRESHOLD`).
pub const CUT_PREVIEW_DRAG_THRESHOLD_PX: f64 = 5.0;
/// Minimum cut-preview overlay length (Helper's `CUT_PREVIEW_SAFETY_MIN_SECONDS`), so
/// selections well under a second can be drawn, resized and looped.
pub const CUT_PREVIEW_MIN_DURATION_SECONDS: f64 = 0.008;
/// Pointer travel that turns a held middle-click autoscroll into "drag" mode.
pub const TIMELINE_AUTOSCROLL_HOLD_MOVE_THRESHOLD_FRACTION: f32 = 0.004;
/// Autoscroll distance below which the viewport does not move.
pub const TIMELINE_AUTOSCROLL_DEAD_ZONE_SECONDS: f64 = 0.025;
/// Viewport pan speed per second of pointer offset.
pub const TIMELINE_AUTOSCROLL_SPEED_MULTIPLIER: f64 = 4.0;
/// Segment id of the create-drag preview region.
pub const CREATE_PREVIEW_SEGMENT_ID: &str = "__studio_create_preview__";

/// Splits `text` into two word groups proportional to `ratio`; an interior ratio always
/// leaves at least one word on each side.
pub fn split_text_by_word_ratio(text: &str, ratio: f64) -> (String, String) {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return (String::new(), String::new());
    }
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut split_count = (words.len() as f64 * ratio).round() as usize;
    if ratio > 0.0 && ratio < 1.0 && words.len() > 1 {
        split_count = split_count.clamp(1, words.len() - 1);
    } else {
        split_count = split_count.min(words.len());
    }
    (
        words[..split_count].join(" "),
        words[split_count..].join(" "),
    )
}

/// Why a split point was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SplitError {
    #[error("Split must be inside the segment.")]
    OutsideSegment,
    #[error("Split must be at least 100ms before the segment end.")]
    TooCloseToEnd,
}

/// Validates a split time and returns the fraction of the segment before it.
pub fn split_ratio(segment: &Segment, split_at: f64) -> Result<f64, SplitError> {
    if split_at <= segment.start_seconds || split_at >= segment.end_seconds {
        return Err(SplitError::OutsideSegment);
    }
    if split_at >= segment.end_seconds - SPLIT_TAIL_GUARD_SECONDS {
        return Err(SplitError::TooCloseToEnd);
    }
    let duration = segment.end_seconds - segment.start_seconds;
    Ok(if duration.is_finite() && duration > 0.0 {
        (split_at - segment.start_seconds) / duration
    } else {
        0.0
    })
}

/// Same-speaker, same-lane neighbour that a merge above (`previous`) / below would absorb.
pub fn merge_neighbor_segment_id(
    project: &Project,
    segment_id: &str,
    previous: bool,
) -> Option<String> {
    let segment = project.segment(segment_id)?;
    let candidates = project.segments.iter().filter(|candidate| {
        candidate.id != segment_id
            && candidate.speaker_id == segment.speaker_id
            && candidate.lane_id == segment.lane_id
            && if previous {
                candidate.end_seconds <= segment.start_seconds
            } else {
                segment.end_seconds <= candidate.start_seconds
            }
    });
    let compare = |l: f64, r: f64| l.partial_cmp(&r).unwrap_or(Ordering::Equal);
    if previous {
        candidates.max_by(|l, r| compare(l.end_seconds, r.end_seconds))
    } else {
        candidates.min_by(|l, r| compare(l.start_seconds, r.start_seconds))
    }
    .map(|candidate| candidate.id.clone())
}

/// Cut-preview range spanning `anchor`..`current`, at least [`CUT_PREVIEW_MIN_DURATION_SECONDS`]
/// (or the whole timeline when shorter), clamped to `bounds`.
pub fn anchored_cut_preview_bounds(
    bounds: TimeRange,
    anchor_seconds: f64,
    current_seconds: f64,
) -> (f64, f64) {
    let min_duration = CUT_PREVIEW_MIN_DURATION_SECONDS.min(bounds.duration_seconds().max(0.0));
    let anchor = anchor_seconds.clamp(bounds.start_seconds, bounds.end_seconds);
    let current = if current_seconds.is_finite() {
        current_seconds.clamp(bounds.start_seconds, bounds.end_seconds)
    } else {
        anchor
    };
    if current <= anchor {
        let end = anchor;
        let mut start = current.min(anchor - min_duration).max(bounds.start_seconds);
        if end - start < min_duration {
            start = (end - min_duration).max(bounds.start_seconds);
        }
        return (start, end);
    }
    let start = anchor;
    let mut end = current.max(anchor + min_duration).min(bounds.end_seconds);
    if end - start < min_duration {
        end = (start + min_duration).min(bounds.end_seconds);
    }
    (start, end)
}

/// Part of the cut preview under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutOverlayHit {
    StartHandle,
    EndHandle,
    Body,
}

/// What of the cut preview `range` lies at `time_seconds`: an edge within `handle_seconds`
/// (the nearer one when both are), else the body when inside the range.
pub fn cut_overlay_hit(
    range: TimeRange,
    time_seconds: f64,
    handle_seconds: f64,
) -> Option<CutOverlayHit> {
    if !time_seconds.is_finite() {
        return None;
    }
    let to_start = (time_seconds - range.start_seconds).abs();
    let to_end = (time_seconds - range.end_seconds).abs();
    if to_start.min(to_end) <= handle_seconds {
        // Equidistant (a sliver of an overlay): the side the pointer is on wins.
        let start =
            to_start < to_end || (to_start == to_end && time_seconds <= range.start_seconds);
        return Some(if start {
            CutOverlayHit::StartHandle
        } else {
            CutOverlayHit::EndHandle
        });
    }
    (range.start_seconds..=range.end_seconds)
        .contains(&time_seconds)
        .then_some(CutOverlayHit::Body)
}

/// Held boundary-audition key (Q/W/E/R by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditionKey {
    /// 500ms before the start boundary.
    BeforeStart,
    /// 500ms after the start boundary.
    AfterStart,
    /// 500ms before the end boundary.
    BeforeEnd,
    /// 500ms after the end boundary.
    AfterEnd,
}

impl AuditionKey {
    pub const ALL: [AuditionKey; 4] = [
        AuditionKey::BeforeStart,
        AuditionKey::AfterStart,
        AuditionKey::BeforeEnd,
        AuditionKey::AfterEnd,
    ];
}

/// Range auditioned for `segment` while `key` is held. `None` when empty.
pub fn audition_range(segment: &Segment, key: AuditionKey) -> Option<(f64, f64)> {
    let (start, end) = match key {
        AuditionKey::BeforeStart => (
            (segment.start_seconds - AUDITION_BUFFER_SECONDS).max(0.0),
            segment.start_seconds,
        ),
        AuditionKey::AfterStart => (
            segment.start_seconds,
            (segment.start_seconds + AUDITION_BUFFER_SECONDS).min(segment.end_seconds),
        ),
        AuditionKey::BeforeEnd => (
            (segment.end_seconds - AUDITION_BUFFER_SECONDS).max(segment.start_seconds),
            segment.end_seconds,
        ),
        AuditionKey::AfterEnd => (
            segment.end_seconds,
            segment.end_seconds + AUDITION_BUFFER_SECONDS,
        ),
    };
    (end > start).then_some((start, end))
}

/// Part of the cut-preview overlay being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayDragTarget {
    /// Alt-drag on a lane draws a new overlay there.
    Create,
    /// Secondary-drag inside the overlay moves it.
    Body,
    StartHandle,
    EndHandle,
    /// Plain press inside the overlay body: nothing moves.
    Locked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayDrag {
    pub lane_id: String,
    pub target: OverlayDragTarget,
    pub grab_time_seconds: f64,
    pub original_start_seconds: f64,
    pub original_end_seconds: f64,
    pub has_moved: bool,
}

impl OverlayDrag {
    /// New overlay range for a pointer at `time_seconds`, or `None` when nothing should move.
    pub fn range_at(&self, bounds: TimeRange, time_seconds: f64) -> Option<(f64, f64)> {
        let delta = time_seconds - self.grab_time_seconds;
        if !delta.is_finite() {
            return None;
        }
        match self.target {
            OverlayDragTarget::Locked => None,
            OverlayDragTarget::Create if !self.has_moved => None,
            OverlayDragTarget::Create => Some(anchored_cut_preview_bounds(
                bounds,
                self.grab_time_seconds,
                time_seconds,
            )),
            OverlayDragTarget::StartHandle => Some(anchored_cut_preview_bounds(
                bounds,
                self.original_end_seconds,
                time_seconds,
            )),
            OverlayDragTarget::EndHandle => Some(anchored_cut_preview_bounds(
                bounds,
                self.original_start_seconds,
                time_seconds,
            )),
            OverlayDragTarget::Body => {
                let duration = (self.original_end_seconds - self.original_start_seconds)
                    .max(CUT_PREVIEW_MIN_DURATION_SECONDS)
                    .min(
                        bounds
                            .duration_seconds()
                            .max(CUT_PREVIEW_MIN_DURATION_SECONDS),
                    );
                let mut start = self.original_start_seconds + delta;
                let mut end = self.original_end_seconds + delta;
                if start < bounds.start_seconds {
                    end += bounds.start_seconds - start;
                    start = bounds.start_seconds;
                }
                if end > bounds.end_seconds {
                    start -= end - bounds.end_seconds;
                    end = bounds.end_seconds;
                }
                if end - start < duration {
                    end = (start + duration).min(bounds.end_seconds);
                    start = (end - duration).max(bounds.start_seconds);
                }
                Some((start, end))
            }
        }
    }
}

/// Boundary resize of an existing region by its start/end handle.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveformDrag {
    pub lane_id: String,
    pub segment_id: String,
    pub target: WaveformHitTarget,
    pub grab_time_seconds: f64,
    pub original_start_seconds: f64,
    pub original_end_seconds: f64,
    pub preview_start_seconds: f64,
    pub preview_end_seconds: f64,
    pub has_moved: bool,
}

impl WaveformDrag {
    /// Moves the dragged handle to `time_seconds`; ignores non-finite or inverted ranges.
    pub fn update(&mut self, time_seconds: f64) {
        let (start, end) = match self.target {
            WaveformHitTarget::RegionStartHandle => (time_seconds, self.original_end_seconds),
            WaveformHitTarget::RegionEndHandle => (self.original_start_seconds, time_seconds),
            WaveformHitTarget::RegionBody | WaveformHitTarget::Waveform => return,
        };
        if !start.is_finite() || !end.is_finite() || start >= end {
            return;
        }
        let moved = self.preview_start_seconds.to_bits() != start.to_bits()
            || self.preview_end_seconds.to_bits() != end.to_bits();
        self.preview_start_seconds = start;
        self.preview_end_seconds = end;
        self.has_moved |= moved;
    }

    pub fn preview_differs_from_original(&self) -> bool {
        self.preview_start_seconds.to_bits() != self.original_start_seconds.to_bits()
            || self.preview_end_seconds.to_bits() != self.original_end_seconds.to_bits()
    }

    pub fn timeline_preview(&self) -> TimelineSegmentPreview {
        TimelineSegmentPreview {
            segment_id: self.segment_id.clone(),
            lane_id: self.lane_id.clone(),
            start_seconds: self.preview_start_seconds,
            end_seconds: self.preview_end_seconds,
        }
    }
}

/// Drag on empty waveform that creates a region on release.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveformCreateDrag {
    pub lane_id: String,
    pub start_seconds: f64,
    pub current_seconds: f64,
    pub start_fraction: f32,
    pub current_fraction: f32,
    pub has_moved: bool,
}

impl WaveformCreateDrag {
    /// Normalized (start <= end, non-negative) range of the drag.
    pub fn range(&self) -> (f64, f64) {
        (
            self.start_seconds.min(self.current_seconds).max(0.0),
            self.start_seconds.max(self.current_seconds).max(0.0),
        )
    }

    /// Preview region shown while dragging (after the pointer actually moved).
    pub fn timeline_preview(&self) -> Option<TimelineSegmentPreview> {
        if !self.has_moved {
            return None;
        }
        let (start_seconds, end_seconds) = self.range();
        if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds >= end_seconds {
            return None;
        }
        Some(TimelineSegmentPreview {
            segment_id: CREATE_PREVIEW_SEGMENT_ID.to_string(),
            lane_id: self.lane_id.clone(),
            start_seconds,
            end_seconds,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoscrollMode {
    /// Middle button held: scrolls while held, stops on release (if the pointer moved).
    Held,
    /// Middle click without movement: scrolls until the next middle click / Escape.
    Click,
}

/// Middle-button timeline autoscroll.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineAutoscroll {
    pub lane_id: String,
    pub anchor_fraction: f32,
    pub target_fraction: f32,
    pub mode: AutoscrollMode,
    pub moved_while_held: bool,
}

impl TimelineAutoscroll {
    /// Viewport pan (seconds) for `elapsed_seconds` given the viewport duration.
    pub fn pan_delta_seconds(&self, viewport_duration: f64, elapsed_seconds: f64) -> Option<f64> {
        let distance = (self.target_fraction - self.anchor_fraction) as f64 * viewport_duration;
        if !distance.is_finite() || distance.abs() <= TIMELINE_AUTOSCROLL_DEAD_ZONE_SECONDS {
            return None;
        }
        Some(distance * elapsed_seconds * TIMELINE_AUTOSCROLL_SPEED_MULTIPLIER)
    }
}
