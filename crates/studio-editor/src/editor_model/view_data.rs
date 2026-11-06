use std::collections::BTreeMap;
use std::sync::Arc;

use super::EditorSession;
use crate::timeline_model::{
    LaneColor, OverviewLaneWaveformView, OverviewWaveformBlock, OverviewWaveformView, TimeRange,
    TimelineLaneBuildState, TimelineLaneHeader, TimelineLaneView, TimelineSegmentIndex,
    TimelineSegmentPreview, WaveformBlock, WaveformPeakSource, build_timeline_lane_headers,
    build_timeline_lanes, build_timeline_lanes_with_segment_index,
};
use crate::track_filter::TrackFilter;
use crate::transcript_model::{
    TranscriptActionMenuEntry, TranscriptRowView, TranscriptRowVisibleCells,
    build_transcript_action_menu, build_transcript_rows,
};
use studio_core::Project;

/// Save indicator state for the action strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    Saving,
    /// Last save failed; carries the failure detail when one was reported.
    Failed {
        message: Option<String>,
    },
    Dirty,
    Saved,
}

/// Counts behind the "N errors, M warnings" problem summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProblemCounts {
    pub errors: usize,
    /// Unacknowledged warnings plus unresolved low-confidence words.
    pub warnings: usize,
}

impl ProblemCounts {
    pub fn is_empty(&self) -> bool {
        self.errors == 0 && self.warnings == 0
    }
}

impl EditorSession {
    pub fn save_status(&self) -> SaveStatus {
        if self.is_saving {
            SaveStatus::Saving
        } else if self.save_has_error {
            SaveStatus::Failed {
                message: self
                    .save_error_message
                    .clone()
                    .filter(|message| !message.trim().is_empty()),
            }
        } else if self.is_dirty {
            SaveStatus::Dirty
        } else {
            SaveStatus::Saved
        }
    }

    pub fn problem_counts(&self, project: &Project) -> ProblemCounts {
        ProblemCounts {
            errors: self.blocking_lint_count(project),
            warnings: self.unacknowledged_warning_count(project),
        }
    }

    pub fn track_filter_options(project: &Project) -> Vec<TrackFilter> {
        let mut options = vec![TrackFilter::All];
        options.extend(
            project
                .lanes
                .iter()
                .map(|lane| TrackFilter::lane(lane.id.clone(), lane.label.clone())),
        );
        options
    }

    pub fn timeline_lanes(&self, project: &Project) -> Vec<TimelineLaneView> {
        build_timeline_lanes(
            project,
            self.lane_build_state(self.timeline_bounds(project), None, None),
        )
    }

    /// Headers of the lanes shown under the track filter (no viewport content is built).
    pub fn timeline_lane_headers(&self, project: &Project) -> Vec<TimelineLaneHeader> {
        build_timeline_lane_headers(
            project,
            &self.lane_build_state(self.timeline_bounds(project), None, None),
        )
    }

    pub fn timeline_lanes_with_segment_index(
        &self,
        project: &Project,
        segment_index: &TimelineSegmentIndex,
    ) -> Vec<TimelineLaneView> {
        self.timeline_lanes_with_segment_index_bounds_and_preview(
            project,
            segment_index,
            self.timeline_bounds(project),
            None,
            None,
        )
    }

    /// Lane models for the visible viewport. `strip_blocks` overrides the per-lane block strip
    /// (e.g. blocks cached before a drag); `segment_preview` renders a segment at a dragged time.
    pub fn timeline_lanes_with_segment_index_bounds_and_preview(
        &self,
        project: &Project,
        segment_index: &TimelineSegmentIndex,
        bounds: TimeRange,
        strip_blocks: Option<&BTreeMap<String, Arc<[WaveformBlock]>>>,
        segment_preview: Option<&TimelineSegmentPreview>,
    ) -> Vec<TimelineLaneView> {
        build_timeline_lanes_with_segment_index(
            project,
            segment_index,
            self.lane_build_state(bounds, strip_blocks, segment_preview),
        )
    }

