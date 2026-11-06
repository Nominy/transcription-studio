//! Gold Draft (toolbar wand) and Gold segment transcription (Alt+Shift+G).
//!
//! Gold Draft follows the extension's overlay: word timestamps on the configured backend
//! (This PC / Remote GPU), draft text from those timings, then **writing the drafted segments
//! over the timed tracks** (replacement first, as Gold does), and optionally an OpenRouter
//! polish streamed row by row. "Undo polish" restores the rows as written by the draft
//! (post-timestamps, pre-polish), not the transcript from before the draft. Word timings are
//! installed into the document (smart split, Alt+Click seek, ghost cursor, auto-segmentation)
//! and cached per task.

use std::rc::Rc;
use std::sync::Arc;

use exaudio_extension::document::{DocumentEdit, DocumentTransaction};
use exaudio_gold::number_markup::strip_number_markup;
use exaudio_gold::polish::{
    AudioTrack, DraftAudioCueStatus, DraftAudioGuard, DraftAudioMode, DraftResult, DraftRow,
    DraftRowPreview, DraftStreamEvent, DraftSummary, DraftingConfig, broker_audio_tracks,
    draft_audio_cue_status, draft_row_previews, resolve_draft_audio,
};
use exaudio_gold::prefs::ExecutionLocation;
use exaudio_gold::protocol::lane_name;
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
use studio_inference::{
    DraftResponse, InferenceBackend, InferenceError, InferenceErrorKind, InferenceProgress,
    InferenceResult, LaneAudio, PreservedRow, ProgressSink,
};
use studio_waveform::DecodedAudio;

use super::gold_progress::{
    DraftProgress, DraftStage as Stage, DraftTextOverall, StageWeights, TimingOverall, label,
};
use crate::editor::inference_audio::{AudioReadError, read_lane_audio, until_cancelled};
use crate::editor::timing_cache::{SavedTiming, TimingCache, lane_timings, timed_lanes};
use crate::screens::ai_settings::AiSettingsView;
use crate::services::GoldServices;
use exaudio_shell::editor::model::{EditorModel, LaneSource};
use exaudio_shell::editor::panels::modal::{CloseModal, ModalFocus, badge, dialog_surface, modal};
use exaudio_shell::editor::panels::ui::{caption, muted};
use exaudio_shell::screens::ui::{details, error_notification, localized};

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

/// A failure the dialog shows, with the backend's error kind to pick the action. `message`
/// is plain text; `detail` is technical text shown under "Details".
#[derive(Debug, Clone, PartialEq)]
struct RunError {
    kind: InferenceErrorKind,
    message: String,
    detail: Option<String>,
}

impl RunError {
    fn other(message: impl Into<String>) -> Self {
        Self {
            kind: InferenceErrorKind::Other,
            message: message.into(),
            detail: None,
        }
    }

    /// The editor closed while the run was in flight.
    fn gone() -> Self {
        Self {
            kind: InferenceErrorKind::Cancelled,
            message: String::new(),
            detail: None,
        }
    }

    fn title(&self, cx: &App) -> &'static str {
        match self.kind {
            InferenceErrorKind::NeedsModels => {
                localized(cx, "Download the models first", "Сначала скачайте модели")
            }
            InferenceErrorKind::NoWebGpu => localized(
                cx,
                "This computer can't run the models",
                "Этот компьютер не может запускать модели",
            ),
            InferenceErrorKind::NoCapacity => {
                localized(cx, "The remote server is busy", "Удалённый сервер занят")
            }
            InferenceErrorKind::ReleaseMismatch => {
                localized(cx, "The server needs an update", "Серверу нужно обновление")
            }
            InferenceErrorKind::Unsupported => localized(
                cx,
                "Drafting isn't available here",
                "Черновик здесь недоступен",
            ),
            InferenceErrorKind::Cancelled => localized(cx, "Cancelled", "Отменено"),
            InferenceErrorKind::Other => {
                localized(cx, "Drafting failed", "Не удалось создать черновик")
            }
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
            detail: None,
        }
    }
}

#[derive(Debug, Clone)]
enum Phase {
    Running {
        stage: Stage,
        /// "Aligning words on this computer…", "Writing text…", …
        action: String,
        /// Current sub-step ("Encoding audio, speaker 1", "3 of 24 segments"); may be empty.
        step: String,
    },
    /// Polish waits for a choice about missing speaker audio.
    AudioGuard(DraftAudioGuard),
    Ready,
    Failed(RunError),
}

/// Why polishing stopped: a plain headline and, for a failed request, the technical text.
#[derive(Debug, Clone, PartialEq)]
struct PolishError {
    headline: String,
    detail: Option<String>,
}

impl PolishError {
    fn plain(headline: impl Into<String>) -> Self {
        Self {
            headline: headline.into(),
            detail: None,
        }
    }

