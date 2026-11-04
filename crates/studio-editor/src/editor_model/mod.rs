pub mod buffered_playback;
pub mod context_panels;
pub mod diff_toolbar;
pub mod feedback_assessment;
mod review;
pub mod review_actions;
mod selection;
pub mod submit_control;
pub mod task_lookup;
mod transport;
mod view_data;

use std::collections::{BTreeMap, BTreeSet};

use crate::babel_metadata::server_lint_snapshot_from_project;
use crate::linter::{LintSeverity, ServerLintSnapshot};
use crate::timeline_model::{LaneWaveform, TimeRange, display_waveform};
use crate::track_filter::TrackFilter;
use crate::transcript_model::{TranscriptRowView, build_transcript_rows};
use studio_core::Project;

pub use transport::PLAYBACK_SPEED_STEPS;
pub use view_data::{ProblemCounts, SaveStatus};

const DEFAULT_SPEED: f32 = 1.0;
const DEFAULT_ZOOM: f32 = 1.0;
/// Lane width assumed until the timeline reports its own (CSS px).
const DEFAULT_TIMELINE_WIDTH_PX: f64 = 1_000.0;
/// Babel's `minPxPerSec` at zoom 1: the viewport shows `lane width / (zoom × 200)` seconds.
const DEFAULT_MIN_PX_PER_SEC: f64 = 200.0;
const MIN_LOOP_DURATION_SECONDS: f64 = 0.01;

#[derive(Clone, Debug, PartialEq)]
pub struct EditorSession {
    pub speed: f32,
    pub zoom: f32,
    /// Width of the timeline's lane surface in CSS px (see [`Self::set_timeline_width`]).
    timeline_width_px: f64,
    pub is_playing: bool,
    pub playhead_seconds: f64,
    pub diff_view: bool,
    pub loop_enabled: bool,
    pub loop_range: TimeRange,
    pub stored_loop_range: Option<TimeRange>,
    pub overlay_range: Option<TimeRange>,
    pub viewport: TimeRange,
    pub track_filter: TrackFilter,
    pub selected_segment_id: Option<String>,
    pub collapsed_lane_ids: BTreeSet<String>,
    pub muted_lane_ids: BTreeSet<String>,
    pub lane_vertical_zoom: BTreeMap<String, f32>,
    pub lane_waveforms: BTreeMap<String, LaneWaveform>,
    /// Waveforms painted instead of `lane_waveforms` (the timeline, segment blocks and the
    /// overview): what a lane plays while its enhanced audio is mixed in. Analysis (silence,
    /// drafts) keeps reading the originals in `lane_waveforms`.
    lane_display_waveforms: BTreeMap<String, LaneWaveform>,
    pub open_actions_segment_id: Option<String>,
    pub asserted_warning_keys: BTreeSet<String>,
    server_lint_snapshot: Option<ServerLintSnapshot>,
    pub problem_row_index: usize,
    pub is_dirty: bool,
    pub is_saving: bool,
    pub save_has_error: bool,
    pub save_error_message: Option<String>,
    pub last_saved_at: Option<String>,
    edit_revision: u64,
    saved_revision: u64,
    saving_revision: Option<u64>,
    pub review_state: ReviewState,
    pub support_id: String,
}

impl EditorSession {
    pub fn for_project(project: &Project) -> Self {
        let bounds = TimeRange::for_project(project);
        let viewport = viewport_centered_on(
            bounds,
            bounds.start_seconds,
            DEFAULT_ZOOM,
            DEFAULT_TIMELINE_WIDTH_PX,
        );
        Self {
            speed: DEFAULT_SPEED,
            zoom: DEFAULT_ZOOM,
            timeline_width_px: DEFAULT_TIMELINE_WIDTH_PX,
            is_playing: false,
            playhead_seconds: bounds.start_seconds,
            diff_view: false,
            loop_enabled: false,
            loop_range: bounds,
            stored_loop_range: None,
            overlay_range: None,
            viewport,
            track_filter: TrackFilter::All,
            selected_segment_id: first_segment_id(project),
            collapsed_lane_ids: BTreeSet::new(),
            muted_lane_ids: BTreeSet::new(),
            lane_vertical_zoom: BTreeMap::new(),
            lane_waveforms: BTreeMap::new(),
            lane_display_waveforms: BTreeMap::new(),
            open_actions_segment_id: None,
            asserted_warning_keys: BTreeSet::new(),
            server_lint_snapshot: server_lint_snapshot_from_project(project, 0),
            problem_row_index: 0,
            is_dirty: false,
            is_saving: false,
            save_has_error: false,
            save_error_message: None,
            last_saved_at: None,
            edit_revision: 0,
            saved_revision: 0,
            saving_revision: None,
            review_state: ReviewState::from_project(project),
            support_id: support_id_for_project(project),
        }
    }

