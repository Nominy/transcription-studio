//! Speech enhancement (denoise) for the open task: the toolbar's Generate button, the
//! Enhanced ↔ Original mix slider and the status popover.
//!
//! Enhancement runs through [`InferenceBackend::enhance`] on the backend chosen by the
//! denoise location setting (This PC or Remote GPU). Results are cached on disk per task,
//! enhancer model and original source digest; when the task's audio loads a cached result
//! is installed without inference, and with "denoise automatically" a miss generates one.
//! The mix ratio changes live in the audio engine, and lane waveforms follow it (they paint
//! the blend each lane plays); playback, waveforms and every timing keep the originals' clock.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_audio::{DecodedAudio, EnhancedAudioCache, decode_audio_bytes, sha256_hex};
use studio_core::AudioLane;
use studio_services::inference::{
    EnhancedLane, GOLD_ENHANCEMENT_MODEL, InferenceBackend, InferenceProgress, LaneAudio,
    ProgressSink,
};
use studio_services::paths::app_support_dir;
use studio_services::prefs::ExecutionLocation;

use crate::editor::inference_audio::{LaneSource, read_lane_audio, until_cancelled};
use crate::editor::model::EditorModel;
use crate::editor::panels::ui::{
    dialog_footer, dialog_header, error_text, muted, stable_text, tabular_figures,
};
use crate::services::AppServices;

/// Slider detent, in percent.
const MIX_STEP_PERCENT: u8 = 10;
/// Enhanced share of a new task's mix (Helper: 90% Enhanced / 10% Original).
const DEFAULT_ENHANCED_PERCENT: u8 = 90;
/// Lanes the enhancement contract takes (Gold enhances both speaker tracks together).
const ENHANCED_LANE_COUNT: usize = 2;
const ELAPSED_REFRESH: Duration = Duration::from_secs(1);
/// Folder under the app support dir holding [`EnhancedAudioCache`].
const CACHE_DIR_NAME: &str = "enhanced-audio";

/// Enhanced share of the mix, a multiple of [`MIX_STEP_PERCENT`] in `0..=100`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EnhancedPercent(u8);

impl EnhancedPercent {
    /// The slider runs Enhanced (left, 0) → Original (right, 100): its value is the original
    /// share, snapped to the detents.
    fn from_slider(original_percent: f32) -> Self {
        let steps = 100 / MIX_STEP_PERCENT;
        let detent = (original_percent / f32::from(MIX_STEP_PERCENT))
            .round()
            .clamp(0.0, f32::from(steps)) as u8;
        Self(100 - detent * MIX_STEP_PERCENT)
    }

    fn slider_value(self) -> f32 {
        f32::from(100 - self.0)
    }

    fn ratio(self) -> f32 {
        f32::from(self.0) / 100.0
    }

    fn is_full(self) -> bool {
        self.0 == 100
    }

    fn label(self) -> String {
        match self.0 {
            0 => "Original".to_string(),
            percent => format!("{percent}% enhanced"),
        }
    }

    fn short_label(self) -> String {
        match self.0 {
            0 => "Original".to_string(),
            percent => format!("{percent}% enh."),
        }
    }
}

/// Tasks whose user confirmed 100% enhanced audio in this app session; later transitions to
/// 100% on such a task do not ask again.
#[derive(Default)]
struct FullEnhancementApprovals(HashSet<String>);

