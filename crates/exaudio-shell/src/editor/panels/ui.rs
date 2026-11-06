//! Small building blocks shared by the editor panels and dialogs.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Dialog heading: title plus optional description.
pub fn dialog_header(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    cx: &App,
) -> Div {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .when_some(description, |this, description| {
            this.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(description),
            )
        })
}

/// Right-aligned dialog button row.
pub fn dialog_footer() -> Div {
    h_flex().pt_2().gap_2().justify_end()
}

/// A labelled form field.
pub fn field(label: impl Into<SharedString>, control: impl IntoElement, cx: &App) -> Div {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(control)
}

/// Inline error text under a field or in a dialog.
pub fn error_text(message: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().danger)
        .child(message.into())
}

/// Secondary text.
pub fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// Uppercase caption above a group of controls.
pub fn caption(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(text.into().to_uppercase())
}

/// Thin vertical divider between toolbar clusters.
pub fn vertical_rule(cx: &App) -> Div {
    div()
        .h(rems(1.25))
        .w(px(1.))
        .flex_none()
        .bg(cx.theme().border)
}

/// Font features with tabular (fixed-width) figures, so a changing number keeps its width.
pub fn tabular_figures() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

/// `text` laid out at the width of the widest of itself and `variants` (the other values the
/// label shows), so changing the value never moves its neighbours. The variants are laid out
/// invisibly at zero height under the shown text. The shown text is centred in the slot;
/// `items_start`/`items_end` anchor it to an edge instead. (Text alignment cannot do this:
/// unwrapped text ignores `text_align`.)
pub fn stable_text(
    text: impl Into<SharedString>,
    variants: impl IntoIterator<Item = impl Into<SharedString>>,
) -> Div {
    v_flex()
        .items_center()
        .whitespace_nowrap()
        .child(div().child(text.into()))
        .children(variants.into_iter().map(|variant| {
            div()
                .h_0()
                .overflow_hidden()
                .invisible()
                .child(variant.into())
        }))
}

/// One option of a [`choice`] dropdown.
pub struct Choice<T> {
    pub value: T,
    pub label: SharedString,
}

impl<T> Choice<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}

/// A compact select: an outlined button showing the current choice that opens a menu of
/// options. `on_select` receives the chosen value.
pub fn choice<T: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    selected: Option<&T>,
    options: Vec<Choice<T>>,
    placeholder: &'static str,
    on_select: impl Fn(T, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = selected
        .and_then(|value| options.iter().find(|option| &option.value == value))
        .map_or_else(
            || SharedString::from(placeholder),
            |option| option.label.clone(),
        );
    // Like a native select, the button is as wide as its widest option.
    let widths = stable_text(
        label.clone(),
        options
            .iter()
            .map(|option| option.label.clone())
            .chain([SharedString::from(placeholder)]),
    );
    let selected = selected.cloned();
    let options = Rc::new(options);
    let on_select = Rc::new(on_select);
    Button::new(id)
        .outline()
        .small()
        .child(widths)
        .accessibility_label(label)
        .dropdown_caret(true)
        .disabled(options.is_empty())
        .dropdown_menu(move |mut menu, window, _| {
            for option in options.iter() {
                let value = option.value.clone();
                let on_select = on_select.clone();
                menu = menu.item(
                    PopupMenuItem::new(option.label.clone())
                        .checked(selected.as_ref() == Some(&option.value))
                        .on_click(move |_, window, cx| on_select(value.clone(), window, cx)),
                );
            }
            menu.max_h(px(360. * crate::zoom::ui_scale(window)))
                .scrollable(true)
        })
        .into_any_element()
}

/// `primary` button that shows a loading spinner while `pending`.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    pending: bool,
    disabled: bool,
) -> Button {
    Button::new(id)
        .primary()
        .label(label)
        .loading(pending)
        .disabled(disabled || pending)
}
