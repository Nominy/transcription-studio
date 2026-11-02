//! Assistant drawer ("Assistant Tools"): the Reviewer workspace for L2 review tasks and Gold
//! Drafting (row rewrites, selected-segment transcription, text redistribution). Backend
//! settings (OpenRouter key/model/tier/effort) come from Settings via
//! [`crate::services::DraftingSettings`].

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_core::Project;
use studio_services::review::{
    AudioTrack, DraftAudioCueStatus, DraftAudioGuard, DraftAudioMode, DraftRowPreview,
    DraftSummary, DraftTranscription, DraftingConfig, RedistributionPreview, ReviewerCard,
    apply_draft_row, apply_draft_rows, apply_redistribution, apply_segment_text,
    broker_audio_tracks, build_draft_generate_payload, build_draft_redistribute_payload,
    build_draft_transcribe_segment_payload, draft_audio_cue_status, draft_row_previews,
    resolve_draft_audio,
};
use studio_waveform::DecodedAudio;

use super::task::{Drawer, TaskPanels};
use super::ui::{caption, muted};
use crate::editor::model::EditorModel;
use crate::services::{AppServices, spawn_service_in};

const PREVIEW_ROW_LIMIT: usize = 40;
const COMPACT_LIMIT: usize = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssistantTab {
    Reviewer,
    Drafting,
}

/// Selected-segment transcription result and the row it was requested for.
struct TranscriptionPreview {
    segment_id: Option<String>,
    original: String,
    result: DraftTranscription,
}

struct DraftingState {
    rows: Vec<DraftRowPreview>,
    summary: Option<DraftSummary>,
    transcription: Option<TranscriptionPreview>,
    redistribution: Vec<RedistributionPreview>,
    guard: Option<DraftAudioGuard>,
    pending: Vec<&'static str>,
    error: Option<String>,
    tasks: Vec<Task<()>>,
}

