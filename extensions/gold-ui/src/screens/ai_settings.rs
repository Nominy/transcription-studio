//! Settings → AI: where inference runs (this computer / remote server) with the local model
//! status (download / cancel / test / delete) and the remote server's per-operation status,
//! polish (OpenRouter key/model/tier/effort) and denoise. Every value is persisted through
//! [`GoldServices::update_ai`]; keys are masked and never logged.

use std::sync::Arc;

use exaudio_gold::bundle::{DOWNLOAD_STAGE, EXTRACT_STAGE};
use exaudio_gold::polish::BackendHealth;
use exaudio_gold::prefs::{
    AiSettings, ExecutionLocation, REASONING_EFFORTS, SERVICE_TIERS, normalize_base_url,
};
use exaudio_gold::protocol::{HostStatus, ModelsState, SampleResult};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_inference::{BackendStatus, InferenceProgress, InferenceResult, ProgressSink};

use crate::editor::inference_audio::until_cancelled;
use crate::editor::panels::gold_progress::{hold, label};
use crate::services::GoldServices;
use exaudio_shell::screens::ui::{self, Tone, localized};
use exaudio_shell::services::{AppServices, spawn_service};

/// Execution locations with their EN / RU labels.
const LOCATIONS: [(ExecutionLocation, &str, &str); 2] = [
    (ExecutionLocation::Local, "This computer", "Этот компьютер"),
    (
        ExecutionLocation::Remote,
        "Remote server",
        "Удалённый сервер",
    ),
];

/// Display label of a raw [`SERVICE_TIERS`] value.
fn tier_label(cx: &App, tier: &str) -> &'static str {
    match tier {
        "flex" => localized(cx, "Cheaper", "Дешевле"),
        "priority" => localized(cx, "Faster", "Быстрее"),
        _ => localized(cx, "Standard", "Обычный"),
    }
}

/// Display label of a raw [`REASONING_EFFORTS`] value.
fn effort_label(cx: &App, effort: &str) -> &'static str {
    match effort {
        "none" => localized(cx, "Off", "Выкл."),
        "minimal" => localized(cx, "Minimal", "Минимум"),
        "low" => localized(cx, "Low", "Низкое"),
        "medium" => localized(cx, "Medium", "Среднее"),
        "high" => localized(cx, "High", "Высокое"),
        "xhigh" => localized(cx, "Very high", "Очень высокое"),
        _ => localized(cx, "Default", "По умолчанию"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextField {
    RemoteKey,
    GoldKey,
    GoldModel,
}

/// A model operation running on this computer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelOp {
    Install,
    Test,
    Remove,
}

/// This computer: host status plus the operation in flight.
#[derive(Default)]
struct LocalModels {
    status: Option<Result<HostStatus, String>>,
    checking: bool,
    op: Option<ModelOp>,
    /// Progress of the running operation, labelled like Gold's options page:
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
    /// Localized phase name without the trailing ellipsis ([`label`] adds it).
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
fn byte_count(bytes: u64, cx: &App) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * MB;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} {}", bytes / GB, localized(cx, "GB", "ГБ"))
    } else {
        format!("{:.0} {}", bytes / MB, localized(cx, "MB", "МБ"))
    }
}

