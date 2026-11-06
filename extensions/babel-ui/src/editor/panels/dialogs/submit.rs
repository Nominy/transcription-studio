//! Submit confirmation dialog (review / practice / final assessment copy).

use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use sonus_babel_domain::editor_model::submit_control::SubmitKind;

use crate::editor::panels::task::TaskPanels;
use sonus_shell::editor::panels::ui::{dialog_footer, dialog_header, error_text, primary_button};

pub struct SubmitCopy {
    pub trigger: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub confirm: &'static str,
}

pub fn copy(kind: SubmitKind) -> SubmitCopy {
    match kind {
        SubmitKind::Review => SubmitCopy {
            trigger: "Submit Review",
            title: "Confirm Submission",
            description: "Are you sure you want to submit?",
            confirm: "Submit",
        },
        SubmitKind::FinalAssessment => SubmitCopy {
            trigger: "Submit Assessment",
            title: "Submit Final Assessment",
            description: "This is your final assessment submission. Your transcription will be reviewed by our team. You will not be able to retry after submitting.",
            confirm: "Submit Assessment",
        },
        SubmitKind::Practice => SubmitCopy {
            trigger: "Submit Practice",
            title: "Submit Practice Attempt",
            description: "Your transcription will be compared to the answer key. You'll be able to review your results and retry as many times as you like. When you're ready, click \"Take Assessment\" to submit for real.",
            confirm: "Submit Practice",
        },
    }
}

const REJECTED: &str = "Submission was rejected. See the status bar below for the exact reason.";

struct SubmitDialog {
    panels: Entity<TaskPanels>,
    _observe: Subscription,
}

impl Render for SubmitDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels = self.panels.read(cx);
        let doc = panels.model().read(cx).doc();
        let kind = panels
            .state
            .borrow()
            .view(doc.session())
            .submit_eligibility(doc.project())
            .kind;
        let copy = copy(kind);
        let submit = &panels.flows.submit;
        let pending = submit.pending;
        let status = panels.model().read(cx).status().cloned();
        let body = if submit.error {
            v_flex()
                .gap_1()
                .child(error_text(REJECTED, cx))
                .children(status.map(|status| error_text(status, cx)))
        } else {
            v_flex()
        };
        let confirm_panels = self.panels.clone();
        v_flex()
            .gap_3()
            .child(dialog_header(copy.title, Some(copy.description.into()), cx))
            .child(body)
            .when(pending, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Submitting your review..."),
                )
            })
            .child(
                dialog_footer()
                    .child(
                        Button::new("submit-cancel")
                            .label("Cancel")
                            .disabled(pending)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        primary_button(
                            "submit-confirm",
                            if pending {
                                "Submitting..."
                            } else {
                                copy.confirm
                            },
                            pending,
                            false,
                        )
                        .on_click(move |_, window, cx| {
                            confirm_panels
                                .update(cx, |panels, cx| panels.confirm_submit(window, cx));
                        }),
                    ),
            )
    }
}

pub fn open(panels: Entity<TaskPanels>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| SubmitDialog {
        _observe: cx.observe(&panels, |_, _, cx| cx.notify()),
        panels: panels.clone(),
    });
    window.open_dialog(cx, move |dialog, window, _| {
        let guard = panels.clone();
        let confirm = panels.clone();
        dialog
            .w(rems(30.).to_pixels(window.rem_size()))
            .close_button(false)
            .on_cancel(move |_, _, cx| !guard.read(cx).flows.submit.pending)
            .on_ok(move |_, window, cx| {
                confirm.update(cx, |panels, cx| panels.confirm_submit(window, cx));
                false
            })
            .child(view.clone())
    });
}
