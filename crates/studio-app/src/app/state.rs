use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use iced::{Point, Task, keyboard::Modifiers};
use serde_json::Value;
use studio_app::{
    BabelProjectLintTagConfig, EditorSession, LaneWaveform, LoopStripHit, LoopStripHitTarget,
    META_BABEL_PROJECT_ID, NoWorkModalInput, NoWorkModalModel, ORIGINAL_SCRIPT_STORAGE_KEY,
    OverviewWaveformView, SCRIPT_CONTEXT_STORAGE_KEY, StudioWorkflow, SubmitControlView,
    TaskLookupEffect, TaskLookupState, TimeRange, TimelineLaneView, TimelineScrollbarView,
    TimelineSegmentIndex, TimelineSegmentPreview, TrackFilter, TranscriptAnnotationKind,
    TranscriptRowView, TransportStripView, WaveformBlock, babel_project_lint_tag_metadata,
    build_no_work_modal_model, load_sample_project, resolve_babel_project_lint_tag_config,
};
use studio_core::{AudioLane, Project, Segment};
use studio_waveform::{WaveformHit, WaveformHitTarget, decode_pcm16_wav};

use super::audio_backend::{AudioPlaybackBackend, PlaybackTrack};
use super::babel_audio::{
    BabelPayloadLoadKind, PreparedBabelAudioPayload, babel_payload_needs_audio_preparation,
    prepare_babel_audio_payload,
};
use super::hotkeys::TRANSPORT_TICK_SECONDS;
use super::large_project_smoke::{
    large_project_live_pan_enabled, large_project_live_transcript_scroll_enabled,
};
use super::native_capabilities::{
    DraftRedistributionPreview, DraftRowPreview, DraftSummaryPreview, DraftTranscriptionPreview,
    NativeCapabilityHealth, ReviewerFinalizePreview, ReviewerGeneratedReviewPreview,
    ReviewerSessionPreview, ReviewerTemplatePreview,
};
use super::panel_persistence::PanelCollapseStorage;
use super::review_backend_client::{ReviewBackendRequest, dispatch_review_backend_requests};
use super::service_callbacks::{
    BabelServiceRequest, ClaimByProcessedTranscriptionRequest, ClaimNextFromQueueRequest,
    LoadLatestReviewActionRequest, LoadReviewActionRequest, META_ACTIVE_QUEUE_ID,
    META_CHUNKED_QUEUE_ID, META_QUEUE_ID, META_REVIEW_ACTION_ID, META_STITCHED_QUEUE_ID,
    build_admin_rate_review_action_request, build_drop_worker_action_request,
    build_skip_review_action_request, build_submit_review_action_request,
};
use super::service_executor::{
    dispatch_babel_service_requests, is_dispatchable_babel_service_request,
};
use super::startup_route::{
    META_DISPLAY_FEEDBACK, META_FEEDBACK_INITIAL_OPEN, META_TASK_LOOKUP_PROCESSED_ID,
    META_TASK_LOOKUP_REVIEW_ACTION_ID,
};
use super::tag_suggestions::{self, TagSuggestionKeyboardAction};
use super::transcript_virtualization::{
    DEFAULT_TRANSCRIPT_VIEWPORT_HEIGHT, TRANSCRIPT_VIRTUAL_OVERSCAN_VIEWPORTS,
    TRANSCRIPT_VIRTUAL_RENDER_ALL_THRESHOLD, TranscriptVirtualWindow,
    TranscriptVirtualizationInput, centered_row_scroll_offset, estimate_transcript_row_height,
    sanitized_scroll_offset, sanitized_viewport_height, transcript_virtual_window,
};
use super::updater::UpdatePanelState;
use super::{
    AuditionKey, CreateRegionDialog, EditAnnotationDialog, FeedbackPanelState, Message,
    NativeAuthFlowState, NativeAuthStartup, RuntimeGraphicsInfo, SegmentDraft,
};

