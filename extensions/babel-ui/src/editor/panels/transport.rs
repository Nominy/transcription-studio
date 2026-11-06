//! Transport/action strip above the transcript: track filter, segment navigation, 1 s overlay,
//! undo/redo, problem counts with navigation, drawer toggles and the review actions (drop /
//! skip / flag / save / submit, or the read-only notice and actions). A second row carries the
//! diff toolbar when the task has versions to compare.

use exaudio_babel_domain::editor_model::diff_toolbar::{
    AnnotationDiffMetrics, DiffMetrics, DiffToolbarState, MetricTriplet, TranscriptionDiffMetrics,
};
use exaudio_babel_domain::editor_model::review_actions::{
    ReadOnlyActionCommand, ReadOnlyNotice, ReviewActionStrip,
};
use exaudio_babel_domain::editor_model::{AssessmentPhase, EditorSession};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::editor_model::SaveStatus;

use super::dialogs::submit;
use super::flows::skip_disabled_tooltip;
use super::task::{Drawer, TaskPanels, TaskPanelsEvent};
use crate::text::{count, failed, localized, localized_fmt};
use exaudio_shell::ShellHandle;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::editor::panels::ui::{
    Choice, choice, muted, stable_text, tabular_figures, vertical_rule,
};
use exaudio_shell::screens::local_files;

pub struct TransportStrip {
    model: Entity<EditorModel>,
    panels: Entity<TaskPanels>,
    _subscriptions: Vec<Subscription>,
}

