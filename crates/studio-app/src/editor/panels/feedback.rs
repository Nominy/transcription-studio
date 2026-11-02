//! Feedback drawer: the required L1 feedback form of L2 review tasks (rubric ratings and
//! comments, optional other feedback) with autosave to Babel, plus "Magic Review".
//!
//! The form itself lives on [`TaskPanels`] so submit gating and reviewer flows can use it.

use std::collections::BTreeMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::editor_model::feedback_assessment::FEEDBACK_MAX_COMMENT_LENGTH;
use studio_services::review::{
    FeedbackBootstrapIds, FeedbackForm, ReviewerFeedback, feedback_is_tts_project,
    load_feedback_bootstrap, save_feedback_draft,
};

use super::task::{Drawer, TaskPanels};
use super::ui::{caption, muted};
use crate::editor::model::EditorModel;
use crate::services::{AppServices, spawn_service_in};

const RATINGS: [(u8, &str); 3] = [(1, "1 Good"), (2, "2 OK"), (3, "3 Needs Work")];

impl TaskPanels {
    /// Loads the live feedback form (worker id → form → inputs → saved draft).
    pub(super) fn bootstrap_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.model.read(cx).doc().project().clone();
        let is_tts = feedback_is_tts_project(&project);
        self.feedback = Some(FeedbackForm::loading(is_tts));
        self.feedback_attempt += 1;
        self.sync_feedback_submit_state(cx);
        self.status("Loading required L2 feedback form...", cx);
        let attempt = self.feedback_attempt;
        let generation = self.generation;
        let babel = cx.global::<AppServices>().babel.clone();
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
                        this.status("Required L2 feedback form loaded.", cx);
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        if let Some(form) = this.feedback.as_mut() {
                            form.mark_bootstrap_failed(message.clone());
                        }
                        this.drawer = Drawer::Feedback;
                        this.status(message, cx);
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
        self.status("Saving feedback draft...", cx);
        self.sync_feedback_submit_state(cx);
        let babel = cx.global::<AppServices>().babel.clone();
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
                    (None, false) => "Feedback draft saved.".to_string(),
                    (None, true) => {
                        "Feedback changed while saving; saving the latest draft...".to_string()
                    }
                    (Some(error), false) => format!("Feedback draft save failed: {error}"),
                    (Some(error), true) => {
                        format!("Feedback draft save failed ({error}); saving the latest draft...")
                    }
                };
                this.status(message, cx);
                if next.is_some() {
                    this.request_feedback_save(window, cx);
                } else {
                    this.sync_feedback_submit_state(cx);
                    cx.notify();
                }
            },
        );
        self.feedback_tasks.retain(|task| !task.is_ready());
        self.feedback_tasks.push(task);
        cx.notify();
    }

    /// Applies generated/finalized reviewer feedback to the form; opens the drawer and saves
    /// when anything matched. Returns the number of applied items.
    pub(super) fn apply_reviewer_feedback(
        &mut self,
        feedback: &[ReviewerFeedback],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let Some(form) = self.feedback.as_mut() else {
            self.status("Feedback panel is unavailable for this Babel task.", cx);
            return 0;
        };
        if form.read_only {
            self.status(
                "Feedback panel is read-only; reviewer feedback was not applied.",
                cx,
            );
            return 0;
        }
        let applied = form.apply_reviewer_feedback(feedback);
        if applied == 0 {
            self.status(
                "No matching non-empty reviewer feedback was available to apply.",
                cx,
            );
            return 0;
        }
        self.drawer = Drawer::Feedback;
        self.request_feedback_save(window, cx);
        applied
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
                    let input =
                        self.new_editor("Provide specific, actionable feedback...", window, cx);
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
                    let input = self.new_editor("Add optional feedback...", window, cx);
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
            .children(RATINGS.iter().map(|(value, label)| {
                let value = *value;
                let panels = self.panels.clone();
                Button::new(SharedString::from(format!("rating-{key}-{value}")))
                    .small()
                    .label(*label)
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
                                "Required · Complete"
                            } else {
                                "Required · Comment needed"
                            }),
                    ),
            )
            .children(description.map(|description| muted(description, cx)))
            .child(caption("Rating *", cx))
            .child(ratings)
            .child(caption("Comment *", cx))
            .children(comment)
            .child(muted(format!("{comment_len}/{FEEDBACK_MAX_COMMENT_LENGTH}"), cx).text_right())
    }

    fn autosave_label(form: &FeedbackForm) -> (SharedString, Option<SharedString>) {
        if form.read_only {
            ("Read-only".into(), None)
        } else if form.is_loading() {
            ("Loading feedback".into(), None)
        } else if form.bootstrap_error().is_some() {
            ("Feedback unavailable".into(), None)
        } else if let Some(error) = form.last_autosave_error() {
            (
                format!("Autosave failed: {error}").into(),
                Some("Edit again to retry automatically.".into()),
            )
        } else if form.is_saving() {
            ("Autosaving".into(), None)
        } else if form.last_saved().is_some() {
            ("All changes saved".into(), None)
        } else {
            ("Autosave is on".into(), None)
        }
    }

    fn footer_state(form: &FeedbackForm) -> &'static str {
        if form.read_only {
            "Feedback received"
        } else if form.bootstrap_error().is_some() {
            "Feedback unavailable"
        } else if form.save_has_error() {
            "Save failed"
        } else if form.is_saving() {
            "Saving..."
        } else if form.is_ready() {
            "Ready"
        } else {
            "Complete all fields"
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
                    "The feedback form could not be loaded.",
                    error.to_string().into(),
                    cx,
                )
                .border_color(theme.danger)
                .child(
                    div().pt_1().child(
                        Button::new("feedback-retry")
                            .small()
                            .outline()
                            .label("Retry loading")
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
                "Loading feedback"
            } else {
                "Loading rubric categories"
            };
            return v_flex().child(notice(
                title,
                "Your feedback form will appear here when it is ready.".into(),
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
                "No feedback was provided",
                "This completed review does not contain rubric feedback.".into(),
                cx,
            ));
        }
        if form.rubric_fields.is_empty() {
            return v_flex().child(notice(
                "No rubric categories are available",
                "Retry loading the form before completing this review.".into(),
                cx,
            ));
        }
        let mut body = v_flex().gap_3();
        for field in &form.rubric_fields {
            body =
                body.child(self.render_field(form, field.key, field.label, field.description, cx));
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
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Other Feedback"),
                            )
                            .child(muted("Optional", cx)),
                    )
                    .child(muted("Share any additional comments or observations.", cx))
                    .child(caption("Additional comments", cx))
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
        let generating = panels.reviewer_is_pending("generate");
        let theme = cx.theme();
        let (title, description) = if form.read_only {
            ("Feedback Received", "Review the feedback provided.")
        } else {
            (
                "L1 Feedback Form",
                "Provide feedback for the L1 transcriber.",
            )
        };
        let (autosave, autosave_hint) = Self::autosave_label(&form);
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
                                        .label("Magic Review")
                                        .loading(generating)
                                        .disabled(magic_disabled)
                                        .on_click(move |_, window, cx| {
                                            magic.update(cx, |panels, cx| {
                                                panels.generate_review(window, cx)
                                            });
                                        }),
                                )
                            })
                            .child(
                                Button::new("feedback-close")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .tooltip("Close feedback")
                                    .on_click(move |_, _, cx| {
                                        close.update(cx, |panels, cx| {
                                            panels.toggle_drawer(Drawer::Feedback, cx);
                                            panels.status("Feedback panel closed.", cx);
                                        });
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(muted(autosave, cx))
                            .children(autosave_hint.map(|hint| muted(hint, cx)))
                            .when(generating, |this| {
                                this.child(muted("Magic Review is generating feedback.", cx))
                            }),
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
                            .child(caption("Completion", cx))
                            .child(div().text_sm().child(format!(
                                "{completed} of {required} required sections complete"
                            ))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(Self::footer_state(&form)),
                    ),
            )
            .into_any_element()
    }
}