impl Global for FullEnhancementApprovals {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunMode {
    /// Install a cached result if there is one; never run inference.
    CacheOnly,
    /// Use the cache, else run the backend.
    Generate,
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Reading the original audio and checking the cache.
    Preparing,
    Queued {
        position: u32,
        count: u32,
    },
    Stage {
        stage: String,
        completed: u64,
        total: Option<u64>,
        detail: Option<String>,
    },
    Download {
        bytes: u64,
        total: u64,
    },
    /// Decoding the result and switching playback to it.
    Installing,
}

#[derive(Clone, Debug, PartialEq)]
enum ReadySource {
    Cache,
    Generated {
        location: ExecutionLocation,
        took: Duration,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum RunState {
    Idle,
    Running {
        mode: RunMode,
        location: ExecutionLocation,
        started: Instant,
        phase: Phase,
    },
    Ready {
        source: ReadySource,
        /// The cache could not be written; the installed audio is unaffected.
        cache_warning: Option<String>,
    },
    Cancelled,
    Failed(String),
}

enum Outcome {
    Ready {
        source: ReadySource,
        cache_warning: Option<String>,
    },
    /// Cache-only lookup missed.
    NothingCached,
    Failed(String),
}

/// The open task's lanes as the enhancement job needs them.
struct JobInput {
    task_id: String,
    sources: Vec<LaneSource>,
    location: ExecutionLocation,
    backend: Arc<dyn InferenceBackend>,
    cache: EnhancedAudioCache,
}

/// Original lane bytes plus their cache keys.
struct PreparedLanes {
    lanes: Vec<LaneAudio>,
    digests: Vec<String>,
    cached: Option<Vec<(String, Arc<DecodedAudio>)>>,
}

/// A backend result decoded for the engine, with the bytes to cache.
struct DecodedResult {
    model_id: String,
    lanes: Vec<(String, Arc<DecodedAudio>)>,
    wavs: Vec<bytes::Bytes>,
}

pub struct DenoiseControl {
    model: Entity<EditorModel>,
    slider: Entity<SliderState>,
    /// Audible selection.
    selected: EnhancedPercent,
    /// Slider position while dragging (may be an unconfirmed 100%).
    preview: EnhancedPercent,
    run: RunState,
    job: Option<Task<()>>,
    ticker: Option<Task<()>>,
    /// Set once the task's audio finished loading and the startup cache check ran.
    startup_checked: bool,
    /// Hides the slider in narrow toolbars (set by the toolbar each render).
    narrow: bool,
    _subscriptions: Vec<Subscription>,
}

impl DenoiseControl {
    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let selected = EnhancedPercent(DEFAULT_ENHANCED_PERCENT);
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(f32::from(MIX_STEP_PERCENT))
                .default_value(selected.slider_value())
        });
        model.update(cx, |model, cx| {
            model.set_enhanced_ratio(selected.ratio(), cx)
        });
        let subscriptions = vec![
            cx.observe(&model, |this, _, cx| this.check_startup(cx)),
            cx.subscribe_in(
                &slider,
                window,
                |this, _, event: &SliderEvent, window, cx| match event {
                    SliderEvent::Change(value) => {
                        this.preview_mix(EnhancedPercent::from_slider(value.end()), cx);
                    }
                    SliderEvent::Release(value) => {
                        this.commit_mix(EnhancedPercent::from_slider(value.end()), window, cx);
                    }
                },
            ),
        ];
        let mut this = Self {
            model,
            slider,
            selected,
            preview: selected,
            run: RunState::Idle,
            job: None,
            ticker: None,
            startup_checked: false,
            narrow: false,
            _subscriptions: subscriptions,
        };
        this.check_startup(cx);
        this
    }

    pub fn set_narrow(&mut self, narrow: bool) {
        self.narrow = narrow;
    }

    fn task_id(&self, cx: &App) -> String {
        self.model.read(cx).doc().project().id.clone()
    }

    // ---- mix ---------------------------------------------------------------

    fn full_approved(&self, cx: &App) -> bool {
        cx.try_global::<FullEnhancementApprovals>()
            .is_some_and(|approvals| approvals.0.contains(&self.task_id(cx)))
    }

    /// Dragging previews every detent live except an unconfirmed 100%, which keeps the
    /// current audio until the release asks for confirmation.
    fn preview_mix(&mut self, percent: EnhancedPercent, cx: &mut Context<Self>) {
        self.preview = percent;
        if !percent.is_full() || self.full_approved(cx) {
            self.apply_mix(percent, cx);
        }
        cx.notify();
    }