impl TransportStrip {
    pub fn new(
        model: Entity<EditorModel>,
        shell: ShellHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let panels = TaskPanels::for_model(&model, window, cx);
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe(&panels, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &panels,
                window,
                move |this, _, event, window, cx| match event {
                    TaskPanelsEvent::Leave => {
                        let mode = this.model.read(cx).origin().mode_id.clone();
                        let shell = shell.clone();
                        window.defer(cx, move |window, cx| shell.show_mode(&mode, window, cx));
                    }
                },
            ),
        ];
        Self {
            model,
            panels,
            _subscriptions: subscriptions,
        }
    }

    fn render_problems(&self, cx: &mut Context<Self>) -> Option<Div> {
        let doc = self.model.read(cx).doc();
        let counts = doc.problem_counts();
        if counts.errors == 0 && counts.warnings == 0 {
            return None;
        }
        let label = problem_summary(cx, counts.errors, counts.warnings);
        // plurals and both kinds: the count changing never moves the step buttons.
        let digits = doc.project().segments.len().max(1).to_string().len() as u32;
        let most = 10_usize.pow(digits) - 1;
        let label_slot = stable_text(
            label,
            [
                problem_summary(cx, most, 0),
                problem_summary(cx, 0, most),
                problem_summary(cx, most, most),
            ],
        )
        .font_features(tabular_figures());
        let theme = cx.theme();
        let color = if counts.errors > 0 {
            theme.danger
        } else {
            theme.warning
        };
        let nav = |id: &'static str, icon: IconName, tooltip: &'static str, direction: i32| {
            let model = self.model.clone();
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tooltip)
                .on_click(move |_, _, cx| {
                    model.update(cx, |model, cx| {
                        model.run(cx, |doc| doc.navigate_problem_rows(direction));
                    });
                })
        };
        Some(
            h_flex()
                .h(rems(1.75))
                .flex_none()
                .pl_2()
                .pr_0p5()
                .gap_1()
                .rounded(theme.radius)
                .border_1()
                .border_color(color.opacity(0.5))
                .bg(color.opacity(0.08))
                .child(div().text_sm().text_color(color).child(label_slot))
                .child(nav(
                    "problem-previous",
                    IconName::ChevronUp,
                    localized(cx, "Previous problem", "Предыдущая проблема"),
                    -1,
                ))
                .child(nav(
                    "problem-next",
                    IconName::ChevronDown,
                    localized(cx, "Next problem", "Следующая проблема"),
                    1,
                )),
        )
    }

    fn render_export(&self, cx: &App) -> impl IntoElement + use<> {
        let model = self.model.clone();
        let json_label = localized(cx, "Babel JSON…", "Babel JSON…");
        let folder_label = localized(cx, "Project folder…", "Папка проекта…");
        Button::new("export")
            .ghost()
            .small()
            .icon(IconName::FolderOpen)
            .label(localized(cx, "Export", "Экспорт"))
            .tooltip(localized(cx, "Export this task", "Экспортировать задачу"))
            .dropdown_menu(move |menu, _, _| {
                let json = model.clone();
                let folder = model.clone();
                menu.item(
                    PopupMenuItem::new(json_label).on_click(move |_, window, cx| {
                        crate::screens::local_files::export_babel_json(&json, window, cx);
                    }),
                )
                .item(
                    PopupMenuItem::new(folder_label).on_click(move |_, window, cx| {
                        local_files::export_project_folder(&folder, window, cx);
                    }),
                )
            })
    }

    fn render_drawer_toggles(&self, cx: &mut Context<Self>) -> Div {
        let panels = self.panels.read(cx);
        let drawer = panels.drawer;
        let has_feedback = panels.feedback.is_some();
        let opened = localized(cx, "Feedback panel opened", "Панель отзыва открыта");
        let closed = localized(cx, "Feedback panel closed", "Панель отзыва закрыта");
        let toggle = |id: &'static str,
                      icon: IconName,
                      label: &'static str,
                      tooltip: &'static str,
                      target: Drawer| {
            let panels = self.panels.clone();
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .label(label)
                .tooltip(tooltip)
                .selected(drawer == target)
                .on_click(move |_, _, cx| {
                    panels.update(cx, |panels, cx| {
                        let opening = panels.drawer != target;
                        panels.toggle_drawer(target, cx);
                        if target == Drawer::Feedback {
                            panels.status(if opening { opened } else { closed }, cx);
                        }
                    });
                })
        };
        h_flex()
            .gap_1()
            .when(has_feedback, |this| {
                this.child(toggle(
                    "toggle-feedback",
                    IconName::FileText,
                    localized(cx, "Feedback", "Отзыв"),
                    localized(
                        cx,
                        "Leave feedback for the transcriber",
                        "Оставить отзыв транскрибатору",
                    ),
                    Drawer::Feedback,
                ))
            })
            .child(toggle(
                "toggle-assistant",
                IconName::Bot,
                localized(cx, "Assistant", "Помощник"),
                localized(
                    cx,
                    "Review and drafting tools",
                    "Инструменты ревью и черновиков",
                ),
                Drawer::Assistant,
            ))
    }

    fn render_review_actions(&self, cx: &mut Context<Self>) -> Div {
        let doc = self.model.read(cx).doc();
        let project = doc.project();
        match EditorSession::review_action_strip(project) {
            ReviewActionStrip::Editable { drop_claim } => {
                let panels = self.panels.read(cx);
                let flows = &panels.flows;
                let session = doc.session();
                let eligibility = panels
                    .state
                    .borrow()
                    .view(session)
                    .submit_eligibility(project);
                let submit_copy = submit::copy(eligibility.kind, cx);
                let submit_tooltip = eligibility.blocker.map_or(
                    localized(cx, "Submit this task", "Отправить задачу"),
                    |blocker| blocker.reason(),
                );
                let skip = EditorSession::skip_chunk_available(project).then(|| {
                    (
                        flows.skip.pending,
                        EditorSession::skip_chunk_allowed(project),
                    )
                });
                let admin_flag = EditorSession::admin_flag_available(project);
                let save_status = session.save_status();
                let save_clickable =
                    session.is_dirty || session.save_has_error || doc.has_pending_drafts();
                let submit_pending = flows.submit.pending;
                let drop_pending = flows.drop_pending;

                let mut row = h_flex().gap_1p5();
                if let Some(drop) = drop_claim {
                    let panels = self.panels.clone();
                    let dropping = drop.is_dropping || drop_pending;
                    let releasing = localized(cx, "Releasing…", "Отпускаем…");
                    let release = localized(cx, "Release task", "Отпустить задачу");
                    row = row.child(
                        Button::new("drop-claim")
                            .outline()
                            .small()
                            .child(stable_text(
                                if drop.is_dropping { releasing } else { release },
                                [releasing, release],
                            ))
                            .accessibility_label(release)
                            .tooltip(localized(
                                cx,
                                "Let another worker take this task",
                                "Передать задачу другому исполнителю",
                            ))
                            .loading(drop_pending)
                            .disabled(dropping)
                            .on_click(move |_, window, cx| {
                                panels.update(cx, |panels, cx| panels.drop_claim(window, cx));
                            }),
                    );
                }
                if let Some((pending, allowed)) = skip {
                    let panels = self.panels.clone();
                    row = row.child(
                        Button::new("skip-chunk")
                            .outline()
                            .small()
                            .label(localized(cx, "Skip", "Пропустить"))
                            .tooltip(if allowed {
                                localized(
                                    cx,
                                    "Skip this chunk and go back to the project",
                                    "Пропустить этот чанк и вернуться к проекту",
                                )
                            } else {
                                skip_disabled_tooltip(cx)
                            })
                            .disabled(pending || !allowed)
                            .on_click(move |_, window, cx| {
                                panels.update(cx, |panels, cx| panels.request_skip(window, cx));
                            }),
                    );
                }
                if admin_flag {
                    let panels = self.panels.clone();
                    row = row.child(
                        Button::new("flag-recording")
                            .outline()
                            .small()
                            .label(localized(cx, "Flag recording", "Пометить запись"))
                            .on_click(move |_, window, cx| {
                                panels
                                    .update(cx, |panels, cx| panels.request_admin_flag(window, cx));
                            }),
                    );
                }
                let (save_label, save_tooltip) = save_copy(cx, &save_status, save_clickable);
                let save_panels = self.panels.clone();
                let submit_panels = self.panels.clone();
                let saving = matches!(save_status, SaveStatus::Saving);
                let submitting = localized(cx, "Submitting…", "Отправка…");
                // Labels keep the width of their widest state, so saving or submitting never
                // moves the buttons to their left.
                row.child(
                    Button::new("save-progress")
                        .outline()
                        .small()
                        .child(stable_text(save_label, save_labels(cx)))
                        .accessibility_label(save_label)
                        .tooltip(save_tooltip)
                        .loading(saving)
                        .disabled(saving || !save_clickable)
                        .when(matches!(save_status, SaveStatus::Failed { .. }), |button| {
                            button.danger()
                        })
                        .on_click(move |_, window, cx| {
                            save_panels.update(cx, |panels, cx| panels.save(window, cx));
                        }),
                )
                .child(
                    // Ineligible submit stays clickable: the click explains the blocker (and
                    // opens the feedback drawer when that is what blocks).
                    Button::new("submit-review")
                        .small()
                        .child(stable_text(
                            if submit_pending {
                                submitting
                            } else {
                                submit_copy.trigger
                            },
                            [submitting, submit_copy.trigger],
                        ))
                        .accessibility_label(submit_copy.trigger)
                        .tooltip(submit_tooltip)
                        .loading(submit_pending)
                        .when(eligibility.is_allowed(), |button| button.primary())
                        .when(!eligibility.is_allowed(), |button| button.outline())
                        .on_click(move |_, window, cx| {
                            submit_panels
                                .update(cx, |panels, cx| panels.request_submit(window, cx));
                        }),
                )
            }
            ReviewActionStrip::ReadOnly { notice, actions } => {
                let mut row = h_flex().gap_1p5().when_some(notice, |this, notice| {
                    this.child(muted(read_only_notice(cx, notice), cx))
                });
                for action in actions {
                    let panels = self.panels.clone();
                    let command = action.command;
                    row = row.child(
                        Button::new(SharedString::from(format!("read-only-{command:?}")))
                            .outline()
                            .small()
                            .label(read_only_label(cx, command, action.disabled))
                            .disabled(action.disabled)
                            .on_click(move |_, window, cx| {
                                panels.update(cx, |panels, cx| {
                                    panels.read_only_action(command, window, cx)
                                });
                            }),
                    );
                }
                row
            }
        }
    }

    fn render_diff_toolbar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let doc = self.model.read(cx).doc();
        let state = self
            .panels
            .read(cx)
            .state
            .borrow()
            .view(doc.session())
            .diff_toolbar(doc.project());
        let relevant = !state.compare_options.is_empty()
            || state.is_diff_mode
            || state.show_reviewers_control
            || state.assessment_phase.is_some();
        if !relevant {
            return None;
        }
        let theme = cx.theme();
        let mut row = h_flex()
            .flex_wrap()
            .gap_x_3()
            .gap_y_1()
            .px_1()
            .min_h(rems(1.875))
            .text_sm();
        if state.diff_toggle_visible() {
            let model = self.model.clone();
            row = row.child(
                Switch::new("diff-toggle")
                    .checked(state.is_diff_mode)
                    .label(if state.annotation_mode {
                        localized(cx, "Annotation diff", "Сравнение аннотаций")
                    } else {
                        localized(cx, "Diff view", "Сравнение")
                    })
                    .disabled(!state.diff_toggle_enabled())
                    .on_click(move |_, _, cx| {
                        model.update(cx, |model, cx| {
                            model.update_doc(cx, |doc| doc.session_mut().toggle_diff_view());
                        });
                    }),
            );
        }
        if let Some(phase) = state.assessment_phase {
            row = row.child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded(theme.radius)
                    .bg(theme.secondary)
                    .text_xs()
                    .child(match phase {
                        AssessmentPhase::Practice => localized(cx, "Practice", "Тренировка"),
                        AssessmentPhase::Assessment => localized(cx, "Assessment", "Тест"),
                    }),
            );
        }
        if state.no_versions_hint_visible() {
            row = row.child(muted(
                if state.annotation_mode {
                    localized(
                        cx,
                        "No other versions to compare",
                        "Нет других версий для сравнения",
                    )
                } else {
                    localized(
                        cx,
                        "No previous versions to compare",
                        "Нет предыдущих версий для сравнения",
                    )
                },
                cx,
            ));
        }
        if state.compare_select_visible() {
            row = row
                .child(muted(localized(cx, "Compare with", "Сравнить с"), cx))
                .child(self.render_compare(&state, cx));
        }
        if state.unsaved_changes_badge_visible() {
            row = row.child(div().text_xs().text_color(theme.warning).child(localized(
                cx,
                "Unsaved changes",
                "Несохранённые изменения",
            )));
        }
        if state.show_reviewers_control
            && !state.annotation_mode
            && state.assessment_phase.is_none()
        {
            let model = self.model.clone();
            let shown_message =
                localized(cx, "Reviewer feedback shown", "Отзывы ревьюеров показаны");
            let hidden_message =
                localized(cx, "Reviewer feedback hidden", "Отзывы ревьюеров скрыты");
            row = row.child(
                Switch::new("reviewers-toggle")
                    .checked(state.reviewers_checked)
                    .label(localized(cx, "Reviewers", "Ревьюеры"))
                    .on_click(move |_, _, cx| {
                        model.update(cx, |model, cx| {
                            let shown = model.update_doc(cx, |doc| {
                                doc.with_workflow(|workflow| workflow.toggle_diff_reviewers())
                            });
                            model
                                .set_status(if shown { shown_message } else { hidden_message }, cx);
                        });
                    }),
            );
        }
        if state.is_diff_mode
            && let Some(metrics) = &state.metrics
        {
            row = row.child(div().flex_1()).child(render_metrics(metrics, cx));
        }
        Some(row)
    }

    fn render_compare(&self, state: &DiffToolbarState, cx: &App) -> AnyElement {
        if state.is_loading {
            return div()
                .text_sm()
                .child(localized(cx, "Loading…", "Загрузка…"))
                .into_any_element();
        }
        let options = state
            .compare_options
            .iter()
            .map(|option| Choice::new(option.id.clone(), option.label.clone()))
            .collect();
        let model = self.model.clone();
        let status = localized(cx, "Version selected", "Версия выбрана");
        choice(
            "diff-compare",
            state.selected_compare_id.as_ref(),
            options,
            localized(cx, "Select version…", "Выберите версию…"),
            move |action_id: String, _, cx| {
                model.update(cx, |model, cx| {
                    model.update_doc(cx, |doc| {
                        doc.with_workflow(|workflow| workflow.set_diff_compare_action(action_id));
                    });
                    model.set_status(status, cx);
                });
            },
        )
        .into_any_element()
    }
}

