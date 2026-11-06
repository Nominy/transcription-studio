//! Settings → AI: where inference runs (This PC / Remote GPU) with the local model status
//! (install / cancel / test / remove) and the remote server's per-operation status, Gold Draft
//! polish (OpenRouter key/model/tier/effort) and denoise. Every
//! value is persisted through [`GoldServices::update_ai`]; keys are masked and never logged.

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use sonus_gold::bundle::{DOWNLOAD_STAGE, EXTRACT_STAGE};
use sonus_gold::polish::BackendHealth;
use sonus_gold::prefs::{
    AiSettings, ExecutionLocation, REASONING_EFFORTS, SERVICE_TIERS, normalize_base_url,
};
use sonus_gold::protocol::{HostStatus, ModelsState, SampleResult};
use studio_inference::{BackendStatus, InferenceProgress, InferenceResult, ProgressSink};

use crate::editor::inference_audio::until_cancelled;
use crate::editor::panels::gold_progress::{hold, label};
use crate::services::GoldServices;
use sonus_shell::screens::ui::{self, Tone};
use sonus_shell::services::{AppServices, spawn_service};

const LOCATIONS: [(ExecutionLocation, &str); 2] = [
    (ExecutionLocation::Local, "This PC"),
    (ExecutionLocation::Remote, "Remote GPU"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextField {
    RemoteKey,
    GoldKey,
    GoldModel,
}

/// A model operation running on This PC.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelOp {
    Install,
    Test,
    Remove,
}

/// This PC: host status plus the operation in flight.
#[derive(Default)]
struct LocalModels {
    status: Option<Result<HostStatus, String>>,
    checking: bool,
    op: Option<ModelOp>,
    /// Progress of the running operation as Gold's options page labels it:
    /// "Downloading models… 42% · 1.1 GB of 2.6 GB".
    progress: Option<OpProgress>,
    /// Outcome of the last operation (`true` = error).
    notice: Option<(String, bool)>,
    sample: Option<SampleResult>,
    /// Dropping it cancels the running operation.
    cancel: Option<async_channel::Sender<()>>,
    tasks: Vec<Task<()>>,
}

/// One labelled bar of a model operation; a new phase starts its own bar.
struct OpProgress {
    action: &'static str,
    fraction: Option<f32>,
    step: String,
}

impl OpProgress {
    /// Switches to `action` (a new bar) unless it is already running.
    fn phase<'a>(progress: &'a mut Option<Self>, action: &'static str) -> &'a mut Self {
        if progress
            .as_ref()
            .is_none_or(|current| current.action != action)
        {
            *progress = Some(Self {
                action,
                fraction: None,
                step: String::new(),
            });
        }
        progress.as_mut().expect("set above")
    }
}

/// Gold `formatByteCount`-style sizes.
fn byte_count(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * MB;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else {
        format!("{:.0} MB", bytes / MB)
    }
}

/// "X of Y", "finishing" once every byte is in, or nothing without a total.
fn bytes_step(bytes: u64, total: u64) -> String {
    if total == 0 {
        String::new()
    } else if bytes >= total {
        "finishing".to_string()
    } else {
        format!("{} of {}", byte_count(bytes), byte_count(total))
    }
}

#[derive(Default)]
struct SettingsViews(Vec<WeakEntity<AiSettingsView>>);
impl Global for SettingsViews {}

/// Cancels settings work even when a standalone settings dialog outlives its contribution.
pub(crate) fn shutdown(cx: &mut App) {
    let views = if cx.has_global::<SettingsViews>() {
        std::mem::take(&mut cx.global_mut::<SettingsViews>().0)
    } else {
        Vec::new()
    };
    for view in views.into_iter().filter_map(|view| view.upgrade()) {
        view.update(cx, |view, cx| {
            view.unloaded = true;
            view.local.cancel.take();
            view.local.tasks.clear();
            view.remote_task.take();
            view.health_task.take();
            cx.notify();
        });
    }
}

pub struct AiSettingsView {
    unloaded: bool,
    remote_url: Entity<InputState>,
    remote_key: Entity<InputState>,
    gold_key: Entity<InputState>,
    gold_model: Entity<InputState>,
    remote_url_error: Option<String>,
    health: Option<Result<BackendHealth, String>>,
    health_task: Option<Task<()>>,
    local: LocalModels,
    remote_status: Option<BackendStatus>,
    remote_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AiSettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<SettingsViews>() {
            cx.set_global(SettingsViews::default());
        }
        let view = cx.entity().downgrade();
        let views = &mut cx.global_mut::<SettingsViews>().0;
        views.retain(|view| view.upgrade().is_some());
        views.push(view);
        let ai = cx.global::<GoldServices>().ai.clone();
        let input = |placeholder: &'static str,
                     value: &str,
                     masked: bool,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            let value = value.to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .masked(masked)
                    .default_value(value)
            })
        };
        let remote_url = input("Remote server URL", &ai.remote_base_url, false, window, cx);
        let remote_key = input(
            "Remote access key (optional)",
            &ai.remote_access_key,
            true,
            window,
            cx,
        );
        let gold_key = input(
            "OpenRouter API key (sk-or-…)",
            &ai.gold_openrouter_key,
            true,
            window,
            cx,
        );
        let gold_model = input("Polish model", &ai.gold_model, false, window, cx);
        let subscriptions = vec![
            Self::watch(&remote_key, TextField::RemoteKey, window, cx),
            Self::watch(&gold_key, TextField::GoldKey, window, cx),
            Self::watch(&gold_model, TextField::GoldModel, window, cx),
            cx.subscribe_in(&remote_url, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.save_remote_url(window, cx);
                }
            }),
        ];
        let mut this = Self {
            unloaded: false,
            remote_url,
            remote_key,
            gold_key,
            gold_model,
            remote_url_error: None,
            health: None,
            health_task: None,
            local: LocalModels::default(),
            remote_status: None,
            remote_task: None,
            _subscriptions: subscriptions,
        };
        this.refresh_local(cx);
        this.refresh_remote(cx);
        this
    }

    // ---- This PC models / remote status -------------------------------------------------

    fn refresh_local(&mut self, cx: &mut Context<Self>) {
        let local = cx.global::<GoldServices>().inference.local();
        self.local.checking = true;
        let task = spawn_service(
            cx,
            async move { local.host_status().await },
            |this, result, cx| {
                this.local.checking = false;
                this.local.status = Some(result.map_err(|error| error.message));
                cx.notify();
            },
        );
        self.local.tasks.push(task);
        cx.notify();
    }

    fn refresh_remote(&mut self, cx: &mut Context<Self>) {
        let remote = cx
            .global::<GoldServices>()
            .inference
            .backend(ExecutionLocation::Remote);
        self.remote_status = None;
        self.remote_task = Some(spawn_service(
            cx,
            async move { remote.status().await },
            |this, status, cx| {
                this.remote_task = None;
                this.remote_status = Some(status);
                cx.notify();
            },
        ));
        cx.notify();
    }

    /// Runs a model operation with progress; Cancel drops it (the page request is aborted).
    fn run_model_op<T: Send + 'static>(
        &mut self,
        op: ModelOp,
        cx: &mut Context<Self>,
        work: impl FnOnce(
            Arc<sonus_gold::LocalBackend>,
            ProgressSink,
        ) -> std::pin::Pin<Box<dyn Future<Output = InferenceResult<T>> + Send>>,
        done: impl FnOnce(&mut Self, T) -> String + 'static,
    ) {
        if self.local.op.is_some() {
            return;
        }
        let local = cx.global::<GoldServices>().inference.local();
        let (progress_tx, progress_rx) = async_channel::unbounded();
        let sink = ProgressSink::new(move |event| {
            let _ = progress_tx.try_send(event);
        });
        let (cancel, cancelled) = async_channel::bounded::<()>(1);
        self.local.op = Some(op);
        self.local.cancel = Some(cancel);
        self.local.progress = None;
        self.local.notice = None;
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(event) = progress_rx.recv().await {
                if this
                    .update(cx, |this, cx| this.on_model_progress(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let work = work(local, sink);
        let task = spawn_service(
            cx,
            async move { until_cancelled(work, cancelled).await },
            move |this, result, cx| {
                drop(pump);
                this.local.op = None;
                this.local.cancel = None;
                this.local.progress = None;
                this.local.notice = Some(match result {
                    Some(Ok(value)) => (done(this, value), false),
                    Some(Err(error)) => (error.message, true),
                    None => ("Cancelled.".to_string(), true),
                });
                this.refresh_local(cx);
            },
        );
        self.local.tasks.push(task);
        cx.notify();
    }

    fn on_model_progress(&mut self, event: InferenceProgress, cx: &mut Context<Self>) {
        let ratio = |completed: u64, total: u64| {
            (total > 0).then(|| (completed as f32 / total as f32).clamp(0.0, 1.0))
        };
        let progress = &mut self.local.progress;
        match event {
            InferenceProgress::Stage {
                stage,
                completed,
                total,
                ..
            } if stage == DOWNLOAD_STAGE => {
                let bar = OpProgress::phase(progress, "Downloading Gold runtime");
                let total = total.unwrap_or(0);
                bar.fraction = hold(bar.fraction, ratio(completed, total));
                bar.step = bytes_step(completed, total);
            }
            InferenceProgress::Stage { stage, .. } if stage == EXTRACT_STAGE => {
                OpProgress::phase(progress, "Installing Gold runtime");
            }
            // Bytes in place are 98% of an install; validation and activation follow
            // (Gold `modelDownloadFraction`).
            InferenceProgress::Download { bytes, total } => {
                let bar = OpProgress::phase(progress, "Downloading models");
                bar.fraction = hold(bar.fraction, ratio(bytes, total).map(|f| 0.98 * f));
                bar.step = bytes_step(bytes, total);
            }
            // The sample test reports no overall value; Gold shows "Testing…".
            _ => return,
        }
        cx.notify();
    }

    fn install_models(&mut self, cx: &mut Context<Self>) {
        self.run_model_op(
            ModelOp::Install,
            cx,
            |local, sink| Box::pin(async move { local.install_models(sink).await }),
            |_, models| match models.state {
                ModelsState::Ready => "Models installed. Run the test to finish setup.".to_string(),
                _ => models
                    .error
                    .unwrap_or_else(|| "Model download stopped.".to_string()),
            },
        );
    }

    fn test_models(&mut self, cx: &mut Context<Self>) {
        self.run_model_op(
            ModelOp::Test,
            cx,
            |local, sink| Box::pin(async move { local.test_sample(sink).await }),
            |this, sample| {
                let message = format!(
                    "Test passed: {} words in {:.1} s of sample audio.",
                    sample.tokens.len(),
                    sample.duration_seconds
                );
                this.local.sample = Some(sample);
                message
            },
        );
    }

    fn remove_models(&mut self, cx: &mut Context<Self>) {
        self.run_model_op(
            ModelOp::Remove,
            cx,
            |local, _| Box::pin(async move { local.remove_models().await }),
            |this, _| {
                this.local.sample = None;
                "Models removed.".to_string()
            },
        );
    }

    fn cancel_model_op(&mut self, cx: &mut Context<Self>) {
        self.local.cancel = None;
        cx.notify();
    }

    fn watch(
        input: &Entity<InputState>,
        field: TextField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(input, window, move |this, input, event, window, cx| {
            if let InputEvent::Change = event {
                let value = input.read(cx).value().trim().to_string();
                this.update(window, cx, move |ai| match field {
                    TextField::RemoteKey => ai.remote_access_key = value,
                    TextField::GoldKey => ai.gold_openrouter_key = value,
                    TextField::GoldModel => ai.gold_model = value,
                });
            }
        })
    }

    /// Persists one change; a write failure is a visible warning (the in-memory value still
    /// applies for this session).
    fn update(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut AiSettings),
    ) {
        let result = cx.update_global::<GoldServices, _>(|gold, cx| {
            gold.update_ai(&mut cx.global_mut::<AppServices>().storage, change)
        });
        if let Err(error) = result {
            window.push_notification(
                Notification::warning(format!("AI settings could not be saved: {error}")),
                cx,
            );
        }
        cx.notify();
    }

    fn save_remote_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.remote_url.read(cx).value().to_string();
        let Some(url) = normalize_base_url(&raw) else {
            self.remote_url_error = Some("Enter an http(s) server address.".to_string());
            cx.notify();
            return;
        };
        self.remote_url_error = None;
        if url != raw {
            let shown = url.clone();
            self.remote_url
                .update(cx, |input, cx| input.set_value(shown, window, cx));
        }
        self.update(window, cx, move |ai| ai.remote_base_url = url);
        self.refresh_remote(cx);
    }

    fn check_backend(&mut self, cx: &mut Context<Self>) {
        let Some(review) = cx.global::<GoldServices>().polish.clone() else {
            self.health = Some(Err(
                "No polish provider is available for this document.".to_string()
            ));
            cx.notify();
            return;
        };
        self.health = None;
        self.health_task = Some(spawn_service(
            cx,
            async move { review.health().await },
            |this, result, cx| {
                this.health_task = None;
                this.health = Some(result.map_err(|error| format!("{error:#}")));
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn location_group(
        id: &'static str,
        current: ExecutionLocation,
        cx: &mut Context<Self>,
        apply: fn(&mut AiSettings, ExecutionLocation),
    ) -> ButtonGroup {
        ButtonGroup::new(id)
            .outline()
            .small()
            .children(LOCATIONS.iter().enumerate().map(|(ix, (location, label))| {
                Button::new((id, ix))
                    .label(*label)
                    .selected(current == *location)
            }))
            .on_click(cx.listener(move |this, clicked: &Vec<usize>, window, cx| {
                if let Some((location, _)) = clicked.first().and_then(|ix| LOCATIONS.get(*ix)) {
                    let location = *location;
                    this.update(window, cx, move |ai| apply(ai, location));
                }
            }))
    }

    fn choice_group(
        id: &'static str,
        options: &'static [&'static str],
        current: &str,
        cx: &mut Context<Self>,
        apply: fn(&mut AiSettings, &'static str),
    ) -> ButtonGroup {
        ButtonGroup::new(id)
            .outline()
            .xsmall()
            .children(options.iter().enumerate().map(|(ix, option)| {
                Button::new((id, ix))
                    .label(*option)
                    .selected(current == *option)
            }))
            .on_click(cx.listener(move |this, clicked: &Vec<usize>, window, cx| {
                if let Some(option) = clicked.first().and_then(|ix| options.get(*ix)) {
                    let option = *option;
                    this.update(window, cx, move |ai| apply(ai, option));
                }
            }))
    }

    fn labeled(label: &'static str, control: impl IntoElement, cx: &App) -> Div {
        h_flex()
            .flex_wrap()
            .gap_2()
            .justify_between()
            .child(ui::muted(label, cx))
            .child(control)
    }

    fn render_inference(&self, ai: &AiSettings, cx: &mut Context<Self>) -> Div {
        let remote = ai.execution_location == ExecutionLocation::Remote;
        ui::card(
            "AI inference",
            Some("Where word timestamps, draft text and segment transcription run."),
            cx,
        )
        .child(Self::labeled(
            "Where to draft",
            Self::location_group("ai-location", ai.execution_location, cx, |ai, location| {
                ai.execution_location = location
            }),
            cx,
        ))
        .child(
            v_flex()
                .gap_2()
                .child(ui::muted(
                    if remote {
                        "Remote server (used for drafting)"
                    } else {
                        "Remote server (used when drafting on the remote GPU)"
                    },
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.remote_url)))
                        .child(
                            Button::new("ai-remote-save")
                                .outline()
                                .label("Save URL")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_remote_url(window, cx)
                                })),
                        ),
                )
                .when_some(self.remote_url_error.clone(), |this, error| {
                    this.child(div().text_xs().text_color(cx.theme().danger).child(error))
                })
                .child(Input::new(&self.remote_key).mask_toggle()),
        )
    }

    /// Remote server status for each operation.
    fn remote_status_line(&self) -> (Tone, String) {
        match &self.remote_status {
            None => (Tone::Info, "Remote GPU: checking…".to_string()),
            Some(BackendStatus::Ready {
                release,
                operations,
            }) => {
                let mark = |ok: bool| if ok { "available" } else { "unavailable" };
                (
                    if operations.timing && operations.draft {
                        Tone::Success
                    } else {
                        Tone::Warning
                    },
                    format!(
                        "Remote GPU ({release}): timestamps {}, draft text {}, denoise {}, error-task alignment {}",
                        mark(operations.timing),
                        mark(operations.draft),
                        mark(operations.enhance),
                        mark(operations.align)
                    ),
                )
            }
            Some(BackendStatus::NeedsModels { release, .. }) => (
                Tone::Warning,
                format!("Remote GPU: release {release} is not ready."),
            ),
            Some(BackendStatus::Unavailable { message, .. }) => {
                (Tone::Danger, format!("Remote GPU: {message}"))
            }
        }
    }

    fn render_models(&self, cx: &mut Context<Self>) -> Div {
        let local = &self.local;
        let busy = local.op.is_some();
        let theme = cx.theme().clone();
        let mut card = ui::card(
            "Models on this PC",
            Some(
                "Gold's local models (downloaded once, about 2.6 GB) run on this PC's GPU through WebGPU.",
            ),
            cx,
        );
        let (summary, tone) = match &local.status {
            None => ("Checking this PC…".to_string(), Tone::Info),
            Some(Err(error)) => (error.clone(), Tone::Danger),
            Some(Ok(status)) => {
                const GB: f64 = 1024.0 * 1024.0 * 1024.0;
                let models = &status.models;
                let state = match models.state {
                    ModelsState::Ready if models.outdated => {
                        "an older release is installed".to_string()
                    }
                    ModelsState::Ready if models.tested => "installed and tested".to_string(),
                    ModelsState::Ready => "installed, not tested yet".to_string(),
                    ModelsState::Downloading => "downloading".to_string(),
                    ModelsState::NotInstalled => format!(
                        "not installed ({:.1} GB to download)",
                        status.download_bytes.unwrap_or(models.total_bytes) as f64 / GB
                    ),
                    ModelsState::Stopped => "download stopped".to_string(),
                    ModelsState::Error => models
                        .error
                        .clone()
                        .unwrap_or_else(|| "model error".to_string()),
                };
                let gpu = if status.webgpu.available && status.webgpu.shader_f16 {
                    status
                        .webgpu
                        .adapter
                        .as_ref()
                        .map(|adapter| {
                            format!("GPU: {} {}", adapter.vendor, adapter.architecture)
                                .trim()
                                .to_string()
                        })
                        .unwrap_or_else(|| "GPU: WebGPU ready".to_string())
                } else {
                    format!(
                        "GPU: {}",
                        status.webgpu.reason.clone().unwrap_or_else(|| {
                            "WebGPU with shader-f16 is unavailable".to_string()
                        })
                    )
                };
                let tone = if status.ready {
                    Tone::Success
                } else if status.webgpu.available && status.webgpu.shader_f16 {
                    Tone::Warning
                } else {
                    Tone::Danger
                };
                (format!("Release {}: {state}. {gpu}.", status.release), tone)
            }
        };
        card = card.child(ui::banner(tone, "This PC", Some(summary.into()), None, cx));
        if busy {
            let (line, fraction) = match &local.progress {
                Some(bar) => (label(bar.action, bar.fraction, &bar.step), bar.fraction),
                None => (
                    match local.op {
                        Some(ModelOp::Install) => "Downloading models…",
                        Some(ModelOp::Test) => "Testing…",
                        Some(ModelOp::Remove) => "Removing models…",
                        None => "Working…",
                    }
                    .to_string(),
                    None,
                ),
            };
            let bar = Progress::new("ai-models-progress");
            card = card
                .child(match fraction {
                    Some(fraction) => bar.value(fraction * 100.0),
                    None => bar.loading(true),
                })
                .child(ui::muted(line, cx));
        }
        if let Some((notice, error)) = &local.notice {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(if *error { theme.danger } else { theme.success })
                    .child(notice.clone()),
            );
        }
        if let Some(sample) = &local.sample {
            card = card.child(ui::muted(format!("Sample transcript: {}", sample.text), cx));
        }
        let installed = matches!(
            &local.status,
            Some(Ok(status)) if status.models.state == ModelsState::Ready
        );
        let (remote_tone, remote_line) = self.remote_status_line();
        let remote_color = match remote_tone {
            Tone::Success => theme.success,
            Tone::Warning => theme.warning,
            Tone::Danger => theme.danger,
            Tone::Info => theme.muted_foreground,
        };
        card.child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("ai-models-install")
                        .outline()
                        .small()
                        .label(if installed {
                            "Reinstall / update"
                        } else {
                            "Install models"
                        })
                        .loading(local.op == Some(ModelOp::Install))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.install_models(cx))),
                )
                .when(busy, |row| {
                    row.child(
                        Button::new("ai-models-cancel")
                            .outline()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_model_op(cx))),
                    )
                })
                .child(
                    Button::new("ai-models-test")
                        .outline()
                        .small()
                        .label("Test with sample")
                        .loading(local.op == Some(ModelOp::Test))
                        .disabled(busy || !installed)
                        .on_click(cx.listener(|this, _, _, cx| this.test_models(cx))),
                )
                .child(
                    Button::new("ai-models-remove")
                        .outline()
                        .small()
                        .label("Remove models")
                        .disabled(busy || !installed)
                        .on_click(cx.listener(|this, _, _, cx| this.remove_models(cx))),
                )
                .child(
                    Button::new("ai-models-refresh")
                        .ghost()
                        .small()
                        .label("Refresh")
                        .loading(local.checking || self.remote_task.is_some())
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_local(cx);
                            this.refresh_remote(cx);
                        })),
                ),
        )
        .child(div().text_xs().text_color(remote_color).child(remote_line))
    }

    fn render_gold(&self, ai: &AiSettings, cx: &mut Context<Self>) -> Div {
        let checking = self.health_task.is_some();
        let key_hint = key_hint(&ai.gold_openrouter_key);
        let mut card = ui::card(
            "Gold Draft polish",
            Some("Optional OpenRouter refinement after timestamps; the key also powers error-task Initial fill."),
            cx,
        )
        .child(Self::labeled(
            "Polish draft text with OpenRouter",
            Switch::new("ai-polish")
                .checked(ai.gold_polish_enabled)
                .on_click(cx.listener(|this, checked: &bool, window, cx| {
                    let checked = *checked;
                    this.update(window, cx, move |ai| ai.gold_polish_enabled = checked);
                })),
            cx,
        ))
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .child(Input::new(&self.gold_key).mask_toggle()),
                )
                .child(
                    Button::new("check-backend")
                        .outline()
                        .label("Check backend")
                        .loading(checking)
                        .disabled(checking)
                        .on_click(cx.listener(|this, _, _, cx| this.check_backend(cx))),
                ),
        )
        .when_some(key_hint, |this, hint| {
            this.child(div().text_xs().text_color(cx.theme().warning).child(hint))
        })
        .child(Input::new(&self.gold_model))
        .child(Self::labeled(
            "Service tier",
            Self::choice_group(
                "ai-tier",
                &SERVICE_TIERS,
                &ai.gold_service_tier,
                cx,
                |ai, tier| ai.gold_service_tier = tier.to_string(),
            ),
            cx,
        ))
        .child(Self::labeled(
            "Reasoning effort",
            Self::choice_group(
                "ai-effort",
                &REASONING_EFFORTS,
                &ai.gold_reasoning_effort,
                cx,
                |ai, effort| ai.gold_reasoning_effort = effort.to_string(),
            ),
            cx,
        ));
        match &self.health {
            Some(Ok(health)) => {
                let mut details = vec![health.service.clone()];
                details.extend(health.backend_version.clone());
                details.extend(health.openrouter_status.clone());
                if health.test_mode == Some(true) {
                    details.push("test mode".to_string());
                }
                details.extend(health.provider_diagnostics.iter().cloned());
                card = card.child(ui::banner(
                    Tone::Success,
                    "Backend reachable",
                    Some(details.join(" · ").into()),
                    None,
                    cx,
                ));
            }
            Some(Err(error)) => {
                card = card.child(ui::banner(
                    Tone::Danger,
                    "Backend check failed",
                    Some(error.clone().into()),
                    None,
                    cx,
                ));
            }
            None => {}
        }
        card
    }

    fn render_denoise(&self, ai: &AiSettings, cx: &mut Context<Self>) -> Div {
        ui::card(
            "Denoise",
            Some("Speech enhancement for the Original/Enhanced mix."),
            cx,
        )
        .child(Self::labeled(
            "Where to denoise",
            Self::location_group(
                "ai-denoise-location",
                ai.denoise_location,
                cx,
                |ai, location| ai.denoise_location = location,
            ),
            cx,
        ))
        .child(Self::labeled(
            "Denoise automatically when a task opens",
            Switch::new("ai-auto-denoise")
                .checked(ai.auto_denoise)
                .on_click(cx.listener(|this, checked: &bool, window, cx| {
                    let checked = *checked;
                    this.update(window, cx, move |ai| ai.auto_denoise = checked);
                })),
            cx,
        ))
    }
}

/// OpenRouter's key rule (`sk-or-`, ≤512 characters, no whitespace); `None` when valid or empty.
fn key_hint(key: &str) -> Option<&'static str> {
    if key.is_empty() {
        return None;
    }
    let valid =
        key.starts_with("sk-or-") && key.len() <= 512 && !key.chars().any(char::is_whitespace);
    (!valid).then_some("OpenRouter keys start with sk-or- and contain no spaces.")
}

impl Render for AiSettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.unloaded {
            return v_flex()
                .child("Gold is disabled. Re-enable Gold to manage inference settings.");
        }
        let ai = cx.global::<GoldServices>().ai.clone();
        v_flex()
            .w_full()
            .gap_4()
            .child(self.render_inference(&ai, cx))
            .child(self.render_models(cx))
            .when(cx.global::<GoldServices>().polish.is_some(), |view| {
                view.child(self.render_gold(&ai, cx))
            })
            .child(self.render_denoise(&ai, cx))
    }
}
