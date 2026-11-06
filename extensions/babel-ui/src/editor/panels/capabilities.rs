//! Babel assistant drawer: optional Gold commands and review-backed text redistribution.

use super::task::{Drawer, TaskPanels};
use crate::services::BabelServices;
use crate::text::{count, failed, localized, localized_fmt};
use exaudio_babel_services::review::build_draft_redistribute_payload;
use exaudio_gold::polish::{DraftingConfig, RedistributionPreview, apply_redistribution};
use exaudio_gold::prefs::AiPreferences;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::editor::panels::ui::{caption, muted};
use exaudio_shell::services::spawn_service_in;
use exaudio_shell::{ContributionRegistry, ShellHandle};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, h_flex, v_flex};
use gpui_kit::*;

const COMPACT_LIMIT: usize = 180;

struct DraftingState {
    redistribution: Vec<RedistributionPreview>,
    pending: bool,
    error: Option<String>,
    tasks: Vec<Task<()>>,
}

pub struct CapabilityPanel {
    model: Entity<EditorModel>,
    shell: ShellHandle,
    panels: Entity<TaskPanels>,
    drafting: DraftingState,
    _subscriptions: Vec<Subscription>,
}

impl CapabilityPanel {
    /// Drawer width (the editor mounts it at this width, capped to the window).
    pub const WIDTH: Rems = rems(28.75);

