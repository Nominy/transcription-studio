//! "Flag this recording?" dialog with an optional reason.

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, WindowExt, v_flex};
use gpui_kit::*;

use crate::editor::panels::task::TaskPanels;
use crate::text::localized;
use exaudio_shell::editor::panels::ui::{
    dialog_footer, dialog_header, error_text, field, primary_button,
};

struct AdminFlagDialog {
    panels: Entity<TaskPanels>,
    reason: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl AdminFlagDialog {
    fn new(panels: Entity<TaskPanels>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let reason = cx.new(|cx| {
            InputState::new(window, cx).placeholder(localized(
                cx,
                "e.g. background noise, wrong emotion, mispronunciation…",
                "например, фоновый шум, не та эмоция, неверное произношение…",
            ))
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
                localized(cx, "Flag this recording?", "Пометить эту запись?"),
                Some(
                    localized(
                        cx,
                        "Marks the recording as unusable and flags the transcript — for poor audio, poor acting, removed tags or an actor going off-script. Edit and submit an accurate transcript first.",
                        "Помечает запись как непригодную и отмечает расшифровку — при плохом звуке, плохой игре, удалённых тегах или уходе актёра от сценария. Сначала исправьте и отправьте точную расшифровку.",
                    )
                    .into(),
                ),
                cx,
            ))
            .child(field(
                localized(cx, "Reason (optional)", "Причина (необязательно)"),
                Input::new(&self.reason).disabled(pending),
                cx,
            ))
            .children(error.map(|error| error_text(error, cx)))
            .child(
                dialog_footer()
                    .child(
                        Button::new("flag-cancel")
                            .label(localized(cx, "Cancel", "Отмена"))
                            .disabled(pending)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        primary_button(
                            "flag-confirm",
                            if pending {
                                localized(cx, "Flagging…", "Пометка…")
                            } else {
                                localized(cx, "Flag", "Пометить")
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
