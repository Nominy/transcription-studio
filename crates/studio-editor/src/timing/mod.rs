//! Per-task word-timing store and its consumers' algorithms (Babel Helper parity): smart split
//! allocation, Alt+Click word seek, ghost-cursor interpolation and auto-segmentation text
//! placement.
//!
//! Invalidation follows Helper: timings belong to the task's lane audio. Text edits, splits,
//! merges and boundary moves never invalidate them — every lookup aligns the row's *current*
//! text against the clocks inside the row's *current* range. They are replaced by a newer
//! timing run and dropped with the task's document.

mod align;
mod allocation;
mod text;

use std::collections::BTreeMap;

pub use align::{
    Alignment, PreparedLane, TimeRange, cut_boundary_offset, offset_at_time, timestamp_at_offset,
};
pub use allocation::{
    AllocationError, AllocationSegment, TextGroup, allocations_preserve_text, sentence_allocations,
    smart_split_text, timed_allocations,
};
pub use text::{SplitParts, SplitSource, normalize_text};

use studio_core::timing::LaneTimings;

#[derive(Debug, Clone, PartialEq)]
struct LaneEntry {
    source_id: String,
    lane: PreparedLane,
}

/// Word timings of one task, by lane id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WordTimings {
    lanes: BTreeMap<String, LaneEntry>,
}

impl WordTimings {
    /// Installs (replacing) the timings of the given lanes; other lanes keep theirs.
    pub fn install(&mut self, lanes: impl IntoIterator<Item = LaneTimings>) {
        for timings in lanes {
            self.lanes.insert(
                timings.lane_id,
                LaneEntry {
                    source_id: timings.source_id,
                    lane: PreparedLane::new(&timings.tokens),
                },
            );
        }
    }

    pub fn clear(&mut self) {
        self.lanes.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    /// Lane ids with timings.
    pub fn lane_ids(&self) -> impl Iterator<Item = &str> {
        self.lanes.keys().map(String::as_str)
    }

    /// Prepared clocks of a lane.
    pub fn lane(&self, lane_id: &str) -> Option<&PreparedLane> {
        self.lanes
            .get(lane_id)
            .map(|entry| &entry.lane)
            .filter(|lane| !lane.is_empty())
    }
}
