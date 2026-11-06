//! Error-category task screen (Babel `error-categorization` projects, e.g. RU-TX-Error-Cat):
//! the human vs machine comparison of one diff, the target speaker's segment audio, Babel's
//! questionnaire (static categories, pairing rules, submit / discard via
//! `transcriptions.submitTranscriptionErrorLabel`) and Gold's assistant: the diff is located
//! in the audio automatically, and Initial fill answers only the unanswered choices. Nothing
//! is ever submitted for the worker.

use std::path::PathBuf;
use std::sync::Arc;

use exaudio_babel_services::babel::LoadedErrorTask;
use exaudio_babel_services::babel::error_task::{
    AlignBackend, AlignedDiff, AssistantPhase, AssistantStatus, BoundaryNote, CATEGORY_GROUPS,
    CancelToken, ClipSource, DecisionStore, DiffSide, ErrorChoices, ErrorLabelDraft, ErrorSource,
    ErrorTask, ErrorTaskError, ErrorTaskIdentity, ErrorTaskSnapshot, InitialFillRequest, LabelRow,
    MAX_NOTE_UNITS, NormalizedDiff, Receipt, StyleGuide, apply_draft, category_name,
    crop_aligned_error_diff_context, decisions_url, filtered_groups, find_alignment, group_index,
    group_name, grouped_label_categories, initial_fill_decision, label_rows, level_one_categories,
    picks_by_key, readiness, status_with_boundary_note, submission,
};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_audio::{AudioEngine, PlaybackState, decode_audio_file};
use studio_editor::edit::format::format_time;

use exaudio_babel_services::babel::BabelClient;
use exaudio_gold::prefs::AiPreferences;
use exaudio_gold_ui::services::GoldServices;
use exaudio_shell::editor::panels::ui::{dialog_footer, dialog_header};
use exaudio_shell::screens::ui::{self, Tone};

use crate::text::{count, failed, localized, localized_fmt};
use exaudio_shell::services::spawn_service_in;

const LANE: &str = "target";
const WHOLE_QUESTION: (&str, &str) = (
    "Does the highlight capture the whole error?",
    "Выделение охватывает всю ошибку?",
);
const MEANING_QUESTION: (&str, &str) = (
    "Did this error change the meaning?",
    "Эта ошибка меняет смысл?",
);
const MEANING_HELP: (&str, &str) = (
    "Yes if the difference changes what was said or meant. No if it is only formatting, spacing, case, a number written differently (\"24\" vs \"twenty-four\"), a filler, or a pronunciation variant.",
    "Да — если различие меняет сказанное или его смысл. Нет — если это только форматирование, пробелы, регистр, иная запись числа («24» и «двадцать четыре»), слово-паразит или вариант произношения.",
);
const HUMAN_QUESTION: (&str, &str) = (
    "Is the human transcript correct?",
    "Расшифровка человека верна?",
);
const HUMAN_HELP: (&str, &str) = (
    "Yes if it is correct, No if it contains an error. Tag the difference either way.",
    "Да — если верна, Нет — если содержит ошибку. В любом случае отметьте различие.",
);
const CANCELLED: (&str, &str) = ("Cancelled.", "Отменено.");

/// How the screen was opened (from the dashboard).
pub struct ErrorTaskOpen {
    pub client: BabelClient,
    pub loaded: LoadedErrorTask,
    /// Project the task belongs to (style guide for Initial fill).
    pub project_id: Option<String>,
    pub read_only: bool,
    /// "Next task" claims the next task of this project after submitting.
    pub next_project: Option<String>,
    pub status: &'static str,
}

pub enum ErrorTaskEvent {
    /// Back to the projects.
    Leave,
    /// Submitted; claim the next task of `project_id`.
    NextTask { project_id: String },
}

