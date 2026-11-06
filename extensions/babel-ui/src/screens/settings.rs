use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use sonus_babel_services::{auth, relay::RelayStatus};
use sonus_shell::ShellHandle;
use sonus_shell::screens::ui;

use crate::root::{self, BabelMode};
use crate::screens::{ShellModels, local_files};

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
                .placeholder("Relay URL, e.g. http://127.0.0.1:8787")
                .default_value(relay.read(cx).host_url().to_string())
        });
        let subscriptions = vec![
            cx.subscribe_in(&relay, window, |_, _, event, window, cx| {
                if root::controller(BabelMode::Direct, cx).is_some() {
                    return;
                }
                let notification = match event {
                    crate::screens::relay::RelayEvent::Info(message) => {
                        Notification::info(message.clone())
                    }
                    crate::screens::relay::RelayEvent::Error(message) => {
                        Notification::error(message.clone())
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
                if let Some(controller) = root::controller(BabelMode::Direct, cx)
                    && let Some(dashboard) = controller.read(cx).dashboard()
                {
                    let projects = dashboard.read(cx).projects().to_vec();
                    relay.update(cx, |relay, cx| relay.sync_project_queues(&projects, cx));
                }
                Notification::success(format!("Relay URL saved: {url}"))
            }
            Err(error) => Notification::error(format!("Relay URL not saved: {error}")),
        };
        window.push_notification(note, cx);
    }

    fn detach_store(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match auth::detach_store_to_executable() {
            Ok(path) => {
                window.push_notification(
                    Notification::success(format!(
                        "Babel sign-in now lives next to the app: {}",
                        path.display()
                    )),
                    cx,
                );
                if let Some(controller) = root::controller(BabelMode::Direct, cx) {
                    window.defer(cx, move |window, cx| {
                        controller
                            .update(cx, |controller, cx| controller.open_auth_gate(window, cx));
                    });
                }
                cx.notify();
            }
            Err(error) => window.push_notification(
                Notification::error(format!(
                    "Could not make this installation portable: {error:#}"
                )),
                cx,
            ),
        }
    }

    fn open_babel_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        local_files::open_document(self.shell.clone(), window, cx);
    }

    fn render_relay(&self, cx: &mut Context<Self>) -> Div {
        let relay = ShellModels::relay(cx);
        let relay = relay.read(cx);
        let (can_start, can_stop, status) = (relay.can_start(), relay.can_stop(), relay.status());
        let label = relay.status_label();
        let color = match status {
            RelayStatus::Running => cx.theme().success,
            RelayStatus::Failed => cx.theme().danger,
            RelayStatus::Starting => cx.theme().warning,
            RelayStatus::Stopped => cx.theme().muted_foreground,
        };
        ui::card(
            "Sub-account relay",
            Some("Run the bundled relay on this machine and remember its URL for sub-accounts."),
            cx,
        )
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&self.relay_url)))
                .child(
                    Button::new("save-relay-url")
                        .outline()
                        .label("Save URL")
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
                        .label("Start relay")
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
                        .label("Stop relay")
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

    fn render_session(&self, cx: &mut Context<Self>) -> Div {
        let store = auth::store_path();
        let portable = std::env::current_exe()
            .ok()
            .and_then(|exe| auth::store_path_next_to_executable(&exe).ok());
        ui::card("Session & portability", Some("Keep your Babel sign-in with this installation when you move it to another device."), cx)
            .child(ui::detail_row("Sign-in stored at", store.display().to_string(), cx))
            .when_some(portable.filter(|path| *path != store), |this, path| {
                this.child(ui::detail_row("Portable location", path.display().to_string(), cx))
            })
            .child(h_flex().flex_wrap().justify_end()
                .child(Button::new("detach-store").outline().small().label("Make installation portable")
                    .on_click(cx.listener(|this, _, window, cx| this.detach_store(window, cx)))))
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(self.render_relay(cx))
            .child(self.render_session(cx))
            .child(
                ui::card(
                    "Babel task files",
                    Some("Open a Babel JSON export without claiming or submitting a remote task."),
                    cx,
                )
                .child(
                    Button::new("open-babel-json")
                        .outline()
                        .label("Open Babel JSON…")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_babel_file(window, cx)),
                        ),
                ),
            )
    }
}