    /// Whether the drawer shows (it renders nothing otherwise).
    pub fn is_open(panels: &TaskPanels) -> bool {
        panels.drawer == Drawer::Assistant
    }

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
        ];
        Self {
            model,
            shell,
            panels,
            drafting: DraftingState {
                redistribution: Vec::new(),
                pending: false,
                error: None,
                tasks: Vec::new(),
            },
            _subscriptions: subscriptions,
        }
    }

    fn status(&self, message: impl Into<SharedString>, cx: &mut App) {
        let message = message.into();
        self.model
            .update(cx, |model, cx| model.set_status(message, cx));
    }

    /// Commits typed-but-uncommitted row text so requests see the current transcript.
    fn commit_pending_drafts(&self, cx: &mut App) {
        let label = localized(cx, "AI draft", "Черновик ИИ");
        self.model.update(cx, |model, cx| {
            match model.update_doc(cx, |doc| doc.commit_pending_drafts(label)) {
                Ok(outcome) => model.apply_outcome(outcome, cx),
                Err(error) => model.set_status(error.to_string(), cx),
            }
        });
    }

    fn redistribute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drafting.pending {
            let message = localized(
                cx,
                "Text redistribution is already running.",
                "Перераспределение текста уже выполняется.",
            );
            return self.status(message, cx);
        }
        self.commit_pending_drafts(cx);
        let settings = cx
            .global::<exaudio_shell::services::AppServices>()
            .storage
            .ai_settings();
        let config = DraftingConfig::from_inputs(
            &settings.gold_openrouter_key,
            &settings.gold_model,
            &settings.gold_service_tier,
            &settings.gold_reasoning_effort,
            "",
        );
        let doc = self.model.read(cx).doc();
        let selected = doc.session().selected_segment_id.clone();
        let payload =
            match build_draft_redistribute_payload(doc.project(), selected.as_deref(), &config) {
                Ok(payload) => payload,
                Err(error) => {
                    let message =
                        failed(cx, "Couldn't draft.", "Не удалось создать черновик.", error);
                    return self.status(message, cx);
                }
            };
        self.drafting.pending = true;
        self.drafting.error = None;
        let message = localized(cx, "Redistributing text…", "Перераспределение текста…");
        self.status(message, cx);
        let backend = cx.global::<BabelServices>().review.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { backend.redistribute_text(payload).await },
            |this, result, _, cx| {
                this.drafting.pending = false;
                match result {
                    Ok(previews) => {
                        let groups = count(
                            previews.len() as u64,
                            ("group", "groups"),
                            ("группа", "группы", "групп"),
                        );
                        let message = localized_fmt(
                            cx,
                            "Loaded {groups} to review.",
                            "Загружено {groups} для проверки.",
                            &[("groups", &groups)],
                        );
                        this.status(message, cx);
                        this.drafting.redistribution = previews;
                    }
                    Err(error) => {
                        let error = format!("{error:#}");
                        let message = failed(
                            cx,
                            "Couldn't preview the redistribution.",
                            "Не удалось подготовить перераспределение.",
                            &error,
                        );
                        this.status(message, cx);
                        this.drafting.error = Some(error);
                    }
                }
                cx.notify();
            },
        );
        self.drafting.tasks.push(task);
        cx.notify();
    }

    fn apply_group(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(preview) = self.drafting.redistribution.get(index).cloned() else {
            return;
        };
        if let Some(error) = &preview.error {
            let message = failed(
                cx,
                "Couldn't apply this group.",
                "Не удалось применить эту группу.",
                error,
            );
            return self.status(message, cx);
        }
        let label = localized(cx, "Apply redistribution", "Применить перераспределение");
        let result = self.model.update(cx, |model, cx| {
            model
                .update_doc(cx, |doc| {
                    doc.edit_workflow(label, |workflow| apply_redistribution(workflow, &preview))
                })
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(applied) => {
                if let Some(preview) = self.drafting.redistribution.get_mut(index) {
                    preview.applied = true;
                }
                let segments = count(
                    applied.applied.len() as u64,
                    ("segment", "segments"),
                    ("сегмент", "сегмента", "сегментов"),
                );
                let message = if !applied.missing.is_empty() {
                    let missing = count(
                        applied.missing.len() as u64,
                        ("segment", "segments"),
                        ("сегмент", "сегмента", "сегментов"),
                    );
                    localized_fmt(
                        cx,
                        "Applied to {segments}; {missing} not found.",
                        "Применено: {segments}; не найдено: {missing}.",
                        &[("segments", &segments), ("missing", &missing)],
                    )
                } else if !preview.accept_draft && !preview.moves.is_empty() {
                    let moves = count(
                        preview.moves.len() as u64,
                        ("move", "moves"),
                        ("перенос", "переноса", "переносов"),
                    );
                    localized_fmt(
                        cx,
                        "Applied to {segments} with {moves}.",
                        "Применено: {segments}, {moves}.",
                        &[("segments", &segments), ("moves", &moves)],
                    )
                } else {
                    localized_fmt(
                        cx,
                        "Applied to {segments}.",
                        "Применено: {segments}.",
                        &[("segments", &segments)],
                    )
                };
                self.status(message, cx);
            }
            Err(error) => {
                let message = failed(
                    cx,
                    "Couldn't apply this group.",
                    "Не удалось применить эту группу.",
                    error,
                );
                self.status(message, cx);
            }
        }
        cx.notify();
    }

    // ---- rendering ------------------------------------------------------------------------

    fn render_drafting(&self, cx: &mut Context<Self>) -> Div {
        let read_only = self.model.read(cx).doc().is_read_only();
        let danger = cx.theme().danger;
        let button =
            |id: &'static str, label: &'static str| Button::new(id).small().outline().label(label);

        let mut body = v_flex().gap_4().child(
            v_flex()
                .gap_2()
                .child(caption(localized(cx, "AI tools", "Инструменты ИИ"), cx))
                .child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .child(
                            button("dr-gold-draft", localized(cx, "Draft with AI", "Черновик с ИИ"))
                                .icon(gpui_kit::assets::IconName::WandSparkles)
                                .disabled(read_only || !cx.global::<ContributionRegistry>().is_enabled("exaudio.gold"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.shell.dispatch("exaudio.gold.draft", window, cx);
                                })),
                        )
                        .child(button("dr-timestamps", localized(cx, "Align words", "Выровнять слова"))
                            .disabled(!cx.global::<ContributionRegistry>().is_enabled("exaudio.gold"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.shell.dispatch("exaudio.gold.align", window, cx);
                            })))
                        .child(
                            button("dr-transcribe", localized(cx, "Fill this segment", "Заполнить сегмент"))
                                .disabled(read_only || !cx.global::<ContributionRegistry>().is_enabled("exaudio.gold"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.shell.dispatch("exaudio.gold.transcribe-segment", window, cx);
                                })),
                        )
                        .child(
                            button("dr-redistribute", localized(cx, "Redistribute text", "Перераспределить текст"))
                                .loading(self.drafting.pending)
                                .disabled(self.drafting.pending)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.redistribute(window, cx)),
                                ),
                        )
                        .child(
                            button("dr-ai-settings", localized(cx, "AI settings", "Настройки ИИ"))
                                .disabled(!cx.global::<ContributionRegistry>().is_enabled("exaudio.gold"))
                                .on_click(cx.listener(|this, _, window, cx| this.shell.dispatch("exaudio.gold.settings", window, cx))),
                        ),
                )
                .child(muted(
                    localized(
                        cx,
                        "Fill this segment writes text into the selected empty segment (Alt+Shift+G). Where drafting runs, the models and the polish key are in AI settings.",
                        "«Заполнить сегмент» записывает текст в выбранный пустой сегмент (Alt+Shift+G). Где выполняется черновик, модели и ключ доработки — в настройках ИИ.",
                    ),
                    cx,
                )),
        );

        if !self.drafting.redistribution.is_empty() {
            let groups =
                self.drafting
                    .redistribution
                    .iter()
                    .take(5)
                    .enumerate()
                    .map(|(index, preview)| {
                        let state = if preview.accept_draft {
                            localized(cx, "accepted", "принято")
                        } else if preview.error.is_some() {
                            localized(cx, "blocked", "заблокировано")
                        } else {
                            localized(cx, "to review", "на проверку")
                        };
                        let allocations_count = count(
                            preview.allocations.len() as u64,
                            ("segment", "segments"),
                            ("сегмент", "сегмента", "сегментов"),
                        );
                        let moves_count = count(
                            preview.moves.len() as u64,
                            ("move", "moves"),
                            ("перенос", "переноса", "переносов"),
                        );
                        let mut line = format!(
                            "{} · {state} · {allocations_count}, {moves_count}",
                            preview.group_id
                        );
                        if let Some(note) = preview.error.as_ref().or(preview.notes.as_ref()) {
                            line.push_str(&format!(" — {note}"));
                        }
                        let allocations = preview.allocations.iter().take(5).map(|allocation| {
                            muted(
                                compact(&format!("{}: {}", allocation.segment_id, allocation.text)),
                                cx,
                            )
                        });
                        v_flex()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(div().flex_1().text_xs().child(compact(&line)))
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "dr-group-{index}"
                                        )))
                                        .xsmall()
                                        .outline()
                                        .label(if preview.applied {
                                            localized(cx, "Applied", "Применено")
                                        } else {
                                            localized(cx, "Apply group", "Применить группу")
                                        })
                                        .disabled(
                                            read_only || preview.applied || preview.error.is_some(),
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.apply_group(index, cx)
                                            }),
                                        ),
                                    ),
                            )
                            .children(allocations)
                    });
            body = body.child(
                v_flex()
                    .gap_2()
                    .child(caption(
                        localized_fmt(
                            cx,
                            "Redistribution: {groups}",
                            "Перераспределение: {groups}",
                            &[(
                                "groups",
                                &count(
                                    self.drafting.redistribution.len() as u64,
                                    ("group", "groups"),
                                    ("группа", "группы", "групп"),
                                ),
                            )],
                        ),
                        cx,
                    ))
                    .children(groups),
            );
        }
        body.children(
            self.drafting
                .error
                .as_deref()
                .map(|error| div().text_xs().text_color(danger).child(compact(error))),
        )
    }
}

/// Collapses whitespace; long text is cut to 177 characters plus `...`.
fn compact(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > COMPACT_LIMIT {
        let mut cut: String = joined.chars().take(COMPACT_LIMIT - 3).collect();
        cut.push_str("...");
        cut
    } else {
        joined
    }
}

impl Render for CapabilityPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !Self::is_open(self.panels.read(cx)) {
            return div().into_any_element();
        }
        let body = self.render_drafting(cx);
        let status = self.model.read(cx).status().cloned();
        let theme = cx.theme();
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
                                            .child(localized(cx, "Assistant", "Помощник")),
                                    )
                                    .children(status.map(|status| muted(status, cx))),
                            )
                            .child(
                                Button::new("assistant-close")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .tooltip(localized(cx, "Close", "Закрыть"))
                                    .on_click(move |_, _, cx| {
                                        close.update(cx, |panels, cx| {
                                            panels.toggle_drawer(Drawer::Assistant, cx)
                                        });
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .id("assistant-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(div().p_3().child(body)),
            )
            .into_any_element()
    }
}
