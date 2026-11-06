//! "No work available" modal shown after claiming the next task found nothing.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{WindowExt, v_flex};
use gpui_kit::*;

use crate::text::{localized, localized_fmt};
use exaudio_shell::editor::panels::ui::{dialog_footer, dialog_header};

/// Opens the non-dismissible no-work modal; "Back to projects" closes it and calls `on_back`.
/// `job_kind` fills "No {job_kind} tasks right now".
pub fn open(
    job_kind: &str,
    window: &mut Window,
    cx: &mut App,
    on_back: impl Fn(&mut Window, &mut App) + 'static,
) {
    let title: SharedString = localized_fmt(
        cx,
        "No {job_kind} tasks right now",
        "Сейчас нет задач: {job_kind}",
        &[("job_kind", job_kind)],
    )
    .into();
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
                        Some(localized(cx, "Check back later.", "Загляните позже.").into()),
                        cx,
                    ))
                    .child(
                        dialog_footer().child(
                            Button::new("no-work-back")
                                .primary()
                                .label(localized(cx, "Back to projects", "К проектам"))
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    on_back(window, cx);
                                }),
                        ),
                    ),
            )
    });
}
