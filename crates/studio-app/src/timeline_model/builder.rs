use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::waveform::WaveformRenderCacheStats;
use crate::timeline_model::{LaneWaveform, TimeRange, WaveformBlock};
use crate::track_filter::TrackFilter;
use studio_core::{AudioLane, Project, Segment};
use studio_waveform::WaveformRenderModel;

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineLaneView {
    pub lane_id: String,
    pub label: String,
    pub speaker_id: String,
    pub color_token: String,
    pub chrome: TimelineTrackChromeView,
    pub viewport_state: TimelineTrackViewportView,
    pub is_collapsed: bool,
    pub is_muted: bool,
    pub vertical_zoom: f32,
    pub track_height: f32,
    pub waveform_model: Option<WaveformRenderModel>,
    pub overlay: Option<TimelineOverlayView>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineTrackChromeView {
    pub color: String,
    pub border_left: String,
    pub border_right: String,
    pub collapse_tooltip: String,
    pub collapse_aria_label: String,
    pub solo_aria_label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineTrackViewportView {
    pub container_display: String,
    pub pointer_events_disabled: bool,
    pub container_height: f32,
    pub waveform_height: f32,
    pub spectrogram_height: Option<f32>,
    pub render_solo_controls: bool,
    pub render_vertical_zoom_slider: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineOverlayView {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub left_percent: f32,
    pub width_percent: f32,
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
const WAVEFORM_CACHE_TRACE_ENV: &str = "STUDIO_TRACE_WAVEFORM_CACHE";

pub(crate) struct TimelineLaneBuildState<'a> {
    pub(crate) track_filter: &'a TrackFilter,
    pub(crate) collapsed_lane_ids: &'a BTreeSet<String>,
    pub(crate) muted_lane_ids: &'a BTreeSet<String>,
    pub(crate) lane_vertical_zoom: &'a BTreeMap<String, f32>,
    pub(crate) lane_waveforms: &'a BTreeMap<String, LaneWaveform>,
    pub(crate) bounds: TimeRange,
    pub(crate) viewport: TimeRange,
    pub(crate) playhead_seconds: f64,
    pub(crate) overlay_range: Option<TimeRange>,
    pub(crate) strip_blocks: Option<&'a BTreeMap<String, Arc<[WaveformBlock]>>>,
    pub(crate) segment_preview: Option<&'a TimelineSegmentPreview>,
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
            let visible_segment_refs = visible_segments
                .iter()
                .map(|(_, segment)| segment.as_ref())
                .collect::<Vec<_>>();

            let lane_waveform = state.lane_waveforms.get(&lane.id);
            let waveform_model = lane_waveform.and_then(|waveform| {
                waveform.render_model(
                    visible_segment_refs.as_slice(),
                    state.viewport,
                    state.playhead_seconds,
                )
            });
            if waveform_cache_trace_enabled()
                && let Some(waveform) = lane_waveform
            {
                eprintln!(
                    "{}",
                    waveform_cache_runtime_trace_line(
                        &lane.id,
                        waveform.render_cache_stats(),
                        waveform.cached_peak_resolution_count(),
                        waveform_model.is_some(),
                        state.viewport,
                        state.playhead_seconds,
                    )
                );
            }
            let is_collapsed = state.collapsed_lane_ids.contains(&lane.id);
            let is_muted = state.muted_lane_ids.contains(&lane.id);
            let color_token = lane_color_token(index);

            let blocks = visible_segments
                .iter()
                .map(|(static_source_segment, segment)| {
                    WaveformBlock::for_segment_with_static_source(
                        segment.as_ref(),
                        static_source_segment,
                        state.viewport,
                        lane_waveform,
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
                lane_id: lane.id.clone(),
                label: lane.label.clone(),
                speaker_id: lane.speaker_id.clone(),
                color_token: color_token.to_string(),
                chrome: track_chrome_state(color_token, is_collapsed, is_muted),
                viewport_state: track_viewport_state(is_collapsed, false, DEFAULT_TRACK_HEIGHT),
                is_collapsed,
                is_muted,
                vertical_zoom: state
                    .lane_vertical_zoom
                    .get(&lane.id)
                    .copied()
                    .unwrap_or(DEFAULT_VERTICAL_ZOOM),
                track_height: if is_collapsed {
                    COLLAPSED_TRACK_HEIGHT
                } else {
                    DEFAULT_TRACK_HEIGHT
                },
                waveform_model,
                overlay: timeline_overlay_view(state.overlay_range, state.viewport),
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

fn lane_color_token(index: usize) -> &'static str {
    match index % 3 {
        0 => "blue",
        _ => "purple",
    }
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

fn track_chrome_state(
    color_token: &str,
    collapsed: bool,
    is_solo: bool,
) -> TimelineTrackChromeView {
    let color = lane_color_hex(color_token).to_string();
    let collapse_label = if collapsed {
        "Show track"
    } else {
        "Hide track"
    };
    TimelineTrackChromeView {
        border_left: format!("2.5px solid {color}"),
        border_right: format!("2.5px solid {color}"),
        color,
        collapse_tooltip: collapse_label.to_string(),
        collapse_aria_label: collapse_label.to_string(),
        solo_aria_label: if is_solo {
            "Unsolo track".to_string()
        } else {
            "Solo track".to_string()
        },
    }
}

fn lane_color_hex(color_token: &str) -> &'static str {
    match color_token {
        "purple" => "#b083ff",
        _ => "#64b5f6",
    }
}

fn waveform_cache_runtime_trace_line(
    lane_id: &str,
    stats: WaveformRenderCacheStats,
    cached_peak_resolutions: usize,
    rendered: bool,
    viewport: TimeRange,
    playhead_seconds: f64,
) -> String {
    format!(
        "Transcription Studio waveform cache: lane={} rendered={} hits={} misses={} peak_resolutions={} viewport={:.3}-{:.3}s playhead={:.3}s",
        lane_id,
        rendered,
        stats.hits,
        stats.misses,
        cached_peak_resolutions,
        viewport.start_seconds,
        viewport.end_seconds,
        playhead_seconds,
    )
}

fn waveform_cache_trace_enabled() -> bool {
    std::env::var_os(WAVEFORM_CACHE_TRACE_ENV).is_some_and(|value| value != "0")
}

fn track_viewport_state(
    collapsed: bool,
    show_spectrogram: bool,
    default_track_height: f32,
) -> TimelineTrackViewportView {
    TimelineTrackViewportView {
        container_display: if collapsed { "hidden" } else { "block" }.to_string(),
        pointer_events_disabled: collapsed,
        container_height: if collapsed {
            0.0
        } else if show_spectrogram {
            default_track_height * 2.0
        } else {
            default_track_height
        },
        waveform_height: default_track_height,
        spectrogram_height: show_spectrogram.then_some(default_track_height),
        render_solo_controls: !collapsed,
        render_vertical_zoom_slider: !collapsed,
    }
}

fn timeline_overlay_view(
    overlay_range: Option<TimeRange>,
    viewport: TimeRange,
) -> Option<TimelineOverlayView> {
    let overlay_range = overlay_range?;
    let visible_start = overlay_range.start_seconds.max(viewport.start_seconds);
    let visible_end = overlay_range.end_seconds.min(viewport.end_seconds);
    if visible_end <= visible_start {
        return None;
    }

    let viewport_duration = viewport.duration_seconds();
    Some(TimelineOverlayView {
        start_seconds: overlay_range.start_seconds,
        end_seconds: overlay_range.end_seconds,
        left_percent: (((visible_start - viewport.start_seconds) / viewport_duration) * 100.0)
            .clamp(0.0, 100.0) as f32,
        width_percent: (((visible_end - visible_start) / viewport_duration) * 100.0)
            .clamp(0.0, 100.0) as f32,
    })
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
    fn waveform_cache_runtime_trace_line_reports_cache_and_viewport() {
        let line = waveform_cache_runtime_trace_line(
            "speaker-1",
            WaveformRenderCacheStats { hits: 7, misses: 2 },
            1,
            true,
            TimeRange {
                start_seconds: 1.25,
                end_seconds: 4.75,
            },
            2.5,
        );

        assert_eq!(
            line,
            "Transcription Studio waveform cache: lane=speaker-1 rendered=true hits=7 misses=2 peak_resolutions=1 viewport=1.250-4.750s playhead=2.500s"
        );
    }

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
