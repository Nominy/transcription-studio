//! Small shared building blocks for the non-editor screens (cards, banners, labels).

use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// UI storage key of the chosen language (`"en"` / `"ru"`); absent until the user picks one.
pub const LANGUAGE_STORAGE_KEY: &str = "exaudio.language";

/// Language of the OS session, read once: `"ru"` for Russian locales, otherwise `"en"`.
static OS_LANGUAGE: std::sync::LazyLock<&'static str> = std::sync::LazyLock::new(|| {
    if sys_locale::get_locale().is_some_and(|locale| locale.starts_with("ru")) {
        "ru"
    } else {
        "en"
    }
});

/// The active UI language id: the saved choice, or the OS locale before one is saved.
pub fn language(cx: &App) -> &'static str {
    match cx
        .global::<crate::services::AppServices>()
        .storage
        .value(LANGUAGE_STORAGE_KEY)
    {
        Some("en") => "en",
        Some("ru") => "ru",
        _ => *OS_LANGUAGE,
    }
}

/// Pushes the active language into crates that produce user-visible text without `cx`.
pub fn sync_language(cx: &App) {
    studio_editor::i18n::set_russian(language(cx) == "ru");
}

pub fn localized(cx: &App, en: &'static str, ru: &'static str) -> &'static str {
    if language(cx) == "en" { en } else { ru }
}

/// `"{n} {noun}"` with a real plural: EN `one`/`other`; RU `one` (1, 21…), `few` (2–4),
/// `many` (5–20, 0).
pub fn count(cx: &App, n: usize, en: (&str, &str), ru: (&str, &str, &str)) -> String {
    let noun = if language(cx) == "en" {
        if n == 1 { en.0 } else { en.1 }
    } else {
        match (n % 10, n % 100) {
            (1, m) if m != 11 => ru.0,
            (2..=4, m) if !(12..=14).contains(&m) => ru.1,
            _ => ru.2,
        }
    };
    format!("{n} {noun}")
}

/// The one error shape: a plain headline and, when there is technical text, a
/// "Details" disclosure that shows it verbatim. Headlines never carry identifiers, paths or
/// `Debug` output; those go in `detail`.
pub fn error_block(
    headline: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let detail = detail.filter(|detail| !detail.trim().is_empty());
    h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .child(
            Icon::new(IconName::CircleAlert)
                .small()
                .text_color(theme.danger)
                .mt_0p5(),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_sm().child(headline.into()))
                .when_some(detail, |this, detail| this.child(details(detail, cx))),
        )
}

/// Collapsed "Details" disclosure with selectable monospace text.
pub fn details(detail: SharedString, cx: &App) -> Div {
    let theme = cx.theme();
    div().w_full().child(
        Accordion::new("error-details")
            .bordered(false)
            .xsmall()
            .item(|item| {
                item.title(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(localized(cx, "Details", "Подробности")),
                )
                .child(
                    div()
                        .id("error-details-text")
                        .text_xs()
                        .font_family("monospace")
                        .text_color(theme.muted_foreground)
                        .whitespace_normal()
                        .child(detail),
                )
            }),
    )
}

/// Error toast shaped like [`error_block`]: `headline` is the message, `detail` opens
/// under "Details".
pub fn error_notification(
    headline: impl Into<SharedString>,
    detail: Option<SharedString>,
) -> Notification {
    let headline = headline.into();
    let detail = detail.filter(|detail| !detail.trim().is_empty());
    match detail {
        None => Notification::error(headline),
        Some(detail) => Notification::error(headline)
            .content(move |_, _, cx| details(detail.clone(), cx).into_any_element()),
    }
}

/// Warning toast with the same shape as [`error_notification`].
pub fn warning_notification(
    headline: impl Into<SharedString>,
    detail: Option<SharedString>,
) -> Notification {
    let headline = headline.into();
    let detail = detail.filter(|detail| !detail.trim().is_empty());
    match detail {
        None => Notification::warning(headline),
        Some(detail) => Notification::warning(headline)
            .content(move |_, _, cx| details(detail.clone(), cx).into_any_element()),
    }
}

/// Error toast for an `anyhow` chain: the outermost message is the headline (callers put
/// the plain sentence there), the full chain goes under Details.
pub fn anyhow_notification(error: &anyhow::Error) -> Notification {
    let headline = error.to_string();
    let chain = format!("{error:#}");
    let detail = (chain != headline).then(|| SharedString::from(chain));
    error_notification(headline, detail)
}

/// "Couldn't open this workspace." with the registry/activation error under Details.
pub fn workspace_open_failed(error: &anyhow::Error, cx: &App) -> Notification {
    error_notification(
        localized(
            cx,
            "Couldn't open this workspace.",
            "Не удалось открыть это пространство.",
        ),
        Some(format!("{error:#}").into()),
    )
}

/// A titled settings/content card.
pub fn card(title: impl Into<SharedString>, description: Option<&str>, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .w_full()
        .gap_3()
        .p_4()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.group_box)
        .child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                )
                .when_some(description, |this, description| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(SharedString::from(description.to_string())),
                    )
                }),
        )
}

/// A label + value line inside a card.
pub fn detail_row(label: &'static str, value: impl Into<SharedString>, cx: &App) -> Div {
    h_flex()
        .gap_3()
        .text_xs()
        .child(
            div()
                .w(rems(7.5))
                .flex_none()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().min_w_0().child(value.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Warning,
    Danger,
}

/// Inline banner with a leading icon, a title, an optional body and an optional trailing
/// action element.
pub fn banner(
    tone: Tone,
    title: impl Into<SharedString>,
    body: Option<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let (color, icon) = match tone {
        Tone::Info => (theme.info, IconName::Info),
        Tone::Success => (theme.success, IconName::CircleCheck),
        Tone::Warning => (theme.warning, IconName::TriangleAlert),
        Tone::Danger => (theme.danger, IconName::CircleAlert),
    };
    h_flex()
        .w_full()
        .gap_3()
        .px_4()
        .py_3()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(color.opacity(0.4))
        .bg(color.opacity(0.08))
        .child(Icon::new(icon).small().text_color(color))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.into()),
                )
                .when_some(body, |this, body| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(body),
                    )
                }),
        )
        .when_some(action, |this, action| this.child(action))
}

/// Uppercase caption above a group of controls.
pub fn caption(text: &'static str, cx: &App) -> Div {
    div()
        .px_2()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// Secondary text.
pub fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}
