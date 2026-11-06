use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::timeline_model::{LaneWaveform, TimeRange, WaveformBlock};
use crate::track_filter::TrackFilter;
use studio_core::{AudioLane, Project, Segment};
use studio_waveform::LineWaveform;

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineLaneView {
    pub lane_id: String,
    pub label: String,
    pub speaker_id: String,
    pub color: LaneColor,
    pub is_collapsed: bool,
    pub is_muted: bool,
    pub vertical_zoom: f32,
    pub track_height: f32,
    /// Babel's line waveform of the lane's (display) audio, once decoded.
    pub waveform: Option<Arc<LineWaveform>>,
    pub blocks: Arc<[WaveformBlock]>,
    pub strip_blocks: Arc<[WaveformBlock]>,
    pub scrollbar: TimelineScrollbarView,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineSegmentPreview {
    pub segment_id: String,
    pub lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// The viewport-independent part of a timeline lane: what its header shows.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineLaneHeader {
    pub lane_id: String,
    pub label: String,
    pub color: LaneColor,
    pub is_collapsed: bool,
    pub is_muted: bool,
    pub vertical_zoom: f32,
}

/// Lane accent color; lanes alternate by project lane index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneColor {
    Blue,
    Purple,
}

impl LaneColor {
    pub fn for_lane_index(index: usize) -> Self {
        if index.is_multiple_of(3) {
            Self::Blue
        } else {
            Self::Purple
        }
    }

