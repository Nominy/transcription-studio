pub mod buffered_playback;
mod diagnostics;
mod selection;
mod transport;
mod view_data;

use std::collections::{BTreeMap, BTreeSet};

use crate::timeline_model::{LaneWaveform, TimeRange, display_waveform};
use crate::track_filter::TrackFilter;
use crate::transcript_model::{TranscriptRowView, build_transcript_rows};
use exaudio_extension::document::{DiagnosticCollection, DocumentEnvironment, Severity};
use studio_core::Project;

pub use transport::PLAYBACK_SPEED_STEPS;
pub use view_data::{ProblemCounts, SaveStatus};

const DEFAULT_SPEED: f32 = 1.0;
const DEFAULT_ZOOM: f32 = 1.0;
/// Lane width assumed until the timeline reports its own (CSS px).
const DEFAULT_TIMELINE_WIDTH_PX: f64 = 1_000.0;
/// Minimum pixels per second at zoom 1: the viewport shows `lane width / (zoom × 200)` seconds.
const DEFAULT_MIN_PX_PER_SEC: f64 = 200.0;
/// Seconds a timeline opens on when its first audio gives it an extent (a project without
/// segments); the whole file when shorter. Segments give an extent up front, so projects with
/// them open at [`DEFAULT_ZOOM`] like Babel.
const INITIAL_VIEW_SECONDS: f64 = 30.0;
const MIN_LOOP_DURATION_SECONDS: f64 = 0.01;

/// Cut-preview selection (Helper's cut preview): a time range drawn on one lane; S / Shift+S
/// cut it out of that lane, L loops playback over it.
#[derive(Clone, Debug, PartialEq)]
pub struct CutOverlay {
    pub lane_id: String,
    pub range: TimeRange,
    /// Playback loops over `range` (all lanes) while set; moving or resizing the selection
    /// keeps looping over the new range.
    pub looping: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorSession {
    pub speed: f32,
    pub zoom: f32,
    /// Width of the timeline's lane surface in CSS px (see [`Self::set_timeline_width`]).
    timeline_width_px: f64,
    pub is_playing: bool,
    pub playhead_seconds: f64,
    pub diff_view: bool,
    pub overlay: Option<CutOverlay>,
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
    pub environment: DocumentEnvironment,
    diagnostics: Vec<DiagnosticCollection>,
    raw_diagnostics: Vec<DiagnosticCollection>,
    pub problem_row_index: usize,
    pub is_dirty: bool,
    pub is_saving: bool,
    pub save_has_error: bool,
    pub save_error_message: Option<String>,
    pub last_saved_at: Option<String>,
    edit_revision: u64,
    saved_revision: u64,
    saving_revision: Option<u64>,
}

impl EditorSession {
    pub fn for_project(project: &Project) -> Self {
        Self::for_project_with_environment(project, DocumentEnvironment::local(project.id.clone()))
    }