    pub fn timeline_waveform_end_seconds(&self) -> f64 {
        self.lane_waveforms
            .values()
            .map(LaneWaveform::duration_seconds)
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .fold(0.0, f64::max)
    }

    pub fn timeline_bounds(&self, project: &Project) -> TimeRange {
        let mut bounds = TimeRange::for_project(project);
        bounds.end_seconds = bounds.end_seconds.max(self.timeline_waveform_end_seconds());
        bounds
    }

    pub fn mark_dirty(&mut self) {
        self.edit_revision = self.edit_revision.saturating_add(1);
        self.invalidate_server_lint_snapshot();
        self.is_dirty = true;
        if self.review_state == ReviewState::Submitted {
            self.review_state = ReviewState::Draft;
        }
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.edit_revision;
        self.saving_revision = None;
        self.is_dirty = false;
        self.is_saving = false;
        self.save_has_error = false;
        self.save_error_message = None;
    }

    pub fn begin_save(&mut self) {
        self.saving_revision = Some(self.edit_revision);
        self.is_saving = true;
        self.save_has_error = false;
        self.save_error_message = None;
    }

    pub fn complete_save(&mut self, save_succeeded: bool, detail: impl Into<String>) {
        let revision = self.saving_revision.unwrap_or(self.edit_revision);
        let _ = self.complete_save_for_revision(revision, save_succeeded, detail);
    }

    pub fn edit_revision(&self) -> u64 {
        self.edit_revision
    }

    pub fn current_edit_revision(&self) -> u64 {
        self.edit_revision
    }

    pub fn server_lint_snapshot(&self) -> Option<&ServerLintSnapshot> {
        self.server_lint_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.revision == self.edit_revision)
    }

    pub fn set_server_lint_snapshot(&mut self, snapshot: ServerLintSnapshot) -> bool {
        if snapshot.revision != self.edit_revision {
            return false;
        }
        self.server_lint_snapshot = Some(snapshot);
        true
    }

    pub fn invalidate_server_lint_snapshot(&mut self) {
        self.server_lint_snapshot = None;
    }

    pub fn saving_revision(&self) -> Option<u64> {
        self.saving_revision
    }

    pub fn complete_save_for_revision(
        &mut self,
        revision: u64,
        save_succeeded: bool,
        detail: impl Into<String>,
    ) -> bool {
        if self.saving_revision != Some(revision) {
            return false;
        }

        self.is_saving = false;
        self.saving_revision = None;
        let detail = detail.into();
        if save_succeeded {
            self.saved_revision = self.saved_revision.max(revision);
            self.is_dirty = self.edit_revision != self.saved_revision;
            self.save_has_error = false;
            self.save_error_message = None;
            self.last_saved_at = Some(detail);
            return true;
        }

        self.is_dirty = true;
        self.save_has_error = true;
        self.save_error_message = (!detail.trim().is_empty()).then_some(detail);
        self.last_saved_at = None;
        true
    }

    pub fn set_lane_waveform(&mut self, lane_id: impl Into<String>, waveform: LaneWaveform) {
        self.lane_waveforms.insert(lane_id.into(), waveform);
    }

    /// Paints `waveform` for the lane instead of its original (`None` restores the original).
    /// Returns the replaced display waveform with its caches.
    pub fn set_lane_display_waveform(
        &mut self,
        lane_id: &str,
        waveform: Option<LaneWaveform>,
    ) -> Option<LaneWaveform> {
        match waveform {
            Some(waveform) => self
                .lane_display_waveforms
                .insert(lane_id.to_owned(), waveform),
            None => self.lane_display_waveforms.remove(lane_id),
        }
    }

    /// The waveform painted for a lane: its display override, else its original.
    pub fn lane_display_waveform(&self, lane_id: &str) -> Option<&LaneWaveform> {
        display_waveform(&self.lane_display_waveforms, &self.lane_waveforms, lane_id)
    }

    pub fn timeline_min_px_per_sec(&self) -> f64 {
        timeline_min_px_per_sec_for_zoom(self.zoom)
    }

    pub fn timeline_width_px(&self) -> f64 {
        self.timeline_width_px
    }

    /// The lane surface is `width_px` CSS px wide: like Babel's wavesurfer at `minPxPerSec`
    /// = zoom × 200, the viewport keeps its start and shows `width_px / (zoom × 200)` seconds.
    pub fn set_timeline_width(&mut self, project: &Project, width_px: f64) {
        if !width_px.is_finite() || width_px <= 0.0 || width_px == self.timeline_width_px {
            return;
        }
        self.timeline_width_px = width_px;
        let bounds = self.timeline_bounds(project);
        let duration = self.visible_duration(bounds);
        if duration >= bounds.duration_seconds() {
            self.viewport = bounds;
            return;
        }
        let start = self
            .viewport
            .start_seconds
            .clamp(bounds.start_seconds, bounds.end_seconds - duration);
        self.viewport = TimeRange {
            start_seconds: start,
            end_seconds: start + duration,
        };
    }

    pub(super) fn visible_duration(&self, bounds: TimeRange) -> f64 {
        visible_duration_for_zoom(bounds, self.zoom, self.timeline_width_px)
    }

    fn transcript_rows_for_filter(
        &self,
        project: &Project,
        track_filter: &TrackFilter,
    ) -> Vec<TranscriptRowView> {
        build_transcript_rows(
            project,
            track_filter,
            self.selected_segment_id.as_deref(),
            self.diff_view,
            &self.asserted_warning_keys,
            self.server_lint_snapshot(),
        )
    }

    fn problem_rows(&self, project: &Project) -> Vec<TranscriptRowView> {
        self.transcript_rows(project)
            .into_iter()
            .filter(|row| {
                matches!(
                    row.lint.severity,
                    LintSeverity::Error | LintSeverity::Warning
                )
            })
            .collect()
    }

    fn recenter_viewport(&mut self, bounds: TimeRange) {
        self.viewport = viewport_centered_on(
            bounds,
            self.playhead_seconds,
            self.zoom,
            self.timeline_width_px,
        );
    }
}

