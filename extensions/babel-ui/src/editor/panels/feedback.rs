//! Feedback drawer: the required L1 feedback form of L2 review tasks (rubric ratings and
//! comments, optional other feedback) with autosave to Babel, plus "Magic Review" and
//! "Grade review".
//!
//! The form itself lives on [`TaskPanels`] so submit gating and reviewer flows can use it.

use std::collections::BTreeMap;

use exaudio_babel_domain::editor_model::feedback_assessment::{
    FEEDBACK_MAX_COMMENT_LENGTH, FeedbackDraftSaveRequest,
};
use exaudio_babel_services::review::{
    CategoryGrade, FeedbackBootstrapIds, FeedbackForm, ReviewerFeedback, feedback_is_tts_project,
    load_feedback_bootstrap, save_feedback_draft,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::task::{Drawer, TaskPanels};
use crate::text;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::editor::panels::ui::{caption, muted};
use exaudio_shell::services::spawn_service_in;

/// Rating buttons: value and label (EN, RU).
const RATINGS: [(u8, &str, &str); 3] = [
    (1, "1 · Good", "1 · Хорошо"),
    (2, "2 · OK", "2 · Нормально"),
    (3, "3 · Needs work", "3 · Нужно доработать"),
];

impl TaskPanels {
    /// Loads the live feedback form (worker id → form → inputs → saved draft).
    pub(super) fn bootstrap_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.model.read(cx).doc().project().clone();
        let is_tts = feedback_is_tts_project(&project);
        self.feedback = Some(FeedbackForm::loading(is_tts));
        self.feedback_attempt += 1;
        self.sync_feedback_submit_state(cx);
        self.status(
            text::localized(cx, "Loading the feedback form…", "Загрузка формы отзыва…"),
            cx,
        );
        let attempt = self.feedback_attempt;
        let generation = self.generation;
        let babel = crate::services::document_client(&self.model, cx);
        let task = spawn_service_in(
            window,
            cx,
            async move {
                let dashboard = babel.load_dashboard().await?;
                let worker_id = dashboard.worker_id.unwrap_or_default();
                let ids = FeedbackBootstrapIds::for_project(&project, &worker_id)?;
                let bootstrap = load_feedback_bootstrap(&babel, &ids).await?;
                FeedbackForm::from_bootstrap(bootstrap, is_tts)
            },
            move |this, result: anyhow::Result<FeedbackForm>, _, cx| {
                if this.feedback_attempt != attempt || this.generation != generation {
                    return;
                }
                match result {
                    Ok(form) => {
                        this.feedback = Some(form);
                        this.status(
                            text::localized(cx, "Feedback form loaded.", "Форма отзыва загружена."),
                            cx,
                        );
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        if let Some(form) = this.feedback.as_mut() {
                            form.mark_bootstrap_failed(message.clone());
                        }
                        this.drawer = Drawer::Feedback;
                        this.status(
                            text::failed(
                                cx,
                                "Couldn't load the feedback form.",
                                "Не удалось загрузить форму отзыва.",
                                message,
                            ),
                            cx,
                        );
                    }
                }
                this.sync_feedback_submit_state(cx);
                cx.notify();
            },
        );
        self.feedback_tasks.push(task);
        cx.notify();
    }

    /// Mirrors loading/saving/readiness into the workflow's submit gating metadata.
    pub(super) fn sync_feedback_submit_state(&self, cx: &mut App) {
        let Some(form) = self.feedback.as_ref() else {
            return;
        };
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.with_workflow(|workflow| form.sync_submit_state(workflow))
            });
        });
    }

    /// Applies a form edit; changed drafts are saved right away (single-flight).
    pub(super) fn edit_feedback(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut FeedbackForm) -> bool,
    ) {
        let Some(form) = self.feedback.as_mut() else {
            return;
        };
        if !edit(form) {
            cx.notify();
            return;
        }
        self.request_feedback_save(window, cx);
    }

    fn request_feedback_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.feedback.as_mut().and_then(FeedbackForm::request_save) else {
            self.sync_feedback_submit_state(cx);
            cx.notify();
            return;
        };
        self.send_feedback_save(request, window, cx);
    }

    /// Sends `request`, already marked in flight by the form; a draft changed meanwhile is
    /// sent next.
    fn send_feedback_save(
        &mut self,
        request: FeedbackDraftSaveRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status(
            text::localized(cx, "Saving feedback…", "Сохранение отзыва…"),
            cx,
        );
        self.sync_feedback_submit_state(cx);
        let babel = crate::services::document_client(&self.model, cx);
        let generation = self.generation;
        let sent = request.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { save_feedback_draft(&babel, &request).await },
            move |this, result, window, cx| {
                if this.generation != generation {
                    return;
                }
                let error = result.as_ref().err().map(|error| format!("{error:#}"));
                let Some(form) = this.feedback.as_mut() else {
                    return;
                };
                let next = form.finish_save(&sent, result);
                let message = match (&error, next.is_some()) {
                    (None, false) => {
                        text::localized(cx, "Feedback saved.", "Отзыв сохранён.").to_string()
                    }
                    (None, true) => text::localized(
                        cx,
                        "Feedback changed while saving. Saving the latest version…",
                        "Отзыв изменился во время сохранения. Сохраняем последнюю версию…",
                    )
                    .to_string(),
                    (Some(error), false) => text::failed(
                        cx,
                        "Couldn't save the feedback.",
                        "Не удалось сохранить отзыв.",
                        error,
                    ),
                    (Some(error), true) => text::failed(
                        cx,
                        "Couldn't save the feedback. Saving the latest version…",
                        "Не удалось сохранить отзыв. Сохраняем последнюю версию…",
                        error,
                    ),
                };
                this.status(message, cx);
                match next {
                    Some(next) => this.send_feedback_save(next, window, cx),
                    None => {
                        this.sync_feedback_submit_state(cx);
                        cx.notify();
                    }
                }
            },
        );
        self.feedback_tasks.retain(|task| !task.is_ready());
        self.feedback_tasks.push(task);
        cx.notify();
    }

    /// Applies Magic Review category feedback and grade scores (scores only: comments stay) to
    /// the form, opens the drawer and saves. Returns the number of updated categories.
    pub(super) fn apply_feedback_and_grades(
        &mut self,
        feedback: &[ReviewerFeedback],
        grades: &[CategoryGrade],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let Some(form) = self.feedback.as_mut() else {
            return Err(text::localized(
                cx,
                "Couldn't find the feedback form.",
                "Не удалось найти форму отзыва.",
            )
            .to_string());
        };
        if form.read_only || form.is_loading() || form.bootstrap_error().is_some() {
            return Err(text::localized(
                cx,
                "The feedback form can't be edited right now.",
                "Форму отзыва сейчас нельзя изменить.",
            )
            .to_string());
        }
        let applied = form.apply_reviewer_feedback(feedback);
        if !feedback.is_empty() && applied == 0 {
            return Err(text::localized(
                cx,
                "Couldn't find the form fields for this feedback.",
                "Не удалось найти поля формы для этого отзыва.",
            )
            .to_string());
        }
        let graded = form.apply_grade_scores(grades);
        self.drawer = Drawer::Feedback;
        self.request_feedback_save(window, cx);
        Ok(applied.max(graded))
    }
}

