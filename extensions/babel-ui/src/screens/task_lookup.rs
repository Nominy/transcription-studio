//! Task lookup dialog: claim a specific stitched transcription (or, on error-categorization
//! projects, a diff's QA task), or open a review action / processed transcription read-only.

use std::rc::Rc;

use crate::text::localized;
use exaudio_babel_domain::editor_model::task_lookup::{
    TaskLookupEffect, TaskLookupEvent, TaskLookupState,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

fn stitched_description(cx: &App) -> &'static str {
    localized(
        cx,
        "Take a specific task or view one read-only.",
        "Возьмите конкретную задачу или откройте её только для чтения.",
    )
}

fn read_only_description(cx: &App) -> &'static str {
    localized(
        cx,
        "View a task read-only.",
        "Откройте задачу только для чтения.",
    )
}

fn qa_description(cx: &App) -> &'static str {
    localized(
        cx,
        "Take a QA task for a diff or view a task read-only.",
        "Возьмите QA-задачу по диффу или откройте задачу только для чтения.",
    )
}

fn confirm_edit_body(cx: &App) -> &'static str {
    localized(
        cx,
        "Editing reopens this task: you will need to submit it again to mark it verified. \
         For audits, open it read-only.",
        "Редактирование снова открывает задачу: её нужно будет отправить ещё раз, чтобы \
         отметить проверенной. Для аудита откройте её только для чтения.",
    )
}

type OnEffect = Rc<dyn Fn(TaskLookupEffect, &TaskLookupState, &mut Window, &mut App)>;

/// Opens the lookup dialog. `on_effect` receives the requested load with the (trimmed) ids;
/// the dialog closes itself first.
pub fn open(
    state: TaskLookupState,
    window: &mut Window,
    cx: &mut App,
    on_effect: impl Fn(TaskLookupEffect, &TaskLookupState, &mut Window, &mut App) + 'static,
) {
    let view = cx.new(|cx| TaskLookupView::new(state, Rc::new(on_effect), window, cx));
    let focus = view.read(cx).initial_input();
    let initial_focus = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            focus.update(cx, |input, cx| input.focus(window, cx));
        }
        dialog
            .w(rems(30.).to_pixels(window.rem_size()))
            .child(view.clone())
    });
}

struct TaskLookupView {
    state: TaskLookupState,
    processed: Entity<InputState>,
    review_action: Entity<InputState>,
    qa_diff: Entity<InputState>,
    on_effect: OnEffect,
    _subscriptions: Vec<Subscription>,
}

impl TaskLookupView {
    fn new(
        state: TaskLookupState,
        on_effect: OnEffect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let processed = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "Task ID", "ID задачи"))
                .default_value(state.processed_transcription_id.clone())
        });
        let review_action = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(
                    cx,
                    "Review ID (read-only)",
                    "ID просмотра (только чтение)",
                ))
                .default_value(state.review_action_id.clone())
        });
        let qa_diff = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "Diff ID (QA)", "ID диффа (QA)"))
                .default_value(state.qa_diff_id.clone())
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &processed,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        this.apply(
                            TaskLookupEvent::ChangeProcessedTranscriptionId { value },
                            window,
                            cx,
                        );
                    }
                    InputEvent::PressEnter { .. } => {
                        this.apply(TaskLookupEvent::ClickClaimStitched, window, cx)
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &review_action,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        this.apply(TaskLookupEvent::ChangeReviewActionId { value }, window, cx);
                    }
                    InputEvent::PressEnter { .. } => {
                        this.apply(TaskLookupEvent::ClickReadOnly, window, cx)
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &qa_diff,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        this.apply(TaskLookupEvent::ChangeQaDiffId { value }, window, cx);
                    }
                    InputEvent::PressEnter { .. } => {
                        this.apply(TaskLookupEvent::ClickClaimQa, window, cx)
                    }
                    _ => {}
                },
            ),
        ];
        Self {
            state,
            processed,
            review_action,
            qa_diff,
            on_effect,
            _subscriptions: subscriptions,
        }
    }

    fn initial_input(&self) -> Entity<InputState> {
        if self.state.shows_qa_claim() {
            self.qa_diff.clone()
        } else if self.state.shows_stitched_claim() {
            self.processed.clone()
        } else {
            self.review_action.clone()
        }
    }

    fn apply(&mut self, event: TaskLookupEvent, window: &mut Window, cx: &mut Context<Self>) {
        let effect = self.state.apply(event);
        cx.notify();
        if let Some(effect) = effect {
            let state = self.state.clone();
            let on_effect = self.on_effect.clone();
            window.close_dialog(cx);
            on_effect(effect, &state, window, cx);
        }
    }

    fn render_select(&self, cx: &mut Context<Self>) -> Div {
        let description = if self.state.shows_qa_claim() {
            qa_description(cx)
        } else if self.state.show_stitched_option {
            stitched_description(cx)
        } else {
            read_only_description(cx)
        };
        v_flex()
            .gap_3()
            .child(title(localized(cx, "Find a task", "Найти задачу")))
            .child(body(description, cx))
            .when(self.state.shows_qa_claim(), |this| {
                this.child(Input::new(&self.qa_diff))
            })
            .when(self.state.shows_stitched_claim(), |this| {
                this.child(Input::new(&self.processed))
            })
            .child(Input::new(&self.review_action))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .when(self.state.shows_qa_claim(), |this| {
                        this.child(
                            Button::new("claim-qa")
                                .outline()
                                .label(localized(cx, "Take QA task", "Взять QA-задачу"))
                                .disabled(!self.state.can_claim_qa())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.apply(TaskLookupEvent::ClickClaimQa, window, cx)
                                })),
                        )
                    })
                    .when(self.state.shows_stitched_claim(), |this| {
                        this.child(
                            Button::new("claim-stitched")
                                .outline()
                                .label(localized(cx, "Take task", "Взять задачу"))
                                .disabled(!self.state.can_claim_stitched())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.apply(TaskLookupEvent::ClickClaimStitched, window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("view-read-only")
                            .primary()
                            .label(localized(cx, "View read-only", "Только чтение"))
                            .disabled(!self.state.can_view_read_only())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply(TaskLookupEvent::ClickReadOnly, window, cx)
                            })),
                    ),
            )
    }

    fn render_confirm(&self, cx: &mut Context<Self>) -> Div {
        v_flex()
            .gap_3()
            .child(title(localized(
                cx,
                "Edit this task?",
                "Редактировать задачу?",
            )))
            .child(body(confirm_edit_body(cx), cx))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-edit")
                            .ghost()
                            .label(localized(cx, "Cancel", "Отмена"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply(TaskLookupEvent::CancelConfirmEdit, window, cx)
                            })),
                    )
                    .child(
                        Button::new("confirm-edit")
                            .primary()
                            .label(localized(cx, "Edit", "Редактировать"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply(TaskLookupEvent::ConfirmEdit, window, cx)
                            })),
                    ),
            )
    }
}

impl Render for TaskLookupView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.confirming_edit {
            self.render_confirm(cx)
        } else {
            self.render_select(cx)
        }
    }
}

fn title(text: &'static str) -> Div {
    div()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

fn body(text: &'static str, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}
