//! Gold Draft (toolbar wand) and Gold segment transcription (Alt+Shift+G).
//!
//! Gold Draft follows the extension's overlay: word timestamps on the configured backend
//! (This PC / Remote GPU), draft text from those timings, then **writing the drafted segments
//! over the timed tracks** (replacement first, as Gold does), and optionally an OpenRouter
//! polish streamed row by row. "Restore original" restores the rows as written by the draft
//! (post-timestamps, pre-polish), not the transcript from before the draft. Word timings are
//! installed into the document (smart split, Alt+Click seek, ghost cursor, auto-segmentation)
//! and cached per task.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, WindowExt, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_core::Project;
use studio_editor::edit::{
    DraftedRow, RowSnapshot, TranscriptionTarget, remove_word_final_ellipsis,
};
use studio_inference::protocol::lane_name;
use studio_services::inference::{
    DraftResponse, InferenceBackend, InferenceError, InferenceErrorKind, InferenceProgress,
    InferenceResult, LaneAudio, PreservedRow, ProgressSink,
};
use studio_services::prefs::ExecutionLocation;
use studio_services::review::{
    AudioTrack, DraftAudioCueStatus, DraftAudioGuard, DraftAudioMode, DraftResult, DraftRow,
    DraftRowPreview, DraftStreamEvent, DraftSummary, DraftingConfig, apply_draft_rows,
    broker_audio_tracks, build_draft_generate_payload, draft_audio_cue_status, draft_row_previews,
    resolve_draft_audio,
};
use studio_waveform::DecodedAudio;

use super::gold_progress::{
    DraftProgress, DraftStage as Stage, DraftTextOverall, StageWeights, TimingOverall, label,
};
use super::review_workspace::{CloseModal, ModalFocus, badge, dialog_surface, modal};
use super::ui::{caption, muted};
use crate::editor::inference_audio::{LaneSource, read_lane_audio, until_cancelled};
use crate::editor::model::EditorModel;
use crate::editor::timing_cache::{SavedTiming, TimingCache, lane_timings, timed_lanes};
use crate::screens::ai_settings::AiSettingsView;
use crate::services::AppServices;

/// Changed/failed rows listed in the Changes panel.
const CHANGES_LIMIT: usize = 40;

/// Gold's purple wand accent.
fn accent() -> Hsla {
    rgb(0x7c3aed).into()
}

/// What a run produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftMode {
    /// Timestamps → draft text → writing segments → optional polish.
    Draft,
    /// Only word timestamps (installed into the document).
    TimestampsOnly,
}

/// A failure the dialog shows, with the backend's error kind to pick the action.
#[derive(Debug, Clone, PartialEq)]
struct RunError {
    kind: InferenceErrorKind,
    message: String,
}

impl RunError {
    fn other(message: impl Into<String>) -> Self {
        Self {
            kind: InferenceErrorKind::Other,
            message: message.into(),
        }
    }

    /// The editor closed while the run was in flight.
    fn gone() -> Self {
        Self {
            kind: InferenceErrorKind::Cancelled,
            message: "Gold Draft was closed.".to_string(),
        }
    }

    fn title(&self) -> &'static str {
        match self.kind {
            InferenceErrorKind::NeedsModels => "Set up the models on this PC first",
            InferenceErrorKind::NoWebGpu => "This PC can't run the local models",
            InferenceErrorKind::NoCapacity => "No remote GPU is free right now",
            InferenceErrorKind::ReleaseMismatch => "The server runs another model release",
            InferenceErrorKind::Unsupported => "This backend can't draft",
            InferenceErrorKind::Cancelled => "Cancelled",
            InferenceErrorKind::Other => "Gold Draft failed",
        }
    }

    /// Whether "Set up models" (AI settings) is the way forward.
    fn needs_setup(&self) -> bool {
        matches!(
            self.kind,
            InferenceErrorKind::NeedsModels
                | InferenceErrorKind::NoWebGpu
                | InferenceErrorKind::ReleaseMismatch
        )
    }
}

impl From<InferenceError> for RunError {
    fn from(error: InferenceError) -> Self {
        Self {
            kind: error.kind,
            message: error.message,
        }
    }
}

#[derive(Debug, Clone)]
enum Phase {
    Running {
        stage: Stage,
        /// "Creating timestamps on this PC", "Writing Draft text", …
        action: String,
        /// Current sub-step ("encoding speaker 1", "3/24 segments"); may be empty.
        step: String,
    },
    /// Polish waits for a choice about missing speaker audio.
    AudioGuard(DraftAudioGuard),
    Ready,
    Failed(RunError),
}

/// Streamed polish state.
#[derive(Debug, Clone, Default)]
struct Polish {
    rows: Vec<DraftRowPreview>,
    summary: Option<DraftSummary>,
    completed: usize,
    total: usize,
    done: bool,
    error: Option<String>,
}

impl Polish {
    /// Refinement failed: the written draft is kept, so rows streamed before the failure are
    /// dropped (neither listed as changes nor applied).
    fn fail(&mut self, error: String) {
        self.rows.clear();
        self.summary = None;
        self.error = Some(error);
    }

    /// Rows Apply draft writes: rewritten rows not applied yet.
    fn rows_to_apply(&self) -> Vec<DraftRow> {
        self.rows
            .iter()
            .filter(|preview| preview.row.status == "rewritten" && !preview.applied)
            .map(|preview| preview.row.clone())
            .collect()
    }
}

struct Run {
    mode: DraftMode,
    location: ExecutionLocation,
    task_id: String,
    phase: Phase,
    progress: DraftProgress,
    /// Overall progress of the timing run and of the draft-text pass behind the bar.
    timing: TimingOverall,
    text: DraftTextOverall,
    /// Rows as the draft wrote them (Restore original); `None` until written.
    snapshot: Option<Vec<RowSnapshot>>,
    polish: Option<Polish>,
    /// Outcome line of Apply / Restore (`true` = error).
    notice: Option<(String, bool)>,
    timed_tracks: usize,
    /// Refinement was requested for this run (Gold `refinementPending` until it succeeds).
    refine: bool,
}

impl Run {
    fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }
}

/// Inputs captured when a run starts.
struct RunInput {
    mode: DraftMode,
    task_id: String,
    backend: Arc<dyn InferenceBackend>,
    sources: Vec<LaneSource>,
    saved: Option<Arc<SavedTiming>>,
    /// Transcript rows when the run started; writing refuses if they changed meanwhile.
    before: Vec<RowSnapshot>,
    polish: bool,
}

/// Polish payload plus the speaker-audio capture in flight.
type PolishRequest = (
    serde_json::Value,
    Task<(DraftAudioCueStatus, Vec<AudioTrack>)>,
);

/// Toast identity of segment transcriptions (keyed by run number).
struct SegmentToast;

/// Progress of the newest segment transcription (its toast): the timing run, then the
/// one-row text pass, each with its own bar.
struct SegmentBars {
    seq: u64,
    /// "this PC" / "the remote GPU".
    place: &'static str,
    timing: TimingOverall,
    text: DraftTextOverall,
    shown: String,
}

