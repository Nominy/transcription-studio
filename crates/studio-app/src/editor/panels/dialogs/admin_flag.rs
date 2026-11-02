//! "Flag this recording?" dialog with an optional reason.

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, WindowExt, v_flex};
use gpui_kit::*;

use crate::editor::panels::task::TaskPanels;
use crate::editor::panels::ui::{dialog_footer, dialog_header, error_text, field, primary_button};

const DESCRIPTION: &str = "Admin rating marks a recording as unusable and flags the transcription. Use this when the audio or performance has issues that make it unusable - such as poor audio quality, poor acting, removing tags, or a voice actor going fully off-script. You must still edit and submit an accurate transcription before admin rating.";

struct AdminFlagDialog {
    panels: Entity<TaskPanels>,
    reason: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl AdminFlagDialog {
    fn new(panels: Entity<TaskPanels>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let reason = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. background noise, wrong emotion, mispronunciation...")
        });
        let subscriptions = vec![
            cx.observe(&panels, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &reason,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.confirm(window, cx);
                    }
                },
            ),
        ];
        Self {
            panels,
            reason,
            _subscriptions: subscriptions,
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reason = self.reason.read(cx).value();
        self.panels.update(cx, |panels, cx| {
            panels.confirm_admin_flag(&reason, window, cx)
        });
    }
}

impl Render for AdminFlagDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let flow = &self.panels.read(cx).flows.admin_flag;
        let pending = flow.pending;
        let error = flow.error.clone();
        v_flex()
            .gap_3()
            .child(dialog_header(
                "Flag this recording?",
                Some(DESCRIPTION.into()),
                cx,
            ))
            .child(field(
                "Reason (optional)",
                Input::new(&self.reason).disabled(pending),
                cx,
            ))
            .children(error.map(|error| error_text(error, cx)))
            .child(
                dialog_footer()
                    .child(
                        Button::new("flag-cancel")
                            .label("Cancel")
                            .disabled(pending)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        primary_button(
                            "flag-confirm",
                            if pending {
                                "Flagging..."
                            } else {
                                "Confirm Flag"
                            },
                            pending,
                            false,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    ),
            )
    }
}

pub fn open(panels: Entity<TaskPanels>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| AdminFlagDialog::new(panels.clone(), window, cx));
    let reason = view.read(cx).reason.clone();
    let focus = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if focus.replace(false) {
            reason.read(cx).focus_handle(cx).focus(window, cx);
        }
        let guard = panels.clone();
        dialog
            .w(rems(30.).to_pixels(window.rem_size()))
            .close_button(false)
            .on_cancel(move |_, _, cx| !guard.read(cx).flows.admin_flag.pending)
            .child(view.clone())
    });
}
