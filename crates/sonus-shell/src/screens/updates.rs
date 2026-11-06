//! App-lifetime updater: channel choice, background checks (on launch, then at most every
//! [`updater::CHECK_INTERVAL`]), install with download progress, and the unobtrusive
//! "update available" notice.

use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_services::updater::{self, UpdateChannel, UpdateState};

use crate::screens::{ShellModels, ui};
use crate::services::{AppServices, spawn_service};

/// This binary's version (`STUDIO_BUILD_VERSION` from packaging, else the crate version).
pub const APP_VERSION: &str = match option_env!("STUDIO_BUILD_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// UI storage key of the chosen update channel (`"stable"` / `"alpha"`).
const CHANNEL_STORAGE_KEY: &str = "studio.updates.channel.v1";
/// UI storage key of the version the user chose to skip in the update notice.
const SKIPPED_VERSION_STORAGE_KEY: &str = "studio.updates.skipped-version.v1";

pub struct UpdaterModel {
    state: UpdateState,
    skipped_version: Option<String>,
    /// Start of the latest check, manual or background; paces background checks.
    last_check: Option<Instant>,
    task: Option<Task<()>>,
    _progress: Option<Task<()>>,
    _schedule: Option<Task<()>>,
}

impl UpdaterModel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let storage = &cx.global::<AppServices>().storage;
        let channel = updater::resolve_channel(storage.value(CHANNEL_STORAGE_KEY));
        let skipped_version = storage
            .value(SKIPPED_VERSION_STORAGE_KEY)
            .map(str::to_string);
        let state = UpdateState::new(APP_VERSION, channel, updater::is_installed());
        let schedule = state
            .is_supported()
            .then(|| cx.spawn(async move |this, cx| Self::run_schedule(this, cx).await));
        Self {
            state,
            skipped_version,
            last_check: None,
            task: None,
            _progress: None,
            _schedule: schedule,
        }
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    /// Checks right away, then whenever [`updater::CHECK_INTERVAL`] has passed since the
    /// latest check; never blocks the UI.
    async fn run_schedule(this: WeakEntity<Self>, cx: &mut AsyncApp) {
        loop {
            let Ok(delay) = this.update(cx, |this, _| {
                updater::next_check_delay(this.last_check, Instant::now())
            }) else {
                return;
            };
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
                continue;
            }
            if this.update(cx, |this, cx| this.check(cx)).is_err() {
                return;
            }
        }
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        self.last_check = Some(Instant::now());
        if !self.state.begin_check() {
            return;
        }
        let channel = self.state.channel;
        self.task = Some(spawn_service(
            cx,
            updater::check_for_updates(channel),
            |this, result, cx| {
                this.task = None;
                this.state
                    .finish_check(result.map_err(|error| format!("{error:#}")));
                cx.notify();
            },
        ));
        cx.notify();
    }

    pub fn install(&mut self, cx: &mut Context<Self>) {
        if !self.state.begin_install() {
            return;
        }
        let channel = self.state.channel;
        let (progress, reports) = async_channel::unbounded::<u8>();
        self._progress = Some(cx.spawn(async move |this, cx| {
            while let Ok(percent) = reports.recv().await {
                let applied = this.update(cx, |this, cx| {
                    this.state.set_download_percent(percent);
                    cx.notify();
                });
                if applied.is_err() {
                    return;
                }
            }
        }));
        self.task = Some(spawn_service(
            cx,
            updater::install_available_update(channel, move |percent| {
                progress.try_send(percent).ok();
            }),
            |this, result, cx| {
                this.task = None;
                this.state
                    .finish_install(result.map_err(|error| format!("{error:#}")));
                cx.notify();
            },
        ));
        cx.notify();
    }

    /// Persists `channel` and checks it right away. Returns the storage error, if any.
    pub fn set_channel(
        &mut self,
        channel: UpdateChannel,
        cx: &mut Context<Self>,
    ) -> std::io::Result<()> {
        if !self.state.switch_channel(channel) {
            return Ok(());
        }
        let saved = cx
            .global_mut::<AppServices>()
            .storage
            .set_value(CHANNEL_STORAGE_KEY, channel.as_str().to_string());
        self.check(cx);
        saved
    }

    /// Hides the notice for the offered version; Settings still offers it.
    pub fn skip_offered_version(&mut self, cx: &mut Context<Self>) -> std::io::Result<()> {
        let Some(version) = self.state.available_version.clone() else {
            return Ok(());
        };
        let saved = cx
            .global_mut::<AppServices>()
            .storage
            .set_value(SKIPPED_VERSION_STORAGE_KEY, version.clone());
        self.skipped_version = Some(version);
        cx.notify();
        saved
    }

    fn notice_version(&self) -> Option<&str> {
        self.state.notice_version(self.skipped_version.as_deref())
    }
}

/// Channel name shown in Settings and the notice.
pub fn channel_label(channel: UpdateChannel, cx: &App) -> &'static str {
    match channel {
        UpdateChannel::Stable => ui::localized(cx, "Stable", "Стабильный"),
        UpdateChannel::Alpha => ui::localized(cx, "Early access", "Ранний доступ"),
    }
}

