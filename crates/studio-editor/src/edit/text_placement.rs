//! Auto-segmentation text placement (Babel Helper `auto-segmentation.ts`): the visible rows'
//! text is captured in groups before any structural change, then each group's *original* text
//! is redistributed over the rows that now cover it — through the lane's word timings when
//! present, else by whole sentences and duration. Never ASR text, never added or dropped words.

use studio_core::{Segment, SegmentUpdate};

use crate::timing::{
    AllocationError, AllocationSegment, TextGroup, WordTimings, normalize_text,
    sentence_allocations, timed_allocations,
};

/// Consecutive same-lane rows closer than this share one captured text.
pub const AUTO_SEGMENT_MERGE_GAP_SECONDS: f64 = 1.0;
/// Rows reaching this far past a group's range still belong to it.
pub const AUTO_SEGMENT_SPLIT_EDGE_GUARD_SECONDS: f64 = 0.05;

/// Captured text of consecutive same-lane rows.
#[derive(Debug, Clone, PartialEq)]
pub struct BaselineGroup {
    pub lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub full_text: String,
}

/// Groups rows in transcript order (`segments` sorted by start): a new group starts at another
/// lane or a gap over [`AUTO_SEGMENT_MERGE_GAP_SECONDS`]. Groups without text are dropped.
pub fn baseline_groups(segments: &[&Segment]) -> Vec<BaselineGroup> {
    let mut groups: Vec<(BaselineGroup, Vec<String>)> = Vec::new();
    for segment in segments {
        let joins = groups.last().is_some_and(|(group, _)| {
            let gap = segment.start_seconds - group.end_seconds;
            group.lane_id == segment.lane_id
                && gap.is_finite()
                && gap <= AUTO_SEGMENT_MERGE_GAP_SECONDS
        });
        if !joins {
            groups.push((
                BaselineGroup {
                    lane_id: segment.lane_id.clone(),
                    start_seconds: segment.start_seconds,
                    end_seconds: segment.end_seconds,
                    full_text: String::new(),
                },
                Vec::new(),
            ));
        }
        let (group, texts) = groups.last_mut().expect("a group was just ensured");
        group.start_seconds = group.start_seconds.min(segment.start_seconds);
        group.end_seconds = group.end_seconds.max(segment.end_seconds);
        let text = normalize_text(&segment.text);
        if !text.is_empty() {
            texts.push(text);
        }
    }
    groups
        .into_iter()
        .filter_map(|(mut group, texts)| {
            group.full_text = texts.join(" ");
            (!group.full_text.is_empty()).then_some(group)
        })
        .collect()
}

/// The rows' normalized texts joined as a baseline group joins them.
fn joined_text(rows: &[&Segment]) -> String {
    rows.iter()
        .map(|segment| normalize_text(&segment.text))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// How one group's text was placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementSource {
    WordTimings,
    Sentences,
}

/// Placement of every baseline group over the current rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextPlacement {
    /// Row text changes (only rows whose text differs).
    pub updates: Vec<SegmentUpdate>,
    pub timed_groups: usize,
    pub sentence_groups: usize,
    /// Groups left as they were, with the reason.
    pub failed: Vec<(String, AllocationError)>,
}