pub(super) const DEFAULT_IMPORT_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sample-task.json");
pub(super) const DEFAULT_EXPORT_PATH: &str = ".studio-dev/export/babel-task.json";
const BUNDLED_ASSETS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");
const RECOVERED_FIXTURE_AUDIO_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../studio-babel/tests/fixtures/recovered/babel-fixture"
);
pub(super) const SUPPORT_ID_COPIED_RESET_DELAY_MS: u64 = 2000;
const TIMELINE_SCROLLBAR_MIN_THUMB_FRACTION: f32 = 0.024;
const LANE_VERTICAL_ZOOM_SLIDER_MIN: f32 = 0.3;
const LANE_VERTICAL_ZOOM_SLIDER_MAX: f32 = 1000.0;
const DEFAULT_LANE_VERTICAL_ZOOM: f32 = 20.0;
const TRANSCRIPT_ROWS_CACHE_TRACE_ENV: &str = "STUDIO_TRACE_TRANSCRIPT_CACHE";
const TIMELINE_DRAG_TRACE_ENV: &str = "STUDIO_TRACE_TIMELINE_DRAG";
const AUDIO_PLAYBACK_TRACE_ENV: &str = "STUDIO_LOG_AUDIO_PLAYBACK";
const AUDIO_STARTUP_TRACE_ENV: &str = "STUDIO_TRACE_AUDIO_STARTUP";
const FRAME_CADENCE_TRACE_ENV: &str = "STUDIO_TRACE_FRAME_CADENCE";
const FRAME_CADENCE_TRACE_FRAMES_ENV: &str = "STUDIO_TRACE_FRAME_CADENCE_FRAMES";
const FRAME_CADENCE_TRACE_SLOW_MS_ENV: &str = "STUDIO_TRACE_FRAME_CADENCE_SLOW_MS";
const TRANSPORT_TICK_UPDATE_TRACE_ENV: &str = "STUDIO_TRACE_TRANSPORT_TICK_UPDATE";
const TRANSPORT_TICK_UPDATE_TRACE_SLOW_MS_ENV: &str = "STUDIO_TRACE_TRANSPORT_TICK_UPDATE_SLOW_MS";
const TIMELINE_AUTOSCROLL_DEAD_ZONE_SECONDS: f64 = 0.025;
const TIMELINE_AUTOSCROLL_SPEED_MULTIPLIER: f64 = 4.0;
const DEFAULT_FRAME_CADENCE_TRACE_FRAMES: usize = 120;
const TRANSPORT_STRIP_CACHE_CAPACITY: usize = 4;
const STARTUP_PLAYBACK_SMOKE_ENV: &str = "STUDIO_SMOKE_PLAYBACK_ON_START";
const STARTUP_PLAYBACK_SMOKE_DELAY_FRAMES_ENV: &str = "STUDIO_SMOKE_PLAYBACK_DELAY_FRAMES";
pub(super) const BABEL_TRACK_CELL_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);
const BABEL_PLAYBACK_SPEED_STEPS: [f32; 6] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0];
const BABEL_ROUTE_METADATA_KEYS: &[&str] = &[
    META_BABEL_PROJECT_ID,
    META_REVIEW_ACTION_ID,
    META_ACTIVE_QUEUE_ID,
    META_QUEUE_ID,
    META_CHUNKED_QUEUE_ID,
    META_STITCHED_QUEUE_ID,
    META_DISPLAY_FEEDBACK,
    META_FEEDBACK_INITIAL_OPEN,
    META_TASK_LOOKUP_PROCESSED_ID,
    META_TASK_LOOKUP_REVIEW_ACTION_ID,
];
const META_TIMELINE_ZOOM: &str = "studio.native.timeline.zoom";
const META_TIMELINE_SPEED: &str = "studio.native.timeline.speed";
const META_TIMELINE_TRACK_FILTER_LANE_ID: &str = "studio.native.timeline.trackFilterLaneId";
const META_TIMELINE_COLLAPSED_LANE_IDS: &str = "studio.native.timeline.collapsedLaneIds";
const META_TIMELINE_MUTED_LANE_IDS: &str = "studio.native.timeline.mutedLaneIds";
const META_TIMELINE_LANE_VERTICAL_ZOOM: &str = "studio.native.timeline.laneVerticalZoom";
const META_CAPABILITY_EXTRA_PACKAGE: &str = "babel.extra.package";
const REVIEWER_CAPABILITY_ACTION_LEVEL: i64 = 2;
const REVIEWER_ACTION_LEVEL_METADATA_KEYS: &[&str] = &[
    "action_level",
    "actionLevel",
    "review_action_level",
    "reviewActionLevel",
    "studio.babel.actionLevel",
    "studio.babel.reviewActionLevel",
];
const REVIEWER_ACTION_LEVEL_JSON_KEYS: &[&str] = &[
    "action_level",
    "actionLevel",
    "review_action_level",
    "reviewActionLevel",
    "level",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppSurface {
    Dashboard,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeCapabilityModalTab {
    Reviewer,
    GoldDrafting,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct DashboardState {
    pub(super) projects: Vec<DashboardProject>,
    pub(super) project_id: String,
    pub(super) chunked_queue_id: String,
    pub(super) stitched_queue_id: String,
    pub(super) review_action_id: String,
    pub(super) processed_transcription_id: String,
    pub(super) is_pending: bool,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DashboardProject {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) chunked_queue_id: String,
    pub(super) stitched_queue_id: String,
}

pub(super) fn timeline_loop_runtime_trace_line(
    target: LoopStripHitTarget,
    range: TimeRange,
    enabled: bool,
) -> String {
    format!(
        "Transcription Studio timeline drag: action=loop target={target:?} start={:.3} end={:.3} enabled={enabled}",
        range.start_seconds, range.end_seconds
    )
}

pub(super) fn timeline_scrollbar_runtime_trace_line(viewport: TimeRange) -> String {
    format!(
        "Transcription Studio timeline drag: action=scrollbar viewport={:.3}-{:.3}s",
        viewport.start_seconds, viewport.end_seconds
    )
}

pub(super) fn timeline_vertical_zoom_runtime_trace_line(lane_id: &str, value: f32) -> String {
    format!(
        "Transcription Studio timeline drag: action=vertical-zoom lane={lane_id} value={value:.3}"
    )
}

pub(super) fn timeline_pointer_capture_release_runtime_trace_line(
    loop_active: bool,
    scrollbar_active: bool,
    vertical_zoom_active: bool,
    waveform_active: bool,
) -> String {
    format!(
        "Transcription Studio timeline drag: action=captured-release loop={loop_active} scrollbar={scrollbar_active} vertical-zoom={vertical_zoom_active} waveform={waveform_active}"
    )
}

fn timeline_drag_trace_enabled() -> bool {
    std::env::var(TIMELINE_DRAG_TRACE_ENV).is_ok_and(|value| value != "0")
}

pub(super) fn transport_tick_update_runtime_trace_line(
    elapsed: Duration,
    is_playing: bool,
    started_playback: bool,
    phase_timings: TransportTickUpdatePhaseTimings,
) -> String {
    format!(
        "Transcription Studio transport tick update: elapsed_ms={:.3} playing={is_playing} started_playback={started_playback} advance_ms={:.3} live_pan_ms={:.3} live_scroll_ms={:.3} startup_ms={:.3} cadence_ms={:.3}",
        elapsed.as_secs_f64() * 1_000.0,
        phase_timings.advance_transport.as_secs_f64() * 1_000.0,
        phase_timings.live_pan_smoke.as_secs_f64() * 1_000.0,
        phase_timings.live_transcript_scroll_smoke.as_secs_f64() * 1_000.0,
        phase_timings.startup_playback_smoke.as_secs_f64() * 1_000.0,
        phase_timings.cadence_probe.as_secs_f64() * 1_000.0
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TransportTickUpdatePhaseTimings {
    pub(super) advance_transport: Duration,
    pub(super) live_pan_smoke: Duration,
    pub(super) live_transcript_scroll_smoke: Duration,
    pub(super) startup_playback_smoke: Duration,
    pub(super) cadence_probe: Duration,
}

fn transport_tick_update_trace_enabled() -> bool {
    std::env::var(TRANSPORT_TICK_UPDATE_TRACE_ENV).is_ok_and(|value| value != "0")
}

fn transport_tick_update_trace_slow_threshold() -> Option<Duration> {
    std::env::var(TRANSPORT_TICK_UPDATE_TRACE_SLOW_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|millis| Duration::from_secs_f64(millis / 1_000.0))
}

fn transport_tick_update_should_log(elapsed: Duration, started_playback: bool) -> bool {
    if started_playback {
        return true;
    }
    transport_tick_update_trace_slow_threshold().is_none_or(|threshold| elapsed >= threshold)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeFrameCadenceReport {
    pub(super) frames: usize,
    pub(super) intervals: usize,
    pub(super) elapsed: Duration,
    pub(super) total_interval: Duration,
    pub(super) min_interval: Duration,
    pub(super) max_interval: Duration,
}

impl RuntimeFrameCadenceReport {
    pub(super) fn runtime_trace_line(&self) -> String {
        let interval_count = self.intervals.max(1) as f64;
        let avg_millis = self.total_interval.as_secs_f64() * 1_000.0 / interval_count;
        format!(
            "Transcription Studio frame cadence: frames={} intervals={} elapsed_ms={:.3} avg_ms={:.3} min_ms={:.3} max_ms={:.3}",
            self.frames,
            self.intervals,
            self.elapsed.as_secs_f64() * 1_000.0,
            avg_millis,
            self.min_interval.as_secs_f64() * 1_000.0,
            self.max_interval.as_secs_f64() * 1_000.0
        )
    }
}

#[derive(Debug, Clone)]
pub(super) struct FrameCadenceProbe {
    target_frames: usize,
    slow_interval_threshold: Option<Duration>,
    frames: usize,
    first_frame: Option<Instant>,
    last_frame: Option<Instant>,
    total_interval: Duration,
    min_interval: Duration,
    max_interval: Duration,
    reported: bool,
}

impl FrameCadenceProbe {
    pub(super) fn new(target_frames: usize) -> Self {
        Self {
            target_frames: target_frames.max(2),
            slow_interval_threshold: None,
            frames: 0,
            first_frame: None,
            last_frame: None,
            total_interval: Duration::ZERO,
            min_interval: Duration::MAX,
            max_interval: Duration::ZERO,
            reported: false,
        }
    }

    pub(super) fn with_slow_interval_threshold(mut self, threshold: Duration) -> Self {
        self.slow_interval_threshold = Some(threshold);
        self
    }

    pub(super) fn record(&mut self, now: Instant) -> Option<RuntimeFrameCadenceReport> {
        if self.reported {
            return None;
        }

        let Some(previous) = self.last_frame.replace(now) else {
            self.first_frame = Some(now);
            self.frames = 1;
            return None;
        };
        let Some(interval) = now.checked_duration_since(previous) else {
            return None;
        };

        self.frames += 1;
        self.total_interval += interval;
        self.min_interval = self.min_interval.min(interval);
        self.max_interval = self.max_interval.max(interval);
        if self
            .slow_interval_threshold
            .is_some_and(|threshold| interval >= threshold)
        {
            let elapsed = self
                .first_frame
                .and_then(|first| now.checked_duration_since(first))
                .unwrap_or(self.total_interval);
            eprintln!(
                "{}",
                frame_cadence_slow_interval_runtime_trace_line(self.frames, interval, elapsed)
            );
        }

        if self.frames < self.target_frames {
            return None;
        }

        self.reported = true;
        Some(RuntimeFrameCadenceReport {
            frames: self.frames,
            intervals: self.frames.saturating_sub(1),
            elapsed: self
                .first_frame
                .and_then(|first| now.checked_duration_since(first))
                .unwrap_or(self.total_interval),
            total_interval: self.total_interval,
            min_interval: self.min_interval,
            max_interval: self.max_interval,
        })
    }
}

fn frame_cadence_slow_interval_runtime_trace_line(
    frame: usize,
    interval: Duration,
    elapsed: Duration,
) -> String {
    format!(
        "Transcription Studio frame interval: frame={} interval_ms={:.3} elapsed_ms={:.3}",
        frame,
        interval.as_secs_f64() * 1_000.0,
        elapsed.as_secs_f64() * 1_000.0
    )
}

pub(super) fn frame_cadence_probe_from_env() -> Option<FrameCadenceProbe> {
    if !std::env::var(FRAME_CADENCE_TRACE_ENV).is_ok_and(|value| value != "0") {
        return None;
    }

    let target_frames = std::env::var(FRAME_CADENCE_TRACE_FRAMES_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 2)
        .unwrap_or(DEFAULT_FRAME_CADENCE_TRACE_FRAMES);
    let mut probe = FrameCadenceProbe::new(target_frames);
    if let Some(millis) = std::env::var(FRAME_CADENCE_TRACE_SLOW_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        probe = probe.with_slow_interval_threshold(Duration::from_secs_f64(millis / 1_000.0));
    }
    Some(probe)
}

fn draft_audio_lane_label(lane: &AudioLane) -> String {
    let label = lane.label.trim();
    if label.is_empty() {
        lane.id.clone()
    } else {
        label.to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DraftAudioCueStatus {
    pub(super) loaded_tracks: usize,
    pub(super) total_lanes: usize,
    pub(super) missing_lane_labels: Vec<String>,
    pub(super) missing_lane_sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DraftAudioGuard {
    pub(super) expected_speaker_lanes: usize,
    pub(super) captured_speaker_lanes: usize,
    pub(super) captured_tracks: usize,
    pub(super) can_use_partial_audio: bool,
    pub(super) message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct HelperFocusRestore {
    pub(super) segment_id: String,
    pub(super) blur_playhead_seconds: f64,
    pub(super) restore_playhead_seconds: f64,
    pub(super) restore_offset: usize,
    pub(super) synthetic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct HelperGhostCursorView {
    pub(super) segment_id: String,
    pub(super) char_offset: usize,
    pub(super) before_cursor: String,
    pub(super) after_cursor: String,
}

pub(super) struct StudioState {
    pub(super) app_surface: AppSurface,
    pub(super) native_auth_flow: Option<NativeAuthFlowState>,
    pub(super) dashboard: DashboardState,
    pub(super) updater: UpdatePanelState,
    pub(super) workflow: Result<StudioWorkflow, String>,
    pub(super) editor: Option<EditorSession>,
    pub(super) segment_drafts: BTreeMap<String, SegmentDraft>,
    pub(super) keyboard_modifiers: Modifiers,
    pub(super) right_shift_pressed: bool,
    pub(super) held_audition_keys: BTreeSet<AuditionKey>,
    pub(super) audition_key_release_tokens: BTreeMap<String, BTreeSet<AuditionKey>>,
    pub(super) last_transport_tick: Option<Instant>,
    pub(super) large_project_live_pan_active: bool,
    pub(super) large_project_live_pan_frame: u64,
    pub(super) large_project_live_transcript_scroll_active: bool,
    pub(super) large_project_live_transcript_scroll_frame: u64,
    pub(super) waveform_hit_hover: Option<WaveformHitHover>,
    pub(super) active_overlay_drag: Option<OverlayDrag>,
    pub(super) active_waveform_drag: Option<WaveformDrag>,
    pub(super) active_waveform_create_drag: Option<WaveformCreateDrag>,
    pub(super) active_timeline_autoscroll: Option<TimelineAutoscroll>,
    pub(super) loop_strip_hover: Option<LoopStripHit>,
    pub(super) active_loop_drag: Option<LoopStripDrag>,
    pub(super) last_global_pointer_position: Option<Point>,
    pub(super) overview_hover_fraction: Option<f32>,
    pub(super) overview_pointer_sample: Option<OverviewPointerSample>,
    pub(super) overview_drag_capture: Option<DragAxisCapture>,
    pub(super) overview_drag_active: bool,
    pub(super) overview_drag_grab_offset_fraction: f32,
    pub(super) timeline_scrollbar_hover_fraction: Option<f32>,
    pub(super) timeline_scrollbar_pointer_sample: Option<OverviewPointerSample>,
    pub(super) timeline_scrollbar_drag_capture: Option<DragAxisCapture>,
    pub(super) timeline_scrollbar_drag_offset_fraction: Option<f32>,
    pub(super) lane_vertical_zoom_slider_hover: Option<LaneVerticalZoomSliderHover>,
    pub(super) lane_vertical_zoom_slider_pointer_sample:
        Option<LaneVerticalZoomSliderPointerSample>,
    pub(super) lane_vertical_zoom_slider_drag_capture: Option<LaneVerticalZoomSliderDragCapture>,
    pub(super) active_lane_vertical_zoom_slider_id: Option<String>,
    pub(super) lane_vertical_zoom_input: Option<LaneVerticalZoomInput>,
    pub(super) last_lane_vertical_zoom_slider_press: Option<LaneVerticalZoomSliderPress>,
    pub(super) last_track_cell_press: Option<TrackCellPress>,
    pub(super) last_transcript_row_press: Option<TranscriptRowPress>,
    pub(super) transcript_row_hover_segment_id: Option<String>,
    pub(super) tag_suggestion_state: Option<TagSuggestionUiState>,
    pub(super) helper_focused_segment_id: Option<String>,
    pub(super) helper_focus_restore_pending: Option<HelperFocusRestore>,
    pub(super) helper_last_focus_restore: Option<HelperFocusRestore>,
    pub(super) audio_playback: AudioPlaybackBackend,
    pub(super) acknowledged_low_confidence_words: BTreeMap<String, BTreeSet<usize>>,
    pub(super) panel_collapse_storage: PanelCollapseStorage,
    pub(super) script_context_panel_hidden: bool,
    pub(super) original_script_panel_hidden: bool,
    pub(super) copied_submission_id: bool,
    pub(super) import_path: String,
    pub(super) export_path: String,
    pub(super) status: String,
    pub(super) runtime_graphics_info: Option<RuntimeGraphicsInfo>,
    pub(super) runtime_audio_playback_info: Option<RuntimeAudioPlaybackInfo>,
    pub(super) frame_cadence_probe: Option<FrameCadenceProbe>,
    pub(super) startup_playback_smoke_delay_frames: Option<usize>,
    pub(super) renderer_smoke_screenshot_captured: bool,
    pub(super) support_id_copied: bool,
    pub(super) show_submit_dialog: bool,
    pub(super) submit_is_pending: bool,
    pub(super) submit_has_error: bool,
    pub(super) drop_claim_is_pending: bool,
    pub(super) show_skip_chunk_dialog: bool,
    pub(super) skip_is_pending: bool,
    pub(super) show_admin_flag_dialog: bool,
    pub(super) admin_flag_reason: String,
    pub(super) admin_flag_is_pending: bool,
    pub(super) no_work_modal: Option<NoWorkModalModel>,
    pub(super) feedback_panel: Option<FeedbackPanelState>,
    pub(super) task_lookup: Option<TaskLookupState>,
    pub(super) task_lookup_last_effect: Option<TaskLookupEffect>,
    pub(super) babel_service_requests: Vec<BabelServiceRequest>,
    pub(super) dispatched_babel_service_requests: Vec<BabelServiceRequest>,
    pub(super) review_backend_requests: Vec<ReviewBackendRequest>,
    pub(super) dispatched_review_backend_requests: Vec<ReviewBackendRequest>,
    pub(super) native_capability_panel_open: bool,
    pub(super) native_capability_modal_tab: NativeCapabilityModalTab,
    pub(super) reviewer_session_id: String,
    pub(super) reviewer_session_comment: String,
    pub(super) reviewer_card_id: String,
    pub(super) reviewer_card_comment: String,
    pub(super) reviewer_comments_dirty: bool,
    pub(super) reviewer_cards_expanded: bool,
    pub(super) reviewer_suggestions_expanded: bool,
    pub(super) reviewer_template_query: String,
    pub(super) reviewer_template_id: String,
    pub(super) reviewer_proposal_id: String,
    pub(super) reviewer_history_query: String,
    pub(super) draft_openrouter_api_key: String,
    pub(super) draft_model: String,
    pub(super) draft_service_tier: String,
    pub(super) draft_reasoning_effort: String,
    pub(super) draft_session_id: String,
    pub(super) native_capability_health: Option<NativeCapabilityHealth>,
    pub(super) reviewer_generated_review_preview: Option<ReviewerGeneratedReviewPreview>,
    pub(super) reviewer_finalize_preview: Option<ReviewerFinalizePreview>,
    pub(super) reviewer_session_preview: Option<ReviewerSessionPreview>,
    pub(super) reviewer_template_results: Vec<ReviewerTemplatePreview>,
    pub(super) draft_row_previews: Vec<DraftRowPreview>,
    pub(super) draft_summary_preview: Option<DraftSummaryPreview>,
    pub(super) draft_transcription_preview: Option<DraftTranscriptionPreview>,
    pub(super) draft_redistribution_previews: Vec<DraftRedistributionPreview>,
    pub(super) draft_audio_guard: Option<DraftAudioGuard>,
    pub(super) native_capability_last_response: Option<String>,
    pub(super) native_capability_last_error: Option<String>,
    pub(super) show_hotkeys_dialog: bool,
    pub(super) show_exit_confirm_dialog: bool,
    pub(super) playback_followed_segment_id: Option<String>,
    pub(super) find_bar_open: bool,
    pub(super) find_query: String,
    pub(super) create_region_dialog: Option<CreateRegionDialog>,
    pub(super) edit_annotation_dialog: Option<EditAnnotationDialog>,
    pub(super) transcript_scroll_offset_y: f32,
    pub(super) transcript_scroll_viewport_height: f32,
    pub(super) transcript_rows_generation: u64,
    pub(super) transcript_rows_cache: RefCell<TranscriptRowsRenderCache>,
    pub(super) transcript_layout_generation: u64,
    pub(super) transcript_layout_cache: RefCell<TranscriptLayoutRenderCache>,
    pub(super) timeline_lanes_generation: u64,
    pub(super) timeline_segment_index_cache: RefCell<TimelineSegmentIndexRenderCache>,
    pub(super) timeline_lanes_cache: RefCell<TimelineLanesRenderCache>,
    pub(super) timeline_static_block_prewarm_cache: RefCell<TimelineStaticBlockPrewarmCache>,
    pub(super) overview_waveform_cache: RefCell<OverviewWaveformRenderCache>,
    pub(super) transport_strip_cache: RefCell<TransportStripRenderCache>,
    pub(super) submit_control_cache: RefCell<SubmitControlRenderCache>,
}

impl Default for StudioState {
    fn default() -> Self {
        let workflow = load_sample_project()
            .map(StudioWorkflow::from_project)
            .map_err(|error| error.to_string());
        let editor = workflow
            .as_ref()
            .ok()
            .map(|workflow| EditorSession::for_project(workflow.project()));
        let feedback_panel = workflow
            .as_ref()
            .ok()
            .and_then(|workflow| FeedbackPanelState::from_project(workflow.project()));
        let task_lookup = workflow
            .as_ref()
            .ok()
            .and_then(|workflow| Self::task_lookup_from_project(workflow.project()));
        let status = match &workflow {
            Ok(_) => "Loaded bundled Babel sample project.".to_string(),
            Err(error) => format!("Could not load bundled Babel sample project: {error}"),
        };
        let panel_collapse_storage = PanelCollapseStorage::load_default();
        let script_context_panel_hidden =
            panel_collapse_storage.is_collapsed(SCRIPT_CONTEXT_STORAGE_KEY);
        let original_script_panel_hidden =
            panel_collapse_storage.is_collapsed(ORIGINAL_SCRIPT_STORAGE_KEY);
        let mut state = Self {
            app_surface: AppSurface::Dashboard,
            native_auth_flow: None,
            dashboard: DashboardState::default(),
            updater: UpdatePanelState::default(),
            workflow,
            editor,
            segment_drafts: BTreeMap::new(),
            keyboard_modifiers: Modifiers::empty(),
            right_shift_pressed: false,
            held_audition_keys: BTreeSet::new(),
            audition_key_release_tokens: BTreeMap::new(),
            last_transport_tick: None,
            large_project_live_pan_active: false,
            large_project_live_pan_frame: 0,
            large_project_live_transcript_scroll_active: false,
            large_project_live_transcript_scroll_frame: 0,
            waveform_hit_hover: None,
            active_overlay_drag: None,
            active_waveform_drag: None,
            active_waveform_create_drag: None,
            active_timeline_autoscroll: None,
            loop_strip_hover: None,
            active_loop_drag: None,
            last_global_pointer_position: None,
            overview_hover_fraction: None,
            overview_pointer_sample: None,
            overview_drag_capture: None,
            overview_drag_active: false,
            overview_drag_grab_offset_fraction: 0.0,
            timeline_scrollbar_hover_fraction: None,
            timeline_scrollbar_pointer_sample: None,
            timeline_scrollbar_drag_capture: None,
            timeline_scrollbar_drag_offset_fraction: None,
            lane_vertical_zoom_slider_hover: None,
            lane_vertical_zoom_slider_pointer_sample: None,
            lane_vertical_zoom_slider_drag_capture: None,
            active_lane_vertical_zoom_slider_id: None,
            lane_vertical_zoom_input: None,
            last_lane_vertical_zoom_slider_press: None,
            last_track_cell_press: None,
            last_transcript_row_press: None,
            transcript_row_hover_segment_id: None,
            tag_suggestion_state: None,
            helper_focused_segment_id: None,
            helper_focus_restore_pending: None,
            helper_last_focus_restore: None,
            audio_playback: AudioPlaybackBackend::default(),
            acknowledged_low_confidence_words: BTreeMap::new(),
            panel_collapse_storage,
            script_context_panel_hidden,
            original_script_panel_hidden,
            copied_submission_id: false,
            import_path: DEFAULT_IMPORT_PATH.to_string(),
            export_path: DEFAULT_EXPORT_PATH.to_string(),
            status,
            runtime_graphics_info: None,
            runtime_audio_playback_info: None,
            frame_cadence_probe: frame_cadence_probe_from_env(),
            startup_playback_smoke_delay_frames: None,
            renderer_smoke_screenshot_captured: false,
            support_id_copied: false,
            show_submit_dialog: false,
            submit_is_pending: false,
            submit_has_error: false,
            drop_claim_is_pending: false,
            show_skip_chunk_dialog: false,
            skip_is_pending: false,
            show_admin_flag_dialog: false,
            admin_flag_reason: String::new(),
            admin_flag_is_pending: false,
            no_work_modal: None,
            feedback_panel,
            task_lookup,
            task_lookup_last_effect: None,
            babel_service_requests: Vec::new(),
            dispatched_babel_service_requests: Vec::new(),
            review_backend_requests: Vec::new(),
            dispatched_review_backend_requests: Vec::new(),
            native_capability_panel_open: false,
            native_capability_modal_tab: NativeCapabilityModalTab::Reviewer,
            reviewer_session_id: String::new(),
            reviewer_session_comment: String::new(),
            reviewer_card_id: String::new(),
            reviewer_card_comment: String::new(),
            reviewer_comments_dirty: false,
            reviewer_cards_expanded: false,
            reviewer_suggestions_expanded: true,
            reviewer_template_query: String::new(),
            reviewer_template_id: String::new(),
            reviewer_proposal_id: String::new(),
            reviewer_history_query: String::new(),
            draft_openrouter_api_key: String::new(),
            draft_model: String::new(),
            draft_service_tier: "default".to_string(),
            draft_reasoning_effort: "default".to_string(),
            draft_session_id: String::new(),
            native_capability_health: None,
            reviewer_generated_review_preview: None,
            reviewer_finalize_preview: None,
            reviewer_session_preview: None,
            reviewer_template_results: Vec::new(),
            draft_row_previews: Vec::new(),
            draft_summary_preview: None,
            draft_transcription_preview: None,
            draft_redistribution_previews: Vec::new(),
            draft_audio_guard: None,
            native_capability_last_response: None,
            native_capability_last_error: None,
            show_hotkeys_dialog: false,
            show_exit_confirm_dialog: false,
            playback_followed_segment_id: None,
            find_bar_open: false,
            find_query: String::new(),
            create_region_dialog: None,
            edit_annotation_dialog: None,
            transcript_scroll_offset_y: 0.0,
            transcript_scroll_viewport_height: DEFAULT_TRANSCRIPT_VIEWPORT_HEIGHT,
            transcript_rows_generation: 0,
            transcript_rows_cache: RefCell::new(TranscriptRowsRenderCache::default()),
            transcript_layout_generation: 0,
            transcript_layout_cache: RefCell::new(TranscriptLayoutRenderCache::default()),
            timeline_lanes_generation: 0,
            timeline_segment_index_cache: RefCell::new(TimelineSegmentIndexRenderCache::default()),
            timeline_lanes_cache: RefCell::new(TimelineLanesRenderCache::default()),
            timeline_static_block_prewarm_cache: RefCell::new(
                TimelineStaticBlockPrewarmCache::default(),
            ),
            overview_waveform_cache: RefCell::new(OverviewWaveformRenderCache::default()),
            transport_strip_cache: RefCell::new(TransportStripRenderCache::default()),
            submit_control_cache: RefCell::new(SubmitControlRenderCache::default()),
        };
        state.update_feedback_submit_metadata();
        state.reset_segment_drafts();
        state.refresh_dashboard_from_project_metadata();
        state.hydrate_reviewer_selection_from_project_metadata();
        state.hydrate_timeline_defaults_from_project_metadata();
        state.load_lane_waveforms_from_base(bundled_assets_dir());
        state
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct WaveformHitHover {
    pub(super) lane_id: String,
    pub(super) hit: WaveformHit,
    pub(super) fraction: f32,
    pub(super) region_label: Option<String>,
    pub(super) region_tooltip: Option<String>,
}

pub(super) const WAVEFORM_MAGNIFIER_SCALE: f32 = 3.0;
pub(super) const WAVEFORM_MAGNIFIER_WIDTH_PX: f32 = 180.0;
pub(super) const WAVEFORM_MAGNIFIER_MAX_HEIGHT_PX: f32 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WaveformMagnifierOverlay {
    pub(super) time_seconds: f64,
    pub(super) fraction: f32,
    pub(super) scale: f32,
    pub(super) width_px: f32,
    pub(super) max_height_px: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct WaveformDrag {
    pub(super) lane_id: String,
    pub(super) segment_id: String,
    pub(super) target: WaveformHitTarget,
    pub(super) grab_time_seconds: f64,
    pub(super) original_start_seconds: f64,
    pub(super) original_end_seconds: f64,
    pub(super) preview_start_seconds: f64,
    pub(super) preview_end_seconds: f64,
    pub(super) has_moved: bool,
}

impl WaveformDrag {
    pub(super) fn timeline_preview(&self) -> TimelineSegmentPreview {
        TimelineSegmentPreview {
            segment_id: self.segment_id.clone(),
            lane_id: self.lane_id.clone(),
            start_seconds: self.preview_start_seconds,
            end_seconds: self.preview_end_seconds,
        }
    }

    pub(super) fn preview_differs_from_original(&self) -> bool {
        self.preview_start_seconds.to_bits() != self.original_start_seconds.to_bits()
            || self.preview_end_seconds.to_bits() != self.original_end_seconds.to_bits()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct OverlayDrag {
    pub(super) lane_id: String,
    pub(super) target: OverlayDragTarget,
    pub(super) grab_time_seconds: f64,
    pub(super) start_fraction: f32,
    pub(super) original_start_seconds: f64,
    pub(super) original_end_seconds: f64,
    pub(super) has_moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OverlayDragTarget {
    Create,
    Body,
    StartHandle,
    EndHandle,
    Locked,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LaneVerticalZoomSliderHover {
    pub(super) lane_id: String,
    pub(super) fraction: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct OverviewPointerSample {
    pub(super) fraction: f32,
    pub(super) width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LaneVerticalZoomSliderPointerSample {
    pub(super) lane_id: String,
    pub(super) fraction: f32,
    pub(super) height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DragAxisCapture {
    pub(super) origin_px: f32,
    pub(super) extent_px: f32,
}

impl DragAxisCapture {
    fn from_pointer_sample(pointer_px: f32, fraction: f32, extent_px: f32) -> Self {
        let extent_px = extent_px.max(1.0);
        Self {
            origin_px: pointer_px - sanitize_fraction(fraction) * extent_px,
            extent_px,
        }
    }

    fn fraction_at(&self, pointer_px: f32) -> f32 {
        sanitize_fraction((pointer_px - self.origin_px) / self.extent_px)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LaneVerticalZoomSliderDragCapture {
    pub(super) lane_id: String,
    pub(super) axis: DragAxisCapture,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LaneVerticalZoomInput {
    pub(super) lane_id: String,
    pub(super) value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LaneVerticalZoomSliderPress {
    pub(super) lane_id: String,
    pub(super) at: Instant,
    pub(super) zoom_before: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct WaveformCreateDrag {
    pub(super) lane_id: String,
    pub(super) start_seconds: f64,
    pub(super) current_seconds: f64,
    pub(super) start_fraction: f32,
    pub(super) current_fraction: f32,
    pub(super) has_moved: bool,
}

pub(super) const WAVEFORM_CREATE_PREVIEW_SEGMENT_ID: &str = "__studio_create_preview__";

impl WaveformCreateDrag {
    pub(super) fn timeline_preview(&self) -> Option<TimelineSegmentPreview> {
        if !self.has_moved {
            return None;
        }

        let start_seconds = self.start_seconds.min(self.current_seconds).max(0.0);
        let end_seconds = self.start_seconds.max(self.current_seconds).max(0.0);
        if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds >= end_seconds {
            return None;
        }

        Some(TimelineSegmentPreview {
            segment_id: WAVEFORM_CREATE_PREVIEW_SEGMENT_ID.to_string(),
            lane_id: self.lane_id.clone(),
            start_seconds,
            end_seconds,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimelineAutoscrollMode {
    Held,
    Click,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct TimelineAutoscroll {
    pub(super) lane_id: String,
    pub(super) anchor_fraction: f32,
    pub(super) target_fraction: f32,
    pub(super) mode: TimelineAutoscrollMode,
    pub(super) moved_while_held: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LoopStripDrag {
    pub(super) target: LoopStripHitTarget,
    pub(super) grab_time_seconds: f64,
    pub(super) original_start_seconds: f64,
    pub(super) original_end_seconds: f64,
}

#[derive(Debug, Clone)]
pub(super) struct TrackCellPress {
    pub(super) segment_id: String,
    pub(super) at: Instant,
}

#[derive(Debug, Clone)]
pub(super) struct TranscriptRowPress {
    pub(super) segment_id: String,
    pub(super) at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TagSuggestionUiState {
    pub(super) segment_id: String,
    text: String,
    cursor_position: (usize, usize),
    pub(super) highlighted_index: usize,
    pub(super) dismissed: bool,
}

impl TagSuggestionUiState {
    fn for_draft(
        segment_id: &str,
        draft: &SegmentDraft,
        highlighted_index: usize,
        dismissed: bool,
    ) -> Self {
        Self {
            segment_id: segment_id.to_string(),
            text: draft.editor_text(),
            cursor_position: draft.text_content.cursor_position(),
            highlighted_index,
            dismissed,
        }
    }

    fn matches_draft(&self, segment_id: &str, draft: &SegmentDraft) -> bool {
        self.segment_id == segment_id
            && self.text == draft.editor_text()
            && self.cursor_position == draft.text_content.cursor_position()
    }
}

#[derive(Debug, Default)]
pub(super) struct TranscriptRowsRenderCache {
    entry: Option<TranscriptRowsCacheEntry>,
    miss_count: usize,
    last_trace: Option<TranscriptRowsCacheTrace>,
}

#[derive(Debug, Default)]
pub(super) struct TranscriptLayoutRenderCache {
    entry: Option<TranscriptLayoutCacheEntry>,
    miss_count: usize,
}

#[derive(Debug, Default)]
pub(super) struct TimelineLanesRenderCache {
    entry: Option<TimelineLanesCacheEntry>,
    miss_count: usize,
}

impl TimelineLanesRenderCache {
    fn get(&mut self, key: &TimelineLanesCacheKey) -> Option<Arc<[TimelineLaneView]>> {
        self.entry
            .as_ref()
            .filter(|entry| entry.key == *key)
            .map(|entry| Arc::clone(&entry.lanes))
    }

    fn store(&mut self, key: TimelineLanesCacheKey, lanes: Arc<[TimelineLaneView]>) {
        self.entry = Some(TimelineLanesCacheEntry { key, lanes });
    }

    fn clear(&mut self) {
        self.entry = None;
    }
}

#[derive(Debug, Default)]
pub(super) struct TimelineStaticBlockPrewarmCache {
    generation: u64,
    prewarmed_lanes: BTreeSet<TimelineStaticBlockPrewarmLaneKey>,
    strip_blocks: BTreeMap<TimelineStaticBlockPrewarmLaneKey, Arc<[WaveformBlock]>>,
}

impl TimelineStaticBlockPrewarmCache {
    fn clear(&mut self) {
        self.prewarmed_lanes.clear();
        self.strip_blocks.clear();
    }
}

#[derive(Debug, Default)]
pub(super) struct TimelineSegmentIndexRenderCache {
    entry: Option<TimelineSegmentIndexCacheEntry>,
    miss_count: usize,
}

#[derive(Debug, Default)]
pub(super) struct OverviewWaveformRenderCache {
    entry: Option<OverviewWaveformCacheEntry>,
    miss_count: usize,
}

#[derive(Debug, Default)]
pub(super) struct TransportStripRenderCache {
    entries: Vec<TransportStripCacheEntry>,
    miss_count: usize,
}

#[derive(Debug, Default)]
pub(super) struct SubmitControlRenderCache {
    entry: Option<SubmitControlCacheEntry>,
    miss_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RuntimeAudioPlaybackInfo {
    pub(super) track_count: usize,
    pub(super) start_seconds: f64,
    pub(super) speed: f32,
}

#[derive(Clone, Debug)]
pub(super) struct OverviewWaveformSnapshot {
    pub(super) overview: Arc<OverviewWaveformView>,
    pub(super) bounds: TimeRange,
}

#[derive(Debug)]
struct TranscriptRowsCacheEntry {
    key: TranscriptRowsCacheKey,
    rows: Arc<[TranscriptRowView]>,
}

#[derive(Debug)]
struct TranscriptLayoutCacheEntry {
    key: TranscriptLayoutCacheKey,
    scroll_offset_bits: u32,
    viewport_height_bits: u32,
    pinned_row_index: Option<usize>,
    layout: TranscriptLayoutView,
}

#[derive(Debug)]
struct TimelineLanesCacheEntry {
    key: TimelineLanesCacheKey,
    lanes: Arc<[TimelineLaneView]>,
}

#[derive(Debug)]
struct TimelineSegmentIndexCacheEntry {
    generation: u64,
    index: Arc<TimelineSegmentIndex>,
    bounds: TimeRange,
}

#[derive(Debug)]
struct OverviewWaveformCacheEntry {
    key: OverviewWaveformCacheKey,
    dynamic_key: OverviewWaveformDynamicCacheKey,
    bounds: TimeRange,
    overview: Arc<OverviewWaveformView>,
}

#[derive(Debug)]
struct TransportStripCacheEntry {
    key: TransportStripCacheKey,
    strip: Arc<TransportStripView>,
}

#[derive(Debug)]
struct SubmitControlCacheEntry {
    key: SubmitControlCacheKey,
    submit: Arc<SubmitControlView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TranscriptRowsCacheKey {
    generation: u64,
    track_filter: TrackFilter,
    diff_view: bool,
    asserted_warning_keys: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TranscriptLayoutView {
    pub(super) row_heights: Arc<[f32]>,
    pub(super) virtual_window: TranscriptVirtualWindow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TranscriptLayoutCacheKey {
    rows: TranscriptRowsCacheKey,
    layout_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineLanesCacheKey {
    generation: u64,
    track_filter: TrackFilter,
    collapsed_lane_ids: BTreeSet<String>,
    muted_lane_ids: BTreeSet<String>,
    lane_vertical_zoom: Vec<(String, u32)>,
    lane_waveforms: Vec<(String, u64)>,
    viewport_start_bits: u64,
    viewport_end_bits: u64,
    overlay_range: Option<(u64, u64)>,
    segment_preview: Option<TimelineSegmentPreviewCacheKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TimelineStaticBlockPrewarmLaneKey {
    lane_id: String,
    waveform_duration_bits: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineSegmentPreviewCacheKey {
    segment_id: String,
    lane_id: String,
    start_seconds_bits: u64,
    end_seconds_bits: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OverviewWaveformCacheKey {
    generation: u64,
    track_filter: TrackFilter,
    lane_waveforms: Vec<(String, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OverviewWaveformDynamicCacheKey {
    viewport_start_bits: u64,
    viewport_end_bits: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TransportStripCacheKey {
    generation: u64,
    track_filter: TrackFilter,
    asserted_warning_keys: BTreeSet<String>,
    is_playing: bool,
    is_dirty: bool,
    is_saving: bool,
    save_has_error: bool,
    review_state: String,
    can_undo: bool,
    can_redo: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SubmitControlCacheKey {
    generation: u64,
    asserted_warning_keys: BTreeSet<String>,
    show_submit_dialog: bool,
    submit_is_pending: bool,
    submit_has_error: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TranscriptRowsCacheTrace {
    pub(super) event: TranscriptRowsCacheTraceEvent,
    pub(super) generation: u64,
    pub(super) row_count: usize,
    pub(super) miss_count: usize,
    pub(super) playing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TranscriptRowsCacheTraceEvent {
    Hit,
    Miss,
}

impl StudioState {
    pub(super) fn project(&self) -> Option<&Project> {
        self.workflow.as_ref().ok().map(StudioWorkflow::project)
    }

    pub(super) fn reviewer_capability_available(&self) -> bool {
        self.project()
            .and_then(project_review_action_level)
            .is_some_and(|level| level == REVIEWER_CAPABILITY_ACTION_LEVEL)
    }

    pub(super) fn set_native_capability_modal_tab(&mut self, tab: NativeCapabilityModalTab) {
        if tab == NativeCapabilityModalTab::Reviewer && !self.reviewer_capability_available() {
            self.native_capability_modal_tab = NativeCapabilityModalTab::GoldDrafting;
            self.status = "Reviewer is only available for L2 review tasks.".to_string();
            return;
        }
        self.native_capability_modal_tab = tab;
    }

    pub(super) fn draft_audio_cue_status(&self) -> Option<DraftAudioCueStatus> {
        let project = self.project()?;
        let editor = self.editor.as_ref()?;
        let mut missing_lane_labels = Vec::new();
        let mut missing_lane_sources = Vec::new();
        let mut loaded_tracks = 0;

        for lane in &project.lanes {
            if editor
                .lane_waveforms
                .get(&lane.id)
                .and_then(|waveform| waveform.decoded_audio())
                .is_some()
            {
                loaded_tracks += 1;
            } else {
                missing_lane_labels.push(draft_audio_lane_label(lane));
                missing_lane_sources.push(lane.source_path.clone());
            }
        }

        Some(DraftAudioCueStatus {
            loaded_tracks,
            total_lanes: project.lanes.len(),
            missing_lane_labels,
            missing_lane_sources,
        })
    }

    pub(super) fn can_undo(&self) -> bool {
        matches!(&self.workflow, Ok(workflow) if workflow.can_undo())
    }

    pub(super) fn can_redo(&self) -> bool {
        matches!(&self.workflow, Ok(workflow) if workflow.can_redo())
    }

    pub(super) fn project_lint_tag_config(&self) -> BabelProjectLintTagConfig {
        self.project()
            .map(resolve_babel_project_lint_tag_config)
            .unwrap_or_default()
    }

    pub(super) fn should_show_tag_suggestions(
        &self,
        segment_id: &str,
        draft: &SegmentDraft,
    ) -> bool {
        !self
            .tag_suggestion_state
            .as_ref()
            .is_some_and(|state| state.matches_draft(segment_id, draft) && state.dismissed)
    }

    pub(super) fn tag_suggestion_highlighted_index(
        &self,
        segment_id: &str,
        draft: &SegmentDraft,
        suggestion_count: usize,
    ) -> usize {
        if suggestion_count == 0 {
            return 0;
        }

        self.tag_suggestion_state
            .as_ref()
            .filter(|state| state.matches_draft(segment_id, draft) && !state.dismissed)
            .map(|state| state.highlighted_index.min(suggestion_count - 1))
            .unwrap_or(0)
    }

    pub(super) fn clear_tag_suggestion_state_for_segment(&mut self, segment_id: &str) {
        if self
            .tag_suggestion_state
            .as_ref()
            .is_some_and(|state| state.segment_id == segment_id)
        {
            self.tag_suggestion_state = None;
        }
    }

    pub(super) fn handle_tag_suggestion_keyboard_action(
        &mut self,
        segment_id: &str,
        action: TagSuggestionKeyboardAction,
    ) {
        let Some(draft) = self.segment_drafts.get(segment_id) else {
            self.clear_tag_suggestion_state_for_segment(segment_id);
            return;
        };
        let suggestions = tag_suggestions::tag_suggestions_for_draft_with_config(
            draft,
            self.project_lint_tag_config(),
        );
        if suggestions.is_empty() {
            self.clear_tag_suggestion_state_for_segment(segment_id);
            return;
        }

        let last_index = suggestions.len() - 1;
        let current_index =
            self.tag_suggestion_highlighted_index(segment_id, draft, suggestions.len());
        let next_index = match action {
            TagSuggestionKeyboardAction::Previous => current_index.saturating_sub(1),
            TagSuggestionKeyboardAction::Next => (current_index + 1).min(last_index),
            TagSuggestionKeyboardAction::Accept => current_index,
            TagSuggestionKeyboardAction::Dismiss => current_index,
        };

        if action == TagSuggestionKeyboardAction::Accept {
            let suggestion = suggestions[next_index].clone();
            if let Some(draft) = self.segment_drafts.get_mut(segment_id) {
                let before = draft.text.clone();
                tag_suggestions::apply_tag_suggestion(draft, &suggestion);
                if draft.text != before {
                    self.mark_dirty();
                }
            }
            self.clear_tag_suggestion_state_for_segment(segment_id);
            return;
        }

        let Some(draft) = self.segment_drafts.get(segment_id) else {
            self.clear_tag_suggestion_state_for_segment(segment_id);
            return;
        };
        self.tag_suggestion_state = Some(TagSuggestionUiState::for_draft(
            segment_id,
            draft,
            next_index,
            action == TagSuggestionKeyboardAction::Dismiss,
        ));
    }

    pub(super) fn reset_segment_drafts(&mut self) {
        self.segment_drafts.clear();
        self.tag_suggestion_state = None;
        if let Some(project) = self.project() {
            self.segment_drafts = project
                .segments
                .iter()
                .map(|segment| (segment.id.clone(), SegmentDraft::from_segment(segment)))
                .collect();
        }
        self.invalidate_transcript_rows_cache();
    }

    pub(super) fn sync_segment_drafts_preserving_existing(&mut self) {
        let Some(project) = self.project() else {
            self.segment_drafts.clear();
            self.tag_suggestion_state = None;
            self.invalidate_transcript_rows_cache();
            return;
        };
        let current_segment_ids: BTreeSet<_> = project
            .segments
            .iter()
            .map(|segment| segment.id.clone())
            .collect();
        let missing_drafts: Vec<_> = project
            .segments
            .iter()
            .filter(|segment| !self.segment_drafts.contains_key(&segment.id))
            .map(|segment| (segment.id.clone(), SegmentDraft::from_segment(segment)))
            .collect();
        for (segment_id, draft) in missing_drafts {
            self.segment_drafts.entry(segment_id).or_insert(draft);
        }
        self.segment_drafts
            .retain(|segment_id, _| current_segment_ids.contains(segment_id));
        if self
            .tag_suggestion_state
            .as_ref()
            .is_some_and(|state| !current_segment_ids.contains(&state.segment_id))
        {
            self.tag_suggestion_state = None;
        }
        self.invalidate_transcript_rows_cache();
    }

    pub(super) fn refresh_segment_draft(&mut self, segment_id: &str) {
        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .cloned()
        else {
            self.segment_drafts.remove(segment_id);
            self.clear_tag_suggestion_state_for_segment(segment_id);
            self.invalidate_transcript_rows_cache();
            return;
        };
        self.segment_drafts
            .insert(segment_id.to_string(), SegmentDraft::from_segment(&segment));
        self.clear_tag_suggestion_state_for_segment(segment_id);
        self.invalidate_transcript_rows_cache();
    }

    pub(super) fn replace_workflow(&mut self, workflow: StudioWorkflow, status: String) {
        self.audio_playback.stop();
        self.editor = Some(EditorSession::for_project(workflow.project()));
        self.workflow = Ok(workflow);
        self.app_surface = AppSurface::Editor;
        self.status = status;
        self.show_submit_dialog = false;
        self.submit_is_pending = false;
        self.submit_has_error = false;
        self.drop_claim_is_pending = false;
        self.show_skip_chunk_dialog = false;
        self.skip_is_pending = false;
        self.show_admin_flag_dialog = false;
        self.admin_flag_reason.clear();
        self.admin_flag_is_pending = false;
        self.no_work_modal = None;
        self.feedback_panel = self.project().and_then(FeedbackPanelState::from_project);
        self.update_feedback_submit_metadata();
        self.task_lookup = self.project().and_then(Self::task_lookup_from_project);
        self.task_lookup_last_effect = None;
        self.hydrate_reviewer_selection_from_project_metadata();
        self.hydrate_timeline_defaults_from_project_metadata();
        self.show_hotkeys_dialog = false;
        self.create_region_dialog = None;
        self.edit_annotation_dialog = None;
        self.transcript_scroll_offset_y = 0.0;
        self.transcript_scroll_viewport_height = DEFAULT_TRANSCRIPT_VIEWPORT_HEIGHT;
        self.copied_submission_id = false;
        self.runtime_audio_playback_info = None;
        self.active_overlay_drag = None;
        self.active_waveform_drag = None;
        self.active_waveform_create_drag = None;
        self.overview_hover_fraction = None;
        self.transcript_row_hover_segment_id = None;
        self.tag_suggestion_state = None;
        self.helper_focused_segment_id = None;
        self.helper_focus_restore_pending = None;
        self.helper_last_focus_restore = None;
        self.overview_drag_active = false;
        self.held_audition_keys.clear();
        self.acknowledged_low_confidence_words.clear();
        self.reset_segment_drafts();
    }

    pub(super) fn complete_claim_next_from_queue_failure(&mut self, error: String) {
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::ClaimNextFromQueue(_))
        });
        self.no_work_modal = Some(build_no_work_modal_model(NoWorkModalInput {
            open: true,
            mode: "review".to_string(),
        }));
        self.app_surface = AppSurface::Dashboard;
        self.dashboard.is_pending = false;
        self.status = if error.trim().is_empty() {
            "No tasks available".to_string()
        } else {
            error
        };
        self.dashboard.error = Some(self.status.clone());
    }

    pub(super) fn complete_load_review_action_success(&mut self, source: String) -> Task<Message> {
        self.complete_babel_payload_load_success(
            source,
            BabelPayloadLoadKind::ReviewAction,
            |request| matches!(request, BabelServiceRequest::LoadReviewAction(_)),
        )
    }

    pub(super) fn complete_load_latest_review_action_success(
        &mut self,
        source: String,
    ) -> Task<Message> {
        self.complete_babel_payload_load_success(
            source,
            BabelPayloadLoadKind::LatestReviewAction,
            |request| matches!(request, BabelServiceRequest::LoadLatestReviewAction(_)),
        )
    }

    pub(super) fn complete_claim_by_processed_transcription_success(
        &mut self,
        source: String,
    ) -> Task<Message> {
        self.complete_babel_payload_load_success(
            source,
            BabelPayloadLoadKind::ClaimByProcessedTranscription,
            |request| {
                matches!(
                    request,
                    BabelServiceRequest::ClaimByProcessedTranscription(_)
                )
            },
        )
    }

    pub(super) fn complete_claim_next_from_queue_success(
        &mut self,
        source: String,
    ) -> Task<Message> {
        self.complete_babel_payload_load_success(
            source,
            BabelPayloadLoadKind::ClaimNextFromQueue,
            |request| matches!(request, BabelServiceRequest::ClaimNextFromQueue(_)),
        )
    }

    pub(super) fn fail_load_review_action(&mut self, error: String) {
        self.complete_babel_payload_load_failure(error, "Review action load", |request| {
            matches!(request, BabelServiceRequest::LoadReviewAction(_))
        });
    }

    pub(super) fn fail_load_latest_review_action(&mut self, error: String) {
        self.complete_babel_payload_load_failure(
            error,
            "Processed transcription load",
            |request| matches!(request, BabelServiceRequest::LoadLatestReviewAction(_)),
        );
    }

    pub(super) fn fail_claim_by_processed_transcription(&mut self, error: String) {
        self.complete_babel_payload_load_failure(
            error,
            "Stitched transcription claim",
            |request| {
                matches!(
                    request,
                    BabelServiceRequest::ClaimByProcessedTranscription(_)
                )
            },
        );
    }

    fn complete_babel_payload_load_success(
        &mut self,
        source: String,
        kind: BabelPayloadLoadKind,
        should_drain: impl FnMut(&BabelServiceRequest) -> bool,
    ) -> Task<Message> {
        let route_metadata = self.project_metadata_for_keys(BABEL_ROUTE_METADATA_KEYS);
        self.drain_babel_service_requests_matching(should_drain);

        if babel_payload_needs_audio_preparation(&source) {
            self.status = "Downloading Babel audio...".to_string();
            self.app_surface = AppSurface::Dashboard;
            self.dashboard.is_pending = true;
            self.dashboard.error = None;
            return Task::perform(prepare_babel_audio_payload(source), move |result| {
                Message::BabelAudioPrepared {
                    kind,
                    route_metadata: route_metadata.clone(),
                    result,
                }
            });
        }

        self.import_prepared_babel_payload(
            PreparedBabelAudioPayload::local(source),
            route_metadata,
            kind,
        );
        Task::none()
    }

    pub(super) fn complete_babel_audio_prepared(
        &mut self,
        kind: BabelPayloadLoadKind,
        route_metadata: Vec<(String, String)>,
        result: Result<PreparedBabelAudioPayload, String>,
    ) {
        match result {
            Ok(payload) => self.import_prepared_babel_payload(payload, route_metadata, kind),
            Err(error) => {
                self.status = format!("{} failed: {error}", kind.failure_label());
                self.app_surface = AppSurface::Dashboard;
                self.dashboard.is_pending = false;
                self.dashboard.error = Some(self.status.clone());
            }
        }
    }

    fn import_prepared_babel_payload(
        &mut self,
        payload: PreparedBabelAudioPayload,
        route_metadata: Vec<(String, String)>,
        kind: BabelPayloadLoadKind,
    ) {
        match StudioWorkflow::from_babel_json(&payload.source).map_err(|error| error.to_string()) {
            Ok(mut workflow) => {
                workflow.merge_project_metadata(route_metadata);
                let lint_tag_metadata = babel_project_lint_tag_metadata(workflow.project());
                workflow.merge_project_metadata(lint_tag_metadata);
                self.replace_workflow(workflow, kind.success_status().to_string());
                let loaded = self.load_lane_waveforms_from_base(&payload.audio_base_dir);
                self.append_waveform_status(loaded);
                if payload.remote_audio_count > 0 && loaded == 0 {
                    self.status = format!(
                        "{} failed: downloaded Babel audio, but no waveform tracks loaded",
                        kind.failure_label()
                    );
                    self.app_surface = AppSurface::Dashboard;
                    self.dashboard.is_pending = false;
                    self.dashboard.error = Some(self.status.clone());
                    return;
                }
                self.app_surface = AppSurface::Editor;
                self.dashboard.is_pending = false;
                self.dashboard.error = None;
            }
            Err(error) => {
                self.status = format!("{} failed: {error}", kind.failure_label());
                self.app_surface = AppSurface::Dashboard;
                self.dashboard.is_pending = false;
                self.dashboard.error = Some(self.status.clone());
            }
        }
    }

    fn complete_babel_payload_load_failure(
        &mut self,
        error: String,
        failure_label: &str,
        should_drain: impl FnMut(&BabelServiceRequest) -> bool,
    ) {
        self.drain_babel_service_requests_matching(should_drain);
        self.status = format!("{failure_label} failed: {error}");
        self.app_surface = AppSurface::Dashboard;
        self.dashboard.is_pending = false;
        self.dashboard.error = Some(self.status.clone());
    }

    fn project_metadata_for_keys(&self, keys: &[&str]) -> Vec<(String, String)> {
        let Some(project) = self.project() else {
            return Vec::new();
        };
        keys.iter()
            .filter_map(|key| {
                project
                    .metadata
                    .get(*key)
                    .map(|value| ((*key).to_string(), value.clone()))
            })
            .collect()
    }

    pub(super) fn drain_babel_service_requests_matching(
        &mut self,
        mut should_drain: impl FnMut(&BabelServiceRequest) -> bool,
    ) {
        self.babel_service_requests
            .retain(|request| !should_drain(request));
        self.dispatched_babel_service_requests
            .retain(|request| !should_drain(request));
    }

    pub(super) fn dispatch_new_babel_service_requests(&mut self) -> Task<Message> {
        if self.native_auth_flow_blocks_service_dispatch() {
            return Task::none();
        }

        let new_requests: Vec<_> = self
            .babel_service_requests
            .iter()
            .filter(|request| is_dispatchable_babel_service_request(request))
            .filter(|request| !self.dispatched_babel_service_requests.contains(request))
            .cloned()
            .collect();

        if new_requests.is_empty() {
            return Task::none();
        }

        self.dispatched_babel_service_requests
            .extend(new_requests.iter().cloned());
        if std::env::var_os("STUDIO_LOG_BABEL_SERVICE_DISPATCH").is_some() {
            eprintln!("Transcription Studio Babel service dispatch: requests={new_requests:?}");
        }
        dispatch_babel_service_requests(new_requests)
    }

    pub(super) fn dispatch_new_review_backend_requests(&mut self) -> Task<Message> {
        let new_requests: Vec<_> = self
            .review_backend_requests
            .iter()
            .filter(|request| !self.dispatched_review_backend_requests.contains(request))
            .cloned()
            .collect();

        if new_requests.is_empty() {
            return Task::none();
        }

        self.dispatched_review_backend_requests
            .extend(new_requests.iter().cloned());
        if std::env::var_os("STUDIO_LOG_REVIEW_BACKEND_DISPATCH").is_some() {
            eprintln!("Transcription Studio review backend dispatch: requests={new_requests:?}");
        }
        dispatch_review_backend_requests(new_requests)
    }

    pub(super) fn transcript_rows_for_render(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Vec<TranscriptRowView> {
        self.transcript_rows_snapshot_for_render(project, editor)
            .iter()
            .cloned()
            .collect()
    }

    pub(super) fn transcript_rows_snapshot_for_render(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Arc<[TranscriptRowView]> {
        let key = TranscriptRowsCacheKey::from_editor(self.transcript_rows_generation, editor);
        let mut cache = self.transcript_rows_cache.borrow_mut();
        if let Some(entry) = cache.entry.as_ref()
            && entry.key == key
        {
            let rows = Arc::clone(&entry.rows);
            let miss_count = cache.miss_count;
            cache.record_trace(TranscriptRowsCacheTrace::new(
                TranscriptRowsCacheTraceEvent::Hit,
                &key,
                rows.len(),
                miss_count,
                editor.is_playing,
            ));
            return rows;
        }

        let rows = Arc::<[TranscriptRowView]>::from(editor.transcript_rows_for_render(project));
        cache.miss_count += 1;
        let miss_count = cache.miss_count;
        cache.entry = Some(TranscriptRowsCacheEntry {
            key: key.clone(),
            rows: Arc::clone(&rows),
        });
        cache.record_trace(TranscriptRowsCacheTrace::new(
            TranscriptRowsCacheTraceEvent::Miss,
            &key,
            rows.len(),
            miss_count,
            editor.is_playing,
        ));
        rows
    }

    pub(super) fn timeline_lanes_snapshot_for_render(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Arc<[TimelineLaneView]> {
        let segment_preview = self
            .active_waveform_drag
            .as_ref()
            .map(WaveformDrag::timeline_preview)
            .or_else(|| {
                self.active_waveform_create_drag
                    .as_ref()
                    .and_then(WaveformCreateDrag::timeline_preview)
            });
        let key = TimelineLanesCacheKey::from_editor(
            self.timeline_lanes_generation,
            editor,
            segment_preview.as_ref(),
        );
        {
            let mut cache = self.timeline_lanes_cache.borrow_mut();
            if let Some(lanes) = cache.get(&key) {
                return lanes;
            }
        }

        let strip_blocks = self.prewarm_timeline_static_blocks_for_render(project, editor);
        let (segment_index, bounds) = self.timeline_segment_index_and_bounds_for_render(project);
        let lanes = Arc::<[TimelineLaneView]>::from(
            editor.timeline_lanes_with_segment_index_bounds_and_preview(
                project,
                segment_index.as_ref(),
                bounds,
                Some(&strip_blocks),
                segment_preview.as_ref(),
            ),
        );
        let mut cache = self.timeline_lanes_cache.borrow_mut();
        cache.miss_count += 1;
        cache.store(key, Arc::clone(&lanes));
        lanes
    }

    fn prewarm_timeline_static_blocks_for_render(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> BTreeMap<String, Arc<[WaveformBlock]>> {
        let mut cache = self.timeline_static_block_prewarm_cache.borrow_mut();
        if cache.generation != self.timeline_lanes_generation {
            cache.generation = self.timeline_lanes_generation;
            cache.clear();
        }

        let mut strip_blocks_for_lanes = BTreeMap::new();
        for lane in project
            .lanes
            .iter()
            .filter(|lane| match &editor.track_filter {
                TrackFilter::All => true,
                TrackFilter::Lane { id, .. } => id == &lane.id,
            })
        {
            let Some(waveform) = editor.lane_waveforms.get(&lane.id) else {
                continue;
            };
            let prewarm_key = TimelineStaticBlockPrewarmLaneKey {
                lane_id: lane.id.clone(),
                waveform_duration_bits: waveform.duration_seconds().to_bits(),
            };
            if cache.prewarmed_lanes.contains(&prewarm_key)
                && waveform.cached_static_block_count() > 0
                && let Some(strip_blocks) = cache.strip_blocks.get(&prewarm_key)
            {
                strip_blocks_for_lanes.insert(lane.id.clone(), Arc::clone(strip_blocks));
                continue;
            }

            let segments = project
                .segments
                .iter()
                .filter(|segment| segment.lane_id == lane.id)
                .collect::<Vec<_>>();
            let _ = waveform.prewarm_blocks_for_segments(segments.iter().copied());
            let strip_viewport = TimeRange {
                start_seconds: 0.0,
                end_seconds: waveform.duration_seconds().max(0.001),
            };
            let strip_blocks =
                waveform.blocks_for_segments(segments.iter().copied(), strip_viewport);
            cache
                .strip_blocks
                .insert(prewarm_key.clone(), Arc::clone(&strip_blocks));
            cache.prewarmed_lanes.insert(prewarm_key);
            strip_blocks_for_lanes.insert(lane.id.clone(), strip_blocks);
        }
        strip_blocks_for_lanes
    }

    fn timeline_segment_index_and_bounds_for_render(
        &self,
        project: &Project,
    ) -> (Arc<TimelineSegmentIndex>, TimeRange) {
        let generation = self.transcript_rows_generation;
        let mut cache = self.timeline_segment_index_cache.borrow_mut();
        if let Some(entry) = cache.entry.as_ref()
            && entry.generation == generation
        {
            return (Arc::clone(&entry.index), entry.bounds);
        }

        let bounds = TimeRange::for_project(project);
        let index = Arc::new(TimelineSegmentIndex::for_project(project));
        cache.miss_count += 1;
        cache.entry = Some(TimelineSegmentIndexCacheEntry {
            generation,
            bounds,
            index: Arc::clone(&index),
        });
        (index, bounds)
    }

    fn timeline_project_bounds_for_render(&self, project: &Project) -> TimeRange {
        self.timeline_segment_index_and_bounds_for_render(project).1
    }

    pub(super) fn overview_waveform_snapshot_for_render(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> OverviewWaveformSnapshot {
        let key = OverviewWaveformCacheKey::from_editor(self.timeline_lanes_generation, editor);
        let dynamic_key = OverviewWaveformDynamicCacheKey::from_editor(editor);
        let mut cache = self.overview_waveform_cache.borrow_mut();
        if let Some(entry) = cache.entry.as_mut()
            && entry.key == key
        {
            if entry.dynamic_key != dynamic_key {
                let overview = Arc::new(overview_waveform_with_dynamic_state(
                    entry.overview.as_ref(),
                    entry.bounds,
                    editor,
                ));
                entry.dynamic_key = dynamic_key;
                entry.overview = Arc::clone(&overview);
                return OverviewWaveformSnapshot {
                    overview,
                    bounds: entry.bounds,
                };
            }
            return OverviewWaveformSnapshot {
                overview: Arc::clone(&entry.overview),
                bounds: entry.bounds,
            };
        }

        let bounds = TimeRange::for_project(project);
        let overview = Arc::new(editor.overview_waveform(project));
        cache.miss_count += 1;
        cache.entry = Some(OverviewWaveformCacheEntry {
            key,
            dynamic_key,
            bounds,
            overview: Arc::clone(&overview),
        });
        OverviewWaveformSnapshot { overview, bounds }
    }

    pub(super) fn transport_strip_snapshot_for_render(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Arc<TransportStripView> {
        let can_undo = self.can_undo();
        let can_redo = self.can_redo();
        let key = TransportStripCacheKey::from_editor(
            self.transcript_rows_generation,
            editor,
            can_undo,
            can_redo,
        );
        let mut cache = self.transport_strip_cache.borrow_mut();
        if let Some(entry) = cache.entries.iter().find(|entry| entry.key == key) {
            return Arc::clone(&entry.strip);
        }

        let strip = Arc::new(editor.transport_strip_with_history(project, can_undo, can_redo));
        cache.miss_count += 1;
        if cache.entries.len() >= TRANSPORT_STRIP_CACHE_CAPACITY {
            cache.entries.remove(0);
        }
        cache.entries.push(TransportStripCacheEntry {
            key,
            strip: Arc::clone(&strip),
        });
        strip
    }

    pub(super) fn submit_control_snapshot_for_render(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Arc<SubmitControlView> {
        let key = SubmitControlCacheKey::from_state(self.transcript_rows_generation, editor, self);
        let mut cache = self.submit_control_cache.borrow_mut();
        if let Some(entry) = cache.entry.as_ref()
            && entry.key == key
        {
            return Arc::clone(&entry.submit);
        }

        let submit = Arc::new(editor.submit_control(
            project,
            self.show_submit_dialog,
            self.submit_is_pending,
            self.submit_has_error,
        ));
        cache.miss_count += 1;
        cache.entry = Some(SubmitControlCacheEntry {
            key,
            submit: Arc::clone(&submit),
        });
        submit
    }

    pub(super) fn transcript_row_height_estimates(
        &self,
        row_views: &[TranscriptRowView],
    ) -> Vec<f32> {
        row_views
            .iter()
            .map(|row_view| {
                estimate_transcript_row_height(
                    row_view,
                    self.segment_drafts.get(&row_view.segment_id),
                )
            })
            .collect()
    }

    pub(super) fn transcript_layout_for_render(
        &self,
        row_views: &[TranscriptRowView],
        editor: &EditorSession,
    ) -> TranscriptLayoutView {
        let key = TranscriptLayoutCacheKey::from_state(
            self.transcript_rows_generation,
            self.transcript_layout_generation,
            editor,
        );
        let scroll_offset_bits = self.transcript_scroll_offset_y.to_bits();
        let viewport_height_bits = self.transcript_scroll_viewport_height.to_bits();
        let pinned_row_index = editor
            .selected_segment_id
            .as_deref()
            .and_then(|segment_id| {
                row_views
                    .iter()
                    .position(|row_view| row_view.segment_id == segment_id)
            });
        let mut cache = self.transcript_layout_cache.borrow_mut();
        if let Some(entry) = cache.entry.as_mut()
            && entry.key == key
        {
            if entry.scroll_offset_bits == scroll_offset_bits
                && entry.viewport_height_bits == viewport_height_bits
                && entry.pinned_row_index == pinned_row_index
            {
                return entry.layout.clone();
            }

            let row_heights = Arc::clone(&entry.layout.row_heights);
            let layout = TranscriptLayoutView {
                virtual_window: self
                    .transcript_virtual_window_for_heights(&row_heights, pinned_row_index),
                row_heights,
            };
            entry.scroll_offset_bits = scroll_offset_bits;
            entry.viewport_height_bits = viewport_height_bits;
            entry.pinned_row_index = pinned_row_index;
            entry.layout = layout.clone();
            return layout;
        }

        let row_heights = Arc::<[f32]>::from(self.transcript_row_height_estimates(row_views));
        let virtual_window =
            self.transcript_virtual_window_for_heights(&row_heights, pinned_row_index);
        let layout = TranscriptLayoutView {
            row_heights,
            virtual_window,
        };
        cache.miss_count += 1;
        cache.entry = Some(TranscriptLayoutCacheEntry {
            key,
            scroll_offset_bits,
            viewport_height_bits,
            pinned_row_index,
            layout: layout.clone(),
        });
        layout
    }

    pub(super) fn transcript_virtual_window_for_heights(
        &self,
        row_heights: &[f32],
        pinned_row_index: Option<usize>,
    ) -> TranscriptVirtualWindow {
        let viewport_height = sanitized_viewport_height(self.transcript_scroll_viewport_height);
        transcript_virtual_window(
            row_heights,
            TranscriptVirtualizationInput {
                scroll_offset_y: self.transcript_scroll_offset_y,
                viewport_height,
                overscan_height: viewport_height * TRANSCRIPT_VIRTUAL_OVERSCAN_VIEWPORTS,
                render_all_threshold: TRANSCRIPT_VIRTUAL_RENDER_ALL_THRESHOLD,
                pinned_row_index,
            },
        )
    }

    pub(super) fn update_transcript_scroll_viewport(
        &mut self,
        offset_y: f32,
        viewport_height: f32,
    ) {
        self.transcript_scroll_offset_y = sanitized_scroll_offset(offset_y);
        self.transcript_scroll_viewport_height = sanitized_viewport_height(viewport_height);
    }

    pub(super) fn transcript_scroll_offset_for_segment(
        &self,
        project: &Project,
        editor: &EditorSession,
        segment_id: &str,
    ) -> Option<f32> {
        let rows = self.transcript_rows_for_render(project, editor);
        let row_index = rows
            .iter()
            .position(|row_view| row_view.segment_id == segment_id)?;
        let row_heights = self.transcript_row_height_estimates(&rows);
        centered_row_scroll_offset(
            &row_heights,
            row_index,
            self.transcript_scroll_viewport_height,
        )
    }

    pub(super) fn scroll_transcript_to_segment_if_needed(
        &mut self,
        project: &Project,
        segment_id: &str,
    ) -> Option<f32> {
        let editor = self.editor.as_ref()?;
        let rows = self.transcript_rows_for_render(project, editor);
        let row_index = rows
            .iter()
            .position(|row_view| row_view.segment_id == segment_id)?;
        let row_heights = self.transcript_row_height_estimates(&rows);
        let row_top = row_heights.iter().take(row_index).sum::<f32>();
        let row_bottom = row_top + row_heights[row_index];
        let viewport_height = sanitized_viewport_height(self.transcript_scroll_viewport_height);
        let visible_top = self.transcript_scroll_offset_y;
        let visible_bottom = visible_top + viewport_height;
        if row_top >= visible_top && row_bottom <= visible_bottom {
            return None;
        }

        let target =
            centered_row_scroll_offset(&row_heights, row_index, viewport_height).unwrap_or(0.0);
        self.transcript_scroll_offset_y = target;
        Some(target)
    }

    /// While playback advances the selection across segment boundaries, keep
    /// the current transcript row visible like the recovered web editor does.
    /// Returns a scroll offset only when the newly playing row is outside the
    /// viewport, so manual scrolling between boundary crossings is preserved.
    pub(super) fn playback_follow_transcript_scroll_target(&mut self) -> Option<f32> {
        let editor = self.editor.as_ref()?;
        if !editor.is_playing {
            return None;
        }
        let selected_segment_id = editor.selected_segment_id.clone()?;
        if self.playback_followed_segment_id.as_deref() == Some(selected_segment_id.as_str()) {
            return None;
        }
        let project = self.project()?.clone();
        self.playback_followed_segment_id = Some(selected_segment_id.clone());
        self.scroll_transcript_to_segment_if_needed(&project, &selected_segment_id)
    }

    pub(super) fn scroll_transcript_to_segment_centered(
        &mut self,
        project: &Project,
        segment_id: &str,
    ) -> Option<f32> {
        let editor = self.editor.as_ref()?;
        let target = self.transcript_scroll_offset_for_segment(project, editor, segment_id)?;
        if (self.transcript_scroll_offset_y - target).abs() < f32::EPSILON {
            return None;
        }
        self.transcript_scroll_offset_y = target;
        Some(target)
    }

    pub(super) fn transcript_rows_cache_miss_count(&self) -> usize {
        self.transcript_rows_cache.borrow().miss_count
    }

    pub(super) fn transcript_layout_cache_miss_count(&self) -> usize {
        self.transcript_layout_cache.borrow().miss_count
    }

    pub(super) fn timeline_lanes_cache_miss_count(&self) -> usize {
        self.timeline_lanes_cache.borrow().miss_count
    }

    #[cfg(test)]
    pub(super) fn timeline_segment_index_cache_miss_count(&self) -> usize {
        self.timeline_segment_index_cache.borrow().miss_count
    }

    #[cfg(test)]
    pub(super) fn timeline_project_bounds_cache_miss_count(&self) -> usize {
        self.timeline_segment_index_cache.borrow().miss_count
    }

    pub(super) fn overview_waveform_cache_miss_count(&self) -> usize {
        self.overview_waveform_cache.borrow().miss_count
    }

    pub(super) fn transport_strip_cache_miss_count(&self) -> usize {
        self.transport_strip_cache.borrow().miss_count
    }

    pub(super) fn submit_control_cache_miss_count(&self) -> usize {
        self.submit_control_cache.borrow().miss_count
    }

    #[cfg(test)]
    pub(super) fn transcript_rows_cache_last_trace(&self) -> Option<TranscriptRowsCacheTrace> {
        self.transcript_rows_cache.borrow().last_trace.clone()
    }

    fn invalidate_transcript_rows_cache(&mut self) {
        self.transcript_rows_generation = self.transcript_rows_generation.wrapping_add(1);
        self.transcript_rows_cache.borrow_mut().entry = None;
        self.timeline_segment_index_cache.borrow_mut().entry = None;
        self.invalidate_transcript_layout_cache();
        self.invalidate_timeline_lanes_cache();
    }

    pub(super) fn invalidate_transcript_layout_cache(&mut self) {
        self.transcript_layout_generation = self.transcript_layout_generation.wrapping_add(1);
        self.transcript_layout_cache.borrow_mut().entry = None;
    }

    pub(super) fn invalidate_render_caches_for_project_metadata(&mut self) {
        self.invalidate_timeline_lanes_cache();
    }

    fn invalidate_timeline_lanes_cache(&mut self) {
        self.timeline_lanes_generation = self.timeline_lanes_generation.wrapping_add(1);
        self.timeline_lanes_cache.borrow_mut().clear();
        self.timeline_static_block_prewarm_cache
            .borrow_mut()
            .clear();
        self.overview_waveform_cache.borrow_mut().entry = None;
        self.transport_strip_cache.borrow_mut().entries.clear();
        self.submit_control_cache.borrow_mut().entry = None;
    }

    pub(super) fn dismiss_top_surface_or_loop(&mut self) -> bool {
        if self.native_auth_flow.is_some() {
            return true;
        } else if self.show_exit_confirm_dialog {
            self.show_exit_confirm_dialog = false;
            return true;
        } else if self.find_bar_open {
            self.close_find_bar();
            return true;
        } else if self.no_work_modal.is_some() {
            return true;
        } else if self.edit_annotation_dialog.is_some() {
            self.cancel_edit_annotation();
            return true;
        } else if self.create_region_dialog.is_some() {
            self.cancel_create_region();
            return true;
        } else if self.native_capability_panel_open {
            self.native_capability_panel_open = false;
            return true;
        } else if self.feedback_panel.as_ref().is_some_and(|panel| panel.open) {
            self.close_feedback_panel();
            return true;
        } else if self.task_lookup.is_some() {
            self.task_lookup = None;
            return true;
        } else if self.show_hotkeys_dialog {
            self.show_hotkeys_dialog = false;
            return true;
        } else if self.show_admin_flag_dialog {
            self.close_admin_flag_from_open_change();
            return true;
        } else if self.show_skip_chunk_dialog {
            self.cancel_skip_chunk();
            return true;
        } else if self.show_submit_dialog {
            if !self.submit_is_pending {
                self.show_submit_dialog = false;
            }
            return true;
        } else if let Some(project) = self.project().cloned() {
            if self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.stored_loop_range.is_some())
            {
                self.with_editor_mut(|editor| editor.clear_loop(&project));
                return true;
            }
        }
        false
    }

    pub(super) fn handle_helper_focused_escape(&mut self, segment_id: &str) {
        let Some(project) = self.project().cloned() else {
            self.status = "Helper focus toggle unavailable: no project loaded.".to_string();
            return;
        };
        if project.segment(segment_id).is_none() {
            self.status =
                format!("Helper focus toggle unavailable: segment {segment_id} not found.");
            return;
        }

        if let Some(editor) = self.editor.as_mut() {
            editor.select_segment_without_seek(&project, segment_id);
        }
        self.helper_focused_segment_id = Some(segment_id.to_string());

        if self.editor.as_ref().is_some_and(|editor| editor.is_playing) {
            self.set_playback_paused(true);
            self.status =
                "Helper focus toggle paused playback from focused transcript row.".to_string();
            return;
        }

        self.helper_focus_restore_pending =
            self.helper_focus_restore_for_segment(segment_id, false, None);
        self.helper_focused_segment_id = None;
        self.set_playback_paused(false);
    }

    pub(super) fn handle_helper_unfocused_escape(&mut self) {
        if !self.try_helper_unfocused_toggle() {
            self.status = "Helper focus toggle unavailable: no selected segment.".to_string();
        }
    }

    pub(super) fn handle_helper_toggle_playback(&mut self) {
        if !self.try_helper_unfocused_toggle() {
            self.toggle_playback();
        }
    }

    fn try_helper_unfocused_toggle(&mut self) -> bool {
        if self.editor.as_ref().is_some_and(|editor| editor.is_playing) {
            self.restore_helper_focus_for_current_playhead();
            self.set_playback_paused(true);
            return true;
        }

        let Some(segment_id) = self.helper_segment_id_at_playhead().or_else(|| {
            self.editor
                .as_ref()
                .and_then(|editor| editor.selected_segment_id.clone())
        }) else {
            return false;
        };

        self.helper_focus_restore_pending =
            self.helper_focus_restore_for_segment(&segment_id, true, None);
        self.helper_focused_segment_id = None;
        self.set_playback_paused(false);
        true
    }

    pub(super) fn adjust_playback_speed(&mut self, direction: i32) {
        let Some(current_speed) = self.editor.as_ref().map(|editor| editor.speed) else {
            self.status = "Playback speed unavailable: no editor session.".to_string();
            return;
        };
        let next_speed = adjacent_babel_playback_speed(current_speed, direction);
        self.with_editor_mut(|editor| editor.set_speed(next_speed));
        self.sync_audio_playback_speed();
        self.persist_timeline_defaults();
        self.status = format!("Playback speed set to {next_speed:.2}x.");
    }

    pub(super) fn rewind_playback_one_second(&mut self) {
        let Some(project) = self.project().cloned() else {
            self.status = "Rewind unavailable: no project loaded.".to_string();
            return;
        };
        let Some(playhead_seconds) = self.editor.as_ref().map(|editor| editor.playhead_seconds)
        else {
            self.status = "Rewind unavailable: no editor session.".to_string();
            return;
        };
        let next_time =
            (playhead_seconds - 1.0).max(TimeRange::for_project(&project).start_seconds);
        self.with_editor_mut(|editor| editor.seek_to_time(&project, next_time));
        self.last_transport_tick = None;
        self.restart_audio_playback_if_playing();
        self.status = format!("Rewound playback to {next_time:.2}s.");
    }

    pub(super) fn switch_speaker_workflow_lane(&mut self, target_label: &str) {
        let Some(project) = self.project().cloned() else {
            self.status = "Speaker workflow unavailable: no project loaded.".to_string();
            return;
        };
        let Some(target_lane) = project.lanes.iter().find(|lane| {
            normalize_speaker_workflow_label(&lane.label)
                == normalize_speaker_workflow_label(target_label)
        }) else {
            self.status = format!("Speaker workflow unavailable: lane {target_label} not found.");
            return;
        };
        let target_lane_id = target_lane.id.clone();
        let target_lane_label = target_lane.label.clone();
        let speaker_lane_ids = speaker_workflow_lane_ids(&project);

        self.with_editor_mut(|editor| {
            editor.track_filter = TrackFilter::lane(target_lane_id.clone(), target_lane_label);
            editor.collapsed_lane_ids.remove(&target_lane_id);
            editor.muted_lane_ids.remove(&target_lane_id);
            for lane_id in &speaker_lane_ids {
                if lane_id == &target_lane_id {
                    continue;
                }
                editor.collapsed_lane_ids.insert(lane_id.clone());
                editor.muted_lane_ids.insert(lane_id.clone());
            }
        });
        self.persist_timeline_defaults();
        self.status = format!("Speaker workflow switched to {target_label}.");
    }

    pub(super) fn reset_speaker_workflow(&mut self) {
        let Some(project) = self.project().cloned() else {
            self.status = "Speaker workflow reset unavailable: no project loaded.".to_string();
            return;
        };
        let speaker_lane_ids = speaker_workflow_lane_ids(&project);
        self.with_editor_mut(|editor| {
            editor.track_filter = TrackFilter::All;
            for lane_id in &speaker_lane_ids {
                editor.collapsed_lane_ids.remove(lane_id);
                editor.muted_lane_ids.remove(lane_id);
            }
        });
        self.persist_timeline_defaults();
        self.status =
            "Speaker workflow reset: showing all tracks and unmuting speakers.".to_string();
    }

    fn restore_helper_focus_for_current_playhead(&mut self) {
        let pending = self.helper_focus_restore_pending.clone();
        let segment_id = self
            .helper_segment_id_at_playhead()
            .or_else(|| pending.as_ref().map(|restore| restore.segment_id.clone()))
            .or_else(|| self.editor.as_ref()?.selected_segment_id.clone());
        let Some(segment_id) = segment_id else {
            self.status =
                "Helper focus restore failed: no segment at playback position.".to_string();
            return;
        };
        let blur_playhead_seconds = pending
            .as_ref()
            .map(|restore| restore.blur_playhead_seconds)
            .or_else(|| self.editor.as_ref().map(|editor| editor.playhead_seconds));
        let synthetic = pending.as_ref().is_some_and(|restore| restore.synthetic);
        let Some(restore) =
            self.helper_focus_restore_for_segment(&segment_id, synthetic, blur_playhead_seconds)
        else {
            self.status = format!("Helper focus restore failed: segment {segment_id} not found.");
            return;
        };
        let Some(project) = self.project().cloned() else {
            return;
        };
        if let Some(editor) = self.editor.as_mut() {
            editor.select_segment_without_seek(&project, &segment_id);
        }
        self.apply_helper_focus_restore_cursor(&restore);
        self.helper_focused_segment_id = Some(segment_id);
        self.helper_focus_restore_pending = None;
        self.helper_last_focus_restore = Some(restore);
    }

    pub(super) fn apply_helper_focus_restore_cursor(&mut self, restore: &HelperFocusRestore) {
        if let Some(draft) = self.segment_drafts.get_mut(&restore.segment_id) {
            draft.set_cursor_to_char_offset(restore.restore_offset);
        }
    }

    pub(super) fn sync_pending_helper_focus_restore_to_segment(&mut self, segment_id: &str) {
        let Some(previous) = self.helper_focus_restore_pending.clone() else {
            return;
        };
        if previous.segment_id == segment_id {
            return;
        }
        let blur_playhead_seconds = Some(previous.blur_playhead_seconds);
        let synthetic = previous.synthetic;
        if let Some(restore) =
            self.helper_focus_restore_for_segment(segment_id, synthetic, blur_playhead_seconds)
        {
            self.helper_focus_restore_pending = Some(restore);
            self.helper_focused_segment_id = None;
        }
    }

    pub(super) fn helper_ghost_cursor_for_segment(
        &self,
        segment_id: &str,
    ) -> Option<HelperGhostCursorView> {
        let restore = self.helper_focus_restore_pending.as_ref()?;
        if restore.segment_id != segment_id {
            return None;
        }
        if self.helper_focused_segment_id.as_deref() == Some(segment_id) {
            return None;
        }
        let project = self.project()?;
        let segment = project.segment(segment_id)?;
        let text = self
            .segment_drafts
            .get(segment_id)
            .map(|draft| draft.text.as_str())
            .unwrap_or(segment.text.as_str());
        let char_offset =
            if let Some(editor) = self.editor.as_ref().filter(|editor| editor.is_playing) {
                helper_proportional_restore_offset(
                    text,
                    segment,
                    editor.playhead_seconds,
                    restore.restore_offset,
                )
            } else {
                restore.restore_offset.min(text.chars().count())
            };
        let (before_cursor, after_cursor) = split_text_at_char_offset(text, char_offset);

        Some(HelperGhostCursorView {
            segment_id: segment_id.to_string(),
            char_offset,
            before_cursor,
            after_cursor,
        })
    }

    fn helper_focus_restore_for_segment(
        &self,
        segment_id: &str,
        synthetic: bool,
        blur_playhead_seconds: Option<f64>,
    ) -> Option<HelperFocusRestore> {
        let project = self.project()?;
        let editor = self.editor.as_ref()?;
        let segment = project.segment(segment_id)?;
        let text = self
            .segment_drafts
            .get(segment_id)
            .map(|draft| draft.text.as_str())
            .unwrap_or(segment.text.as_str());
        let blur_playhead_seconds = blur_playhead_seconds.unwrap_or(editor.playhead_seconds);
        let restore_offset =
            helper_proportional_restore_offset(text, segment, editor.playhead_seconds, 0);
        Some(HelperFocusRestore {
            segment_id: segment_id.to_string(),
            blur_playhead_seconds,
            restore_playhead_seconds: editor.playhead_seconds,
            restore_offset,
            synthetic,
        })
    }

    fn helper_segment_id_at_playhead(&self) -> Option<String> {
        let project = self.project()?;
        let editor = self.editor.as_ref()?;
        let playhead = editor.playhead_seconds;
        project
            .segments
            .iter()
            .filter(|segment| match &editor.track_filter {
                TrackFilter::All => true,
                TrackFilter::Lane { id, .. } => id == &segment.lane_id,
            })
            .filter(|segment| !editor.collapsed_lane_ids.contains(&segment.lane_id))
            .filter(|segment| TranscriptAnnotationKind::from_segment(segment).is_transcription())
            .filter(|segment| segment.start_seconds <= playhead && playhead < segment.end_seconds)
            .min_by(|left, right| {
                left.start_seconds
                    .total_cmp(&right.start_seconds)
                    .then_with(|| left.id.cmp(&right.id))
            })
            .map(|segment| segment.id.clone())
    }

    fn set_playback_paused(&mut self, paused: bool) {
        let Some(editor) = self.editor.as_mut() else {
            self.status = "Playback unavailable: no editor session.".to_string();
            return;
        };
        if paused {
            if editor.is_playing {
                editor.is_playing = false;
                self.last_transport_tick = None;
                self.audio_playback.stop();
            }
            return;
        }
        if editor.is_playing {
            return;
        }
        editor.is_playing = true;
        self.last_transport_tick = None;
        self.start_audio_playback();
    }

    pub(super) fn activate_default_surface_action(&mut self) -> Option<Message> {
        if let Some(flow) = self.native_auth_flow.as_ref() {
            if flow.can_submit_code() {
                return Some(Message::NativeAuthVerifyLogin);
            }
            if flow.can_start_login() {
                return Some(Message::NativeAuthStartLogin);
            }
        } else if self.edit_annotation_dialog.is_some() {
            self.submit_edit_annotation();
        } else if self.create_region_dialog.is_some() {
            self.submit_create_region();
        } else if self.show_hotkeys_dialog {
            self.show_hotkeys_dialog = false;
        } else if self.find_bar_open {
            return Some(Message::FindNext);
        } else if self.show_admin_flag_dialog {
            if !self.admin_flag_is_pending {
                self.begin_admin_flag_request();
            }
        } else if self.show_skip_chunk_dialog {
            if !self.skip_is_pending {
                self.begin_skip_chunk_request();
            }
        } else if self.show_submit_dialog && !self.submit_is_pending {
            self.begin_submit_review_request();
        }
        None
    }

    pub(super) fn open_native_auth_gate_from_store(&mut self) {
        match NativeAuthFlowState::startup_from_default_store() {
            NativeAuthStartup::Authenticated => {
                self.native_auth_flow = None;
                if self.app_surface == AppSurface::Dashboard {
                    self.queue_dashboard_projects_refresh();
                }
            }
            NativeAuthStartup::Flow(flow) => {
                self.native_auth_flow = Some(flow);
                self.app_surface = AppSurface::Dashboard;
                self.status = "Babel sign-in required.".to_string();
            }
        }
    }

    pub(super) fn native_auth_flow_blocks_service_dispatch(&self) -> bool {
        self.native_auth_flow.is_some()
    }

    pub(super) fn show_dashboard(&self) -> bool {
        self.native_auth_flow.is_none() && self.app_surface == AppSurface::Dashboard
    }

    pub(super) fn enter_dashboard(&mut self) {
        self.app_surface = AppSurface::Dashboard;
        self.dashboard.is_pending = false;
        self.status = "Babel dashboard ready.".to_string();
    }

    pub(super) fn queue_dashboard_projects_refresh(&mut self) {
        self.dashboard.is_pending = true;
        self.dashboard.error = None;
        self.status = "Loading Babel projects...".to_string();
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::LoadWorkerProjects)
        });
        self.babel_service_requests
            .push(BabelServiceRequest::LoadWorkerProjects);
    }

    pub(super) fn complete_dashboard_projects_loaded(&mut self, source: String) {
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::LoadWorkerProjects)
        });
        self.dashboard.is_pending = false;
        match dashboard_projects_from_payload(&source) {
            Ok(projects) => {
                self.dashboard.projects = projects;
                self.dashboard.error = None;
                self.status = format!(
                    "Loaded {} Babel project{}.",
                    self.dashboard.projects.len(),
                    if self.dashboard.projects.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                );
            }
            Err(error) => {
                self.dashboard.error = Some(error.clone());
                self.status = format!("Babel projects load failed: {error}");
            }
        }
    }

    pub(super) fn complete_dashboard_projects_failed(&mut self, error: String) {
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::LoadWorkerProjects)
        });
        self.dashboard.is_pending = false;
        self.dashboard.error = Some(error.clone());
        self.status = format!("Babel projects load failed: {error}");
    }

    pub(super) fn begin_dashboard_start_project(&mut self, project_id: &str) -> bool {
        let Some(project) = self
            .dashboard
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .cloned()
        else {
            self.dashboard.error = Some("Project is no longer available.".to_string());
            return false;
        };
        self.dashboard.project_id = project.id;
        self.dashboard.chunked_queue_id = project.chunked_queue_id;
        self.dashboard.stitched_queue_id = project.stitched_queue_id;
        self.begin_dashboard_claim_next_task()
    }

    pub(super) fn begin_dashboard_claim_next_task(&mut self) -> bool {
        let queue_id = self.dashboard.chunked_queue_id.trim().to_string();
        if queue_id.is_empty() {
            self.dashboard.error = Some("Chunked queue ID is required.".to_string());
            self.status = "Dashboard task start blocked: missing chunked queue ID.".to_string();
            return false;
        }

        self.merge_dashboard_route_metadata();
        self.dashboard.is_pending = true;
        self.dashboard.error = None;
        self.status = "Starting next Babel task...".to_string();
        self.babel_service_requests
            .push(BabelServiceRequest::ClaimNextFromQueue(
                ClaimNextFromQueueRequest { queue_id },
            ));
        true
    }

    pub(super) fn begin_dashboard_load_review_action(&mut self) -> bool {
        let review_action_id = self.dashboard.review_action_id.trim().to_string();
        if review_action_id.is_empty() {
            self.dashboard.error = Some("Review action ID is required.".to_string());
            self.status = "Dashboard review-action load blocked: missing ID.".to_string();
            return false;
        }

        self.merge_dashboard_route_metadata();
        self.dashboard.is_pending = true;
        self.dashboard.error = None;
        self.status = "Loading Babel review action...".to_string();
        self.babel_service_requests
            .push(BabelServiceRequest::LoadReviewAction(
                LoadReviewActionRequest { review_action_id },
            ));
        true
    }

    pub(super) fn begin_dashboard_load_processed_transcription(&mut self) -> bool {
        let processed_transcription_id =
            self.dashboard.processed_transcription_id.trim().to_string();
        let queue_id = self.dashboard.stitched_queue_id.trim().to_string();
        if processed_transcription_id.is_empty() || queue_id.is_empty() {
            self.dashboard.error =
                Some("Processed transcription ID and stitched queue ID are required.".to_string());
            self.status = "Dashboard processed-transcription load blocked: missing ID.".to_string();
            return false;
        }

        self.merge_dashboard_route_metadata();
        self.dashboard.is_pending = true;
        self.dashboard.error = None;
        self.status = "Loading Babel processed transcription...".to_string();
        self.babel_service_requests
            .push(BabelServiceRequest::LoadLatestReviewAction(
                LoadLatestReviewActionRequest {
                    processed_transcription_id,
                    queue_id,
                },
            ));
        true
    }

    pub(super) fn begin_dashboard_claim_processed_transcription(&mut self) -> bool {
        let processed_transcription_id =
            self.dashboard.processed_transcription_id.trim().to_string();
        let queue_id = self.dashboard.stitched_queue_id.trim().to_string();
        if processed_transcription_id.is_empty() || queue_id.is_empty() {
            self.dashboard.error =
                Some("Processed transcription ID and stitched queue ID are required.".to_string());
            self.status = "Dashboard stitched claim blocked: missing ID.".to_string();
            return false;
        }

        self.merge_dashboard_route_metadata();
        self.dashboard.is_pending = true;
        self.dashboard.error = None;
        self.status = "Claiming Babel stitched transcription...".to_string();
        self.babel_service_requests
            .push(BabelServiceRequest::ClaimByProcessedTranscription(
                ClaimByProcessedTranscriptionRequest {
                    processed_transcription_id,
                    queue_id,
                },
            ));
        true
    }

    fn refresh_dashboard_from_project_metadata(&mut self) {
        let metadata = self.project().map(|project| project.metadata.clone());
        let Some(metadata) = metadata else {
            return;
        };
        if self.dashboard.project_id.is_empty()
            && let Some(value) = metadata.get(META_BABEL_PROJECT_ID)
        {
            self.dashboard.project_id = value.clone();
        }
        if self.dashboard.chunked_queue_id.is_empty()
            && let Some(value) = metadata.get(META_CHUNKED_QUEUE_ID)
        {
            self.dashboard.chunked_queue_id = value.clone();
        }
        if self.dashboard.stitched_queue_id.is_empty()
            && let Some(value) = metadata.get(META_STITCHED_QUEUE_ID)
        {
            self.dashboard.stitched_queue_id = value.clone();
        }
        if self.dashboard.review_action_id.is_empty()
            && let Some(value) = metadata.get(META_REVIEW_ACTION_ID)
        {
            self.dashboard.review_action_id = value.clone();
        }
        if self.dashboard.processed_transcription_id.is_empty()
            && let Some(value) = metadata.get(META_TASK_LOOKUP_PROCESSED_ID)
        {
            self.dashboard.processed_transcription_id = value.clone();
        }
    }

    fn merge_dashboard_route_metadata(&mut self) {
        let mut entries = Vec::new();
        let project_id = self.dashboard.project_id.trim();
        if !project_id.is_empty() {
            entries.push((META_BABEL_PROJECT_ID.to_string(), project_id.to_string()));
        }
        let chunked_queue_id = self.dashboard.chunked_queue_id.trim();
        if !chunked_queue_id.is_empty() {
            entries.push((
                META_CHUNKED_QUEUE_ID.to_string(),
                chunked_queue_id.to_string(),
            ));
            entries.push((
                META_ACTIVE_QUEUE_ID.to_string(),
                chunked_queue_id.to_string(),
            ));
        }
        let stitched_queue_id = self.dashboard.stitched_queue_id.trim();
        if !stitched_queue_id.is_empty() {
            entries.push((
                META_STITCHED_QUEUE_ID.to_string(),
                stitched_queue_id.to_string(),
            ));
        }
        let review_action_id = self.dashboard.review_action_id.trim();
        if !review_action_id.is_empty() {
            entries.push((
                META_REVIEW_ACTION_ID.to_string(),
                review_action_id.to_string(),
            ));
            entries.push((
                META_TASK_LOOKUP_REVIEW_ACTION_ID.to_string(),
                review_action_id.to_string(),
            ));
        }
        let processed_transcription_id = self.dashboard.processed_transcription_id.trim();
        if !processed_transcription_id.is_empty() {
            entries.push((
                META_TASK_LOOKUP_PROCESSED_ID.to_string(),
                processed_transcription_id.to_string(),
            ));
        }
        if let Ok(workflow) = &mut self.workflow {
            workflow.merge_project_metadata(entries);
        }
    }

    pub(super) fn toggle_playback(&mut self) {
        let Some(editor) = &mut self.editor else {
            self.status = "Playback unavailable: no editor session.".to_string();
            return;
        };
        let was_playing = editor.is_playing;
        editor.toggle_playback();

        if was_playing {
            self.last_transport_tick = None;
            self.audio_playback.stop();
            return;
        }

        self.last_transport_tick = None;
        self.start_audio_playback();
    }

    pub(super) fn advance_transport_tick(&mut self, now: Instant) {
        let trace_update = transport_tick_update_trace_enabled();
        let tick_update_start = trace_update.then_some(Instant::now());
        let mut phase_timings = TransportTickUpdatePhaseTimings::default();
        let Some(workflow) = self.workflow.as_ref().ok() else {
            self.last_transport_tick = Some(now);
            return;
        };

        let elapsed_seconds = self
            .last_transport_tick
            .replace(now)
            .and_then(|previous| now.checked_duration_since(previous))
            .map(|elapsed| elapsed.as_secs_f64())
            .unwrap_or(TRANSPORT_TICK_SECONDS)
            .clamp(0.0, 0.25);

        if elapsed_seconds <= 0.0 {
            return;
        }

        let phase_start = trace_update.then_some(Instant::now());
        let timeline_bounds;
        {
            let project = workflow.project();
            let bounds = self.timeline_project_bounds_for_render(project);
            timeline_bounds = bounds;
            if let Some(editor) = self.editor.as_mut() {
                editor.advance_transport_with_bounds(project, bounds, elapsed_seconds);
            }
        }
        if let Some(phase_start) = phase_start {
            phase_timings.advance_transport = phase_start.elapsed();
        }
        self.advance_timeline_autoscroll(timeline_bounds, elapsed_seconds);
        let phase_start = trace_update.then_some(Instant::now());
        self.advance_large_project_live_pan_smoke();
        if let Some(phase_start) = phase_start {
            phase_timings.live_pan_smoke = phase_start.elapsed();
        }
        let phase_start = trace_update.then_some(Instant::now());
        self.advance_large_project_live_transcript_scroll_smoke();
        if let Some(phase_start) = phase_start {
            phase_timings.live_transcript_scroll_smoke = phase_start.elapsed();
        }
        let phase_start = trace_update.then_some(Instant::now());
        let started_playback = self.advance_startup_playback_smoke_delay();
        if let Some(phase_start) = phase_start {
            phase_timings.startup_playback_smoke = phase_start.elapsed();
        }

        let is_playing = self.editor.as_ref().is_some_and(|editor| editor.is_playing);
        let phase_start = trace_update.then_some(Instant::now());
        if is_playing {
            if let Some(probe) = self.frame_cadence_probe.as_mut() {
                if let Some(report) = probe.record(now) {
                    eprintln!("{}", report.runtime_trace_line());
                }
            }
        }
        if let Some(phase_start) = phase_start {
            phase_timings.cadence_probe = phase_start.elapsed();
        }
        if let Some(tick_update_start) = tick_update_start {
            let elapsed = tick_update_start.elapsed();
            if transport_tick_update_should_log(elapsed, started_playback) {
                eprintln!(
                    "{}",
                    transport_tick_update_runtime_trace_line(
                        elapsed,
                        is_playing,
                        started_playback,
                        phase_timings
                    )
                );
            }
        }

        if !is_playing && !self.timeline_autoscroll_active() {
            self.last_transport_tick = None;
            self.audio_playback.stop();
        }
    }

    pub(super) fn timeline_autoscroll_active(&self) -> bool {
        self.active_timeline_autoscroll.is_some()
    }

    pub(super) fn timeline_autoscroll_active_for_lane(&self, lane_id: &str) -> Option<f32> {
        self.active_timeline_autoscroll
            .as_ref()
            .filter(|autoscroll| autoscroll.lane_id == lane_id)
            .map(|autoscroll| autoscroll.anchor_fraction)
    }

    pub(super) fn waveform_magnifier_overlay_for_lane(
        &self,
        lane_id: &str,
    ) -> Option<WaveformMagnifierOverlay> {
        let drag = self
            .active_waveform_drag
            .as_ref()
            .filter(|drag| drag.lane_id == lane_id)?;
        if !matches!(
            drag.target,
            WaveformHitTarget::RegionStartHandle | WaveformHitTarget::RegionEndHandle
        ) {
            return None;
        }
        let hover = self
            .waveform_hit_hover
            .as_ref()
            .filter(|hover| hover.lane_id == lane_id)?;
        Some(WaveformMagnifierOverlay {
            time_seconds: hover.hit.time_seconds,
            fraction: hover.fraction,
            scale: WAVEFORM_MAGNIFIER_SCALE,
            width_px: WAVEFORM_MAGNIFIER_WIDTH_PX,
            max_height_px: WAVEFORM_MAGNIFIER_MAX_HEIGHT_PX,
        })
    }

    fn advance_timeline_autoscroll(&mut self, bounds: TimeRange, elapsed_seconds: f64) {
        let Some(autoscroll) = self.active_timeline_autoscroll.clone() else {
            return;
        };
        let Some(viewport_duration) = self
            .editor
            .as_ref()
            .map(|editor| editor.viewport.duration_seconds().max(0.001))
        else {
            return;
        };
        let distance_seconds =
            (autoscroll.target_fraction - autoscroll.anchor_fraction) as f64 * viewport_duration;
        if !distance_seconds.is_finite()
            || distance_seconds.abs() <= TIMELINE_AUTOSCROLL_DEAD_ZONE_SECONDS
        {
            return;
        }
        let delta_seconds =
            distance_seconds * elapsed_seconds * TIMELINE_AUTOSCROLL_SPEED_MULTIPLIER;
        if let Some(editor) = self.editor.as_mut() {
            editor.pan_viewport_by_time_delta_with_bounds(bounds, delta_seconds);
        }
    }

    fn advance_startup_playback_smoke_delay(&mut self) -> bool {
        let Some(remaining_frames) = self.startup_playback_smoke_delay_frames else {
            return false;
        };

        if remaining_frames > 1 {
            self.startup_playback_smoke_delay_frames = Some(remaining_frames - 1);
            return false;
        }

        self.startup_playback_smoke_delay_frames = None;
        if self.editor.as_ref().is_some_and(|editor| editor.is_playing) {
            return false;
        }
        let Some(editor) = &mut self.editor else {
            return false;
        };
        editor.is_playing = true;
        self.last_transport_tick = None;
        self.reset_frame_cadence_probe_from_env();
        self.start_audio_playback();
        true
    }

    fn advance_large_project_live_pan_smoke(&mut self) {
        if !self.large_project_live_pan_active || !large_project_live_pan_enabled() {
            return;
        }

        let pan_phase = self.large_project_live_pan_frame % 24;
        self.large_project_live_pan_frame = self.large_project_live_pan_frame.wrapping_add(1);
        let delta = if pan_phase < 12 { 1.0 } else { -1.0 };
        self.with_project_and_editor_mut(|project, editor| {
            editor.pan_viewport_by_scroll_delta(project, delta);
        });
    }

    pub(super) fn enable_large_project_live_pan_smoke(&mut self) {
        if large_project_live_pan_enabled() {
            self.large_project_live_pan_active = true;
            self.large_project_live_pan_frame = 0;
        }
    }

    fn advance_large_project_live_transcript_scroll_smoke(&mut self) {
        if !self.large_project_live_transcript_scroll_active
            || !large_project_live_transcript_scroll_enabled()
        {
            return;
        }

        let scroll_phase = self.large_project_live_transcript_scroll_frame % 72;
        self.large_project_live_transcript_scroll_frame = self
            .large_project_live_transcript_scroll_frame
            .wrapping_add(1);
        let bounce_phase = if scroll_phase < 36 {
            scroll_phase
        } else {
            72 - scroll_phase
        };
        let next_offset_y = bounce_phase as f32 * 30.0;
        self.update_transcript_scroll_viewport(
            next_offset_y,
            self.transcript_scroll_viewport_height,
        );
    }

    pub(super) fn enable_large_project_live_transcript_scroll_smoke(&mut self) {
        if large_project_live_transcript_scroll_enabled() {
            self.large_project_live_transcript_scroll_active = true;
            self.large_project_live_transcript_scroll_frame = 0;
        }
    }

    pub(super) fn reset_frame_cadence_probe_from_env(&mut self) {
        self.frame_cadence_probe = frame_cadence_probe_from_env();
    }

    pub(super) fn schedule_startup_playback_smoke_after_frames(&mut self, frames: usize) {
        self.startup_playback_smoke_delay_frames = Some(frames.max(1));
        self.last_transport_tick = None;
    }

    pub(super) fn startup_playback_smoke_delay_pending(&self) -> bool {
        self.startup_playback_smoke_delay_frames.is_some()
    }

    pub(super) fn start_audio_playback(&mut self) {
        let startup_start = Instant::now();
        let Some(workflow) = self.workflow.as_ref().ok() else {
            self.status = "Playback unavailable: no project loaded.".to_string();
            return;
        };
        let Some(editor) = self.editor.as_ref() else {
            self.status = "Playback unavailable: no editor session.".to_string();
            return;
        };
        let track_collect_start = Instant::now();
        let tracks = playback_tracks_for(workflow.project(), editor);
        let track_collect_elapsed = track_collect_start.elapsed();
        let requested_track_count = tracks.len();
        let playhead_seconds = editor.playhead_seconds;
        let speed = editor.speed;
        let backend_start = Instant::now();
        let playback_result = self
            .audio_playback
            .play_tracks(tracks, playhead_seconds, speed);
        let backend_elapsed = backend_start.elapsed();
        if audio_playback_startup_trace_enabled() {
            let outcome = match &playback_result {
                Ok(0) => "empty",
                Ok(_) => "started",
                Err(_) => "error",
            };
            eprintln!(
                "{}",
                audio_playback_startup_runtime_trace_line(
                    requested_track_count,
                    track_collect_elapsed,
                    backend_elapsed,
                    startup_start.elapsed(),
                    outcome,
                )
            );
        }
        match playback_result {
            Ok(0) => {}
            Ok(count) => {
                self.record_audio_playback_info(RuntimeAudioPlaybackInfo {
                    track_count: count,
                    start_seconds: playhead_seconds,
                    speed,
                });
                let suffix = if count == 1 {
                    "Started native audio playback for 1 track.".to_string()
                } else {
                    format!("Started native audio playback for {count} tracks.")
                };
                self.status = suffix;
            }
            Err(error) => {
                self.runtime_audio_playback_info = None;
                self.status = format!("Audio playback unavailable: {error}");
            }
        }
    }

    fn record_audio_playback_info(&mut self, info: RuntimeAudioPlaybackInfo) {
        if audio_playback_trace_enabled() {
            eprintln!("{}", info.runtime_log_line());
        }
        self.runtime_audio_playback_info = Some(info);
    }

    pub(super) fn restart_audio_playback_if_playing(&mut self) {
        if self.editor.as_ref().is_some_and(|editor| editor.is_playing) {
            self.start_audio_playback();
        }
    }

    pub(super) fn sync_audio_playback_speed(&mut self) {
        if let Some(editor) = &self.editor {
            self.audio_playback.set_speed(editor.speed);
        }
    }

    pub(super) fn play_audio_range(&mut self, start_seconds: f64, end_seconds: f64) {
        if !start_seconds.is_finite() || !end_seconds.is_finite() || end_seconds <= start_seconds {
            return;
        }
        let Some(workflow) = self.workflow.as_ref().ok() else {
            self.status = "Playback unavailable: no project loaded.".to_string();
            return;
        };
        let Some(editor) = self.editor.as_ref() else {
            self.status = "Playback unavailable: no editor session.".to_string();
            return;
        };
        let speed = editor.speed;
        if let Some(active_editor) = &mut self.editor {
            active_editor.is_playing = false;
            active_editor.playhead_seconds = start_seconds;
        }
        let Some(editor) = self.editor.as_ref() else {
            self.status = "Playback unavailable: no editor session.".to_string();
            return;
        };
        let tracks = playback_tracks_for(workflow.project(), editor);
        match self
            .audio_playback
            .play_tracks_until(tracks, start_seconds, Some(end_seconds), speed)
        {
            Ok(0) => {}
            Ok(count) => {
                self.status = if count == 1 {
                    format!("Auditioning {start_seconds:.3}s-{end_seconds:.3}s on 1 track.")
                } else {
                    format!("Auditioning {start_seconds:.3}s-{end_seconds:.3}s on {count} tracks.")
                };
            }
            Err(error) => {
                self.status = format!("Audio playback unavailable: {error}");
            }
        }
    }

    pub(super) fn acknowledge_low_confidence_word(&mut self, segment_id: &str, word_index: usize) {
        let Some(project) = self.project().cloned() else {
            self.status = "Low-confidence review unavailable: no project loaded.".to_string();
            return;
        };
        let Some(editor) = &self.editor else {
            self.status = "Low-confidence review unavailable: no editor session.".to_string();
            return;
        };

        let Some(row) = editor
            .all_transcript_rows(&project)
            .into_iter()
            .find(|row| row.segment_id == segment_id)
        else {
            self.status = format!("Segment `{segment_id}` was not found.");
            return;
        };

        let reviewable_indices = row
            .word_views
            .iter()
            .filter(|word| word.is_low_confidence)
            .map(|word| word.index)
            .collect::<BTreeSet<_>>();
        if !reviewable_indices.contains(&word_index) {
            self.status = format!("Low-confidence word `{word_index}` was not found.");
            return;
        }

        let acknowledged = self
            .acknowledged_low_confidence_words
            .entry(segment_id.to_string())
            .or_default();
        acknowledged.insert(word_index);

        if reviewable_indices
            .iter()
            .all(|index| acknowledged.contains(index))
        {
            let resolved_segment_id = segment_id.to_string();
            if self.with_editable_workflow_mut("Low-confidence review", |workflow| {
                workflow
                    .resolve_low_confidence(&resolved_segment_id)
                    .map_err(|error| error.to_string())?;
                Ok(format!(
                    "Marked low-confidence words reviewed for `{resolved_segment_id}`."
                ))
            }) {
                self.acknowledged_low_confidence_words.remove(segment_id);
                self.refresh_segment_draft(segment_id);
                self.mark_dirty();
                self.reconcile_editor_with_project();
            }
        } else {
            let reviewed = acknowledged.len();
            let total = reviewable_indices.len();
            self.status =
                format!("Reviewed {reviewed} of {total} low-confidence words in `{segment_id}`.");
        }
    }

    pub(super) fn clear_low_confidence_review_state(&mut self, segment_id: &str) {
        self.acknowledged_low_confidence_words.remove(segment_id);
    }

    pub(super) fn hover_loop_strip(&mut self, hit: LoopStripHit) {
        self.loop_strip_hover = Some(hit);
        let Some(drag) = self.active_loop_drag.clone() else {
            return;
        };
        let Some(project) = self.project().cloned() else {
            return;
        };

        let target = drag.target;
        self.with_editor_mut(|editor| match drag.target {
            LoopStripHitTarget::StartHandle => editor.set_loop_start(&project, hit.time_seconds),
            LoopStripHitTarget::EndHandle => editor.set_loop_end(&project, hit.time_seconds),
            LoopStripHitTarget::RangeBody => editor.move_loop_range(
                &project,
                drag.original_start_seconds,
                drag.original_end_seconds,
                drag.grab_time_seconds,
                hit.time_seconds,
            ),
            LoopStripHitTarget::Empty => {
                editor.set_loop_range_from_drag(&project, drag.grab_time_seconds, hit.time_seconds)
            }
        });
        if timeline_drag_trace_enabled()
            && let Some(editor) = self.editor.as_ref()
        {
            let range = editor.stored_loop_range.unwrap_or(editor.loop_range);
            eprintln!(
                "{}",
                timeline_loop_runtime_trace_line(target, range, editor.loop_enabled)
            );
        }
    }

    pub(super) fn press_loop_strip(&mut self) {
        let Some(hit) = self.loop_strip_hover else {
            return;
        };
        if hit.target == LoopStripHitTarget::Empty {
            self.active_loop_drag = Some(LoopStripDrag {
                target: hit.target,
                grab_time_seconds: hit.time_seconds,
                original_start_seconds: hit.time_seconds,
                original_end_seconds: hit.time_seconds,
            });
            return;
        }
        let Some(editor) = &self.editor else {
            return;
        };
        let Some(loop_range) = editor.stored_loop_range else {
            return;
        };

        self.active_loop_drag = Some(LoopStripDrag {
            target: hit.target,
            grab_time_seconds: hit.time_seconds,
            original_start_seconds: loop_range.start_seconds,
            original_end_seconds: loop_range.end_seconds,
        });
    }

    pub(super) fn release_loop_strip(&mut self) {
        self.active_loop_drag = None;
    }

    pub(super) fn hover_lane_vertical_zoom_slider(&mut self, lane_id: String, fraction: f32) {
        let fraction = sanitize_fraction(fraction);
        self.lane_vertical_zoom_slider_hover = Some(LaneVerticalZoomSliderHover {
            lane_id: lane_id.clone(),
            fraction,
        });
        if self.active_lane_vertical_zoom_slider_id.as_deref() == Some(lane_id.as_str()) {
            self.set_lane_vertical_zoom_from_slider_fraction(&lane_id, fraction);
        }
    }

    pub(super) fn hover_lane_vertical_zoom_slider_with_geometry(
        &mut self,
        lane_id: String,
        fraction: f32,
        height: f32,
    ) {
        let fraction = sanitize_fraction(fraction);
        self.lane_vertical_zoom_slider_pointer_sample = Some(LaneVerticalZoomSliderPointerSample {
            lane_id: lane_id.clone(),
            fraction,
            height: height.max(1.0),
        });
        if self.active_lane_vertical_zoom_slider_id.as_deref() != Some(lane_id.as_str()) {
            self.refresh_lane_vertical_zoom_slider_drag_capture();
        } else if self.lane_vertical_zoom_slider_drag_capture.is_none() {
            self.refresh_lane_vertical_zoom_slider_drag_capture();
        }
        self.hover_lane_vertical_zoom_slider(lane_id, fraction);
    }

    pub(super) fn press_lane_vertical_zoom_slider(&mut self, lane_id: &str) -> bool {
        self.press_lane_vertical_zoom_slider_at(lane_id, Instant::now())
    }

    pub(in crate::app) fn press_lane_vertical_zoom_slider_at(
        &mut self,
        lane_id: &str,
        at: Instant,
    ) -> bool {
        let is_double_click = self
            .last_lane_vertical_zoom_slider_press
            .as_ref()
            .is_some_and(|press| {
                press.lane_id == lane_id
                    && at
                        .checked_duration_since(press.at)
                        .is_some_and(|elapsed| elapsed <= BABEL_TRACK_CELL_DOUBLE_CLICK_WINDOW)
            });
        if is_double_click {
            let zoom_before = self
                .last_lane_vertical_zoom_slider_press
                .take()
                .and_then(|press| press.zoom_before)
                .unwrap_or(DEFAULT_LANE_VERTICAL_ZOOM);
            // Undo the click-to-set jump from the first press and hand the
            // exact value over to a typed input instead.
            self.with_editor_mut(|editor| editor.set_lane_vertical_zoom(lane_id, zoom_before));
            self.release_lane_vertical_zoom_slider();
            self.lane_vertical_zoom_input = Some(LaneVerticalZoomInput {
                lane_id: lane_id.to_string(),
                value: format!("{zoom_before}"),
            });
            return true;
        }

        self.last_lane_vertical_zoom_slider_press = Some(LaneVerticalZoomSliderPress {
            lane_id: lane_id.to_string(),
            at,
            zoom_before: self
                .editor
                .as_ref()
                .and_then(|editor| editor.lane_vertical_zoom.get(lane_id).copied()),
        });
        self.active_lane_vertical_zoom_slider_id = Some(lane_id.to_string());
        if self
            .lane_vertical_zoom_slider_drag_capture
            .as_ref()
            .is_none_or(|capture| capture.lane_id != lane_id)
        {
            self.refresh_lane_vertical_zoom_slider_drag_capture();
        }
        if let Some(fraction) = self
            .lane_vertical_zoom_slider_hover
            .as_ref()
            .filter(|hover| hover.lane_id == lane_id)
            .map(|hover| hover.fraction)
        {
            self.set_lane_vertical_zoom_from_slider_fraction(lane_id, fraction);
        }
        false
    }

    pub(super) fn change_lane_vertical_zoom_input(&mut self, value: String) {
        if let Some(input) = self.lane_vertical_zoom_input.as_mut() {
            input.value = value;
        }
    }

    pub(super) fn submit_lane_vertical_zoom_input(&mut self) {
        let Some(input) = self.lane_vertical_zoom_input.clone() else {
            return;
        };
        let raw = input
            .value
            .trim()
            .trim_end_matches(['x', 'X'])
            .trim()
            .replace(',', ".");
        let Some(value) = raw.parse::<f32>().ok().filter(|value| value.is_finite()) else {
            self.status = format!("Vertical zoom `{}` is not a number.", input.value);
            return;
        };
        let clamped = value.clamp(LANE_VERTICAL_ZOOM_SLIDER_MIN, LANE_VERTICAL_ZOOM_SLIDER_MAX);
        self.with_editor_mut(|editor| editor.set_lane_vertical_zoom(&input.lane_id, clamped));
        self.persist_timeline_defaults();
        self.lane_vertical_zoom_input = None;
        self.status = format!("Lane vertical zoom set to {clamped}x.");
    }

    pub(super) fn cancel_lane_vertical_zoom_input(&mut self) -> bool {
        self.lane_vertical_zoom_input.take().is_some()
    }

    pub(super) fn release_lane_vertical_zoom_slider(&mut self) {
        self.active_lane_vertical_zoom_slider_id = None;
        self.lane_vertical_zoom_slider_drag_capture = None;
    }

    pub(super) fn release_captured_pointer(&mut self) {
        let loop_active = self.active_loop_drag.is_some();
        let scrollbar_active = self.timeline_scrollbar_drag_offset_fraction.is_some();
        let vertical_zoom_active = self.active_lane_vertical_zoom_slider_id.is_some();
        let active_waveform_lane_id = self
            .active_overlay_drag
            .as_ref()
            .map(|drag| drag.lane_id.clone())
            .or_else(|| {
                self.active_waveform_create_drag
                    .as_ref()
                    .map(|drag| drag.lane_id.clone())
            })
            .or_else(|| {
                self.active_waveform_drag
                    .as_ref()
                    .map(|drag| drag.lane_id.clone())
            });
        if timeline_drag_trace_enabled()
            && (loop_active
                || scrollbar_active
                || vertical_zoom_active
                || active_waveform_lane_id.is_some())
        {
            eprintln!(
                "{}",
                timeline_pointer_capture_release_runtime_trace_line(
                    loop_active,
                    scrollbar_active,
                    vertical_zoom_active,
                    active_waveform_lane_id.is_some()
                )
            );
        }

        if let Some(lane_id) = active_waveform_lane_id {
            self.release_waveform_hit(&lane_id);
        }
        self.cancel_timeline_autoscroll();
        self.release_loop_strip();
        self.release_lane_vertical_zoom_slider();
        self.release_timeline_scrollbar();
        self.release_overview();
    }

    fn set_lane_vertical_zoom_from_slider_fraction(&mut self, lane_id: &str, fraction: f32) {
        let vertical_zoom = LANE_VERTICAL_ZOOM_SLIDER_MAX
            - sanitize_fraction(fraction)
                * (LANE_VERTICAL_ZOOM_SLIDER_MAX - LANE_VERTICAL_ZOOM_SLIDER_MIN);
        self.with_editor_mut(|editor| editor.set_lane_vertical_zoom(lane_id, vertical_zoom));
        if timeline_drag_trace_enabled()
            && let Some(value) = self
                .editor
                .as_ref()
                .and_then(|editor| editor.lane_vertical_zoom.get(lane_id))
        {
            eprintln!(
                "{}",
                timeline_vertical_zoom_runtime_trace_line(lane_id, *value)
            );
        }
    }

    pub(super) fn hover_overview(&mut self, fraction: f32) {
        let fraction = sanitize_fraction(fraction);
        self.overview_hover_fraction = Some(fraction);
        if self.overview_drag_active {
            if self.overview_drag_capture.is_none() {
                self.refresh_overview_drag_capture();
            }
            self.drag_overview_viewport(fraction);
        }
    }

    pub(super) fn hover_overview_with_geometry(&mut self, fraction: f32, width: f32) {
        let fraction = sanitize_fraction(fraction);
        self.overview_pointer_sample = Some(OverviewPointerSample {
            fraction,
            width: width.max(1.0),
        });
        if !self.overview_drag_active {
            self.refresh_overview_drag_capture();
        } else if self.overview_drag_capture.is_none() {
            self.refresh_overview_drag_capture();
        }
        self.hover_overview(fraction);
    }

    pub(super) fn exit_overview(&mut self) {
        if !self.overview_drag_active {
            self.overview_hover_fraction = None;
            self.overview_pointer_sample = None;
            self.overview_drag_capture = None;
        }
    }

    pub(super) fn press_overview(&mut self) {
        let Some(fraction) = self.overview_hover_fraction else {
            return;
        };
        if !self.overview_fraction_is_inside_viewport(fraction) {
            // Track click: jump the viewport under the pointer first, then keep
            // dragging it like a regular thumb grab.
            self.seek_overview_fraction(fraction);
        }
        self.overview_drag_active = true;
        self.overview_drag_grab_offset_fraction = self.overview_grab_offset_fraction(fraction);
        if self.overview_drag_capture.is_none() {
            self.refresh_overview_drag_capture();
        }
        // The grab offset makes a drag at the press fraction a no-op, so the
        // viewport only starts moving with the first pointer move.
    }

    pub(super) fn release_overview(&mut self) {
        self.overview_drag_active = false;
        self.overview_drag_grab_offset_fraction = 0.0;
        self.overview_drag_capture = None;
    }

    pub(super) fn pointer_capture_subscription_active(&self) -> bool {
        self.overview_pointer_sample.is_some()
            || self.overview_drag_active
            || self.timeline_scrollbar_pointer_sample.is_some()
            || self.timeline_scrollbar_drag_offset_fraction.is_some()
            || self.lane_vertical_zoom_slider_pointer_sample.is_some()
            || self.active_lane_vertical_zoom_slider_id.is_some()
    }

    pub(super) fn move_global_pointer(&mut self, position: Point) {
        self.last_global_pointer_position = Some(position);

        if self.overview_drag_active {
            if self.overview_drag_capture.is_none() {
                self.refresh_overview_drag_capture();
            }
            if let Some(capture) = self.overview_drag_capture {
                let fraction = capture.fraction_at(position.x);
                self.overview_hover_fraction = Some(fraction);
                self.drag_overview_viewport(fraction);
            }
        } else {
            self.refresh_overview_drag_capture();
        }

        if let Some(grab_offset_fraction) = self.timeline_scrollbar_drag_offset_fraction {
            if self.timeline_scrollbar_drag_capture.is_none() {
                self.refresh_timeline_scrollbar_drag_capture();
            }
            if let Some(capture) = self.timeline_scrollbar_drag_capture {
                let fraction = capture.fraction_at(position.x);
                self.timeline_scrollbar_hover_fraction = Some(fraction);
                self.drag_timeline_scrollbar_thumb(fraction, grab_offset_fraction);
            }
        } else {
            self.refresh_timeline_scrollbar_drag_capture();
        }

        if let Some(lane_id) = self.active_lane_vertical_zoom_slider_id.clone() {
            if self
                .lane_vertical_zoom_slider_drag_capture
                .as_ref()
                .is_none_or(|capture| capture.lane_id != lane_id)
            {
                self.refresh_lane_vertical_zoom_slider_drag_capture();
            }
            if let Some(capture) = self
                .lane_vertical_zoom_slider_drag_capture
                .as_ref()
                .filter(|capture| capture.lane_id == lane_id)
            {
                let fraction = capture.axis.fraction_at(position.y);
                self.lane_vertical_zoom_slider_hover = Some(LaneVerticalZoomSliderHover {
                    lane_id: lane_id.clone(),
                    fraction,
                });
                self.set_lane_vertical_zoom_from_slider_fraction(&lane_id, fraction);
            }
        } else {
            self.refresh_lane_vertical_zoom_slider_drag_capture();
        }
    }

    fn refresh_overview_drag_capture(&mut self) {
        let (Some(position), Some(sample)) = (
            self.last_global_pointer_position,
            self.overview_pointer_sample,
        ) else {
            return;
        };
        self.overview_drag_capture = Some(DragAxisCapture::from_pointer_sample(
            position.x,
            sample.fraction,
            sample.width,
        ));
    }

    fn refresh_lane_vertical_zoom_slider_drag_capture(&mut self) {
        let (Some(position), Some(sample)) = (
            self.last_global_pointer_position,
            self.lane_vertical_zoom_slider_pointer_sample.clone(),
        ) else {
            return;
        };
        self.lane_vertical_zoom_slider_drag_capture = Some(LaneVerticalZoomSliderDragCapture {
            lane_id: sample.lane_id,
            axis: DragAxisCapture::from_pointer_sample(position.y, sample.fraction, sample.height),
        });
    }

    pub(super) fn hover_timeline_scrollbar(&mut self, fraction: f32) {
        let fraction = sanitize_fraction(fraction);
        self.timeline_scrollbar_hover_fraction = Some(fraction);
        if let Some(grab_offset_fraction) = self.timeline_scrollbar_drag_offset_fraction {
            self.drag_timeline_scrollbar_thumb(fraction, grab_offset_fraction);
        }
    }

    pub(super) fn hover_timeline_scrollbar_with_geometry(&mut self, fraction: f32, width: f32) {
        let fraction = sanitize_fraction(fraction);
        self.timeline_scrollbar_pointer_sample = Some(OverviewPointerSample {
            fraction,
            width: width.max(1.0),
        });
        if self.timeline_scrollbar_drag_offset_fraction.is_none()
            || self.timeline_scrollbar_drag_capture.is_none()
        {
            self.refresh_timeline_scrollbar_drag_capture();
        }
        self.hover_timeline_scrollbar(fraction);
    }

    pub(super) fn exit_timeline_scrollbar(&mut self) {
        if self.timeline_scrollbar_drag_offset_fraction.is_none() {
            self.timeline_scrollbar_hover_fraction = None;
            self.timeline_scrollbar_pointer_sample = None;
            self.timeline_scrollbar_drag_capture = None;
        }
    }

    pub(super) fn press_timeline_scrollbar(&mut self) {
        let Some(fraction) = self.timeline_scrollbar_hover_fraction else {
            return;
        };
        let grab_offset_fraction = self.timeline_scrollbar_grab_offset_fraction(fraction);
        self.timeline_scrollbar_drag_offset_fraction = Some(grab_offset_fraction);
        if self.timeline_scrollbar_drag_capture.is_none() {
            self.refresh_timeline_scrollbar_drag_capture();
        }
        self.drag_timeline_scrollbar_thumb(fraction, grab_offset_fraction);
    }

    pub(super) fn release_timeline_scrollbar(&mut self) {
        self.timeline_scrollbar_drag_offset_fraction = None;
        self.timeline_scrollbar_drag_capture = None;
    }

    fn refresh_timeline_scrollbar_drag_capture(&mut self) {
        let (Some(position), Some(sample)) = (
            self.last_global_pointer_position,
            self.timeline_scrollbar_pointer_sample,
        ) else {
            return;
        };
        self.timeline_scrollbar_drag_capture = Some(DragAxisCapture::from_pointer_sample(
            position.x,
            sample.fraction,
            sample.width,
        ));
    }

    fn drag_timeline_scrollbar_thumb(&mut self, fraction: f32, grab_offset_fraction: f32) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        self.with_editor_mut(|editor| {
            editor.drag_timeline_scrollbar_thumb(&project, fraction, grab_offset_fraction);
        });
        if timeline_drag_trace_enabled()
            && let Some(editor) = self.editor.as_ref()
        {
            eprintln!("{}", timeline_scrollbar_runtime_trace_line(editor.viewport));
        }
    }

    fn timeline_scrollbar_grab_offset_fraction(&self, fraction: f32) -> f32 {
        let Some(scrollbar) = self.timeline_scrollbar_view() else {
            return 0.0;
        };
        if !scrollbar.is_visible {
            return 0.0;
        }

        let left = scrollbar.left_percent / 100.0;
        let width =
            (scrollbar.width_percent / 100.0).clamp(TIMELINE_SCROLLBAR_MIN_THUMB_FRACTION, 1.0);
        let right = (left + width).min(1.0);
        if left <= fraction && fraction <= right {
            (fraction - left).clamp(0.0, width)
        } else {
            width / 2.0
        }
    }

    fn timeline_scrollbar_view(&self) -> Option<TimelineScrollbarView> {
        let (Some(project), Some(editor)) = (self.project(), self.editor.as_ref()) else {
            return None;
        };
        self.timeline_lanes_snapshot_for_render(project, editor)
            .first()
            .map(|lane| lane.scrollbar)
    }

    pub(super) fn seek_overview_fraction(&mut self, fraction: f32) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        self.with_editor_mut(|editor| editor.seek_overview_fraction(&project, fraction));
        self.restart_audio_playback_if_playing();
    }

    pub(super) fn drag_overview_viewport(&mut self, fraction: f32) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        let grab_offset_fraction = if self.overview_drag_active {
            self.overview_drag_grab_offset_fraction
        } else {
            0.0
        };
        self.with_editor_mut(|editor| {
            editor.drag_timeline_scrollbar_thumb(&project, fraction, grab_offset_fraction);
        });
    }

    fn overview_viewport_fractions(&self) -> Option<(f32, f32)> {
        let (Some(project), Some(editor)) = (self.project(), self.editor.as_ref()) else {
            return None;
        };
        let overview = self.overview_waveform_snapshot_for_render(project, editor);
        Some((
            overview.overview.viewport_left_percent / 100.0,
            (overview.overview.viewport_width_percent / 100.0).max(0.0),
        ))
    }

    fn overview_fraction_is_inside_viewport(&self, fraction: f32) -> bool {
        let Some((left, width)) = self.overview_viewport_fractions() else {
            return false;
        };
        left <= fraction && fraction <= left + width
    }

    fn overview_grab_offset_fraction(&self, fraction: f32) -> f32 {
        let Some((left, width)) = self.overview_viewport_fractions() else {
            return 0.0;
        };
        (fraction - left).clamp(0.0, width)
    }

    pub(super) fn load_lane_waveforms_from_base(&mut self, base: impl AsRef<Path>) -> usize {
        let Some(project) = self.project().cloned() else {
            return 0;
        };
        let Some(editor) = &mut self.editor else {
            return 0;
        };

        let base = base.as_ref();
        let mut loaded = 0;
        for lane in &project.lanes {
            for audio_path in audio_source_candidates(base, &lane.source_path, &lane.speaker_id) {
                let Ok(bytes) = fs::read(&audio_path) else {
                    continue;
                };
                let Ok(decoded) = decode_pcm16_wav(&bytes) else {
                    continue;
                };
                editor.set_lane_waveform(&lane.id, LaneWaveform::from_decoded(decoded));
                loaded += 1;
                break;
            }
        }
        if loaded > 0 {
            self.invalidate_timeline_lanes_cache();
            self.prewarm_initial_waveform_render_caches();
            self.prewarm_audio_playback_backend();
        }
        loaded
    }

    fn prewarm_initial_waveform_render_caches(&self) {
        let (Some(project), Some(editor)) = (self.project(), self.editor.as_ref()) else {
            return;
        };

        let _ = self.timeline_lanes_snapshot_for_render(project, editor);
        let _ = self.overview_waveform_snapshot_for_render(project, editor);
        let _ = self.transport_strip_snapshot_for_render(project, editor);
        let mut playing_editor = editor.clone();
        playing_editor.is_playing = true;
        let _ = self.transport_strip_snapshot_for_render(project, &playing_editor);
    }

    fn prewarm_audio_playback_backend(&mut self) {
        let _ = self.audio_playback.prepare();
    }

    pub(super) fn load_bundled_sample_waveforms_for_runtime(&mut self) -> usize {
        let mut loaded = self.load_lane_waveforms_from_base(bundled_assets_dir());
        if loaded == 0 {
            loaded = self.load_lane_waveforms_from_base(recovered_fixture_audio_dir());
        }
        self.append_waveform_status(loaded);
        loaded
    }

    pub(super) fn start_recovered_audio_playback_smoke_for_runtime(&mut self) -> bool {
        if self.load_bundled_sample_waveforms_for_runtime() == 0 {
            return false;
        }

        let Some(editor) = &mut self.editor else {
            return false;
        };
        editor.is_playing = true;
        self.last_transport_tick = None;
        self.start_audio_playback();
        self.runtime_audio_playback_info.is_some()
    }

    pub(super) fn append_waveform_status(&mut self, loaded: usize) {
        if loaded == 0 {
            return;
        }

        let suffix = if loaded == 1 {
            "Loaded 1 waveform track.".to_string()
        } else {
            format!("Loaded {loaded} waveform tracks.")
        };
        self.status = format!("{} {suffix}", self.status);
    }

    pub(super) fn with_workflow_mut(
        &mut self,
        action_name: &str,
        action: impl FnOnce(&mut StudioWorkflow) -> Result<String, String>,
    ) -> bool {
        match &mut self.workflow {
            Ok(workflow) => match action(workflow) {
                Ok(status) => {
                    self.status = status;
                    self.sync_segment_drafts_preserving_existing();
                    true
                }
                Err(error) => {
                    self.status = format!("{action_name} failed: {error}");
                    false
                }
            },
            Err(error) => {
                self.status = format!("{action_name} unavailable: {error}");
                false
            }
        }
    }

    pub(super) fn with_editor_mut(&mut self, action: impl FnOnce(&mut EditorSession)) {
        if let Some(editor) = &mut self.editor {
            action(editor);
        }
    }

    pub(super) fn with_project_and_editor_mut(
        &mut self,
        action: impl FnOnce(&Project, &mut EditorSession),
    ) -> bool {
        let (Ok(workflow), Some(editor)) = (&self.workflow, &mut self.editor) else {
            return false;
        };
        action(workflow.project(), editor);
        true
    }

    pub(super) fn persist_timeline_defaults(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let track_filter_lane_id = match &editor.track_filter {
            TrackFilter::All => String::new(),
            TrackFilter::Lane { id, .. } => id.clone(),
        };
        let lane_vertical_zoom = editor
            .lane_vertical_zoom
            .iter()
            .filter(|(_, zoom)| zoom.is_finite())
            .map(|(lane_id, zoom)| (lane_id.clone(), *zoom))
            .collect::<BTreeMap<_, _>>();
        let collapsed_lane_ids =
            serde_json::to_string(&editor.collapsed_lane_ids).unwrap_or_else(|_| "[]".to_string());
        let muted_lane_ids =
            serde_json::to_string(&editor.muted_lane_ids).unwrap_or_else(|_| "[]".to_string());
        let lane_vertical_zoom =
            serde_json::to_string(&lane_vertical_zoom).unwrap_or_else(|_| "{}".to_string());
        let entries = [
            (
                META_TIMELINE_ZOOM.to_string(),
                format!("{:.6}", editor.zoom),
            ),
            (
                META_TIMELINE_SPEED.to_string(),
                format!("{:.6}", editor.speed),
            ),
            (
                META_TIMELINE_TRACK_FILTER_LANE_ID.to_string(),
                track_filter_lane_id,
            ),
            (
                META_TIMELINE_COLLAPSED_LANE_IDS.to_string(),
                collapsed_lane_ids,
            ),
            (META_TIMELINE_MUTED_LANE_IDS.to_string(), muted_lane_ids),
            (
                META_TIMELINE_LANE_VERTICAL_ZOOM.to_string(),
                lane_vertical_zoom,
            ),
        ];
        let updated = if let Ok(workflow) = &mut self.workflow {
            workflow.merge_project_metadata(entries);
            true
        } else {
            false
        };
        if updated {
            self.invalidate_render_caches_for_project_metadata();
        }
    }

    pub(super) fn hydrate_timeline_defaults_from_project_metadata(&mut self) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if let Some(zoom) = metadata_f32(&project, META_TIMELINE_ZOOM) {
            editor.set_zoom(&project, zoom);
        }
        if let Some(speed) = metadata_f32(&project, META_TIMELINE_SPEED) {
            editor.set_speed(speed);
        }

        editor.track_filter = project
            .metadata
            .get(META_TIMELINE_TRACK_FILTER_LANE_ID)
            .map(|value| value.trim())
            .filter(|lane_id| !lane_id.is_empty())
            .and_then(|lane_id| {
                project
                    .lanes
                    .iter()
                    .find(|lane| lane.id == lane_id)
                    .map(|lane| TrackFilter::lane(lane.id.clone(), lane.label.clone()))
            })
            .unwrap_or(TrackFilter::All);

        let lane_ids = project
            .lanes
            .iter()
            .map(|lane| lane.id.as_str())
            .collect::<BTreeSet<_>>();
        editor.collapsed_lane_ids = metadata_string_set(&project, META_TIMELINE_COLLAPSED_LANE_IDS)
            .into_iter()
            .filter(|lane_id| lane_ids.contains(lane_id.as_str()))
            .collect();
        editor.muted_lane_ids = metadata_string_set(&project, META_TIMELINE_MUTED_LANE_IDS)
            .into_iter()
            .filter(|lane_id| lane_ids.contains(lane_id.as_str()))
            .collect();
        editor.lane_vertical_zoom.clear();
        for (lane_id, vertical_zoom) in metadata_f32_map(&project, META_TIMELINE_LANE_VERTICAL_ZOOM)
        {
            if lane_ids.contains(lane_id.as_str()) {
                editor.set_lane_vertical_zoom(&lane_id, vertical_zoom);
            }
        }
        self.invalidate_render_caches_for_project_metadata();
    }

    pub(super) fn block_read_only_edit(&mut self, action_name: &str) -> bool {
        if self
            .project()
            .is_some_and(EditorSession::project_is_read_only)
        {
            self.status = format!("{action_name} unavailable: project is read-only.");
            return true;
        }

        false
    }

    pub(super) fn with_editable_workflow_mut(
        &mut self,
        action_name: &str,
        action: impl FnOnce(&mut StudioWorkflow) -> Result<String, String>,
    ) -> bool {
        if self.block_read_only_edit(action_name) {
            return false;
        }

        self.with_workflow_mut(action_name, action)
    }

    pub(super) fn request_submit_review(&mut self) {
        if self.submit_is_pending {
            self.show_submit_dialog = true;
            return;
        }

        if self.block_read_only_edit("Submit") {
            self.show_submit_dialog = false;
            return;
        }

        let submit = match (self.project(), self.editor.as_ref()) {
            (Some(project), Some(editor)) => editor.submit_control(
                project,
                self.show_submit_dialog,
                self.submit_is_pending,
                self.submit_has_error,
            ),
            (None, _) => {
                self.status = "Submit unavailable: no project loaded.".to_string();
                return;
            }
            (_, None) => {
                self.status = "Submit unavailable: no editor session.".to_string();
                return;
            }
        };

        if submit.trigger_disabled {
            self.show_submit_dialog = false;
            self.status = submit.trigger_tooltip_content;
            return;
        }

        self.submit_has_error = false;
        self.show_submit_dialog = true;
    }

    pub(super) fn begin_submit_review_request(&mut self) -> bool {
        if self.submit_is_pending {
            return false;
        }
        if self.block_read_only_edit("Submit") {
            return false;
        }

        self.submit_has_error = false;
        if !self.apply_pending_drafts("Submit") {
            self.submit_has_error = true;
            self.show_submit_dialog = true;
            return false;
        }

        let Some(project) = self.project().cloned() else {
            self.status = "Submit unavailable: no project loaded.".to_string();
            self.show_submit_dialog = true;
            self.submit_has_error = true;
            return false;
        };
        let Some(editor) = self.editor.as_ref() else {
            self.status = "Submit unavailable: no editor session.".to_string();
            self.show_submit_dialog = true;
            self.submit_has_error = true;
            return false;
        };
        let submit = editor.submit_control(&project, true, false, false);
        if submit.trigger_disabled {
            self.status = format!("Submit blocked: {}", submit.trigger_tooltip_content);
            self.show_submit_dialog = true;
            self.submit_has_error = true;
            return false;
        }

        let mut validator = editor.clone();
        if let Err(error) = validator.submit_review(&project) {
            self.status = format!("Submit blocked: {error}");
            self.show_submit_dialog = true;
            self.submit_has_error = true;
            return false;
        }
        let request = match self
            .workflow
            .as_ref()
            .map_err(|error| format!("no workflow loaded: {error}"))
            .and_then(build_submit_review_action_request)
        {
            Ok(request) => request,
            Err(error) => {
                self.status = format!("Submit unavailable: {error}");
                self.show_submit_dialog = true;
                self.submit_has_error = true;
                return false;
            }
        };

        self.show_submit_dialog = true;
        self.submit_is_pending = true;
        self.status = "Submitting review...".to_string();
        self.babel_service_requests
            .push(BabelServiceRequest::SubmitReviewAction(request));
        true
    }

    pub(super) fn complete_submit_review_success(&mut self) {
        if !self.submit_is_pending {
            return;
        }

        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::SubmitReviewAction(_))
        });
        self.submit_review();
    }

    pub(super) fn complete_submit_review_failure(&mut self, error: String) {
        if !self.submit_is_pending {
            return;
        }

        self.submit_is_pending = false;
        self.submit_has_error = true;
        self.show_submit_dialog = true;
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::SubmitReviewAction(_))
        });
        self.status = format!("Submit failed: {error}");
    }

    pub(super) fn begin_drop_claim_request(&mut self) -> bool {
        if self.drop_claim_is_pending {
            return false;
        }
        if self.block_read_only_edit("Drop claim") {
            return false;
        }

        self.drop_claim_is_pending = true;
        self.status = "Dropping claim...".to_string();
        if let Some(project) = self.project() {
            self.babel_service_requests
                .push(BabelServiceRequest::DropWorkerAction(
                    build_drop_worker_action_request(project),
                ));
        }
        true
    }

    pub(super) fn complete_drop_claim_success(&mut self) {
        if !self.drop_claim_is_pending {
            return;
        }

        self.drop_claim_is_pending = false;
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::DropWorkerAction(_))
        });
        self.drop_claim();
    }

    pub(super) fn complete_drop_claim_failure(&mut self, error: String) {
        if !self.drop_claim_is_pending {
            return;
        }

        self.drop_claim_is_pending = false;
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::DropWorkerAction(_))
        });
        self.status = format!("Drop claim failed: {error}");
    }

    pub(super) fn drop_claim(&mut self) {
        if self.block_read_only_edit("Drop claim") {
            return;
        }

        self.with_editable_workflow_mut("Drop claim", |workflow| {
            workflow.mark_claim_dropped();
            Ok("Claim dropped.".to_string())
        });
    }

    pub(super) fn request_skip_chunk(&mut self) {
        if self.skip_is_pending {
            self.show_skip_chunk_dialog = true;
            return;
        }

        if self.block_read_only_edit("Skip chunk") {
            self.show_skip_chunk_dialog = false;
            return;
        }

        let Some(project) = self.project() else {
            self.status = "Skip unavailable: no project loaded.".to_string();
            return;
        };
        if !EditorSession::skip_chunk_control_available(project) {
            self.show_skip_chunk_dialog = false;
            self.status = "Skip unavailable for this task.".to_string();
            return;
        }
        let skip_chunk = EditorSession::skip_chunk_control(
            project,
            self.show_skip_chunk_dialog,
            self.skip_is_pending,
        );
        if skip_chunk.trigger_disabled {
            self.show_skip_chunk_dialog = false;
            self.status = skip_chunk
                .trigger_tooltip
                .unwrap_or_else(|| "Skip unavailable.".to_string());
            return;
        }

        self.show_skip_chunk_dialog = true;
    }

    pub(super) fn cancel_skip_chunk(&mut self) {
        if !self.skip_is_pending {
            self.show_skip_chunk_dialog = false;
        }
    }

    pub(super) fn begin_skip_chunk_request(&mut self) -> bool {
        if self.skip_is_pending {
            return false;
        }
        if self.block_read_only_edit("Skip chunk") {
            return false;
        }

        let Some(project) = self.project() else {
            self.status = "Skip unavailable: no project loaded.".to_string();
            return false;
        };
        if !EditorSession::skip_chunk_control_available(project) {
            self.show_skip_chunk_dialog = false;
            self.status = "Skip unavailable for this task.".to_string();
            return false;
        }
        let skip_chunk =
            EditorSession::skip_chunk_control(project, self.show_skip_chunk_dialog, false);
        if skip_chunk.trigger_disabled {
            self.show_skip_chunk_dialog = false;
            self.status = skip_chunk
                .trigger_tooltip
                .unwrap_or_else(|| "Skip unavailable.".to_string());
            return false;
        }
        let request = build_skip_review_action_request(project);

        self.show_skip_chunk_dialog = true;
        self.skip_is_pending = true;
        self.status = "Skipping chunk...".to_string();
        self.babel_service_requests
            .push(BabelServiceRequest::SkipReviewAction(request));
        true
    }

    pub(super) fn confirm_skip_chunk(&mut self) {
        if self.block_read_only_edit("Skip chunk") {
            return;
        }

        self.skip_is_pending = false;
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::SkipReviewAction(_))
        });
        if self.with_editable_workflow_mut("Skip chunk", |workflow| {
            workflow.mark_skip_requested();
            Ok("Skip requested.".to_string())
        }) {
            self.show_skip_chunk_dialog = false;
        }
    }

    pub(super) fn fail_skip_chunk(&mut self, error: String) {
        if !self.skip_is_pending {
            return;
        }

        self.skip_is_pending = false;
        self.show_skip_chunk_dialog = true;
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::SkipReviewAction(_))
        });
        self.status = format!("Skip failed: {error}");
    }

    pub(super) fn request_admin_flag(&mut self) {
        if self.admin_flag_is_pending {
            self.show_admin_flag_dialog = true;
            return;
        }

        if self.block_read_only_edit("Flag recording") {
            self.show_admin_flag_dialog = false;
            return;
        }

        let Some(project) = self.project() else {
            self.status = "Flag recording unavailable: no project loaded.".to_string();
            return;
        };
        if !EditorSession::admin_flag_control_available(project) {
            self.show_admin_flag_dialog = false;
            self.status = "Flag recording unavailable for this task.".to_string();
            return;
        }

        self.show_admin_flag_dialog = true;
    }

    pub(super) fn cancel_admin_flag(&mut self) {
        if !self.admin_flag_is_pending {
            self.show_admin_flag_dialog = false;
        }
    }

    pub(super) fn close_admin_flag_from_open_change(&mut self) {
        if !self.admin_flag_is_pending {
            self.show_admin_flag_dialog = false;
            self.admin_flag_reason.clear();
        }
    }

    pub(super) fn begin_admin_flag_request(&mut self) -> bool {
        if self.admin_flag_is_pending {
            return false;
        }
        if self.block_read_only_edit("Flag recording") {
            return false;
        }

        let Some(project) = self.project().cloned() else {
            self.status = "Flag recording unavailable: no project loaded.".to_string();
            return false;
        };
        if !EditorSession::admin_flag_control_available(&project) {
            self.show_admin_flag_dialog = false;
            self.status = "Flag recording unavailable for this task.".to_string();
            return false;
        }

        self.show_admin_flag_dialog = true;
        self.admin_flag_is_pending = true;
        self.status = "Flagging recording...".to_string();
        let payload = EditorSession::admin_flag_payload(&self.admin_flag_reason);
        self.babel_service_requests
            .push(BabelServiceRequest::AdminRateReviewAction(
                build_admin_rate_review_action_request(&project, payload),
            ));
        true
    }

    pub(super) fn confirm_admin_flag(&mut self) {
        if self.block_read_only_edit("Flag recording") {
            return;
        }

        self.admin_flag_is_pending = false;
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::AdminRateReviewAction(_))
        });
        let payload = EditorSession::admin_flag_payload(&self.admin_flag_reason);
        if self.with_editable_workflow_mut("Flag recording", |workflow| {
            workflow.record_admin_flag(payload);
            Ok("Recording flagged.".to_string())
        }) {
            self.show_admin_flag_dialog = false;
        }
    }

    pub(super) fn fail_admin_flag(&mut self, error: String) {
        if !self.admin_flag_is_pending {
            return;
        }

        self.admin_flag_is_pending = false;
        self.show_admin_flag_dialog = true;
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::AdminRateReviewAction(_))
        });
        self.status = format!("Flag recording failed: {error}");
    }

    pub(super) fn mark_dirty(&mut self) {
        self.with_editor_mut(EditorSession::mark_dirty);
    }

    pub(super) fn mark_saved(&mut self) {
        self.with_editor_mut(EditorSession::mark_saved);
    }

    pub(super) fn reconcile_editor_with_project(&mut self) {
        if let Some(project) = self.project().cloned() {
            self.with_editor_mut(|editor| editor.reconcile_project(&project));
        }
    }

    pub(super) fn reconcile_editor_with_project_preserving_viewport(&mut self) {
        if let Some(project) = self.project().cloned() {
            self.with_editor_mut(|editor| editor.reconcile_project_preserving_viewport(&project));
        }
    }

    pub(super) fn segment_draft_differs(&self, segment: &Segment) -> Result<bool, String> {
        let Some(draft) = self.segment_drafts.get(&segment.id) else {
            return Ok(false);
        };
        let (start_seconds, end_seconds) = super::formatting::parse_draft_times(segment, draft)?;
        if start_seconds >= end_seconds {
            return Err("Start must be before end.".to_string());
        }

        Ok(segment.start_seconds != start_seconds
            || segment.end_seconds != end_seconds
            || segment.text != draft.text)
    }
}

impl TranscriptRowsCacheKey {
    fn from_editor(generation: u64, editor: &EditorSession) -> Self {
        Self {
            generation,
            track_filter: editor.track_filter.clone(),
            diff_view: editor.diff_view,
            asserted_warning_keys: editor.asserted_warning_keys.clone(),
        }
    }
}

impl TranscriptLayoutCacheKey {
    fn from_state(rows_generation: u64, layout_generation: u64, editor: &EditorSession) -> Self {
        Self {
            rows: TranscriptRowsCacheKey::from_editor(rows_generation, editor),
            layout_generation,
        }
    }
}

impl TimelineLanesCacheKey {
    fn from_editor(
        generation: u64,
        editor: &EditorSession,
        segment_preview: Option<&TimelineSegmentPreview>,
    ) -> Self {
        Self {
            generation,
            track_filter: editor.track_filter.clone(),
            collapsed_lane_ids: editor.collapsed_lane_ids.clone(),
            muted_lane_ids: editor.muted_lane_ids.clone(),
            lane_vertical_zoom: editor
                .lane_vertical_zoom
                .iter()
                .map(|(lane_id, zoom)| (lane_id.clone(), zoom.to_bits()))
                .collect(),
            lane_waveforms: editor
                .lane_waveforms
                .iter()
                .map(|(lane_id, waveform)| (lane_id.clone(), waveform.duration_seconds().to_bits()))
                .collect(),
            viewport_start_bits: editor.viewport.start_seconds.to_bits(),
            viewport_end_bits: editor.viewport.end_seconds.to_bits(),
            overlay_range: editor
                .overlay_range
                .map(|range| (range.start_seconds.to_bits(), range.end_seconds.to_bits())),
            segment_preview: segment_preview.map(TimelineSegmentPreviewCacheKey::from_preview),
        }
    }
}

impl TimelineSegmentPreviewCacheKey {
    fn from_preview(preview: &TimelineSegmentPreview) -> Self {
        Self {
            segment_id: preview.segment_id.clone(),
            lane_id: preview.lane_id.clone(),
            start_seconds_bits: preview.start_seconds.to_bits(),
            end_seconds_bits: preview.end_seconds.to_bits(),
        }
    }
}

impl OverviewWaveformCacheKey {
    fn from_editor(generation: u64, editor: &EditorSession) -> Self {
        Self {
            generation,
            track_filter: editor.track_filter.clone(),
            lane_waveforms: editor
                .lane_waveforms
                .iter()
                .map(|(lane_id, waveform)| (lane_id.clone(), waveform.duration_seconds().to_bits()))
                .collect(),
        }
    }
}

impl OverviewWaveformDynamicCacheKey {
    fn from_editor(editor: &EditorSession) -> Self {
        Self {
            viewport_start_bits: editor.viewport.start_seconds.to_bits(),
            viewport_end_bits: editor.viewport.end_seconds.to_bits(),
        }
    }
}

fn overview_waveform_with_dynamic_state(
    cached: &OverviewWaveformView,
    bounds: TimeRange,
    editor: &EditorSession,
) -> OverviewWaveformView {
    let duration = bounds.duration_seconds();
    OverviewWaveformView {
        waveform_model: cached.waveform_model.clone(),
        lane_waveform_models: Arc::clone(&cached.lane_waveform_models),
        blocks: Arc::clone(&cached.blocks),
        block_lane_rows: cached.block_lane_rows,
        peak_source: cached.peak_source,
        viewport_left_percent: percent_of_range_for_overview(
            editor.viewport.start_seconds,
            bounds,
            duration,
        ),
        viewport_width_percent: ((editor.viewport.duration_seconds() / duration) * 100.0)
            .clamp(0.0, 100.0) as f32,
        playhead_percent: percent_of_range_for_overview(editor.playhead_seconds, bounds, duration),
    }
}

fn percent_of_range_for_overview(value: f64, bounds: TimeRange, duration: f64) -> f32 {
    if duration <= 0.0 || !duration.is_finite() || !value.is_finite() {
        return 0.0;
    }

    (((value - bounds.start_seconds) / duration) * 100.0).clamp(0.0, 100.0) as f32
}

impl TransportStripCacheKey {
    fn from_editor(
        generation: u64,
        editor: &EditorSession,
        can_undo: bool,
        can_redo: bool,
    ) -> Self {
        Self {
            generation,
            track_filter: editor.track_filter.clone(),
            asserted_warning_keys: editor.asserted_warning_keys.clone(),
            is_playing: editor.is_playing,
            is_dirty: editor.is_dirty,
            is_saving: editor.is_saving,
            save_has_error: editor.save_has_error,
            review_state: format!("{:?}", editor.review_state),
            can_undo,
            can_redo,
        }
    }
}

impl SubmitControlCacheKey {
    fn from_state(generation: u64, editor: &EditorSession, state: &StudioState) -> Self {
        Self {
            generation,
            asserted_warning_keys: editor.asserted_warning_keys.clone(),
            show_submit_dialog: state.show_submit_dialog,
            submit_is_pending: state.submit_is_pending,
            submit_has_error: state.submit_has_error,
        }
    }
}

impl TranscriptRowsRenderCache {
    fn record_trace(&mut self, trace: TranscriptRowsCacheTrace) {
        if transcript_rows_cache_trace_enabled() {
            eprintln!("{}", trace.runtime_log_line());
        }
        self.last_trace = Some(trace);
    }
}

impl TranscriptRowsCacheTrace {
    fn new(
        event: TranscriptRowsCacheTraceEvent,
        key: &TranscriptRowsCacheKey,
        row_count: usize,
        miss_count: usize,
        playing: bool,
    ) -> Self {
        Self {
            event,
            generation: key.generation,
            row_count,
            miss_count,
            playing,
        }
    }

    fn runtime_log_line(&self) -> String {
        format!(
            "Transcription Studio transcript rows cache: event={} generation={} rows={} misses={} playing={}",
            self.event.as_str(),
            self.generation,
            self.row_count,
            self.miss_count,
            self.playing
        )
    }
}

impl TranscriptRowsCacheTraceEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
        }
    }
}

impl RuntimeAudioPlaybackInfo {
    pub(super) fn runtime_log_line(&self) -> String {
        format!(
            "Transcription Studio audio playback: tracks={} start={:.3}s speed={:.2}x",
            self.track_count, self.start_seconds, self.speed
        )
    }
}

pub(super) fn audio_playback_startup_runtime_trace_line(
    requested_track_count: usize,
    track_collect_elapsed: Duration,
    backend_elapsed: Duration,
    total_elapsed: Duration,
    outcome: &str,
) -> String {
    format!(
        "Transcription Studio audio startup: tracks={} outcome={} track_collect_ms={:.3} backend_ms={:.3} total_ms={:.3}",
        requested_track_count,
        outcome,
        track_collect_elapsed.as_secs_f64() * 1_000.0,
        backend_elapsed.as_secs_f64() * 1_000.0,
        total_elapsed.as_secs_f64() * 1_000.0
    )
}

fn transcript_rows_cache_trace_enabled() -> bool {
    std::env::var_os(TRANSCRIPT_ROWS_CACHE_TRACE_ENV).is_some_and(|value| value != "0")
}

fn audio_playback_trace_enabled() -> bool {
    std::env::var_os(AUDIO_PLAYBACK_TRACE_ENV).is_some_and(|value| value != "0")
}

fn audio_playback_startup_trace_enabled() -> bool {
    std::env::var_os(AUDIO_STARTUP_TRACE_ENV).is_some_and(|value| value != "0")
}

fn dashboard_projects_from_payload(source: &str) -> Result<Vec<DashboardProject>, String> {
    let value: Value = serde_json::from_str(source)
        .map_err(|error| format!("failed to decode Babel projects response: {error}"))?;
    let queue_configs = dashboard_queue_configs_by_project_id(&value);
    let mut projects = Vec::new();
    collect_dashboard_projects(&value, &queue_configs, &mut projects);
    projects.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    projects.dedup_by(|left, right| left.id == right.id);
    if projects.is_empty() {
        return Err("Babel returned no active projects with queue metadata.".to_string());
    }
    Ok(projects)
}

fn collect_dashboard_projects(
    value: &Value,
    queue_configs: &BTreeMap<String, DashboardQueueConfig>,
    projects: &mut Vec<DashboardProject>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_dashboard_projects(item, queue_configs, projects);
            }
        }
        Value::Object(map) => {
            if let Some(project) = dashboard_project_from_object(value, queue_configs) {
                projects.push(project);
            }
            for child in map.values() {
                collect_dashboard_projects(child, queue_configs, projects);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Default)]
struct DashboardQueueConfig {
    chunked_queue_id: String,
    stitched_queue_id: String,
}

fn dashboard_project_from_object(
    value: &Value,
    queue_configs: &BTreeMap<String, DashboardQueueConfig>,
) -> Option<DashboardProject> {
    if dashboard_object_is_queue_config_only(value) {
        return None;
    }
    if !dashboard_project_is_active(value) {
        return None;
    }

    let id = direct_string_by_keys(value, &["id", "projectId", "project_id", "slug", "name"])?;
    let raw_name = direct_string_by_keys(
        value,
        &["displayName", "display_name", "title", "label", "name"],
    )
    .unwrap_or_else(|| id.clone());
    let recovered = recovered_babel_project(&id).or_else(|| recovered_babel_project(&raw_name));
    let discovered = queue_configs.get(&id);
    let name = recovered
        .map(|project| project.display_name.to_string())
        .unwrap_or(raw_name);
    let chunked_queue_id = direct_string_by_keys(
        value,
        &[
            "chunkedQueueId",
            "chunked_queue_id",
            "chunkedReviewQueueId",
            "reviewQueueId",
            "queueId",
        ],
    )
    .or_else(|| discovered.map(|config| config.chunked_queue_id.clone()))
    .or_else(|| recovered.map(|project| project.chunked_queue_id.to_string()))
    .unwrap_or_default();
    let stitched_queue_id = direct_string_by_keys(
        value,
        &[
            "stitchedTxQueueId",
            "stitched_tx_queue_id",
            "stitchedQueueId",
            "stitched_queue_id",
        ],
    )
    .or_else(|| discovered.map(|config| config.stitched_queue_id.clone()))
    .or_else(|| recovered.map(|project| project.stitched_queue_id.to_string()))
    .unwrap_or_default();

    if chunked_queue_id.is_empty() && stitched_queue_id.is_empty() {
        return None;
    }

    Some(DashboardProject {
        id,
        name,
        chunked_queue_id,
        stitched_queue_id,
    })
}

fn dashboard_project_is_active(value: &Value) -> bool {
    if let Some(status) = direct_string_by_keys(value, &["status"])
        && !status.eq_ignore_ascii_case("active")
    {
        return false;
    }
    if let Some(permission_status) =
        direct_string_by_keys(value, &["permissionStatus", "permission_status"])
        && !permission_status.eq_ignore_ascii_case("active")
    {
        return false;
    }
    true
}

fn dashboard_queue_configs_by_project_id(value: &Value) -> BTreeMap<String, DashboardQueueConfig> {
    let mut configs = BTreeMap::new();
    collect_dashboard_queue_configs(value, &mut configs);
    configs
}

fn collect_dashboard_queue_configs(
    value: &Value,
    configs: &mut BTreeMap<String, DashboardQueueConfig>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_dashboard_queue_configs(item, configs);
            }
        }
        Value::Object(map) => {
            if let Some((project_id, config)) = dashboard_queue_config_from_object(value) {
                configs.insert(project_id, config);
            }
            for child in map.values() {
                collect_dashboard_queue_configs(child, configs);
            }
        }
        _ => {}
    }
}

fn dashboard_queue_config_from_object(value: &Value) -> Option<(String, DashboardQueueConfig)> {
    let project_id = direct_string_by_keys(value, &["projectId", "project_id", "id"])?;
    let chunked_queue_id = direct_string_by_keys(
        value,
        &[
            "chunkedQueueId",
            "chunked_queue_id",
            "chunkedReviewQueueId",
            "reviewQueueId",
            "queueId",
        ],
    )
    .unwrap_or_default();
    let stitched_queue_id = direct_string_by_keys(
        value,
        &[
            "stitchedTxQueueId",
            "stitched_tx_queue_id",
            "stitchedQueueId",
            "stitched_queue_id",
        ],
    )
    .unwrap_or_default();
    if chunked_queue_id.is_empty() && stitched_queue_id.is_empty() {
        return None;
    }
    Some((
        project_id,
        DashboardQueueConfig {
            chunked_queue_id,
            stitched_queue_id,
        },
    ))
}

fn dashboard_object_is_queue_config_only(value: &Value) -> bool {
    dashboard_queue_config_from_object(value).is_some()
        && direct_string_by_keys(
            value,
            &[
                "displayName",
                "display_name",
                "title",
                "label",
                "name",
                "slug",
                "status",
                "permissionStatus",
                "permission_status",
            ],
        )
        .is_none()
}

#[derive(Clone, Copy)]
struct RecoveredBabelProject {
    project_id: &'static str,
    slug: &'static str,
    display_name: &'static str,
    chunked_queue_id: &'static str,
    stitched_queue_id: &'static str,
}

fn recovered_babel_project(id_or_slug: &str) -> Option<RecoveredBabelProject> {
    const PROJECTS: &[RecoveredBabelProject] = &[
        RecoveredBabelProject {
            project_id: "0bf29f0c-371a-506a-8b77-5fffd14e5762",
            slug: "RU-tx-gold",
            display_name: "Russian Transcription - Gold",
            chunked_queue_id: "f95babcc-8681-521c-a39f-66454a16347f",
            stitched_queue_id: "a47543d7-8687-5ffe-95d3-754387fe3426",
        },
        RecoveredBabelProject {
            project_id: "73f8e130-9fb4-5c17-b142-5a2e75d89b6d",
            slug: "RU-tx-silver",
            display_name: "Russian Transcription - Silver",
            chunked_queue_id: "866acc27-eb40-51ee-bb4e-57b4cb15fb0c",
            stitched_queue_id: "a10c7c04-49bc-5376-97d6-03c3d154bfa8",
        },
        RecoveredBabelProject {
            project_id: "fc3f4db4-a201-5286-9a7c-411365bf6eb4",
            slug: "RU-tx-gold-non-bg",
            display_name: "Russian Transcription - Gold (Non-BG Noise)",
            chunked_queue_id: "40ab3a13-32d2-5095-a67c-eaa1fb109fbd",
            stitched_queue_id: "6d29da26-5417-5e96-a6f5-7235e1dd40e4",
        },
        RecoveredBabelProject {
            project_id: "d6031436-1fd5-4c79-9b8f-2a7452fc8c4b",
            slug: "RU-transcription",
            display_name: "Russian Transcription",
            chunked_queue_id: "bab7b5be-eb9e-43a0-b713-16b9d2fed66a",
            stitched_queue_id: "638f1a1f-d811-4140-ad5e-ec92a84c03ae",
        },
    ];
    let needle = id_or_slug.trim();
    PROJECTS
        .iter()
        .copied()
        .find(|project| project.project_id == needle || project.slug.eq_ignore_ascii_case(needle))
}

fn direct_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    let Value::Object(map) = value else {
        return None;
    };
    for key in keys {
        if let Some(found) = map.get(*key).and_then(Value::as_str) {
            let found = found.trim();
            if !found.is_empty() {
                return Some(found.to_string());
            }
        }
    }
    None
}

pub(super) fn startup_playback_smoke_enabled() -> bool {
    std::env::var_os(STARTUP_PLAYBACK_SMOKE_ENV).is_some_and(|value| value != "0")
}

pub(super) fn startup_playback_smoke_delay_frames_from_env() -> Option<usize> {
    std::env::var(STARTUP_PLAYBACK_SMOKE_DELAY_FRAMES_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|frames| *frames > 0)
}

fn metadata_f32(project: &Project, key: &str) -> Option<f32> {
    project
        .metadata
        .get(key)
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
}

fn project_review_action_level(project: &Project) -> Option<i64> {
    REVIEWER_ACTION_LEVEL_METADATA_KEYS
        .iter()
        .find_map(|key| {
            project
                .metadata
                .get(*key)
                .and_then(|value| review_action_level_from_text(value))
        })
        .or_else(|| {
            project
                .metadata
                .get(META_CAPABILITY_EXTRA_PACKAGE)
                .and_then(|value| review_action_level_from_text(value))
        })
}

fn review_action_level_from_text(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    parse_review_action_level_literal(value).or_else(|| {
        serde_json::from_str::<Value>(value)
            .ok()
            .and_then(|json| review_action_level_from_json(&json))
    })
}

fn review_action_level_from_json(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(value) => parse_review_action_level_literal(value),
        Value::Object(map) => REVIEWER_ACTION_LEVEL_JSON_KEYS
            .iter()
            .find_map(|key| map.get(*key).and_then(review_action_level_from_json)),
        _ => None,
    }
}

fn parse_review_action_level_literal(value: &str) -> Option<i64> {
    let value = value.trim();
    let value = value
        .strip_prefix('L')
        .or_else(|| value.strip_prefix('l'))
        .unwrap_or(value);
    value.parse::<i64>().ok()
}

fn metadata_string_set(project: &Project, key: &str) -> BTreeSet<String> {
    project
        .metadata
        .get(key)
        .and_then(|value| serde_json::from_str::<BTreeSet<String>>(value).ok())
        .unwrap_or_default()
}

fn metadata_f32_map(project: &Project, key: &str) -> BTreeMap<String, f32> {
    project
        .metadata
        .get(key)
        .and_then(|value| serde_json::from_str::<BTreeMap<String, f32>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, value)| value.is_finite())
        .collect()
}

fn audio_source_candidates(base: &Path, source_path: &str, speaker_id: &str) -> Vec<PathBuf> {
    let mut candidates = vec![resolve_audio_source_path(base, source_path)];
    if let Some(speaker_number) = speaker_id.strip_prefix("speaker-") {
        let local_name = format!("speaker_{speaker_number}.wav");
        push_candidate(&mut candidates, base.join(&local_name));
        push_candidate(&mut candidates, base.join("audio").join(local_name));
    }
    candidates
}

fn bundled_assets_dir() -> PathBuf {
    PathBuf::from(BUNDLED_ASSETS_DIR)
}

fn recovered_fixture_audio_dir() -> PathBuf {
    PathBuf::from(RECOVERED_FIXTURE_AUDIO_DIR)
}

fn resolve_audio_source_path(base: &Path, source_path: &str) -> PathBuf {
    let source_path = Path::new(source_path);
    if source_path.is_absolute() {
        source_path.to_path_buf()
    } else {
        base.join(source_path)
    }
}

fn push_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn sanitize_fraction(fraction: f32) -> f32 {
    if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn adjacent_babel_playback_speed(current_speed: f32, direction: i32) -> f32 {
    if direction >= 0 {
        BABEL_PLAYBACK_SPEED_STEPS
            .iter()
            .copied()
            .find(|speed| *speed > current_speed + 0.001)
            .unwrap_or(*BABEL_PLAYBACK_SPEED_STEPS.last().unwrap_or(&1.0))
    } else {
        BABEL_PLAYBACK_SPEED_STEPS
            .iter()
            .rev()
            .copied()
            .find(|speed| *speed < current_speed - 0.001)
            .unwrap_or(BABEL_PLAYBACK_SPEED_STEPS[0])
    }
}

fn normalize_speaker_workflow_label(label: &str) -> String {
    label.trim().to_ascii_lowercase().replace('-', " ")
}

fn speaker_workflow_lane_ids(project: &Project) -> Vec<String> {
    project
        .lanes
        .iter()
        .filter(|lane| {
            matches!(
                normalize_speaker_workflow_label(&lane.label).as_str(),
                "speaker 1" | "speaker 2"
            )
        })
        .map(|lane| lane.id.clone())
        .collect()
}

fn helper_proportional_restore_offset(
    text: &str,
    segment: &Segment,
    playhead_seconds: f64,
    baseline_offset: usize,
) -> usize {
    let text_len = text.chars().count();
    if text_len == 0 {
        return 0;
    }
    let duration = segment.end_seconds - segment.start_seconds;
    if !duration.is_finite() || duration <= 0.0 || !playhead_seconds.is_finite() {
        return baseline_offset.min(text_len);
    }
    let ratio = ((playhead_seconds - segment.start_seconds) / duration).clamp(0.0, 1.0);
    let offset = snap_to_word_boundary(text, (text_len as f64 * ratio).round() as usize);
    offset.max(baseline_offset).min(text_len)
}

fn snap_to_word_boundary(text: &str, offset: usize) -> usize {
    let characters = text.chars().collect::<Vec<_>>();
    let text_len = characters.len();
    if text_len == 0 || offset == 0 {
        return 0;
    }
    if offset >= text_len {
        return text_len;
    }
    if characters[offset - 1].is_whitespace() {
        return offset;
    }

    let mut backward = offset;
    while backward > 0 && !characters[backward - 1].is_whitespace() {
        backward -= 1;
    }

    let mut forward = offset;
    while forward < text_len && !characters[forward].is_whitespace() {
        forward += 1;
    }
    if forward < text_len {
        forward += 1;
    }

    if offset - backward <= forward.saturating_sub(offset) {
        backward
    } else {
        forward
    }
}

fn split_text_at_char_offset(text: &str, char_offset: usize) -> (String, String) {
    let byte_offset = text
        .char_indices()
        .nth(char_offset)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    (
        text[..byte_offset].to_string(),
        text[byte_offset..].to_string(),
    )
}

fn playback_tracks_for(project: &Project, editor: &EditorSession) -> Vec<PlaybackTrack> {
    project
        .lanes
        .iter()
        .filter(|lane| !editor.muted_lane_ids.contains(&lane.id))
        .filter_map(|lane| {
            editor
                .lane_waveforms
                .get(&lane.id)
                .and_then(|waveform| waveform.decoded_audio_shared())
                .map(|audio| PlaybackTrack::new(lane.id.clone(), audio))
        })
        .collect()
}