fn problem_summary(cx: &App, errors: usize, warnings: usize) -> String {
    crate::text::is_russian(cx);
    let term = |n: usize, en: (&str, &str), ru: (&str, &str, &str)| {
        (n > 0).then(|| count(n as u64, en, ru))
    };
    [
        term(
            errors,
            ("problem", "problems"),
            ("проблема", "проблемы", "проблем"),
        ),
        term(
            warnings,
            ("warning", "warnings"),
            ("предупреждение", "предупреждения", "предупреждений"),
        ),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}

/// Every label [`save_copy`] returns (the save button is as wide as the widest).
fn save_labels(cx: &App) -> [&'static str; 4] {
    [
        localized(cx, "Saving…", "Сохранение…"),
        localized(cx, "Retry save", "Повторить сохранение"),
        localized(cx, "Save", "Сохранить"),
        localized(cx, "Saved", "Сохранено"),
    ]
}

fn save_copy(cx: &App, status: &SaveStatus, clickable: bool) -> (&'static str, SharedString) {
    let [saving, retry, save, saved] = save_labels(cx);
    match status {
        SaveStatus::Saving => (
            saving,
            localized(cx, "Saving your changes", "Сохранение изменений").into(),
        ),
        SaveStatus::Failed { message } => (
            retry,
            match message {
                Some(message) => {
                    failed(cx, "Couldn't save.", "Не удалось сохранить.", message).into()
                }
                None => localized(
                    cx,
                    "Couldn't save. Try again.",
                    "Не удалось сохранить. Попробуйте снова.",
                )
                .into(),
            },
        ),
        SaveStatus::Dirty => (
            save,
            localized(
                cx,
                "You have unsaved changes",
                "Есть несохранённые изменения",
            )
            .into(),
        ),
        SaveStatus::Saved if clickable => (
            save,
            localized(
                cx,
                "You have unsaved changes",
                "Есть несохранённые изменения",
            )
            .into(),
        ),
        SaveStatus::Saved => (
            saved,
            localized(cx, "All changes saved", "Все изменения сохранены").into(),
        ),
    }
}

fn read_only_notice(cx: &App, notice: ReadOnlyNotice) -> &'static str {
    match notice {
        ReadOnlyNotice::FinalSubmitted => localized(
            cx,
            "Submitted. The team will review it and get back to you.",
            "Отправлено. Команда проверит и свяжется с вами.",
        ),
        ReadOnlyNotice::AssessmentSession => localized(
            cx,
            "This assessment is read only.",
            "Этот тест только для чтения.",
        ),
        ReadOnlyNotice::ReadOnlySession => localized(
            cx,
            "This task is read only.",
            "Эта задача только для чтения.",
        ),
    }
}