pub struct GoldDraftPanel {
    model: Entity<EditorModel>,
    focus: ModalFocus,
    open: bool,
    run: Option<Run>,
    job: Option<Task<()>>,
    /// Latest timing run of this task (memory + per-task cache).
    saved: Option<Arc<SavedTiming>>,
    saved_checked: bool,
    cache: TimingCache,
    segment_seq: u64,
    segment_bars: Option<SegmentBars>,
    segment_job: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl GoldDraftPanel {
    pub fn new(model: Entity<EditorModel>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&model, |this, _, cx| this.load_saved_timing(cx))];
        let mut this = Self {
            model,
            focus: ModalFocus::new(cx),
            open: false,
            run: None,
            job: None,
            saved: None,
            saved_checked: false,
            cache: TimingCache::for_app(),
            segment_seq: 0,
            segment_bars: None,
            segment_job: None,
            _subscriptions: subscriptions,
        };
        this.load_saved_timing(cx);
        this
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn task_id(&self, cx: &App) -> String {
        self.model.read(cx).doc().project().id.clone()
    }

    fn status(&self, message: impl Into<SharedString>, cx: &mut App) {
        let message = message.into();
        self.model
            .update(cx, |model, cx| model.set_status(message, cx));
    }

    // ---- saved timings -----------------------------------------------------------------