    fn commit_mix(
        &mut self,
        percent: EnhancedPercent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preview = percent;
        if percent.is_full() && !self.full_approved(cx) {
            self.confirm_full_enhancement(window, cx);
        } else {
            self.apply_mix(percent, cx);
        }
        cx.notify();
    }

    fn apply_mix(&mut self, percent: EnhancedPercent, cx: &mut Context<Self>) {
        self.selected = percent;
        self.model.update(cx, |model, cx| {
            model.set_enhanced_ratio(percent.ratio(), cx)
        });
    }

    /// Restores the slider to the audible selection (cancelled 100% confirmation).
    fn revert_mix(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preview = self.selected;
        let value = self.selected.slider_value();
        self.slider
            .update(cx, |slider, cx| slider.set_value(value, window, cx));
        cx.notify();
    }

    fn approve_full_enhancement(&mut self, cx: &mut Context<Self>) {
        let task_id = self.task_id(cx);
        cx.default_global::<FullEnhancementApprovals>()
            .0
            .insert(task_id);
        self.preview = EnhancedPercent(100);
        self.apply_mix(EnhancedPercent(100), cx);
        cx.notify();
    }

    fn confirm_full_enhancement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let control = cx.entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            let cancel = control.clone();
            let cancel_button = control.clone();
            let confirm = control.clone();
            dialog
                .w(rems(28.).to_pixels(window.rem_size()))
                .close_button(false)
                .on_cancel(move |_, window, cx| {
                    cancel.update(cx, |this, cx| this.revert_mix(window, cx));
                    true
                })
                .child(
                    v_flex()
                        .gap_3()
                        .child(dialog_header(
                            "Use 100% enhanced audio?",
                            Some(
                                "Full strength can suppress or remove speech. 90% keeps more of the voice."
                                    .into(),
                            ),
                            cx,
                        ))
                        .child(
                            dialog_footer()
                                .child(Button::new("denoise-full-cancel").label("Cancel").on_click(
                                    move |_, window, cx| {
                                        cancel_button
                                            .update(cx, |this, cx| this.revert_mix(window, cx));
                                        window.close_dialog(cx);
                                    },
                                ))
                                .child(
                                    Button::new("denoise-full-confirm")
                                        .warning()
                                        .label("Use 100% enhanced")
                                        .on_click(move |_, window, cx| {
                                            confirm.update(cx, |this, cx| {
                                                this.approve_full_enhancement(cx);
                                            });
                                            window.close_dialog(cx);
                                        }),
                                ),
                        ),
                )
        });
    }

    // ---- generation --------------------------------------------------------

    /// Once every lane finished loading: install a cached result, or generate one when
    /// "denoise automatically" is on.
    fn check_startup(&mut self, cx: &mut Context<Self>) {
        if self.startup_checked {
            return;
        }
        let load = self.model.read(cx).audio_load();
        if load.pending > 0 || load.loaded == 0 {
            return;
        }
        self.startup_checked = true;
        let mode = if cx.global::<AppServices>().ai.auto_denoise {
            RunMode::Generate
        } else {
            RunMode::CacheOnly
        };
        self.start(mode, cx);
    }

    fn is_generating(&self) -> bool {
        matches!(
            self.run,
            RunState::Running {
                mode: RunMode::Generate,
                ..
            }
        )
    }

    /// Starts generation; supersedes a startup cache check still in flight.
    fn generate(&mut self, cx: &mut Context<Self>) {
        if !self.is_generating() {
            self.start(RunMode::Generate, cx);
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.is_generating() {
            // Dropping the job drops its cancel sender, which drops the backend future.
            self.job = None;
            self.ticker = None;
            self.run = RunState::Cancelled;
            cx.notify();
        }
    }

    fn start(&mut self, mode: RunMode, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let load = model.audio_load();
        let lane_count = model.doc().project().lanes.len();
        let sources = model.inference_lane_sources();
        let unusable = if load.pending > 0 {
            Some("Speaker audio is still loading.".to_string())
        } else if lane_count != ENHANCED_LANE_COUNT || sources.len() != ENHANCED_LANE_COUNT {
            Some(format!(
                "Denoise needs both speaker tracks loaded ({} of {lane_count} loaded; {ENHANCED_LANE_COUNT} required).",
                sources.len()
            ))
        } else {
            None
        };
        if let Some(message) = unusable {
            if mode == RunMode::Generate {
                self.run = RunState::Failed(message);
                cx.notify();
            }
            return;
        }
        let services = cx.global::<AppServices>();
        let location = services.ai.denoise_location;
        let input = JobInput {
            task_id: model.doc().project().id.clone(),
            sources,
            location,
            backend: services.inference.backend(location),
            cache: EnhancedAudioCache::new(app_support_dir().join(CACHE_DIR_NAME)),
        };
        self.run = RunState::Running {
            mode,
            location,
            started: Instant::now(),
            phase: Phase::Preparing,
        };
        self.job = Some(cx.spawn(async move |this, cx| {
            let outcome = run_job(this.clone(), mode, input, cx).await;
            this.update(cx, |this, cx| this.finish(outcome, cx)).ok();
        }));
        self.ticker = (mode == RunMode::Generate).then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(ELAPSED_REFRESH).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
        });
        cx.notify();
    }

    fn set_phase(&mut self, next: Phase, cx: &mut Context<Self>) {
        if let RunState::Running { phase, .. } = &mut self.run {
            *phase = next;
            cx.notify();
        }
    }

    fn on_progress(&mut self, progress: InferenceProgress, cx: &mut Context<Self>) {
        self.set_phase(
            match progress {
                InferenceProgress::Queued { position, count } => Phase::Queued { position, count },
                InferenceProgress::Stage {
                    stage,
                    completed,
                    total,
                    detail,
                    ..
                } => Phase::Stage {
                    stage,
                    completed,
                    total,
                    detail,
                },
                InferenceProgress::Download { bytes, total } => Phase::Download { bytes, total },
            },
            cx,
        );
    }

    /// Installs enhanced PCM with the current mix ratio.
    fn install(
        &mut self,
        lanes: Vec<(String, Arc<DecodedAudio>)>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let ratio = self.selected.ratio();
        self.model.update(cx, |model, cx| {
            model.set_enhanced_ratio(ratio, cx);
            model
                .set_enhanced_audio(lanes, cx)
                .map_err(|error| format!("Enhanced audio was rejected: {error}"))
        })
    }

    fn finish(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        self.job = None;
        self.ticker = None;
        self.run = match outcome {
            Outcome::Ready {
                source,
                cache_warning,
            } => RunState::Ready {
                source,
                cache_warning,
            },
            Outcome::NothingCached => RunState::Idle,
            Outcome::Failed(message) => RunState::Failed(message),
        };
        cx.notify();
    }

    // ---- rendering ---------------------------------------------------------

    fn generate_button(&self, cx: &mut Context<Self>) -> Button {
        let running = self.is_generating();
        let ready = matches!(self.run, RunState::Ready { .. });
        Button::new("denoise-generate")
            .outline()
            .small()
            .icon(IconName::AudioLines)
            .child(stable_text(
                if running { "Denoising" } else { "Denoise" },
                ["Denoise", "Denoising"],
            ))
            .accessibility_label(if running { "Denoising" } else { "Denoise" })
            .loading(running)
            .disabled(running || ready)
            .tooltip(if ready {
                "Enhanced audio is ready"
            } else {
                "Generate enhanced (denoised) audio"
            })
            .on_click(cx.listener(|this, _, _, cx| this.generate(cx)))
    }

    fn status_trigger(&self, cx: &App) -> Button {
        let theme = cx.theme();
        let (icon, color, label): (IconName, Option<Hsla>, Option<String>) = match &self.run {
            RunState::Running { phase, .. } => (
                IconName::LoaderCircle,
                None,
                Some(phase_fraction(phase).map_or_else(
                    || "…".to_string(),
                    |fraction| format!("{}%", (fraction * 100.0).round()),
                )),
            ),
            RunState::Ready { cache_warning, .. } => (
                if cache_warning.is_some() {
                    IconName::TriangleAlert
                } else {
                    IconName::CircleCheck
                },
                Some(if cache_warning.is_some() {
                    theme.warning
                } else {
                    theme.success
                }),
                None,
            ),
            RunState::Failed(_) => (IconName::CircleAlert, Some(theme.danger), None),
            RunState::Idle | RunState::Cancelled => (IconName::Info, None, None),
        };
        // The progress slot is always reserved (as wide as "100%"), so a run starting or
        // ending never moves the overview beside it.
        Button::new("denoise-status")
            .ghost()
            .small()
            .icon(icon)
            .when_some(color, |button, color| button.text_color(color))
            .child(
                stable_text(label.unwrap_or_default(), ["100%"])
                    .text_right()
                    .font_features(tabular_figures()),
            )
            .accessibility_label("Denoise status")
            .tooltip("Denoise status")
    }

    fn status_content(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let mut rows = v_flex().gap_1();
        let location_text = |location: ExecutionLocation| match location {
            ExecutionLocation::Local => "This PC",
            ExecutionLocation::Remote => "Remote GPU",
        };
        let configured = cx.global::<AppServices>().ai.denoise_location;
        let mut actions = h_flex().gap_2().justify_end().pt_1();
        match &self.run {
            RunState::Idle => {
                rows = rows
                    .child(muted("No enhanced audio yet.", cx))
                    .child(muted(format!("Runs on: {}", location_text(configured)), cx));
            }
            RunState::Running {
                location,
                started,
                phase,
                ..
            } => {
                let elapsed = started.elapsed();
                let fraction = phase_fraction(phase);
                rows = rows
                    .child(div().text_sm().child(phase_text(phase)))
                    .children(
                        phase_detail(phase, &self.model.read(cx).doc().project().lanes)
                            .map(|detail| muted(detail, cx)),
                    )
                    .child(
                        Progress::new("denoise-progress")
                            .value(fraction.map_or(0.0, |fraction| fraction * 100.0))
                            .loading(fraction.is_none()),
                    )
                    .child(muted(
                        format!(
                            "{} · elapsed {}",
                            location_text(*location),
                            format_duration(elapsed)
                        ),
                        cx,
                    ));
                actions = actions.child(
                    Button::new("denoise-cancel")
                        .small()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                );
            }
            RunState::Ready {
                source,
                cache_warning,
            } => {
                rows = rows.child(div().text_sm().child(match source {
                    ReadySource::Cache => "Using saved audio".to_string(),
                    ReadySource::Generated { location, took } => format!(
                        "Enhanced on {} in {}",
                        location_text(*location),
                        format_duration(*took)
                    ),
                }));
                if let Some(warning) = cache_warning {
                    rows = rows.child(error_text(
                        format!("Could not save enhanced audio for next time: {warning}"),
                        cx,
                    ));
                }
            }
            RunState::Cancelled => {
                rows = rows.child(muted("Denoising was cancelled.", cx));
            }
            RunState::Failed(message) => {
                rows = rows.child(error_text(message.clone(), cx));
            }
        }
        if configured == ExecutionLocation::Remote && !matches!(self.run, RunState::Ready { .. }) {
            rows = rows.child(muted(
                "Remote GPU uploads both original tracks to the configured server unless a saved result exists.",
                cx,
            ));
        }
        rows = rows.child(muted(format!("Mix: {}", self.preview.label()), cx));
        if matches!(
            self.run,
            RunState::Idle | RunState::Cancelled | RunState::Failed(_)
        ) {
            actions = actions.child(
                Button::new("denoise-popover-generate")
                    .small()
                    .primary()
                    .label(if matches!(self.run, RunState::Failed(_)) {
                        "Retry"
                    } else {
                        "Generate"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.generate(cx))),
            );
        }
        v_flex()
            .w(rems(20.).to_pixels(window.rem_size()))
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Denoise"),
            )
            .child(rows)
            .child(actions)
    }
}