fn read_only_label(cx: &App, command: ReadOnlyActionCommand, disabled: bool) -> &'static str {
    match command {
        ReadOnlyActionCommand::RetryPractice => {
            localized(cx, "Practice again", "Тренироваться снова")
        }
        ReadOnlyActionCommand::TakeAssessment if disabled => {
            localized(cx, "Loading the assessment…", "Загрузка теста…")
        }
        ReadOnlyActionCommand::TakeAssessment => {
            localized(cx, "Take the assessment", "Пройти тест")
        }
        ReadOnlyActionCommand::RetryAssessment => localized(cx, "Try again", "Попробовать снова"),
        ReadOnlyActionCommand::BackToProjects => localized(cx, "Back to projects", "К проектам"),
    }
}

fn percent(fraction: f64, decimals: usize) -> String {
    format!("{:.decimals$}%", fraction * 100.0)
}

/// A metric chip with a hover card explaining it.
fn metric_chip(
    id: &'static str,
    label: String,
    detail: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    cx: &App,
) -> Stateful<Div> {
    let detail = std::rc::Rc::new(detail);
    div()
        .id(id)
        .px_2()
        .py_0p5()
        .rounded(cx.theme().radius)
        .bg(cx.theme().secondary)
        .text_xs()
        .child(label)
        .tooltip(move |window, cx| {
            let detail = detail.clone();
            Tooltip::element(move |window, cx| detail(window, cx)).build(window, cx)
        })
}

