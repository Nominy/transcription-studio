//! Interactive Magic Review workspace (the Review Helper's review dialog): per-change cards
//! with exact before/after evidence, the model's suggestion, inline template search/match,
//! per-card and overall comments, template ideas, Refresh and Apply.

use std::collections::HashMap;
use std::rc::Rc;

use exaudio_babel_services::review::{
    EvidenceDetail, ProposalDecision, ReviewerCard, ReviewerSuggestion, TemplateSearch,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::task::TaskPanels;
use crate::text;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::editor::panels::modal::{CloseModal, ModalFocus, badge, dialog_surface, modal};
use exaudio_shell::editor::panels::ui::{caption, muted};

/// Inputs owned by one card.
struct CardInputs {
    comment: Entity<TextareaState>,
    search: Entity<InputState>,
    /// The template search was open at the last sync (it takes focus when it opens).
    search_open: bool,
}

pub struct ReviewWorkspacePanel {
    panels: Entity<TaskPanels>,
    session_comment: Entity<TextareaState>,
    cards: HashMap<String, CardInputs>,
    focus: ModalFocus,
    _subscriptions: Vec<Subscription>,
}

impl ReviewWorkspacePanel {
    pub fn is_open(panels: &TaskPanels) -> bool {
        panels.review.open
    }

    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panels = TaskPanels::for_model(&model, window, cx);
        let session_comment = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder(text::localized(
                    cx,
                    "Anything else about this review?",
                    "Что-нибудь ещё об этой проверке?",
                ))
        });
        let subscriptions = vec![
            cx.observe_in(&panels, window, |this, panels, window, cx| {
                this.focus.sync(Self::is_open(panels.read(cx)), window, cx);
                this.sync_inputs(window, cx);
                cx.notify();
            }),
            cx.subscribe(&session_comment, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.panels.update(cx, |panels, cx| {
                        panels.review.workspace.set_session_comment(&value);
                        cx.notify();
                    });
                }
            }),
        ];
        let mut this = Self {
            panels,
            session_comment,
            cards: HashMap::new(),
            focus: ModalFocus::new(cx),
            _subscriptions: subscriptions,
        };
        this.sync_inputs(window, cx);
        this
    }

    /// Creates inputs for new cards and mirrors comments adopted from the backend.
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = &self.panels.read(cx).review.workspace;
        let cards: Vec<(String, String, bool)> = workspace
            .session()
            .map(|session| {
                session
                    .cards
                    .iter()
                    .map(|card| {
                        (
                            card.id.clone(),
                            workspace.card_comment(&card.id).to_string(),
                            workspace.template_search(&card.id).is_some(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let session_comment = workspace.session_comment().to_string();
        set_if_changed(&self.session_comment, &session_comment, window, cx);
        self.cards
            .retain(|id, _| cards.iter().any(|(card, _, _)| card == id));
        for (card_id, comment, search_open) in cards {
            if !self.cards.contains_key(&card_id) {
                let inputs = self.new_card_inputs(&card_id, window, cx);
                self.cards.insert(card_id.clone(), inputs);
            }
            let Some(inputs) = self.cards.get_mut(&card_id) else {
                continue;
            };
            let opened = search_open && !inputs.search_open;
            inputs.search_open = search_open;
            let (comment_input, search_input) = (inputs.comment.clone(), inputs.search.clone());
            set_if_changed(&comment_input, &comment, window, cx);
            if !search_open && !search_input.read(cx).value().is_empty() {
                search_input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            if opened {
                search_input.update(cx, |input, cx| input.focus(window, cx));
            }
        }
    }

    fn new_card_inputs(
        &mut self,
        card_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CardInputs {
        let comment = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder(text::localized(
                    cx,
                    "What should change?",
                    "Что нужно изменить?",
                ))
        });
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(text::localized(
                cx,
                "Search templates…",
                "Поиск шаблонов…",
            ))
        });
        let id = card_id.to_string();
        self._subscriptions.push(cx.subscribe(
            &comment,
            move |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.panels.update(cx, |panels, cx| {
                        panels.review.workspace.set_card_comment(&id, &value);
                        cx.notify();
                    });
                }
            },
        ));
        let id = card_id.to_string();
        self._subscriptions.push(cx.subscribe_in(
            &search,
            window,
            move |this, input, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.panels.update(cx, |panels, cx| {
                        if panels.review.workspace.template_search(&id).is_some() {
                            panels.search_card_templates(&id, &value, window, cx);
                        }
                    });
                }
            },
        ));
        CardInputs {
            comment,
            search,
            search_open: false,
        }
    }

    fn render_card(&self, card: &ReviewerCard, busy: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let panels = self.panels.read(cx);
        let workspace = &panels.review.workspace;
        let expanded = workspace.is_expanded(&card.id);
        let matched = card.matched_template_id.is_some();
        let id = card.id.clone();
        let p = self.panels.clone();
        let header = h_flex()
            .id(SharedString::from(format!("rw-card-{id}")))
            .gap_2()
            .items_start()
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                p.update(cx, |panels, cx| {
                    panels.review.workspace.toggle_expanded(&id);
                    cx.notify();
                })
            })
            .child(
                div()
                    .mt_1()
                    .size_2()
                    .flex_none()
                    .rounded_full()
                    .bg(if matched {
                        theme.success
                    } else {
                        theme.muted_foreground
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!(
                                "#{}: {}",
                                card.change_index,
                                card.display_summary()
                            )),
                    )
                    .when(!expanded, |this| {
                        this.child(muted(
                            card.template_title.clone().unwrap_or_else(|| {
                                text::localized(cx, "No template", "Шаблон не выбран").to_string()
                            }),
                            cx,
                        ))
                    }),
            )
            .when(matched, |this| {
                let p = self.panels.clone();
                let id = card.id.clone();
                this.child(
                    Button::new(SharedString::from(format!("rw-unmatch-{id}")))
                        .xsmall()
                        .danger()
                        .label(text::localized(cx, "Remove match", "Убрать совпадение"))
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            p.update(cx, |panels, cx| {
                                panels.clear_card_template(id.clone(), window, cx)
                            });
                        }),
                )
            })
            .child(badge(
                if card.kind.is_empty() {
                    text::localized(cx, "Unknown", "Неизвестно")
                } else {
                    card.kind.as_str()
                },
                theme.warning,
                cx,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(if expanded { "▲" } else { "▼" }),
            );

        let mut body = v_flex().gap_2();
        if expanded {
            if card.opinion_text.is_some() || card.rationale.is_some() {
                body = body.child(
                    v_flex()
                        .gap_1()
                        .p_2()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.warning)
                        .bg(theme.warning.opacity(0.08))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.warning)
                                .child(text::localized(cx, "Suggestion", "Предложение")),
                        )
                        .children(
                            card.opinion_text
                                .clone()
                                .map(|text| div().text_sm().child(text)),
                        )
                        .children(card.rationale.clone().map(|text| muted(text, cx))),
                );
            }
            body = body.children(evidence_block(card, cx));
            body = body.child(self.render_template_panel(card, busy, cx));
            if let Some(inputs) = self.cards.get(&card.id) {
                body = body.child(
                    v_flex()
                        .gap_1()
                        .child(caption(
                            text::localized(cx, "Your comment", "Ваш комментарий"),
                            cx,
                        ))
                        .child(Textarea::new(&inputs.comment).disabled(busy)),
                );
            }
        }
        v_flex()
            .gap_2()
            .p_2()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .child(header)
            .when(expanded, |this| this.child(body))
    }

    fn render_template_panel(&self, card: &ReviewerCard, busy: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let search = self
            .panels
            .read(cx)
            .review
            .workspace
            .template_search(&card.id)
            .cloned();
        let id = card.id.clone();
        let p = self.panels.clone();
        let Some(search) = search else {
            return h_flex().child(
                Button::new(SharedString::from(format!("rw-open-search-{id}")))
                    .small()
                    .outline()
                    .label(if card.matched_template_id.is_some() {
                        text::localized(cx, "Change template", "Сменить шаблон")
                    } else {
                        text::localized(cx, "Match template", "Подобрать шаблон")
                    })
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        p.update(cx, |panels, cx| {
                            panels.review.workspace.set_template_search_open(&id, true);
                            cx.notify();
                        });
                    }),
            );
        };
        let title = card
            .template_title
            .clone()
            .or_else(|| card.matched_template_id.clone())
            .unwrap_or_else(|| text::localized(cx, "No template", "Шаблон не выбран").to_string());
        let source = card.match_source_label().to_string();
        let input = self.cards.get(&card.id).map(|inputs| inputs.search.clone());
        v_flex()
            .gap_2()
            .p_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(caption(
                text::localized(cx, "Template match", "Совпадение с шаблоном"),
                cx,
            ))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .children(
                card.template_description
                    .clone()
                    .map(|text| muted(text, cx)),
            )
            .child(muted(source, cx))
            .children(card.previous_template().map(|previous| {
                muted(
                    text::localized_fmt(
                        cx,
                        "Previously matched: {template}",
                        "Ранее подобран: {template}",
                        &[("template", previous)],
                    ),
                    cx,
                )
            }))
            .children(input.map(|input| Input::new(&input).small().disabled(busy)))
            .child(self.render_search_results(card, &search, busy, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child({
                        let p = self.panels.clone();
                        let id = card.id.clone();
                        Button::new(SharedString::from(format!("rw-search-clear-{id}")))
                            .small()
                            .danger()
                            .label(text::localized(cx, "Remove match", "Убрать совпадение"))
                            .disabled(busy || card.matched_template_id.is_none())
                            .on_click(move |_, window, cx| {
                                p.update(cx, |panels, cx| {
                                    panels.clear_card_template(id.clone(), window, cx)
                                });
                            })
                    })
                    .child({
                        let p = self.panels.clone();
                        let id = card.id.clone();
                        Button::new(SharedString::from(format!("rw-search-cancel-{id}")))
                            .small()
                            .ghost()
                            .label(text::localized(cx, "Cancel", "Отмена"))
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                p.update(cx, |panels, cx| {
                                    panels.review.workspace.set_template_search_open(&id, false);
                                    cx.notify();
                                });
                            })
                    }),
            )
    }

    fn render_search_results(
        &self,
        card: &ReviewerCard,
        search: &TemplateSearch,
        busy: bool,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        if search.loading {
            return muted(text::localized(cx, "Searching…", "Поиск…"), cx).into_any_element();
        }
        if let Some(error) = &search.error {
            return div()
                .text_xs()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element();
        }
        if search.results.is_empty() {
            return if search.query.trim().is_empty() {
                div().into_any_element()
            } else {
                muted(text::localized(cx, "No results.", "Ничего не найдено."), cx)
                    .into_any_element()
            };
        }
        let results = search.results.iter().map(|template| {
            let p = self.panels.clone();
            let card_id = card.id.clone();
            let template_id = template.id.clone();
            v_flex()
                .gap_1()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(template.title.clone()),
                        )
                        .when(!template.category.is_empty(), |this| {
                            this.child(badge(&template.category, theme.warning, cx))
                        }),
                )
                .when(!template.description.is_empty(), |this| {
                    this.child(muted(template.description.clone(), cx))
                })
                .child(
                    h_flex().child(
                        Button::new(SharedString::from(format!(
                            "rw-select-{}-{}",
                            card.id, template.id
                        )))
                        .xsmall()
                        .primary()
                        .label(text::localized(cx, "Select", "Выбрать"))
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            p.update(cx, |panels, cx| {
                                panels.match_card_template(
                                    card_id.clone(),
                                    template_id.clone(),
                                    window,
                                    cx,
                                )
                            });
                        }),
                    ),
                )
        });
        div()
            .id(SharedString::from(format!("rw-results-{}", card.id)))
            .max_h(px(200.))
            .overflow_y_scroll()
            .child(v_flex().gap_1().children(results))
            .into_any_element()
    }

    fn render_idea(&self, idea: &ReviewerSuggestion, busy: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let pending = idea.is_pending();
        let decide = |decision: ProposalDecision, id: &'static str, label: &'static str| {
            let p = self.panels.clone();
            let proposal = idea.proposal_id.clone();
            let button = Button::new(SharedString::from(format!(
                "rw-idea-{id}-{}",
                idea.proposal_id
            )))
            .xsmall()
            .label(label)
            .disabled(busy || !pending)
            .on_click(move |_, window, cx| {
                p.update(cx, |panels, cx| {
                    panels.decide_template_idea(proposal.clone(), decision, window, cx)
                });
            });
            match decision {
                ProposalDecision::Approved => button.primary(),
                ProposalDecision::Rejected => button.outline(),
            }
        };
        let title = if idea.title.trim().is_empty() {
            idea.target_template_id.clone().unwrap_or_else(|| {
                text::localized(cx, "Untitled suggestion", "Предложение без названия").to_string()
            })
        } else {
            idea.title.clone()
        };
        v_flex()
            .gap_1()
            .p_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_1()
                    .child(badge(&idea.operation, theme.primary, cx))
                    .child(badge(&idea.category, theme.warning, cx))
                    .child(badge(
                        &idea.decision,
                        if pending {
                            theme.muted_foreground
                        } else {
                            theme.success
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .children(
                idea.description
                    .clone()
                    .map(|text| div().text_xs().child(text)),
            )
            .child(muted(
                text::localized_fmt(
                    cx,
                    "Reason: {reason}",
                    "Причина: {reason}",
                    &[("reason", &idea.reason)],
                ),
                cx,
            ))
            .when(!idea.report_texts.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_0p5()
                        .p_2()
                        .rounded(theme.radius)
                        .bg(theme.muted)
                        .child(caption(
                            text::localized(cx, "Proposed text", "Предлагаемый текст"),
                            cx,
                        ))
                        .children(
                            idea.report_texts.iter().map(|text| {
                                div().text_xs().whitespace_normal().child(text.clone())
                            }),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(decide(
                        ProposalDecision::Approved,
                        "approve",
                        text::localized(cx, "Approve", "Одобрить"),
                    ))
                    .child(decide(
                        ProposalDecision::Rejected,
                        "reject",
                        text::localized(cx, "Reject", "Отклонить"),
                    )),
            )
    }
}

/// Before/After panes for a text diff, else the monospace raw evidence.
fn evidence_block(card: &ReviewerCard, cx: &App) -> Option<Div> {
    let theme = cx.theme();
    let pane = |label: &'static str, text: &str, extra: Option<String>| {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .p_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(caption(label, cx))
            .child(
                div()
                    .text_sm()
                    .whitespace_normal()
                    .child(if text.is_empty() {
                        text::localized(cx, "(empty)", "(пусто)").to_string()
                    } else {
                        text.to_string()
                    }),
            )
            .children(extra.map(|diff| muted(diff, cx)))
    };
    match card.evidence_view()? {
        EvidenceDetail::TextDiff {
            before,
            after,
            inline_diff,
        } => Some(
            h_flex()
                .gap_2()
                .items_start()
                .child(pane(text::localized(cx, "Before", "До"), &before, None))
                .child(pane(
                    text::localized(cx, "After", "После"),
                    &after,
                    inline_diff,
                )),
        ),
        EvidenceDetail::Raw(text) => Some(
            v_flex()
                .gap_1()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .child(caption(
                    text::localized(cx, "Evidence", "Подтверждение"),
                    cx,
                ))
                .child(
                    div()
                        .text_xs()
                        .font_family("monospace")
                        .whitespace_normal()
                        .child(text),
                ),
        ),
    }
}

fn set_if_changed(input: &Entity<TextareaState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value() != value {
        let value = value.to_string();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

impl Render for ReviewWorkspacePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels = self.panels.read(cx);
        if !Self::is_open(panels) {
            return div().into_any_element();
        }
        let theme = cx.theme().clone();
        let review = &panels.review;
        let busy = review.busy.is_some();
        let session = review.workspace.session().cloned();
        let has_session = session.is_some();
        let status = review.status.clone();
        let busy_label = review.busy.clone();
        let grading = review.grading_status.clone();
        let ideas_open = review.ideas_open;
        let save_note = if review.workspace.comment_save_in_flight() {
            Some(text::localized(
                cx,
                "Saving comments…",
                "Сохранение комментариев…",
            ))
        } else if review.workspace.comments_dirty() {
            Some(text::localized(
                cx,
                "Unsaved comments",
                "Несохранённые комментарии",
            ))
        } else {
            None
        };

        let p = self.panels.clone();
        let action =
            move |id: &'static str,
                  label: &'static str,
                  disabled: bool,
                  run: fn(&mut TaskPanels, &mut Window, &mut Context<TaskPanels>)| {
                let p = p.clone();
                Button::new(id)
                    .small()
                    .label(label)
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        p.update(cx, |panels, cx| run(panels, window, cx))
                    })
            };

        let header = v_flex()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(text::localized(cx, "Review", "Проверка")),
                    )
                    .child(
                        action(
                            "rw-refresh",
                            text::localized(cx, "Refresh", "Обновить"),
                            !has_session || busy,
                            TaskPanels::refresh_review,
                        )
                        .outline(),
                    )
                    .child(
                        action(
                            "rw-apply-top",
                            text::localized(cx, "Apply", "Применить"),
                            !has_session || busy,
                            |panels, window, cx| panels.finalize_review("apply", window, cx),
                        )
                        .primary(),
                    )
                    .child(
                        action(
                            "rw-close",
                            text::localized(cx, "Close", "Закрыть"),
                            busy,
                            TaskPanels::close_review,
                        )
                        .ghost()
                        .icon(IconName::Close),
                    ),
            )
            .children(busy_label.map(|label| {
                muted(
                    text::localized_fmt(
                        cx,
                        "Working… {label}",
                        "Выполняется… {label}",
                        &[("label", &label)],
                    ),
                    &*cx,
                )
            }))
            .children(status.map(|(text, error)| {
                div()
                    .text_xs()
                    .text_color(if error {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(text)
            }))
            .children(grading.map(|(text, error)| {
                div()
                    .text_xs()
                    .text_color(if error {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(text)
            }));

        let mut body = v_flex().gap_3().p_3();
        match &session {
            None if busy => {
                body = body.child(muted(
                    text::localized(cx, "Preparing review…", "Подготовка проверки…"),
                    cx,
                ))
            }
            None => {
                body = body.child(muted(
                    text::localized(cx, "Review unavailable.", "Проверка недоступна."),
                    cx,
                ))
            }
            Some(session) => {
                let matched = session
                    .cards
                    .iter()
                    .filter(|card| card.matched_template_id.is_some())
                    .count();
                let p = self.panels.clone();
                let q = self.panels.clone();
                body = body
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(div().flex_1().text_sm().child(text::localized_fmt(
                                cx,
                                "{changes} · {matched} matched · {suggestions}",
                                "{changes} · подобрано: {matched} · {suggestions}",
                                &[
                                    (
                                        "changes",
                                        &text::count(
                                            session.cards.len() as u64,
                                            ("change", "changes"),
                                            ("изменение", "изменения", "изменений"),
                                        ),
                                    ),
                                    ("matched", &matched.to_string()),
                                    (
                                        "suggestions",
                                        &text::count(
                                            session.suggestions.len() as u64,
                                            ("suggestion", "suggestions"),
                                            ("предложение", "предложения", "предложений"),
                                        ),
                                    ),
                                ],
                            )))
                            .children(save_note.map(|note| muted(note, cx)))
                            .child(
                                Button::new("rw-expand-all")
                                    .xsmall()
                                    .ghost()
                                    .label(text::localized(cx, "Expand all", "Развернуть все"))
                                    .on_click(move |_, _, cx| {
                                        p.update(cx, |panels, cx| {
                                            panels.review.workspace.set_all_expanded(true);
                                            cx.notify();
                                        })
                                    }),
                            )
                            .child(
                                Button::new("rw-collapse-all")
                                    .xsmall()
                                    .ghost()
                                    .label(text::localized(cx, "Collapse all", "Свернуть все"))
                                    .on_click(move |_, _, cx| {
                                        q.update(cx, |panels, cx| {
                                            panels.review.workspace.set_all_expanded(false);
                                            cx.notify();
                                        })
                                    }),
                            )
                            .child(
                                action(
                                    "rw-rescan",
                                    text::localized(
                                        cx,
                                        "Rescan for suggestions",
                                        "Искать предложения снова",
                                    ),
                                    busy,
                                    TaskPanels::rescan_template_ideas,
                                )
                                .xsmall()
                                .outline(),
                            ),
                    )
                    .when(session.cards.is_empty(), |this| {
                        this.child(muted(
                            text::localized(
                                cx,
                                "No changes found in this review.",
                                "В этой проверке нет изменений.",
                            ),
                            cx,
                        ))
                    });
                for card in &session.cards {
                    body = body.child(self.render_card(card, busy, cx));
                }
                let toggle = self.panels.clone();
                let mut ideas = v_flex()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(text::localized_fmt(
                                        cx,
                                        "Template ideas ({count})",
                                        "Идеи шаблонов ({count})",
                                        &[("count", &session.suggestions.len().to_string())],
                                    )),
                            )
                            .child(
                                Button::new("rw-ideas-toggle")
                                    .xsmall()
                                    .ghost()
                                    .label(if ideas_open {
                                        text::localized(cx, "Hide", "Скрыть")
                                    } else {
                                        text::localized(cx, "Show", "Показать")
                                    })
                                    .on_click(move |_, _, cx| {
                                        toggle.update(cx, |panels, cx| {
                                            panels.review.ideas_open = !panels.review.ideas_open;
                                            cx.notify();
                                        })
                                    }),
                            ),
                    );
                if ideas_open {
                    ideas = ideas
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(caption(
                                            text::localized(
                                                cx,
                                                "Overall comment",
                                                "Общий комментарий",
                                            ),
                                            cx,
                                        ))
                                        .child(muted(
                                            text::localized(cx, "Optional", "Необязательно"),
                                            cx,
                                        )),
                                )
                                .child(Textarea::new(&self.session_comment).disabled(busy)),
                        )
                        .when(session.suggestions.is_empty(), |this| {
                            this.child(muted(
                                text::localized(
                                    cx,
                                    "No template suggestions yet.",
                                    "Предложений по шаблонам пока нет.",
                                ),
                                cx,
                            ))
                        })
                        .children(
                            session
                                .suggestions
                                .iter()
                                .map(|idea| self.render_idea(idea, busy, cx)),
                        );
                }
                body = body.child(ideas);
            }
        }

        let footer = h_flex()
            .gap_2()
            .justify_end()
            .p_3()
            .border_t_1()
            .border_color(theme.border)
            .child(
                action(
                    "rw-skip",
                    text::localized(cx, "Apply without review", "Применить без проверки"),
                    !has_session || busy,
                    |panels, window, cx| panels.finalize_review("skip", window, cx),
                )
                .outline(),
            )
            .child(
                action(
                    "rw-apply",
                    text::localized(cx, "Apply and sync", "Применить и синхронизировать"),
                    !has_session || busy,
                    |panels, window, cx| panels.finalize_review("apply", window, cx),
                )
                .primary(),
            );

        let surface = dialog_surface("review-workspace", rems(61.25), theme.warning, cx)
            .child(header)
            .child(
                div()
                    .id("review-workspace-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(body),
            )
            .child(footer);
        let p = self.panels.clone();
        let close: CloseModal = Rc::new(move |window, cx| {
            p.update(cx, |panels, cx| {
                if panels.review.busy.is_none() {
                    panels.close_review(window, cx);
                }
            })
        });
        modal(surface, &self.focus, close, true).into_any_element()
    }
}
