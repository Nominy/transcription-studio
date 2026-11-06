//! Shared appearance, shortcuts and updates, followed by registered extension settings.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, Theme, ThemeMode, WindowExt as _,
    h_flex, v_flex,
};
use gpui_kit::*;
use studio_services::prefs::EditorLayout;
use studio_services::updater::UpdateChannel;

use crate::editor::panels::dialogs::hotkeys;
use crate::screens::ShellModels;
use crate::screens::ui;
use crate::services::AppServices;
use crate::{ContributionRegistry, PanelLocation, ShellHandle};

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

pub struct SettingsView {
    shell: ShellHandle,
    layout: EditorLayout,
    contributions: Vec<(String, u64, AnyView)>,
    registry_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(shell: ShellHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let layout = cx
            .global::<AppServices>()
            .storage
            .preferences()
            .editor_layout;
        let panels = cx
            .global::<ContributionRegistry>()
            .panels(PanelLocation::Settings);
        let contributions = panels
            .into_iter()
            .map(|panel| {
                let view = (panel.factory)(shell.clone(), window, cx);
                (panel.id, panel.generation, view)
            })
            .collect();
        let subscriptions = vec![cx.observe(&ShellModels::updater(cx), |_, _, cx| cx.notify())];
        Self {
            shell,
            layout,
            contributions,
            registry_generation: cx.global::<ContributionRegistry>().generation(),
            _subscriptions: subscriptions,
        }
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

    fn render_updates(&self, cx: &mut Context<Self>) -> Div {
        let updater = ShellModels::updater(cx);
        let state = updater.read(cx).state().clone();
        let status_color = if state.is_error() {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };
        ui::card(
            "Updates",
            Some("Sonus checks for updates on launch and every few hours."),
            cx,
        )
        .child(ui::detail_row("Version", state.current_version.clone(), cx))
        .child(ui::detail_row("Build", state.build_commit.clone(), cx))
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .justify_between()
                .child(ui::muted("Channel", cx))
                .child(
                    ButtonGroup::new("update-channel")
                        .outline()
                        .small()
                        .disabled(!state.can_switch_channel())
                        .children(UpdateChannel::ALL.iter().enumerate().map(|(ix, channel)| {
                            Button::new(("update-channel", ix))
                                .label(channel.label())
                                .selected(state.channel == *channel)
                        }))
                        .on_click(|clicked: &Vec<usize>, window, cx| {
                            let Some(channel) =
                                clicked.first().and_then(|ix| UpdateChannel::ALL.get(*ix))
                            else {
                                return;
                            };
                            let saved = ShellModels::updater(cx)
                                .update(cx, |updater, cx| updater.set_channel(*channel, cx));
                            if let Err(error) = saved {
                                window.push_notification(
                                    Notification::warning(format!(
                                        "Update channel could not be saved: {error}"
                                    )),
                                    cx,
                                );
                            }
                        }),
                ),
        )
        .child(ui::muted(state.channel.description(), cx))
        .child(ui::detail_row("Source", state.channel.source_url(), cx))
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
                            ShellModels::updater(cx).update(cx, |updater, cx| updater.install(cx))
                        }),
                ),
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let generation = cx.global::<ContributionRegistry>().generation();
        if self.registry_generation != generation {
            let panels = cx
                .global::<ContributionRegistry>()
                .panels(PanelLocation::Settings);
            self.contributions.retain(|(id, generation, _)| {
                panels
                    .iter()
                    .any(|p| &p.id == id && p.generation == *generation)
            });
            for panel in panels {
                if !self.contributions.iter().any(|(id, _, _)| id == &panel.id) {
                    let view = (panel.factory)(self.shell.clone(), window, cx);
                    self.contributions.push((panel.id, panel.generation, view));
                }
            }
            self.registry_generation = generation;
        }
        v_flex()
            .size_full()
            .id("shared-settings")
            .overflow_y_scroll()
            .p_4()
            .gap_4()
            .child(
                Button::new("settings-close")
                    .label(ui::localized(cx, "Close", "Закрыть"))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(super::home::language_picker(cx))
            .child(self.render_appearance(cx))
            .children(self.contributions.iter().map(|(_, _, view)| view.clone()))
            .child(self.render_updates(cx))
    }
}