/// `(name EN, name RU, text EN, text RU)` explanations under a metric tooltip.
type Note = (&'static str, &'static str, &'static str, &'static str);

fn tooltip_lines(
    title: (&'static str, &'static str),
    lines: Vec<String>,
    notes: &'static [Note],
) -> impl Fn(&mut Window, &mut App) -> AnyElement + 'static {
    move |_, cx| {
        v_flex()
            .max_w(rems(22.5))
            .gap_1()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(localized(cx, title.0, title.1)),
            )
            .children(lines.iter().map(|line| div().text_xs().child(line.clone())))
            .child(
                div()
                    .pt_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(localized(cx, "What these mean", "Что это значит")),
            )
            .children(notes.iter().map(|(name_en, name_ru, text_en, text_ru)| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{}: {}",
                        localized(cx, name_en, name_ru),
                        localized(cx, text_en, text_ru)
                    ))
            }))
            .into_any_element()
    }
}

const F1: (&str, &str) = ("F1", "F1");
const PRECISION: (&str, &str) = ("Precision", "Точность");
const RECALL: (&str, &str) = ("Recall", "Полнота");

const TIMESTAMP_NOTES: &[Note] = &[
    (
        F1.0,
        F1.1,
        "Overall timing score combining precision and recall. 100% means both annotations have the same speech boundaries.",
        "Общая оценка таймингов, объединяющая точность и полноту. 100% — границы речи в обеих аннотациях совпадают.",
    ),
    (
        RECALL.0,
        RECALL.1,
        "Low recall means the older annotation misses speech the newer one captured, e.g. segments cut off early or started late.",
        "Низкая полнота — в старой аннотации нет речи, которую нашла новая: сегменты обрезаны рано или начаты поздно.",
    ),
    (
        PRECISION.0,
        PRECISION.1,
        "Low precision means the older annotation marked as speech what the newer one treats as silence or noise.",
        "Низкая точность — старая аннотация отметила как речь то, что новая считает тишиной или шумом.",
    ),
    (
        "Matched segments and tolerance",
        "Совпавшие сегменты и допуск",
        "Segments paired one-to-one by word alignment (splits and merges excluded). The bars show how many matched segments have both start and end within ±50 ms, ±100 ms and ±200 ms.",
        "Сегменты сопоставлены один к одному по выравниванию слов (без разделений и объединений). Полосы показывают долю совпавших сегментов, у которых начало и конец попадают в ±50 мс, ±100 мс и ±200 мс.",
    ),
];
const EVENT_NOTES: &[Note] = &[
    (
        F1.0,
        F1.1,
        "Overall agreement on whether a sound event exists in each time window, ignoring the label. 100% means both annotations agree on the presence and absence of every event.",
        "Общее согласие о наличии звукового события в каждом окне времени без учёта метки. 100% — обе аннотации одинаково отмечают наличие и отсутствие всех событий.",
    ),
    (
        PRECISION.0,
        PRECISION.1,
        "Low precision means the older annotation marks events the newer one treats as silence.",
        "Низкая точность — старая аннотация отмечает события, которые новая считает тишиной.",
    ),
    (
        RECALL.0,
        RECALL.1,
        "Low recall means the older annotation misses events the newer one captured.",
        "Низкая полнота — в старой аннотации нет событий, которые нашла новая.",
    ),
];
const LABEL_NOTES: &[Note] = &[
    (
        F1.0,
        F1.1,
        "Agreement on the primary label, averaged across all label classes. 100% means both annotations assign the same label to every event window.",
        "Согласие по основной метке, усреднённое по всем классам меток. 100% — обе аннотации ставят одну и ту же метку в каждом окне события.",
    ),
    (
        PRECISION.0,
        PRECISION.1,
        "Low precision means the older annotation applies labels the newer one disagrees with.",
        "Низкая точность — старая аннотация ставит метки, с которыми новая не согласна.",
    ),
    (
        RECALL.0,
        RECALL.1,
        "Low recall means the older annotation misses label categories the newer one uses.",
        "Низкая полнота — в старой аннотации нет категорий меток, которые использует новая.",
    ),
];
const INTENSITY_NOTES: &[Note] = &[
    (
        F1.0,
        F1.1,
        "Agreement on the intensity group (low 0–1 vs. high 2–4) per event window. 100% means both annotations rate the strength of every sound event the same way.",
        "Согласие по группе интенсивности (низкая 0–1 или высокая 2–4) в каждом окне события. 100% — обе аннотации одинаково оценивают силу каждого звукового события.",
    ),
    (
        PRECISION.0,
        PRECISION.1,
        "Low precision means the older annotation rates as high intensity what the newer one rates low.",
        "Низкая точность — старая аннотация считает высокой интенсивность, которую новая считает низкой.",
    ),
    (
        RECALL.0,
        RECALL.1,
        "Low recall means the older annotation rates as low intensity what the newer one rates high.",
        "Низкая полнота — старая аннотация считает низкой интенсивность, которую новая считает высокой.",
    ),
];

