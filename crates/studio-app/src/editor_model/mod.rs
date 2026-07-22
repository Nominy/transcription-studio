mod action_row;
mod buffered_playback;
mod context_panels;
mod diff_toolbar;
mod diff_workbench;
mod feedback_assessment;
mod no_work_modal;
mod page_controller;
mod review;
mod review_event;
mod selection;
mod submit_control;
mod task_lookup;
mod transport;
mod view_data;

use std::collections::{BTreeMap, BTreeSet};

use crate::babel_metadata::server_lint_snapshot_from_project;
use crate::linter::{LintSeverity, ServerLintSnapshot};
use crate::timeline_model::{LaneWaveform, TimeRange};
use crate::track_filter::TrackFilter;
use crate::transcript_model::{TranscriptRowView, build_transcript_rows};
use studio_core::Project;

pub use action_row::{
    DropClaimControlView, ReadOnlyActionCommand, ReadOnlyActionView, ReviewActionStripView,
};
pub use buffered_playback::{
    BufferedPlaybackDirection, BufferedPlaybackRange, compute_buffered_playback_range,
};
pub use context_panels::{
    COLLAPSE_COPY_RESET_DELAY_MS, CollapsiblePanelInput, CollapsiblePanelModel,
    CollapsiblePanelToggleIcon, CollapsiblePanelToggleResult, ContextPanelTextDirection,
    ORIGINAL_SCRIPT_STORAGE_KEY, OriginalScriptPanelInput, OriginalScriptPanelModel,
    SCRIPT_CONTEXT_STORAGE_KEY, SUBMISSION_ID_COPY_RESET_DELAY_MS, ScriptContext,
    ScriptContextFieldModel, ScriptContextPanelInput, ScriptContextPanelModel, SubmissionCopyIcon,
    SubmissionIdCopyResult, SubmissionIdModel, TtsPanelEffect, build_collapsible_panel_model,
    build_original_script_panel_model, build_script_context_panel_model, build_submission_id_model,
    filter_script_context_fields, read_stored_collapsed_state, reduce_collapsible_panel_toggle,
    resolve_submission_id_copy_failure, resolve_submission_id_copy_success,
};
pub use diff_toolbar::{
    DiffAnnotationMetricTooltipView, DiffAnnotationMetricView, DiffCompareOptionView,
    DiffCompareSelectView, DiffMetricCountView, DiffMetricValueView,
    DiffTimestampMetricTooltipView, DiffToleranceBarView, DiffToolbarBadgeView,
    DiffToolbarLegendItemView, DiffToolbarMetricsView, DiffToolbarMode, DiffToolbarToggleView,
    DiffToolbarView,
};
pub use diff_workbench::{
    DiffAnnotationDiffPairInput, DiffAnnotationForMatch, DiffAnnotationRow, DiffAnnotationStatus,
    DiffAnnotationStatusEntry, DiffComparePair, DiffReviewActionSummary,
    DiffTranscriptionDiffActionsInput, DiffTranscriptionSelectionInput,
    TranscriptAnnotationDiffSource, build_annotation_diff_actions, build_annotation_diff_pair,
    build_annotation_status_map, build_transcription_diff_actions, build_transcription_diff_pair,
    choose_annotation_compare_selection, choose_transcription_compare_selection,
    is_transcription_compare_newer, match_annotation_diff_rows,
    resolve_annotation_diff_perspective,
};
pub use feedback_assessment::{
    AssessmentPhase, AssessmentRetry, AssessmentSubmitTarget, FEEDBACK_MAX_COMMENT_LENGTH,
    FEEDBACK_RUBRIC_FIELDS, FeedbackDraft, FeedbackDraftRuntimeState, FeedbackDraftSaveRequest,
    FeedbackDraftSuccess, FeedbackDraftSuccessInput, FeedbackDraftSuccessResponse, FeedbackInput,
    FeedbackInputResponse, FeedbackRubricEntry, FeedbackRubricField, FeedbackSaveRequestInput,
    FinalAssessmentSubmitSuccess, OTHER_FEEDBACK_FLAGS, PracticeSubmitSuccess,
    apply_feedback_draft_success, clear_feedback_draft, compute_assessment_phase,
    compute_assessment_retry, compute_assessment_submit_target, compute_feedback_draft_reset,
    compute_feedback_draft_save_request, compute_final_assessment_submit_success,
    compute_practice_submit_success, get_rubric_fields_for_flags, has_other_feedback_flag,
    parse_feedback_responses, serialize_feedback_draft,
};
pub use no_work_modal::{
    NO_WORK_DESCRIPTION, NoWorkModalEffect, NoWorkModalEvent, NoWorkModalInput, NoWorkModalModel,
    build_no_work_modal_model, reduce_no_work_modal_event,
};
pub use page_controller::{
    ARABIC_PROJECT_IDS, BACKGROUND_NOISE_PROJECT_IDS, PageControllerAutoClaimResult,
    PageControllerEffect, PageControllerInitialLoadInput, PageControllerRenderBranch,
    PageControllerState, PageControllerTransition, PageMode, ProcessedRecording, ProjectConfig,
    REVIEW_ACTION_IN_PROGRESS, ReviewActionPageData, WorkbenchProps, WorkerPermission,
    apply_auto_claim_success, apply_processed_transcription_url_load_success,
    apply_review_action_url_load_success, apply_task_lookup_claim_success,
    build_task_lookup_claim_effect, build_workbench_props, choose_page_controller_render_branch,
    choose_task_lookup_read_only_action, compute_initial_load_effect,
    create_initial_page_controller_state, derive_is_rtl, derive_page_mode,
    map_processed_recordings_for_workbench, resolve_stitched_permission,
};
pub use review_event::{
    ReviewEventThrottleInput, ReviewEventThrottleResult, compute_review_event_throttle,
};
pub use submit_control::{
    AdminFlagControlView, AdminFlagPayload, SkipChunkControlView, SubmitControlView,
    SubmitDialogDescriptionTone, SubmitTooltipVariant,
};
pub use task_lookup::{
    CONFIRM_EDIT_BODY, ConfirmEditModel, PROCESSED_TRANSCRIPTION_PLACEHOLDER,
    READ_ONLY_SELECT_DESCRIPTION, REVIEW_ACTION_PLACEHOLDER, STITCHED_SELECT_DESCRIPTION,
    SelectActionModel, TaskLookupEffect, TaskLookupEvent, TaskLookupInitialInput,
    TaskLookupSelectInput, TaskLookupState, TaskLookupUpdate, build_confirm_edit_model,
    build_select_action_model, create_initial_task_lookup_state, reduce_task_lookup_event,
    trim_task_lookup_field_value,
};
pub use view_data::{
    SaveTooltipVariant, SupportIdButtonView, ToolbarFeedbackButtonView, TransportStripView,
    build_toolbar_feedback_button,
};

