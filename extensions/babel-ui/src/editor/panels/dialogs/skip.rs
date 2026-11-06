//! "Skip this chunk?" confirmation.

use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable, WindowExt, v_flex};
use gpui_kit::*;

use crate::editor::panels::task::TaskPanels;
use crate::text::localized;
use exaudio_shell::editor::panels::ui::{dialog_footer, dialog_header, error_text, primary_button};

struct SkipDialog {
    panels: Entity<TaskPanels>,
    _observe: Subscription,
}

impl Render for SkipDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let flow = &self.panels.read(cx).flows.skip;
        let pending = flow.pending;
        let error = flow.error.clone();
        let panels = self.panels.clone();
        v_flex()
            .gap_3()
            .child(dialog_header(
                localized(cx, "Skip this chunk?", "Пропустить этот чанк?"),
                Some(
                    localized(
                        cx,
                        "The chunk goes back to the queue and you return to the project. Skips are limited to 20% of your daily task limit.",
                        "Чанк вернётся в очередь, а вы — к проекту. Пропуски ограничены 20% дневного лимита задач.",
                    )
                    .into(),
                ),
                cx,
            ))
            .children(error.map(|error| error_text(error, cx)))
            .child(
                dialog_footer()
                    .child(
                        Button::new("skip-cancel")
                            .label(localized(cx, "Cancel", "Отмена"))
                            .disabled(pending)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        primary_button(
                            "skip-confirm",
                            if pending {
                                localized(cx, "Skipping…", "Пропуск…")
                            } else {
                                localized(cx, "Skip", "Пропустить")
                            },
                            pending,
                            false,
                        )
                        .on_click(move |_, window, cx| {
                            panels.update(cx, |panels, cx| panels.confirm_skip(window, cx));
                        }),
                    ),
            )
    }
}

pub fn open(panels: Entity<TaskPanels>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| SkipDialog {
        _observe: cx.observe(&panels, |_, _, cx| cx.notify()),
        panels: panels.clone(),
    });
    window.open_dialog(cx, move |dialog, window, _| {
        let guard = panels.clone();
        dialog
            .w(rems(28.75).to_pixels(window.rem_size()))
            .close_button(false)
            .on_cancel(move |_, _, cx| !guard.read(cx).flows.skip.pending)
            .child(view.clone())
    });
}