/// The feedback drawer view.
pub struct FeedbackPanel {
    panels: Entity<TaskPanels>,
    comments: BTreeMap<&'static str, Entity<TextareaState>>,
    other: Option<Entity<TextareaState>>,
    _subscriptions: Vec<Subscription>,
}

impl FeedbackPanel {
    /// Drawer width (the editor mounts it at this width, capped to the window).
    pub const WIDTH: Rems = rems(27.5);

    /// Whether the drawer shows (it renders nothing otherwise).
    pub fn is_open(panels: &TaskPanels) -> bool {
        panels.drawer == Drawer::Feedback && panels.feedback.is_some()
    }

    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panels = TaskPanels::for_model(&model, window, cx);
        let subscriptions = vec![cx.observe_in(&panels, window, |this, _, window, cx| {
            this.sync_inputs(window, cx);
            cx.notify();
        })];
        let mut this = Self {
            panels,
            comments: BTreeMap::new(),
            other: None,
            _subscriptions: subscriptions,
        };
        this.sync_inputs(window, cx);
        this
    }

    /// Creates editors for the form's fields and mirrors external changes (reviewer apply,
    /// rejected over-long edits) into them.
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.panels.read(cx).feedback.clone() else {
            return;
        };
        for field in &form.rubric_fields {
            let text = form
                .draft()
                .entries
                .get(field.key)
                .map(|entry| entry.comment.clone())
                .unwrap_or_default();
            let key = field.key;
            let input = match self.comments.get(key) {
                Some(input) => input.clone(),
                None => {
                    let placeholder = text::localized(
                        cx,
                        "What should the transcriber change?",
                        "Что расшифровщику стоит изменить?",
                    );
                    let input = self.new_editor(placeholder, window, cx);
                    self._subscriptions.push(cx.subscribe_in(
                        &input,
                        window,
                        move |this, input, event: &InputEvent, window, cx| {
                            if let InputEvent::Change = event {
                                let value = input.read(cx).value().to_string();
                                this.panels.update(cx, |panels, cx| {
                                    panels.edit_feedback(window, cx, |form| {
                                        form.set_comment(key, &value)
                                    });
                                });
                            }
                        },
                    ));
                    self.comments.insert(key, input.clone());
                    input
                }
            };
            set_if_changed(&input, &text, window, cx);
        }
        if form.has_other_feedback {
            let input = match &self.other {
                Some(input) => input.clone(),
                None => {
                    let placeholder = text::localized(cx, "Anything else?", "Что-нибудь ещё?");
                    let input = self.new_editor(placeholder, window, cx);
                    self._subscriptions.push(cx.subscribe_in(
                        &input,
                        window,
                        |this, input, event: &InputEvent, window, cx| {
                            if let InputEvent::Change = event {
                                let value = input.read(cx).value().to_string();
                                this.panels.update(cx, |panels, cx| {
                                    panels.edit_feedback(window, cx, |form| {
                                        form.set_other_feedback(&value)
                                    });
                                });
                            }
                        },
                    ));
                    self.other = Some(input.clone());
                    input
                }
            };
            set_if_changed(&input, &form.draft().other_feedback, window, cx);
        }
    }

    fn new_editor(
        &self,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 8)
                .placeholder(placeholder)
        })
    }

    fn render_field(
        &self,
        form: &FeedbackForm,
        key: &'static str,
        label: &'static str,
        description: Option<&'static str>,
        cx: &App,
    ) -> Div {
        let theme = cx.theme();
        let entry = form.draft().entries.get(key);
        let rating = entry.map_or(1, |entry| entry.rating);
        let comment_len = entry.map_or(0, |entry| entry.comment.chars().count());
        let complete = entry.is_some_and(|entry| !entry.comment.trim().is_empty());
        let editable = !form.read_only && form.bootstrap_error().is_none() && !form.is_loading();
        let ratings = h_flex()
            .gap_1()
            .children(RATINGS.iter().map(|&(value, en, ru)| {
                let panels = self.panels.clone();
                Button::new(SharedString::from(format!("rating-{key}-{value}")))
                    .small()
                    .label(text::localized(cx, en, ru))
                    .when(rating == value, |button| button.primary())
                    .when(rating != value, |button| button.outline())
                    .disabled(!editable)
                    .on_click(move |_, window, cx| {
                        panels.update(cx, |panels, cx| {
                            panels.edit_feedback(window, cx, |form| form.set_rating(key, value));
                        });
                    })
            }));
        let comment = self
            .comments
            .get(key)
            .map(|input| Textarea::new(input).disabled(!editable).into_any_element());
        v_flex()
            .gap_2()
            .p_3()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if complete {
                                theme.success
                            } else {
                                theme.warning
                            })
                            .child(if complete {
                                text::localized(cx, "Required · done", "Обязательно · готово")
                            } else {
                                text::localized(
                                    cx,
                                    "Required · add a comment",
                                    "Обязательно · добавьте комментарий",
                                )
                            }),
                    ),
            )
            .children(description.map(|description| muted(description, cx)))
            .child(caption(text::localized(cx, "Rating *", "Оценка *"), cx))
            .child(ratings)
            .child(caption(
                text::localized(cx, "Comment *", "Комментарий *"),
                cx,
            ))
            .children(comment)
            .child(muted(format!("{comment_len}/{FEEDBACK_MAX_COMMENT_LENGTH}"), cx).text_right())
    }

    fn autosave_label(form: &FeedbackForm, cx: &App) -> (SharedString, Option<SharedString>) {
        if form.read_only {
            (
                text::localized(cx, "Read-only", "Только чтение").into(),
                None,
            )
        } else if form.is_loading() {
            (
                text::localized(cx, "Loading feedback", "Загрузка отзыва").into(),
                None,
            )
        } else if form.bootstrap_error().is_some() {
            (
                text::localized(cx, "Feedback unavailable", "Отзыв недоступен").into(),
                None,
            )
        } else if let Some(error) = form.last_autosave_error() {
            (
                text::failed(
                    cx,
                    "Autosave didn't work.",
                    "Автосохранение: не удалось.",
                    error,
                )
                .into(),
                Some(
                    text::localized(
                        cx,
                        "Edit again to retry.",
                        "Измените текст, чтобы повторить.",
                    )
                    .into(),
                ),
            )
        } else if form.is_saving() {
            (text::localized(cx, "Saving…", "Сохранение…").into(), None)
        } else if form.last_saved().is_some() {
            (
                text::localized(cx, "All changes saved", "Все изменения сохранены").into(),
                None,
            )
        } else {
            (
                text::localized(cx, "Autosave is on", "Автосохранение включено").into(),
                None,
            )
        }
    }

    fn footer_state(form: &FeedbackForm, cx: &App) -> &'static str {
        if form.read_only {
            text::localized(cx, "Feedback received", "Отзыв получен")
        } else if form.bootstrap_error().is_some() {
            text::localized(cx, "Feedback unavailable", "Отзыв недоступен")
        } else if form.save_has_error() {
            text::localized(cx, "Couldn't save", "Не удалось сохранить")
        } else if form.is_saving() {
            text::localized(cx, "Saving…", "Сохранение…")
        } else if form.is_ready() {
            text::localized(cx, "Ready", "Готово")
        } else {
            text::localized(cx, "Fill in all fields", "Заполните все поля")
        }
    }

    fn render_body(&self, form: &FeedbackForm, cx: &App) -> Div {
        let theme = cx.theme();
        let notice = |title: &'static str, body: SharedString, cx: &App| {
            v_flex()
                .gap_1()
                .p_3()
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(muted(body, cx))
        };
        if let Some(error) = form.bootstrap_error() {
            let panels = self.panels.clone();
            return v_flex().child(
                notice(
                    text::localized(
                        cx,
                        "Couldn't load the feedback form.",
                        "Не удалось загрузить форму отзыва.",
                    ),
                    error.to_string().into(),
                    cx,
                )
                .border_color(theme.danger)
                .child(
                    div().pt_1().child(
                        Button::new("feedback-retry")
                            .small()
                            .outline()
                            .label(text::localized(cx, "Try again", "Повторить"))
                            .on_click(move |_, window, cx| {
                                panels
                                    .update(cx, |panels, cx| panels.bootstrap_feedback(window, cx));
                            }),
                    ),
                ),
            );
        }
        if form.is_loading() {
            let title = if form.read_only {
                text::localized(cx, "Loading feedback", "Загрузка отзыва")
            } else {
                text::localized(cx, "Loading the rubric", "Загрузка рубрики")
            };
            return v_flex().child(notice(
                title,
                text::localized(
                    cx,
                    "The feedback form will appear here.",
                    "Форма отзыва появится здесь.",
                )
                .into(),
                cx,
            ));
        }
        let draft_empty = form
            .draft()
            .entries
            .values()
            .all(|entry| entry.comment.trim().is_empty())
            && form.draft().other_feedback.trim().is_empty();
        if form.read_only && draft_empty {
            return v_flex().child(notice(
                text::localized(cx, "No feedback", "Отзыва нет"),
                text::localized(
                    cx,
                    "This review was completed without rubric feedback.",
                    "Эта проверка завершена без отзыва по рубрике.",
                )
                .into(),
                cx,
            ));
        }
        if form.rubric_fields.is_empty() {
            return v_flex().child(notice(
                text::localized(cx, "No rubric categories", "Нет категорий рубрики"),
                text::localized(
                    cx,
                    "Reload the form before completing this review.",
                    "Загрузите форму заново, прежде чем завершить проверку.",
                )
                .into(),
                cx,
            ));
        }
        let mut body = v_flex().gap_3();
        for field in &form.rubric_fields {
            body = body.child(self.render_field(form, field.key, field.title(), field.hint(), cx));
        }
        if form.has_other_feedback {
            let len = form.draft().other_feedback.chars().count();
            let editable = !form.read_only;
            body = body.child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                                    text::localized(cx, "Other feedback", "Другие замечания"),
                                ),
                            )
                            .child(muted(text::localized(cx, "Optional", "Необязательно"), cx)),
                    )
                    .child(muted(
                        text::localized(
                            cx,
                            "Anything else the transcriber should know.",
                            "Всё, что ещё стоит знать расшифровщику.",
                        ),
                        cx,
                    ))
                    .child(caption(
                        text::localized(cx, "Additional comments", "Дополнительные комментарии"),
                        cx,
                    ))
                    .children(
                        self.other
                            .as_ref()
                            .map(|input| Textarea::new(input).disabled(!editable)),
                    )
                    .child(muted(format!("{len}/{FEEDBACK_MAX_COMMENT_LENGTH}"), cx).text_right()),
            );
        }
        body
    }
}