pub struct CapabilityPanel {
    model: Entity<EditorModel>,
    panels: Entity<TaskPanels>,
    tab: AssistantTab,
    cards_expanded: bool,
    suggestions_shown: bool,
    session_id: Entity<InputState>,
    session_comment: Entity<TextareaState>,
    card_comment: Entity<TextareaState>,
    template_query: Entity<InputState>,
    template_id: Entity<InputState>,
    proposal_id: Entity<InputState>,
    history_query: Entity<InputState>,
    draft_session: Entity<InputState>,
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

    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panels = TaskPanels::for_model(&model, window, cx);
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let textarea = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(2, 6)
                    .placeholder(placeholder)
            })
        };
        let session_id = input("Session ID", window, cx);
        let session_comment = textarea("Comment about the full review session", window, cx);
        let card_comment = textarea("Add or edit this card's comment", window, cx);
        let template_query = input("Search templates", window, cx);
        let template_id = input("Template ID", window, cx);
        let proposal_id = input("Proposal ID", window, cx);
        let history_query = input("History query (key=value&...)", window, cx);
        let draft_session = input("Draft session", window, cx);

        let reviewer_tab = panels.read(cx).reviewer_available(cx);
        let mut subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe_in(&panels, window, |this, _, window, cx| {
                this.sync_reviewer_inputs(window, cx);
                cx.notify();
            }),
        ];
        subscriptions.push(
            cx.subscribe(&session_id, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.panels
                        .update(cx, |panels, cx| panels.set_reviewer_session_id(&value, cx));
                }
            }),
        );
        subscriptions.push(cx.subscribe(
            &session_comment,
            |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.panels.update(cx, |panels, cx| {
                        panels.reviewer.set_session_comment(&value);
                        cx.notify();
                    });
                }
            },
        ));
        subscriptions.push(
            cx.subscribe(&card_comment, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.panels.update(cx, |panels, cx| {
                        panels.reviewer.set_card_comment(&value);
                        cx.notify();
                    });
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&template_id, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.panels
                        .update(cx, |panels, _| panels.reviewer.set_template_id(&value));
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&proposal_id, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.panels
                        .update(cx, |panels, _| panels.reviewer.set_proposal_id(&value));
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &template_query,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.search_templates(window, cx);
                }
            },
        ));

        let mut this = Self {
            model,
            panels,
            tab: if reviewer_tab {
                AssistantTab::Reviewer
            } else {
                AssistantTab::Drafting
            },
            cards_expanded: false,
            suggestions_shown: true,
            session_id,
            session_comment,
            card_comment,
            template_query,
            template_id,
            proposal_id,
            history_query,
            draft_session,
            drafting: DraftingState {
                rows: Vec::new(),
                summary: None,
                transcription: None,
                redistribution: Vec::new(),
                guard: None,
                pending: Vec::new(),
                error: None,
                tasks: Vec::new(),
            },
            _subscriptions: subscriptions,
        };
        this.sync_reviewer_inputs(window, cx);
        this
    }

    /// Mirrors workspace values adopted from backend responses into the inputs.
    fn sync_reviewer_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reviewer = &self.panels.read(cx).reviewer;
        let values = [
            (self.session_id.clone(), reviewer.session_id().to_string()),
            (self.template_id.clone(), reviewer.template_id().to_string()),
            (self.proposal_id.clone(), reviewer.proposal_id().to_string()),
        ];
        let comments = [
            (
                self.session_comment.clone(),
                reviewer.session_comment().to_string(),
            ),
            (
                self.card_comment.clone(),
                reviewer.card_comment().to_string(),
            ),
        ];
        for (input, value) in values {
            if input.read(cx).value() != value {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        for (input, value) in comments {
            if input.read(cx).value() != value {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
    }

    fn status(&self, message: impl Into<SharedString>, cx: &mut App) {
        let message = message.into();
        self.model
            .update(cx, |model, cx| model.set_status(message, cx));
    }

    fn search_templates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.template_query.read(cx).value().to_string();
        self.panels.update(cx, |panels, cx| {
            panels.search_reviewer_templates(query, window, cx);
        });
    }

    // ---- drafting -------------------------------------------------------------------------

    fn drafting_config(&self, cx: &App) -> DraftingConfig {
        let settings = &cx.global::<AppServices>().drafting;
        DraftingConfig::from_inputs(
            &settings.openrouter_api_key,
            &settings.model,
            &settings.service_tier,
            &settings.reasoning_effort,
            &self.draft_session.read(cx).value(),
        )
    }

    fn drafting_pending(&self, operation: &str) -> bool {
        self.drafting.pending.contains(&operation)
    }

    fn finish_drafting(&mut self, operation: &str) {
        self.drafting
            .pending
            .retain(|pending| *pending != operation);
    }

    /// Encodes the decoded lane audio to WAV tracks off the UI thread.
    fn capture_audio(
        &self,
        cx: &mut Context<Self>,
    ) -> Task<(DraftAudioCueStatus, Vec<AudioTrack>)> {
        let doc = self.model.read(cx).doc();
        let project: Project = doc.project().clone();
        let audio: BTreeMap<String, Arc<DecodedAudio>> = doc
            .session()
            .lane_waveforms
            .iter()
            .filter_map(|(lane, waveform)| Some((lane.clone(), waveform.decoded_audio_shared()?)))
            .collect();
        cx.background_spawn(async move {
            let lookup = |lane: &str| audio.get(lane).map(AsRef::as_ref);
            let status = draft_audio_cue_status(&project, lookup);
            let tracks = broker_audio_tracks(&project, lookup);
            (status, tracks)
        })
    }

    /// "Generate Rows" (and the audio guard's Retry / Continue Text Only / Use Captured Audio).
    pub fn generate_rows(
        &mut self,
        mode: DraftAudioMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drafting_pending("generate") {
            return self.status("Gold Drafting row generation is already pending.", cx);
        }
        let config = self.drafting_config(cx);
        let payload =
            match build_draft_generate_payload(self.model.read(cx).doc().project(), &config) {
                Ok(payload) => payload,
                Err(error) => return self.status(format!("Drafting unavailable: {error:#}"), cx),
            };
        self.drafting.pending.push("generate");
        self.drafting.error = None;
        let audio = self.capture_audio(cx);
        let task = cx.spawn_in(window, async move |this, cx| {
            let (status, tracks) = audio.await;
            this.update_in(cx, |this, window, cx| {
                this.send_generate(mode, status, tracks, payload, window, cx);
            })
            .ok();
        });
        self.drafting.tasks.push(task);
        cx.notify();
    }

    fn send_generate(
        &mut self,
        mode: DraftAudioMode,
        status: DraftAudioCueStatus,
        tracks: Vec<AudioTrack>,
        payload: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tracks = match resolve_draft_audio(mode, &status, tracks) {
            Ok(tracks) => tracks,
            Err(guard) => {
                self.finish_drafting("generate");
                self.drafting.guard = Some(guard);
                self.status("Audio capture needs review before generating.", cx);
                cx.notify();
                return;
            }
        };
        self.drafting.guard = None;
        self.status("Streaming Gold Drafting rows...", cx);
        let backend = cx.global::<AppServices>().review.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { backend.generate_drafts(payload, tracks).await },
            |this, result, _, cx| {
                this.finish_drafting("generate");
                match result {
                    Ok(result) => {
                        let project = this.model.read(cx).doc().project();
                        this.drafting.rows = draft_row_previews(project, &result);
                        this.drafting.summary = result.summary.clone();
                        this.status(
                            format!("Loaded {} drafted row previews.", this.drafting.rows.len()),
                            cx,
                        );
                    }
                    Err(error) => {
                        let error = format!("{error:#}");
                        this.status(format!("Draft preview failed: {error}"), cx);
                        this.drafting.error = Some(error);
                    }
                }
                cx.notify();
            },
        );
        self.drafting.tasks.push(task);
        cx.notify();
    }

    /// "Transcribe Segment" and the `DraftTranscribeSelectedSegment` hotkey: previews a
    /// transcription of the selected segment from its lane audio (applied on request).
    pub fn draft_transcribe_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drafting_pending("transcribe") {
            return self.status("Selected-segment transcription is already pending.", cx);
        }
        let config = self.drafting_config(cx);
        let doc = self.model.read(cx).doc();
        let selected = doc.session().selected_segment_id.clone();
        let payload = match build_draft_transcribe_segment_payload(
            doc.project(),
            selected.as_deref(),
            &config,
        ) {
            Ok(payload) => payload,
            Err(error) => return self.status(format!("Drafting unavailable: {error:#}"), cx),
        };
        let original = selected
            .as_deref()
            .and_then(|id| doc.project().segment(id))
            .map(|segment| segment.text.clone())
            .unwrap_or_default();
        self.drafting.pending.push("transcribe");
        self.drafting.error = None;
        let audio = self.capture_audio(cx);
        let task = cx.spawn_in(window, async move |this, cx| {
            let (_, tracks) = audio.await;
            this.update_in(cx, |this, window, cx| {
                this.send_transcribe(selected, original, tracks, payload, window, cx);
            })
            .ok();
        });
        self.drafting.tasks.push(task);
        cx.notify();
    }

    fn send_transcribe(
        &mut self,
        selected: Option<String>,
        original: String,
        tracks: Vec<AudioTrack>,
        payload: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tracks.is_empty() {
            self.finish_drafting("transcribe");
            self.status(
                "Drafting transcription blocked: load native lane audio first.",
                cx,
            );
            cx.notify();
            return;
        }
        self.status("Transcribing selected segment...", cx);
        let backend = cx.global::<AppServices>().review.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { backend.transcribe_segment(payload, tracks).await },
            move |this, result, _, cx| {
                this.finish_drafting("transcribe");
                match result {
                    Ok(result) => {
                        this.drafting.transcription = Some(TranscriptionPreview {
                            segment_id: selected,
                            original,
                            result,
                        });
                        this.status("Loaded drafted segment transcription preview.", cx);
                    }
                    Err(error) => {
                        let error = format!("{error:#}");
                        this.status(format!("Draft transcription preview failed: {error}"), cx);
                        this.drafting.error = Some(error);
                    }
                }
                cx.notify();
            },
        );
        self.drafting.tasks.push(task);
        cx.notify();
    }

    fn redistribute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drafting_pending("redistribute") {
            return self.status("Text redistribution is already pending.", cx);
        }
        let config = self.drafting_config(cx);
        let doc = self.model.read(cx).doc();
        let selected = doc.session().selected_segment_id.clone();
        let payload =
            match build_draft_redistribute_payload(doc.project(), selected.as_deref(), &config) {
                Ok(payload) => payload,
                Err(error) => return self.status(format!("Drafting unavailable: {error:#}"), cx),
            };
        self.drafting.pending.push("redistribute");
        self.drafting.error = None;
        self.status("Reviewing text redistribution...", cx);
        let backend = cx.global::<AppServices>().review.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { backend.redistribute_text(payload).await },
            |this, result, _, cx| {
                this.finish_drafting("redistribute");
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

    /// Runs an undoable workflow edit through the doc; `Err` carries the status message.
    fn apply_edit<T>(
        &self,
        action: &'static str,
        cx: &mut App,
        change: impl FnOnce(&mut studio_editor::workflow::StudioWorkflow) -> anyhow::Result<T>,
    ) -> Result<T, String> {
        self.model.update(cx, |model, cx| {
            model
                .update_doc(cx, |doc| doc.edit_workflow(action, change))
                .map_err(|error| error.to_string())
        })
    }

    fn apply_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(preview) = self.drafting.rows.get(index) else {
            return;
        };
        let row = preview.row.clone();
        if preview.skipped {
            return self.status(
                format!("Apply drafting row blocked: `{}` was skipped.", row.row_id),
                cx,
            );
        }
        match self.apply_edit("Apply drafted row", cx, |workflow| {
            apply_draft_row(workflow, &row)
        }) {
            Ok(()) => {
                if let Some(preview) = self.drafting.rows.get_mut(index) {
                    preview.applied = true;
                    preview.skipped = false;
                }
                self.status(format!("Applied drafted row `{}`.", row.row_id), cx);
            }
            Err(error) => self.status(format!("Apply drafting row blocked: {error}"), cx),
        }
        cx.notify();
    }

    fn skip_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(preview) = self.drafting.rows.get_mut(index) {
            preview.skipped = true;
            preview.applied = false;
            let id = preview.row.row_id.clone();
            self.status(format!("Skipped drafted row `{id}`."), cx);
        }
        cx.notify();
    }

    fn apply_all_rows(&mut self, cx: &mut Context<Self>) {
        if self.drafting.rows.is_empty() {
            return self.status("Apply drafting blocked: no draft response available.", cx);
        }
        let rows: Vec<_> = self
            .drafting
            .rows
            .iter()
            .map(|preview| preview.row.clone())
            .collect();
        match self.apply_edit("Apply drafted rows", cx, |workflow| {
            apply_draft_rows(workflow, &rows)
        }) {
            Ok(applied) => {
                for preview in &mut self.drafting.rows {
                    if applied.applied.contains(&preview.row.row_id) {
                        preview.applied = true;
                    }
                }
                let message = if applied.missing.is_empty() {
                    format!("Applied {} drafted rows.", applied.applied.len())
                } else {
                    format!(
                        "Applied {} drafted rows; missing {} rows.",
                        applied.applied.len(),
                        applied.missing.len()
                    )
                };
                self.status(message, cx);
            }
            Err(error) => self.status(error, cx),
        }
        cx.notify();
    }

    fn apply_transcription(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.drafting.transcription.as_ref() else {
            return self.status(
                "Apply drafting segment blocked: no transcription response available.",
                cx,
            );
        };
        let text = preview.result.text.trim().to_string();
        if text.is_empty() {
            return self.status(
                "Apply drafting segment blocked: response did not include text.",
                cx,
            );
        }
        let target = preview
            .result
            .segment_id
            .clone()
            .or_else(|| preview.segment_id.clone())
            .or_else(|| {
                self.model
                    .read(cx)
                    .doc()
                    .session()
                    .selected_segment_id
                    .clone()
            });
        let Some(target) = target else {
            return self.status("Apply drafting segment blocked: no selected segment.", cx);
        };
        match self.apply_edit("Apply drafted text", cx, |workflow| {
            apply_segment_text(workflow, &target, &text)
        }) {
            Ok(()) => self.status(format!("Applied drafted text to `{target}`."), cx),
            Err(error) => self.status(
                format!("Apply drafting segment failed for `{target}`: {error}"),
                cx,
            ),
        }
        cx.notify();
    }

    fn apply_group(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(preview) = self.drafting.redistribution.get(index).cloned() else {
            return;
        };
        if let Some(error) = &preview.error {
            return self.status(format!("Apply redistribution blocked: {error}"), cx);
        }
        match self.apply_edit("Apply redistribution", cx, |workflow| {
            apply_redistribution(workflow, &preview)
        }) {
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

    fn render_reviewer(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let panels = self.panels.read(cx);
        let reviewer = &panels.reviewer;
        let pending = |op: &str| panels.reviewer_is_pending(op);
        let theme = cx.theme();
        let p = self.panels.clone();
        let action =
            move |id: &'static str,
                  label: &'static str,
                  busy: bool,
                  run: fn(&mut TaskPanels, &mut Window, &mut Context<TaskPanels>)| {
                let p = p.clone();
                Button::new(id)
                    .small()
                    .outline()
                    .label(label)
                    .loading(busy)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        p.update(cx, |panels, cx| run(panels, window, cx))
                    })
            };

        let mut body = v_flex().gap_4();

        // Generate / session.
        body = body.child(
            v_flex()
                .gap_2()
                .child(caption("Review", cx))
                .child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .child(action(
                            "rv-generate",
                            "Generate Review",
                            pending("generate"),
                            TaskPanels::generate_review,
                        ))
                        .child(action(
                            "rv-create",
                            "Create Session",
                            pending("session"),
                            TaskPanels::create_reviewer_session,
                        ))
                        .child(action(
                            "rv-analytics",
                            "Analytics",
                            pending("analytics"),
                            TaskPanels::submit_reviewer_analytics,
                        )),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1().child(Input::new(&self.session_id).small()))
                        .child(action(
                            "rv-load",
                            "Load",
                            pending("session"),
                            TaskPanels::load_reviewer_session,
                        )),
                )
                .children(reviewer.generated_review.as_ref().map(|review| {
                    muted(
                        format!(
                            "Generated review: {} card(s), {} feedback item(s){}",
                            review.cards.len(),
                            review.feedback.len(),
                            review
                                .model
                                .as_deref()
                                .map(|m| format!(" ({m})"))
                                .unwrap_or_default()
                        ),
                        cx,
                    )
                })),
        );

        // Session + comments.
        if let Some(session) = reviewer.session() {
            let matched = session
                .cards
                .iter()
                .filter(|card| card.matched_template_id.is_some())
                .count();
            let save_label = if reviewer.comment_save_in_flight() {
                "Saving Comments..."
            } else if reviewer.comments_dirty() {
                "Save Comment Changes"
            } else {
                "Save Comments"
            };
            let save_hint = if reviewer.comment_save_in_flight() {
                "Saving the session comment and all card comments..."
            } else if reviewer.comments_dirty() {
                "Unsaved comment changes. They will auto-save shortly, or save them now."
            } else {
                "Session and card comments are saved."
            };
            let selected = reviewer.selected_card_id().to_string();
            let cards = session
                .cards
                .iter()
                .map(|card| self.render_card(card, card.id == selected, cx));
            body = body.child(
                v_flex()
                    .gap_2()
                    .child(caption("Session", cx))
                    .child(muted(
                        format!(
                            "Session {}: {} cards, {matched} matched, {} suggestions",
                            session.session_id,
                            session.cards.len(),
                            session.suggestions.len()
                        ),
                        cx,
                    ))
                    .child(Textarea::new(&self.session_comment))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(action(
                                "rv-save-comments",
                                save_label,
                                reviewer.comment_save_in_flight(),
                                TaskPanels::save_reviewer_comments,
                            ))
                            .child(muted(save_hint, cx)),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .child(caption("Card comments", cx))
                            .child(
                                Button::new("rv-expand")
                                    .ghost()
                                    .xsmall()
                                    .label(if self.cards_expanded { "Collapse All" } else { "Expand All" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cards_expanded = !this.cards_expanded;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(muted("Select a card to edit its comment. Saving preserves comments on every card.", cx))
                    .when(session.cards.is_empty(), |this| {
                        this.child(muted("No changes were detected for this review.", cx))
                    })
                    .children(cards)
                    .when(!selected.is_empty(), |this| {
                        let p = self.panels.clone();
                        this.child(
                            v_flex()
                                .gap_1()
                                .child(caption(format!("Selected card {selected}"), cx))
                                .child(Textarea::new(&self.card_comment))
                                .child(
                                    Button::new("rv-apply-card")
                                        .small()
                                        .outline()
                                        .label("Apply Feedback")
                                        .on_click(move |_, window, cx| {
                                            p.update(cx, |panels, cx| panels.apply_selected_card_feedback(window, cx));
                                        }),
                                ),
                        )
                    }),
            );

            // Suggestions.
            let suggestions = session.suggestions.iter().take(20).map(|suggestion| {
                let title = if suggestion.title.trim().is_empty() {
                    suggestion
                        .target_template_id
                        .clone()
                        .unwrap_or_else(|| "Untitled suggestion".to_string())
                } else {
                    suggestion.title.clone()
                };
                let proposal = suggestion.proposal_id.clone();
                let p = self.panels.clone();
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_xs().child(compact(&format!(
                        "{} [{}; {}; {}] {title}",
                        suggestion.proposal_id,
                        suggestion.operation,
                        suggestion.category,
                        suggestion.decision
                    ))))
                    .child(
                        Button::new(SharedString::from(format!("rv-proposal-{proposal}")))
                            .ghost()
                            .xsmall()
                            .label("Select")
                            .on_click(move |_, _, cx| {
                                p.update(cx, |panels, cx| {
                                    panels.reviewer.set_proposal_id(&proposal);
                                    cx.notify();
                                });
                            }),
                    )
            });
            let suggestion_count = session.suggestions.len();
            body = body.child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(caption(
                                if suggestion_count == 0 {
                                    "Template suggestions".to_string()
                                } else {
                                    format!("Improve the system: {suggestion_count} suggestion(s)")
                                },
                                cx,
                            ))
                            .child(
                                Button::new("rv-suggestions-toggle")
                                    .ghost()
                                    .xsmall()
                                    .label(if self.suggestions_shown {
                                        "Hide Suggestions"
                                    } else {
                                        "Show Suggestions"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.suggestions_shown = !this.suggestions_shown;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.suggestions_shown, |this| {
                        this.when(suggestion_count == 0, |this| {
                            this.child(muted("No template suggestions yet.", cx))
                        })
                        .children(suggestions)
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(action(
                                "rv-suggest",
                                if suggestion_count == 0 {
                                    "Suggest Templates"
                                } else {
                                    "Rescan Suggestions"
                                },
                                pending("suggestions"),
                                TaskPanels::suggest_reviewer_templates,
                            ))
                            .child(
                                div()
                                    .w(rems(7.5))
                                    .child(Input::new(&self.proposal_id).small()),
                            )
                            .child(action(
                                "rv-approve",
                                "Approve",
                                pending("suggestions"),
                                |p, w, cx| p.decide_reviewer_suggestion("approved", w, cx),
                            ))
                            .child(action(
                                "rv-reject",
                                "Reject",
                                pending("suggestions"),
                                |p, w, cx| p.decide_reviewer_suggestion("rejected", w, cx),
                            )),
                    ),
            );

            // Finalize.
            body = body.child(
                v_flex()
                    .gap_2()
                    .child(caption("Finalize", cx))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(action(
                                "rv-finalize-apply",
                                "Apply Review",
                                reviewer.finalize_pending().is_some(),
                                |p, w, cx| p.finalize_review("apply", w, cx),
                            ))
                            .child(action(
                                "rv-finalize-skip",
                                "Skip Review",
                                reviewer.finalize_pending().is_some(),
                                |p, w, cx| p.finalize_review("skip", w, cx),
                            )),
                    )
                    .children(reviewer.finalize_result.as_ref().map(|finalize| {
                        let mut line = format!(
                            "Finalize result: {} [{}]",
                            finalize.session_id, finalize.mode
                        );
                        if let Some(at) = &finalize.applied_at {
                            line.push_str(&format!(" at {at}"));
                        }
                        v_flex().gap_0p5().child(muted(line, cx)).children(
                            finalize.feedback.iter().map(|feedback| {
                                muted(
                                    compact(&format!("{}: {}", feedback.category, feedback.note)),
                                    cx,
                                )
                            }),
                        )
                    })),
            );
        }

        // Templates.
        let templates = reviewer.template_results.iter().take(5).map(|template| {
            let id = template.id.clone();
            let p = self.panels.clone();
            h_flex()
                .gap_2()
                .child(div().flex_1().min_w_0().text_xs().child(compact(&format!(
                    "{} [{}] {} ({})",
                    template.id, template.category, template.title, template.score
                ))))
                .child(
                    Button::new(SharedString::from(format!("rv-template-{id}")))
                        .ghost()
                        .xsmall()
                        .label("Use")
                        .on_click(move |_, _, cx| {
                            p.update(cx, |panels, cx| {
                                panels.reviewer.set_template_id(&id);
                                cx.notify();
                            });
                        }),
                )
        });
        let template_count = reviewer.template_results.len();
        body = body.child(
            v_flex()
                .gap_2()
                .child(caption("Templates", cx))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.template_query).small()),
                        )
                        .child(
                            Button::new("rv-template-search")
                                .small()
                                .outline()
                                .label("Search")
                                .loading(pending("templates"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.search_templates(window, cx)
                                })),
                        ),
                )
                .when(template_count > 0, |this| {
                    this.child(muted(format!("Templates: {template_count} result(s)"), cx))
                })
                .children(templates)
                .child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1().child(Input::new(&self.template_id).small()))
                        .child(action(
                            "rv-match",
                            "Match",
                            pending("template-match"),
                            TaskPanels::match_reviewer_template,
                        ))
                        .child(action(
                            "rv-clear",
                            "Clear",
                            pending("template-match"),
                            TaskPanels::clear_reviewer_template,
                        )),
                ),
        );

        // History + backend responses.
        let history = self.panels.clone();
        let history_query = self.history_query.clone();
        body = body.child(
            v_flex().gap_2().child(caption("History", cx)).child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.history_query).small()),
                    )
                    .child(
                        Button::new("rv-history")
                            .small()
                            .outline()
                            .label("History")
                            .loading(pending("history"))
                            .on_click(move |_, window, cx| {
                                let query = history_query.read(cx).value().to_string();
                                history.update(cx, |panels, cx| {
                                    panels.load_reviewer_history(&query, window, cx)
                                });
                            }),
                    ),
            ),
        );
        let _ = window;
        body.children(response_tail(
            panels.reviewer_error(),
            panels.reviewer_last_response.as_deref(),
            theme.danger,
            cx,
        ))
    }

    fn render_card(&self, card: &ReviewerCard, selected: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let template = card
            .template_title
            .clone()
            .or_else(|| card.matched_template_id.clone())
            .unwrap_or_else(|| "No template selected".to_string());
        let summary = compact(&format!(
            "#{} {} [{}] {} - {template}",
            card.change_index, card.id, card.kind, card.summary
        ));
        let id = card.id.clone();
        let p = self.panels.clone();
        let mut details = Vec::new();
        if self.cards_expanded {
            if !card.categories.is_empty() {
                details.push(format!("Categories: {}", card.categories.join(", ")));
            }
            for (label, value) in [
                ("Evidence", Some(card.evidence.as_str())),
                ("Opinion", card.opinion_text.as_deref()),
                ("Rationale", card.rationale.as_deref()),
                ("Comment", Some(card.comment.as_str())),
            ] {
                if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
                    details.push(compact(&format!("{label}: {value}")));
                }
            }
        }
        v_flex()
            .gap_0p5()
            .p_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                theme.border
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_xs().child(summary))
                    .child(
                        Button::new(SharedString::from(format!("rv-card-{id}")))
                            .ghost()
                            .xsmall()
                            .label("Select")
                            .selected(selected)
                            .on_click(move |_, _, cx| {
                                p.update(cx, |panels, cx| {
                                    panels.reviewer.select_card(&id);
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .children(details.into_iter().map(|line| muted(line, cx)))
    }

    fn render_drafting(&self, cx: &mut Context<Self>) -> Div {
        let doc = self.model.read(cx).doc();
        let project = doc.project();
        let audio = draft_audio_cue_status(project, |lane| {
            doc.session()
                .lane_waveforms
                .get(lane)
                .and_then(|w| w.decoded_audio())
        });
        let mut audio_line = format!(
            "Audio cues: {}/{} lanes ready",
            audio.loaded_tracks, audio.total_lanes
        );
        if !audio.missing_lane_labels.is_empty() {
            let missing: Vec<String> = audio
                .missing_lane_labels
                .iter()
                .zip(
                    audio
                        .missing_lane_sources
                        .iter()
                        .map(Some)
                        .chain(std::iter::repeat(None)),
                )
                .map(
                    |(label, source)| match source.filter(|s| !s.trim().is_empty()) {
                        Some(source) => format!("{label} ({source})"),
                        None => label.clone(),
                    },
                )
                .collect();
            audio_line.push_str(&format!("; missing {}", compact(&missing.join(", "))));
        }
        let read_only = doc.is_read_only();
        let theme = cx.theme();
        let button = |id: &'static str, label: &'static str, busy: bool| {
            Button::new(id)
                .small()
                .outline()
                .label(label)
                .loading(busy)
                .disabled(busy)
        };

        let mut body = v_flex().gap_4().child(
            v_flex()
                .gap_2()
                .child(caption("Gold Drafting", cx))
                .child(Input::new(&self.draft_session).small())
                .child(muted(audio_line, cx))
                .child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .child(
                            button(
                                "dr-generate",
                                "Generate Rows",
                                self.drafting_pending("generate"),
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.generate_rows(DraftAudioMode::Guard, window, cx)
                                },
                            )),
                        )
                        .child(
                            button(
                                "dr-transcribe",
                                "Transcribe Segment",
                                self.drafting_pending("transcribe"),
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.draft_transcribe_selected(window, cx),
                            )),
                        )
                        .child(
                            button(
                                "dr-redistribute",
                                "Redistribute Text",
                                self.drafting_pending("redistribute"),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.redistribute(window, cx)),
                            ),
                        ),
                )
                .child(muted(
                    "Backend key, model, tier and reasoning are set in Settings.",
                    cx,
                )),
        );

        if let Some(guard) = &self.drafting.guard {
            let mut message = guard.message.clone();
            message.push_str(&format!(
                " Loaded {}/{} speaker lane(s); captured {} audio track(s).",
                guard.captured_speaker_lanes, guard.expected_speaker_lanes, guard.captured_tracks
            ));
            body = body.child(
                v_flex()
                    .gap_2()
                    .p_2()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.warning)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Audio capture needs review"),
                    )
                    .child(muted(message, cx))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("dr-retry-audio")
                                    .small()
                                    .outline()
                                    .label("Retry Audio")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.generate_rows(DraftAudioMode::Guard, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("dr-text-only")
                                    .small()
                                    .outline()
                                    .label("Continue Text Only")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.generate_rows(DraftAudioMode::TextOnly, window, cx)
                                    })),
                            )
                            .when(guard.can_use_partial_audio, |this| {
                                this.child(
                                    Button::new("dr-use-audio")
                                        .small()
                                        .outline()
                                        .label("Use Captured Audio")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.generate_rows(
                                                DraftAudioMode::UseAvailableAudio,
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                            }),
                    ),
            );
        }

        if !self.drafting.rows.is_empty() {
            let summary = match &self.drafting.summary {
                Some(summary) => format!(
                    "Draft rows: {} rewritten, {} unchanged, {} failed",
                    summary.rewritten_rows, summary.unchanged_rows, summary.failed_rows
                ),
                None => format!("{} row(s)", self.drafting.rows.len()),
            };
            let total = self.drafting.rows.len();
            let rows = self
                .drafting
                .rows
                .iter()
                .take(PREVIEW_ROW_LIMIT)
                .enumerate()
                .map(|(index, preview)| {
                    let mut title = format!("{} [{}]", preview.row.row_id, preview.row.status);
                    if preview.applied {
                        title.push_str(" applied");
                    }
                    if preview.skipped {
                        title.push_str(" skipped");
                    }
                    let mut diff_line = format!(
                        "Diff: {}",
                        word_diff(&preview.original_text, &preview.row.rewritten_text)
                    );
                    if !preview.row.warnings.is_empty() {
                        diff_line
                            .push_str(&format!("; warnings: {}", preview.row.warnings.join(", ")));
                    }
                    v_flex()
                        .gap_0p5()
                        .p_2()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.border)
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "dr-row-apply-{index}"
                                    )))
                                    .xsmall()
                                    .outline()
                                    .label("Apply")
                                    .disabled(read_only || preview.applied)
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.apply_row(index, cx)
                                        }),
                                    ),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("dr-row-skip-{index}")))
                                        .xsmall()
                                        .ghost()
                                        .label("Skip")
                                        .disabled(preview.skipped)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.skip_row(index, cx)
                                        })),
                                ),
                        )
                        .child(muted(
                            compact(&format!("Before: {}", preview.original_text)),
                            cx,
                        ))
                        .child(muted(
                            compact(&format!("After: {}", preview.row.rewritten_text)),
                            cx,
                        ))
                        .child(muted(compact(&diff_line), cx))
                });
            body = body.child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(caption(summary, cx))
                            .child(
                                Button::new("dr-apply-rows")
                                    .small()
                                    .primary()
                                    .label("Apply Rows")
                                    .disabled(read_only)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.apply_all_rows(cx)),
                                    ),
                            ),
                    )
                    .when(total > PREVIEW_ROW_LIMIT, |this| {
                        this.child(muted(
                            format!("Showing first {PREVIEW_ROW_LIMIT} of {total} changed rows."),
                            cx,
                        ))
                    })
                    .children(rows),
            );
        }

        if let Some(preview) = &self.drafting.transcription {
            let id = preview
                .result
                .segment_id
                .clone()
                .or_else(|| preview.segment_id.clone())
                .map(|id| format!(" {id}"))
                .unwrap_or_default();
            let headline = if preview.original.trim().is_empty() {
                preview.result.text.clone()
            } else {
                word_diff(&preview.original, &preview.result.text)
            };
            let model = preview
                .result
                .model
                .as_deref()
                .map(|m| format!(" ({m})"))
                .unwrap_or_default();
            body = body.child(
                v_flex()
                    .gap_1()
                    .child(caption("Segment transcription", cx))
                    .child(div().text_sm().child(compact(&format!(
                        "Segment transcription{id}: {headline}{model}"
                    ))))
                    .child(muted(compact(&format!("Before: {}", preview.original)), cx))
                    .child(muted(
                        compact(&format!("After: {}", preview.result.text)),
                        cx,
                    ))
                    .child(
                        Button::new("dr-apply-segment")
                            .small()
                            .primary()
                            .label("Apply Segment")
                            .disabled(read_only)
                            .on_click(cx.listener(|this, _, _, cx| this.apply_transcription(cx))),
                    ),
            );
        }

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
        body.children(response_tail(
            self.drafting.error.as_deref(),
            None,
            theme.danger,
            cx,
        ))
    }
}