/// One line on what the channel delivers.
pub fn channel_description(channel: UpdateChannel, cx: &App) -> &'static str {
    match channel {
        UpdateChannel::Stable => ui::localized(
            cx,
            "Tested releases published on GitHub.",
            "Проверенные выпуски, опубликованные на GitHub.",
        ),
        UpdateChannel::Alpha => ui::localized(
            cx,
            "Every build of the main branch: newest features first.",
            "Каждая сборка основной ветки: новые функции раньше всех.",
        ),
    }
}

/// The updater's current state in one sentence; the failure detail is returned separately
/// so the caller can show it under "Details".
pub fn status_text(state: &UpdateState, cx: &App) -> (String, Option<SharedString>) {
    let channel = channel_label(state.channel, cx);
    let text = match state.phase {
        updater::UpdatePhase::Unsupported => ui::localized(
            cx,
            "Updates work in installed builds only.",
            "Обновления работают только в установленных сборках.",
        )
        .to_string(),
        updater::UpdatePhase::Idle => ui::localized(cx, "Channel: {channel}", "Канал: {channel}")
            .replace("{channel}", channel),
        updater::UpdatePhase::Checking => {
            ui::localized(cx, "Checking for updates…", "Проверка обновлений…").to_string()
        }
        updater::UpdatePhase::UpdateAvailable => match state.available_version.as_ref() {
            Some(version) => ui::localized(
                cx,
                "Exaudio {version} is ready to install",
                "Доступна версия Exaudio {version}",
            )
            .replace("{version}", version),
            None => ui::localized(cx, "An update is ready to install", "Доступно обновление")
                .to_string(),
        },
        updater::UpdatePhase::UpToDate => {
            ui::localized(cx, "Up to date", "Установлена последняя версия").to_string()
        }
        updater::UpdatePhase::RemoteEmpty => ui::localized(
            cx,
            "No releases for this platform yet",
            "Для этой платформы пока нет выпусков",
        )
        .to_string(),
        updater::UpdatePhase::Installing => match state.download_percent {
            Some(percent) => ui::localized(cx, "Downloading… {p}%", "Загрузка… {p}%")
                .replace("{p}", &percent.to_string()),
            None => ui::localized(cx, "Downloading…", "Загрузка…").to_string(),
        },
        updater::UpdatePhase::Restarting => {
            ui::localized(cx, "Installed. Restarting…", "Установлено. Перезапуск…").to_string()
        }
        updater::UpdatePhase::Failed => {
            let text = ui::localized(cx, "Couldn't update.", "Не удалось обновить.").to_string();
            return (text, state.last_error.clone().map(SharedString::from));
        }
    };
    (text, None)
}

/// Small "update available" card with Update / Not now, or `None` when there is nothing to
/// offer. Placed on Start so it never covers the editor.
pub fn update_notice(cx: &App) -> Option<Div> {
    let model = ShellModels::updater(cx);
    let updater = model.read(cx);
    let version = updater.notice_version()?.to_string();
    let state = updater.state();
    let theme = cx.theme();
    let (detail, detail_text) = match state.phase {
        updater::UpdatePhase::Installing | updater::UpdatePhase::Failed => status_text(state, cx),
        _ => (channel_label(state.channel, cx).to_string(), None),
    };
    Some(
        v_flex()
            .w_full()
            .gap_1p5()
            .p_2p5()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                div().text_sm().font_weight(FontWeight::MEDIUM).child(
                    ui::localized(
                        cx,
                        "Exaudio {version} is ready to install",
                        "Доступна версия Exaudio {version}",
                    )
                    .replace("{version}", &version),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if state.is_error() {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(detail),
            )
            .when_some(detail_text, |this, detail| {
                this.child(ui::details(detail, cx))
            })
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("update-notice-install")
                            .primary()
                            .xsmall()
                            .label(ui::localized(cx, "Update", "Обновить"))
                            .disabled(!state.can_install())
                            .on_click(|_, _, cx| {
                                ShellModels::updater(cx)
                                    .update(cx, |updater, cx| updater.install(cx))
                            }),
                    )
                    .when(state.can_install(), |this| {
                        this.child(
                            Button::new("update-notice-skip")
                                .ghost()
                                .xsmall()
                                .label(ui::localized(cx, "Not now", "Не сейчас"))
                                .on_click(|_, window, cx| {
                                    let saved = ShellModels::updater(cx)
                                        .update(cx, |updater, cx| updater.skip_offered_version(cx));
                                    if let Err(error) = saved {
                                        window.push_notification(
                                            ui::warning_notification(
                                                ui::localized(
                                                    cx,
                                                    "Couldn't remember the skipped version.",
                                                    "Не удалось запомнить пропущенную версию.",
                                                ),
                                                Some(error.to_string().into()),
                                            ),
                                            cx,
                                        );
                                    }
                                }),
                        )
                    }),
            ),
    )
}