fn set_if_changed(input: &Entity<TextareaState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value() != value {
        let value = value.to_string();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

impl Render for FeedbackPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels = self.panels.read(cx);
        if !Self::is_open(panels) {
            return div().into_any_element();
        }
        let Some(form) = panels.feedback.clone() else {
            return div().into_any_element();
        };
        let generating = panels.review.running;
        let grading_status = panels.review.grading_status.clone();
        let theme = cx.theme();
        let (title, description) = if form.read_only {
            (
                text::localized(cx, "Feedback received", "Отзыв получен"),
                text::localized(cx, "Feedback on this task.", "Отзыв по этой задаче."),
            )
        } else {
            (
                text::localized(cx, "L1 feedback", "Отзыв для L1"),
                text::localized(
                    cx,
                    "Feedback for the L1 transcriber.",
                    "Отзыв для расшифровщика L1.",
                ),
            )
        };
        let (autosave, autosave_hint) = Self::autosave_label(&form, cx);
        let required = form.rubric_fields.len();
        let completed = form
            .rubric_fields
            .iter()
            .filter(|field| {
                form.draft()
                    .entries
                    .get(field.key)
                    .is_some_and(|entry| !entry.comment.trim().is_empty())
            })
            .count();
        let magic_disabled =
            form.read_only || form.is_loading() || form.bootstrap_error().is_some() || generating;
        let magic = self.panels.clone();
        let grade = self.panels.clone();
        let close = self.panels.clone();
        v_flex()
            .size_full()
            .occlude()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .child(muted(description, cx)),
                            )
                            .when(!form.read_only, |this| {
                                this.child(
                                    Button::new("feedback-magic-review")
                                        .small()
                                        .outline()
                                        .icon(IconName::Bot)
                                        .label(text::localized(cx, "Magic Review", "Magic Review"))
                                        .loading(generating)
                                        .disabled(magic_disabled)
                                        .on_click(move |_, window, cx| {
                                            magic.update(cx, |panels, cx| {
                                                panels.magic_review(window, cx)
                                            });
                                        }),
                                )
                                .child(
                                    Button::new("feedback-grade-review")
                                        .small()
                                        .outline()
                                        .label(text::localized(cx, "Grade", "Оценить"))
                                        .disabled(magic_disabled)
                                        .on_click(move |_, _, cx| {
                                            grade.update(cx, |panels, cx| panels.open_grader(cx));
                                        }),
                                )
                            })
                            .child(
                                Button::new("feedback-close")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .tooltip(text::localized(cx, "Close feedback", "Закрыть отзыв"))
                                    .on_click(move |_, _, cx| {
                                        close.update(cx, |panels, cx| {
                                            panels.toggle_drawer(Drawer::Feedback, cx);
                                            panels.status(
                                                text::localized(
                                                    cx,
                                                    "Feedback closed.",
                                                    "Отзыв закрыт.",
                                                ),
                                                cx,
                                            );
                                        });
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(muted(autosave, cx))
                            .children(autosave_hint.map(|hint| muted(hint, cx)))
                            .children(grading_status.map(|(status, error)| {
                                let line = muted(status, cx);
                                if error {
                                    line.text_color(theme.danger)
                                } else {
                                    line
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .id("feedback-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(div().p_3().child(self.render_body(&form, cx))),
            )
            .child(
                h_flex()
                    .justify_between()
                    .p_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        v_flex()
                            .child(caption(text::localized(cx, "Progress", "Прогресс"), cx))
                            .child(div().text_sm().child(text::localized_fmt(
                                cx,
                                "{completed} of {required} required sections done",
                                "Готово {completed} из {required} обязательных разделов",
                                &[
                                    ("completed", &completed.to_string()),
                                    ("required", &required.to_string()),
                                ],
                            ))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(Self::footer_state(&form, cx)),
                    ),
            )
            .into_any_element()
    }
}