    /// Once the lane audio finished loading: installs the task's cached timings when they
    /// were computed from exactly this audio.
    fn load_saved_timing(&mut self, cx: &mut Context<Self>) {
        if self.saved_checked {
            return;
        }
        let model = self.model.read(cx);
        let load = model.audio_load();
        if load.pending > 0 || load.loaded == 0 {
            return;
        }
        self.saved_checked = true;
        let sources = model.inference_lane_sources();
        let task_id = self.task_id(cx);
        let cache = self.cache.clone();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    let saved = cache.load(&task_id)?;
                    let lanes = read_lane_audio(&sources).ok()?;
                    saved
                        .matches(&timed_lanes(&lanes))
                        .then_some((saved, lanes))
                })
                .await;
            if let Some((saved, lanes)) = found {
                this.update(cx, |this, cx| {
                    if this.saved.is_none() {
                        this.install_timing(Arc::new(saved), &lanes, false, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Installs a timing run into the document and remembers it (`store`: also writes the
    /// per-task cache).
    fn install_timing(
        &mut self,
        saved: Arc<SavedTiming>,
        lanes: &[LaneAudio],
        store: bool,
        cx: &mut Context<Self>,
    ) {
        let timings = lane_timings(&saved.timing, lanes);
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| doc.install_word_timings(timings));
        });
        self.saved = Some(saved.clone());
        if store {
            let cache = self.cache.clone();
            let task_id = self.task_id(cx);
            cx.spawn(async move |this, cx| {
                let stored = cx
                    .background_spawn(async move { cache.store(&task_id, &saved) })
                    .await;
                if let Err(error) = stored {
                    this.update(cx, |this, cx| {
                        this.status(format!("Word timestamps could not be saved: {error}"), cx)
                    })
                    .ok();
                }
            })
            .detach();
        }
    }

    // ---- Gold Draft dialog ----------------------------------------------------------------

    /// Toolbar wand / Assistant drawer: opens the dialog and starts a run (an open dialog
    /// keeps its run).
    pub fn open(&mut self, mode: DraftMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        self.open = true;
        self.start(mode, window, cx);
    }

    /// Close: cancels inference in flight; rows already written stay.
    fn close(&mut self, cx: &mut Context<Self>) {
        if self.run.as_ref().is_some_and(Run::is_running) {
            self.status("Gold Draft cancelled.", cx);
        }
        self.job = None;
        self.run = None;
        self.open = false;
        cx.notify();
    }

    fn start(&mut self, mode: DraftMode, window: &mut Window, cx: &mut Context<Self>) {
        self.job = None;
        self.model.update(cx, |model, cx| {
            match model.update_doc(cx, |doc| doc.commit_pending_drafts("Gold Draft")) {
                Ok(outcome) => model.apply_outcome(outcome, cx),
                Err(error) => model.set_status(error.to_string(), cx),
            }
        });
        let services = cx.global::<AppServices>();
        let location = services.ai.execution_location;
        let polish = services.ai.gold_polish_enabled && mode == DraftMode::Draft;
        let backend = services.inference.backend(location);
        let model = self.model.read(cx);
        let doc = model.doc();
        let task_id = doc.project().id.clone();
        let load = model.audio_load();
        let sources = model.inference_lane_sources();
        let refusal = if doc.is_read_only() {
            Some("Gold Draft is unavailable: this task is read-only.".to_string())
        } else if load.pending > 0 {
            Some(
                "Waiting for audio: speaker audio is still loading. Try again in a moment."
                    .to_string(),
            )
        } else if sources.is_empty() {
            Some("Gold Draft needs the speaker audio, and none of it is loaded.".to_string())
        } else {
            None
        };
        let mut stages = match mode {
            DraftMode::Draft => vec![Stage::Timestamps, Stage::Drafting, Stage::Writing],
            DraftMode::TimestampsOnly => vec![Stage::Timestamps],
        };
        if polish {
            stages.push(Stage::Refining);
        }
        // Local row text is the LLM draft-text pass inside "drafting" (Gold
        // `DRAFT_STAGE_WEIGHTS_WITH_TEXT`); remote text arrives with the draft.
        let weights = match location {
            ExecutionLocation::Local => StageWeights::WithText,
            ExecutionLocation::Remote => StageWeights::TextReady,
        };
        let lane_count = sources.len().clamp(1, 2);
        let mut run = Run {
            mode,
            location,
            task_id: task_id.clone(),
            phase: Phase::Running {
                stage: Stage::Timestamps,
                action: "Reading transcript".to_string(),
                step: String::new(),
            },
            progress: DraftProgress::new(stages, weights),
            timing: TimingOverall::new(lane_count),
            text: DraftTextOverall::new(lane_count),
            snapshot: None,
            polish: None,
            notice: None,
            timed_tracks: 0,
            refine: polish,
        };
        if let Some(message) = refusal {
            run.phase = Phase::Failed(RunError::other(message));
            self.run = Some(run);
            cx.notify();
            return;
        }
        let input = RunInput {
            mode,
            task_id,
            backend,
            sources,
            saved: self.saved.clone(),
            before: doc.transcription_row_snapshot(),
            polish,
        };
        self.run = Some(run);
        self.job = Some(cx.spawn_in(window, async move |this, cx| {
            let result = run_draft(this.clone(), input, cx).await;
            this.update(cx, |this, cx| this.finish(result, cx)).ok();
        }));
        cx.notify();
    }

    fn finish(&mut self, result: Result<(), RunError>, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        match result {
            Ok(()) => {
                if matches!(run.phase, Phase::Running { .. }) {
                    run.phase = Phase::Ready;
                    run.progress.complete();
                }
            }
            Err(error) => run.phase = Phase::Failed(error),
        }
        let message = self.headline();
        self.status(message, cx);
        cx.notify();
    }

    /// Starts `stage`: later stages begin at their start share; the timestamps bar stays
    /// indeterminate until the timing run reports a value.
    fn enter(&mut self, stage: Stage, action: String, step: String, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut() {
            run.progress
                .advance(stage, (stage != Stage::Timestamps).then_some(0.0));
            run.phase = Phase::Running {
                stage,
                action,
                step,
            };
            cx.notify();
        }
    }

    /// A status step without a value of its own ("Capturing audio"); the bar holds.
    fn set_action(&mut self, action: &str, cx: &mut Context<Self>) {
        if let Some(Run {
            phase:
                Phase::Running {
                    action: shown,
                    step,
                    ..
                },
            ..
        }) = self.run.as_mut()
        {
            *shown = action.to_string();
            step.clear();
            cx.notify();
        }
    }

    /// Backend progress → the stage's own overall value (Gold `advanceL0TimingOverall` /
    /// `advanceDraftTextOverall`) → its share of the draft bar.
    fn on_progress(&mut self, progress: InferenceProgress, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let Phase::Running {
            stage,
            action,
            step,
        } = &mut run.phase
        else {
            return;
        };
        match *stage {
            Stage::Timestamps => {
                run.timing.advance(&progress);
                run.progress.advance(Stage::Timestamps, run.timing.fraction);
                step.clone_from(&run.timing.step);
            }
            Stage::Drafting => {
                if run.text.fraction.is_none() {
                    *action = "Writing Draft text".to_string();
                }
                run.text.advance(&progress);
                // Rendering the prepared rows is the small remainder of the stage.
                run.progress
                    .advance(Stage::Drafting, run.text.fraction.map(|f| 0.95 * f));
                step.clone_from(&run.text.step);
            }
            Stage::Writing | Stage::Refining => return,
        }
        cx.notify();
    }

    /// "Writing N segments": replaces the timed tracks' rows with the draft (one undo step)
    /// after checking nothing changed while inference ran.
    fn write_draft(
        &mut self,
        lanes: &[LaneAudio],
        timed_lane_ids: &[String],
        draft: DraftResponse,
        before: &[RowSnapshot],
        cx: &mut Context<Self>,
    ) -> Result<(), RunError> {
        let count = draft.rows.len();
        self.enter(
            Stage::Writing,
            format!("Writing {count} segments"),
            String::new(),
            cx,
        );
        let location = cx.global::<AppServices>().ai.execution_location;
        if self.run.as_ref().map(|run| run.location) != Some(location) {
            return Err(RunError::other(
                "\"Where to draft\" changed during the run. Generate again.",
            ));
        }
        // LLM rows stay verbatim (the A/B that chose the LLM judged its raw output); CTC rows
        // drop word-final ellipses (Gold overlay `runL0Replacement`).
        let llm_text = draft.models["e2e"]["llm"].as_bool() == Some(true);
        let rows = draft
            .rows
            .into_iter()
            .map(|row| {
                let lane = lanes
                    .iter()
                    .find(|lane| lane_name(lane) == row.lane)
                    .ok_or_else(|| {
                        RunError::other(format!(
                            "The draft returned rows for an unknown track `{}`.",
                            row.lane
                        ))
                    })?;
                Ok(DraftedRow {
                    lane_id: lane.lane_id.clone(),
                    start_seconds: row.start_seconds,
                    end_seconds: row.end_seconds,
                    text: if llm_text {
                        row.text
                    } else {
                        remove_word_final_ellipsis(&row.text)
                    },
                })
            })
            .collect::<Result<Vec<_>, RunError>>()?;
        let snapshot = self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                if doc.transcription_row_snapshot() != before {
                    return Err(RunError::other(
                        "The transcript changed while drafting. Generate again.",
                    ));
                }
                doc.replace_lane_rows(timed_lane_ids, rows)
                    .map_err(|error| RunError::other(error.to_string()))?;
                Ok(doc.transcription_row_snapshot())
            })
        })?;
        if let Some(run) = self.run.as_mut() {
            run.snapshot = Some(snapshot);
            run.timed_tracks = timed_lane_ids.len();
        }
        self.status(format!("Gold Draft wrote {count} segments."), cx);
        Ok(())
    }

    /// Retry refinement / the audio guard's choices.
    fn restart_polish(
        &mut self,
        mode: DraftAudioMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if run.is_running() || run.snapshot.is_none() {
            return;
        }
        run.notice = None;
        // A retry is its own operation with its own bar (Gold `beginDraftProgress(['refining'])`).
        run.progress = DraftProgress::new(vec![Stage::Refining], StageWeights::TextReady);
        run.phase = Phase::Running {
            stage: Stage::Refining,
            action: "Refining".to_string(),
            step: String::new(),
        };
        self.job = Some(cx.spawn_in(window, async move |this, cx| {
            let result = run_polish(this.clone(), mode, cx).await;
            this.update(cx, |this, cx| this.finish(result, cx)).ok();
        }));
        cx.notify();
    }

    /// Polish request inputs: payload from the current (drafted) rows plus captured audio.
    fn prepare_polish(&mut self, cx: &mut Context<Self>) -> Result<PolishRequest, String> {
        let settings = &cx.global::<AppServices>().ai;
        if !settings.gold_polish_enabled {
            return Err("Refinement is off in settings.".to_string());
        }
        let config = DraftingConfig::from_inputs(
            &settings.gold_openrouter_key,
            &settings.gold_model,
            &settings.gold_service_tier,
            &settings.gold_reasoning_effort,
            "",
        );
        let doc = self.model.read(cx).doc();
        let payload = build_draft_generate_payload(doc.project(), &config).map_err(|error| {
            if settings.gold_openrouter_key.trim().is_empty() {
                "Refinement needs an OpenRouter API key in settings.".to_string()
            } else {
                format!("{error:#}")
            }
        })?;
        Ok((payload, capture_audio(&self.model, cx)))
    }

    fn start_refining(&mut self, cx: &mut Context<Self>) {
        let segments = self
            .run
            .as_ref()
            .and_then(|run| run.snapshot.as_ref())
            .map_or(0, Vec::len);
        self.enter(
            Stage::Refining,
            "Refining".to_string(),
            format!("{segments} segments"),
            cx,
        );
        if let Some(run) = self.run.as_mut() {
            run.polish = Some(Polish::default());
        }
    }

    fn show_audio_guard(&mut self, guard: DraftAudioGuard, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut() {
            run.phase = Phase::AudioGuard(guard);
            cx.notify();
        }
    }

    fn apply_stream_event(&mut self, event: DraftStreamEvent, cx: &mut Context<Self>) {
        let project_text = |id: &str, cx: &App| {
            self.model
                .read(cx)
                .doc()
                .project()
                .segment(id)
                .map(|segment| segment.text.clone())
                .unwrap_or_default()
        };
        let original = match &event {
            DraftStreamEvent::Row { row, .. } => project_text(&row.row_id, cx),
            _ => String::new(),
        };
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let Some(polish) = run.polish.as_mut() else {
            return;
        };
        let step = match event {
            DraftStreamEvent::Started { total_rows } => {
                polish.total = total_rows;
                run.progress.advance(Stage::Refining, Some(0.0));
                format!("0/{total_rows} segments")
            }
            DraftStreamEvent::Row {
                row,
                completed_rows,
                total_rows,
                summary,
            } => {
                let preview = DraftRowPreview {
                    row,
                    original_text: original,
                    applied: false,
                    skipped: false,
                };
                match polish
                    .rows
                    .iter_mut()
                    .find(|existing| existing.row.row_id == preview.row.row_id)
                {
                    Some(existing) => *existing = preview,
                    None => polish.rows.push(preview),
                }
                if summary.is_some() {
                    polish.summary = summary;
                }
                polish.completed = completed_rows;
                polish.total = total_rows;
                if total_rows > 0 {
                    run.progress.advance(
                        Stage::Refining,
                        Some(completed_rows as f32 / total_rows as f32),
                    );
                }
                format!("{completed_rows}/{total_rows} segments")
            }
            DraftStreamEvent::Reconnecting => "connection lost, reconnecting".to_string(),
        };
        if let Phase::Running { step: shown, .. } = &mut run.phase {
            *shown = step;
        }
        cx.notify();
    }

    fn finish_polish(&mut self, result: Result<DraftResult, String>, cx: &mut Context<Self>) {
        let rows = match result.as_ref() {
            Ok(result) => draft_row_previews(self.model.read(cx).doc().project(), result),
            Err(_) => Vec::new(),
        };
        // Refined text drops word-final ellipses (Gold overlay `generateDraftFromCapture`).
        let rows: Vec<DraftRowPreview> = rows
            .into_iter()
            .map(|mut preview| {
                preview.row.rewritten_text =
                    remove_word_final_ellipsis(&preview.row.rewritten_text);
                preview
            })
            .collect();
        let Some(polish) = self.run.as_mut().and_then(|run| run.polish.as_mut()) else {
            return;
        };
        match result {
            Ok(result) => {
                polish.rows = rows;
                polish.summary = result.summary;
                polish.done = true;
            }
            Err(error) => polish.fail(error),
        }
        cx.notify();
    }

    /// Apply draft: writes the polished rows (one undo step) and closes; without polish the
    /// draft is already in the transcript.
    fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_ref() else {
            return;
        };
        let Some(snapshot) = run.snapshot.clone() else {
            return;
        };
        let rows = run
            .polish
            .as_ref()
            .map_or_else(Vec::new, Polish::rows_to_apply);
        if rows.is_empty() {
            let count = snapshot.len();
            self.status(format!("Applied to {count} segments."), cx);
            self.close(cx);
            return;
        }
        let result = self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                if !same_structure(&doc.transcription_row_snapshot(), &snapshot) {
                    return Err(
                        "Apply refused: segments were added, removed, moved or resized since the draft was written."
                            .to_string(),
                    );
                }
                doc.edit_workflow("Apply draft", |workflow| apply_draft_rows(workflow, &rows))
                    .map_err(|error| error.to_string())
            })
        });
        match result {
            Ok(applied) => {
                self.status(
                    format!("Applied to {} segments.", applied.applied.len()),
                    cx,
                );
                self.close(cx);
            }
            Err(error) => {
                if let Some(run) = self.run.as_mut() {
                    run.notice = Some((error, true));
                }
                cx.notify();
            }
        }
    }

    /// Restore original: the rows as the draft wrote them (before polish). Stays open.
    fn restore(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.run.as_ref().and_then(|run| run.snapshot.clone()) else {
            return;
        };
        let result = self.model.update(cx, |model, cx| {
            let result = model.update_doc(cx, |doc| doc.restore_row_texts(&snapshot));
            match result {
                Ok(outcome) => {
                    let message = outcome
                        .status
                        .clone()
                        .unwrap_or_else(|| "Restored.".to_string());
                    model.apply_outcome(outcome, cx);
                    Ok(message)
                }
                Err(error) => Err(error.to_string()),
            }
        });
        if let Some(run) = self.run.as_mut() {
            if let Some(polish) = run.polish.as_mut() {
                for preview in &mut polish.rows {
                    preview.applied = false;
                }
            }
            run.notice = Some(match result {
                Ok(message) => (message, false),
                Err(error) => (error, true),
            });
        }
        cx.notify();
    }

    /// Status line: `Action… 45% · sub-step`, or the outcome.
    fn headline(&self) -> String {
        let Some(run) = self.run.as_ref() else {
            return String::new();
        };
        match &run.phase {
            Phase::Running { action, step, .. } => label(action, run.progress.fraction, step),
            Phase::AudioGuard(_) => "Refinement needs a decision about speaker audio.".to_string(),
            Phase::Failed(error) => format!("{}: {}", error.title(), error.message),
            Phase::Ready => match run.mode {
                DraftMode::TimestampsOnly => format!(
                    "Timestamps ready for {} track(s). Smart split, Alt+Click and auto-segmentation use them now.",
                    self.saved
                        .as_ref()
                        .map_or(0, |saved| saved.timing.tracks.len())
                ),
                DraftMode::Draft => match &run.polish {
                    Some(Polish {
                        error: Some(error), ..
                    }) => format!(
                        "Draft kept. Refinement failed: {error} Apply the draft or retry refinement."
                    ),
                    Some(polish) => {
                        let (changed, _, failed) = polish_summary(polish);
                        format!("Draft ready. {changed} changed, {failed} left as is.")
                    }
                    None => format!(
                        "Draft ready: {} segments.",
                        run.snapshot.as_ref().map_or(0, Vec::len)
                    ),
                },
            },
        }
    }

    // ---- Alt+Shift+G ----------------------------------------------------------------------

    /// Transcribes the current empty row from its lane audio through the Gold backend
    /// (`draft` with the row preserved), then writes the text once if the row is still the
    /// current, unchanged, empty row.
    pub fn transcribe_segment(
        &mut self,
        focused: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.segment_seq += 1;
        let seq = self.segment_seq;
        let toast = move |message: String, kind: Option<bool>| {
            let note = match kind {
                None => Notification::info(message).autohide(false),
                Some(true) => Notification::success(message),
                Some(false) => Notification::error(message).autohide(false),
            };
            note.id1::<SegmentToast>(SharedString::from(format!("segment-{seq}")))
        };
        self.model.update(cx, |model, cx| {
            if let Ok(outcome) =
                model.update_doc(cx, |doc| doc.commit_pending_drafts("Transcribe segment"))
            {
                model.apply_outcome(outcome, cx);
            }
        });
        let model = self.model.read(cx);
        let prepared = model
            .doc()
            .transcription_target(focused.as_deref())
            .map_err(|refusal| refusal.to_string())
            .and_then(|target| {
                let sources = model.inference_lane_sources();
                if sources
                    .iter()
                    .any(|source| source.lane.id == target.lane_id)
                {
                    Ok((target, sources, model.doc().project().id.clone()))
                } else {
                    Err("Transcribe segment: this track's audio is not loaded.".to_string())
                }
            });
        let (target, sources, task_id) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => {
                window.push_notification(toast(message, Some(false)), cx);
                return;
            }
        };
        let services = cx.global::<AppServices>();
        let location = services.ai.execution_location;
        let place = match location {
            ExecutionLocation::Local => "this PC",
            ExecutionLocation::Remote => "the remote GPU",
        };
        let lane_count = sources.len().clamp(1, 2);
        let input = SegmentInput {
            task_id,
            backend: services.inference.backend(location),
            sources,
            saved: self.saved.clone(),
            target: target.clone(),
        };
        let first = format!("Transcribing on {place}…");
        window.push_notification(toast(first.clone(), None), cx);
        self.segment_bars = Some(SegmentBars {
            seq,
            place,
            timing: TimingOverall::new(lane_count),
            text: DraftTextOverall::new(lane_count),
            shown: first,
        });
        self.segment_job = Some(cx.spawn_in(window, async move |this, cx| {
            let result = run_segment(this.clone(), seq, input, cx).await;
            this.update_in(cx, |this, window, cx| {
                if this.segment_seq != seq {
                    return;
                }
                let outcome = match result {
                    Ok(text) => this.model.update(cx, |model, cx| {
                        let result = model.update_doc(cx, |doc| {
                            doc.apply_segment_transcription(&target, None, &text)
                        });
                        match result {
                            Ok(outcome) => {
                                let message = outcome.status.clone().unwrap_or_default();
                                model.apply_outcome(outcome, cx);
                                Ok(message)
                            }
                            Err(refusal) => Err(refusal.to_string()),
                        }
                    }),
                    Err(error) => Err(format!("Transcribe segment failed: {}", error.message)),
                };
                let (message, ok) = match outcome {
                    Ok(message) => (message, true),
                    Err(message) => (message, false),
                };
                this.status(message.clone(), cx);
                window.push_notification(toast(message, Some(ok)), cx);
            })
            .ok();
        }));
    }

    /// Progress of the newest segment transcription, shown in its toast.
    fn segment_progress(
        &mut self,
        seq: u64,
        stage: Stage,
        progress: InferenceProgress,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bars) = self
            .segment_bars
            .as_mut()
            .filter(|bars| bars.seq == seq && seq == self.segment_seq)
        else {
            return;
        };
        let message = if stage == Stage::Timestamps {
            bars.timing.advance(&progress);
            label(
                &format!("Creating timestamps on {}", bars.place),
                bars.timing.fraction,
                &bars.timing.step,
            )
        } else {
            bars.text.advance(&progress);
            label(
                &format!("Transcribing on {}", bars.place),
                bars.text.fraction,
                &bars.text.step,
            )
        };
        if message == bars.shown {
            return;
        }
        bars.shown.clone_from(&message);
        window.push_notification(
            Notification::info(message)
                .autohide(false)
                .id1::<SegmentToast>(SharedString::from(format!("segment-{seq}"))),
            cx,
        );
    }

    // ---- rendering ------------------------------------------------------------------------

    fn render_summary(&self, run: &Run, cx: &App) -> Div {
        let theme = cx.theme();
        let location = match run.location {
            ExecutionLocation::Local => "This PC",
            ExecutionLocation::Remote => "Remote GPU",
        };
        let segments = run.snapshot.as_ref().map_or_else(
            || self.model.read(cx).doc().transcription_row_snapshot().len(),
            Vec::len,
        );
        let mut pills = vec![
            badge(location, accent(), cx),
            badge(&format!("Task {}", run.task_id), theme.muted_foreground, cx),
            badge(&format!("{segments} segments"), theme.muted_foreground, cx),
        ];
        // Gold's pills: the result's rewritten/unchanged/failed counts (the written draft
        // counts as all rewritten until a refinement result replaces it), or the stream's
        // "done" count while refinement runs.
        let written = run.snapshot.as_ref().map(|rows| (rows.len(), 0, 0));
        let counts = match &run.polish {
            None => written,
            Some(polish) if polish.done => Some(polish_summary(polish)),
            Some(polish) if polish.error.is_some() => written,
            Some(polish) => {
                if polish.total > 0 {
                    pills.push(badge(
                        &format!("{}/{} done", polish.completed, polish.total),
                        theme.muted_foreground,
                        cx,
                    ));
                }
                polish.summary.as_ref().map(|summary| {
                    (
                        summary.rewritten_rows,
                        summary.unchanged_rows,
                        summary.failed_rows,
                    )
                })
            }
        };
        if let Some((rewritten, unchanged, failed)) = counts {
            pills.push(badge(&format!("{rewritten} rewritten"), theme.success, cx));
            pills.push(badge(
                &format!("{unchanged} unchanged"),
                theme.muted_foreground,
                cx,
            ));
            pills.push(badge(
                &format!("{failed} failed"),
                if failed > 0 {
                    theme.danger
                } else {
                    theme.muted_foreground
                },
                cx,
            ));
        }
        v_flex()
            .gap_1()
            .child(caption("Summary", cx))
            .child(h_flex().gap_1().flex_wrap().children(pills))
    }

    fn render_changes(&self, run: &Run, cx: &App) -> Div {
        let theme = cx.theme();
        let empty_state = |text: &'static str| muted(text, cx);
        let body: AnyElement = match (&run.snapshot, &run.polish) {
            (None, _) => empty_state("Waiting for the first segment…").into_any_element(),
            (Some(_), None) => empty_state("No changes.").into_any_element(),
            (Some(snapshot), Some(polish)) => {
                let changed: Vec<&DraftRowPreview> = polish
                    .rows
                    .iter()
                    .filter(|preview| preview.row.status != "unchanged")
                    .take(CHANGES_LIMIT)
                    .collect();
                if changed.is_empty() {
                    empty_state(if polish.done || polish.error.is_some() {
                        "No changes."
                    } else if polish.rows.is_empty() {
                        "Waiting for the first segment…"
                    } else {
                        "No changes yet."
                    })
                    .into_any_element()
                } else {
                    v_flex()
                        .gap_2()
                        .children(changed.into_iter().map(|preview| {
                            let index = snapshot
                                .iter()
                                .position(|row| row.segment_id == preview.row.row_id)
                                .map_or_else(|| "?".to_string(), |index| (index + 1).to_string());
                            let status_color = match preview.row.status.as_str() {
                                "rewritten" => theme.success,
                                "failed" => theme.danger,
                                _ => theme.muted_foreground,
                            };
                            let pane = |label: &'static str, text: &str| {
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_0p5()
                                    .p_2()
                                    .rounded(theme.radius)
                                    .bg(theme.secondary)
                                    .child(caption(label, cx))
                                    .child(div().text_sm().child(if text.trim().is_empty() {
                                        "(empty)".to_string()
                                    } else {
                                        text.to_string()
                                    }))
                            };
                            v_flex()
                                .gap_1()
                                .p_2()
                                .rounded(theme.radius)
                                .border_1()
                                .border_color(theme.border)
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(format!("Segment {index}")),
                                        )
                                        .child(badge(&preview.row.status, status_color, cx))
                                        .when(preview.applied, |row| {
                                            row.child(badge("applied", theme.success, cx))
                                        }),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_start()
                                        .child(pane("Before", &preview.original_text))
                                        .child(pane("After", &preview.row.rewritten_text)),
                                )
                                .when(!preview.row.warnings.is_empty(), |row| {
                                    row.child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.warning)
                                            .child(preview.row.warnings.join(" · ")),
                                    )
                                })
                        }))
                        .into_any_element()
                }
            }
        };
        v_flex().gap_1().child(caption("Changes", cx)).child(body)
    }

    fn render_problem(&self, run: &Run, cx: &mut Context<Self>) -> Option<Div> {
        let theme = cx.theme().clone();
        match &run.phase {
            Phase::Failed(error) if error.kind != InferenceErrorKind::Cancelled => {
                let setup = error.needs_setup();
                Some(
                    v_flex()
                        .gap_2()
                        .p_2()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.danger)
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.danger)
                                .child(error.title()),
                        )
                        .child(div().text_sm().child(error.message.clone()))
                        .child(
                            h_flex()
                                .gap_1()
                                .when(setup, |row| {
                                    row.child(
                                        Button::new("gold-setup-models")
                                            .small()
                                            .primary()
                                            .label("Set up models")
                                            .on_click(|_, window, cx| open_ai_settings(window, cx)),
                                    )
                                })
                                .child(
                                    Button::new("gold-try-again")
                                        .small()
                                        .outline()
                                        .label("Try again")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let mode = this
                                                .run
                                                .as_ref()
                                                .map_or(DraftMode::Draft, |run| run.mode);
                                            this.start(mode, window, cx);
                                        })),
                                ),
                        ),
                )
            }
            Phase::AudioGuard(guard) => {
                let message = if guard.captured_speaker_lanes == 0 {
                    "Couldn't capture speaker audio. The draft may miss laughter and other sounds."
                        .to_string()
                } else {
                    format!(
                        "Captured audio for {} of {} speakers. The draft may miss laughter and other sounds.",
                        guard.captured_speaker_lanes, guard.expected_speaker_lanes
                    )
                };
                Some(
                    v_flex()
                        .gap_2()
                        .p_2()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.warning)
                        .child(div().text_sm().child(message))
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("gold-retry-audio")
                                        .small()
                                        .outline()
                                        .label("Retry audio")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.restart_polish(DraftAudioMode::Guard, window, cx)
                                        })),
                                )
                                .when(
                                    guard.can_use_partial_audio && guard.captured_tracks > 0,
                                    |row| {
                                        row.child(
                                            Button::new("gold-use-audio")
                                                .small()
                                                .outline()
                                                .label("Use captured audio")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.restart_polish(
                                                        DraftAudioMode::UseAvailableAudio,
                                                        window,
                                                        cx,
                                                    )
                                                })),
                                        )
                                    },
                                )
                                .child(
                                    Button::new("gold-text-only")
                                        .small()
                                        .outline()
                                        .label("Continue without audio")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.restart_polish(
                                                DraftAudioMode::TextOnly,
                                                window,
                                                cx,
                                            )
                                        })),
                                ),
                        ),
                )
            }
            _ => None,
        }
    }
}

