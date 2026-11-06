//! Shared keyboard-focus and pointer isolation for extension editor overlays.
use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::*;
use std::rc::Rc;

/// Small outlined status label used by extension overlays.
pub fn badge(text: &str, color: Hsla, cx: &App) -> Div {
    div()
        .flex_none()
        .px_1p5()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(color)
        .text_xs()
        .text_color(color)
        .child(text.to_string())
}

pub fn dialog_surface(id: &'static str, width: Rems, accent: Hsla, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    v_flex()
        .id(id)
        .occlude()
        .w(width)
        .max_w_full()
        .h_full()
        .rounded(theme.radius_lg)
        .border_t_4()
        .border_color(accent)
        .bg(theme.background)
        .shadow_lg()
        .overflow_hidden()
}

pub type CloseModal = Rc<dyn Fn(&mut Window, &mut App)>;

pub fn modal(
    surface: Stateful<Div>,
    focus: &ModalFocus,
    close: CloseModal,
    close_on_backdrop: bool,
) -> Stateful<Div> {
    let backdrop = div()
        .id("modal-backdrop")
        .absolute()
        .inset_0()
        .bg(Hsla::from(rgba(0x0f0a_196b)));
    let backdrop = if close_on_backdrop {
        let close = close.clone();
        backdrop.on_click(move |_, window, cx| close(window, cx))
    } else {
        backdrop
    };
    div()
        .id("modal")
        .absolute()
        .inset_0()
        .occlude()
        .track_focus(&focus.handle)
        .on_key_down(move |event, window, cx| {
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                close(window, cx);
            }
        })
        .child(backdrop)
        .child(
            div()
                .absolute()
                .inset_0()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .child(surface),
        )
}

pub struct ModalFocus {
    handle: FocusHandle,
    restore: Option<FocusHandle>,
    open: bool,
}

impl ModalFocus {
    pub fn new(cx: &mut App) -> Self {
        Self {
            handle: cx.focus_handle(),
            restore: None,
            open: false,
        }
    }

    pub fn sync(&mut self, open: bool, window: &mut Window, cx: &mut App) {
        if open == self.open {
            return;
        }
        self.open = open;
        if open {
            self.restore = window.focused(cx);
            self.handle.focus(window, cx);
        } else if let Some(restore) = self.restore.take()
            && (window.focused(cx).is_none() || self.handle.contains_focused(window, cx))
        {
            restore.focus(window, cx);
        }
    }
}