    /// A polish request that failed: the draft stays, `detail` goes under "Details".
    fn failed(cx: &App, detail: String) -> Self {
        Self {
            headline: localized(
                cx,
                "Draft is ready, but polishing failed. Keep the draft or try polishing again.",
                "Черновик готов, но доработка не удалась. Оставьте черновик или попробуйте снова.",
            )
            .to_string(),
            detail: Some(detail),
        }
    }
}

/// Streamed polish state.
#[derive(Debug, Clone, Default)]
struct Polish {
    rows: Vec<DraftRowPreview>,
    summary: Option<DraftSummary>,
    completed: usize,
    total: usize,
    done: bool,
    error: Option<PolishError>,
}

impl Polish {
    /// Refinement failed: the written draft is kept, so rows streamed before the failure are
    /// dropped (neither listed as changes nor applied).
    fn fail(&mut self, error: PolishError) {
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
    location: ExecutionLocation,
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
    pub(crate) fn shutdown(&mut self, cx: &mut Context<Self>) {
        self.job.take();
        self.segment_job.take();
        self.segment_seq += 1;
        self.segment_bars = None;
        self.run = None;
        self.open = false;
        cx.notify();
    }

    fn task_id(&self, cx: &App) -> String {
        self.model.read(cx).doc().project().id.clone()
    }

    /// Whether this document keeps Babel's `N {СКАЗ: words}` number markup.
    fn number_markup(&self, cx: &App) -> bool {
        self.model.read(cx).doc().environment().policy.number_markup
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
                        this.install_timing(Arc::new(saved), &lanes, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Installs a timing run into the document and remembers it.
    fn install_timing(
        &mut self,
        saved: Arc<SavedTiming>,
        lanes: &[LaneAudio],
        cx: &mut Context<Self>,
    ) {
        let timings = lane_timings(&saved.timing, lanes);
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| doc.install_word_timings(timings));
        });
        self.saved = Some(saved);
    }