impl Render for GoldDraftPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus.sync(self.open && self.run.is_some(), window, cx);
        let Some(run) = self.run.as_ref().filter(|_| self.open) else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let running = run.is_running();
        let headline = self.headline();
        // Gold shows the bar only while busy (`activity.hidden = !busy`): a finished, failed
        // or paused run has no bar.
        let fraction = match &run.phase {
            Phase::Running { .. } => run.progress.fraction.map(|fraction| fraction * 100.0),
            _ => None,
        };
        let problem = self.render_problem(run, cx);
        let summary = (run.mode == DraftMode::Draft).then(|| self.render_summary(run, cx));
        let changes = (run.mode == DraftMode::Draft).then(|| self.render_changes(run, cx));
        let notice = run.notice.clone();
        let has_snapshot = run.snapshot.is_some();
        let polish = run.polish.as_ref();
        // Gold shows "Retry refinement" while a requested refinement is still pending
        // (failed, or waiting on the audio choice), not after it succeeded.
        let retry_polish = run.refine
            && has_snapshot
            && !running
            && polish.is_none_or(|polish| polish.error.is_some());
        let ready = matches!(run.phase, Phase::Ready);
        let draft_mode = run.mode == DraftMode::Draft;

        let header = h_flex()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .flex_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Icon::new(gpui_kit::assets::IconName::WandSparkles)
                                    .text_color(accent())
                                    .small(),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Gold Draft"),
                            ),
                    )
                    .child(muted("Review before you save or submit.", cx)),
            )
            .child(
                Button::new("gold-close")
                    .small()
                    .ghost()
                    .icon(IconName::Close)
                    .label("Close")
                    .tooltip(if running {
                        "Close and cancel the run"
                    } else {
                        "Close"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            );
        let status = v_flex()
            .gap_2()
            .child(div().text_sm().child(headline))
            .when_some(fraction, |this, value| {
                this.child(Progress::new("gold-progress").value(value))
            })
            .when(running && fraction.is_none(), |this| {
                this.child(Progress::new("gold-progress").loading(true))
            })
            .when_some(notice, |this, (text, error)| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(if error { theme.danger } else { theme.success })
                        .child(text),
                )
            });
        let footer = h_flex()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(theme.border)
            .when(draft_mode, |footer| {
                footer
                    .child(
                        Button::new("gold-restore")
                            .small()
                            .outline()
                            .label("Restore original")
                            .tooltip(
                                "Restore the segments as the draft wrote them, before refinement",
                            )
                            .disabled(running || !has_snapshot)
                            .on_click(cx.listener(|this, _, _, cx| this.restore(cx))),
                    )
                    .child(div().flex_1())
                    .when(retry_polish, |footer| {
                        footer.child(
                            Button::new("gold-retry-polish")
                                .small()
                                .outline()
                                .label("Retry refinement")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.restart_polish(DraftAudioMode::Guard, window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("gold-apply")
                            .small()
                            .primary()
                            .label("Apply draft")
                            .disabled(!ready || !has_snapshot)
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    )
            })
            .when(!draft_mode, |footer| {
                footer.child(div().flex_1()).child(
                    Button::new("gold-done")
                        .small()
                        .primary()
                        .label("Done")
                        .disabled(running)
                        .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                )
            });
        let surface = dialog_surface("gold-draft", rems(46.), accent(), cx)
            .child(header)
            .child(
                div()
                    .id("gold-draft-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .gap_3()
                            .p_3()
                            .child(status)
                            .children(problem)
                            .children(summary)
                            .children(changes),
                    ),
            )
            .child(footer);
        let this = cx.entity().downgrade();
        let close: CloseModal = Rc::new(move |_, cx| {
            this.update(cx, |this, cx| this.close(cx)).ok();
        });
        modal(surface, &self.focus, close, false).into_any_element()
    }
}

/// Opens Settings → AI (models, where to draft) over the editor.
pub fn open_ai_settings(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| AiSettingsView::new(window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(rems(44.).to_pixels(window.rem_size()))
            .title("AI settings")
            .child(
                div()
                    .id("ai-settings-dialog")
                    .max_h(rems(36.))
                    .overflow_y_scrollbar()
                    .child(view.clone()),
            )
    });
}

