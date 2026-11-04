//! Transport/action strip above the transcript: track filter, segment navigation, 1 s overlay,
//! undo/redo, problem counts with navigation, drawer toggles and the review actions (drop /
//! skip / flag / save / submit, or the read-only notice and actions). A second row carries the
//! diff toolbar when the task has versions to compare.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::editor_model::diff_toolbar::{
    AnnotationDiffMetrics, DiffMetrics, DiffToolbarState, MetricTriplet, TranscriptionDiffMetrics,
};
use studio_editor::editor_model::review_actions::{
    ReadOnlyActionCommand, ReadOnlyNotice, ReviewActionStrip,
};
use studio_editor::editor_model::{AssessmentPhase, EditorSession, SaveStatus};
use studio_editor::track_filter::TrackFilter;

use super::dialogs::submit;
use super::flows::SKIP_DISABLED_TOOLTIP;
use super::task::{Drawer, TaskPanels, TaskPanelsEvent};
use super::ui::{Choice, choice, muted, stable_text, tabular_figures, vertical_rule};
use crate::editor::RequestLeave;
use crate::editor::model::EditorModel;
use crate::screens::local_files;

pub struct TransportStrip {
    model: Entity<EditorModel>,
    panels: Entity<TaskPanels>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RequestLeave> for TransportStrip {}

impl TransportStrip {
    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panels = TaskPanels::for_model(&model, window, cx);
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe(&panels, |_, _, cx| cx.notify()),
            cx.subscribe(&panels, |_, _, event, cx| match event {
                TaskPanelsEvent::Leave => cx.emit(RequestLeave),
            }),
        ];
        Self {
            model,
            panels,
            _subscriptions: subscriptions,
        }
    }

    fn icon_button(
        &self,
        id: &'static str,
        icon: IconName,
        tooltip: &'static str,
        disabled: bool,
        on_click: impl Fn(&mut EditorModel, &mut Context<EditorModel>) + 'static,
    ) -> Button {
        let model = self.model.clone();
        Button::new(id)
            .ghost()
            .small()
            .icon(icon)
            .tooltip(tooltip)
            .disabled(disabled)
            .on_click(move |_, _, cx| model.update(cx, |model, cx| on_click(model, cx)))
    }

    fn render_navigation(&self, cx: &mut Context<Self>) -> Div {
        let doc = self.model.read(cx).doc();
        let project = doc.project();
        let session = doc.session();
        let show_track = session.transcript_visible_cells(project).track;
        let overlay_on = session.overlay_range.is_some();
        let (can_undo, can_redo) = (doc.can_undo(), doc.can_redo());
        let track_filter = show_track.then(|| {
            let options = EditorSession::track_filter_options(project)
                .into_iter()
                .map(|filter| {
                    let label = filter.label();
                    Choice::new(filter, label)
                })
                .collect();
            let model = self.model.clone();
            choice(
                "track-filter",
                Some(&session.track_filter),
                options,
                "All Tracks",
                move |filter: TrackFilter, _, cx| {
                    model.update(cx, |model, cx| {
                        model.update_doc(cx, |doc| doc.set_track_filter(filter))
                    });
                },
            )
        });
        h_flex()
            .gap_1()
            .when_some(track_filter, |this, filter| {
                this.child(filter).child(vertical_rule(cx))
            })
            .child(self.icon_button(
                "previous-segment",
                IconName::ChevronLeft,
                "Previous Segment",
                false,
                |model, cx| model.run(cx, |doc| doc.select_adjacent_segment(-1)),
            ))
            .child(self.icon_button(
                "next-segment",
                IconName::ChevronRight,
                "Next Segment",
                false,
                |model, cx| model.run(cx, |doc| doc.select_adjacent_segment(1)),
            ))
            .child(
                self.icon_button(
                    "overlay-toggle",
                    IconName::Frame,
                    "1-second overlay at playhead (T)",
                    false,
                    |model, cx| model.update_doc(cx, |doc| doc.toggle_overlay()),
                )
                .selected(overlay_on),
            )
            .child(vertical_rule(cx))
            .child(
                self.icon_button("undo", IconName::Undo2, "Undo", !can_undo, |model, cx| {
                    model.edit(cx, |doc| doc.undo())
                }),
            )
            .child(
                self.icon_button("redo", IconName::Redo2, "Redo", !can_redo, |model, cx| {
                    model.edit(cx, |doc| doc.redo())
                }),
            )
    }

    fn render_problems(&self, cx: &mut Context<Self>) -> Option<Div> {
        let doc = self.model.read(cx).doc();
        let counts = doc.problem_counts();
        if counts.errors == 0 && counts.warnings == 0 {
            return None;
        }
        let label = problem_summary(counts.errors, counts.warnings);
        // Tabular figures, sized for counts with as many digits as the row count, both
        // plurals and both kinds: the count changing never moves the step buttons.
        let digits = doc.project().segments.len().max(1).to_string().len() as u32;
        let most = 10_usize.pow(digits) - 1;
        let label_slot = stable_text(
            label,
            [
                problem_summary(most, 0),
                problem_summary(0, most),
                problem_summary(most, most),
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
                    "Previous problem",
                    -1,
                ))
                .child(nav(
                    "problem-next",
                    IconName::ChevronDown,
                    "Next problem",
                    1,
                )),
        )
    }

    /// "Export" menu: Babel JSON file or Studio project folder.
    fn render_export(&self) -> impl IntoElement + use<> {
        let model = self.model.clone();
        Button::new("export")
            .ghost()
            .small()
            .icon(IconName::FolderOpen)
            .label("Export")
            .tooltip("Export this task")
            .dropdown_menu(move |menu, _, _| {
                let json = model.clone();
                let folder = model.clone();
                menu.item(
                    PopupMenuItem::new("Babel JSON…").on_click(move |_, window, cx| {
                        local_files::export_babel_json(&json, window, cx);
                    }),
                )
                .item(
                    PopupMenuItem::new("Project folder…").on_click(move |_, window, cx| {
                        local_files::export_project_folder(&folder, window, cx);
                    }),
                )
            })
    }

    fn render_drawer_toggles(&self, cx: &mut Context<Self>) -> Div {
        let panels = self.panels.read(cx);
        let drawer = panels.drawer;
        let has_feedback = panels.feedback.is_some();
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
                            panels.status(
                                if opening {
                                    "Feedback panel opened."
                                } else {
                                    "Feedback panel closed."
                                },
                                cx,
                            );
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
                    "Feedback",
                    "Add feedback for L1 transcriber",
                    Drawer::Feedback,
                ))
            })
            .child(toggle(
                "toggle-assistant",
                IconName::Bot,
                "Assistant",
                "Reviewer and drafting tools",
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
                let eligibility = session.submit_eligibility(project);
                let submit_copy = submit::copy(eligibility.kind);
                let submit_tooltip = eligibility
                    .blocker
                    .map_or("Submit your review", |blocker| blocker.reason());
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
                    row = row.child(
                        Button::new("drop-claim")
                            .outline()
                            .small()
                            .child(
                                stable_text(
                                    if drop.is_dropping {
                                        "Dropping..."
                                    } else {
                                        "Drop Claim"
                                    },
                                    ["Dropping...", "Drop Claim"],
                                )
                                .text_center(),
                            )
                            .accessibility_label("Drop Claim")
                            .tooltip("Drop action for another worker")
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
                            .label("Skip Chunk")
                            .tooltip(if allowed {
                                "Archive this chunk and return to the project page."
                            } else {
                                SKIP_DISABLED_TOOLTIP
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
                            .label("Flag Recording")
                            .on_click(move |_, window, cx| {
                                panels
                                    .update(cx, |panels, cx| panels.request_admin_flag(window, cx));
                            }),
                    );
                }
                let (save_label, save_tooltip) = save_copy(&save_status, save_clickable);
                let save_panels = self.panels.clone();
                let submit_panels = self.panels.clone();
                let saving = matches!(save_status, SaveStatus::Saving);
                // Labels keep the width of their widest state, so saving or submitting never
                // moves the buttons to their left.
                row.child(
                    Button::new("save-progress")
                        .outline()
                        .small()
                        .child(stable_text(save_label, SAVE_LABELS).text_center())
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
                        .child(
                            stable_text(
                                if submit_pending {
                                    "Submitting..."
                                } else {
                                    submit_copy.trigger
                                },
                                ["Submitting...", submit_copy.trigger],
                            )
                            .text_center(),
                        )
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
                    this.child(muted(read_only_notice(notice), cx))
                });
                for action in actions {
                    let panels = self.panels.clone();
                    let command = action.command;
                    row = row.child(
                        Button::new(SharedString::from(format!("read-only-{command:?}")))
                            .outline()
                            .small()
                            .label(read_only_label(command, action.disabled))
                            .disabled(action.disabled)
                            .on_click(move |_, _, cx| {
                                panels
                                    .update(cx, |panels, cx| panels.read_only_action(command, cx));
                            }),
                    );
                }
                row
            }
        }
    }

    fn render_diff_toolbar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let doc = self.model.read(cx).doc();
        let state = doc.session().diff_toolbar(doc.project());
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
                        "Annotation Diff"
                    } else {
                        "Diff View"
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
                        AssessmentPhase::Practice => "Practice Mode",
                        AssessmentPhase::Assessment => "Assessment",
                    }),
            );
        }
        if state.no_versions_hint_visible() {
            row = row.child(muted(
                if state.annotation_mode {
                    "No other versions to compare"
                } else {
                    "No previous versions to compare"
                },
                cx,
            ));
        }
        if state.compare_select_visible() {
            row = row
                .child(muted("Compare:", cx))
                .child(self.render_compare(&state));
        }
        if state.unsaved_changes_badge_visible() {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(theme.warning)
                    .child("Unsaved changes"),
            );
        }
        if state.show_reviewers_control
            && !state.annotation_mode
            && state.assessment_phase.is_none()
        {
            let model = self.model.clone();
            row = row.child(
                Switch::new("reviewers-toggle")
                    .checked(state.reviewers_checked)
                    .label("Reviewers")
                    .on_click(move |_, _, cx| {
                        model.update(cx, |model, cx| {
                            let shown = model.update_doc(cx, |doc| {
                                doc.with_workflow(|workflow| workflow.toggle_diff_reviewers())
                            });
                            model.set_status(
                                if shown {
                                    "Reviewer feedback shown."
                                } else {
                                    "Reviewer feedback hidden."
                                },
                                cx,
                            );
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

    fn render_compare(&self, state: &DiffToolbarState) -> AnyElement {
        if state.is_loading {
            return div()
                .text_sm()
                .child(if state.annotation_mode {
                    "Loading…"
                } else {
                    "Loading..."
                })
                .into_any_element();
        }
        let options = state
            .compare_options
            .iter()
            .map(|option| Choice::new(option.id.clone(), option.label.clone()))
            .collect();
        let model = self.model.clone();
        choice(
            "diff-compare",
            state.selected_compare_id.as_ref(),
            options,
            if state.annotation_mode {
                "Select version…"
            } else {
                "Select version..."
            },
            move |action_id: String, _, cx| {
                model.update(cx, |model, cx| {
                    let status = format!("Compare version `{action_id}` selected.");
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

fn problem_summary(errors: usize, warnings: usize) -> String {
    let term = |count: usize, noun: &str| {
        (count > 0).then(|| format!("{count} {noun}{}", if count == 1 { "" } else { "s" }))
    };
    [term(errors, "error"), term(warnings, "warning")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every label [`save_copy`] returns (the save button is as wide as the widest).
const SAVE_LABELS: [&str; 4] = ["Saving...", "Error - Retry", "Save progress", "Saved"];

fn save_copy(status: &SaveStatus, clickable: bool) -> (&'static str, SharedString) {
    match status {
        SaveStatus::Saving => ("Saving...", "Saving your changes".into()),
        SaveStatus::Failed { message } => (
            "Error - Retry",
            match message {
                Some(message) => format!("Save failed: {message}").into(),
                None => "Save failed. Retry to save changes.".into(),
            },
        ),
        SaveStatus::Dirty => ("Save progress", "You have unsaved changes".into()),
        SaveStatus::Saved if clickable => ("Save progress", "You have unsaved changes".into()),
        SaveStatus::Saved => ("Saved", "All changes saved".into()),
    }
}

fn read_only_notice(notice: ReadOnlyNotice) -> &'static str {
    match notice {
        ReadOnlyNotice::FinalSubmitted => {
            "Submitted successfully! Our team will review and get back to you soon."
        }
        ReadOnlyNotice::AssessmentSession => {
            "No changes will persist during this assessment session."
        }
        ReadOnlyNotice::ReadOnlySession => "No changes will persist during this read only session.",
    }
}

fn read_only_label(command: ReadOnlyActionCommand, disabled: bool) -> &'static str {
    match command {
        ReadOnlyActionCommand::RetryPractice => "Retry Practice",
        ReadOnlyActionCommand::TakeAssessment if disabled => "Loading Assessment...",
        ReadOnlyActionCommand::TakeAssessment => "Take Assessment",
        ReadOnlyActionCommand::RetryAssessment => "Retry",
        ReadOnlyActionCommand::BackToProjects => "Back to Projects",
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

fn tooltip_lines(
    title: &'static str,
    lines: Vec<String>,
    notes: &'static [(&'static str, &'static str)],
) -> impl Fn(&mut Window, &mut App) -> AnyElement + 'static {
    move |_, cx| {
        v_flex()
            .max_w(rems(22.5))
            .gap_1()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .children(lines.iter().map(|line| div().text_xs().child(line.clone())))
            .child(
                div()
                    .pt_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if notes.len() > 3 {
                        "What do these mean?"
                    } else {
                        "What does this mean?"
                    }),
            )
            .children(notes.iter().map(|(name, text)| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{name}: {text}"))
            }))
            .into_any_element()
    }
}

const TIMESTAMP_NOTES: &[(&str, &str)] = &[
    (
        "F1 Score",
        "Overall timestamp alignment score combining Precision and Recall. 100% means both annotations have identical speech boundaries.",
    ),
    (
        "Recall",
        "Low recall means the older annotation is missing speech the newer annotation captured, e.g. segments cut off early or started too late.",
    ),
    (
        "Precision",
        "Low precision means the older annotation marked time as speech that the newer annotation considers silence or background noise.",
    ),
    (
        "Segments matched & tolerance",
        "Segments paired one-to-one by word alignment (splits and merges excluded). The bars show what percentage of those matched segments have both start and end timestamps within ±50ms, ±100ms, and ±200ms.",
    ),
];
const EVENT_NOTES: &[(&str, &str)] = &[
    (
        "F1 Score",
        "Overall agreement on whether a sound event exists in each time window, ignoring label. 100% means the older and newer annotations agree on the exact presence and absence of all events.",
    ),
    (
        "Precision",
        "Low precision means the older annotation marks events the newer annotation considers silence.",
    ),
    (
        "Recall",
        "Low recall means the older annotation is missing events the newer annotation captured.",
    ),
];
const LABEL_NOTES: &[(&str, &str)] = &[
    (
        "F1 Score",
        "Agreement on primary label identity, macro-averaged across all label classes. 100% means the older and newer annotations assign the same label to every event window.",
    ),
    (
        "Precision",
        "Low precision means the older annotation applies labels the newer annotation does not agree with.",
    ),
    (
        "Recall",
        "Low recall means the older annotation is missing label categories the newer annotation uses.",
    ),
];
const INTENSITY_NOTES: &[(&str, &str)] = &[
    (
        "F1 Score",
        "Agreement on intensity group (low 0-1 vs. high 2-4) per event window. 100% means the older and newer annotations consistently rate the strength of every sound event the same way.",
    ),
    (
        "Precision",
        "Low precision means the older annotation rates events as high intensity that the newer annotation considers low.",
    ),
    (
        "Recall",
        "Low recall means the older annotation rates events as low intensity that the newer annotation considers high.",
    ),
];

fn triplet_lines(triplet: &MetricTriplet) -> Vec<String> {
    vec![
        format!("F1 Score {}", percent(triplet.f1, 1)),
        format!("Precision {}", percent(triplet.precision, 1)),
        format!("Recall {}", percent(triplet.recall, 1)),
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
        format!("F1 Score {}", percent(timestamp.timeline.f1, 1)),
        format!("Recall {}", percent(timestamp.timeline.recall, 1)),
        format!("Precision {}", percent(timestamp.timeline.precision, 1)),
        format!(
            "{}/{} segments matched",
            timestamp.matched_segments, timestamp.total_segments
        ),
    ];
    lines.extend([
        format!("±50ms {}%", timestamp.within_50_ms),
        format!("±100ms {}%", timestamp.within_100_ms),
        format!("±200ms {}%", timestamp.within_200_ms),
    ]);
    let theme = cx.theme();
    h_flex()
        .gap_2()
        .child(metric_chip(
            "metric-shift",
            format!("Avg shift: {}ms", timestamp.avg_shift_ms),
            tooltip_lines("Timestamp Accuracy", lines, TIMESTAMP_NOTES),
            cx,
        ))
        .child(div().text_xs().child(format!(
            "Word Diff: {}",
            percent(metrics.overall_word_error_rate, 2)
        )))
        .child(
            div()
                .text_xs()
                .text_color(theme.warning)
                .child(format!("S {}", metrics.total_substitutions)),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.success)
                .child(format!("A {}", metrics.total_insertions)),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.danger)
                .child(format!("R {}", metrics.total_deletions)),
        )
}

fn render_annotation_metrics(metrics: &AnnotationDiffMetrics, cx: &App) -> Div {
    h_flex()
        .gap_2()
        .child(metric_chip(
            "metric-event",
            format!("Event {}", percent(metrics.event_detection.f1, 1)),
            tooltip_lines(
                "Event Detection",
                triplet_lines(&metrics.event_detection),
                EVENT_NOTES,
            ),
            cx,
        ))
        .child(metric_chip(
            "metric-label",
            format!("Label {}", percent(metrics.label_macro_average.f1, 1)),
            tooltip_lines(
                "Label Accuracy",
                triplet_lines(&metrics.label_macro_average),
                LABEL_NOTES,
            ),
            cx,
        ))
        .child(metric_chip(
            "metric-intensity",
            format!("Intensity {}", percent(metrics.intensity_calibration.f1, 1)),
            tooltip_lines(
                "Intensity Calibration",
                triplet_lines(&metrics.intensity_calibration),
                INTENSITY_NOTES,
            ),
            cx,
        ))
}

impl Render for TransportStrip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let navigation = self.render_navigation(cx);
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
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_2()
                            .child(navigation)
                            .children(problems),
                    )
                    .child(
                        h_flex()
                            .ml_auto()
                            .flex_wrap()
                            .justify_end()
                            .gap_2()
                            .child(self.render_export())
                            .child(toggles)
                            .child(vertical_rule(cx))
                            .child(actions),
                    ),
            )
            .children(diff.map(|diff| diff.px_1().pb_1()))
    }
}
