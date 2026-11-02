//! Small shared building blocks for the non-editor screens (cards, banners, labels).

use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

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