fn metric_line(cx: &App, name: (&'static str, &'static str), value: f64) -> String {
    format!("{} {}", localized(cx, name.0, name.1), percent(value, 1))
}

fn triplet_lines(cx: &App, triplet: &MetricTriplet) -> Vec<String> {
    vec![
        metric_line(cx, F1, triplet.f1),
        metric_line(cx, PRECISION, triplet.precision),
        metric_line(cx, RECALL, triplet.recall),
    ]
}

fn render_metrics(metrics: &DiffMetrics, cx: &App) -> Div {
    match metrics {
        DiffMetrics::Transcription(metrics) => render_transcription_metrics(metrics, cx),
        DiffMetrics::Annotation(metrics) => render_annotation_metrics(metrics, cx),
    }
}

fn render_transcription_metrics(metrics: &TranscriptionDiffMetrics, cx: &App) -> Div {
    let timestamp = &metrics.timestamp;
    let mut lines = vec![
        metric_line(cx, F1, timestamp.timeline.f1),
        metric_line(cx, RECALL, timestamp.timeline.recall),
        metric_line(cx, PRECISION, timestamp.timeline.precision),
        localized_fmt(
            cx,
            "{matched} of {total} segments matched",
            "Совпало сегментов: {matched} из {total}",
            &[
                ("matched", &timestamp.matched_segments.to_string()),
                ("total", &timestamp.total_segments.to_string()),
            ],
        ),
    ];
    let within = localized(cx, "ms", "мс");
    lines.extend([
        format!("±50 {within} {}%", timestamp.within_50_ms),
        format!("±100 {within} {}%", timestamp.within_100_ms),
        format!("±200 {within} {}%", timestamp.within_200_ms),
    ]);
    let theme = cx.theme();
    h_flex()
        .gap_2()
        .child(metric_chip(
            "metric-shift",
            localized_fmt(
                cx,
                "Avg shift {ms} ms",
                "Средний сдвиг {ms} мс",
                &[("ms", &timestamp.avg_shift_ms.to_string())],
            ),
            tooltip_lines(
                ("Timing accuracy", "Точность таймингов"),
                lines,
                TIMESTAMP_NOTES,
            ),
            cx,
        ))
        .child(div().text_xs().child(localized_fmt(
            cx,
            "Word errors {rate}",
            "Ошибки слов {rate}",
            &[("rate", &percent(metrics.overall_word_error_rate, 2))],
        )))
        .child(div().text_xs().text_color(theme.warning).child(format!(
            "{} {}",
            localized(cx, "S", "З"),
            metrics.total_substitutions
        )))
        .child(div().text_xs().text_color(theme.success).child(format!(
            "{} {}",
            localized(cx, "A", "Д"),
            metrics.total_insertions
        )))
        .child(div().text_xs().text_color(theme.danger).child(format!(
            "{} {}",
            localized(cx, "R", "У"),
            metrics.total_deletions
        )))
}

