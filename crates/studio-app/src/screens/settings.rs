//! Settings: appearance (theme, editor layout), keyboard shortcuts, drafting backend, relay,
//! session portability, updates and local task files.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, Theme, ThemeMode, WindowExt as _,
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_services::auth;
use studio_services::prefs::EditorLayout;
use studio_services::relay::RelayStatus;
use studio_services::review::BackendHealth;

use crate::editor::panels::dialogs::hotkeys;
use crate::root::StudioRoot;
use crate::screens::ShellModels;
use crate::screens::local_files::{self, LocalSource};
use crate::screens::ui::{self, Tone};
use crate::services::{AppServices, spawn_service};

/// UI storage key of the persisted theme (`"light"` / `"dark"`).
pub const THEME_STORAGE_KEY: &str = "studio.theme.v1";

const LAYOUTS: [(EditorLayout, &str); 3] = [
    (EditorLayout::Classic, "Classic table"),
    (EditorLayout::SplitStacked, "Split stacked"),
    (EditorLayout::SplitSideBySide, "Split side by side"),
];

/// Applies the persisted theme (dark by default). Call once at startup.
pub fn apply_saved_theme(cx: &mut App) {
    let mode = match cx.global::<AppServices>().storage.value(THEME_STORAGE_KEY) {
        Some("light") => ThemeMode::Light,
        _ => ThemeMode::Dark,
    };
    Theme::change(mode, None, cx);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DraftField {
    ApiKey,
    Model,
    Tier,
    Effort,
}

pub struct SettingsView {
    root: WeakEntity<StudioRoot>,
    layout: EditorLayout,
    api_key: Entity<InputState>,
    model: Entity<InputState>,
    tier: Entity<InputState>,
    effort: Entity<InputState>,
    relay_url: Entity<InputState>,
    health: Option<Result<BackendHealth, String>>,
    health_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(root: WeakEntity<StudioRoot>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let drafting = cx.global::<AppServices>().drafting.clone();
        let layout = cx
            .global::<AppServices>()
            .storage
            .preferences()
            .editor_layout;
        let input = |placeholder: &'static str,
                     value: String,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let api_key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("OpenRouter API key (falls back to OPENROUTER_API_KEY)")
                .masked(true)
                .default_value(drafting.openrouter_api_key.clone())
        });
        let model = input("Model (backend default)", drafting.model, window, cx);
        let tier = input("Service tier", drafting.service_tier, window, cx);
        let effort = input("Reasoning effort", drafting.reasoning_effort, window, cx);
        let relay = ShellModels::relay(cx);
        let relay_url = input(
            "Relay URL, e.g. http://127.0.0.1:8787",
            relay.read(cx).host_url().to_string(),
            window,
            cx,
        );

        let mut subscriptions = vec![
            Self::watch_draft(&api_key, DraftField::ApiKey, window, cx),
            Self::watch_draft(&model, DraftField::Model, window, cx),
            Self::watch_draft(&tier, DraftField::Tier, window, cx),
            Self::watch_draft(&effort, DraftField::Effort, window, cx),
            cx.subscribe_in(&relay_url, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.save_relay_url(window, cx);
                }
            }),
            cx.observe_in(&relay, window, |this, relay, window, cx| {
                let host_url = relay.read(cx).host_url().to_string();
                if this.relay_url.read(cx).value() != host_url.as_str() {
                    this.relay_url
                        .update(cx, |input, cx| input.set_value(host_url, window, cx));
                }
                cx.notify();
            }),
        ];
        subscriptions.push(cx.observe(&ShellModels::updater(cx), |_, _, cx| cx.notify()));

        Self {
            root,
            layout,
            api_key,
            model,
            tier,
            effort,
            relay_url,
            health: None,
            health_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn watch_draft(
        input: &Entity<InputState>,
        field: DraftField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(input, window, move |_, input, event, _, cx| {
            if let InputEvent::Change = event {
                let value = input.read(cx).value().trim().to_string();
                let drafting = &mut cx.global_mut::<AppServices>().drafting;
                match field {
                    DraftField::ApiKey => drafting.openrouter_api_key = value,
                    DraftField::Model => drafting.model = value,
                    DraftField::Tier => drafting.service_tier = value,
                    DraftField::Effort => drafting.reasoning_effort = value,
                }
            }
        })
    }

    fn set_theme(&mut self, mode: ThemeMode, window: &mut Window, cx: &mut Context<Self>) {
        Theme::change(mode, Some(window), cx);
        crate::zoom::reapply(cx);
        let value = if mode.is_dark() { "dark" } else { "light" };
        if let Err(error) = cx
            .global_mut::<AppServices>()
            .storage
            .set_value(THEME_STORAGE_KEY, value.to_string())
        {
            window.push_notification(
                Notification::warning(format!("Theme could not be saved: {error}")),
                cx,
            );
        }
        cx.refresh_windows();
    }

    fn set_layout(&mut self, layout: EditorLayout, window: &mut Window, cx: &mut Context<Self>) {
        self.layout = layout;
        let storage = &mut cx.global_mut::<AppServices>().storage;
        let mut preferences = storage.preferences();
        preferences.editor_layout = layout;
        if let Err(error) = storage.set_preferences(&preferences) {
            window.push_notification(
                Notification::warning(format!("Layout could not be saved: {error}")),
                cx,
            );
        }
        cx.notify();
    }

    fn check_backend(&mut self, cx: &mut Context<Self>) {
        let review = cx.global::<AppServices>().review.clone();
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

    fn save_relay_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.relay_url.read(cx).value().to_string();
        let relay = ShellModels::relay(cx);
        let note = match relay.update(cx, |relay, cx| relay.remember_host_url(&value, cx)) {
            Ok(url) => {
                if let Some(projects) = self.dashboard_projects(cx) {
                    relay.update(cx, |relay, cx| relay.sync_project_queues(&projects, cx));
                }
                Notification::success(format!("Relay URL saved: {url}"))
            }
            Err(error) => Notification::error(format!("Relay URL not saved: {error}")),
        };
        window.push_notification(note, cx);
    }

    fn dashboard_projects(
        &self,
        cx: &App,
    ) -> Option<Vec<studio_services::babel::DashboardProject>> {
        let root = self.root.upgrade()?;
        let dashboard = root.read(cx).dashboard()?;
        Some(dashboard.read(cx).projects().to_vec())
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
                self.root
                    .update(cx, |root, cx| root.open_auth_gate(window, cx))
                    .ok();
            }
            Err(error) => window.push_notification(
                Notification::error(format!(
                    "Could not make this installation portable: {error:#}"
                )),
                cx,
            ),
        }
    }

    fn open_local(&mut self, source: LocalSource, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.clone();
        local_files::open(source, window, cx, move |result, window, cx| match result {
            Ok(task) => {
                let label = format!("Opened {}.", task.path.display());
                root.update(cx, |root, cx| {
                    let model =
                        root.open_task(task.workflow, Some(task.audio_base_dir), window, cx);
                    model.update(cx, |model, cx| model.set_status(label, cx));
                })
                .ok();
            }
            Err(error) => {
                window.push_notification(Notification::error(format!("Open failed: {error}")), cx)
            }
        });
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> Div {
        let dark = cx.theme().is_dark();
        ui::card(
            "Appearance",
            Some("Theme and the transcript layout used when a task opens."),
            cx,
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .justify_between()
                .child(ui::muted("Theme", cx))
                .child(
                    ButtonGroup::new("theme")
                        .outline()
                        .small()
                        .child(
                            Button::new("theme-light")
                                .icon(IconName::Sun)
                                .label("Light")
                                .selected(!dark),
                        )
                        .child(
                            Button::new("theme-dark")
                                .icon(IconName::Moon)
                                .label("Dark")
                                .selected(dark),
                        )
                        .on_click(cx.listener(|this, clicked: &Vec<usize>, window, cx| {
                            let mode = if clicked.contains(&0) {
                                ThemeMode::Light
                            } else {
                                ThemeMode::Dark
                            };
                            this.set_theme(mode, window, cx);
                        })),
                ),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .justify_between()
                .child(ui::muted("Interface size (Ctrl + / Ctrl − / Ctrl 0)", cx))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("zoom-out")
                                .outline()
                                .small()
                                .icon(IconName::Minus)
                                .on_click(|_, _, cx| crate::zoom::step(-1, cx)),
                        )
                        .child(
                            Button::new("zoom-reset")
                                .outline()
                                .small()
                                .label(format!("{:.0}%", crate::zoom::level(cx) * 100.))
                                .on_click(|_, _, cx| crate::zoom::set(1.0, cx)),
                        )
                        .child(
                            Button::new("zoom-in")
                                .outline()
                                .small()
                                .icon(IconName::Plus)
                                .on_click(|_, _, cx| crate::zoom::step(1, cx)),
                        ),
                ),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .justify_between()
                .child(ui::muted("Editor layout", cx))
                .child(
                    ButtonGroup::new("layout")
                        .outline()
                        .small()
                        .children(LAYOUTS.iter().enumerate().map(|(ix, (layout, label))| {
                            Button::new(("layout", ix))
                                .label(*label)
                                .selected(self.layout == *layout)
                        }))
                        .on_click(cx.listener(|this, clicked: &Vec<usize>, window, cx| {
                            if let Some((layout, _)) =
                                clicked.first().and_then(|ix| LAYOUTS.get(*ix))
                            {
                                this.set_layout(*layout, window, cx);
                            }
                        })),
                ),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .justify_between()
                .child(ui::muted("Keyboard shortcuts", cx))
                .child(
                    Button::new("hotkeys")
                        .outline()
                        .small()
                        .label("Customize shortcuts…")
                        .on_click(|_, window, cx| hotkeys::open(window, cx)),
                ),
        )
    }

    fn render_drafting(&self, cx: &mut Context<Self>) -> Div {
        let checking = self.health_task.is_some();
        let mut card = ui::card(
            "Drafting backend",
            Some("Reviewer appears on L2 review tasks; drafting uses these backend settings."),
            cx,
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .child(Input::new(&self.api_key).mask_toggle()),
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
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&self.model)))
                .child(div().w(rems(9.375)).child(Input::new(&self.tier)))
                .child(div().w(rems(9.375)).child(Input::new(&self.effort))),
        );
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

    fn render_relay(&self, cx: &mut Context<Self>) -> Div {
        let relay = ShellModels::relay(cx);
        let relay = relay.read(cx);
        let (can_start, can_stop, status) = (relay.can_start(), relay.can_stop(), relay.status());
        let label = relay.status_label();
        let status_color = match status {
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
                        .child(div().size_2().rounded_full().bg(status_color))
                        .child(label),
                ),
        )
    }

    fn render_session(&self, cx: &mut Context<Self>) -> Div {
        let store = auth::store_path();
        let portable = std::env::current_exe()
            .ok()
            .and_then(|exe| auth::store_path_next_to_executable(&exe).ok());
        ui::card(
            "Session & portability",
            Some("Keep your Babel sign-in with this installation when you move it to another device."),
            cx,
        )
        .child(ui::detail_row("Sign-in stored at", store.display().to_string(), cx))
        .when_some(portable.filter(|path| *path != store), |this, path| {
            this.child(ui::detail_row("Portable location", path.display().to_string(), cx))
        })
        .child(
            h_flex().flex_wrap().justify_end().child(
                Button::new("detach-store")
                    .outline()
                    .small()
                    .label("Make installation portable")
                    .on_click(cx.listener(|this, _, window, cx| this.detach_store(window, cx))),
            ),
        )
    }

    fn render_updates(&self, cx: &mut Context<Self>) -> Div {
        let updater = ShellModels::updater(cx);
        let state = updater.read(cx).state().clone();
        let status_color = if state.is_error() {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };
        ui::card("Updates", Some("Early Access channel"), cx)
            .child(ui::detail_row("Version", state.current_version.clone(), cx))
            .child(ui::detail_row("Build", state.build_commit.clone(), cx))
            .when(cfg!(debug_assertions), |this| {
                this.child(ui::detail_row("Update feed", state.feed_url.clone(), cx))
            })
            // Status on its own wrapping line: a long updater error must never push the
            // buttons out of the card, at any window width or zoom level.
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_xs()
                    .text_color(status_color)
                    .child(state.status_label()),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("check-updates")
                            .outline()
                            .small()
                            .label("Check for updates")
                            .disabled(!state.can_check())
                            .on_click(|_, _, cx| {
                                ShellModels::updater(cx).update(cx, |updater, cx| updater.check(cx))
                            }),
                    )
                    .child(
                        Button::new("install-update")
                            .primary()
                            .small()
                            .label("Install update")
                            .disabled(!state.can_install())
                            .on_click(|_, _, cx| {
                                ShellModels::updater(cx)
                                    .update(cx, |updater, cx| updater.install(cx))
                            }),
                    ),
            )
    }

    fn render_local_files(&self, cx: &mut Context<Self>) -> Div {
        ui::card(
            "Local task files",
            Some("Open a Babel JSON export or a Studio project folder for offline work. Export the open task from the editor."),
            cx,
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("open-json")
                        .outline()
                        .small()
                        .icon(IconName::FileText)
                        .label("Open Babel JSON…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_local(LocalSource::BabelJson, window, cx)
                        })),
                )
                .child(
                    Button::new("open-folder")
                        .outline()
                        .small()
                        .icon(IconName::FolderOpen)
                        .label("Open project folder…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_local(LocalSource::ProjectFolder, window, cx)
                        })),
                ),
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(self.render_appearance(cx))
            .child(self.render_local_files(cx))
            .child(self.render_drafting(cx))
            .child(self.render_relay(cx))
            .child(self.render_session(cx))
            .child(self.render_updates(cx))
    }
}