/// Redistributes each baseline group's text over `current` rows (sorted by start) of its lane
/// inside its range. A group with no rows left is skipped (Helper); so is a group whose rows no
/// longer hold exactly its text — a removed silent row's text was stitched into them — since
/// placing the captured text there would drop the stitched words.
pub fn place_group_text(
    groups: &[BaselineGroup],
    current: &[&Segment],
    timings: &WordTimings,
) -> TextPlacement {
    let mut placement = TextPlacement::default();
    for baseline in groups {
        let rows: Vec<&Segment> = current
            .iter()
            .copied()
            .filter(|segment| {
                segment.lane_id == baseline.lane_id
                    && segment.end_seconds
                        > baseline.start_seconds - AUTO_SEGMENT_SPLIT_EDGE_GUARD_SECONDS
                    && segment.start_seconds
                        < baseline.end_seconds + AUTO_SEGMENT_SPLIT_EDGE_GUARD_SECONDS
            })
            .collect();
        if rows.is_empty() || joined_text(&rows) != baseline.full_text {
            continue;
        }
        let group = TextGroup {
            lane_id: baseline.lane_id.clone(),
            full_text: baseline.full_text.clone(),
            segments: rows
                .iter()
                .map(|segment| AllocationSegment {
                    id: segment.id.clone(),
                    start_seconds: segment.start_seconds,
                    end_seconds: segment.end_seconds,
                })
                .collect(),
        };
        let (result, source) = match timings.lane(&baseline.lane_id) {
            Some(lane) => (
                timed_allocations(&group, lane),
                PlacementSource::WordTimings,
            ),
            None => (sentence_allocations(&group), PlacementSource::Sentences),
        };
        match result {
            Ok(allocations) => {
                match source {
                    PlacementSource::WordTimings => placement.timed_groups += 1,
                    PlacementSource::Sentences => placement.sentence_groups += 1,
                }
                for (segment, text) in rows.iter().zip(allocations) {
                    if segment.text != text {
                        placement.updates.push(SegmentUpdate {
                            segment_id: segment.id.clone(),
                            start_seconds: segment.start_seconds,
                            end_seconds: segment.end_seconds,
                            text,
                        });
                    }
                }
            }
            Err(error) => placement.failed.push((baseline.lane_id.clone(), error)),
        }
    }
    placement
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use studio_core::timing::{LaneTimings, TimingToken};

    fn seg(id: &str, lane: &str, start: f64, end: f64, text: &str) -> Segment {
        Segment {
            id: id.into(),
            source_annotation_id: None,
            speaker_id: "s".into(),
            lane_id: lane.into(),
            start_seconds: start,
            end_seconds: end,
            text: text.into(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn groups_break_on_lane_changes_and_long_gaps() {
        let rows = [
            seg("a", "l1", 0.0, 1.0, "one"),
            seg("b", "l1", 1.5, 2.0, "two"),
            seg("c", "l2", 2.1, 3.0, "other"),
            seg("d", "l1", 3.1, 4.0, "three"),
            seg("e", "l1", 6.0, 7.0, ""),
        ];
        let refs: Vec<&Segment> = rows.iter().collect();
        let groups = baseline_groups(&refs);
        let summary: Vec<(&str, &str)> = groups
            .iter()
            .map(|group| (group.lane_id.as_str(), group.full_text.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![("l1", "one two"), ("l2", "other"), ("l1", "three")]
        );
    }

    #[test]
    fn placement_moves_original_text_with_timings_and_never_loses_words() {
        let before = [seg("a", "l1", 0.0, 4.0, "раз два три четыре")];
        let refs: Vec<&Segment> = before.iter().collect();
        let groups = baseline_groups(&refs);
        // After a split at 2.0 both halves still hold ratio text.
        let after = [
            seg("a", "l1", 0.0, 1.9, "раз два"),
            seg("a2", "l1", 2.1, 4.0, "три четыре"),
        ];
        let current: Vec<&Segment> = after.iter().collect();
        let mut timings = WordTimings::default();
        timings.install([LaneTimings {
            lane_id: "l1".into(),
            source_id: "sha".into(),
            tokens: [("раз", 0.1), ("два", 0.5), ("три", 1.0), ("четыре", 2.5)]
                .iter()
                .map(|(text, start)| TimingToken {
                    text: text.to_string(),
                    start_seconds: *start,
                    end_seconds: start + 0.3,
                })
                .collect(),
        }]);
        let placement = place_group_text(&groups, &current, &timings);
        assert_eq!(placement.timed_groups, 1);
        assert_eq!(
            placement
                .updates
                .iter()
                .map(|update| (update.segment_id.as_str(), update.text.as_str()))
                .collect::<Vec<_>>(),
            vec![("a", "раз два три"), ("a2", "четыре")]
        );

        let untimed = place_group_text(&groups, &current, &WordTimings::default());
        assert_eq!(untimed.sentence_groups, 1);
        assert!(untimed.failed.is_empty());
    }
}