impl EventEmitter<ErrorTaskEvent> for ErrorTaskScreen {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterSubmit {
    NextTask,
    Exit,
}

enum AudioLoad {
    /// Synthetic diffs have no audio.
    None,
    /// The task has no recording for the diff's speaker.
    Missing,
    Loading,
    Ready(ClipSource),
    /// Decoding failed; the technical detail.
    Failed(SharedString),
}

/// One assistant step in flight. Dropping it cancels the step (a dispatched paid decision
/// still finishes into the cache).
struct Run {
    generation: u64,
    drafting: bool,
    cancel: CancelToken,
    _tasks: Vec<Task<()>>,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[derive(Default)]
struct Assistant {
    alignment: Option<AlignedDiff>,
    status: Option<AssistantStatus>,
    run: Option<Run>,
    generation: u64,
    /// Automatic alignment failed; only an explicit Initial fill retries.
    failed: bool,
    /// Initial fill waits for the alignment in flight.
    fill_after_alignment: bool,
}

pub struct ErrorTaskScreen {
    client: BabelClient,
    task: ErrorTask,
    project_id: Option<String>,
    read_only: bool,
    next_project: Option<String>,
    status: Option<SharedString>,
    choices: ErrorChoices,
    filter: Entity<InputState>,
    notes: Entity<InputState>,
    other_issues: Entity<InputState>,
    submitting: Option<AfterSubmit>,
    submitted: bool,
    submit_error: Option<SharedString>,
    submit_task: Option<Task<()>>,
    audio: AudioLoad,
    audio_task: Option<Task<()>>,
    engine: AudioEngine,
    frame_scheduled: bool,
    assistant: Assistant,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl ErrorTaskScreen {
    pub fn new(open: ErrorTaskOpen, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(localized(
                cx,
                "Filter categories, e.g. homophone, date, filler",
                "Фильтр категорий, например омофон, дата, слово-паразит",
            ))
        });
        let optional = localized(cx, "Optional", "Необязательно");
        let notes = cx.new(|cx| InputState::new(window, cx).placeholder(optional));
        let other_issues = cx.new(|cx| InputState::new(window, cx).placeholder(optional));
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |_, _, event, _, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&notes, window, |this, input, event, window, cx| {
                this.text_event(input, event, false, window, cx)
            }),
            cx.subscribe_in(&other_issues, window, |this, input, event, window, cx| {
                this.text_event(input, event, true, window, cx)
            }),
        ];
        let ErrorTaskOpen {
            client,
            loaded,
            project_id,
            read_only,
            next_project,
            status,
        } = open;
        let mut this = Self {
            client,
            project_id,
            read_only,
            next_project: next_project.filter(|_| !read_only),
            status: Some(status.into()),
            choices: ErrorChoices::default(),
            filter,
            notes,
            other_issues,
            submitting: None,
            submitted: false,
            submit_error: None,
            submit_task: None,
            audio: AudioLoad::None,
            audio_task: None,
            engine: AudioEngine::new(),
            frame_scheduled: false,
            assistant: Assistant::default(),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
            task: loaded.task,
        };
        this.load_audio(loaded.audio_base_dir, window, cx);
        this
    }

    /// Answers changed and not yet sent (leaving asks first, like Babel's unload guard).
    pub fn has_unsent_answers(&self) -> bool {
        !self.read_only && !self.submitted && self.choices != ErrorChoices::default()
    }

    /// Stops playback and the assistant and releases the decoded audio.
    pub fn unload(&mut self) {
        self.assistant.run = None;
        self.audio_task = None;
        self.audio = AudioLoad::None;
        self.engine.unload();
    }

    /// Runs `on_confirmed` now, or after the worker agrees to discard unsent answers.
    pub fn confirm_leave(
        &self,
        window: &mut Window,
        cx: &mut App,
        on_confirmed: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        if !self.has_unsent_answers() {
            window.defer(cx, on_confirmed);
            return;
        }
        let on_confirmed = std::cell::Cell::new(Some(on_confirmed));
        let on_confirmed = std::rc::Rc::new(on_confirmed);
        window.open_dialog(cx, move |dialog, window, cx| {
            let on_confirmed = on_confirmed.clone();
            dialog.w(rems(27.5).to_pixels(window.rem_size())).child(
                v_flex()
                    .gap_3()
                    .child(dialog_header(
                        localized(cx, "Leave without submitting?", "Выйти без отправки?"),
                        Some(
                            localized(
                                cx,
                                "Your answers for this diff will be lost.",
                                "Ваши ответы по этому диффу будут потеряны.",
                            )
                            .into(),
                        ),
                        cx,
                    ))
                    .child(
                        dialog_footer()
                            .child(
                                Button::new("error-task-stay")
                                    .label(localized(cx, "Keep labeling", "Продолжить разметку"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("error-task-discard")
                                    .danger()
                                    .label(localized(cx, "Leave", "Выйти"))
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        if let Some(confirmed) = on_confirmed.take() {
                                            window.defer(cx, confirmed);
                                        }
                                    }),
                            ),
                    ),
            )
        });
    }

    fn source(&self) -> ErrorSource {
        self.task.error.source
    }

    fn language(&self) -> &str {
        &self.task.error.language
    }

    // ---- audio --------------------------------------------------------------------------

    fn load_audio(&mut self, base: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(focus) = &self.task.focus else {
            return;
        };
        let Some(path) = focus.audio_path.clone() else {
            self.audio = AudioLoad::Missing;
            return;
        };
        let path = base.join(path);
        self.audio = AudioLoad::Loading;
        self.set_assistant(AssistantStatus::new(
            AssistantPhase::WaitingAudio,
            localized(cx, "Waiting for audio…", "Ожидание аудио…"),
        ));
        let decode = cx.background_spawn(async move { decode_audio_file(&path) });
        self.audio_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = decode.await;
            this.update_in(cx, |this, window, cx| {
                this.audio_task = None;
                match result {
                    Ok(decoded) => {
                        let audio = Arc::new(decoded);
                        if let Err(error) = this.engine.insert_lane(LANE, Arc::clone(&audio)) {
                            this.status = Some(Self::playback_failed(error, cx).into());
                        }
                        this.engine.set_timeline_end(Some(audio.duration()));
                        this.audio = AudioLoad::Ready(audio);
                        this.assistant.status = None;
                        this.start_alignment(false, window, cx);
                    }
                    Err(error) => {
                        this.audio = AudioLoad::Failed(format!("{error}").into());
                        this.assistant.status = None;
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn playback_failed(error: impl std::fmt::Display, cx: &App) -> String {
        failed(
            cx,
            "Can't play audio on this device.",
            "Не удаётся воспроизвести аудио на этом устройстве.",
            error,
        )
    }

    fn play(&mut self, start: f64, end: f64, cx: &mut Context<Self>) {
        if let Err(error) = self.engine.audition(start, end) {
            self.status = Some(Self::playback_failed(error, cx).into());
        }
        cx.notify();
    }

    fn pause(&mut self, cx: &mut Context<Self>) {
        self.engine.pause();
        cx.notify();
    }

    fn schedule_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_scheduled || self.engine.state() == PlaybackState::Stopped {
            return;
        }
        self.frame_scheduled = true;
        cx.on_next_frame(window, |this, window, cx| {
            this.frame_scheduled = false;
            if this.engine.poll().is_some() {
                cx.notify();
            }
            this.schedule_frame(window, cx);
        });
    }

    // ---- questionnaire ------------------------------------------------------------------

    fn set_choices(&mut self, change: impl FnOnce(&mut ErrorChoices), cx: &mut Context<Self>) {
        if self.read_only || self.submitting.is_some() || self.submitted {
            return;
        }
        change(&mut self.choices);
        self.submit_error = None;
        cx.notify();
    }

    fn text_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        other_issues: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let value = input.read(cx).value().to_string();
                let units: Vec<u16> = value.encode_utf16().collect();
                let value = if units.len() > MAX_NOTE_UNITS {
                    let clipped = String::from_utf16_lossy(&units[..MAX_NOTE_UNITS]);
                    input.update(cx, |input, cx| input.set_value(clipped.clone(), window, cx));
                    clipped
                } else {
                    value
                };
                self.set_choices(
                    |choices| {
                        if other_issues {
                            choices.other_issues = value;
                        } else {
                            choices.notes = value;
                        }
                    },
                    cx,
                );
            }
            InputEvent::PressEnter {
                secondary: true, ..
            } => self.submit(None, cx),
            _ => {}
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if event.keystroke.key == "enter" && (modifiers.control || modifiers.platform) {
            self.submit(None, cx);
            cx.stop_propagation();
        }
    }

    /// Submits (or discards) the label. `None` = the primary action (Next task when
    /// offered, else Submit and exit).
    fn submit(&mut self, after: Option<AfterSubmit>, cx: &mut Context<Self>) {
        if self.read_only || self.submitting.is_some() || self.submitted {
            return;
        }
        if readiness(&self.choices, self.source()).is_err() {
            return;
        }
        let after = after.unwrap_or(if self.next_project.is_some() {
            AfterSubmit::NextTask
        } else {
            AfterSubmit::Exit
        });
        let payload = submission(&self.task.review_action_id, &self.choices, self.source());
        let babel = self.client.clone();
        self.submitting = Some(after);
        self.submit_error = None;
        // The assistant never writes after a submit started.
        self.assistant.run = None;
        self.submit_task = Some(exaudio_shell::services::spawn_service(
            cx,
            async move { babel.submit_error_label(payload).await },
            move |this, result, cx| {
                this.submitting = None;
                this.submit_task = None;
                match result {
                    Ok(()) => {
                        this.submitted = true;
                        this.status =
                            Some(localized(cx, "Label submitted.", "Разметка отправлена.").into());
                        match (after, this.next_project.clone()) {
                            (AfterSubmit::NextTask, Some(project_id)) => {
                                cx.emit(ErrorTaskEvent::NextTask { project_id })
                            }
                            _ => cx.emit(ErrorTaskEvent::Leave),
                        }
                    }
                    Err(error) => {
                        this.submit_error = Some(error.message.into());
                    }
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        self.confirm_leave(window, cx, move |_, cx| {
            this.update(cx, |_, cx| cx.emit(ErrorTaskEvent::Leave)).ok();
        });
    }

    // ---- assistant ----------------------------------------------------------------------

    fn snapshot(&self) -> Option<ErrorTaskSnapshot> {
        let focus = self.task.focus.as_ref()?;
        (self.source() == ErrorSource::Real).then(|| ErrorTaskSnapshot {
            identity: ErrorTaskIdentity {
                review_action_id: self.task.review_action_id.clone(),
                diff_id: self.task.error.id.clone(),
                annotation_id: focus.annotation_id.clone(),
                track_id: focus.processed_recording_id.clone(),
            },
            language: self.task.error.language.clone(),
            diff: self.task.error.diff.clone(),
            segment: focus.interval(),
        })
    }

    fn set_assistant(&mut self, status: AssistantStatus) {
        self.assistant.status = Some(status);
    }

    fn assistant_error(&mut self, error: &ErrorTaskError, cx: &App) {
        self.set_assistant(AssistantStatus::new(
            if error.is_cancelled() {
                AssistantPhase::Cancelled
            } else {
                AssistantPhase::Error
            },
            if error.is_cancelled() {
                localized(cx, CANCELLED.0, CANCELLED.1).to_string()
            } else {
                error.message.clone()
            },
        ));
    }

    fn gold_inference_available(cx: &App) -> bool {
        cx.global::<exaudio_shell::ContributionRegistry>()
            .is_enabled("exaudio.gold")
            && cx.try_global::<GoldServices>().is_some()
    }

    fn align_backend(cx: &App) -> Option<AlignBackend> {
        if !Self::gold_inference_available(cx) {
            return None;
        }
        let services = cx.try_global::<GoldServices>()?;
        let ai = cx
            .global::<exaudio_shell::services::AppServices>()
            .storage
            .ai_settings();
        Some(AlignBackend {
            backend: services.inference.backend(ai.execution_location),
            location: ai.execution_location,
            remote_base_url: ai.remote_base_url,
            secrets: vec![
                ai.gold_openrouter_key,
                ai.remote_access_key,
                ai.review_openrouter_key,
            ],
        })
    }

    /// Starts a run and forwards its statuses while it is current.
    fn begin_run(
        &mut self,
        drafting: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (u64, CancelToken, async_channel::Sender<AssistantStatus>) {
        self.assistant.generation += 1;
        let generation = self.assistant.generation;
        let cancel = CancelToken::new();
        let (sender, receiver) = async_channel::unbounded::<AssistantStatus>();
        let forward = cx.spawn_in(window, async move |this, cx| {
            while let Ok(status) = receiver.recv().await {
                let applied = this.update(cx, |this, cx| {
                    if this.run_is_current(generation) {
                        let message = status_with_boundary_note(
                            status.phase,
                            &status.message,
                            this.assistant
                                .alignment
                                .as_ref()
                                .map(|aligned| &aligned.alignment),
                        );
                        this.assistant.status = Some(AssistantStatus { message, ..status });
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        self.assistant.run = Some(Run {
            generation,
            drafting,
            cancel: cancel.clone(),
            _tasks: vec![forward],
        });
        (generation, cancel, sender)
    }

    fn run_is_current(&self, generation: u64) -> bool {
        self.assistant
            .run
            .as_ref()
            .is_some_and(|run| run.generation == generation)
    }

    fn attach(&mut self, task: Task<()>) {
        if let Some(run) = self.assistant.run.as_mut() {
            run._tasks.push(task);
        }
    }

    /// Locates the diff in the audio (automatically once the audio is ready, or for an
    /// explicit Initial fill after a failure).
    fn start_alignment(&mut self, explicit: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.assistant.alignment.is_some() || self.assistant.run.is_some() || self.submitted {
            return;
        }
        if self.assistant.failed && !explicit {
            return;
        }
        let (Some(snapshot), AudioLoad::Ready(audio)) = (self.snapshot(), &self.audio) else {
            return;
        };
        let audio = Arc::clone(audio);
        self.assistant.failed = false;
        let Some(backend) = Self::align_backend(cx) else {
            self.assistant.fill_after_alignment = false;
            self.set_assistant(AssistantStatus::new(
                AssistantPhase::Cancelled,
                localized(
                    cx,
                    "Turn on the AI extension in Extensions to align audio and use Fill in.",
                    "Включите расширение ИИ в разделе «Расширения», чтобы выровнять аудио и использовать заполнение.",
                ),
            ));
            cx.notify();
            return;
        };
        let (generation, cancel, sender) = self.begin_run(false, window, cx);
        let task = spawn_service_in(
            window,
            cx,
            async move {
                let status = move |status: AssistantStatus| {
                    let _ = sender.try_send(status);
                };
                find_alignment(&snapshot, audio, &backend, &cancel, &status).await
            },
            move |this, result, window, cx| this.alignment_done(generation, result, window, cx),
        );
        self.attach(task);
        cx.notify();
    }

    fn alignment_done(
        &mut self,
        generation: u64,
        result: Result<AlignedDiff, ErrorTaskError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.run_is_current(generation) {
            return;
        }
        self.assistant.run = None;
        let fill = std::mem::take(&mut self.assistant.fill_after_alignment);
        match result {
            Ok(aligned) => {
                let message = match aligned.alignment.boundary_note {
                    Some(BoundaryNote::SegmentOnly) => localized(
                        cx,
                        "Using the whole segment. Check the audio and choices before you submit.",
                        "Используется весь сегмент. Проверьте аудио и ответы перед отправкой.",
                    ),
                    Some(BoundaryNote::OverlappingAsrWords) => localized(
                        cx,
                        "Words overlap here. Check the audio and choices before you submit.",
                        "Слова здесь накладываются. Проверьте аудио и ответы перед отправкой.",
                    ),
                    None => localized(
                        cx,
                        "Audio found. Check it and the choices before you submit.",
                        "Аудио найдено. Проверьте его и ответы перед отправкой.",
                    ),
                };
                self.set_assistant(AssistantStatus {
                    phase: AssistantPhase::Aligned,
                    message: message.to_string(),
                    receipt: Some(aligned.receipt.clone()),
                });
                self.assistant.alignment = Some(aligned);
                if fill {
                    self.load_style_guide(window, cx);
                }
            }
            Err(error) => {
                self.assistant.failed = true;
                self.assistant_error(&error, cx);
            }
        }
        cx.notify();
    }

    fn initial_fill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            self.set_assistant(AssistantStatus::new(
                AssistantPhase::Error,
                localized(
                    cx,
                    "This questionnaire is read-only.",
                    "Эта анкета только для чтения.",
                ),
            ));
            cx.notify();
            return;
        }
        if self.submitting.is_some() || self.submitted {
            return;
        }
        match &self.assistant.run {
            Some(run) if run.drafting => return,
            Some(_) => {
                // Alignment in flight: fill as soon as it finishes.
                self.assistant.fill_after_alignment = true;
                cx.notify();
                return;
            }
            None => {}
        }
        if self.assistant.alignment.is_some() {
            self.load_style_guide(window, cx);
            return;
        }
        if !matches!(self.audio, AudioLoad::Ready(_)) {
            self.set_assistant(AssistantStatus::new(
                AssistantPhase::Error,
                localized(
                    cx,
                    "Audio is still loading. Try again in a moment.",
                    "Аудио ещё загружается. Попробуйте чуть позже.",
                ),
            ));
            cx.notify();
            return;
        }
        self.assistant.fill_after_alignment = true;
        self.start_alignment(true, window, cx);
    }

    fn load_style_guide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_id.clone() else {
            self.set_assistant(AssistantStatus::new(
                AssistantPhase::Error,
                localized(
                    cx,
                    "Couldn’t find this project’s style guide. Open the task from its project.",
                    "Не удалось найти гайд проекта. Откройте задачу из её проекта.",
                ),
            ));
            cx.notify();
            return;
        };
        let babel = self.client.clone();
        let (generation, _, sender) = self.begin_run(true, window, cx);
        drop(sender);
        self.set_assistant(AssistantStatus::new(
            AssistantPhase::Drafting,
            localized(cx, "Loading the style guide…", "Загрузка гайда…"),
        ));
        let task = spawn_service_in(
            window,
            cx,
            async move { babel.error_task_style_guide(&project_id).await },
            move |this, result, window, cx| this.style_guide_loaded(generation, result, window, cx),
        );
        self.attach(task);
        cx.notify();
    }

    fn style_guide_loaded(
        &mut self,
        generation: u64,
        result: Result<StyleGuide, ErrorTaskError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.run_is_current(generation) {
            return;
        }
        self.assistant.run = None;
        let style_guide = match result {
            Ok(guide) => guide,
            Err(error) => {
                self.assistant_error(&error, cx);
                cx.notify();
                return;
            }
        };
        let (Some(snapshot), Some(aligned), AudioLoad::Ready(audio)) = (
            self.snapshot(),
            self.assistant.alignment.clone(),
            &self.audio,
        ) else {
            self.set_assistant(AssistantStatus::new(
                AssistantPhase::Error,
                localized(
                    cx,
                    "The audio isn’t ready. Try again.",
                    "Аудио не готово. Попробуйте ещё раз.",
                ),
            ));
            cx.notify();
            return;
        };
        let audio = Arc::clone(audio);
        // The answers Initial fill may complete: any edit before it returns wins.
        let baseline = self.choices.clone();
        let ai = cx
            .global::<exaudio_shell::services::AppServices>()
            .storage
            .ai_settings();
        let request = InitialFillRequest {
            snapshot,
            alignment: aligned.alignment,
            style_guide,
            api_key: ai.gold_openrouter_key,
            store: DecisionStore::default_location(),
            endpoint: decisions_url(),
        };
        let (generation, cancel, sender) = self.begin_run(true, window, cx);
        let task = spawn_service_in(
            window,
            cx,
            async move {
                let status = move |status: AssistantStatus| {
                    let _ = sender.try_send(status);
                };
                initial_fill_decision(request, audio, &cancel, &status).await
            },
            move |this, result, _, cx| this.fill_done(generation, baseline, result, cx),
        );
        self.attach(task);
        cx.notify();
    }

    fn fill_done(
        &mut self,
        generation: u64,
        baseline: ErrorChoices,
        result: Result<(ErrorLabelDraft, Receipt), ErrorTaskError>,
        cx: &mut Context<Self>,
    ) {
        if !self.run_is_current(generation) {
            return;
        }
        self.assistant.run = None;
        match result {
            Ok((draft, receipt)) => match apply_draft(&baseline, &self.choices, &draft) {
                Ok(next) => {
                    let message = if let Some(next) = next {
                        self.choices = next;
                        localized(
                            cx,
                            "Choices filled. Review them, then submit.",
                            "Ответы заполнены. Проверьте их и отправьте.",
                        )
                    } else {
                        localized(
                            cx,
                            "Your choices were already complete.",
                            "Ваши ответы уже заполнены.",
                        )
                    };
                    self.set_assistant(AssistantStatus {
                        phase: AssistantPhase::Filled,
                        message: status_with_boundary_note(
                            AssistantPhase::Filled,
                            message,
                            self.assistant
                                .alignment
                                .as_ref()
                                .map(|aligned| &aligned.alignment),
                        ),
                        receipt: Some(receipt),
                    });
                }
                Err(error) => self.set_assistant(AssistantStatus {
                    phase: AssistantPhase::Error,
                    message: error.to_string(),
                    receipt: Some(receipt),
                }),
            },
            Err(error) => self.assistant_error(&error, cx),
        }
        cx.notify();
    }

    fn cancel_assistant(&mut self, cx: &mut Context<Self>) {
        if self.assistant.run.take().is_some() {
            self.assistant.fill_after_alignment = false;
            if self.assistant.alignment.is_none() {
                self.assistant.failed = true;
            }
            self.set_assistant(AssistantStatus::new(
                AssistantPhase::Cancelled,
                localized(cx, CANCELLED.0, CANCELLED.1),
            ));
            cx.notify();
        }
    }
}

impl Focusable for ErrorTaskScreen {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// ---- rendering ----------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Palette {
    fg: Hsla,
    muted: Hsla,
    border: Hsla,
    surface: Hsla,
    group: Hsla,
    radius: Pixels,
    human: Hsla,
    machine: Hsla,
    warning: Hsla,
    success: Hsla,
    danger: Hsla,
    info: Hsla,
    primary: Hsla,
}

impl Palette {
    fn new(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            fg: theme.foreground,
            muted: theme.muted_foreground,
            border: theme.border,
            surface: theme.background,
            group: theme.group_box,
            radius: theme.radius_lg,
            human: theme.danger,
            machine: theme.success,
            warning: theme.warning,
            success: theme.success,
            danger: theme.danger,
            info: theme.info,
            primary: theme.primary,
        }
    }
}

fn tone_label(text: &'static str, color: Hsla) -> Div {
    div()
        .flex_none()
        .w(rems(4.25))
        .pt_0p5()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(text.to_uppercase())
}

/// Text with its UTF-16 span highlighted; an empty span shows `∅`.
fn passage(side: &DiffSide, color: Hsla) -> StyledText {
    let (before, highlighted, after) = side.parts();
    let marked = if highlighted.is_empty() {
        "∅".to_string()
    } else {
        highlighted
    };
    let start = before.len();
    let end = start + marked.len();
    let text = format!("{before}{marked}{after}");
    StyledText::new(SharedString::from(text)).with_highlights([(
        start..end,
        HighlightStyle {
            color: Some(color),
            background_color: Some(color.opacity(0.14)),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        },
    )])
}

fn quoted(text: &str) -> String {
    if text.is_empty() {
        "∅".to_string()
    } else {
        format!("\"{text}\"")
    }
}

fn yes_no(value: Option<bool>, cx: &App) -> &'static str {
    match value {
        None => "-",
        Some(true) => localized(cx, "Yes", "Да"),
        Some(false) => localized(cx, "No", "Нет"),
    }
}

impl ErrorTaskScreen {
    fn displayed_diff(&self) -> NormalizedDiff {
        self.assistant
            .alignment
            .as_ref()
            .and_then(|aligned| {
                crop_aligned_error_diff_context(&self.task.error.diff, &aligned.alignment).ok()
            })
            .unwrap_or_else(|| self.task.error.diff.clone())
    }

    fn render_header(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let title = if self.read_only {
            localized(
                cx,
                "Error task (read-only)",
                "Задача по ошибкам (только чтение)",
            )
        } else if self.task.error.is_qa_review {
            localized(cx, "QA review", "Проверка QA")
        } else {
            localized(cx, "Label this diff", "Разметьте дифф")
        };
        h_flex()
            .w_full()
            .gap_3()
            .child(
                Button::new("error-task-back")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowLeft)
                    .label(localized(cx, "Projects", "Проекты"))
                    .on_click(cx.listener(|this, _, window, cx| this.leave(window, cx))),
            )
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(div().flex_1())
            .when_some(self.status.clone(), |this, status| {
                this.child(div().text_xs().text_color(palette.muted).child(status))
            })
    }

    /// "Human vs Machine · Diff … · time · Diff type · Synthetic" (Babel's player bar).
    fn render_diff_bar(&self, palette: Palette, cx: &App) -> Div {
        let error = &self.task.error;
        let human = error.diff.human.parts().1;
        let machine = error.diff.machine.parts().1;
        let separator = || div().w_px().h_4().bg(palette.border);
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_x_3()
            .gap_y_1()
            .px_4()
            .py_2()
            .text_xs()
            .text_color(palette.muted)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.fg)
                    .child(localized(cx, "Human vs Machine", "Человек и машина")),
            )
            .child(separator())
            .child(
                h_flex()
                    .gap_1p5()
                    .child(localized(cx, "Diff", "Дифф"))
                    .child(
                        h_flex()
                            .gap_1()
                            .px_1p5()
                            .rounded_sm()
                            .border_1()
                            .border_color(palette.border)
                            .bg(palette.surface)
                            .child(div().text_color(palette.human).child(quoted(&human)))
                            .child("→")
                            .child(div().text_color(palette.machine).child(quoted(&machine))),
                    ),
            )
            .when_some(self.task.focus.as_ref(), |this, focus| {
                this.child(separator()).child(format!(
                    "{} – {}",
                    format_time(focus.start_seconds),
                    format_time(focus.end_seconds)
                ))
            })
            .when_some(error.diff_type_label(), |this, label| {
                this.child(separator()).child(
                    h_flex()
                        .gap_1p5()
                        .child(localized(cx, "Diff type", "Тип диффа"))
                        .child(
                            div()
                                .px_2()
                                .rounded_full()
                                .border_1()
                                .border_color(palette.border)
                                .text_color(palette.fg)
                                .child(label),
                        ),
                )
            })
            .when(error.source == ErrorSource::Synthetic, |this| {
                this.child(separator()).child(
                    div()
                        .px_2()
                        .rounded_full()
                        .border_1()
                        .border_color(palette.info.opacity(0.4))
                        .bg(palette.info.opacity(0.08))
                        .text_color(palette.info)
                        .child(localized(
                            cx,
                            "Synthetic · text only",
                            "Синтетический · только текст",
                        )),
                )
            })
    }

    fn render_comparison(&self, palette: Palette, cx: &App) -> Div {
        let diff = self.displayed_diff();
        v_flex()
            .w_full()
            .gap_1()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(palette.border)
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(tone_label(localized(cx, "Human", "Человек"), palette.human))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .child(passage(&diff.human, palette.human)),
                    ),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(tone_label(
                        localized(cx, "Machine", "Машина"),
                        palette.machine,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .child(passage(&diff.machine, palette.machine)),
                    ),
            )
            .child(div().text_xs().text_color(palette.muted).child(localized(
                cx,
                "Normalized text",
                "Нормализованный текст",
            )))
    }

    fn render_audio(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Div> {
        let focus = self.task.focus.as_ref()?;
        let playing = self.engine.state() != PlaybackState::Stopped;
        let ready = matches!(self.audio, AudioLoad::Ready(_));
        let segment = focus.interval();
        let alignment = self
            .assistant
            .alignment
            .as_ref()
            .map(|aligned| aligned.alignment.clone());
        let state: SharedString = match &self.audio {
            AudioLoad::Loading => localized(cx, "Loading the audio…", "Загрузка аудио…").into(),
            AudioLoad::Missing => localized(
                cx,
                "This task has no audio for the diff's speaker.",
                "В задаче нет аудио для спикера этого диффа.",
            )
            .into(),
            AudioLoad::Failed(error) => failed(
                cx,
                "Couldn't load the audio.",
                "Не удалось загрузить аудио.",
                error,
            )
            .into(),
            _ => focus.speaker_label().into(),
        };
        Some(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap_2()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(palette.border)
                .child(div().text_xs().text_color(palette.muted).child(state))
                .child(div().flex_1())
                .when(playing, |this| {
                    this.child(
                        Button::new("error-task-pause")
                            .small()
                            .icon(IconName::Pause)
                            .label(localized(cx, "Pause", "Пауза"))
                            .on_click(cx.listener(|this, _, _, cx| this.pause(cx))),
                    )
                })
                .when(!playing, |this| {
                    this.child(
                        Button::new("error-task-play-segment")
                            .small()
                            .outline()
                            .icon(IconName::Play)
                            .label(localized(cx, "Play segment", "Воспроизвести сегмент"))
                            .disabled(!ready)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.play(segment.start_seconds, segment.end_seconds, cx)
                            })),
                    )
                    .when_some(alignment, |this, alignment| {
                        this.child(
                            Button::new("error-task-play-context")
                                .small()
                                .outline()
                                .icon(IconName::Play)
                                .label(localized(cx, "Play context", "Воспроизвести контекст"))
                                .disabled(!ready)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.play(
                                        alignment.context_start_seconds,
                                        alignment.context_end_seconds,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new("error-task-play-diff")
                                .small()
                                .outline()
                                .icon(IconName::Play)
                                .label(localized(cx, "Play diff", "Воспроизвести дифф"))
                                .disabled(!ready)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.play(
                                        alignment.diff_start_seconds,
                                        alignment.diff_end_seconds,
                                        cx,
                                    )
                                })),
                        )
                    })
                }),
        )
    }

    fn render_assistant(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Div> {
        if self.source() != ErrorSource::Real || self.task.focus.is_none() {
            return None;
        }
        let status = self.assistant.status.as_ref();
        let phase = status.map(|status| status.phase);
        let drafting = self.assistant.run.as_ref().is_some_and(|run| run.drafting);
        let busy = self.assistant.run.is_some()
            || phase == Some(AssistantPhase::WaitingAudio)
            || self.assistant.fill_after_alignment;
        let color = match phase {
            Some(AssistantPhase::Error) => palette.danger,
            Some(AssistantPhase::Aligned | AssistantPhase::Filled) => palette.success,
            Some(AssistantPhase::Cancelled) => palette.warning,
            _ => palette.muted,
        };
        let receipt = status
            .and_then(|status| status.receipt.as_ref())
            .map(|receipt| {
                let cost = receipt
                    .cost
                    .map(|cost| format!(" · ${cost:.6}"))
                    .unwrap_or_default();
                format!("{}{cost}", receipt.model)
            });
        Some(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap_2()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(palette.border)
                .child(
                    Button::new("error-task-initial-fill")
                        .small()
                        .primary()
                        .icon(IconName::Bot)
                        .label(localized(cx, "Fill in", "Заполнить"))
                        .tooltip(
                            if !Self::gold_inference_available(cx)
                                && self.assistant.alignment.is_none()
                            {
                                localized(
                                    cx,
                                    "Turn on the AI extension in Extensions to align audio first.",
                                    "Сначала включите расширение ИИ в разделе «Расширения», чтобы выровнять аудио.",
                                )
                            } else {
                                localized(
                                    cx,
                                    "Fills the unanswered choices for you to review.",
                                    "Заполняет неотвеченные вопросы — проверьте перед отправкой.",
                                )
                            },
                        )
                        .loading(busy)
                        .disabled(
                            self.read_only
                                || (!Self::gold_inference_available(cx)
                                    && self.assistant.alignment.is_none())
                                || drafting
                                || self.submitting.is_some()
                                || self.submitted
                                || phase == Some(AssistantPhase::WaitingAudio),
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.initial_fill(window, cx))),
                )
                .when(self.assistant.run.is_some(), |this| {
                    this.child(
                        Button::new("error-task-cancel-assistant")
                            .small()
                            .ghost()
                            .label(localized(cx, "Cancel", "Отмена"))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_assistant(cx))),
                    )
                })
                .when_some(status, |this, status| {
                    this.child(
                        div()
                            .flex_1()
                            .min_w(rems(12.))
                            .text_xs()
                            .text_color(color)
                            .child(status.message.clone()),
                    )
                })
                .when_some(receipt, |this, receipt| {
                    this.child(div().text_xs().text_color(palette.muted).child(receipt))
                }),
        )
    }

    fn render_label_table(&self, rows: Vec<LabelRow>, palette: Palette, cx: &App) -> Option<Div> {
        if rows.is_empty() {
            return None;
        }
        let has_audio = self.source() == ErrorSource::Real;
        let language = self.language().to_string();
        let cell = |text: SharedString| div().flex_1().min_w(rems(4.)).px_2().py_1().child(text);
        let mut header = h_flex()
            .w_full()
            .text_xs()
            .text_color(palette.muted)
            .bg(palette.group)
            .child(
                div()
                    .w(rems(3.5))
                    .px_2()
                    .py_1()
                    .child(localized(cx, "Label", "Разметка")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(rems(8.))
                    .px_2()
                    .py_1()
                    .child(localized(cx, "Categories", "Категории")),
            )
            .child(cell(
                localized(cx, "Changed meaning", "Меняет смысл").into(),
            ));
        if has_audio {
            header = header.child(cell(localized(cx, "Human correct", "Человек прав").into()));
        }
        header = header
            .child(cell(localized(cx, "Whole error", "Вся ошибка").into()))
            .child(cell(
                localized(cx, "Other issues", "Другие проблемы").into(),
            ))
            .child(cell(localized(cx, "Notes", "Заметки").into()));
        let body = rows.into_iter().map(|row| {
            let categories = grouped_label_categories(&row.label.category_keys, &language)
                .into_iter()
                .map(|(bucket, names)| format!("{bucket} › {}", names.join(", ")))
                .collect::<Vec<_>>()
                .join("; ");
            let mut line = h_flex()
                .w_full()
                .text_xs()
                .border_t_1()
                .border_color(palette.border)
                .child(
                    div()
                        .w(rems(3.5))
                        .px_2()
                        .py_1()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(row.name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(rems(8.))
                        .px_2()
                        .py_1()
                        .child(categories),
                )
                .child(cell(yes_no(row.label.meaning_changed, cx).into()));
            if has_audio {
                line = line.child(cell(
                    yes_no(row.label.human_transcript_wrong.map(|wrong| !wrong), cx).into(),
                ));
            }
            line.child(cell(yes_no(row.label.captures_whole_error, cx).into()))
                .child(cell(
                    row.label
                        .other_issues
                        .clone()
                        .unwrap_or_else(|| "-".into())
                        .into(),
                ))
                .child(cell(
                    row.label.notes.clone().unwrap_or_else(|| "-".into()).into(),
                ))
        });
        Some(
            v_flex()
                .w_full()
                .rounded(palette.radius)
                .border_1()
                .border_color(palette.border)
                .child(header)
                .children(body),
        )
    }

    fn yes_no_group(
        &self,
        id: &'static str,
        value: Option<bool>,
        cx: &mut Context<Self>,
        set: impl Fn(&mut ErrorChoices, bool) + 'static,
    ) -> ButtonGroup {
        ButtonGroup::new(id)
            .outline()
            .small()
            .disabled(self.read_only || self.submitting.is_some() || self.submitted)
            .child(
                Button::new("yes")
                    .label(localized(cx, "Yes", "Да"))
                    .selected(value == Some(true)),
            )
            .child(
                Button::new("no")
                    .label(localized(cx, "No", "Нет"))
                    .selected(value == Some(false)),
            )
            .on_click(cx.listener(move |this, clicked: &Vec<usize>, _, cx| {
                if let Some(&index) = clicked.first() {
                    this.set_choices(|choices| set(choices, index == 0), cx);
                }
            }))
    }

    fn question(
        &self,
        label: &'static str,
        help: Option<&'static str>,
        group: ButtonGroup,
        palette: Palette,
    ) -> Div {
        v_flex()
            .gap_1()
            .flex_1()
            .min_w(rems(16.))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_0p5()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label)
                            .child(div().text_color(palette.danger).child("*")),
                    )
                    .child(group),
            )
            .when_some(help, |this, help| {
                this.child(div().text_xs().text_color(palette.muted).child(help))
            })
    }

    fn render_categories(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let language = self.language().to_string();
        let has_audio = self.source() == ErrorSource::Real;
        let selected = self.choices.category_keys.clone();
        let locked = self.read_only || self.submitting.is_some() || self.submitted;
        let query = self.filter.read(cx).value().to_string();
        let groups = filtered_groups(&query, has_audio, &language);
        let picks = (!self.task.error.prior_labels.is_empty())
            .then(|| picks_by_key(&self.task.error.prior_labels));
        let selected_groups = {
            let mut indices: Vec<usize> =
                selected.iter().filter_map(|key| group_index(key)).collect();
            indices.sort_unstable();
            indices.dedup();
            indices.len()
        };
        let agree = level_one_categories(&self.task.error.prior_labels);
        let chips = selected.iter().enumerate().map(|(index, key)| {
            let remove = key.clone();
            h_flex()
                .gap_1()
                .pl_2()
                .rounded_full()
                .border_1()
                .border_color(palette.warning)
                .bg(palette.warning.opacity(0.08))
                .text_xs()
                .child(category_name(key, &language))
                .child(
                    Button::new(("remove-category", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .disabled(locked)
                        .tooltip(localized_fmt(
                            cx,
                            "Remove {name}",
                            "Убрать {name}",
                            &[("name", &category_name(key, &language))],
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let key = remove.clone();
                            this.set_choices(|choices| choices.toggle_category(&key), cx);
                        })),
                )
        });
        let list = groups.into_iter().map(|(group, keys)| {
            let picked_here = keys
                .iter()
                .filter(|key| selected.iter().any(|s| s == *key))
                .count();
            let group_picks: Vec<String> = picks
                .as_ref()
                .map(|picks| {
                    let mut levels: Vec<String> = group
                        .categories
                        .iter()
                        .filter_map(|key| picks.get(*key))
                        .flatten()
                        .cloned()
                        .collect();
                    levels.sort();
                    levels.dedup();
                    levels
                })
                .unwrap_or_default();
            let buttons = keys.into_iter().map(|key| {
                let is_selected = selected.iter().any(|s| s == key);
                let blocked = self.choices.category_blocked(key);
                let key_picks = picks
                    .as_ref()
                    .and_then(|picks| picks.get(key))
                    .map(|levels| levels.join(" + "));
                let toggle = key.to_string();
                Button::new(SharedString::from(format!("category-{key}")))
                    .ghost()
                    .small()
                    .selected(is_selected)
                    .disabled(locked || blocked)
                    .icon(if is_selected {
                        IconName::Check
                    } else {
                        IconName::Plus
                    })
                    .label(match key_picks {
                        Some(levels) => format!("{}  ({levels})", category_name(key, &language)),
                        None => category_name(key, &language),
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let key = toggle.clone();
                        this.set_choices(|choices| choices.toggle_category(&key), cx);
                        this.filter
                            .update(cx, |filter, cx| filter.set_value("", window, cx));
                    }))
            });
            v_flex()
                .w_full()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .px_2p5()
                        .py_1()
                        .bg(palette.group)
                        .text_xs()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(group_name(group, &language).to_uppercase()),
                        )
                        .child(div().flex_1())
                        .when(!group_picks.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_color(palette.warning)
                                    .child(group_picks.join(" + ")),
                            )
                        })
                        .when(picked_here > 0, |this| {
                            this.child(
                                div()
                                    .px_1p5()
                                    .rounded_full()
                                    .bg(palette.primary)
                                    .text_color(palette.surface)
                                    .child(picked_here.to_string()),
                            )
                        }),
                )
                .child(h_flex().flex_wrap().gap_1().px_1().py_1().children(buttons))
        });
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_0p5()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(localized(cx, "Categories", "Категории"))
                    .child(div().text_color(palette.danger).child("*")),
            )
            .child(div().text_xs().text_color(palette.muted).child(localized_fmt(
                cx,
                "Pick the first category that applies, top to bottom. Add a second only if one further down also applies, never from {a} or {b}, except {c}. Most errors need just one.",
                "Выберите первую подходящую категорию сверху вниз. Добавьте вторую, только если подходит и одна из нижних, но не из {a} или {b}, кроме {c}. Большинству ошибок хватает одной.",
                &[
                    ("a", &group_name(&CATEGORY_GROUPS[4], &language)),
                    ("b", &group_name(&CATEGORY_GROUPS[5], &language)),
                    ("c", &category_name("affix", &language)),
                ],
            )))
            .when_some(agree, |this, keys| {
                this.child(
                    Button::new("agree-with-l1")
                        .outline()
                        .small()
                        .label(localized(cx, "Agree with L1’s selection", "Согласиться с выбором L1"))
                        .disabled(locked)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let keys = keys.clone();
                            this.set_choices(|choices| choices.category_keys = keys, cx);
                        })),
                )
            })
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1p5()
                    .min_h(rems(1.75))
                    .when(selected.is_empty(), |this| {
                        this.child(div().text_xs().text_color(palette.muted).child(localized(
                            cx,
                            "No categories selected",
                            "Категории не выбраны",
                        )))
                    })
                    .children(chips),
            )
            .when(selected.len() == 2, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(palette.warning)
                        .child(localized(
                            cx,
                            "Two is the maximum. Keep the second only if it also applies.",
                            "Максимум две. Оставьте вторую, только если она тоже подходит.",
                        )),
                )
            })
            .child(
                v_flex()
                    .w_full()
                    .rounded(palette.radius)
                    .border_1()
                    .border_color(palette.border)
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .px_2p5()
                            .py_1p5()
                            .child(div().flex_1().child(Input::new(&self.filter).small()))
                            .when(!selected.is_empty(), |this| {
                                this.child(div().text_xs().text_color(palette.muted).child(format!(
                                    "{} · {}",
                                    count(
                                        selected.len() as u64,
                                        ("category", "categories"),
                                        ("категория", "категории", "категорий"),
                                    ),
                                    count(
                                        selected_groups as u64,
                                        ("group", "groups"),
                                        ("группа", "группы", "групп"),
                                    ),
                                )))
                            }),
                    )
                    .child(
                        div()
                            .id("error-task-categories")
                            .w_full()
                            .max_h(px(340.))
                            .overflow_y_scroll()
                            .child(v_flex().w_full().children(list).when(
                                filtered_groups(&query, has_audio, &language).is_empty(),
                                |this| {
                                    this.child(
                                        div()
                                            .p_2p5()
                                            .text_xs()
                                            .text_color(palette.muted)
                                            .child(localized(cx, "No category matches.", "Ничего не найдено.")),
                                    )
                                },
                            )),
                    ),
            )
    }

    fn render_questionnaire(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let source = self.source();
        let ready = readiness(&self.choices, source);
        let submittable = ready.is_ok();
        let busy = self.submitting.is_some() || self.submitted;
        let discard = self.choices.captures_whole_error == Some(false);
        let action = if discard {
            localized(cx, "Discard this diff", "Отбросить дифф")
        } else {
            localized(cx, "Submit the label", "Отправить разметку")
        };
        let status_text: SharedString = match &ready {
            Ok(()) => localized(cx, "Ready", "Готово").into(),
            Err(message) => message.clone().into(),
        };
        let qa = self.task.error.is_qa_review;
        let whole = self.yes_no_group(
            "whole-error",
            self.choices.captures_whole_error,
            cx,
            |choices, yes| choices.captures_whole_error = Some(yes),
        );
        let mut section = v_flex()
            .w_full()
            .gap_2()
            .px_3()
            .py_2p5()
            .rounded(palette.radius)
            .border_1()
            .border_color(palette.border)
            .bg(palette.surface)
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        h_flex()
                            .flex_1()
                            .flex_wrap()
                            .gap_2p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if qa {
                                        localized(cx, "QA review", "Проверка QA")
                                    } else {
                                        localized(cx, "Label this diff", "Разметьте дифф")
                                    }),
                            )
                            .child(div().text_xs().text_color(palette.muted).child(if qa {
                                localized(
                                    cx,
                                    "L1 and L2 labeled this diff. Your label is saved next to theirs.",
                                    "L1 и L2 разметили этот дифф. Ваша разметка сохраняется рядом с ними.",
                                )
                            } else {
                                localized(
                                    cx,
                                    "Answer the first question. If Yes, pick the category that applies.",
                                    "Ответьте на первый вопрос. Если Да — выберите подходящую категорию.",
                                )
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if submittable { palette.success } else { palette.muted })
                            .child(status_text.clone()),
                    )
                    .when(self.next_project.is_some(), |this| {
                        this.child(
                            Button::new("error-task-submit-exit")
                                .outline()
                                .small()
                                .label(localized(cx, "Submit and exit", "Отправить и выйти"))
                                .tooltip(if submittable {
                                    SharedString::from(localized_fmt(
                                        cx,
                                        "{action} and go back to projects",
                                        "{action} и вернуться к проектам",
                                        &[("action", action)],
                                    ))
                                } else {
                                    status_text.clone()
                                })
                                .loading(self.submitting == Some(AfterSubmit::Exit))
                                .disabled(!submittable || busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.submit(Some(AfterSubmit::Exit), cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("error-task-submit")
                            .primary()
                            .small()
                            .label(if self.next_project.is_some() {
                                localized(cx, "Next task", "Следующая задача")
                            } else {
                                localized(cx, "Submit", "Отправить")
                            })
                            .tooltip(if submittable {
                                SharedString::from(if self.next_project.is_some() {
                                    localized_fmt(
                                        cx,
                                        "{action} and open the next task (Ctrl+Enter)",
                                        "{action} и открыть следующую задачу (Ctrl+Enter)",
                                        &[("action", action)],
                                    )
                                } else {
                                    format!("{action} (Ctrl+Enter)")
                                })
                            } else {
                                status_text
                            })
                            .loading(self.submitting.is_some_and(|after| {
                                after == AfterSubmit::NextTask || self.next_project.is_none()
                            }))
                            .disabled(!submittable || busy)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(None, cx))),
                    ),
            )
            .when_some(self.submit_error.clone(), |this, error| {
                this.child(ui::banner(
                    Tone::Danger,
                    localized(cx, "Couldn't submit the label.", "Не удалось отправить разметку."),
                    Some(error).filter(|error| !error.trim().is_empty()),
                    None,
                    cx,
                ))
            })
            .children(self.render_label_table(
                label_rows(&self.task.error.prior_labels, None, qa),
                palette,
                cx,
            ))
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap_x_3()
                    .gap_y_1p5()
                    .px_2p5()
                    .py_2()
                    .rounded(palette.radius)
                    .bg(palette.group)
                    .child(
                        h_flex()
                            .gap_0p5()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(localized(cx, WHOLE_QUESTION.0, WHOLE_QUESTION.1))
                            .child(div().text_color(palette.danger).child("*")),
                    )
                    .child(whole)
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(localized(
                                cx,
                                "No discards this diff; then just submit.",
                                "Нет — дифф отбрасывается; останется только отправить.",
                            )),
                    ),
            );
        if discard {
            section = section.child(
                div()
                    .px_2p5()
                    .py_2()
                    .rounded(palette.radius)
                    .border_1()
                    .border_color(palette.warning.opacity(0.4))
                    .bg(palette.warning.opacity(0.08))
                    .text_xs()
                    .text_color(palette.warning)
                    .child(localized(
                        cx,
                        "This diff will be discarded. Submit to continue.",
                        "Этот дифф будет отброшен. Отправьте, чтобы продолжить.",
                    )),
            );
        }
        if self.choices.captures_whole_error == Some(true) {
            let meaning = self.yes_no_group(
                "meaning-changed",
                self.choices.meaning_changed,
                cx,
                |choices, yes| choices.meaning_changed = Some(yes),
            );
            let human = (source == ErrorSource::Real).then(|| {
                // Babel asks whether the human transcript is CORRECT; the label stores "wrong".
                self.yes_no_group(
                    "human-correct",
                    self.choices.human_transcript_wrong.map(|wrong| !wrong),
                    cx,
                    |choices, correct| choices.human_transcript_wrong = Some(!correct),
                )
            });
            let locked = self.read_only || busy;
            section = section
                .child(self.render_categories(palette, cx))
                .child(div().h_px().w_full().bg(palette.border))
                .child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .gap_4()
                        .child(self.question(
                            localized(cx, MEANING_QUESTION.0, MEANING_QUESTION.1),
                            Some(localized(cx, MEANING_HELP.0, MEANING_HELP.1)),
                            meaning,
                            palette,
                        ))
                        .when_some(human, |this, human| {
                            this.child(self.question(
                                localized(cx, HUMAN_QUESTION.0, HUMAN_QUESTION.1),
                                Some(localized(cx, HUMAN_HELP.0, HUMAN_HELP.1)),
                                human,
                                palette,
                            ))
                        }),
                )
                .child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .gap_4()
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w(rems(16.))
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(localized(cx, "Other issues", "Другие проблемы")),
                                )
                                .child(div().flex_1().child(
                                    Input::new(&self.other_issues).small().disabled(locked),
                                )),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w(rems(16.))
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(localized(cx, "Notes", "Заметки")),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .child(Input::new(&self.notes).small().disabled(locked)),
                                ),
                        ),
                );
        }
        section
    }

    fn render_submitted(&self, palette: Palette, cx: &App) -> Option<Div> {
        let error = &self.task.error;
        let table = self.render_label_table(
            label_rows(
                &error.prior_labels,
                error.submitted_label.as_ref(),
                error.is_qa_review,
            ),
            palette,
            cx,
        )?;
        Some(
            v_flex()
                .w_full()
                .gap_2()
                .px_3()
                .py_2p5()
                .rounded(palette.radius)
                .border_1()
                .border_color(palette.border)
                .bg(palette.surface)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(localized(cx, "Submitted labels", "Отправленные разметки")),
                )
                .child(table),
        )
    }
}

impl Render for ErrorTaskScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !Self::gold_inference_available(cx)
            && self.assistant.run.as_ref().is_some_and(|run| !run.drafting)
        {
            self.cancel_assistant(cx);
        }
        self.schedule_frame(window, cx);
        let palette = Palette::new(cx);
        let header = self.render_header(palette, cx);
        let diff_bar = self.render_diff_bar(palette, cx);
        let comparison = self.render_comparison(palette, cx);
        let audio = self.render_audio(palette, cx);
        let assistant = self.render_assistant(palette, cx);
        let bottom = if self.read_only {
            self.render_submitted(palette, cx)
        } else {
            Some(self.render_questionnaire(palette, cx))
        };
        div()
            .id("error-task-screen")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .bg(palette.surface)
            .text_color(palette.fg)
            .child(
                div().size_full().overflow_y_scrollbar().child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .px_6()
                        .py_4()
                        .child(header)
                        .child(
                            v_flex()
                                .w_full()
                                .rounded(palette.radius)
                                .border_1()
                                .border_color(palette.border)
                                .child(diff_bar)
                                .child(comparison)
                                .children(audio)
                                .children(assistant),
                        )
                        .children(bottom),
                ),
            )
    }
}