    /// Writes a timing run to the per-task cache; a failure is a toast.
    fn store_timing(&self, saved: Arc<SavedTiming>, window: &mut Window, cx: &mut Context<Self>) {
        let cache = self.cache.clone();
        let task_id = self.task_id(cx);
        let headline = localized(
            cx,
            "Couldn't save the word alignment.",
            "Не удалось сохранить выравнивание слов.",
        );
        cx.spawn_in(window, async move |this, cx| {
            let stored = cx
                .background_spawn(async move { cache.store(&task_id, &saved) })
                .await;
            if let Err(error) = stored {
                this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        error_notification(headline, Some(error.to_string().into())),
                        cx,
                    );
                })
                .ok();
            }
        })
        .detach();
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
            self.status(localized(cx, "Draft cancelled", "Черновик отменён"), cx);
        }
        self.job = None;
        self.run = None;
        self.open = false;
        cx.notify();
    }

    fn start(&mut self, mode: DraftMode, window: &mut Window, cx: &mut Context<Self>) {
        self.job = None;
        let commit_label = localized(cx, "AI draft", "Черновик ИИ");
        self.model.update(cx, |model, cx| {
            match model.update_doc(cx, |doc| doc.commit_pending_drafts(commit_label)) {
                Ok(outcome) => model.apply_outcome(outcome, cx),
                Err(error) => model.set_status(error.to_string(), cx),
            }
        });
        let services = cx.global::<GoldServices>();
        let location = services.ai.execution_location;
        let polish = services.ai.gold_polish_enabled
            && services.polish.is_some()
            && mode == DraftMode::Draft;
        let backend = services.inference.backend(location);
        let model = self.model.read(cx);
        let doc = model.doc();
        let task_id = doc.project().id.clone();
        let load = model.audio_load();
        let sources = model.inference_lane_sources();
        let refusal = if doc.is_read_only() {
            Some(localized(
                cx,
                "This project is read only.",
                "Этот проект только для чтения.",
            ))
        } else if load.pending > 0 {
            Some(localized(
                cx,
                "Audio is still loading. Try again in a moment.",
                "Аудио ещё загружается. Попробуйте через мгновение.",
            ))
        } else if sources.is_empty() {
            Some(localized(
                cx,
                "No audio is loaded for the speakers.",
                "Аудио спикеров не загружено.",
            ))
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
            phase: Phase::Running {
                stage: Stage::Timestamps,
                action: localized(cx, "Reading transcript…", "Чтение расшифровки…").to_string(),
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
        let message = self.headline(cx);
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

    /// A status step without a value of its own ("Preparing audio…"); the bar holds.
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
                *step = run.timing.step.text(cx);
            }
            Stage::Drafting => {
                if run.text.fraction.is_none() {
                    *action = localized(cx, "Writing text…", "Запись текста…").to_string();
                }
                run.text.advance(&progress);
                // Rendering the prepared rows is the small remainder of the stage.
                run.progress
                    .advance(Stage::Drafting, run.text.fraction.map(|f| 0.95 * f));
                *step = run.text.step.text(cx);
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
            localized(cx, "Writing {n} segments…", "Запись {n} сегментов…")
                .replace("{n}", &count.to_string()),
            String::new(),
            cx,
        );
        let location = cx.global::<GoldServices>().ai.execution_location;
        if self.run.as_ref().map(|run| run.location) != Some(location) {
            return Err(RunError::other(localized(
                cx,
                "Settings changed during the run. Start again.",
                "Настройки изменились во время работы. Начните заново.",
            )));
        }
        // LLM rows stay verbatim (the A/B that chose the LLM judged its raw output); CTC rows
        // drop word-final ellipses (Gold overlay `runL0Replacement`).
        let llm_text = draft.models["e2e"]["llm"].as_bool() == Some(true);
        let number_markup = self.number_markup(cx);
        let unknown_track = localized(
            cx,
            "The draft doesn't match the tracks in this project.",
            "Черновик не совпадает с дорожками в этом проекте.",
        );
        let changed = localized(
            cx,
            "The transcript changed during the run. Start again.",
            "Расшифровка изменилась во время работы. Начните заново.",
        );
        let rows = draft
            .rows
            .into_iter()
            .map(|row| {
                let lane = lanes
                    .iter()
                    .find(|lane| lane_name(lane) == row.lane)
                    .ok_or_else(|| RunError {
                        kind: InferenceErrorKind::Other,
                        message: unknown_track.to_string(),
                        detail: Some(row.lane.clone()),
                    })?;
                Ok(DraftedRow {
                    lane_id: lane.lane_id.clone(),
                    start_seconds: row.start_seconds,
                    end_seconds: row.end_seconds,
                    text: {
                        let text = if llm_text {
                            row.text
                        } else {
                            remove_word_final_ellipsis(&row.text)
                        };
                        spoken_text(text, number_markup)
                    },
                })
            })
            .collect::<Result<Vec<_>, RunError>>()?;
        let snapshot = self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                if doc.transcription_row_snapshot() != before {
                    return Err(RunError::other(changed));
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
        self.status(
            localized(
                cx,
                "Draft written to {n} segments",
                "Черновик записан в {n} сегментов",
            )
            .replace("{n}", &count.to_string()),
            cx,
        );
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
            action: localized(cx, "Polishing…", "Доработка…").to_string(),
            step: String::new(),
        };
        self.job = Some(cx.spawn_in(window, async move |this, cx| {
            let result = run_polish(this.clone(), mode, cx).await;
            this.update(cx, |this, cx| this.finish(result, cx)).ok();
        }));
        cx.notify();
    }

    /// Polish request inputs: payload from the current (drafted) rows plus captured audio.
    fn prepare_polish(&mut self, cx: &mut Context<Self>) -> Result<PolishRequest, PolishError> {
        let services = cx.global::<GoldServices>();
        let settings = &services.ai;
        let provider = services.polish.as_ref().ok_or_else(|| {
            PolishError::plain(localized(
                cx,
                "Polishing isn't available for this project.",
                "Доработка недоступна для этого проекта.",
            ))
        })?;
        if !settings.gold_polish_enabled {
            return Err(PolishError::plain(localized(
                cx,
                "Polishing is turned off in Settings › AI.",
                "Доработка выключена в Настройки › ИИ.",
            )));
        }
        let config = DraftingConfig::from_inputs(
            &settings.gold_openrouter_key,
            &settings.gold_model,
            &settings.gold_service_tier,
            &settings.gold_reasoning_effort,
            "",
        );
        let doc = self.model.read(cx).doc();
        let payload = provider
            .prepare_payload(doc.project(), &config)
            .map_err(|error| {
                if settings.gold_openrouter_key.trim().is_empty() {
                    PolishError::plain(localized(
                        cx,
                        "Add an OpenRouter key in Settings › AI to polish.",
                        "Добавьте ключ OpenRouter в Настройки › ИИ.",
                    ))
                } else {
                    PolishError::failed(cx, format!("{error:#}"))
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
            localized(cx, "Polishing…", "Доработка…").to_string(),
            localized(cx, "{n} segments", "{n} сегментов").replace("{n}", &segments.to_string()),
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
        let number_markup = self.number_markup(cx);
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
                localized(cx, "{i} of {n} segments", "{i} из {n} сегментов")
                    .replace("{i}", "0")
                    .replace("{n}", &total_rows.to_string())
            }
            DraftStreamEvent::Row {
                mut row,
                completed_rows,
                total_rows,
                summary,
            } => {
                row.rewritten_text = spoken_text(row.rewritten_text, number_markup);
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
                localized(cx, "{i} of {n} segments", "{i} из {n} сегментов")
                    .replace("{i}", &completed_rows.to_string())
                    .replace("{n}", &total_rows.to_string())
            }
            DraftStreamEvent::Reconnecting => localized(
                cx,
                "Connection lost, reconnecting…",
                "Связь потеряна, переподключение…",
            )
            .to_string(),
        };
        if let Phase::Running { step: shown, .. } = &mut run.phase {
            *shown = step;
        }
        cx.notify();
    }

    fn finish_polish(&mut self, result: Result<DraftResult, PolishError>, cx: &mut Context<Self>) {
        let rows = match result.as_ref() {
            Ok(result) => draft_row_previews(self.model.read(cx).doc().project(), result),
            Err(_) => Vec::new(),
        };
        // Refined text drops word-final ellipses (Gold overlay `generateDraftFromCapture`).
        let number_markup = self.number_markup(cx);
        let rows: Vec<DraftRowPreview> = rows
            .into_iter()
            .map(|mut preview| {
                preview.row.rewritten_text = spoken_text(
                    remove_word_final_ellipsis(&preview.row.rewritten_text),
                    number_markup,
                );
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
        let kept = localized(
            cx,
            "Draft kept for {n} segments",
            "Черновик оставлен для {n} сегментов",
        );
        if rows.is_empty() {
            let count = snapshot.len();
            self.status(kept.replace("{n}", &count.to_string()), cx);
            self.close(cx);
            return;
        }
        let structure_changed = localized(
            cx,
            "The segments changed since the draft was made. Make a new draft.",
            "Сегменты изменились после создания черновика. Сделайте новый черновик.",
        );
        let rows_gone = localized(
            cx,
            "The draft's segments are no longer in this project.",
            "Сегментов черновика больше нет в этом проекте.",
        );
        let undo_label = localized(cx, "Keep draft", "Оставить черновик");
        let result = self.model.update(cx, |model, cx| {
            let applied = model.update_doc(cx, |doc| {
                if !same_structure(&doc.transcription_row_snapshot(), &snapshot) {
                    return Err(structure_changed.to_string());
                }
                let edits: Vec<_> = rows
                    .into_iter()
                    .filter(|row| doc.project().segment(&row.row_id).is_some())
                    .map(|row| DocumentEdit::ReplaceText {
                        segment_id: row.row_id,
                        range: None,
                        text: row.rewritten_text,
                    })
                    .collect();
                let count = edits.len();
                if count == 0 {
                    return Err(rows_gone.to_string());
                }
                let outcome = doc
                    .apply_transaction(DocumentTransaction {
                        label: undo_label.to_string(),
                        expected_revision: doc.session().current_edit_revision(),
                        edits,
                    })
                    .map_err(|error| error.to_string())?;
                Ok((count, outcome))
            });
            applied.map(|(count, outcome)| {
                model.apply_outcome(outcome, cx);
                count
            })
        });
        match result {
            Ok(applied) => {
                self.status(kept.replace("{n}", &applied.to_string()), cx);
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
        let undone = localized(cx, "Polish undone", "Доработка отменена");
        let result = self.model.update(cx, |model, cx| {
            let result = model.update_doc(cx, |doc| doc.restore_row_texts(&snapshot));
            match result {
                Ok(outcome) => {
                    let message = outcome.status.clone().unwrap_or_else(|| undone.to_string());
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
    fn headline(&self, cx: &App) -> String {
        let Some(run) = self.run.as_ref() else {
            return String::new();
        };
        match &run.phase {
            Phase::Running { action, step, .. } => label(action, run.progress.fraction, step),
            Phase::AudioGuard(_) => localized(
                cx,
                "Polishing is waiting for your choice about the audio.",
                "Доработка ждёт вашего выбора по аудио.",
            )
            .to_string(),
            Phase::Failed(error) => error.title(cx).to_string(),
            Phase::Ready => match run.mode {
                DraftMode::TimestampsOnly => localized(
                    cx,
                    "Words aligned on {n} tracks. Smart split and Alt+click now snap to words.",
                    "Слова выровнены на {n} дорожках. Умное разделение и Alt+клик теперь привязаны к словам.",
                )
                .replace(
                    "{n}",
                    &self
                        .saved
                        .as_ref()
                        .map_or(0, |saved| saved.timing.tracks.len())
                        .to_string(),
                ),
                DraftMode::Draft => match &run.polish {
                    Some(Polish {
                        error: Some(error), ..
                    }) => error.headline.clone(),
                    Some(polish) => {
                        let (changed, unchanged, _) = polish_summary(polish);
                        localized(
                            cx,
                            "Draft ready — {a} segments changed, {b} unchanged.",
                            "Черновик готов — изменено {a}, без изменений {b}.",
                        )
                        .replace("{a}", &changed.to_string())
                        .replace("{b}", &unchanged.to_string())
                    }
                    None => localized(
                        cx,
                        "Draft ready — {n} segments.",
                        "Черновик готов — {n} сегментов.",
                    )
                    .replace(
                        "{n}",
                        &run.snapshot.as_ref().map_or(0, Vec::len).to_string(),
                    ),
                },
            },
        }
    }

    /// Technical text behind the status line (a failed polish request).
    fn headline_detail(&self) -> Option<SharedString> {
        let run = self.run.as_ref()?;
        if !matches!(run.phase, Phase::Ready) {
            return None;
        }
        run.polish
            .as_ref()?
            .error
            .as_ref()?
            .detail
            .clone()
            .map(SharedString::from)
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
        let toast_id = move |note: Notification| {
            note.id1::<SegmentToast>(SharedString::from(format!("segment-{seq}")))
        };
        let commit_label = localized(cx, "Fill segment", "Заполнить сегмент");
        self.model.update(cx, |model, cx| {
            if let Ok(outcome) = model.update_doc(cx, |doc| doc.commit_pending_drafts(commit_label))
            {
                model.apply_outcome(outcome, cx);
            }
        });
        let no_audio = localized(
            cx,
            "This track has no audio loaded.",
            "У этой дорожки не загружено аудио.",
        );
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
                    Err(no_audio.to_string())
                }
            });
        let (target, sources, task_id) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => {
                window.push_notification(toast_id(error_notification(message, None)), cx);
                return;
            }
        };
        let services = cx.global::<GoldServices>();
        let location = services.ai.execution_location;
        let lane_count = sources.len().clamp(1, 2);
        let input = SegmentInput {
            task_id,
            backend: services.inference.backend(location),
            sources,
            saved: self.saved.clone(),
            target: target.clone(),
        };
        let first = fill_action(location, cx);
        window.push_notification(
            toast_id(Notification::info(first.clone()).autohide(false)),
            cx,
        );
        self.segment_bars = Some(SegmentBars {
            seq,
            location,
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
                        let text =
                            spoken_text(text, model.doc().environment().policy.number_markup);
                        let result = model.update_doc(cx, |doc| {
                            doc.apply_segment_transcription(&target, None, &text)
                        });
                        match result {
                            Ok(outcome) => {
                                let message = outcome.status.clone().unwrap_or_default();
                                model.apply_outcome(outcome, cx);
                                Ok(message)
                            }
                            Err(refusal) => Err(RunError::other(refusal.to_string())),
                        }
                    }),
                    Err(error) => Err(error),
                };
                match outcome {
                    Ok(message) => {
                        this.status(message.clone(), cx);
                        window.push_notification(toast_id(Notification::success(message)), cx);
                    }
                    Err(error) => {
                        this.status(error.message.clone(), cx);
                        window.push_notification(
                            toast_id(
                                error_notification(
                                    error.message,
                                    error.detail.map(SharedString::from),
                                )
                                .autohide(false),
                            ),
                            cx,
                        );
                    }
                }
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
                align_action(bars.location, cx),
                bars.timing.fraction,
                &bars.timing.step.text(cx),
            )
        } else {
            bars.text.advance(&progress);
            label(
                &fill_action(bars.location, cx),
                bars.text.fraction,
                &bars.text.step.text(cx),
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
            ExecutionLocation::Local => localized(cx, "This computer", "Этот компьютер"),
            ExecutionLocation::Remote => localized(cx, "Remote server", "Удалённый сервер"),
        };
        let segments = run.snapshot.as_ref().map_or_else(
            || self.model.read(cx).doc().transcription_row_snapshot().len(),
            Vec::len,
        );
        let mut pills = vec![
            badge(location, accent(), cx),
            badge(
                &localized(cx, "{n} segments", "{n} сегментов")
                    .replace("{n}", &segments.to_string()),
                theme.muted_foreground,
                cx,
            ),
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
                        &localized(cx, "{a} of {b} done", "{a} из {b} готово")
                            .replace("{a}", &polish.completed.to_string())
                            .replace("{b}", &polish.total.to_string()),
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
            pills.push(badge(
                &localized(cx, "{n} changed", "{n} изменено")
                    .replace("{n}", &rewritten.to_string()),
                theme.success,
                cx,
            ));
            pills.push(badge(
                &localized(cx, "{n} unchanged", "{n} без изменений")
                    .replace("{n}", &unchanged.to_string()),
                theme.muted_foreground,
                cx,
            ));
            pills.push(badge(
                &localized(cx, "{n} failed", "{n} не удалось").replace("{n}", &failed.to_string()),
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
            .child(caption(localized(cx, "Summary", "Итог"), cx))
            .child(h_flex().gap_1().flex_wrap().children(pills))
    }

    /// The Changes list, matching the summary pills: the written rows until a refinement
    /// result replaces them, then the rewritten/failed rows. `None` when the summary says 0.
    fn render_changes(&self, run: &Run, cx: &App) -> Option<Div> {
        let theme = cx.theme();
        let snapshot = run.snapshot.as_ref();
        let polish = run.polish.as_ref().filter(|polish| polish.error.is_none());
        let changed_status = (localized(cx, "changed", "изменён"), theme.success);
        let waiting = || {
            muted(
                localized(
                    cx,
                    "Waiting for the first segment…",
                    "Ожидание первого сегмента…",
                ),
                cx,
            )
            .into_any_element()
        };
        let body: AnyElement = match (snapshot, polish) {
            (None, _) => waiting(),
            // The written draft: every row is new text.
            (Some(snapshot), None) => {
                if snapshot.is_empty() {
                    return None;
                }
                v_flex()
                    .gap_2()
                    .children(snapshot.iter().take(CHANGES_LIMIT).enumerate().map(
                        |(index, row)| {
                            change_card(
                                (index + 1).to_string(),
                                changed_status,
                                false,
                                None,
                                &row.text,
                                &[],
                                cx,
                            )
                        },
                    ))
                    .into_any_element()
            }
            (Some(snapshot), Some(polish)) => {
                let changed: Vec<&DraftRowPreview> = polish
                    .rows
                    .iter()
                    .filter(|preview| preview.row.status != "unchanged")
                    .take(CHANGES_LIMIT)
                    .collect();
                if changed.is_empty() {
                    if polish.done {
                        return None;
                    }
                    if polish.rows.is_empty() {
                        waiting()
                    } else {
                        muted(localized(cx, "No changes yet", "Пока без изменений"), cx)
                            .into_any_element()
                    }
                } else {
                    v_flex()
                        .gap_2()
                        .children(changed.into_iter().map(|preview| {
                            let index = snapshot
                                .iter()
                                .position(|row| row.segment_id == preview.row.row_id)
                                .map_or_else(|| "?".to_string(), |index| (index + 1).to_string());
                            let status = match preview.row.status.as_str() {
                                "rewritten" => changed_status,
                                "failed" => (localized(cx, "failed", "не удалось"), theme.danger),
                                _ => (localized(cx, "pending", "ожидает"), theme.muted_foreground),
                            };
                            change_card(
                                index,
                                status,
                                preview.applied,
                                Some(&preview.original_text),
                                &preview.row.rewritten_text,
                                &preview.row.warnings,
                                cx,
                            )
                        }))
                        .into_any_element()
                }
            }
        };
        Some(
            v_flex()
                .gap_1()
                .child(caption(localized(cx, "Changes", "Изменения"), cx))
                .child(body),
        )
    }

    fn render_problem(&self, run: &Run, cx: &mut Context<Self>) -> Option<Div> {
        let theme = cx.theme().clone();
        match &run.phase {
            Phase::Failed(error) if error.kind != InferenceErrorKind::Cancelled => {
                let setup = error.needs_setup();
                // The status line above is the title. A known kind's backend message restates
                // it, so only "Drafting failed" shows the message; otherwise it is a detail.
                let message = (!error.message.is_empty()).then(|| error.message.clone());
                let (message, detail) = if error.kind == InferenceErrorKind::Other {
                    (message, error.detail.clone())
                } else {
                    (None, error.detail.clone().or(message))
                };
                Some(
                    v_flex()
                        .gap_2()
                        .p_2()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.danger)
                        .when_some(message, |this, message| {
                            this.child(div().text_sm().child(message))
                        })
                        .when_some(detail, |this, detail| {
                            this.child(details(detail.into(), cx))
                        })
                        .child(
                            h_flex()
                                .gap_1()
                                .when(setup, |row| {
                                    row.child(
                                        Button::new("gold-setup-models")
                                            .small()
                                            .primary()
                                            .label(localized(
                                                cx,
                                                "Download models",
                                                "Скачать модели",
                                            ))
                                            .on_click(|_, window, cx| open_ai_settings(window, cx)),
                                    )
                                })
                                .child(
                                    Button::new("gold-try-again")
                                        .small()
                                        .outline()
                                        .label(localized(cx, "Try again", "Попробовать снова"))
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
                    localized(
                        cx,
                        "Couldn't read the audio. The draft may miss laughter and other sounds.",
                        "Не удалось прочитать аудио. В черновике могут отсутствовать смех и другие звуки.",
                    )
                    .to_string()
                } else {
                    localized(
                        cx,
                        "Only {a} of {b} speakers have audio. The draft may miss laughter and other sounds.",
                        "Аудио есть только у {a} из {b} спикеров. В черновике могут отсутствовать смех и другие звуки.",
                    )
                    .replace("{a}", &guard.captured_speaker_lanes.to_string())
                    .replace("{b}", &guard.expected_speaker_lanes.to_string())
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
                                        .label(localized(cx, "Try again", "Попробовать снова"))
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
                                                .label(localized(
                                                    cx,
                                                    "Continue with what loaded",
                                                    "Продолжить с тем, что загрузилось",
                                                ))
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
                                        .label(localized(
                                            cx,
                                            "Continue without audio",
                                            "Продолжить без аудио",
                                        ))
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
        let headline = self.headline(cx);
        let headline_detail = self.headline_detail();
        // Gold shows the bar only while busy (`activity.hidden = !busy`): a finished, failed
        // or paused run has no bar.
        let fraction = match &run.phase {
            Phase::Running { .. } => run.progress.fraction.map(|fraction| fraction * 100.0),
            _ => None,
        };
        let problem = self.render_problem(run, cx);
        // A run that failed before writing anything is the problem card alone.
        let show_result = run.mode == DraftMode::Draft
            && (run.snapshot.is_some() || !matches!(run.phase, Phase::Failed(_)));
        let summary = show_result.then(|| self.render_summary(run, cx));
        let changes = show_result.then(|| self.render_changes(run, cx)).flatten();
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
                                    .child(localized(cx, "AI draft", "Черновик ИИ")),
                            ),
                    )
                    .child(muted(
                        localized(
                            cx,
                            "Check the result before you keep it.",
                            "Проверьте результат, прежде чем оставить его.",
                        ),
                        cx,
                    )),
            )
            .child(
                Button::new("gold-close")
                    .small()
                    .ghost()
                    .icon(IconName::Close)
                    .label(localized(cx, "Close", "Закрыть"))
                    .tooltip(if running {
                        localized(cx, "Close and cancel", "Закрыть и отменить")
                    } else {
                        localized(cx, "Close", "Закрыть")
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            );
        let status = v_flex()
            .gap_2()
            .child(div().text_sm().child(headline))
            .when_some(headline_detail, |this, detail| {
                this.child(details(detail, cx))
            })
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
                            .label(localized(cx, "Undo polish", "Отменить доработку"))
                            .tooltip(localized(
                                cx,
                                "Go back to the text before polishing",
                                "Вернуть текст до доработки",
                            ))
                            .disabled(running || !has_snapshot)
                            .on_click(cx.listener(|this, _, _, cx| this.restore(cx))),
                    )
                    .child(div().flex_1())
                    .when(retry_polish, |footer| {
                        footer.child(
                            Button::new("gold-retry-polish")
                                .small()
                                .outline()
                                .label(localized(cx, "Polish again", "Доработать снова"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.restart_polish(DraftAudioMode::Guard, window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("gold-apply")
                            .small()
                            .primary()
                            .label(localized(cx, "Keep draft", "Оставить черновик"))
                            .disabled(!ready || !has_snapshot)
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    )
            })
            .when(!draft_mode, |footer| {
                footer.child(div().flex_1()).child(
                    Button::new("gold-done")
                        .small()
                        .primary()
                        .label(localized(cx, "Done", "Готово"))
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
    let title = localized(cx, "AI settings", "Настройки ИИ");
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(rems(44.).to_pixels(window.rem_size()))
            .title(title)
            .child(
                div()
                    .id("ai-settings-dialog")
                    .max_h(rems(36.))
                    .overflow_y_scrollbar()
                    .child(view.clone()),
            )
    });
}

/// "Aligning words on this computer…" / "… on the remote server…".
fn align_action(location: ExecutionLocation, cx: &App) -> &'static str {
    match location {
        ExecutionLocation::Local => localized(
            cx,
            "Aligning words on this computer…",
            "Выравнивание слов на этом компьютере…",
        ),
        ExecutionLocation::Remote => localized(
            cx,
            "Aligning words on the remote server…",
            "Выравнивание слов на удалённом сервере…",
        ),
    }
}

/// "Filling the segment on this computer…" / "… on the remote server…".
fn fill_action(location: ExecutionLocation, cx: &App) -> String {
    let place = match location {
        ExecutionLocation::Local => localized(cx, "this computer", "этот компьютер"),
        ExecutionLocation::Remote => localized(cx, "the remote server", "удалённый сервер"),
    };
    localized(
        cx,
        "Filling the segment on {place}…",
        "Заполнение сегмента: {place}…",
    )
    .replace("{place}", place)
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

/// Segment text as the document stores it: Babel keeps `N {СКАЗ: words}`, other documents
/// get the spoken words.
fn spoken_text(text: String, number_markup: bool) -> String {
    if number_markup {
        text
    } else {
        strip_number_markup(&text).into_owned()
    }
}

/// One row of the Changes list: `Segment i`, its status, the text (Before/After once a
/// refinement result exists) and the model's warnings.
fn change_card(
    index: String,
    (status_text, status_color): (&'static str, Hsla),
    applied: bool,
    before: Option<&str>,
    after: &str,
    warnings: &[String],
    cx: &App,
) -> Div {
    let theme = cx.theme();
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
                localized(cx, "(empty)", "(пусто)").to_string()
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
                        .child(localized(cx, "Segment {i}", "Сегмент {i}").replace("{i}", &index)),
                )
                .child(badge(status_text, status_color, cx))
                .when(applied, |row| {
                    row.child(badge(
                        localized(cx, "applied", "применён"),
                        theme.success,
                        cx,
                    ))
                }),
        )
        .child(match before {
            Some(before) => h_flex()
                .gap_2()
                .items_start()
                .child(pane(localized(cx, "Before", "До"), before))
                .child(pane(localized(cx, "After", "После"), after)),
            None => h_flex().child(pane(localized(cx, "Text", "Текст"), after)),
        })
        .when(!warnings.is_empty(), |row| {
            row.child(
                div()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(warnings.join(" · ")),
            )
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
    result.unwrap_or_else(|| Err(InferenceError::new(InferenceErrorKind::Cancelled, "")))
}

/// Reads the lanes' Original WAVs off the UI thread. A failure names the track in plain
/// words; the path and OS error go under "Details".
async fn read_lanes(
    sources: Vec<LaneSource>,
    cx: &mut AsyncWindowContext,
) -> Result<Vec<LaneAudio>, RunError> {
    let lanes = cx
        .background_spawn(async move { read_lane_audio(&sources) })
        .await;
    lanes.map_err(|error| {
        let detail = error
            .chain()
            .skip(1)
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(": ");
        let message = match error.downcast_ref::<AudioReadError>() {
            Some(read) => cx
                .update(|_, cx| {
                    localized(
                        cx,
                        "Couldn't read the audio for {label}.",
                        "Не удалось прочитать аудио дорожки {label}.",
                    )
                    .replace("{label}", &read.label)
                })
                .unwrap_or_default(),
            None => error.to_string(),
        };
        RunError {
            kind: InferenceErrorKind::Other,
            message,
            detail: Some(detail),
        }
    })
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
    this.update_in(cx, |this, window, cx| {
        this.install_timing(installed.clone(), &lanes, cx);
        this.store_timing(installed, window, cx);
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
        let location = this
            .run
            .as_ref()
            .map_or(ExecutionLocation::Local, |run| run.location);
        let action = align_action(location, cx).to_string();
        this.enter(Stage::Timestamps, action, String::new(), cx)
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
        let action = localized(cx, "Drafting…", "Черновик…").to_string();
        this.enter(Stage::Drafting, action, String::new(), cx)
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
    this.update(cx, |this, cx| {
        this.set_action(localized(cx, "Preparing audio…", "Подготовка аудио…"), cx)
    })
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
        .update(|_, cx| {
            cx.global::<GoldServices>().polish.clone().ok_or_else(|| {
                RunError::other(localized(
                    cx,
                    "Polishing isn't available for this project.",
                    "Доработка недоступна для этого проекта.",
                ))
            })
        })
        .map_err(|_| RunError::gone())??;
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
            review.generate_drafts(
                payload,
                tracks,
                Arc::new(move |event| {
                    events_tx.try_send(event).ok();
                }),
            ),
            cancelled,
        )
        .await
    })
    .await;
    // Let already-sent rows land before the final result replaces them.
    pump.await;
    this.update(cx, |this, cx| {
        let result = match result {
            Some(result) => result.map_err(|error| PolishError::failed(cx, format!("{error:#}"))),
            None => Err(PolishError::plain(localized(
                cx,
                "Polishing was cancelled.",
                "Доработка отменена.",
            ))),
        };
        this.finish_polish(result, cx)
    })
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
    let timing = timing_for(
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
    .await;
    let (saved, _) = timing.map_err(|error| fill_failed(error, cx))?;
    let target = input.target;
    let lane = lanes
        .iter()
        .find(|lane| lane.lane_id == target.lane_id)
        .ok_or_else(|| {
            RunError::other(localized_async(
                cx,
                "This track has no audio loaded.",
                "У этой дорожки не загружено аудио.",
            ))
        })?;
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
    .await
    .map_err(|error| fill_failed(error.into(), cx))?;
    let text = response
        .rows
        .iter()
        .find(|row| row.id == target.segment_id)
        .or_else(|| (response.rows.len() == 1).then(|| &response.rows[0]))
        .map(|row| row.text.clone())
        .ok_or_else(|| {
            RunError::other(localized_async(
                cx,
                "Nothing was heard in this segment.",
                "В этом сегменте ничего не распознано.",
            ))
        })?;
    Ok(text)
}

/// `localized` from an async window context.
fn localized_async(
    cx: &mut AsyncWindowContext,
    en: &'static str,
    ru: &'static str,
) -> &'static str {
    cx.update(|_, cx| localized(cx, en, ru)).unwrap_or(en)
}

/// A backend failure while filling a segment: one headline, the message under "Details".
fn fill_failed(error: RunError, cx: &mut AsyncWindowContext) -> RunError {
    let headline = localized_async(
        cx,
        "Couldn't fill the segment.",
        "Не удалось заполнить сегмент.",
    );
    RunError {
        kind: error.kind,
        message: headline.to_string(),
        detail: error
            .detail
            .or(Some(error.message))
            .filter(|detail| !detail.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{DraftRow, DraftRowPreview, DraftSummary, Polish, PolishError};

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

        polish.fail(PolishError::plain("model overloaded"));

        assert!(polish.rows_to_apply().is_empty());
        assert!(polish.rows.is_empty());
        assert!(polish.error.is_some());
    }
}