fn visible_duration_for_zoom(bounds: TimeRange, zoom: f32, width_px: f64) -> f64 {
    let full_duration = bounds.duration_seconds();
    (width_px / timeline_min_px_per_sec_for_zoom(zoom)).min(full_duration)
}

fn viewport_centered_on(
    bounds: TimeRange,
    center_seconds: f64,
    zoom: f32,
    width_px: f64,
) -> TimeRange {
    let full_duration = bounds.duration_seconds();
    let visible_duration = visible_duration_for_zoom(bounds, zoom, width_px);
    if visible_duration >= full_duration {
        return bounds;
    }

    let center = if center_seconds.is_finite() {
        center_seconds.clamp(bounds.start_seconds, bounds.end_seconds)
    } else {
        bounds.start_seconds
    };
    let max_start = bounds.end_seconds - visible_duration;
    let start = (center - visible_duration / 2.0).clamp(bounds.start_seconds, max_start);
    TimeRange {
        start_seconds: start,
        end_seconds: start + visible_duration,
    }
}

fn timeline_min_px_per_sec_for_zoom(zoom: f32) -> f64 {
    let zoom = if zoom.is_finite() {
        zoom.clamp(0.05, 10.0)
    } else {
        DEFAULT_ZOOM
    };
    zoom as f64 * DEFAULT_MIN_PX_PER_SEC
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewState {
    Draft,
    InProgress,
    Submitted,
}

impl ReviewState {
    fn from_project(project: &Project) -> Self {
        if project.metadata.get("babel.format").map(String::as_str) != Some("review-action") {
            return Self::Draft;
        }

        match project
            .metadata
            .get("babel.review_action.actionDecision")
            .map(String::as_str)
        {
            Some("pass") => Self::Submitted,
            Some("in-progress") => Self::InProgress,
            _ => Self::Draft,
        }
    }
}

/// Practice/assessment phase of an assessment task (from project metadata).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssessmentPhase {
    Practice,
    Assessment,
}

impl AssessmentPhase {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "practice" => Some(Self::Practice),
            "assessment" => Some(Self::Assessment),
            _ => None,
        }
    }
}

fn first_segment_id(project: &Project) -> Option<String> {
    project
        .segments
        .iter()
        .min_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds))
        .map(|segment| segment.id.clone())
}

fn support_id_for_project(project: &Project) -> String {
    project
        .segments
        .iter()
        .find_map(|segment| {
            segment
                .metadata
                .get("reviewActionId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| project.metadata.get("action_id").cloned())
        .unwrap_or_else(|| format!("{}:{}", project.id, project.segments.len()))
}