fn render_annotation_metrics(metrics: &AnnotationDiffMetrics, cx: &App) -> Div {
    h_flex()
        .gap_2()
        .child(metric_chip(
            "metric-event",
            metric_line(cx, ("Events", "События"), metrics.event_detection.f1),
            tooltip_lines(
                ("Event detection", "Обнаружение событий"),
                triplet_lines(cx, &metrics.event_detection),
                EVENT_NOTES,
            ),
            cx,
        ))
        .child(metric_chip(
            "metric-label",
            metric_line(cx, ("Labels", "Метки"), metrics.label_macro_average.f1),
            tooltip_lines(
                ("Label accuracy", "Точность меток"),
                triplet_lines(cx, &metrics.label_macro_average),
                LABEL_NOTES,
            ),
            cx,
        ))
        .child(metric_chip(
            "metric-intensity",
            metric_line(
                cx,
                ("Intensity", "Интенсивность"),
                metrics.intensity_calibration.f1,
            ),
            tooltip_lines(
                ("Intensity calibration", "Калибровка интенсивности"),
                triplet_lines(cx, &metrics.intensity_calibration),
                INTENSITY_NOTES,
            ),
            cx,
        ))
}

impl Render for TransportStrip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let problems = self.render_problems(cx);
        let toggles = self.render_drawer_toggles(cx);
        let actions = self.render_review_actions(cx);
        let diff = self.render_diff_toolbar(cx);
        // Wraps onto a second line (actions right-aligned) when the pane is too narrow.
        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border)
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .min_h(rems(2.75))
                    .py_1()
                    .px_2()
                    .gap_2()
                    .child(h_flex().flex_none().gap_2().children(problems))
                    .child(
                        h_flex()
                            .ml_auto()
                            .flex_wrap()
                            .justify_end()
                            .gap_2()
                            .child(self.render_export(cx))
                            .child(toggles)
                            .child(vertical_rule(cx))
                            .child(actions),
                    ),
            )
            .children(diff.map(|diff| diff.px_1().pb_1()))
    }
}