/// "X of Y", "Finishing…" once every byte is in, or nothing without a total.
fn bytes_step(bytes: u64, total: u64, cx: &App) -> String {
    if total == 0 {
        String::new()
    } else if bytes >= total {
        localized(cx, "Finishing…", "Завершение…").to_string()
    } else {
        localized(cx, "{a} of {b}", "{a} из {b}")
            .replace("{a}", &byte_count(bytes, cx))
            .replace("{b}", &byte_count(total, cx))
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
    /// Backend health; the error is the technical text shown under "Details".
    health: Option<Result<BackendHealth, String>>,
    health_task: Option<Task<()>>,
    local: LocalModels,
    remote_status: Option<BackendStatus>,
    remote_task: Option<Task<()>>,
    /// "Advanced" disclosures: remote server inputs, polish model/priority/thinking.
    advanced_remote: bool,
    advanced_polish: bool,
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
        let remote_url = input(
            localized(cx, "Address", "Адрес"),
            &ai.remote_base_url,
            false,
            window,
            cx,
        );
        let remote_key = input(
            localized(cx, "Access key (optional)", "Ключ доступа (необязательно)"),
            &ai.remote_access_key,
            true,
            window,
            cx,
        );
        let gold_key = input(
            localized(cx, "OpenRouter key", "Ключ OpenRouter"),
            &ai.gold_openrouter_key,
            true,
            window,
            cx,
        );
        let gold_model = input(
            localized(cx, "Model", "Модель"),
            &ai.gold_model,
            false,
            window,
            cx,
        );
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
            advanced_remote: ai.execution_location == ExecutionLocation::Remote
                || ai.denoise_location == ExecutionLocation::Remote,
            advanced_polish: false,
            _subscriptions: subscriptions,
        };
        this.refresh_local(cx);
        this.refresh_remote(cx);
        this
    }

    // ---- Models on this computer / remote status ----------------------------------------

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
            Arc<exaudio_gold::LocalBackend>,
            ProgressSink,
        ) -> std::pin::Pin<Box<dyn Future<Output = InferenceResult<T>> + Send>>,
        done: impl FnOnce(&mut Self, T, &App) -> String + 'static,
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
                    Some(Ok(value)) => (done(this, value, cx), false),
                    Some(Err(error)) => (error.message, true),
                    None => (localized(cx, "Cancelled", "Отменено").to_string(), true),
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
                let bar = OpProgress::phase(
                    progress,
                    localized(cx, "Downloading runtime", "Загрузка среды"),
                );
                let total = total.unwrap_or(0);
                bar.fraction = hold(bar.fraction, ratio(completed, total));
                bar.step = bytes_step(completed, total, cx);
            }
            InferenceProgress::Stage { stage, .. } if stage == EXTRACT_STAGE => {
                OpProgress::phase(
                    progress,
                    localized(cx, "Installing runtime", "Установка среды"),
                );
            }
            // Bytes in place are 98% of an install; validation and activation follow
            // (Gold `modelDownloadFraction`).
            InferenceProgress::Download { bytes, total } => {
                let bar = OpProgress::phase(
                    progress,
                    localized(cx, "Downloading models", "Загрузка моделей"),
                );
                bar.fraction = hold(bar.fraction, ratio(bytes, total).map(|f| 0.98 * f));
                bar.step = bytes_step(bytes, total, cx);
            }
            // The sample test reports no overall value; the card shows "Testing…".
            _ => return,
        }
        cx.notify();
    }

    fn install_models(&mut self, cx: &mut Context<Self>) {
        self.run_model_op(
            ModelOp::Install,
            cx,
            |local, sink| Box::pin(async move { local.install_models(sink).await }),
            |_, models, cx| match models.state {
                ModelsState::Ready => localized(
                    cx,
                    "Downloaded. Run the test to finish.",
                    "Загружено. Запустите проверку.",
                )
                .to_string(),
                _ => models.error.unwrap_or_else(|| {
                    localized(cx, "Download stopped", "Загрузка остановлена").to_string()
                }),
            },
        );
    }

    fn test_models(&mut self, cx: &mut Context<Self>) {
        self.run_model_op(
            ModelOp::Test,
            cx,
            |local, sink| Box::pin(async move { local.test_sample(sink).await }),
            |this, sample, cx| {
                let message = localized(
                    cx,
                    "Test passed — {n} words from {s} s of audio.",
                    "Проверка пройдена — {n} слов из {s} с аудио.",
                )
                .replace("{n}", &sample.tokens.len().to_string())
                .replace("{s}", &format!("{:.1}", sample.duration_seconds));
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
            |this, _, cx| {
                this.local.sample = None;
                localized(cx, "Models deleted", "Модели удалены").to_string()
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
                ui::warning_notification(
                    localized(
                        cx,
                        "Couldn't save AI settings.",
                        "Не удалось сохранить настройки ИИ.",
                    ),
                    Some(format!("{error:#}").into()),
                ),
                cx,
            );
        }
        cx.notify();
    }

    fn save_remote_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.remote_url.read(cx).value().to_string();
        let Some(url) = normalize_base_url(&raw) else {
            self.remote_url_error = Some(
                localized(
                    cx,
                    "Enter an address starting with http:// or https://.",
                    "Введите адрес, начинающийся с http:// или https://.",
                )
                .to_string(),
            );
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
            self.health = Some(Err(localized(
                cx,
                "Polish isn't available in this project.",
                "Доработка текста недоступна в этом проекте.",
            )
            .to_string()));
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
            .children(
                LOCATIONS
                    .iter()
                    .enumerate()
                    .map(|(ix, (location, en, ru))| {
                        Button::new((id, ix))
                            .label(localized(cx, en, ru))
                            .selected(current == *location)
                    }),
            )
            .on_click(cx.listener(move |this, clicked: &Vec<usize>, window, cx| {
                if let Some((location, ..)) = clicked.first().and_then(|ix| LOCATIONS.get(*ix)) {
                    let location = *location;
                    this.update(window, cx, move |ai| apply(ai, location));
                }
            }))
    }

    /// Buttons for raw option values shown through `label`.
    fn choice_group(
        id: &'static str,
        options: &'static [&'static str],
        current: &str,
        label: fn(&App, &str) -> &'static str,
        cx: &mut Context<Self>,
        apply: fn(&mut AiSettings, &'static str),
    ) -> ButtonGroup {
        ButtonGroup::new(id)
            .outline()
            .xsmall()
            .children(options.iter().enumerate().map(|(ix, option)| {
                Button::new((id, ix))
                    .label(label(cx, option))
                    .selected(current == *option)
            }))
            .on_click(cx.listener(move |this, clicked: &Vec<usize>, window, cx| {
                if let Some(option) = clicked.first().and_then(|ix| options.get(*ix)) {
                    let option = *option;
                    this.update(window, cx, move |ai| apply(ai, option));
                }
            }))
    }

    /// "Advanced" disclosure toggle shaped like the shell's settings screen.
    fn advanced_toggle(
        id: &'static str,
        open: bool,
        cx: &mut Context<Self>,
        toggle: fn(&mut Self),
    ) -> Button {
        Button::new(id)
            .ghost()
            .small()
            .icon(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .label(localized(cx, "Advanced", "Дополнительно"))
            .on_click(cx.listener(move |this, _, _, cx| {
                toggle(this);
                cx.notify();
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
        ui::card(
            localized(cx, "AI", "ИИ"),
            Some(localized(
                cx,
                "Used for aligning words, drafting text and filling segments.",
                "Используется для выравнивания слов, черновиков и заполнения сегментов.",
            )),
            cx,
        )
        .child(Self::labeled(
            localized(cx, "Runs on", "Где выполняется"),
            Self::location_group("ai-location", ai.execution_location, cx, |ai, location| {
                ai.execution_location = location
            }),
            cx,
        ))
        .child(Self::advanced_toggle(
            "ai-remote-advanced",
            self.advanced_remote,
            cx,
            |this| this.advanced_remote = !this.advanced_remote,
        ))
        .when(self.advanced_remote, |card| {
            card.child(
                v_flex()
                    .gap_2()
                    .child(ui::muted(
                        localized(cx, "Remote server", "Удалённый сервер"),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.remote_url)))
                            .child(
                                Button::new("ai-remote-save")
                                    .outline()
                                    .label(localized(cx, "Save", "Сохранить"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_remote_url(window, cx)
                                    })),
                            ),
                    )
                    .when_some(self.remote_url_error.clone(), |this, error| {
                        this.child(ui::error_block(error, None, cx))
                    })
                    .child(Input::new(&self.remote_key).mask_toggle()),
            )
        })
    }

    /// Remote server status line, its tone, and technical text for a "Details" disclosure.
    fn remote_status_line(&self, cx: &App) -> (Tone, String, Option<SharedString>) {
        match &self.remote_status {
            None => (
                Tone::Info,
                localized(
                    cx,
                    "Remote server: checking…",
                    "Удалённый сервер: проверка…",
                )
                .to_string(),
                None,
            ),
            Some(BackendStatus::Ready { operations, .. }) => {
                let features = [
                    (
                        operations.timing,
                        localized(cx, "Align words", "Выровнять слова"),
                    ),
                    (
                        operations.draft,
                        localized(cx, "Draft with AI", "Черновик с ИИ"),
                    ),
                    (operations.enhance, localized(cx, "Denoise", "Убрать шум")),
                    (
                        operations.align,
                        localized(cx, "Align segments", "Выровнять сегменты"),
                    ),
                ];
                let unavailable = features
                    .iter()
                    .filter(|(ok, _)| !ok)
                    .map(|(_, name)| *name)
                    .collect::<Vec<_>>();
                let detail = (!unavailable.is_empty()).then(|| {
                    localized(cx, "Unavailable: {list}", "Недоступно: {list}")
                        .replace("{list}", &unavailable.join(", "))
                        .into()
                });
                (
                    if operations.timing && operations.draft {
                        Tone::Success
                    } else {
                        Tone::Warning
                    },
                    localized(cx, "Remote server: ready", "Удалённый сервер: готов").to_string(),
                    detail,
                )
            }
            Some(BackendStatus::NeedsModels { .. }) => (
                Tone::Warning,
                localized(cx, "Remote server: not ready", "Удалённый сервер: не готов").to_string(),
                None,
            ),
            Some(BackendStatus::Unavailable { message, .. }) => (
                Tone::Danger,
                localized(
                    cx,
                    "Remote server: {message}",
                    "Удалённый сервер: {message}",
                )
                .replace("{message}", message),
                None,
            ),
        }
    }

    fn render_models(&self, cx: &mut Context<Self>) -> Div {
        let local = &self.local;
        let busy = local.op.is_some();
        let theme = cx.theme().clone();
        // One size for the card and the status line: the backend's download total.
        let download_size = match &local.status {
            Some(Ok(status)) => {
                let bytes = status.download_bytes.unwrap_or(status.models.total_bytes);
                (bytes > 0).then(|| byte_count(bytes, cx))
            }
            _ => None,
        };
        let description = match &download_size {
            Some(size) => localized(
                cx,
                "About {size}, downloaded once. Needs a graphics card that supports WebGPU.",
                "Около {size}, загружаются один раз. Нужна видеокарта с поддержкой WebGPU.",
            )
            .replace("{size}", size),
            None => localized(
                cx,
                "Downloaded once. Needs a graphics card that supports WebGPU.",
                "Загружаются один раз. Нужна видеокарта с поддержкой WebGPU.",
            )
            .to_string(),
        };
        let mut card = ui::card(
            localized(cx, "Models on this computer", "Модели на этом компьютере"),
            Some(&description),
            cx,
        );
        // Summary line, tone, and technical text (model error, graphics reason) for "Details".
        let (summary, tone, detail) = match &local.status {
            None => (
                localized(cx, "Checking…", "Проверка…").to_string(),
                Tone::Info,
                None,
            ),
            Some(Err(error)) => (error.clone(), Tone::Danger, None),
            Some(Ok(status)) => {
                let models = &status.models;
                let state = match models.state {
                    ModelsState::Ready if models.outdated => {
                        localized(cx, "Update available", "Доступно обновление").to_string()
                    }
                    ModelsState::Ready if models.tested => {
                        localized(cx, "Ready", "Готово").to_string()
                    }
                    ModelsState::Ready => localized(
                        cx,
                        "Installed — run the test",
                        "Установлено — запустите проверку",
                    )
                    .to_string(),
                    ModelsState::Downloading => {
                        localized(cx, "Downloading…", "Загрузка…").to_string()
                    }
                    ModelsState::NotInstalled => match &download_size {
                        Some(size) => {
                            localized(cx, "Not installed ({size})", "Не установлено ({size})")
                                .replace("{size}", size)
                        }
                        None => localized(cx, "Not installed", "Не установлено").to_string(),
                    },
                    ModelsState::Stopped => {
                        localized(cx, "Download paused", "Загрузка остановлена").to_string()
                    }
                    ModelsState::Error => localized(cx, "Problem", "Ошибка").to_string(),
                };
                let graphics_ok = status.webgpu.available && status.webgpu.shader_f16;
                let gpu = if graphics_ok {
                    status
                        .webgpu
                        .adapter
                        .as_ref()
                        .map(|adapter| {
                            localized(
                                cx,
                                "Graphics: {vendor} {arch}",
                                "Видеокарта: {vendor} {arch}",
                            )
                            .replace("{vendor}", &adapter.vendor)
                            .replace("{arch}", &adapter.architecture)
                            .trim()
                            .to_string()
                        })
                        .unwrap_or_else(|| {
                            localized(cx, "Graphics: ready", "Видеокарта: готова").to_string()
                        })
                } else {
                    localized(
                        cx,
                        "This graphics card can't run the models",
                        "Эта видеокарта не может запускать модели",
                    )
                    .to_string()
                };
                let mut detail = Vec::new();
                if models.state == ModelsState::Error {
                    detail.extend(models.error.clone());
                }
                if !graphics_ok {
                    detail.extend(status.webgpu.reason.clone());
                }
                let tone = if status.ready {
                    Tone::Success
                } else if graphics_ok {
                    Tone::Warning
                } else {
                    Tone::Danger
                };
                (
                    format!("{state} · {gpu}"),
                    tone,
                    (!detail.is_empty()).then(|| detail.join("\n").into()),
                )
            }
        };
        card = card
            .child(ui::banner(
                tone,
                localized(cx, "This computer", "Этот компьютер"),
                Some(summary.into()),
                None,
                cx,
            ))
            .when_some(detail, |card, detail| card.child(ui::details(detail, cx)));
        if busy {
            let (line, fraction) = match &local.progress {
                Some(bar) => (label(bar.action, bar.fraction, &bar.step), bar.fraction),
                None => (
                    match local.op {
                        Some(ModelOp::Install) => {
                            localized(cx, "Downloading models…", "Загрузка моделей…")
                        }
                        Some(ModelOp::Test) => localized(cx, "Testing…", "Проверка…"),
                        Some(ModelOp::Remove) => {
                            localized(cx, "Removing models…", "Удаление моделей…")
                        }
                        None => localized(cx, "Working…", "Выполняется…"),
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
            card = card.child(if *error {
                ui::error_block(notice.clone(), None, cx)
            } else {
                div()
                    .text_xs()
                    .text_color(theme.success)
                    .child(notice.clone())
            });
        }
        if let Some(sample) = &local.sample {
            card = card.child(ui::muted(
                localized(cx, "Test transcript: {text}", "Расшифровка теста: {text}")
                    .replace("{text}", &sample.text),
                cx,
            ));
        }
        let installed = matches!(
            &local.status,
            Some(Ok(status)) if status.models.state == ModelsState::Ready
        );
        let (remote_tone, remote_line, remote_detail) = self.remote_status_line(cx);
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
                            localized(cx, "Update", "Обновить")
                        } else {
                            localized(cx, "Download", "Скачать")
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
                            .label(localized(cx, "Cancel", "Отмена"))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_model_op(cx))),
                    )
                })
                .child(
                    Button::new("ai-models-test")
                        .outline()
                        .small()
                        .label(localized(cx, "Run test", "Запустить проверку"))
                        .loading(local.op == Some(ModelOp::Test))
                        .disabled(busy || !installed)
                        .on_click(cx.listener(|this, _, _, cx| this.test_models(cx))),
                )
                .child(
                    Button::new("ai-models-remove")
                        .outline()
                        .small()
                        .label(localized(cx, "Delete models", "Удалить модели"))
                        .disabled(busy || !installed)
                        .on_click(cx.listener(|this, _, _, cx| this.remove_models(cx))),
                )
                .child(
                    Button::new("ai-models-refresh")
                        .ghost()
                        .small()
                        .label(localized(cx, "Check again", "Проверить снова"))
                        .loading(local.checking || self.remote_task.is_some())
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_local(cx);
                            this.refresh_remote(cx);
                        })),
                ),
        )
        .child(div().text_xs().text_color(remote_color).child(remote_line))
        .when_some(remote_detail, |card, detail| {
            card.child(ui::details(detail, cx))
        })
    }

    fn render_gold(&self, ai: &AiSettings, cx: &mut Context<Self>) -> Div {
        let checking = self.health_task.is_some();
        let key_hint = key_hint(&ai.gold_openrouter_key, cx);
        let mut card = ui::card(
            localized(cx, "Polish", "Доработка текста"),
            Some(localized(
                cx,
                "After drafting, an online model tidies the text. Needs an OpenRouter key.",
                "После черновика онлайн-модель приводит текст в порядок. Нужен ключ OpenRouter.",
            )),
            cx,
        )
        .child(Self::labeled(
            localized(cx, "Polish drafts", "Дорабатывать черновики"),
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
                        .label(localized(cx, "Test key", "Проверить ключ"))
                        .loading(checking)
                        .disabled(checking)
                        .on_click(cx.listener(|this, _, _, cx| this.check_backend(cx))),
                ),
        )
        .when_some(key_hint, |this, hint| {
            this.child(div().text_xs().text_color(cx.theme().warning).child(hint))
        });
        match &self.health {
            Some(Ok(health)) => {
                let mut details = vec![health.service.clone()];
                details.extend(health.backend_version.clone());
                details.extend(health.openrouter_status.clone());
                if health.test_mode == Some(true) {
                    details.push("test mode".to_string());
                }
                details.extend(health.provider_diagnostics.iter().cloned());
                card = card
                    .child(ui::banner(
                        Tone::Success,
                        localized(cx, "Key works", "Ключ работает"),
                        None,
                        None,
                        cx,
                    ))
                    .child(ui::details(details.join(" · ").into(), cx));
            }
            Some(Err(error)) => {
                card = card.child(ui::error_block(
                    localized(cx, "Key doesn't work", "Ключ не работает"),
                    Some(error.clone().into()),
                    cx,
                ));
            }
            None => {}
        }
        card.child(Self::advanced_toggle(
            "ai-polish-advanced",
            self.advanced_polish,
            cx,
            |this| this.advanced_polish = !this.advanced_polish,
        ))
        .when(self.advanced_polish, |card| {
            card.child(Self::labeled(
                localized(cx, "Model", "Модель"),
                div().min_w_64().child(Input::new(&self.gold_model)),
                cx,
            ))
            .child(Self::labeled(
                localized(cx, "Priority", "Приоритет"),
                Self::choice_group(
                    "ai-tier",
                    &SERVICE_TIERS,
                    &ai.gold_service_tier,
                    tier_label,
                    cx,
                    |ai, tier| ai.gold_service_tier = tier.to_string(),
                ),
                cx,
            ))
            .child(Self::labeled(
                localized(cx, "Thinking", "Размышление"),
                Self::choice_group(
                    "ai-effort",
                    &REASONING_EFFORTS,
                    &ai.gold_reasoning_effort,
                    effort_label,
                    cx,
                    |ai, effort| ai.gold_reasoning_effort = effort.to_string(),
                ),
                cx,
            ))
        })
    }

    fn render_denoise(&self, ai: &AiSettings, cx: &mut Context<Self>) -> Div {
        ui::card(
            localized(cx, "Denoise", "Шум"),
            Some(localized(
                cx,
                "Removes background noise. You can mix the original and cleaned audio in the editor.",
                "Убирает фоновый шум. В редакторе можно смешивать исходный и очищенный звук.",
            )),
            cx,
        )
        .child(Self::labeled(
            localized(cx, "Runs on", "Где выполняется"),
            Self::location_group(
                "ai-denoise-location",
                ai.denoise_location,
                cx,
                |ai, location| ai.denoise_location = location,
            ),
            cx,
        ))
        .child(Self::labeled(
            localized(
                cx,
                "Denoise automatically when a project opens",
                "Убирать шум автоматически при открытии проекта",
            ),
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
fn key_hint(key: &str, cx: &App) -> Option<&'static str> {
    if key.is_empty() {
        return None;
    }
    let valid =
        key.starts_with("sk-or-") && key.len() <= 512 && !key.chars().any(char::is_whitespace);
    (!valid).then(|| localized(cx, "Keys start with sk-or-.", "Ключ начинается с sk-or-."))
}

impl Render for AiSettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.unloaded {
            return v_flex().child(localized(
                cx,
                "The AI extension is turned off. Turn it on in Extensions to change these settings.",
                "Дополнение ИИ выключено. Включите его в Расширениях.",
            ));
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
