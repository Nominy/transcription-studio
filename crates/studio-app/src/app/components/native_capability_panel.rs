use iced::widget::{
    Space, button, column, container, horizontal_rule, row, scrollable, text, text_input,
};
use iced::{Background, Color, Element, Fill, Length, Theme, border};

use super::super::native_capabilities::{
    DraftRedistributionPreview, DraftRowPreview, DraftTranscriptionPreview, NativeCapabilityHealth,
    ReviewerCardPreview, ReviewerFinalizePreview, ReviewerGeneratedReviewPreview,
    ReviewerSessionPreview, ReviewerSuggestionPreview, apply_redistribution_moves_to_allocations,
};
use super::super::state::{DraftAudioCueStatus, DraftAudioGuard, NativeCapabilityModalTab};
use super::super::styles::{modal_surface_style, outline_button_style, subtle_panel_style};
use super::super::{Message, StudioState};

const NATIVE_CAPABILITY_MODAL_WIDTH: f32 = 920.0;
const NATIVE_CAPABILITY_MODAL_HEIGHT: f32 = 680.0;

pub(super) fn view_native_capability_panel(state: &StudioState) -> Element<'_, Message> {
    let active_tab = if state.native_capability_modal_tab == NativeCapabilityModalTab::Reviewer
        && !state.reviewer_capability_available()
    {
        NativeCapabilityModalTab::GoldDrafting
    } else {
        state.native_capability_modal_tab
    };
    let active_workspace = match active_tab {
        NativeCapabilityModalTab::Reviewer => view_reviewer_workspace_section(state),
        NativeCapabilityModalTab::GoldDrafting => view_gold_drafting_section(state),
    };

    let mut panel = column![view_capability_modal_tabs(state), active_workspace].spacing(10);

    if let Some(audio_status) = state.draft_audio_cue_status() {
        panel = panel.push(text(draft_audio_cue_status_label(&audio_status)).size(12));
    }

    if let Some(audio_guard) = state.draft_audio_guard.as_ref() {
        let mut guard = column![text(draft_audio_guard_label(audio_guard)).size(12)].spacing(6);
        let mut actions = row![
            action_button("Retry Audio", Message::DraftRetryAudioCapture),
            action_button("Continue Text Only", Message::DraftContinueTextOnly),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        if audio_guard.can_use_partial_audio {
            actions = actions.push(action_button(
                "Use Captured Audio",
                Message::DraftUseCapturedAudio,
            ));
        }
        guard = guard.push(actions);
        panel = panel.push(
            container(guard)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    if let Some(health) = state.native_capability_health.as_ref() {
        panel = panel.push(text(native_health_label(health)).size(12));
    }

    if let Some(generated) = state.reviewer_generated_review_preview.as_ref() {
        let mut review = column![
            text(format!(
                "Generated review: {} card(s), {} feedback item(s){}",
                generated.cards.len(),
                generated.feedback.len(),
                generated
                    .model
                    .as_ref()
                    .map(|model| format!(" ({model})"))
                    .unwrap_or_default()
            ))
            .size(12)
        ]
        .spacing(4);
        for card in generated.cards.iter().take(5) {
            for line in reviewer_card_detail_lines(card) {
                review = review.push(text(line).size(12));
            }
        }
        for feedback in generated.feedback.iter().take(3) {
            review = review.push(
                text(format!(
                    "{}: {}",
                    feedback.category,
                    compact_preview(&feedback.note)
                ))
                .size(12),
            );
        }
        panel = panel.push(
            container(review)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    if let Some(finalize) = state.reviewer_finalize_preview.as_ref() {
        let mut finalize_view = column![].spacing(4);
        for line in reviewer_finalize_detail_lines(finalize) {
            finalize_view = finalize_view.push(text(line).size(12));
        }
        panel = panel.push(
            container(finalize_view)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    let selected_card_lines = reviewer_selected_card_detail_lines(
        &state.reviewer_card_id,
        state.reviewer_generated_review_preview.as_ref(),
        state.reviewer_session_preview.as_ref(),
    );
    if !selected_card_lines.is_empty() {
        let mut selected_card = column![].spacing(4);
        for line in selected_card_lines {
            selected_card = selected_card.push(text(line).size(12));
        }
        panel = panel.push(
            container(selected_card)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    if let Some(session) = state.reviewer_session_preview.as_ref() {
        let matched_count = session
            .cards
            .iter()
            .filter(|card| card.matched_template_id.is_some())
            .count();
        let mut cards = column![
            text(format!(
                "Session {}: {} cards, {} matched, {} suggestions",
                session.session_id,
                session.cards.len(),
                matched_count,
                session.suggestions.len()
            ))
            .size(12)
        ]
        .spacing(4);

        cards = cards.push(
            row![
                action_button("Expand All", Message::ReviewerExpandAllCards),
                action_button("Collapse All", Message::ReviewerCollapseAllCards),
                action_button(
                    if state.reviewer_suggestions_expanded {
                        "Hide Suggestions"
                    } else {
                        "Show Suggestions"
                    },
                    Message::ReviewerToggleSuggestionsExpanded,
                ),
                action_button(
                    "Rescan Suggestions",
                    Message::ReviewerGenerateTemplateSuggestions
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );

        if session.cards.is_empty() {
            cards = cards.push(text("No changes were detected for this review.").size(12));
        } else if state.reviewer_cards_expanded {
            for card in &session.cards {
                cards = cards.push(
                    row![
                        text(reviewer_card_summary_line(card)).size(12).width(Fill),
                        action_button("Select", Message::ReviewerCardIdChanged(card.id.clone())),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                );
                for line in reviewer_card_detail_lines(card).into_iter().skip(1) {
                    cards = cards.push(text(line).size(12));
                }
            }
        } else {
            for card in &session.cards {
                cards = cards.push(
                    row![
                        text(reviewer_card_summary_line(card)).size(12).width(Fill),
                        action_button("Select", Message::ReviewerCardIdChanged(card.id.clone())),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                );
            }
        }

        if state.reviewer_suggestions_expanded {
            if session.suggestions.is_empty() {
                cards = cards.push(text("No template suggestions yet.").size(12));
            } else {
                cards = cards.push(
                    text(format!(
                        "Improve the system: {} suggestion(s)",
                        session.suggestions.len()
                    ))
                    .size(12),
                );
                for suggestion in &session.suggestions {
                    for line in reviewer_suggestion_detail_lines(suggestion) {
                        cards = cards.push(text(line).size(12));
                    }
                }
            }
        }
        panel = panel.push(
            container(cards)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    if !state.reviewer_template_results.is_empty() {
        let mut templates = column![
            text(format!(
                "Templates: {} result(s)",
                state.reviewer_template_results.len()
            ))
            .size(12)
        ]
        .spacing(4);
        for template in state.reviewer_template_results.iter().take(5) {
            templates = templates.push(
                text(format!(
                    "{} [{}] {} ({})",
                    template.id, template.category, template.title, template.score
                ))
                .size(12),
            );
        }
        panel = panel.push(
            container(templates)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    if !state.draft_row_previews.is_empty() {
        let summary = state
            .draft_summary_preview
            .as_ref()
            .map(|summary| {
                format!(
                    "{} rewritten, {} unchanged, {} failed",
                    summary.rewritten_rows, summary.unchanged_rows, summary.failed_rows
                )
            })
            .unwrap_or_else(|| format!("{} row(s)", state.draft_row_previews.len()));
        let mut rows = column![text(format!("Draft rows: {summary}")).size(12)].spacing(6);
        for preview in state.draft_row_previews.iter().take(40) {
            let detail_lines = draft_row_preview_detail_lines(preview);
            let mut row_preview = column![
                row![
                    text(detail_lines.first().cloned().unwrap_or_default())
                        .size(12)
                        .width(Fill),
                    action_button("Apply", Message::DraftApplyRow(preview.row_id.clone())),
                    action_button("Skip", Message::DraftSkipRow(preview.row_id.clone())),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center)
            ]
            .spacing(3);
            for line in detail_lines.into_iter().skip(1) {
                row_preview = row_preview.push(text(line).size(12));
            }
            rows = rows.push(row_preview);
        }
        if state.draft_row_previews.len() > 40 {
            rows = rows.push(
                text(format!(
                    "Showing first 40 of {} changed rows.",
                    state.draft_row_previews.len()
                ))
                .size(12),
            );
        }
        panel = panel.push(
            container(rows)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    if let Some(transcription) = state.draft_transcription_preview.as_ref() {
        let detail_lines = draft_transcription_preview_detail_lines(transcription);
        let mut transcription_view = column![
            row![
                text(detail_lines.first().cloned().unwrap_or_default())
                    .size(12)
                    .width(Fill),
                action_button("Apply Segment", Message::DraftApplySegmentText),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
        ]
        .spacing(3);
        for line in detail_lines.into_iter().skip(1) {
            transcription_view = transcription_view.push(text(line).size(12));
        }
        panel = panel.push(
            container(transcription_view)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    if !state.draft_redistribution_previews.is_empty() {
        let mut groups = column![
            text(format!(
                "Redistribution: {} group(s)",
                state.draft_redistribution_previews.len()
            ))
            .size(12)
        ]
        .spacing(6);
        for preview in state.draft_redistribution_previews.iter().take(5) {
            let detail_lines = draft_redistribution_detail_lines(preview);
            let mut group = column![
                row![
                    text(detail_lines.first().cloned().unwrap_or_default())
                        .size(12)
                        .width(Fill),
                    action_button(
                        "Apply Group",
                        Message::DraftApplyRedistributionGroup(preview.group_id.clone()),
                    ),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center)
            ]
            .spacing(3);
            for line in detail_lines.into_iter().skip(1) {
                group = group.push(text(line).size(12));
            }
            groups = groups.push(group);
        }
        panel = panel.push(
            container(groups)
                .padding(8)
                .width(Fill)
                .style(subtle_panel_style),
        );
    }

    if let Some(error) = state.native_capability_last_error.as_ref() {
        panel = panel.push(text(format!("Error: {}", compact_preview(error))).size(12));
    } else if let Some(response) = state.native_capability_last_response.as_ref() {
        panel = panel.push(text(format!("Last response: {}", compact_preview(response))).size(12));
    }

    let header = row![
        column![
            text("Assistant Tools").size(16),
            text(state.status.clone()).size(12).width(Fill),
        ]
        .spacing(2)
        .width(Fill),
        action_button("Close", Message::ToggleNativeCapabilityPanel),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    container(column![header, scrollable(panel).height(Fill)].spacing(10))
        .padding(12)
        .width(Length::Fixed(NATIVE_CAPABILITY_MODAL_WIDTH))
        .height(Length::Fixed(NATIVE_CAPABILITY_MODAL_HEIGHT))
        .style(modal_surface_style)
        .into()
}

fn view_capability_modal_tabs(state: &StudioState) -> Element<'_, Message> {
    let mut tabs = row![].spacing(6).align_y(iced::Alignment::Center);
    if state.reviewer_capability_available() {
        tabs = tabs.push(tab_button(
            "Reviewer",
            NativeCapabilityModalTab::Reviewer,
            state.native_capability_modal_tab == NativeCapabilityModalTab::Reviewer,
        ));
    } else {
        tabs = tabs.push(text("Reviewer: L2 only").size(12));
    }
    tabs.push(tab_button(
        "Gold Drafting",
        NativeCapabilityModalTab::GoldDrafting,
        state.native_capability_modal_tab == NativeCapabilityModalTab::GoldDrafting
            || !state.reviewer_capability_available(),
    ))
    .into()
}

fn view_reviewer_workspace_section(state: &StudioState) -> Element<'_, Message> {
    let primary = column![
        row![
            action_button("Generate Review", Message::ReviewerGenerateReview),
            action_button("Create Session", Message::ReviewerCreateSession),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            text_input("Session ID", &state.reviewer_session_id)
                .on_input(Message::ReviewerSessionIdChanged)
                .padding(6)
                .size(13)
                .width(Fill),
            action_button("Load", Message::ReviewerLoadSession),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            action_button("Apply Review", Message::ReviewerFinalizeApply),
            action_button("Skip Review", Message::ReviewerFinalizeSkip),
            action_button("Analytics", Message::ReviewerSubmitAnalytics),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(8);

    let comments = column![
        text_input("Session comment", &state.reviewer_session_comment)
            .on_input(Message::ReviewerSessionCommentChanged)
            .padding(6)
            .size(13)
            .width(Fill),
        row![
            text_input("Selected card", &state.reviewer_card_id)
                .on_input(Message::ReviewerCardIdChanged)
                .padding(6)
                .size(13)
                .width(Fill),
            action_button("Apply Feedback", Message::ReviewerApplySelectedCardFeedback),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            text_input("Card comment", &state.reviewer_card_comment)
                .on_input(Message::ReviewerCardCommentChanged)
                .padding(6)
                .size(13)
                .width(Fill),
            action_button("Save Comments", Message::ReviewerSaveComments),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(8);

    let templates = column![
        row![
            text_input("Template query", &state.reviewer_template_query)
                .on_input(Message::ReviewerTemplateQueryChanged)
                .padding(6)
                .size(13)
                .width(Fill),
            action_button("Search", Message::ReviewerSearchTemplates),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            text_input("Template ID", &state.reviewer_template_id)
                .on_input(Message::ReviewerTemplateIdChanged)
                .padding(6)
                .size(13)
                .width(Fill),
            action_button("Match", Message::ReviewerSetTemplateMatch),
            action_button("Clear", Message::ReviewerClearTemplateMatch),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            action_button(
                "Suggest Templates",
                Message::ReviewerGenerateTemplateSuggestions
            ),
            Space::with_width(Fill),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            text_input("Proposal ID", &state.reviewer_proposal_id)
                .on_input(Message::ReviewerProposalIdChanged)
                .padding(6)
                .size(13)
                .width(Fill),
            action_button("Approve", Message::ReviewerApproveTemplateSuggestion),
            action_button("Reject", Message::ReviewerRejectTemplateSuggestion),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            text_input("History query", &state.reviewer_history_query)
                .on_input(Message::ReviewerHistoryQueryChanged)
                .padding(6)
                .size(13)
                .width(Fill),
            action_button("History", Message::ReviewerLoadHistory),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(8);

    section_container(
        "Reviewer",
        column![
            primary,
            horizontal_rule(1),
            comments,
            horizontal_rule(1),
            templates
        ]
        .spacing(10)
        .into(),
    )
}

fn view_gold_drafting_section(state: &StudioState) -> Element<'_, Message> {
    let actions = column![
        row![
            action_button("Generate Rows", Message::DraftGenerateRows),
            action_button(
                "Transcribe Segment",
                Message::DraftTranscribeSelectedSegment
            ),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            action_button("Redistribute Text", Message::DraftRedistributeText),
            action_button("Apply Segment", Message::DraftApplySegmentText),
            action_button("Apply Rows", Message::DraftApplyLastRows),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(8);

    let session = column![
        text_input("Draft session", &state.draft_session_id)
            .on_input(Message::DraftSessionIdChanged)
            .padding(6)
            .size(13)
            .width(Fill),
    ]
    .spacing(8);

    section_container(
        "Gold Drafting",
        column![actions, horizontal_rule(1), session]
            .spacing(10)
            .into(),
    )
}

fn section_container<'a>(title: &'a str, content: Element<'a, Message>) -> Element<'a, Message> {
    container(column![text(title).size(13), content].spacing(8))
        .padding(10)
        .width(Fill)
        .style(subtle_panel_style)
        .into()
}

fn action_button<'a>(label: impl Into<String>, message: Message) -> Element<'a, Message> {
    button(text(label.into()).size(13))
        .padding([6, 10])
        .style(outline_button_style)
        .on_press(message)
        .into()
}

fn tab_button<'a>(
    label: &'a str,
    tab: NativeCapabilityModalTab,
    active: bool,
) -> Element<'a, Message> {
    button(text(label).size(13))
        .padding([7, 12])
        .style(move |theme, status| tab_button_style(theme, status, active))
        .on_press(Message::NativeCapabilityModalTabChanged(tab))
        .into()
}

fn tab_button_style(_theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let background = match (active, status) {
        (_, button::Status::Pressed) => Color::from_rgb8(219, 234, 254),
        (true, _) => Color::from_rgb8(239, 246, 255),
        (false, button::Status::Hovered) => Color::from_rgb8(245, 247, 250),
        (false, _) => Color::WHITE,
    };
    let border_color = if active {
        Color::from_rgb8(60, 130, 246)
    } else {
        Color::from_rgb8(210, 216, 226)
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::from_rgb8(33, 37, 41),
        border: border::rounded(999).width(1).color(border_color),
        shadow: Default::default(),
    }
}

fn draft_audio_cue_status_label(status: &DraftAudioCueStatus) -> String {
    let base = format!(
        "Audio cues: {}/{} lanes ready",
        status.loaded_tracks, status.total_lanes
    );
    if status.missing_lane_labels.is_empty() {
        return base;
    }
    format!(
        "{}; missing {}",
        base,
        compact_preview(&draft_audio_missing_lane_labels(status).join(", "))
    )
}

fn draft_audio_guard_label(guard: &DraftAudioGuard) -> String {
    format!(
        "{} Loaded {}/{} speaker lane(s); captured {} audio track(s).",
        guard.message,
        guard.captured_speaker_lanes,
        guard.expected_speaker_lanes,
        guard.captured_tracks
    )
}

fn draft_audio_missing_lane_labels(status: &DraftAudioCueStatus) -> Vec<String> {
    status
        .missing_lane_labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let source = status
                .missing_lane_sources
                .get(index)
                .map(String::as_str)
                .unwrap_or_default()
                .trim();
            if source.is_empty() {
                label.clone()
            } else {
                format!("{label} ({source})")
            }
        })
        .collect()
}

fn native_health_label(health: &NativeCapabilityHealth) -> String {
    let mut parts = vec![format!(
        "Backend: {}{}",
        health.service,
        health
            .backend_version
            .as_ref()
            .map(|version| format!(" {version}"))
            .unwrap_or_default()
    )];
    if let Some(status) = health.openrouter_status.as_ref() {
        parts.push(status.clone());
    }
    parts.extend(health.provider_diagnostics.iter().cloned());
    parts.join("; ")
}

fn reviewer_card_detail_lines(card: &ReviewerCardPreview) -> Vec<String> {
    let mut lines = vec![format!(
        "{} [{}] {}",
        card.id,
        card.kind,
        compact_preview(&card.summary)
    )];
    let mut metadata = Vec::new();
    if !card.categories.is_empty() {
        metadata.push(format!("Categories: {}", card.categories.join(", ")));
    }
    if let Some(template_id) = card.matched_template_id.as_ref() {
        let mut template = template_id.clone();
        let mut details = Vec::new();
        if let Some(title) = card.template_title.as_ref() {
            details.push(title.clone());
        }
        if let Some(source) = card.match_source.as_ref() {
            details.push(source.clone());
        }
        if !details.is_empty() {
            template.push_str(&format!(" ({})", details.join(", ")));
        }
        metadata.push(format!("template: {template}"));
    }
    if !metadata.is_empty() {
        lines.push(metadata.join("; "));
    }
    if !card.evidence.trim().is_empty() {
        lines.push(format!("Evidence: {}", compact_preview(&card.evidence)));
    }
    if let Some(opinion) = card
        .opinion_text
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("Opinion: {}", compact_preview(opinion)));
    }
    if let Some(rationale) = card
        .rationale
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("Rationale: {}", compact_preview(rationale)));
    }
    if !card.comment.trim().is_empty() {
        lines.push(format!("Comment: {}", compact_preview(&card.comment)));
    }
    lines
}

fn reviewer_card_summary_line(card: &ReviewerCardPreview) -> String {
    let mut summary = format!(
        "#{} {} [{}] {}",
        card.change_index,
        card.id,
        card.kind,
        compact_preview(&card.summary)
    );
    let template = card
        .template_title
        .as_ref()
        .or(card.matched_template_id.as_ref())
        .map(String::as_str)
        .unwrap_or("No template selected");
    summary.push_str(&format!(" - {template}"));
    summary
}

fn reviewer_suggestion_detail_lines(suggestion: &ReviewerSuggestionPreview) -> Vec<String> {
    let mut title = suggestion.title.trim();
    if title.is_empty() {
        title = suggestion
            .target_template_id
            .as_deref()
            .unwrap_or("Untitled suggestion");
    }
    let mut lines = vec![format!(
        "{} [{}; {}; {}] {}",
        suggestion.proposal_id,
        suggestion.operation,
        suggestion.category,
        suggestion.decision,
        compact_preview(title)
    )];
    if let Some(description) = suggestion
        .description
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("Description: {}", compact_preview(description)));
    }
    if !suggestion.reason.trim().is_empty() {
        lines.push(format!("Reason: {}", compact_preview(&suggestion.reason)));
    }
    if !suggestion.report_texts.is_empty() {
        lines.push(format!(
            "Proposed text: {}",
            compact_preview(&suggestion.report_texts.join(" | "))
        ));
    }
    if !suggestion.source_card_ids.is_empty() {
        lines.push(format!(
            "Source cards: {}",
            suggestion.source_card_ids.join(", ")
        ));
    }
    lines
}

fn reviewer_selected_card_detail_lines(
    card_id: &str,
    generated: Option<&ReviewerGeneratedReviewPreview>,
    session: Option<&ReviewerSessionPreview>,
) -> Vec<String> {
    let card_id = card_id.trim();
    if card_id.is_empty() {
        return Vec::new();
    }
    let card = generated
        .and_then(|generated| generated.cards.iter().find(|card| card.id == card_id))
        .or_else(|| {
            session.and_then(|session| session.cards.iter().find(|card| card.id == card_id))
        });
    let Some(card) = card else {
        return Vec::new();
    };
    let mut lines = vec!["Selected card".to_string()];
    lines.extend(reviewer_card_detail_lines(card));
    lines
}

fn reviewer_finalize_detail_lines(preview: &ReviewerFinalizePreview) -> Vec<String> {
    let applied_at = preview
        .applied_at
        .as_ref()
        .map(|value| format!(" at {value}"))
        .unwrap_or_default();
    let backend = match (&preview.backend_service, &preview.backend_release) {
        (Some(service), Some(release)) => format!(" ({service} {release})"),
        (Some(service), None) => format!(" ({service})"),
        (None, Some(release)) => format!(" ({release})"),
        (None, None) => String::new(),
    };
    let mut lines = vec![format!(
        "Finalize result: {} [{}]{}{}",
        preview.session_id, preview.mode, applied_at, backend
    )];
    for feedback in &preview.feedback {
        let score = if feedback.score.trim().is_empty() {
            String::new()
        } else {
            format!(" [{}]", feedback.score)
        };
        lines.push(format!(
            "{}{}: {}",
            feedback.category,
            score,
            compact_preview(&feedback.note)
        ));
    }
    lines
}

fn draft_row_preview_diff_label(preview: &DraftRowPreview) -> String {
    let diff = compact_token_diff_label(&preview.original_text, &preview.rewritten_text);
    if preview.warnings.is_empty() {
        return diff;
    }
    format!("{}; warnings: {}", diff, preview.warnings.join(", "))
}

fn draft_row_preview_detail_lines(preview: &DraftRowPreview) -> Vec<String> {
    let mut title = format!("{} [{}]", preview.row_id, preview.status);
    if preview.applied {
        title.push_str(" applied");
    }
    if preview.skipped {
        title.push_str(" skipped");
    }
    vec![
        title,
        format!("Before: {}", compact_preview(&preview.original_text)),
        format!("After: {}", compact_preview(&preview.rewritten_text)),
        format!("Diff: {}", draft_row_preview_diff_label(preview)),
    ]
}

fn draft_transcription_preview_label(transcription: &DraftTranscriptionPreview) -> String {
    let model = transcription
        .model
        .as_ref()
        .map(|model| format!(" ({model})"))
        .unwrap_or_default();
    let segment = transcription
        .segment_id
        .as_ref()
        .map(|segment_id| format!(" {segment_id}"))
        .unwrap_or_default();
    if !transcription.original_text.trim().is_empty() {
        return format!(
            "Segment transcription{segment}: {}{}",
            compact_token_diff_label(&transcription.original_text, &transcription.text),
            model
        );
    }
    format!(
        "Segment transcription{segment}: {}{}",
        compact_preview(&transcription.text),
        model
    )
}

fn draft_transcription_preview_detail_lines(
    transcription: &DraftTranscriptionPreview,
) -> Vec<String> {
    vec![
        draft_transcription_preview_label(transcription),
        format!("Before: {}", compact_preview(&transcription.original_text)),
        format!("After: {}", compact_preview(&transcription.text)),
    ]
}

fn draft_redistribution_preview_label(preview: &DraftRedistributionPreview) -> String {
    let state = if preview.accept_draft {
        "accepted"
    } else if preview.error.is_some() {
        "blocked"
    } else {
        "review"
    };
    let detail = if let Some(error) = preview.error.as_ref() {
        format!(
            ", {} move(s) - {}",
            preview.moves.len(),
            compact_preview(error)
        )
    } else if let Some(notes) = preview.notes.as_ref() {
        format!(
            ", {} move(s) - {}",
            preview.moves.len(),
            compact_preview(notes)
        )
    } else {
        format!(", {} move(s)", preview.moves.len())
    };
    format!(
        "{} [{}] {} allocation(s){}",
        preview.group_id,
        state,
        preview.allocations.len(),
        detail
    )
}

fn draft_redistribution_detail_lines(preview: &DraftRedistributionPreview) -> Vec<String> {
    let mut lines = vec![draft_redistribution_preview_label(preview)];
    if preview.error.is_some() || preview.allocations.is_empty() {
        return lines;
    }
    let proposed = if preview.accept_draft {
        Some(preview.allocations.clone())
    } else if preview.moves.is_empty() {
        None
    } else {
        apply_redistribution_moves_to_allocations(&preview.allocations, &preview.moves).ok()
    };
    let Some(proposed) = proposed else {
        return lines;
    };
    for (before, after) in preview.allocations.iter().zip(proposed.iter()).take(5) {
        lines.push(format!(
            "{}: {} -> {}",
            before.segment_id,
            compact_preview(&before.text),
            compact_preview(&after.text)
        ));
    }
    lines
}

fn compact_token_diff_label(original: &str, revised: &str) -> String {
    let original = compact_preview(original);
    let revised = compact_preview(revised);
    if original == revised {
        return revised;
    }

    let original_tokens = original.split_whitespace().collect::<Vec<_>>();
    let revised_tokens = revised.split_whitespace().collect::<Vec<_>>();
    if original_tokens.is_empty() || revised_tokens.is_empty() {
        return format!("{original} -> {revised}");
    }

    let mut prefix_len = 0;
    while prefix_len < original_tokens.len()
        && prefix_len < revised_tokens.len()
        && original_tokens[prefix_len] == revised_tokens[prefix_len]
    {
        prefix_len += 1;
    }

    let mut suffix_len = 0;
    while suffix_len < original_tokens.len().saturating_sub(prefix_len)
        && suffix_len < revised_tokens.len().saturating_sub(prefix_len)
        && original_tokens[original_tokens.len() - suffix_len - 1]
            == revised_tokens[revised_tokens.len() - suffix_len - 1]
    {
        suffix_len += 1;
    }

    let original_mid_end = original_tokens.len() - suffix_len;
    let revised_mid_end = revised_tokens.len() - suffix_len;
    let mut parts = Vec::new();
    parts.extend(
        original_tokens[..prefix_len]
            .iter()
            .map(|token| (*token).to_string()),
    );
    if prefix_len < original_mid_end {
        parts.push(format!(
            "[-{}]",
            original_tokens[prefix_len..original_mid_end].join(" ")
        ));
    }
    if prefix_len < revised_mid_end {
        parts.push(format!(
            "[+{}]",
            revised_tokens[prefix_len..revised_mid_end].join(" ")
        ));
    }
    parts.extend(
        revised_tokens[revised_mid_end..]
            .iter()
            .map(|token| (*token).to_string()),
    );

    compact_preview(&parts.join(" "))
}

fn compact_preview(value: &str) -> String {
    let single_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= 180 {
        return single_line;
    }
    let mut preview = single_line.chars().take(177).collect::<String>();
    preview.push_str("...");
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::native_capabilities::{
        DraftRedistributionAllocationPreview, DraftRedistributionMovePreview,
        DraftRedistributionPreview, DraftRowPreview, DraftTranscriptionPreview,
        NativeCapabilityHealth, ReviewerCardPreview, ReviewerFeedbackPreview,
        ReviewerFinalizePreview, ReviewerGeneratedReviewPreview,
    };
    use serde_json::Value;

    #[test]
    fn draft_row_preview_diff_label_includes_before_after_and_warnings() {
        let preview = DraftRowPreview {
            row_id: "row-1".to_string(),
            original_text: "Original row text.".to_string(),
            rewritten_text: "Rewritten row text.".to_string(),
            status: "rewritten".to_string(),
            warnings: vec!["length_delta".to_string(), "punctuation".to_string()],
            applied: false,
            skipped: false,
        };

        assert_eq!(
            draft_row_preview_diff_label(&preview),
            "[-Original] [+Rewritten] row text.; warnings: length_delta, punctuation"
        );
    }

    #[test]
    fn draft_row_preview_diff_label_marks_changed_word_tokens() {
        let preview = DraftRowPreview {
            row_id: "row-1".to_string(),
            original_text: "Alpha old word.".to_string(),
            rewritten_text: "Alpha new word.".to_string(),
            status: "rewritten".to_string(),
            warnings: Vec::new(),
            applied: false,
            skipped: false,
        };

        assert_eq!(
            draft_row_preview_diff_label(&preview),
            "Alpha [-old] [+new] word."
        );
    }

    #[test]
    fn draft_transcription_preview_label_marks_token_diff_against_original() {
        let preview = DraftTranscriptionPreview {
            segment_id: Some("row-1".to_string()),
            original_text: "Alpha old word.".to_string(),
            text: "Alpha new word.".to_string(),
            model: Some("openai/gpt-4o-transcribe".to_string()),
        };

        assert_eq!(
            draft_transcription_preview_label(&preview),
            "Segment transcription row-1: Alpha [-old] [+new] word. (openai/gpt-4o-transcribe)"
        );
    }

    #[test]
    fn draft_row_preview_detail_lines_include_recovered_before_after_panes() {
        let preview = DraftRowPreview {
            row_id: "row-1".to_string(),
            original_text: "Original row text.".to_string(),
            rewritten_text: "Rewritten row text.".to_string(),
            status: "rewritten".to_string(),
            warnings: vec!["length_delta".to_string()],
            applied: false,
            skipped: false,
        };

        assert_eq!(
            draft_row_preview_detail_lines(&preview),
            vec![
                "row-1 [rewritten]".to_string(),
                "Before: Original row text.".to_string(),
                "After: Rewritten row text.".to_string(),
                "Diff: [-Original] [+Rewritten] row text.; warnings: length_delta".to_string(),
            ]
        );
    }

    #[test]
    fn draft_transcription_preview_detail_lines_include_recovered_before_after_panes() {
        let preview = DraftTranscriptionPreview {
            segment_id: Some("row-1".to_string()),
            original_text: "Old segment text.".to_string(),
            text: "New segment text.".to_string(),
            model: Some("google/gemini".to_string()),
        };

        assert_eq!(
            draft_transcription_preview_detail_lines(&preview),
            vec![
                "Segment transcription row-1: [-Old] [+New] segment text. (google/gemini)"
                    .to_string(),
                "Before: Old segment text.".to_string(),
                "After: New segment text.".to_string(),
            ]
        );
    }

    #[test]
    fn draft_audio_cue_status_label_includes_missing_lane_sources() {
        let status = DraftAudioCueStatus {
            loaded_tracks: 1,
            total_lanes: 2,
            missing_lane_labels: vec!["Speaker 2".to_string()],
            missing_lane_sources: vec!["audio/speaker_2.wav".to_string()],
        };

        assert_eq!(
            draft_audio_cue_status_label(&status),
            "Audio cues: 1/2 lanes ready; missing Speaker 2 (audio/speaker_2.wav)"
        );
    }

    #[test]
    fn native_health_label_includes_provider_diagnostics() {
        let health = NativeCapabilityHealth {
            service: "babel-review-backend".to_string(),
            backend_version: Some("0.3.9".to_string()),
            openrouter_status: Some("credits remaining: 8".to_string()),
            test_mode: Some(false),
            provider_diagnostics: vec![
                "provider: openrouter".to_string(),
                "model: openai/gpt-4o-mini".to_string(),
            ],
        };

        assert_eq!(
            native_health_label(&health),
            "Backend: babel-review-backend 0.3.9; credits remaining: 8; provider: openrouter; model: openai/gpt-4o-mini"
        );
    }

    #[test]
    fn draft_redistribution_detail_lines_include_before_after_allocations() {
        let preview = DraftRedistributionPreview {
            group_id: "speaker-group-1".to_string(),
            model: Some("openai/gpt-4o-mini".to_string()),
            accept_draft: false,
            notes: Some("Move last sentence down.".to_string()),
            moves: vec![DraftRedistributionMovePreview {
                from_index: 1,
                to_index: 2,
                sentence_count: 1,
            }],
            allocations: vec![
                DraftRedistributionAllocationPreview {
                    segment_id: "row-1".to_string(),
                    text: "Alpha. Beta.".to_string(),
                },
                DraftRedistributionAllocationPreview {
                    segment_id: "row-2".to_string(),
                    text: "Gamma.".to_string(),
                },
            ],
            error: None,
            applied: false,
        };

        assert_eq!(
            draft_redistribution_detail_lines(&preview),
            vec![
                "speaker-group-1 [review] 2 allocation(s), 1 move(s) - Move last sentence down."
                    .to_string(),
                "row-1: Alpha. Beta. -> Alpha.".to_string(),
                "row-2: Gamma. -> Beta. Gamma.".to_string(),
            ]
        );
    }

    #[test]
    fn reviewer_card_detail_lines_include_evidence_template_and_comments() {
        let card = ReviewerCardPreview {
            id: "card-1".to_string(),
            change_index: 4,
            kind: "TEXT CHANGE".to_string(),
            summary: "Text changed".to_string(),
            evidence: "[-old +new]".to_string(),
            categories: vec!["Word Accuracy".to_string(), "Punctuation".to_string()],
            matched_template_id: Some("tpl-1".to_string()),
            template_title: Some("Wrong word".to_string()),
            match_source: Some("model".to_string()),
            opinion_text: Some("Use the template.".to_string()),
            rationale: Some("The diff is a word substitution.".to_string()),
            comment: "Reviewer note".to_string(),
        };

        assert_eq!(
            reviewer_card_detail_lines(&card),
            vec![
                "card-1 [TEXT CHANGE] Text changed".to_string(),
                "Categories: Word Accuracy, Punctuation; template: tpl-1 (Wrong word, model)"
                    .to_string(),
                "Evidence: [-old +new]".to_string(),
                "Opinion: Use the template.".to_string(),
                "Rationale: The diff is a word substitution.".to_string(),
                "Comment: Reviewer note".to_string(),
            ]
        );
    }

    #[test]
    fn reviewer_finalize_detail_lines_include_mode_timestamp_and_feedback() {
        let preview = ReviewerFinalizePreview {
            session_id: "session-1".to_string(),
            mode: "skip".to_string(),
            applied_at: Some("2026-06-29T10:00:00Z".to_string()),
            backend_service: Some("babel-review-backend".to_string()),
            backend_release: Some("0.3.9".to_string()),
            feedback: vec![ReviewerFeedbackPreview {
                category: "Timestamp Accuracy".to_string(),
                score: "1".to_string(),
                note: "Timing issue note.".to_string(),
            }],
        };

        assert_eq!(
            reviewer_finalize_detail_lines(&preview),
            vec![
                "Finalize result: session-1 [skip] at 2026-06-29T10:00:00Z (babel-review-backend 0.3.9)".to_string(),
                "Timestamp Accuracy [1]: Timing issue note.".to_string(),
            ]
        );
    }

    #[test]
    fn reviewer_selected_card_detail_lines_include_card_beyond_preview_limit() {
        let cards = (0..6)
            .map(|index| ReviewerCardPreview {
                id: format!("card-{index}"),
                change_index: index,
                kind: "TEXT CHANGE".to_string(),
                summary: format!("Card {index} summary"),
                evidence: format!("Card {index} evidence"),
                categories: vec!["Word Accuracy".to_string()],
                matched_template_id: None,
                template_title: None,
                match_source: None,
                opinion_text: None,
                rationale: None,
                comment: String::new(),
            })
            .collect::<Vec<_>>();
        let generated = ReviewerGeneratedReviewPreview {
            model: None,
            received_at: None,
            template_registry_version: None,
            matched_template_ids: Vec::new(),
            feedback: Vec::new(),
            cards,
            prepared: Value::Null,
            ai_review: Value::Null,
        };

        assert_eq!(
            reviewer_selected_card_detail_lines("card-5", Some(&generated), None),
            vec![
                "Selected card".to_string(),
                "card-5 [TEXT CHANGE] Card 5 summary".to_string(),
                "Categories: Word Accuracy".to_string(),
                "Evidence: Card 5 evidence".to_string(),
            ]
        );
    }
}
