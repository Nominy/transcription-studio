//! "Skip this chunk?" confirmation.

use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable, WindowExt, v_flex};
use gpui_kit::*;

use crate::editor::panels::task::TaskPanels;
use crate::editor::panels::ui::{dialog_footer, dialog_header, error_text, primary_button};

const DESCRIPTION: &str = "The chunk will be archived and you'll be returned to the project page. This counts against your daily skip cap (20% of your task limit). It will not count toward your task limits or quality metrics.";

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
                "Skip this chunk?",
                Some(DESCRIPTION.into()),
                cx,
            ))
            .children(error.map(|error| error_text(error, cx)))
            .child(
                dialog_footer()
                    .child(
                        Button::new("skip-cancel")
                            .label("Cancel")
                            .disabled(pending)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        primary_button(
                            "skip-confirm",
                            if pending {
                                "Skipping..."
                            } else {
                                "Confirm Skip"
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