    /// `0xRRGGBB`.
    pub fn rgb(self) -> u32 {
        match self {
            Self::Blue => 0x64b5f6,
            Self::Purple => 0xb083ff,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineScrollbarView {
    pub is_visible: bool,
    pub left_percent: f32,
    pub width_percent: f32,
}

pub(crate) const DEFAULT_TRACK_HEIGHT: f32 = 165.0;
pub(crate) const COLLAPSED_TRACK_HEIGHT: f32 = 52.0;
pub(crate) const DEFAULT_VERTICAL_ZOOM: f32 = 20.0;
pub(crate) const MIN_VERTICAL_ZOOM: f32 = 0.3;
pub(crate) const MAX_VERTICAL_ZOOM: f32 = 1000.0;

pub(crate) struct TimelineLaneBuildState<'a> {
    pub(crate) environment: &'a exaudio_extension::document::DocumentEnvironment,
    pub(crate) track_filter: &'a TrackFilter,
    pub(crate) collapsed_lane_ids: &'a BTreeSet<String>,
    pub(crate) muted_lane_ids: &'a BTreeSet<String>,
    pub(crate) lane_vertical_zoom: &'a BTreeMap<String, f32>,
    pub(crate) lane_waveforms: &'a BTreeMap<String, LaneWaveform>,
    pub(crate) lane_display_waveforms: &'a BTreeMap<String, LaneWaveform>,
    pub(crate) bounds: TimeRange,
    pub(crate) viewport: TimeRange,
    pub(crate) strip_blocks: Option<&'a BTreeMap<String, Arc<[WaveformBlock]>>>,
    pub(crate) segment_preview: Option<&'a TimelineSegmentPreview>,
}

/// The waveform painted for a lane: its display override, else its original.
pub(crate) fn display_waveform<'a>(
    display: &'a BTreeMap<String, LaneWaveform>,
    original: &'a BTreeMap<String, LaneWaveform>,
    lane_id: &str,
) -> Option<&'a LaneWaveform> {
    display.get(lane_id).or_else(|| original.get(lane_id))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineSegmentIndex {
    segments_by_lane: BTreeMap<String, TimelineLaneSegmentIndex>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct TimelineLaneSegmentIndex {
    indices_by_start: Arc<[usize]>,
    prefix_max_end_seconds: Arc<[f64]>,
}

impl TimelineSegmentIndex {
    pub fn for_project(project: &Project) -> Self {
        let mut segments_by_lane = BTreeMap::new();
        for (index, segment) in project.segments.iter().enumerate() {
            segments_by_lane
                .entry(segment.lane_id.clone())
                .or_insert_with(Vec::new)
                .push(index);
        }
        for segments in segments_by_lane.values_mut() {
            segments.sort_by(|left, right| {
                project.segments[*left]
                    .start_seconds
                    .total_cmp(&project.segments[*right].start_seconds)
            });
        }

        Self {
            segments_by_lane: segments_by_lane
                .into_iter()
                .map(|(lane_id, segments)| {
                    let prefix_max_end_seconds = prefix_max_end_seconds(project, &segments);
                    (
                        lane_id,
                        TimelineLaneSegmentIndex {
                            indices_by_start: Arc::from(segments),
                            prefix_max_end_seconds: Arc::from(prefix_max_end_seconds),
                        },
                    )
                })
                .collect(),
        }
    }

    fn candidate_segment_indices_for_lane(
        &self,
        project: &Project,
        lane_id: &str,
        viewport: TimeRange,
    ) -> &[usize] {
        let Some(index) = self.segments_by_lane.get(lane_id) else {
            return &[];
        };
        let indices = index.indices_by_start.as_ref();
        let prefix_max_end_seconds = index.prefix_max_end_seconds.as_ref();

        let candidate_start = prefix_max_end_seconds
            .partition_point(|end_seconds| *end_seconds <= viewport.start_seconds);
        let candidate_end = indices.partition_point(|segment_index| {
            project
                .segments
                .get(*segment_index)
                .is_some_and(|segment| segment.start_seconds < viewport.end_seconds)
        });

        if candidate_start >= candidate_end {
            &[]
        } else {
            &indices[candidate_start..candidate_end]
        }
    }
}

pub(crate) fn build_timeline_lanes(
    project: &Project,
    state: TimelineLaneBuildState<'_>,
) -> Vec<TimelineLaneView> {
    let segment_index = TimelineSegmentIndex::for_project(project);
    build_timeline_lanes_with_segment_index(project, &segment_index, state)
}

/// Headers of the lanes shown under the track filter, in timeline order.
pub(crate) fn build_timeline_lane_headers(
    project: &Project,
    state: &TimelineLaneBuildState<'_>,
) -> Vec<TimelineLaneHeader> {
    project
        .lanes
        .iter()
        .enumerate()
        .filter(|(_, lane)| state.track_filter.matches_lane(&lane.id))
        .map(|(index, lane)| lane_header(index, lane, state))
        .collect()
}

fn lane_header(
    index: usize,
    lane: &AudioLane,
    state: &TimelineLaneBuildState<'_>,
) -> TimelineLaneHeader {
    TimelineLaneHeader {
        lane_id: lane.id.clone(),
        label: lane.label.clone(),
        color: LaneColor::for_lane_index(index),
        is_collapsed: state.collapsed_lane_ids.contains(&lane.id),
        is_muted: state.muted_lane_ids.contains(&lane.id),
        vertical_zoom: state
            .lane_vertical_zoom
            .get(&lane.id)
            .copied()
            .unwrap_or(DEFAULT_VERTICAL_ZOOM),
    }
}

pub(crate) fn build_timeline_lanes_with_segment_index(
    project: &Project,
    segment_index: &TimelineSegmentIndex,
    state: TimelineLaneBuildState<'_>,
) -> Vec<TimelineLaneView> {
    let scrollbar = timeline_scrollbar_view(state.bounds, state.viewport);
    let indexed_lanes: Vec<_> = project
        .lanes
        .iter()
        .enumerate()
        .filter(|(_, lane)| state.track_filter.matches_lane(&lane.id))
        .collect();

    indexed_lanes
        .into_iter()
        .map(|(index, lane)| {
            let mut visible_segments: Vec<(Cow<'_, Segment>, Cow<'_, Segment>)> = segment_index
                .candidate_segment_indices_for_lane(project, lane.id.as_str(), state.viewport)
                .iter()
                .filter_map(|segment_index| project.segments.get(*segment_index))
                .map(|segment| {
                    (
                        Cow::Borrowed(segment),
                        segment_with_preview(segment, lane.id.as_str(), state.segment_preview),
                    )
                })
                .filter(|(_, segment)| state.viewport.intersects(segment.as_ref()))
                .collect::<Vec<_>>();
            if let Some(preview_segment) =
                create_preview_segment_for_lane(lane, state.segment_preview, &visible_segments)
            {
                visible_segments.push((
                    Cow::Owned(preview_segment.clone()),
                    Cow::Owned(preview_segment),
                ));
                visible_segments.sort_by(|left, right| {
                    left.1
                        .start_seconds
                        .total_cmp(&right.1.start_seconds)
                        .then_with(|| left.1.id.cmp(&right.1.id))
                });
            }
            let lane_waveform =
                display_waveform(state.lane_display_waveforms, state.lane_waveforms, &lane.id);
            let waveform = lane_waveform.map(LaneWaveform::line);
            let header = lane_header(index, lane, &state);

            let blocks = visible_segments
                .iter()
                .map(|(static_source_segment, segment)| {
                    WaveformBlock::for_segment_with_static_source(
                        segment.as_ref(),
                        static_source_segment,
                        state.viewport,
                        lane_waveform,
                        state.environment.is_text_segment(segment.as_ref()),
                    )
                })
                .collect::<Vec<_>>()
                .into();
            let lane_has_segment_preview = state
                .segment_preview
                .is_some_and(|preview| preview.lane_id == lane.id);
            let strip_blocks = if lane_has_segment_preview {
                Arc::clone(&blocks)
            } else {
                state
                    .strip_blocks
                    .and_then(|strip_blocks| strip_blocks.get(&lane.id))
                    .cloned()
                    .unwrap_or_else(|| Arc::clone(&blocks))
            };

            TimelineLaneView {
                lane_id: header.lane_id,
                label: header.label,
                speaker_id: lane.speaker_id.clone(),
                color: header.color,
                is_collapsed: header.is_collapsed,
                is_muted: header.is_muted,
                vertical_zoom: header.vertical_zoom,
                track_height: if header.is_collapsed {
                    COLLAPSED_TRACK_HEIGHT
                } else {
                    DEFAULT_TRACK_HEIGHT
                },
                waveform,
                blocks,
                strip_blocks,
                scrollbar,
            }
        })
        .collect()
}

fn segment_with_preview<'a>(
    segment: &'a Segment,
    lane_id: &str,
    preview: Option<&TimelineSegmentPreview>,
) -> Cow<'a, Segment> {
    let Some(preview) = preview else {
        return Cow::Borrowed(segment);
    };
    if preview.segment_id != segment.id || preview.lane_id != lane_id {
        return Cow::Borrowed(segment);
    }

