//! App-lifetime updater: channel choice, background checks (on launch, then at most every
//! [`updater::CHECK_INTERVAL`]), install with download progress, and the unobtrusive
//! "update available" notice.

use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_services::updater::{self, UpdateChannel, UpdateState};

use crate::screens::ShellModels;
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

/// Small "update available" card with Install / Skip, or `None` when there is nothing to
/// offer. Placed in the dashboard sidebar so it never covers the editor.
pub fn update_notice(cx: &App) -> Option<Div> {
    let model = ShellModels::updater(cx);
    let updater = model.read(cx);
    let version = updater.notice_version()?.to_string();
    let state = updater.state();
    let theme = cx.theme();
    let detail = match state.phase {
        updater::UpdatePhase::Installing | updater::UpdatePhase::Failed => state.status_label(),
        _ => format!("{} channel", state.channel.label()),
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
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("Exaudio {version} is available")),
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
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("update-notice-install")
                            .primary()
                            .xsmall()
                            .label("Install")
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
                                .label("Skip")
                                .on_click(|_, window, cx| {
                                    let saved = ShellModels::updater(cx)
                                        .update(cx, |updater, cx| updater.skip_offered_version(cx));
                                    if let Err(error) = saved {
                                        window.push_notification(
                                            Notification::warning(format!(
                                                "Skipped version could not be saved: {error}"
                                            )),
                                            cx,
                                        );
                                    }
                                }),
                        )
                    }),
            ),
    )
}
