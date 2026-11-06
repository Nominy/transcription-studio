//! General (language), appearance, shortcuts and updates, then registered extension
//! settings, then an Advanced disclosure holding provider accounts.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, Theme, ThemeConfig, ThemeMode,
    ThemeRegistry, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;
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
const LANGUAGES: [(&str, &str); 2] = [("en", "English"), ("ru", "Русский")];

/// Brand accent (amber) and the ink it carries text in; the same on both modes.
const ACCENT: &str = "#D99A2B";
const ACCENT_HOVER: &str = "#E2A942";
const ACCENT_ACTIVE: &str = "#C2881F";
const ACCENT_FOREGROUND: &str = "#14161C";

/// gpui-kit's default theme with the brand accent in place of its neutral primary.
fn branded(config: &Rc<ThemeConfig>) -> Rc<ThemeConfig> {
    let mut config = ThemeConfig::clone(config);
    let colors = &mut config.colors;
    colors.primary = Some(ACCENT.into());
    colors.primary_hover = Some(ACCENT_HOVER.into());
    colors.primary_active = Some(ACCENT_ACTIVE.into());
    colors.primary_foreground = Some(ACCENT_FOREGROUND.into());
    colors.ring = Some(ACCENT.into());
    colors.progress_bar = Some(ACCENT.into());
    colors.slider_bar = Some(ACCENT.into());
    colors.sidebar_primary = Some(ACCENT.into());
    colors.sidebar_primary_foreground = Some(ACCENT_FOREGROUND.into());
    Rc::new(config)
}

/// Installs the branded light and dark themes, then applies the persisted mode (dark by
/// default). Call once at startup.
pub fn apply_saved_theme(cx: &mut App) {
    let mode = match cx.global::<AppServices>().storage.value(THEME_STORAGE_KEY) {
        Some("light") => ThemeMode::Light,
        _ => ThemeMode::Dark,
    };
    let registry = ThemeRegistry::global(cx);
    let light = branded(registry.default_light_theme());
    let dark = branded(registry.default_dark_theme());
    Theme::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark;
    });
    Theme::change(mode, None, cx);
}

pub struct SettingsView {
    shell: ShellHandle,
    layout: EditorLayout,
    contributions: Vec<(String, u64, AnyView)>,
    registry_generation: u64,
    advanced: bool,
    accounts: Vec<super::accounts::AccountSection>,
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
            advanced: false,
            accounts: Vec::new(),
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

    fn set_language(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = cx
            .global_mut::<AppServices>()
            .storage
            .set_value("sonus.language", id.into())
        {
            window.push_notification(Notification::error(error.to_string()), cx);
        }
        cx.refresh_windows();
    }

    fn render_general(&self, cx: &mut Context<Self>) -> Div {
        let choice = cx
            .global::<AppServices>()
            .storage
            .value("sonus.language")
            .unwrap_or("ru")
            .to_owned();
        ui::card(ui::localized(cx, "General", "Общие"), None, cx).child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .justify_between()
                .child(ui::muted(ui::localized(cx, "Language", "Язык"), cx))
                .child(
                    ButtonGroup::new("language")
                        .outline()
                        .small()
                        .children(LANGUAGES.iter().map(|(id, label)| {
                            Button::new(*id).label(*label).selected(choice == *id)
                        }))
                        .on_click(cx.listener(|this, clicked: &Vec<usize>, window, cx| {
                            if let Some((id, _)) = clicked.first().and_then(|ix| LANGUAGES.get(*ix))
                            {
                                this.set_language(id, window, cx);
                            }
                        })),
                ),
        )
    }

    fn render_advanced(&self, cx: &mut Context<Self>) -> Div {
        let open = self.advanced;
        v_flex()
            .gap_3()
            .child(
                Button::new("settings-advanced")
                    .ghost()
                    .small()
                    .icon(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .label(ui::localized(cx, "Advanced", "Дополнительно"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        if this.advanced {
                            this.accounts = super::accounts::sections(cx);
                        }
                        cx.notify();
                    })),
            )
            .when(open, |this| {
                let view = cx.entity().downgrade();
                let on_change: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
                    view.update(cx, |this, cx| {
                        this.accounts = super::accounts::sections(cx);
                        cx.notify();
                    })
                    .ok();
                });
                this.child(super::accounts::accounts_card(
                    self.shell.clone(),
                    &self.accounts,
                    on_change,
                    cx,
                ))
            })
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
            Some("Exaudio checks for updates on launch and every few hours."),
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
            .child(self.render_general(cx))
            .child(self.render_appearance(cx))
            .children(self.contributions.iter().map(|(_, _, view)| view.clone()))
            .child(self.render_updates(cx))
            .child(self.render_advanced(cx))
    }
}
