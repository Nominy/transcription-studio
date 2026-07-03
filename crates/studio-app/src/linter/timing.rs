use studio_core::{Project, Segment};

pub(super) const OVERLAPPING_SEGMENTS_REASON: &str = "Segments cannot overlap.";
pub(super) const DISTANCE_BETWEEN_SEGMENTS_REASON: &str =
    "Segments must be at least 1 second apart.";

pub(super) fn pairwise_overlap_issue_count(project: &Project, segment: &Segment) -> usize {
    sorted_transcription_lane_segments(project, &segment.lane_id)
        .windows(2)
        .filter(|pair| pair[0].end_seconds > pair[1].start_seconds)
        .filter(|pair| pair[0].id == segment.id || pair[1].id == segment.id)
        .count()
}

pub(super) fn overlaps_same_lane(project: &Project, segment: &Segment) -> bool {
    project.segments.iter().any(|other| {
        other.id != segment.id
            && other.lane_id == segment.lane_id
            && segment.start_seconds < other.end_seconds
            && other.start_seconds < segment.end_seconds
    })
}

pub(super) fn has_too_close_next_segment(project: &Project, segment: &Segment) -> bool {
    sorted_transcription_lane_segments(project, &segment.lane_id)
        .windows(2)
        .any(|pair| pair[0].id == segment.id && pair[1].start_seconds - pair[0].end_seconds < 1.0)
}

fn sorted_transcription_lane_segments<'a>(project: &'a Project, lane_id: &str) -> Vec<&'a Segment> {
    let mut lane_segments = project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == lane_id)
        .filter(|segment| segment.start_seconds.is_finite() && segment.end_seconds.is_finite())
        .filter(|segment| is_transcription_segment(segment))
        .collect::<Vec<_>>();
    lane_segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    lane_segments
}

fn is_transcription_segment(segment: &Segment) -> bool {
    segment
        .metadata
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("transcription")
        == "transcription"
}