    pub fn for_project_with_environment(
        project: &Project,
        environment: DocumentEnvironment,
    ) -> Self {
        let bounds = TimeRange::for_project(project);
        let viewport = viewport_centered_on(
            bounds,
            bounds.start_seconds,
            DEFAULT_ZOOM,
            DEFAULT_TIMELINE_WIDTH_PX,
        );
        let raw_diagnostics: Vec<_> = environment
            .providers
            .diagnostics
            .iter()
            .map(|provider| provider.diagnose(project, 0))
            .filter(|collection| collection.revision == 0)
            .collect();
        let mut diagnostics = raw_diagnostics.clone();
        for postprocessor in &environment.providers.diagnostic_postprocessors {
            postprocessor.process(project, 0, &mut diagnostics);
        }
        Self {
            speed: DEFAULT_SPEED,
            zoom: DEFAULT_ZOOM,
            timeline_width_px: DEFAULT_TIMELINE_WIDTH_PX,
            is_playing: false,
            playhead_seconds: bounds.start_seconds,
            diff_view: false,
            overlay: None,
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
            diagnostics,
            raw_diagnostics,
            environment,
            problem_row_index: 0,
            is_dirty: false,
            is_saving: false,
            save_has_error: false,
            save_error_message: None,
            last_saved_at: None,
            edit_revision: 0,
            saved_revision: 0,
            saving_revision: None,
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
        self.diagnostics.clear();
        self.raw_diagnostics.clear();
        self.is_dirty = true;
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

    pub fn environment(&self) -> &DocumentEnvironment {
        &self.environment
    }

    pub fn diagnostics(&self) -> &[DiagnosticCollection] {
        &self.diagnostics
    }

    pub fn publish_diagnostics(
        &mut self,
        project: &Project,
        collection: DiagnosticCollection,
    ) -> bool {
        if !self.replace_raw_diagnostics(collection) {
            return false;
        }
        self.compose_diagnostics(project);
        true
    }

    fn replace_raw_diagnostics(&mut self, collection: DiagnosticCollection) -> bool {
        if collection.revision != self.edit_revision {
            return false;
        }
        if let Some(existing) = self
            .raw_diagnostics
            .iter_mut()
            .find(|existing| existing.source == collection.source)
        {
            *existing = collection;
        } else {
            self.raw_diagnostics.push(collection);
        }
        true
    }

    fn compose_diagnostics(&mut self, project: &Project) {
        self.diagnostics.clone_from(&self.raw_diagnostics);
        for postprocessor in &self.environment.providers.diagnostic_postprocessors {
            postprocessor.process(project, self.edit_revision, &mut self.diagnostics);
        }
    }

    pub fn remove_diagnostics(&mut self, project: &Project, source: &str) {
        self.raw_diagnostics
            .retain(|collection| collection.source != source);
        self.compose_diagnostics(project);
    }

    pub fn refresh_diagnostics(&mut self, project: &Project) {
        let collections: Vec<_> = self
            .environment
            .providers
            .diagnostics
            .iter()
            .map(|provider| provider.diagnose(project, self.edit_revision))
            .collect();
        for collection in collections {
            self.replace_raw_diagnostics(collection);
        }
        self.compose_diagnostics(project);
    }

    pub fn decorations(&self, project: &Project) -> Vec<exaudio_extension::document::Decoration> {
        self.environment
            .providers
            .decorations
            .iter()
            .flat_map(|provider| provider.decorate(project, self.edit_revision))
            .collect()
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

    /// Opens the view on a timeline that just gained its extent (its first audio): the first
    /// [`INITIAL_VIEW_SECONDS`] (the whole file when shorter) at the zoom fitting them into the
    /// lane width, from the start. A zoom already changed from the default keeps its scale.
    pub fn show_initial_view(&mut self, project: &Project) {
        let bounds = self.timeline_bounds(project);
        if self.zoom == DEFAULT_ZOOM {
            let visible_seconds = INITIAL_VIEW_SECONDS.min(bounds.duration_seconds());
            let zoom = self.timeline_width_px / (visible_seconds * DEFAULT_MIN_PX_PER_SEC);
            self.zoom = (zoom as f32).clamp(0.05, 10.0);
        }
        self.viewport = viewport_centered_on(
            bounds,
            bounds.start_seconds,
            self.zoom,
            self.timeline_width_px,
        );
    }

    /// The timeline bounds grew (audio arrived): the viewport keeps its start and shows the
    /// seconds the zoom gives it, as [`Self::set_timeline_width`] does.
    pub fn fit_viewport_to_zoom(&mut self, project: &Project) {
        self.fit_viewport_to_zoom_within(self.timeline_bounds(project));
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

    /// The lane surface is `width_px` CSS px wide: at `minPxPerSec`
    /// = zoom × 200, the viewport keeps its start and shows `width_px / (zoom × 200)` seconds.
    pub fn set_timeline_width(&mut self, project: &Project, width_px: f64) {
        if !width_px.is_finite() || width_px <= 0.0 || width_px == self.timeline_width_px {
            return;
        }
        self.timeline_width_px = width_px;
        self.fit_viewport_to_zoom_within(self.timeline_bounds(project));
    }

    fn fit_viewport_to_zoom_within(&mut self, bounds: TimeRange) {
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
            crate::transcript_model::RowContributions {
                diagnostics: self.diagnostics(),
                decorations: self.decorations(project),
            },
            self.environment.policy.read_only,
        )
    }

    fn problem_rows(&self, project: &Project) -> Vec<TranscriptRowView> {
        self.transcript_rows(project)
            .into_iter()
            .filter(|row| {
                row.lint_issues
                    .iter()
                    .any(|issue| matches!(issue.severity, Severity::Error | Severity::Warning))
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

fn first_segment_id(project: &Project) -> Option<String> {
    project
        .segments
        .iter()
        .min_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds))
        .map(|segment| segment.id.clone())
}
