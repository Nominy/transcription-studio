//! Start: one tile per installed mode, "Get more" → Add-ons, footer Settings · Add-ons.
//! Local is a tile like any other; its launch window owns recent projects and recovery.
use super::{accounts, ui};
use crate::{ContributionRegistry, ProviderRegistry, ShellHandle};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Provider instance chip shown on a tile only when its provider type has several accounts.
fn account_chip(mode_id: &str, cx: &App) -> Option<(String, String)> {
    let provider = cx
        .global::<ContributionRegistry>()
        .mode_manifest(mode_id)?
        .provider
        .clone()?;
    let registry = cx.global::<ProviderRegistry>();
    if registry.instances(&provider).len() < 2 {
        return None;
    }
    let label = registry
        .selected(&provider)
        .map(|instance| instance.label)
        .unwrap_or_else(|| ui::localized(cx, "Choose account", "Выбрать аккаунт").into());
    Some((provider, label))
}

fn tile(cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .w(px(260.))
        .min_h(px(140.))
        .gap_2()
        .p_4()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.group_box)
}

pub fn render(shell: ShellHandle, _window: &mut Window, cx: &App) -> impl IntoElement + use<> {
    let modes = cx.global::<ContributionRegistry>().modes();
    let settings = shell.clone();
    let extensions = shell.clone();
    let more = shell.clone();
    v_flex()
        .size_full()
        .id("sonus-home")
        .overflow_y_scroll()
        .p_6()
        .gap_6()
        .child(div().text_2xl().child("Exaudio"))
        .child(
            h_flex()
                .gap_4()
                .flex_wrap()
                .items_start()
                .children(modes.into_iter().map(|mode| {
                    let handle = shell.clone();
                    let id = mode.id.clone();
                    let chip = account_chip(&mode.id, cx);
                    tile(cx)
                        .child(
                            Button::new(SharedString::from(format!("mode-{}", mode.id)))
                                .primary()
                                .label(mode.label.clone())
                                .on_click(move |_, window, cx| handle.show_mode(&id, window, cx)),
                        )
                        .child(ui::muted(mode.description.clone(), cx))
                        .when_some(chip, |this, (provider, label)| {
                            this.child(
                                Button::new(SharedString::from(format!("account-{}", mode.id)))
                                    .outline()
                                    .small()
                                    .label(label)
                                    .tooltip(ui::localized(cx, "Switch account", "Сменить аккаунт"))
                                    .on_click(move |_, window, cx| {
                                        accounts::open_switch_dialog(&provider, window, cx)
                                    }),
                            )
                        })
                }))
                .child(
                    tile(cx)
                        .child(
                            Button::new("home-get-more")
                                .outline()
                                .label(ui::localized(cx, "Get more", "Добавить ещё"))
                                .on_click(move |_, window, cx| more.show_extensions(window, cx)),
                        )
                        .child(ui::muted(
                            ui::localized(
                                cx,
                                "Workspaces and tools from the add-on catalog.",
                                "Рабочие пространства и инструменты из каталога дополнений.",
                            ),
                            cx,
                        )),
                ),
        )
        .child(div().flex_1())
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("home-settings")
                        .label(ui::localized(cx, "Settings", "Настройки"))
                        .on_click(move |_, window, cx| settings.show_settings(window, cx)),
                )
                .child(
                    Button::new("home-extensions")
                        .label(ui::localized(cx, "Add-ons", "Дополнения"))
                        .on_click(move |_, window, cx| extensions.show_extensions(window, cx)),
                ),
        )
        .children(super::updates::update_notice(cx))
}
