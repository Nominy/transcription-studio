//! Assistant drawer ("Assistant Tools"): entry points to Gold Draft, word timestamps and
//! segment transcription (run by [`GoldDraftPanel`]), plus text redistribution. Magic Review
//! lives in [`super::review_workspace`].

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, h_flex, v_flex};
use gpui_kit::*;
use studio_services::review::{
    DraftingConfig, RedistributionPreview, apply_redistribution, build_draft_redistribute_payload,
};

use super::gold_draft::{DraftMode, GoldDraftPanel, open_ai_settings};
use super::task::{Drawer, TaskPanels};
use super::ui::{caption, muted};
use crate::editor::model::EditorModel;
use crate::services::{AppServices, spawn_service_in};

const COMPACT_LIMIT: usize = 180;

struct DraftingState {
    redistribution: Vec<RedistributionPreview>,
    pending: bool,
    error: Option<String>,
    tasks: Vec<Task<()>>,
}

pub struct CapabilityPanel {
    model: Entity<EditorModel>,
    gold_draft: Entity<GoldDraftPanel>,
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
        gold_draft: Entity<GoldDraftPanel>,
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
            gold_draft,
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
        self.model.update(cx, |model, cx| {
            match model.update_doc(cx, |doc| doc.commit_pending_drafts("Gold Drafting")) {
                Ok(outcome) => model.apply_outcome(outcome, cx),
                Err(error) => model.set_status(error.to_string(), cx),
            }
        });
    }

    fn redistribute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drafting.pending {
            return self.status("Text redistribution is already pending.", cx);
        }
        self.commit_pending_drafts(cx);
        let settings = &cx.global::<AppServices>().ai;
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
                Err(error) => return self.status(format!("Drafting unavailable: {error:#}"), cx),
            };
        self.drafting.pending = true;
        self.drafting.error = None;
        self.status("Reviewing text redistribution...", cx);
        let backend = cx.global::<AppServices>().review.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { backend.redistribute_text(payload).await },
            |this, result, _, cx| {
                this.drafting.pending = false;
                match result {
                    Ok(previews) => {
                        this.status(
                            format!("Loaded {} redistribution previews.", previews.len()),
                            cx,
                        );
                        this.drafting.redistribution = previews;
                    }
                    Err(error) => {
                        let error = format!("{error:#}");
                        this.status(format!("Redistribution preview failed: {error}"), cx);
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
            return self.status(format!("Apply redistribution blocked: {error}"), cx);
        }
        let result = self.model.update(cx, |model, cx| {
            model
                .update_doc(cx, |doc| {
                    doc.edit_workflow("Apply redistribution", |workflow| {
                        apply_redistribution(workflow, &preview)
                    })
                })
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(applied) => {
                if let Some(preview) = self.drafting.redistribution.get_mut(index) {
                    preview.applied = true;
                }
                let group = &preview.group_id;
                let message = if !applied.missing.is_empty() {
                    format!(
                        "Applied redistribution group `{group}` to {} rows; missing {} rows.",
                        applied.applied.len(),
                        applied.missing.len()
                    )
                } else if !preview.accept_draft && !preview.moves.is_empty() {
                    format!(
                        "Applied redistribution group `{group}` to {} rows using {} model move(s).",
                        applied.applied.len(),
                        preview.moves.len()
                    )
                } else {
                    format!(
                        "Applied redistribution group `{group}` to {} rows.",
                        applied.applied.len()
                    )
                };
                self.status(message, cx);
            }
            Err(error) => self.status(format!("Apply redistribution blocked: {error}"), cx),
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
                .child(caption("Gold Drafting", cx))
                .child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .child(
                            button("dr-gold-draft", "Gold Draft")
                                .icon(gpui_kit::assets::IconName::WandSparkles)
                                .disabled(read_only)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.gold_draft.update(cx, |gold, cx| {
                                        gold.open(DraftMode::Draft, window, cx)
                                    });
                                })),
                        )
                        .child(button("dr-timestamps", "Get timestamps").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.gold_draft.update(cx, |gold, cx| {
                                    gold.open(DraftMode::TimestampsOnly, window, cx)
                                });
                            }),
                        ))
                        .child(
                            button("dr-transcribe", "Transcribe Segment")
                                .disabled(read_only)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.gold_draft.update(cx, |gold, cx| {
                                        gold.transcribe_segment(None, window, cx)
                                    });
                                })),
                        )
                        .child(
                            button("dr-redistribute", "Redistribute Text")
                                .loading(self.drafting.pending)
                                .disabled(self.drafting.pending)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.redistribute(window, cx)),
                                ),
                        )
                        .child(
                            button("dr-ai-settings", "AI settings")
                                .on_click(|_, window, cx| open_ai_settings(window, cx)),
                        ),
                )
                .child(muted(
                    "Transcribe Segment fills the selected empty segment (Alt+Shift+G). Where to draft, the models on this PC and the polish key are in AI settings.",
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
                            "accepted"
                        } else if preview.error.is_some() {
                            "blocked"
                        } else {
                            "review"
                        };
                        let mut line = format!(
                            "{} [{state}] {} allocation(s), {} move(s)",
                            preview.group_id,
                            preview.allocations.len(),
                            preview.moves.len()
                        );
                        if let Some(note) = preview.error.as_ref().or(preview.notes.as_ref()) {
                            line.push_str(&format!(" - {note}"));
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
                                            "Applied"
                                        } else {
                                            "Apply Group"
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
                        format!(
                            "Redistribution: {} group(s)",
                            self.drafting.redistribution.len()
                        ),
                        cx,
                    ))
                    .children(groups),
            );
        }
        body.children(self.drafting.error.as_deref().map(|error| {
            div()
                .text_xs()
                .text_color(danger)
                .child(compact(&format!("Error: {error}")))
        }))
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
                                            .child("Assistant Tools"),
                                    )
                                    .children(status.map(|status| muted(status, cx))),
                            )
                            .child(
                                Button::new("assistant-close")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .tooltip("Close assistant tools")
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
