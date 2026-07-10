use std::collections::BTreeMap;

use studio_core::{Project, Segment};

pub(super) const OVERLAPPING_SEGMENTS_REASON: &str = "Segments cannot overlap.";
pub(super) const DISTANCE_BETWEEN_SEGMENTS_REASON: &str =
    "Segments must be at least 1 second apart.";

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SegmentTimingAnalysis {
    pub adjacent_overlap_issue_count: usize,
    pub has_too_close_next_segment: bool,
    pub overlaps_same_lane: bool,
}

pub(super) fn analyze_project_timing(project: &Project) -> BTreeMap<&str, SegmentTimingAnalysis> {
    let mut analysis = project
        .segments
        .iter()
        .map(|segment| (segment.id.as_str(), SegmentTimingAnalysis::default()))
        .collect::<BTreeMap<_, _>>();
    let mut all_by_lane: BTreeMap<&str, Vec<&Segment>> = BTreeMap::new();

    for segment in &project.segments {
        if !segment.start_seconds.is_nan() && !segment.end_seconds.is_nan() {
            all_by_lane
                .entry(&segment.lane_id)
                .or_default()
                .push(segment);
        }
    }

    for lane_segments in all_by_lane.values_mut() {
        lane_segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
        let mut previous_transcription: Option<&Segment> = None;
        for right in lane_segments.iter().copied().filter(|segment| {
            segment.start_seconds.is_finite()
                && segment.end_seconds.is_finite()
                && is_transcription_segment(segment)
        }) {
            if let Some(left) = previous_transcription {
                if left.end_seconds > right.start_seconds {
                    analysis
                        .entry(&left.id)
                        .or_default()
                        .adjacent_overlap_issue_count += 1;
                    if left.id != right.id {
                        analysis
                            .entry(&right.id)
                            .or_default()
                            .adjacent_overlap_issue_count += 1;
                    }
                }
                if right.start_seconds - left.end_seconds < 1.0 {
                    analysis
                        .entry(&left.id)
                        .or_default()
                        .has_too_close_next_segment = true;
                }
            }
            previous_transcription = Some(right);
        }
        let mut best_ends = Vec::with_capacity(lane_segments.len() + 1);
        let mut best = DistinctEndCandidates::default();
        best_ends.push(best);
        for segment in lane_segments.iter().copied() {
            best.insert(EndCandidate {
                id: &segment.id,
                end_seconds: segment.end_seconds,
            });
            best_ends.push(best);
        }

        for segment in lane_segments.iter().copied() {
            let prefix_len = lane_segments
                .partition_point(|candidate| candidate.start_seconds < segment.end_seconds);
            analysis.entry(&segment.id).or_default().overlaps_same_lane = best_ends[prefix_len]
                .best_excluding(&segment.id)
                .is_some_and(|candidate| candidate.end_seconds > segment.start_seconds);
        }
    }

    analysis
}

#[derive(Clone, Copy, Debug)]
struct EndCandidate<'a> {
    id: &'a str,
    end_seconds: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct DistinctEndCandidates<'a> {
    first: Option<EndCandidate<'a>>,
    second: Option<EndCandidate<'a>>,
}

impl<'a> DistinctEndCandidates<'a> {
    fn insert(&mut self, candidate: EndCandidate<'a>) {
        if self.first.is_some_and(|first| first.id == candidate.id) {
            if self
                .first
                .is_some_and(|first| candidate.end_seconds > first.end_seconds)
            {
                self.first = Some(candidate);
            }
        } else if self.second.is_some_and(|second| second.id == candidate.id) {
            if self
                .second
                .is_some_and(|second| candidate.end_seconds > second.end_seconds)
            {
                self.second = Some(candidate);
            }
        } else if self
            .first
            .is_none_or(|first| candidate.end_seconds > first.end_seconds)
        {
            self.second = self.first;
            self.first = Some(candidate);
        } else if self
            .second
            .is_none_or(|second| candidate.end_seconds > second.end_seconds)
        {
            self.second = Some(candidate);
        }

        if self
            .first
            .zip(self.second)
            .is_some_and(|(first, second)| second.end_seconds > first.end_seconds)
        {
            std::mem::swap(&mut self.first, &mut self.second);
        }
    }

    fn best_excluding(self, id: &str) -> Option<EndCandidate<'a>> {
        self.first
            .filter(|candidate| candidate.id != id)
            .or_else(|| self.second.filter(|candidate| candidate.id != id))
    }
}

fn is_transcription_segment(segment: &Segment) -> bool {
    segment
        .metadata
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("transcription")
        == "transcription"
}