/// `(rewritten, unchanged, failed)` of a polish result.
fn polish_summary(polish: &Polish) -> (usize, usize, usize) {
    match &polish.summary {
        Some(summary) => (
            summary.rewritten_rows,
            summary.unchanged_rows,
            summary.failed_rows,
        ),
        None => {
            let count = |status: &str| {
                polish
                    .rows
                    .iter()
                    .filter(|preview| preview.row.status == status)
                    .count()
            };
            (count("rewritten"), count("unchanged"), count("failed"))
        }
    }
}

/// Same rows (ids, tracks, bounds), ignoring text.
fn same_structure(now: &[RowSnapshot], then: &[RowSnapshot]) -> bool {
    now.len() == then.len()
        && now.iter().zip(then).all(|(now, then)| {
            now.segment_id == then.segment_id
                && now.lane_id == then.lane_id
                && now.start_seconds == then.start_seconds
                && now.end_seconds == then.end_seconds
        })
}

/// Encodes the decoded lane audio to polish tracks off the UI thread.
fn capture_audio(
    model: &Entity<EditorModel>,
    cx: &mut App,
) -> Task<(DraftAudioCueStatus, Vec<AudioTrack>)> {
    let doc = model.read(cx).doc();
    let project: Project = doc.project().clone();
    let audio: std::collections::BTreeMap<String, Arc<DecodedAudio>> = doc
        .session()
        .lane_waveforms
        .iter()
        .filter_map(|(lane, waveform)| Some((lane.clone(), waveform.decoded_audio_shared()?)))
        .collect();
    cx.background_spawn(async move {
        let lookup = |lane: &str| audio.get(lane).map(AsRef::as_ref);
        let status = draft_audio_cue_status(&project, lookup);
        let tracks = broker_audio_tracks(&project, lookup);
        (status, tracks)
    })
}

