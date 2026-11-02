//! "No work available" modal shown after claiming the next task found nothing.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{WindowExt, v_flex};
use gpui_kit::*;

use crate::editor::panels::ui::{dialog_footer, dialog_header};

/// Opens the non-dismissible no-work modal; "Back to Projects" closes it and calls `on_back`.
/// `job_kind` fills "Sorry, we weren't able to find a {job_kind} job for you!".
pub fn open(
    job_kind: &str,
    window: &mut Window,
    cx: &mut App,
    on_back: impl Fn(&mut Window, &mut App) + 'static,
) {
    let title: SharedString =
        format!("Sorry, we weren't able to find a {job_kind} job for you!").into();
    let on_back = Rc::new(on_back);
    window.open_dialog(cx, move |dialog, window, cx| {
        let on_back = on_back.clone();
        dialog
            .w(rems(27.5).to_pixels(window.rem_size()))
            .close_button(false)
            .overlay_closable(false)
            .keyboard(false)
            .on_cancel(|_, _, _| false)
            .child(
                v_flex()
                    .gap_3()
                    .child(dialog_header(
                        title.clone(),
                        Some("Come back later and we'll have more work ready.".into()),
                        cx,
                    ))
                    .child(
                        dialog_footer().child(
                            Button::new("no-work-back")
                                .primary()
                                .label("Back to Projects")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    on_back(window, cx);
                                }),
                        ),
                    ),
            )
    });
}
