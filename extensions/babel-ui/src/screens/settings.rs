//! Settings › Babel: the sub-account relay host and where Babel sign-ins are stored.
//! Accounts themselves (sign in / sign out / add another) live in Settings › Accounts.
use exaudio_babel_services::{account, relay::RelayStatus};
use exaudio_shell::ShellHandle;
use exaudio_shell::screens::ui;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ID;
use crate::root;
use crate::screens::ShellModels;
use crate::text::{localized, localized_fmt};

/// Env override of the providers root; a portable move is refused while it is set.
const PROVIDER_DIR_ENV: &str = "STUDIO_PROVIDER_DIR";

pub struct SettingsView {
    shell: ShellHandle,
    relay_url: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(shell: ShellHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let relay = ShellModels::relay(cx);
        let relay_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "Address", "Адрес"))
                .default_value(relay.read(cx).host_url().to_string())
        });
        let subscriptions = vec![
            cx.subscribe_in(&relay, window, |_, _, event, window, cx| {
                // A live Babel dashboard already reports relay events once.
                if root::controller_for_mode(ID, cx).is_some() {
                    return;
                }
                let notification = match event {
                    crate::screens::relay::RelayEvent::Info(message) => {
                        Notification::info(message.clone())
                    }
                    crate::screens::relay::RelayEvent::Error { headline, detail } => {
                        ui::error_notification(headline.clone(), detail.clone())
                    }
                };
                window.push_notification(notification, cx);
            }),
            cx.subscribe_in(&relay_url, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.save_relay_url(window, cx);
                }
            }),
            cx.observe_in(&relay, window, |this, relay, window, cx| {
                let host = relay.read(cx).host_url().to_string();
                if this.relay_url.read(cx).value() != host.as_str() {
                    this.relay_url
                        .update(cx, |input, cx| input.set_value(host, window, cx));
                }
                cx.notify();
            }),
        ];
        Self {
            shell,
            relay_url,
            _subscriptions: subscriptions,
        }
    }

    fn save_relay_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.relay_url.read(cx).value().to_string();
        let relay = ShellModels::relay(cx);
        let note = match relay.update(cx, |relay, cx| relay.remember_host_url(&value, cx)) {
            Ok(url) => {
                // The owner's dashboard (if open) pushes its project queues to the new host.
                if let Some(controller) = root::controller_for_mode(ID, cx)
                    && let Some(auth_store) = controller.read(cx).owner_auth_store()
                    && let Some(dashboard) = controller.read(cx).dashboard()
                {
                    let projects = dashboard.read(cx).projects().to_vec();
                    relay.update(cx, |relay, cx| {
                        relay.sync_project_queues(&projects, auth_store, cx)
                    });
                }
                Notification::success(localized_fmt(
                    cx,
                    "Relay address saved: {url}",
                    "Адрес ретранслятора сохранён: {url}",
                    &[("url", &url)],
                ))
            }
            Err(error) => ui::error_notification(
                localized(
                    cx,
                    "Couldn't save the relay address.",
                    "Не удалось сохранить адрес ретранслятора.",
                ),
                Some(error.into()),
            ),
        };
        window.push_notification(note, cx);
    }

    /// Portable install: moves the whole providers root next to the executable. Every open
    /// Babel workspace is dropped (its client pointed at the old directories) and the shell
    /// returns to Start.
    fn move_sign_in_next_to_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = portable_target().and_then(|target| {
            if let Ok(value) = std::env::var(PROVIDER_DIR_ENV)
                && !value.trim().is_empty()
            {
                anyhow::bail!(
                    "{PROVIDER_DIR_ENV} is set to {}; unset it before moving the sign-in",
                    value.trim()
                );
            }
            let source = exaudio_shell::providers::providers_root();
            account::move_providers_root(&source, &target)?;
            Ok(target)
        });
        match result {
            Ok(path) => {
                root::reset_controllers(cx);
                window.push_notification(
                    Notification::success(path.display().to_string()).title(localized(
                        cx,
                        "Sign-ins now live next to the app",
                        "Входы теперь хранятся рядом с приложением",
                    )),
                    cx,
                );
                let shell = self.shell.clone();
                window.defer(cx, move |window, cx| shell.show_home(window, cx));
            }
            Err(error) => window.push_notification(
                ui::error_notification(
                    localized(
                        cx,
                        "Couldn't move the sign-ins.",
                        "Не удалось перенести входы.",
                    ),
                    Some(format!("{error:#}").into()),
                ),
                cx,
            ),
        }
    }

    fn render_relay(&self, cx: &mut Context<Self>) -> Div {
        let relay = ShellModels::relay(cx);
        let relay = relay.read(cx);
        let (can_start, can_stop, status) = (relay.can_start(), relay.can_stop(), relay.status());
        let label = relay.status_label(cx);
        let color = match status {
            RelayStatus::Running => cx.theme().success,
            RelayStatus::Failed => cx.theme().danger,
            RelayStatus::Starting => cx.theme().warning,
            RelayStatus::Stopped => cx.theme().muted_foreground,
        };
        ui::card(
            localized(cx, "Sub-account relay", "Ретранслятор для под-аккаунтов"),
            Some(localized(
                cx,
                "Lets sub-accounts sign in through this computer.",
                "Позволяет под-аккаунтам входить через этот компьютер.",
            )),
            cx,
        )
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&self.relay_url)))
                .child(
                    Button::new("save-relay-url")
                        .outline()
                        .label(localized(cx, "Save", "Сохранить"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.save_relay_url(window, cx)),
                        ),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("start-relay")
                        .primary()
                        .small()
                        .label(localized(cx, "Start", "Запустить"))
                        .loading(status == RelayStatus::Starting)
                        .disabled(!can_start)
                        .on_click(|_, _, cx| {
                            ShellModels::relay(cx).update(cx, |relay, cx| relay.start(cx))
                        }),
                )
                .child(
                    Button::new("stop-relay")
                        .outline()
                        .small()
                        .label(localized(cx, "Stop", "Остановить"))
                        .disabled(!can_stop)
                        .on_click(|_, _, cx| {
                            ShellModels::relay(cx).update(cx, |relay, cx| relay.stop(cx))
                        }),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .text_xs()
                        .child(div().size_2().rounded_full().bg(color))
                        .child(label),
                ),
        )
    }

    fn render_sign_in(&self, cx: &mut Context<Self>) -> Div {
        let current = exaudio_shell::providers::providers_root();
        let portable = portable_target().ok();
        let already_portable = portable.as_ref() == Some(&current);
        ui::card(
            localized(cx, "Sign-in", "Вход"),
            Some(localized(
                cx,
                "Keep your sign-ins with the app folder so they move with the app.",
                "Хранить входы рядом с приложением, чтобы они переносились вместе с ним.",
            )),
            cx,
        )
        .child(ui::detail_row(
            localized(cx, "Stored in", "Расположение"),
            current.display().to_string(),
            cx,
        ))
        .when_some(portable.filter(|_| !already_portable), |this, path| {
            this.child(ui::detail_row(
                localized(cx, "Next to the app", "Рядом с приложением"),
                path.display().to_string(),
                cx,
            ))
        })
        .child(
            h_flex().flex_wrap().justify_end().child(
                Button::new("move-sign-in")
                    .outline()
                    .small()
                    .label(localized(
                        cx,
                        "Store sign-ins next to the app",
                        "Хранить рядом с приложением",
                    ))
                    .disabled(already_portable)
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.move_sign_in_next_to_app(window, cx)
                        }),
                    ),
            ),
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(self.render_relay(cx))
            .child(self.render_sign_in(cx))
    }
}

/// `<executable dir>/providers`: where [`exaudio_shell::providers::providers_root`] looks first.
fn portable_target() -> anyhow::Result<std::path::PathBuf> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| anyhow::anyhow!("the executable has no parent directory"))?;
    Ok(directory.join("providers"))
}