    let mut segment = segment.clone();
    segment.start_seconds = preview.start_seconds;
    segment.end_seconds = preview.end_seconds;
    Cow::Owned(segment)
}

fn create_preview_segment_for_lane(
    lane: &AudioLane,
    preview: Option<&TimelineSegmentPreview>,
    visible_segments: &[(Cow<'_, Segment>, Cow<'_, Segment>)],
) -> Option<Segment> {
    let preview = preview?;
    if preview.lane_id != lane.id
        || visible_segments
            .iter()
            .any(|(_, segment)| segment.id == preview.segment_id)
    {
        return None;
    }

    let start_seconds = preview.start_seconds.min(preview.end_seconds).max(0.0);
    let end_seconds = preview.start_seconds.max(preview.end_seconds).max(0.0);
    if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds >= end_seconds {
        return None;
    }

    Some(Segment {
        id: preview.segment_id.clone(),
        source_annotation_id: Some(preview.segment_id.clone()),
        speaker_id: lane.speaker_id.clone(),
        lane_id: lane.id.clone(),
        start_seconds,
        end_seconds,
        text: String::new(),
        metadata: Default::default(),
    })
}

fn prefix_max_end_seconds(project: &Project, segments: &[usize]) -> Vec<f64> {
    let mut max_end_seconds = f64::NEG_INFINITY;
    segments
        .iter()
        .map(|segment_index| {
            let end_seconds = project
                .segments
                .get(*segment_index)
                .map(|segment| segment.end_seconds)
                .filter(|seconds| seconds.is_finite())
                .unwrap_or(f64::NEG_INFINITY);
            max_end_seconds = max_end_seconds.max(end_seconds);
            max_end_seconds
        })
        .collect()
}

fn timeline_scrollbar_view(bounds: TimeRange, viewport: TimeRange) -> TimelineScrollbarView {
    let duration = bounds.duration_seconds();
    let visible_duration = viewport.duration_seconds().min(duration);
    let is_visible = visible_duration < duration;
    if !is_visible {
        return TimelineScrollbarView {
            is_visible: false,
            left_percent: 0.0,
            width_percent: 100.0,
        };
    }

    TimelineScrollbarView {
        is_visible,
        left_percent: (((viewport.start_seconds - bounds.start_seconds) / duration) * 100.0)
            .clamp(0.0, 100.0) as f32,
        width_percent: ((visible_duration / duration) * 100.0).clamp(0.0, 100.0) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::{AudioLane, Segment, Speaker};

    #[test]
    fn segment_index_bounds_late_viewports_without_dropping_long_overlaps() {
        let project = Project {
            id: "project".to_string(),
            name: "Project".to_string(),
            speakers: vec![Speaker {
                id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
            }],
            lanes: vec![AudioLane {
                id: "track-1".to_string(),
                speaker_id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
                source_path: "audio/speaker-1.wav".to_string(),
            }],
            segments: vec![
                test_segment("old", 0.0, 1.0),
                test_segment("long-overlap", 2.0, 12.0),
                test_segment("left-edge", 9.5, 10.2),
                test_segment("inside", 10.4, 10.8),
                test_segment("right-edge", 10.9, 11.5),
                test_segment("future", 12.0, 13.0),
            ],
            metadata: Default::default(),
        };
        let index = TimelineSegmentIndex::for_project(&project);
        let viewport = TimeRange {
            start_seconds: 10.0,
            end_seconds: 11.0,
        };

        let candidate_ids = index
            .candidate_segment_indices_for_lane(&project, "track-1", viewport)
            .iter()
            .filter_map(|segment_index| project.segments.get(*segment_index))
            .filter(|segment| viewport.intersects(segment))
            .map(|segment| segment.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            candidate_ids,
            vec!["long-overlap", "left-edge", "inside", "right-edge"]
        );
        assert!(
            index
                .candidate_segment_indices_for_lane(&project, "track-1", viewport)
                .len()
                < project.segments.len(),
            "late moving viewports should not scan the entire lane"
        );
    }

    fn test_segment(id: &str, start_seconds: f64, end_seconds: f64) -> Segment {
        Segment {
            id: id.to_string(),
            source_annotation_id: Some(id.to_string()),
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-1".to_string(),
            start_seconds,
            end_seconds,
            text: id.to_string(),
            metadata: Default::default(),
        }
    }
}
