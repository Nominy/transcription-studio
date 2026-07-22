use super::{EditorSession, ReviewActionStripView, SubmitControlView};
use crate::timeline_model::{
    OverviewLaneWaveformView, OverviewWaveformBlock, OverviewWaveformView, TimeRange,
    TimelineLaneBuildState, TimelineLaneView, TimelineSegmentIndex, TimelineSegmentPreview,
    WaveformBlock, WaveformPeakSource, build_timeline_lanes,
    build_timeline_lanes_with_segment_index, lane_color_token,
};
use crate::track_filter::TrackFilter;
use crate::transcript_model::{
    TranscriptActionMenuView, TranscriptEmptyStateView, TranscriptRowView,
    TranscriptRowVisibleCells, build_transcript_action_menu, build_transcript_empty_state,
    build_transcript_rows, derive_transcription_row_visible_cells,
};
use studio_core::Project;

impl EditorSession {
    pub fn support_id_button(&self, copied: bool) -> SupportIdButtonView {
        SupportIdButtonView {
            tooltip_variant: "info".to_string(),
            tooltip_content: format!("ID: {}", self.support_id),
            button_label: if copied {
                "Copied!".to_string()
            } else {
                "Copy Support ID".to_string()
            },
            icon: "copy".to_string(),
            clipboard_text: self.support_id.clone(),
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
        let bounds = self.timeline_bounds(project);
        build_timeline_lanes(
            project,
            TimelineLaneBuildState {
                track_filter: &self.track_filter,
                collapsed_lane_ids: &self.collapsed_lane_ids,
                muted_lane_ids: &self.muted_lane_ids,
                lane_vertical_zoom: &self.lane_vertical_zoom,
                lane_waveforms: &self.lane_waveforms,
                bounds,
                viewport: self.viewport,
                playhead_seconds: self.playhead_seconds,
                overlay_range: self.overlay_range,
                strip_blocks: None,
                segment_preview: None,
            },
        )
    }

    pub fn timeline_lanes_with_segment_index(
        &self,
        project: &Project,
        segment_index: &TimelineSegmentIndex,
    ) -> Vec<TimelineLaneView> {
        self.timeline_lanes_with_segment_index_and_bounds(
            project,
            segment_index,
            self.timeline_bounds(project),
        )
    }

    pub fn timeline_lanes_with_segment_index_and_bounds(
        &self,
        project: &Project,
        segment_index: &TimelineSegmentIndex,
        bounds: TimeRange,
    ) -> Vec<TimelineLaneView> {
        self.timeline_lanes_with_segment_index_bounds_and_preview(
            project,
            segment_index,
            bounds,
            None,
            None,
        )
    }

    pub fn timeline_lanes_with_segment_index_bounds_and_preview(
        &self,
        project: &Project,
        segment_index: &TimelineSegmentIndex,
        bounds: TimeRange,
        strip_blocks: Option<&std::collections::BTreeMap<String, std::sync::Arc<[WaveformBlock]>>>,
        segment_preview: Option<&TimelineSegmentPreview>,
    ) -> Vec<TimelineLaneView> {
        build_timeline_lanes_with_segment_index(
            project,
            segment_index,
            TimelineLaneBuildState {
                track_filter: &self.track_filter,
                collapsed_lane_ids: &self.collapsed_lane_ids,
                muted_lane_ids: &self.muted_lane_ids,
                lane_vertical_zoom: &self.lane_vertical_zoom,
                lane_waveforms: &self.lane_waveforms,
                bounds,
                viewport: self.viewport,
                playhead_seconds: self.playhead_seconds,
                overlay_range: self.overlay_range,
                strip_blocks,
                segment_preview,
            },
        )
    }

    pub fn transport_strip(&self, project: &Project) -> TransportStripView {
        self.transport_strip_with_history(project, false, false)
    }

    pub fn transport_strip_with_history(
        &self,
        project: &Project,
        can_undo: bool,
        can_redo: bool,
    ) -> TransportStripView {
        let submit = self.submit_control(project, false, false, false);
        let save_is_clickable = self.is_dirty || self.save_has_error;
        let save_tooltip_variant = if save_is_clickable {
            SaveTooltipVariant::Warning
        } else {
            SaveTooltipVariant::Default
        };
        TransportStripView {
            track_filter: self.track_filter.clone(),
            track_filter_options: Self::track_filter_options(project),
            play_label: if self.is_playing { "Pause" } else { "Play" }.to_string(),
            problem_summary_label: self.problem_summary_label(project),
            undo_disabled: !can_undo,
            redo_disabled: !can_redo,
            save_label: if self.is_saving {
                "Saving..."
            } else if self.save_has_error {
                "Error - Retry"
            } else if self.is_dirty {
                "Save progress"
            } else {
                "Saved"
            }
            .to_string(),
            save_disabled: self.is_saving,
            save_clickable: save_is_clickable,
            save_tooltip_variant,
            save_tooltip_content: if self.save_has_error {
                self.save_error_message
                    .as_deref()
                    .filter(|message| !message.trim().is_empty())
                    .map(|message| format!("Save failed: {message}"))
                    .unwrap_or_else(|| "Save failed. Retry to save changes.".to_string())
            } else if self.is_dirty {
                "You have unsaved changes".to_string()
            } else {
                "All changes saved".to_string()
            },
            submit_label: submit.trigger_button.clone(),
            submit,
            review_actions: self.review_action_strip(project),
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
                self.lane_waveforms
                    .get(&lane.id)
                    .and_then(|waveform| waveform.overview_render_model(self.playhead_seconds))
                    .map(|waveform_model| OverviewLaneWaveformView {
                        lane_id: lane.id.clone(),
                        color_token: lane_color_token(lane_index).to_string(),
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

    pub fn transcript_rows(&self, project: &Project) -> Vec<TranscriptRowView> {
        self.transcript_rows_for_filter(project, &self.track_filter)
    }

    pub fn transcript_rows_for_render(&self, project: &Project) -> Vec<TranscriptRowView> {
        build_transcript_rows(
            project,
            &self.track_filter,
            None,
            &self.active_region_by_track,
            self.diff_view,
            &self.asserted_warning_keys,
            self.server_lint_snapshot(),
        )
    }

    pub fn active_transcript_row_id(&self, project: &Project) -> Option<String> {
        self.transcript_rows(project)
            .into_iter()
            .find(|row| row.is_active)
            .map(|row| row.segment_id)
    }

    pub fn all_transcript_rows(&self, project: &Project) -> Vec<TranscriptRowView> {
        self.transcript_rows_for_filter(project, &TrackFilter::All)
    }

    pub fn transcript_visible_cells(&self, project: &Project) -> TranscriptRowVisibleCells {
        derive_transcription_row_visible_cells(project)
    }

    pub fn transcript_empty_state(&self, project: &Project) -> TranscriptEmptyStateView {
        build_transcript_empty_state(project)
    }

    pub fn transcript_action_menu(
        &self,
        project: &Project,
        row: &TranscriptRowView,
    ) -> TranscriptActionMenuView {
        build_transcript_action_menu(project, row)
    }
}

pub fn build_toolbar_feedback_button(
    show_feedback_button: bool,
    on_open_feedback_provided: bool,
) -> ToolbarFeedbackButtonView {
    ToolbarFeedbackButtonView {
        visible: show_feedback_button && on_open_feedback_provided,
        tooltip_variant: "info".to_string(),
        tooltip_content: "Add feedback for L1 transcriber".to_string(),
        label: "Feedback".to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportIdButtonView {
    pub tooltip_variant: String,
    pub tooltip_content: String,
    pub button_label: String,
    pub icon: String,
    pub clipboard_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolbarFeedbackButtonView {
    pub visible: bool,
    pub tooltip_variant: String,
    pub tooltip_content: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransportStripView {
    pub track_filter: TrackFilter,
    pub track_filter_options: Vec<TrackFilter>,
    pub play_label: String,
    pub problem_summary_label: Option<String>,
    pub undo_disabled: bool,
    pub redo_disabled: bool,
    pub save_label: String,
    pub save_disabled: bool,
    pub save_clickable: bool,
    pub save_tooltip_variant: SaveTooltipVariant,
    pub save_tooltip_content: String,
    pub submit_label: String,
    pub submit: SubmitControlView,
    pub review_actions: ReviewActionStripView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveTooltipVariant {
    Default,
    Warning,
}

impl EditorSession {
    fn problem_summary_label(&self, project: &Project) -> Option<String> {
        let error_count = self.blocking_lint_count(project);
        let warning_count = self.unacknowledged_warning_count(project)
            + self.unresolved_low_confidence_count(project);
        if error_count == 0 && warning_count == 0 {
            return None;
        }

        let mut parts = Vec::new();
        if error_count > 0 {
            let suffix = if error_count == 1 { "error" } else { "errors" };
            parts.push(format!("{error_count} {suffix}"));
        }
        if warning_count > 0 {
            let suffix = if warning_count == 1 {
                "warning"
            } else {
                "warnings"
            };
            parts.push(format!("{warning_count} {suffix}"));
        }

        Some(parts.join(", "))
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
                    color_token: lane_color_token(lane_index).to_string(),
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
