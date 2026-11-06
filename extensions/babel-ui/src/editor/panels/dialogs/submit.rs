//! Submit confirmation dialog (review / practice / final assessment copy).

use exaudio_babel_domain::editor_model::submit_control::SubmitKind;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::editor::panels::task::TaskPanels;
use crate::text::localized;
use exaudio_shell::editor::panels::ui::{dialog_footer, dialog_header, error_text, primary_button};

pub struct SubmitCopy {
    pub trigger: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub confirm: &'static str,
}

pub fn copy(kind: SubmitKind, cx: &App) -> SubmitCopy {
    match kind {
        SubmitKind::Review => SubmitCopy {
            trigger: localized(cx, "Submit", "Отправить"),
            title: localized(cx, "Submit this task?", "Отправить задачу?"),
            description: localized(
                cx,
                "You can't edit it afterwards.",
                "После этого изменить её нельзя.",
            ),
            confirm: localized(cx, "Submit", "Отправить"),
        },
        SubmitKind::FinalAssessment => SubmitCopy {
            trigger: localized(cx, "Submit assessment", "Отправить тест"),
            title: localized(
                cx,
                "Submit the final assessment?",
                "Отправить итоговый тест?",
            ),
            description: localized(
                cx,
                "Our team reviews your transcript. You can't retry afterwards.",
                "Наша команда проверит вашу расшифровку. Повторить попытку потом нельзя.",
            ),
            confirm: localized(cx, "Submit", "Отправить"),
        },
        SubmitKind::Practice => SubmitCopy {
            trigger: localized(cx, "Submit practice", "Отправить тренировку"),
            title: localized(
                cx,
                "Submit this practice attempt?",
                "Отправить тренировочную попытку?",
            ),
            description: localized(
                cx,
                "Your transcript is compared with the answer key. You can review the result and try again as often as you like. When you're ready, choose “Take assessment” to submit for real.",
                "Ваша расшифровка сравнивается с эталоном. Результат можно посмотреть и повторить попытку сколько угодно раз. Когда будете готовы, выберите «Пройти тест», чтобы отправить по-настоящему.",
            ),
            confirm: localized(cx, "Submit", "Отправить"),
        },
    }
}

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
        let copy = copy(kind, cx);
        let submit = &panels.flows.submit;
        let pending = submit.pending;
        let status = panels.model().read(cx).status().cloned();
        let body = if submit.error {
            v_flex()
                .gap_1()
                .child(error_text(
                    localized(
                        cx,
                        "The task wasn't accepted. The reason is in the status bar.",
                        "Задача не принята. Причина указана в строке состояния.",
                    ),
                    cx,
                ))
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
                        .child(localized(cx, "Submitting…", "Отправка…")),
                )
            })
            .child(
                dialog_footer()
                    .child(
                        Button::new("submit-cancel")
                            .label(localized(cx, "Cancel", "Отмена"))
                            .disabled(pending)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        primary_button(
                            "submit-confirm",
                            if pending {
                                localized(cx, "Submitting…", "Отправка…")
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
