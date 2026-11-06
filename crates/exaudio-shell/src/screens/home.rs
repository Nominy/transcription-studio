//! Start: one tile per installed mode, "Get more" → Extensions, footer Settings · Extensions.
//! Local is a tile like any other; its launch window owns recent projects and recovery.
use super::{accounts, ui};
use crate::{ContributionRegistry, ProviderRegistry, ShellHandle};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::sync::{Arc, LazyLock};

/// Start heading art, parsed once; gpui rasterizes SVGs and caches them by content hash.
static ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../../assets/brand/exaudio-icon.svg").to_vec(),
    ))
});
static WORDMARK_LIGHT: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../../assets/brand/exaudio-wordmark.svg").to_vec(),
    ))
});
static WORDMARK_DARK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../../assets/brand/exaudio-wordmark-dark.svg").to_vec(),
    ))
});

/// Width-to-height ratio of `exaudio-wordmark*.svg` (printed by `tools/brand/render.py`).
const WORDMARK_ASPECT: f32 = 4.6264;

/// Icon tile beside the wordmark, ink on light and paper on dark, 28 px tall.
fn heading(cx: &App) -> Div {
    let wordmark = if cx.theme().is_dark() {
        WORDMARK_DARK.clone()
    } else {
        WORDMARK_LIGHT.clone()
    };
    h_flex()
        .gap_3()
        .items_center()
        .child(img(ICON.clone()).size(px(28.)).flex_none())
        .child(
            img(wordmark)
                .h(px(28.))
                .w(px(28. * WORDMARK_ASPECT))
                .flex_none(),
        )
}

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
    let registry = cx.global::<ContributionRegistry>();
    let mut modes = registry.modes();
    // Local first, then other modes that open without an account, then the rest by label.
    modes.sort_by_cached_key(|mode| {
        let needs_provider = registry
            .mode_manifest(&mode.id)
            .is_some_and(|manifest| manifest.provider.is_some());
        (
            mode.id != "exaudio.local",
            needs_provider,
            mode.label.to_lowercase(),
        )
    });
    let settings = shell.clone();
    let extensions = shell.clone();
    let more = shell.clone();
    v_flex()
        .size_full()
        .id("exaudio-home")
        .overflow_y_scroll()
        .p_6()
        .gap_6()
        .child(heading(cx))
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
                                .label(ui::localized(cx, "Get more", "Ещё"))
                                .on_click(move |_, window, cx| more.show_extensions(window, cx)),
                        )
                        .child(ui::muted(
                            ui::localized(
                                cx,
                                "Extensions from the catalog",
                                "Расширения из каталога",
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
                        .label(ui::localized(cx, "Extensions", "Расширения"))
                        .on_click(move |_, window, cx| extensions.show_extensions(window, cx)),
                ),
        )
        .children(super::updates::update_notice(cx))
}