/// `Error: …` / `Last response: …` footer line.
fn response_tail(
    error: Option<&str>,
    response: Option<&str>,
    danger: Hsla,
    cx: &App,
) -> Option<Div> {
    match (error, response) {
        (Some(error), _) => Some(
            div()
                .text_xs()
                .text_color(danger)
                .child(compact(&format!("Error: {error}"))),
        ),
        (None, Some(response)) => Some(muted(compact(&format!("Last response: {response}")), cx)),
        (None, None) => None,
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

/// `common [-removed] [+added] common` over whitespace tokens.
fn word_diff(original: &str, revised: &str) -> String {
    let before: Vec<&str> = original.split_whitespace().collect();
    let after: Vec<&str> = revised.split_whitespace().collect();
    if before == after {
        return after.join(" ");
    }
    if before.is_empty() || after.is_empty() {
        return format!("{} -> {}", before.join(" "), after.join(" "));
    }
    let prefix = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let removed = &before[prefix..before.len() - suffix];
    let added = &after[prefix..after.len() - suffix];
    let mut parts: Vec<String> = before[..prefix].iter().map(|s| s.to_string()).collect();
    if !removed.is_empty() {
        parts.push(format!("[-{}]", removed.join(" ")));
    }
    if !added.is_empty() {
        parts.push(format!("[+{}]", added.join(" ")));
    }
    parts.extend(
        before[before.len() - suffix..]
            .iter()
            .map(|s| s.to_string()),
    );
    parts.join(" ")
}

impl Render for CapabilityPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !Self::is_open(self.panels.read(cx)) {
            return div().into_any_element();
        }
        let reviewer_available = self.panels.read(cx).reviewer_available(cx);
        let tab = if reviewer_available {
            self.tab
        } else {
            AssistantTab::Drafting
        };
        let body = match tab {
            AssistantTab::Reviewer => self.render_reviewer(window, cx),
            AssistantTab::Drafting => self.render_drafting(cx),
        };
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
                    )
                    .child(
                        // A button group instead of `TabBar`: gpui-component tabs have fixed
                        // pixel heights and clip their text when the interface is zoomed.
                        ButtonGroup::new("assistant-tabs")
                            .outline()
                            .small()
                            .child(
                                Button::new("assistant-tab-reviewer")
                                    .label(if reviewer_available {
                                        "Reviewer"
                                    } else {
                                        "Reviewer: L2 only"
                                    })
                                    .selected(tab == AssistantTab::Reviewer)
                                    .disabled(!reviewer_available),
                            )
                            .child(
                                Button::new("assistant-tab-drafting")
                                    .label("Gold Drafting")
                                    .selected(tab == AssistantTab::Drafting),
                            )
                            .on_click(cx.listener(move |this, clicked: &Vec<usize>, _, cx| {
                                this.tab = if clicked.contains(&0) && reviewer_available {
                                    AssistantTab::Reviewer
                                } else {
                                    AssistantTab::Drafting
                                };
                                cx.notify();
                            })),
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
