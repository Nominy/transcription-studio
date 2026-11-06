//! "Babel Review Grader" dialog: five category grades (1 is best) with rubric notes and
//! expandable correction examples; Generate/Regenerate, editable scores, Apply grades
//! (scores only, the reviewer's notes stay).

use std::rc::Rc;

use exaudio_babel_services::review::{CategoryGrade, grade_category_title};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::task::TaskPanels;
use crate::text;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::editor::panels::modal::{CloseModal, ModalFocus, dialog_surface, modal};
use exaudio_shell::editor::panels::ui::muted;

/// Score buttons: value and label (EN, RU).
const SCORE_LABELS: [(u8, &str, &str); 3] = [
    (1, "1 · Few or no issues", "1 · Почти без ошибок"),
    (2, "2 · Repeated issues", "2 · Повторяющиеся ошибки"),
    (3, "3 · Systemic issues", "3 · Системные ошибки"),
];

pub struct GraderPanel {
    panels: Entity<TaskPanels>,
    focus: ModalFocus,
    _subscription: Subscription,
}

impl GraderPanel {
    pub fn is_open(panels: &TaskPanels) -> bool {
        panels.grader.open
    }

    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panels = TaskPanels::for_model(&model, window, cx);
        let subscription = cx.observe_in(&panels, window, |this, panels, window, cx| {
            this.focus.sync(Self::is_open(panels.read(cx)), window, cx);
            cx.notify();
        });
        Self {
            panels,
            focus: ModalFocus::new(cx),
            _subscription: subscription,
        }
    }

    fn render_grade(&self, grade: &CategoryGrade, busy: bool, expanded: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let category = grade.category;
        let scores = h_flex()
            .gap_1()
            .flex_wrap()
            .children(SCORE_LABELS.iter().map(|&(score, en, ru)| {
                let p = self.panels.clone();
                Button::new(SharedString::from(format!("grade-{category}-{score}")))
                    .xsmall()
                    .label(text::localized(cx, en, ru))
                    .when(grade.score == score, |button| button.primary())
                    .when(grade.score != score, |button| button.outline())
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        p.update(cx, |panels, cx| panels.set_grade_score(category, score, cx));
                    })
            }));
        let examples = (!grade.samples.is_empty()).then(|| {
            let p = self.panels.clone();
            let count = grade.evidence_count.max(grade.samples.len());
            v_flex()
                .gap_1()
                .child(
                    Button::new(SharedString::from(format!("grade-examples-{category}")))
                        .xsmall()
                        .ghost()
                        .label(format!(
                            "{} {}",
                            if expanded { "▾" } else { "▸" },
                            text::localized_fmt(
                                cx,
                                "Examples · {count}",
                                "Примеры · {count}",
                                &[(
                                    "count",
                                    &text::count(
                                        count as u64,
                                        ("correction", "corrections"),
                                        ("исправление", "исправления", "исправлений"),
                                    ),
                                )],
                            )
                        ))
                        .on_click(move |_, _, cx| {
                            p.update(cx, |panels, cx| panels.toggle_grade_examples(category, cx));
                        }),
                )
                .when(expanded, |this| {
                    this.children(grade.samples.iter().map(|sample| {
                        v_flex()
                            .gap_0p5()
                            .pl_2()
                            .border_l_2()
                            .border_color(theme.border)
                            .child(muted(sample.note.clone(), cx))
                            .children(sample.before.clone().map(|before| {
                                div()
                                    .text_xs()
                                    .whitespace_normal()
                                    .child(text::localized_fmt(
                                        cx,
                                        "Original: {text}",
                                        "Исходный: {text}",
                                        &[("text", &before)],
                                    ))
                            }))
                            .children(sample.after.clone().map(|after| {
                                div()
                                    .text_xs()
                                    .whitespace_normal()
                                    .child(text::localized_fmt(
                                        cx,
                                        "Reviewed: {text}",
                                        "Проверенный: {text}",
                                        &[("text", &after)],
                                    ))
                            }))
                    }))
                })
        });
        v_flex()
            .gap_2()
            .p_3()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(grade_category_title(category).to_string()),
            )
            .child(scores)
            .when(!grade.note.is_empty(), |this| {
                this.child(muted(grade.note.clone(), cx))
            })
            .children(examples)
    }
}

/// The Review Grader accent (teal).
fn accent() -> Hsla {
    rgb(0x0f766e).into()
}

impl Render for GraderPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels = self.panels.read(cx);
        if !Self::is_open(panels) {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let grader = &panels.grader;
        let busy = grader.busy;
        let has_result = grader.result.is_some();
        let grades: Vec<Div> = grader
            .result
            .iter()
            .flat_map(|result| &result.grades)
            .map(|grade| {
                self.render_grade(grade, busy, grader.expanded.contains(grade.category), cx)
            })
            .collect();
        let close = self.panels.clone();
        let generate = self.panels.clone();
        let apply = self.panels.clone();
        let header = h_flex()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text::localized(cx, "Grader", "Grader")),
            )
            .child(
                Button::new("grader-close")
                    .small()
                    .ghost()
                    .icon(IconName::Close)
                    .label(text::localized(cx, "Close", "Закрыть"))
                    .on_click(move |_, _, cx| {
                        close.update(cx, |panels, cx| panels.close_grader(cx))
                    }),
            );
        let status = v_flex()
            .gap_1()
            .children(grader.status.clone().map(|(text, error)| {
                div()
                    .text_sm()
                    .text_color(if error {
                        theme.danger
                    } else {
                        theme.foreground
                    })
                    .child(text)
            }))
            .when(busy, |this| {
                this.child(muted(text::localized(cx, "Working…", "Выполняется…"), cx))
            });
        let footer = h_flex()
            .gap_2()
            .justify_end()
            .p_3()
            .border_t_1()
            .border_color(theme.border)
            .child(
                Button::new("grader-generate")
                    .small()
                    .outline()
                    .label(if has_result {
                        text::localized(cx, "Generate again", "Сгенерировать заново")
                    } else {
                        text::localized(cx, "Generate grades", "Сгенерировать оценки")
                    })
                    .loading(busy)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        generate.update(cx, |panels, cx| panels.generate_grades(window, cx));
                    }),
            )
            .child(
                Button::new("grader-apply")
                    .small()
                    .primary()
                    .label(text::localized(cx, "Apply grades", "Применить оценки"))
                    .disabled(busy || !has_result)
                    .on_click(move |_, window, cx| {
                        apply.update(cx, |panels, cx| panels.apply_grades(window, cx));
                    }),
            );
        let surface = dialog_surface("review-grader", rems(35.), accent(), cx)
            .child(header)
            .child(
                div()
                    .id("grader-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(v_flex().gap_3().p_3().child(status).children(grades)),
            )
            .child(footer);
        let p = self.panels.clone();
        let close: CloseModal =
            Rc::new(move |_, cx| p.update(cx, |panels, cx| panels.close_grader(cx)));
        modal(surface, &self.focus, close, false).into_any_element()
    }
}