const DEFAULT_SPEED: f32 = 1.0;
const DEFAULT_ZOOM: f32 = 1.0;
const BABEL_TIMELINE_RENDER_WIDTH_PX: f64 = 1_000.0;
const BABEL_DEFAULT_MIN_PX_PER_SEC: f64 = 200.0;
const MIN_LOOP_DURATION_SECONDS: f64 = 0.01;

#[derive(Clone, Debug, PartialEq)]
pub struct EditorSession {
    pub speed: f32,
    pub zoom: f32,
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
    pub active_region_by_track: BTreeMap<String, String>,
    pub collapsed_lane_ids: BTreeSet<String>,
    pub muted_lane_ids: BTreeSet<String>,
    pub lane_vertical_zoom: BTreeMap<String, f32>,
    pub lane_waveforms: BTreeMap<String, LaneWaveform>,
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
        let viewport = viewport_centered_on(bounds, bounds.start_seconds, DEFAULT_ZOOM);
        Self {
            speed: DEFAULT_SPEED,
            zoom: DEFAULT_ZOOM,
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
            active_region_by_track: BTreeMap::new(),
            collapsed_lane_ids: BTreeSet::new(),
            muted_lane_ids: BTreeSet::new(),
            lane_vertical_zoom: BTreeMap::new(),
            lane_waveforms: BTreeMap::new(),
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

    pub fn should_run_auto_save_tick(&self, paused: bool) -> bool {
        self.is_dirty && !self.is_saving && !paused
    }

    pub fn set_lane_waveform(&mut self, lane_id: impl Into<String>, waveform: LaneWaveform) {
        self.lane_waveforms.insert(lane_id.into(), waveform);
    }

    pub fn timeline_min_px_per_sec(&self) -> f64 {
        timeline_min_px_per_sec_for_zoom(self.zoom)
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
            &self.active_region_by_track,
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
        self.viewport = viewport_centered_on(bounds, self.playhead_seconds, self.zoom);
    }
}

pub(super) fn visible_duration_for_zoom(bounds: TimeRange, zoom: f32) -> f64 {
    let full_duration = bounds.duration_seconds();
    (BABEL_TIMELINE_RENDER_WIDTH_PX / timeline_min_px_per_sec_for_zoom(zoom)).min(full_duration)
}

fn viewport_centered_on(bounds: TimeRange, center_seconds: f64, zoom: f32) -> TimeRange {
    let full_duration = bounds.duration_seconds();
    let visible_duration = visible_duration_for_zoom(bounds, zoom);
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
    zoom as f64 * BABEL_DEFAULT_MIN_PX_PER_SEC
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