/// Runs one backend operation on the services runtime, forwarding its progress to
/// `on_progress`. Dropping the calling job cancels the operation.
async fn call_backend<T, Fut>(
    this: &WeakEntity<GoldDraftPanel>,
    cx: &mut AsyncWindowContext,
    on_progress: impl Fn(
        &mut GoldDraftPanel,
        InferenceProgress,
        &mut Window,
        &mut Context<GoldDraftPanel>,
    ) + 'static,
    work: impl FnOnce(ProgressSink) -> Fut,
) -> InferenceResult<T>
where
    T: Send + 'static,
    Fut: Future<Output = InferenceResult<T>> + Send + 'static,
{
    let (progress_tx, progress_rx) = async_channel::unbounded();
    let sink = ProgressSink::new(move |event| {
        let _ = progress_tx.try_send(event);
    });
    let pump = {
        let this = this.clone();
        cx.spawn(async move |cx| {
            while let Ok(event) = progress_rx.recv().await {
                if this
                    .update_in(cx, |this, window, cx| on_progress(this, event, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
    };
    let (_cancel, cancelled) = async_channel::bounded::<()>(1);
    let work = work(sink);
    let result =
        studio_services::runtime::spawn(async move { until_cancelled(work, cancelled).await })
            .await;
    // A backend may keep clones of the sink; stop forwarding instead of waiting for it.
    drop(pump);
    result.unwrap_or_else(|| {
        Err(InferenceError::new(
            InferenceErrorKind::Cancelled,
            "Cancelled.",
        ))
    })
}

/// Reads the lanes' Original WAVs off the UI thread.
async fn read_lanes(
    sources: Vec<LaneSource>,
    cx: &mut AsyncWindowContext,
) -> Result<Vec<LaneAudio>, RunError> {
    cx.background_spawn(async move { read_lane_audio(&sources) })
        .await
        .map_err(|error| RunError::other(format!("{error:#}")))
}

/// The saved timing run when it covers exactly `lanes`, else a new one from `backend`.
/// Returns the run and whether it is new.
async fn timing_for(
    this: &WeakEntity<GoldDraftPanel>,
    cx: &mut AsyncWindowContext,
    backend: Arc<dyn InferenceBackend>,
    task_id: &str,
    lanes: &Arc<Vec<LaneAudio>>,
    saved: Option<Arc<SavedTiming>>,
    on_progress: impl Fn(
        &mut GoldDraftPanel,
        InferenceProgress,
        &mut Window,
        &mut Context<GoldDraftPanel>,
    ) + 'static,
) -> Result<(Arc<SavedTiming>, bool), RunError> {
    let keys = {
        let lanes = lanes.clone();
        cx.background_spawn(async move { timed_lanes(&lanes) })
            .await
    };
    if let Some(saved) = saved.filter(|saved| saved.matches(&keys)) {
        return Ok((saved, false));
    }
    let request_lanes = lanes.clone();
    let task = task_id.to_string();
    let timing = call_backend(this, cx, on_progress, move |sink| async move {
        backend.timing(&task, &request_lanes, sink).await
    })
    .await?;
    let saved = Arc::new(SavedTiming {
        lanes: keys,
        timing,
    });
    let installed = saved.clone();
    let lanes = lanes.clone();
    this.update(cx, |this, cx| {
        this.install_timing(installed, &lanes, true, cx)
    })
    .map_err(|_| RunError::gone())?;
    Ok((saved, true))
}

async fn run_draft(
    this: WeakEntity<GoldDraftPanel>,
    input: RunInput,
    cx: &mut AsyncWindowContext,
) -> Result<(), RunError> {
    let lanes = Arc::new(read_lanes(input.sources, cx).await?);
    this.update(cx, |this, cx| {
        let action = match this.run.as_ref().map(|run| run.location) {
            Some(ExecutionLocation::Remote) => "Creating timestamps on the remote GPU",
            _ => "Creating timestamps on this PC",
        };
        this.enter(Stage::Timestamps, action.to_string(), String::new(), cx)
    })
    .map_err(|_| RunError::gone())?;
    let (saved, _) = timing_for(
        &this,
        cx,
        input.backend.clone(),
        &input.task_id,
        &lanes,
        input.saved,
        |this, progress, _, cx| this.on_progress(progress, cx),
    )
    .await?;
    if input.mode == DraftMode::TimestampsOnly {
        return Ok(());
    }

    this.update(cx, |this, cx| {
        this.enter(Stage::Drafting, "Drafting".to_string(), String::new(), cx)
    })
    .map_err(|_| RunError::gone())?;
    let backend = input.backend.clone();
    let task = input.task_id.clone();
    let draft_lanes = lanes.clone();
    let draft_timing = saved.clone();
    let draft = call_backend(
        &this,
        cx,
        |this, progress, _, cx| this.on_progress(progress, cx),
        move |sink| async move {
            backend
                .draft(&task, &draft_lanes, &draft_timing.timing, None, sink)
                .await
        },
    )
    .await?;

    // Untimed lanes (no track in the timing result) keep their rows.
    let timed_lane_ids: Vec<String> = lanes
        .iter()
        .filter(|lane| {
            saved
                .timing
                .tracks
                .iter()
                .any(|track| track.lane == lane_name(lane))
        })
        .map(|lane| lane.lane_id.clone())
        .collect();
    this.update(cx, |this, cx| {
        this.write_draft(&lanes, &timed_lane_ids, draft, &input.before, cx)
    })
    .map_err(|_| RunError::gone())??;

    if input.polish {
        run_polish(this, DraftAudioMode::Guard, cx).await?;
    }
    Ok(())
}

/// Optional OpenRouter refinement of the written rows. A refinement failure keeps the draft
/// (recorded on the polish state, not returned as a run failure).
async fn run_polish(
    this: WeakEntity<GoldDraftPanel>,
    mode: DraftAudioMode,
    cx: &mut AsyncWindowContext,
) -> Result<(), RunError> {
    let prepared = this
        .update(cx, |this, cx| this.prepare_polish(cx))
        .map_err(|_| RunError::gone())?;
    let (payload, capture) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            this.update(cx, |this, cx| {
                this.start_refining(cx);
                this.finish_polish(Err(error), cx);
            })
            .map_err(|_| RunError::gone())?;
            return Ok(());
        }
    };
    this.update(cx, |this, cx| this.set_action("Capturing audio", cx))
        .map_err(|_| RunError::gone())?;
    let (status, tracks) = capture.await;
    let tracks = match resolve_draft_audio(mode, &status, tracks) {
        Ok(tracks) => tracks,
        Err(guard) => {
            this.update(cx, |this, cx| this.show_audio_guard(guard, cx))
                .map_err(|_| RunError::gone())?;
            return Ok(());
        }
    };
    this.update(cx, |this, cx| this.start_refining(cx))
        .map_err(|_| RunError::gone())?;
    let review = cx
        .update(|_, cx| cx.global::<AppServices>().review.clone())
        .map_err(|_| RunError::gone())?;
    let (events_tx, events_rx) = async_channel::unbounded::<DraftStreamEvent>();
    let pump = {
        let this = this.clone();
        cx.spawn(async move |cx| {
            while let Ok(event) = events_rx.recv().await {
                if this
                    .update(cx, |this, cx| this.apply_stream_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
    };
    let (_cancel, cancelled) = async_channel::bounded::<()>(1);
    let result = studio_services::runtime::spawn(async move {
        until_cancelled(
            review.generate_drafts(payload, tracks, move |event| {
                events_tx.try_send(event).ok();
            }),
            cancelled,
        )
        .await
    })
    .await;
    // Let already-sent rows land before the final result replaces them.
    pump.await;
    let result = match result {
        Some(result) => result.map_err(|error| format!("{error:#}")),
        None => Err("Cancelled.".to_string()),
    };
    this.update(cx, |this, cx| this.finish_polish(result, cx))
        .map_err(|_| RunError::gone())?;
    Ok(())
}

struct SegmentInput {
    task_id: String,
    backend: Arc<dyn InferenceBackend>,
    sources: Vec<LaneSource>,
    saved: Option<Arc<SavedTiming>>,
    target: TranscriptionTarget,
}

/// Alt+Shift+G backend part: timings (saved or new), then `draft` with the target row
/// preserved. Returns the row's text.
async fn run_segment(
    this: WeakEntity<GoldDraftPanel>,
    seq: u64,
    input: SegmentInput,
    cx: &mut AsyncWindowContext,
) -> Result<String, RunError> {
    let lanes = Arc::new(read_lanes(input.sources, cx).await?);
    let (saved, _) = timing_for(
        &this,
        cx,
        input.backend.clone(),
        &input.task_id,
        &lanes,
        input.saved,
        move |this, progress, window, cx| {
            this.segment_progress(seq, Stage::Timestamps, progress, window, cx)
        },
    )
    .await?;
    let target = input.target;
    let lane = lanes
        .iter()
        .find(|lane| lane.lane_id == target.lane_id)
        .ok_or_else(|| RunError::other("This track's audio is not loaded."))?;
    let row = PreservedRow {
        row_id: target.segment_id.clone(),
        speaker_key: lane_name(lane).to_string(),
        start_seconds: target.start_seconds,
        end_seconds: target.end_seconds,
        text: String::new(),
        index: u32::try_from(target.index).unwrap_or(u32::MAX),
    };
    let backend = input.backend;
    let task = input.task_id;
    let draft_lanes = lanes.clone();
    let response = call_backend(
        &this,
        cx,
        move |this, progress, window, cx| {
            this.segment_progress(seq, Stage::Drafting, progress, window, cx)
        },
        move |sink| async move {
            backend
                .draft(&task, &draft_lanes, &saved.timing, Some(vec![row]), sink)
                .await
        },
    )
    .await?;
    let text = response
        .rows
        .iter()
        .find(|row| row.id == target.segment_id)
        .or_else(|| (response.rows.len() == 1).then(|| &response.rows[0]))
        .map(|row| row.text.clone())
        .ok_or_else(|| RunError::other("the backend returned no text for this segment."))?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{DraftRow, DraftRowPreview, DraftSummary, Polish};

    fn streamed(id: &str, status: &str) -> DraftRowPreview {
        DraftRowPreview {
            row: DraftRow {
                row_id: id.to_string(),
                rewritten_text: format!("refined {id}"),
                status: status.to_string(),
                warnings: Vec::new(),
            },
            original_text: format!("draft {id}"),
            applied: false,
            skipped: false,
        }
    }

    #[test]
    fn a_failed_refinement_keeps_the_draft_instead_of_its_partial_rows() {
        let mut polish = Polish {
            rows: vec![streamed("a", "rewritten"), streamed("b", "unchanged")],
            summary: Some(DraftSummary {
                total_rows: 3,
                rewritten_rows: 1,
                unchanged_rows: 1,
                failed_rows: 0,
            }),
            completed: 2,
            total: 3,
            ..Polish::default()
        };
        assert_eq!(polish.rows_to_apply().len(), 1);

        polish.fail("model overloaded".to_string());

        assert!(polish.rows_to_apply().is_empty());
        assert!(polish.rows.is_empty());
        assert_eq!(polish.error.as_deref(), Some("model overloaded"));
    }
}