    fn lane_build_state<'a>(
        &'a self,
        bounds: TimeRange,
        strip_blocks: Option<&'a BTreeMap<String, Arc<[WaveformBlock]>>>,
        segment_preview: Option<&'a TimelineSegmentPreview>,
    ) -> TimelineLaneBuildState<'a> {
        TimelineLaneBuildState {
            environment: &self.environment,
            track_filter: &self.track_filter,
            collapsed_lane_ids: &self.collapsed_lane_ids,
            muted_lane_ids: &self.muted_lane_ids,
            lane_vertical_zoom: &self.lane_vertical_zoom,
            lane_waveforms: &self.lane_waveforms,
            lane_display_waveforms: &self.lane_display_waveforms,
            bounds,
            viewport: self.viewport,
            strip_blocks,
            segment_preview,
        }
    }

    pub fn overview_waveform(&self, project: &Project) -> OverviewWaveformView {
        let bounds = self.timeline_bounds(project);
        let duration = bounds.duration_seconds();
        let lane_waveform_models = project
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| self.track_filter.matches_lane(&lane.id))
            .filter_map(|(lane_index, lane)| {
                self.lane_display_waveform(&lane.id)
                    .and_then(|waveform| waveform.overview_render_model(self.playhead_seconds))
                    .map(|waveform_model| OverviewLaneWaveformView {
                        lane_id: lane.id.clone(),
                        color: LaneColor::for_lane_index(lane_index),
                        waveform_model,
                    })
            })
            .collect::<Vec<_>>();
        let waveform_model = lane_waveform_models
            .first()
            .map(|lane| lane.waveform_model.clone());

        OverviewWaveformView {
            peak_source: if !lane_waveform_models.is_empty() {
                WaveformPeakSource::Audio
            } else {
                WaveformPeakSource::Synthetic
            },
            waveform_model,
            lane_waveform_models: lane_waveform_models.into(),
            blocks: overview_blocks(project, &self.track_filter, bounds).into(),
            block_lane_rows: overview_block_lane_rows(project, &self.track_filter),
            viewport_left_percent: percent_of_range(self.viewport.start_seconds, bounds, duration),
            viewport_width_percent: ((self.viewport.duration_seconds() / duration) * 100.0)
                .clamp(0.0, 100.0) as f32,
            playhead_percent: percent_of_range(self.playhead_seconds, bounds, duration),
        }
    }

    /// Rows for the current track filter, with the selection flag applied.
    pub fn transcript_rows(&self, project: &Project) -> Vec<TranscriptRowView> {
        self.transcript_rows_for_filter(project, &self.track_filter)
    }

    /// Rows for the current track filter without selection state (selection-independent cache key).
    pub fn transcript_rows_for_render(&self, project: &Project) -> Vec<TranscriptRowView> {
        build_transcript_rows(
            project,
            &self.track_filter,
            None,
            self.diff_view,
            &self.asserted_warning_keys,
            crate::transcript_model::RowContributions {
                diagnostics: self.diagnostics(),
                decorations: self.decorations(project),
            },
            self.environment.policy.read_only,
        )
    }

    pub fn all_transcript_rows(&self, project: &Project) -> Vec<TranscriptRowView> {
        self.transcript_rows_for_filter(project, &TrackFilter::All)
    }

    pub fn transcript_visible_cells(&self, project: &Project) -> TranscriptRowVisibleCells {
        self.transcript_rows(project)
            .first()
            .map(|row| row.visible_cells)
            .unwrap_or_default()
    }

    pub fn transcript_action_menu(row: &TranscriptRowView) -> Vec<TranscriptActionMenuEntry> {
        build_transcript_action_menu(row)
    }
}

fn overview_blocks(
    project: &Project,
    track_filter: &TrackFilter,
    bounds: TimeRange,
) -> Vec<OverviewWaveformBlock> {
    let duration = bounds.duration_seconds();
    project
        .lanes
        .iter()
        .enumerate()
        .filter(|(_, lane)| track_filter.matches_lane(&lane.id))
        .enumerate()
        .flat_map(|(lane_row, (lane_index, lane))| {
            project
                .segments
                .iter()
                .filter(move |segment| segment.lane_id == lane.id)
                .map(move |segment| OverviewWaveformBlock {
                    color: LaneColor::for_lane_index(lane_index),
                    lane_row,
                    left_percent: percent_of_range(segment.start_seconds, bounds, duration),
                    width_percent: (((segment.end_seconds - segment.start_seconds).max(0.0)
                        / duration)
                        * 100.0)
                        .clamp(0.5, 100.0) as f32,
                })
        })
        .collect()
}

fn overview_block_lane_rows(project: &Project, track_filter: &TrackFilter) -> usize {
    project
        .lanes
        .iter()
        .filter(|lane| track_filter.matches_lane(&lane.id))
        .count()
        .max(1)
}

fn percent_of_range(seconds: f64, bounds: TimeRange, duration: f64) -> f32 {
    (((seconds - bounds.start_seconds) / duration) * 100.0).clamp(0.0, 100.0) as f32
}