impl Render for DenoiseControl {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let control = cx.entity();
        let status = Popover::new("denoise-status-popover")
            .anchor(Anchor::TopRight)
            .trigger(self.status_trigger(cx))
            .content(move |_, window, cx| {
                control.update(cx, |this, cx| this.status_content(window, cx))
            });
        h_flex()
            .flex_none()
            .gap_1()
            .child(self.generate_button(cx))
            .when(!self.narrow, |row| {
                row.child(
                    div()
                        .id("denoise-mix")
                        .w(rems(6.))
                        .child(Slider::new(&self.slider).horizontal())
                        .tooltip(|window, cx| {
                            Tooltip::new("Mix: Enhanced (left) ↔ Original (right)")
                                .build(window, cx)
                        }),
                )
                .child(
                    div()
                        .w(rems(4.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.preview.short_label()),
                )
            })
            .child(status)
    }
}

// ---- job -------------------------------------------------------------------

async fn run_job(
    this: WeakEntity<DenoiseControl>,
    mode: RunMode,
    input: JobInput,
    cx: &mut AsyncApp,
) -> Outcome {
    let started = Instant::now();
    let JobInput {
        task_id,
        sources,
        location,
        backend,
        cache,
    } = input;

    let prepare = {
        let cache = cache.clone();
        let task_id = task_id.clone();
        cx.background_spawn(async move { prepare_lanes(&sources, &cache, &task_id) })
    };
    let prepared = match prepare.await {
        Ok(prepared) => prepared,
        Err(message) => {
            return match mode {
                RunMode::CacheOnly => Outcome::NothingCached,
                RunMode::Generate => Outcome::Failed(message),
            };
        }
    };
    if let Some(cached) = prepared.cached {
        match this.update(cx, |this, cx| this.install(cached, cx)) {
            Ok(Ok(())) => {
                return Outcome::Ready {
                    source: ReadySource::Cache,
                    cache_warning: None,
                };
            }
            // A cached result that no longer fits the originals is a miss.
            Ok(Err(_)) => {}
            Err(_) => return Outcome::NothingCached,
        }
    }
    if mode == RunMode::CacheOnly {
        return Outcome::NothingCached;
    }

    let (progress_tx, progress_rx) = async_channel::unbounded();
    let progress = ProgressSink::new(move |event| {
        let _ = progress_tx.try_send(event);
    });
    let pump = {
        let this = this.clone();
        cx.spawn(async move |cx| {
            while let Ok(event) = progress_rx.recv().await {
                if this
                    .update(cx, |this, cx| this.on_progress(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
    };
    // Dropping this job (cancel, regenerate, leaving the task) drops `_cancel`, which ends
    // the backend future on the services runtime.
    let (_cancel, cancelled) = async_channel::bounded::<()>(1);
    let lanes = prepared.lanes;
    let expected_ids = lanes
        .iter()
        .map(|lane| lane.lane_id.clone())
        .collect::<Vec<_>>();
    let request_task_id = task_id.clone();
    let result = studio_services::runtime::spawn(async move {
        until_cancelled(
            backend.enhance(&request_task_id, &lanes, progress),
            cancelled,
        )
        .await
    })
    .await;
    // A backend may keep clones of the sink; stop forwarding instead of waiting for it.
    drop(pump);
    let enhanced = match result {
        Some(Ok(enhanced)) => enhanced,
        Some(Err(error)) => return Outcome::Failed(format!("Denoising failed: {error}")),
        None => return Outcome::Failed("Denoising was cancelled.".to_string()),
    };

    this.update(cx, |this, cx| this.set_phase(Phase::Installing, cx))
        .ok();
    let decoded = match cx
        .background_spawn(async move { decode_result(enhanced, &expected_ids) })
        .await
    {
        Ok(decoded) => decoded,
        Err(message) => return Outcome::Failed(message),
    };
    match this.update(cx, |this, cx| this.install(decoded.lanes, cx)) {
        Ok(Ok(())) => {}
        Ok(Err(message)) => return Outcome::Failed(message),
        Err(_) => return Outcome::NothingCached,
    }
    let took = started.elapsed();
    let digests = prepared.digests;
    let wavs = decoded.wavs;
    let model_id = decoded.model_id;
    let stored = cx
        .background_spawn(async move {
            let entries = digests
                .iter()
                .zip(&wavs)
                .map(|(digest, wav)| (digest.as_str(), wav.as_ref()))
                .collect::<Vec<_>>();
            cache.store(&task_id, &model_id, &entries)
        })
        .await;
    Outcome::Ready {
        source: ReadySource::Generated { location, took },
        cache_warning: stored.err().map(|error| error.to_string()),
    }
}

/// Reads the original lane bytes, digests them and decodes a cached result if one exists.
/// Unreadable or undecodable cache entries count as a miss.
fn prepare_lanes(
    sources: &[LaneSource],
    cache: &EnhancedAudioCache,
    task_id: &str,
) -> Result<PreparedLanes, String> {
    let lanes = read_lane_audio(sources).map_err(|error| format!("{error:#}"))?;
    let digests = lanes
        .iter()
        .map(|lane| sha256_hex(&lane.wav))
        .collect::<Vec<_>>();
    let keys = digests.iter().map(String::as_str).collect::<Vec<_>>();
    let cached = cache
        .load(task_id, GOLD_ENHANCEMENT_MODEL.id, &keys)
        .ok()
        .flatten()
        .and_then(|wavs| {
            lanes
                .iter()
                .zip(wavs)
                .map(|(lane, wav)| {
                    decode_audio_bytes(&wav)
                        .ok()
                        .map(|audio| (lane.lane_id.clone(), Arc::new(audio)))
                })
                .collect::<Option<Vec<_>>>()
        });
    Ok(PreparedLanes {
        lanes,
        digests,
        cached,
    })
}

/// Decodes the backend's lanes in `expected_ids` order; every requested lane must come back
/// exactly once from one enhancer model.
fn decode_result(
    enhanced: Vec<EnhancedLane>,
    expected_ids: &[String],
) -> Result<DecodedResult, String> {
    let model_id = enhanced
        .first()
        .map(|lane| lane.model_id.clone())
        .unwrap_or_default();
    if enhanced.len() != expected_ids.len() || enhanced.iter().any(|lane| lane.model_id != model_id)
    {
        return Err(format!(
            "Denoising returned {} tracks for {} requested.",
            enhanced.len(),
            expected_ids.len()
        ));
    }
    let mut lanes = Vec::with_capacity(expected_ids.len());
    let mut wavs = Vec::with_capacity(expected_ids.len());
    for lane_id in expected_ids {
        let lane = enhanced
            .iter()
            .find(|lane| lane.lane_id == *lane_id)
            .ok_or_else(|| format!("Denoising returned no audio for track {lane_id}."))?;
        let audio = decode_audio_bytes(&lane.wav).map_err(|error| {
            format!("Denoising returned unreadable audio for track {lane_id}: {error}")
        })?;
        lanes.push((lane_id.clone(), Arc::new(audio)));
        wavs.push(lane.wav.clone());
    }
    Ok(DecodedResult {
        model_id,
        lanes,
        wavs,
    })
}

/// Completed share of the current phase, when it has a known total.
fn phase_fraction(phase: &Phase) -> Option<f32> {
    match phase {
        Phase::Stage {
            completed,
            total: Some(total),
            ..
        } if *total > 0 => Some((*completed as f32 / *total as f32).clamp(0.0, 1.0)),
        Phase::Download { bytes, total } if *total > 0 => {
            Some((*bytes as f32 / *total as f32).clamp(0.0, 1.0))
        }
        _ => None,
    }
}

fn phase_text(phase: &Phase) -> String {
    match phase {
        Phase::Preparing => "Reading the original audio".to_string(),
        Phase::Queued { position: 0, .. } => "Running on the server".to_string(),
        Phase::Queued { position, count } => format!("Queued: position {position} of {count}"),
        Phase::Stage {
            stage,
            completed,
            total: Some(total),
            ..
        } => format!("{}: chunk {completed}/{total}", sentence_case(stage)),
        Phase::Stage {
            stage, completed, ..
        } if *completed > 0 => format!("{}: {completed} done", sentence_case(stage)),
        Phase::Stage { stage, .. } => sentence_case(stage),
        Phase::Download { bytes, total } => format!(
            "Downloading models: {:.1} / {:.1} MB",
            *bytes as f64 / 1_000_000.0,
            *total as f64 / 1_000_000.0
        ),
        Phase::Installing => "Switching playback to the enhanced audio".to_string(),
    }
}

/// The stage detail; a lane id (the track being processed) is shown as its label.
fn phase_detail(phase: &Phase, lanes: &[AudioLane]) -> Option<String> {
    match phase {
        Phase::Stage {
            detail: Some(detail),
            ..
        } if !detail.is_empty() => Some(
            lanes
                .iter()
                .find(|lane| lane.id == *detail)
                .map_or_else(|| detail.clone(), |lane| lane.label.clone()),
        ),
        _ => None,
    }
}

fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{DEFAULT_ENHANCED_PERCENT, EnhancedLane, EnhancedPercent, decode_result};

    #[test]
    fn slider_positions_snap_to_ten_percent_detents() {
        assert_eq!(EnhancedPercent::from_slider(0.0), EnhancedPercent(100));
        assert_eq!(EnhancedPercent::from_slider(4.0), EnhancedPercent(100));
        assert_eq!(EnhancedPercent::from_slider(6.0), EnhancedPercent(90));
        assert_eq!(EnhancedPercent::from_slider(100.0), EnhancedPercent(0));
        assert_eq!(EnhancedPercent::from_slider(250.0), EnhancedPercent(0));
        assert_eq!(EnhancedPercent::from_slider(-5.0), EnhancedPercent(100));
        for percent in (0..=100).step_by(10) {
            let selection = EnhancedPercent(percent);
            assert_eq!(
                EnhancedPercent::from_slider(selection.slider_value()),
                selection
            );
        }
        assert_eq!(EnhancedPercent(DEFAULT_ENHANCED_PERCENT).ratio(), 0.9);
    }

    #[test]
    fn decoded_results_follow_the_requested_lane_order() {
        let wav = |value: i16| {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(b"RIFF");
            bytes.extend_from_slice(&38_u32.to_le_bytes());
            bytes.extend_from_slice(b"WAVEfmt ");
            bytes.extend_from_slice(&16_u32.to_le_bytes());
            bytes.extend_from_slice(&1_u16.to_le_bytes());
            bytes.extend_from_slice(&1_u16.to_le_bytes());
            bytes.extend_from_slice(&8_000_u32.to_le_bytes());
            bytes.extend_from_slice(&16_000_u32.to_le_bytes());
            bytes.extend_from_slice(&2_u16.to_le_bytes());
            bytes.extend_from_slice(&16_u16.to_le_bytes());
            bytes.extend_from_slice(b"data");
            bytes.extend_from_slice(&2_u32.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
            bytes::Bytes::from(bytes)
        };
        let lane = |id: &str, value: i16| EnhancedLane {
            lane_id: id.to_string(),
            model_id: "model".to_string(),
            wav: wav(value),
        };
        let expected = vec!["a".to_string(), "b".to_string()];

        let decoded = decode_result(vec![lane("b", 8_192), lane("a", 16_384)], &expected).unwrap();
        assert_eq!(decoded.model_id, "model");
        let samples = decoded
            .lanes
            .iter()
            .map(|(id, audio)| (id.as_str(), audio.channel_data(0).unwrap()[0]))
            .collect::<Vec<_>>();
        assert_eq!(samples, vec![("a", 0.5), ("b", 0.25)]);

        assert!(decode_result(vec![lane("a", 1)], &expected).is_err());
        assert!(decode_result(vec![lane("a", 1), lane("c", 1)], &expected).is_err());
        let mut other_model = lane("b", 1);
        other_model.model_id = "other".to_string();
        assert!(decode_result(vec![lane("a", 1), other_model], &expected).is_err());
    }
}
