use std::{collections::BTreeSet, hash::Hasher, sync::LazyLock};

use iced::widget::svg::{self, Handle as SvgHandle, Svg};
use iced::widget::{
    Column, Row, Space, button, column, container, lazy, mouse_area, pick_list, rich_text, row,
    scrollable, span, stack, text, text_editor, text_input,
};
use iced::{Alignment, Background, Color, Element, Fill, Length, mouse};
use studio_app::{
    EditorSession, TranscriptDiffCellKind, TranscriptDiffView, TranscriptReviewerView,
    TranscriptRowKind, TranscriptRowView, TranscriptWordDiffKind, TranscriptWordDiffStatus,
    TranscriptWordDiffTokenView, TranscriptWordDiffView, TranscriptWordView,
};
use studio_core::Project;

use super::super::super::state::HelperGhostCursorView;
use super::super::super::styles::{
    diff_added_badge_style, diff_change_badge_style, diff_deleted_badge_style,
    diff_new_badge_style, diff_no_change_badge_style, diff_word_added_chip_style,
    diff_word_removed_chip_style, low_confidence_ack_button_style, low_confidence_text_shell_style,
    low_confidence_word_dotted_style, table_plain_text_editor_style, table_plain_text_input_style,
    tag_suggestion_button_style, transcript_modified_row_style, transcript_row_style,
};
use super::super::super::tag_suggestions::{self, TagSuggestion, TagSuggestionKeyboardAction};
use super::super::super::{Message, SegmentDraft, StudioState, TimeField};
use super::super::focusable_text_editor;

const LOW_CONFIDENCE_ACK_ICON_SIZE: f32 = 12.0;
const LOW_CONFIDENCE_ACK_BUTTON_SIZE: f32 = 16.0;
const LOW_CONFIDENCE_ACK_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>"#;
static LOW_CONFIDENCE_ACK_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(LOW_CONFIDENCE_ACK_ICON));

struct LowConfidenceWordToken {
    index: usize,
    text: String,
    start_seconds: Option<f64>,
}

struct IdCellDependency {
    display_id: String,
}

impl std::hash::Hash for IdCellDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.display_id.hash(state);
    }
}

struct SpeakerLabelDependency {
    segment_id: String,
    speaker_label: String,
    lane_id: String,
}

impl std::hash::Hash for SpeakerLabelDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.segment_id.hash(state);
        self.speaker_label.hash(state);
        self.lane_id.hash(state);
    }
}

pub(super) fn view_transcript_row<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
    row_view: &TranscriptRowView,
    draft: &'a SegmentDraft,
    is_find_match: bool,
) -> Element<'a, Message> {
    let start_input = if row_view.is_read_only {
        view_read_only_time_cell(row_view.start_time.clone())
    } else {
        view_time_cell(row_view.segment_id.clone(), TimeField::Start, draft)
    };
    let end_input = if row_view.is_read_only {
        view_read_only_time_cell(row_view.end_time.clone())
    } else {
        view_time_cell(row_view.segment_id.clone(), TimeField::End, draft)
    };

    let text_id = row_view.segment_id.clone();
    let text_placeholder = if row_view.annotation_kind.is_transcription() {
        "What was said..."
    } else {
        "Annotation..."
    };
    let acknowledged_words = state
        .acknowledged_low_confidence_words
        .get(&row_view.segment_id)
        .cloned()
        .unwrap_or_default();
    let mut transcript_text = if !row_view.word_views.is_empty() {
        column![view_low_confidence_transcript_text(
            row_view.segment_id.clone(),
            row_view.text.clone(),
            low_confidence_word_tokens(&row_view.word_views),
            acknowledged_words,
            row_view.is_read_only,
        )]
        .spacing(3)
        .width(Fill)
    } else if row_view.is_read_only {
        column![text(row_view.text.clone()).size(12).width(Fill)]
            .spacing(3)
            .width(Fill)
    } else {
        let raw_tag_suggestions = tag_suggestions::tag_suggestions_for_draft(draft);
        let has_text_selection = draft.text_content.selection().is_some();
        let tag_suggestions = raw_tag_suggestions
            .into_iter()
            .filter(|_| state.should_show_tag_suggestions(&row_view.segment_id, draft))
            .collect::<Vec<_>>();
        let tag_suggestions_visible = !tag_suggestions.is_empty();
        let key_binding_segment_id = row_view.segment_id.clone();
        let transcript_text_input = text_editor(&draft.text_content)
            .placeholder(text_placeholder)
            .on_action(move |action| Message::SegmentTextEdited(text_id.clone(), action))
            .key_binding(move |event| {
                babel_transcript_key_binding_with_selection(
                    &key_binding_segment_id,
                    tag_suggestions_visible,
                    has_text_selection,
                    event,
                )
            })
            .padding([2, 4])
            .size(12)
            .style(table_plain_text_editor_style)
            .height(Length::Shrink);
        let transcript_text_input = focusable_text_editor::view_focusable_text_editor(
            focusable_text_editor::transcript_text_editor_id(&row_view.segment_id),
            transcript_text_input.into(),
        );
        let helper_focus_restore_pending = state.helper_focus_restore_pending.as_ref();
        let ghost_cursor = helper_focus_restore_pending
            .and_then(|_| state.helper_ghost_cursor_for_segment(&row_view.segment_id))
            .map(view_helper_ghost_cursor);
        let transcript_text_input = if let Some(ghost_cursor) = ghost_cursor {
            stack![
                transcript_text_input,
                ghost_cursor,
                transcript_text_wheel_overlay(),
            ]
            .width(Fill)
        } else {
            stack![transcript_text_input, transcript_text_wheel_overlay(),].width(Fill)
        };
        let mut transcript_text = column![transcript_text_input].spacing(3).width(Fill);
        if !tag_suggestions.is_empty() {
            let highlighted_index = state.tag_suggestion_highlighted_index(
                &row_view.segment_id,
                draft,
                tag_suggestions.len(),
            );
            transcript_text = transcript_text.push(view_tag_suggestions(
                row_view.segment_id.clone(),
                tag_suggestions,
                highlighted_index,
            ));
        }
        transcript_text
    };
    if let Some(diff) = row_view.diff.as_ref() {
        transcript_text = transcript_text.push(view_diff_cell(diff.clone()));
    }

    let (lint_severity, lint_message) = state.draft_lint_status(row_view, draft);
    let row_is_hovered =
        state.transcript_row_hover_segment_id.as_deref() == Some(row_view.segment_id.as_str());
    let row_is_selected =
        editor.selected_segment_id.as_deref() == Some(row_view.segment_id.as_str());
    let row_is_active = row_view.is_active || row_view.is_selected || row_is_selected;
    let row_is_playing = editor.is_playing && row_is_active;
    let track_picker = view_speaker_label(row_view, draft);
    let speaker_column = if row_view.annotation_kind.is_transcription() {
        column![track_picker]
    } else {
        let mut annotation_column = column![
            track_picker,
            text(row_view.annotation_type_label.clone())
                .size(11)
                .color(Color::from_rgb8(75, 85, 99)),
        ];
        if let Some(annotation_diff) = &row_view.annotation_diff {
            annotation_column = annotation_column.push(
                text(annotation_diff_status_label(&annotation_diff.status))
                    .size(11)
                    .color(annotation_diff_status_color(&annotation_diff.status)),
            );
        }
        annotation_column
    };
    let mut row_content = Column::new().spacing(4);
    let mut main_row = Row::new().spacing(0);
    if row_view.visible_cells.id {
        main_row = main_row.push(view_lazy_id_cell(IdCellDependency {
            display_id: row_view.display_id.clone(),
        }));
    }
    if row_view.visible_cells.track {
        main_row = main_row.push(table_cell(
            speaker_column.width(Fill),
            Length::Fixed(super::SPEAKER_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.start {
        main_row = main_row.push(table_cell(
            start_input,
            Length::Fixed(super::TIME_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.end {
        main_row = main_row.push(table_cell(
            end_input,
            Length::Fixed(super::TIME_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.reviewer {
        main_row = main_row.push(table_cell(
            view_reviewer_cell(row_view.reviewer.clone()),
            Length::Fixed(super::REVIEWER_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.text {
        main_row = main_row.push(table_cell(transcript_text, Fill));
    }
    if row_view.visible_cells.linter {
        main_row = main_row.push(super::lint::view_lint(
            row_view,
            lint_severity,
            lint_message,
        ));
    }
    let action_cells = if row_view.visible_cells.actions && !row_view.is_read_only {
        Some((
            super::actions::view_action_cell(
                project,
                editor,
                row_view,
                editor.open_actions_segment_id.as_deref() == Some(row_view.segment_id.as_str()),
            ),
            super::view_scrollbar_gutter(),
        ))
    } else {
        None
    };

    row_content = row_content.push(main_row);

    let selectable_segment_id = row_view.segment_id.clone();
    let selectable_body = mouse_area(container(row_content).width(Fill))
        .on_press(Message::TranscriptRowBodyPressed(
            selectable_segment_id.clone(),
        ))
        .on_enter(Message::TranscriptRowHovered(selectable_segment_id.clone()))
        .on_exit(Message::TranscriptRowExited(selectable_segment_id))
        .interaction(mouse::Interaction::Pointer);

    let mut full_row = Row::new().spacing(0).push(selectable_body);
    if let Some((action_cell, scrollbar_gutter)) = action_cells {
        full_row = full_row.push(action_cell).push(scrollbar_gutter);
    }

    container(full_row)
        .width(Fill)
        .style(transcript_row_style(
            row_is_active,
            row_is_playing,
            row_is_hovered,
            is_find_match,
        ))
        .into()
}

pub(super) fn view_modified_transcript_row(
    row_view: &TranscriptRowView,
) -> Element<'static, Message> {
    let is_added = row_view.row_kind == TranscriptRowKind::ModifiedAdded;
    let mut main_row = Row::new().spacing(0);
    if row_view.visible_cells.id {
        main_row = main_row.push(view_lazy_id_cell(IdCellDependency {
            display_id: row_view.display_id.clone(),
        }));
    }
    if row_view.visible_cells.track {
        main_row = main_row.push(table_cell(
            text(row_view.speaker_label.clone())
                .size(12)
                .color(if is_added {
                    Color::from_rgb8(22, 101, 52)
                } else {
                    Color::from_rgb8(185, 28, 28)
                })
                .width(Fill),
            Length::Fixed(super::SPEAKER_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.start {
        main_row = main_row.push(table_cell(
            view_read_only_time_cell(row_view.start_time.clone()),
            Length::Fixed(super::TIME_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.end {
        main_row = main_row.push(table_cell(
            view_read_only_time_cell(row_view.end_time.clone()),
            Length::Fixed(super::TIME_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.reviewer {
        main_row = main_row.push(table_cell(
            view_reviewer_cell(row_view.reviewer.clone()),
            Length::Fixed(super::REVIEWER_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.text {
        let content: Element<'static, Message> = if let Some(diff) = row_view.diff.clone() {
            view_diff_cell(diff)
        } else {
            text(row_view.text.clone())
                .size(12)
                .color(Color::from_rgb8(75, 85, 99))
                .width(Fill)
                .into()
        };
        main_row = main_row.push(table_cell(content, Fill));
    }
    if row_view.visible_cells.linter {
        main_row = main_row.push(table_cell(
            text("-").size(12).width(Fill),
            Length::Fixed(super::LINTER_COLUMN_WIDTH),
        ));
    }
    if row_view.visible_cells.actions {
        main_row = main_row.push(table_cell(
            text("-").size(12).width(Fill),
            Length::Fixed(super::ACTION_COLUMN_WIDTH),
        ));
        main_row = main_row.push(super::view_scrollbar_gutter());
    }

    container(Column::new().spacing(4).push(main_row))
        .width(Fill)
        .style(transcript_modified_row_style(is_added))
        .into()
}

fn table_cell<'a>(
    content: impl Into<Element<'a, Message>>,
    width: impl Into<Length>,
) -> Element<'a, Message> {
    container(content).padding([1, 8]).width(width).into()
}

fn view_helper_ghost_cursor<'a>(ghost: HelperGhostCursorView) -> Element<'a, Message> {
    let transparent = Color::from_rgba8(0, 0, 0, 0.0);
    let helper_ghost_cursor_color = Color::from_rgba8(245, 158, 11, 0.75);

    container(
        rich_text([
            span(ghost.before_cursor).color(transparent),
            span("\u{200a}")
                .color(transparent)
                .background(Background::Color(helper_ghost_cursor_color)),
            span(ghost.after_cursor).color(transparent),
        ])
        .size(12)
        .width(Fill),
    )
    .padding([2, 4])
    .width(Fill)
    .into()
}

fn id_table_cell<'a>(
    content: impl Into<Element<'a, Message>>,
    width: impl Into<Length>,
) -> Element<'a, Message> {
    container(content).padding([5, 8]).width(width).into()
}

fn view_lazy_id_cell(dependency: IdCellDependency) -> Element<'static, Message> {
    lazy(dependency, view_id_cell_content).into()
}

fn view_id_cell_content(dependency: &IdCellDependency) -> Element<'static, Message> {
    id_table_cell(
        text(dependency.display_id.clone()).size(12).width(Fill),
        Length::Fixed(super::ID_COLUMN_WIDTH),
    )
}

fn view_read_only_time_cell<'a>(value: String) -> Element<'a, Message> {
    text(value).size(12).width(Fill).into()
}

fn view_speaker_label<'a>(
    row_view: &TranscriptRowView,
    draft: &SegmentDraft,
) -> Element<'a, Message> {
    if draft.track_edit.is_editing {
        let selected = row_view
            .track_options
            .iter()
            .find(|option| option.lane_id == draft.track_edit.draft_lane_id)
            .cloned()
            .or_else(|| row_view.selected_track_option.clone());
        let change_segment_id = row_view.segment_id.clone();
        let open_segment_id = row_view.segment_id.clone();
        let close_segment_id = row_view.segment_id.clone();
        return pick_list(row_view.track_options.clone(), selected, move |option| {
            Message::SegmentTrackChanged(change_segment_id.clone(), option.lane_id)
        })
        .on_open(Message::SegmentTrackSelectOpenChanged(
            open_segment_id,
            true,
        ))
        .on_close(Message::SegmentTrackSelectOpenChanged(
            close_segment_id,
            false,
        ))
        .width(Fill)
        .into();
    }

    view_lazy_speaker_label(SpeakerLabelDependency {
        segment_id: row_view.segment_id.clone(),
        speaker_label: row_view.speaker_label.clone(),
        lane_id: row_view.lane_id.clone(),
    })
}

fn view_lazy_speaker_label(dependency: SpeakerLabelDependency) -> Element<'static, Message> {
    lazy(dependency, view_speaker_label_content).into()
}

fn view_speaker_label_content(dependency: &SpeakerLabelDependency) -> Element<'static, Message> {
    mouse_area(
        text(dependency.speaker_label.clone())
            .size(12)
            .color(speaker_label_color_for(
                &dependency.speaker_label,
                &dependency.lane_id,
            ))
            .width(Fill),
    )
    .on_press(Message::SegmentTrackCellPressed(
        dependency.segment_id.clone(),
    ))
    .interaction(mouse::Interaction::Pointer)
    .into()
}

fn speaker_label_color_for(speaker_label: &str, lane_id: &str) -> Color {
    if speaker_label.contains('1') || lane_id.contains('1') {
        Color::from_rgb8(134, 63, 255)
    } else {
        Color::from_rgb8(32, 154, 255)
    }
}

fn annotation_diff_status_label(status: &str) -> &'static str {
    match status {
        "matched" => "Unchanged",
        "label-mismatch" => "Label changed",
        "subcategory-mismatch" => "Subcategory changed",
        "intensity-mismatch" => "Intensity changed",
        "unmatched-prediction" => "Added",
        "unmatched-reference" => "Removed",
        _ => "Changed",
    }
}

fn annotation_diff_status_color(status: &str) -> Color {
    match status {
        "label-mismatch" | "subcategory-mismatch" | "intensity-mismatch" => {
            Color::from_rgb8(180, 105, 25)
        }
        "unmatched-prediction" => Color::from_rgb8(31, 140, 80),
        "unmatched-reference" => Color::from_rgb8(185, 28, 28),
        _ => Color::from_rgb8(75, 85, 99),
    }
}

fn view_time_cell<'a>(
    segment_id: String,
    field: TimeField,
    draft: &'a SegmentDraft,
) -> Element<'a, Message> {
    let input_id = segment_id.clone();
    let submit_id = segment_id;
    let input = text_input("mm:ss.xx", draft.time_value(field))
        .on_input(move |value| Message::SegmentTimeChanged(input_id.clone(), field, value))
        .on_submit(Message::CommitSegmentTimeEdit {
            segment_id: submit_id,
            field,
            from_blur: false,
        })
        .padding(0)
        .size(12)
        .style(table_plain_text_input_style)
        .width(Fill);

    let mut cell = column![input].spacing(2).width(Fill);
    if let Some(error) = draft.time_error(field) {
        cell = cell.push(text(error).size(10).color(Color::from_rgb8(185, 28, 28)));
    }

    cell.into()
}

fn view_reviewer_cell<'a>(reviewer: Option<TranscriptReviewerView>) -> Element<'a, Message> {
    match reviewer {
        Some(reviewer) => {
            let mut label = text(reviewer.label).size(12).width(Fill);
            if let Some(color) = hsl_color(&reviewer.color) {
                label = label.color(color);
            }
            label.into()
        }
        None => text("").width(Fill).into(),
    }
}

fn view_tag_suggestions<'a>(
    segment_id: String,
    suggestions: Vec<TagSuggestion>,
    highlighted_index: usize,
) -> Element<'a, Message> {
    let mut items = Column::new().spacing(1);
    for (index, suggestion) in suggestions.into_iter().enumerate() {
        let label = text(suggestion.label.clone()).size(11);
        items = items.push(
            button(label)
                .style(tag_suggestion_button_style(index == highlighted_index))
                .on_press(Message::ApplyTagSuggestion(segment_id.clone(), suggestion))
                .width(Fill),
        );
    }

    container(scrollable(items).height(Length::Fixed(140.0)))
        .width(Length::Fixed(180.0))
        .into()
}

fn transcript_text_wheel_overlay<'a>() -> Element<'a, Message> {
    mouse_area(container(text("")).width(Fill).height(Fill))
        .on_scroll(|delta| Message::TranscriptTextWheelScrolled {
            delta_y: transcript_scroll_delta_y(delta),
        })
        .into()
}

fn transcript_scroll_delta_y(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => -y * 60.0,
        mouse::ScrollDelta::Pixels { y, .. } => -y,
    }
}

fn view_diff_cell<'a>(diff: TranscriptDiffView) -> Element<'a, Message> {
    row![view_diff_badge(&diff), view_diff_word_chips(diff.word_diff),]
        .spacing(6)
        .width(Fill)
        .into()
}

fn view_diff_badge<'a>(diff: &TranscriptDiffView) -> Element<'a, Message> {
    let label = diff.badge_label.clone();
    let style = match diff.kind {
        TranscriptDiffCellKind::ModifiedAdded => diff_added_badge_style,
        TranscriptDiffCellKind::ModifiedDeleted => diff_deleted_badge_style,
        TranscriptDiffCellKind::New => diff_new_badge_style,
        TranscriptDiffCellKind::Diff if label == "No changes" => diff_no_change_badge_style,
        TranscriptDiffCellKind::Diff => diff_change_badge_style,
    };

    container(text(label).size(11))
        .padding([2, 6])
        .width(Length::Fixed(76.0))
        .style(style)
        .into()
}

fn view_diff_word_chips<'a>(word_diff: TranscriptWordDiffView) -> Element<'a, Message> {
    match word_diff.kind {
        TranscriptWordDiffKind::Empty | TranscriptWordDiffKind::Plain => {
            text(word_diff.text.unwrap_or_default())
                .size(11)
                .width(Fill)
                .color(Color::from_rgb8(108, 117, 125))
                .into()
        }
        TranscriptWordDiffKind::Tokens => {
            let mut chips = Row::new().spacing(3).width(Fill);
            for token in word_diff.tokens {
                chips = chips.push(view_diff_word_chip(token));
            }

            container(chips.wrap()).width(Fill).into()
        }
    }
}

fn view_diff_word_chip<'a>(token: TranscriptWordDiffTokenView) -> Element<'a, Message> {
    let text_color = word_diff_token_color(token.status);
    let label = rich_text([span(token.value)
        .color(text_color)
        .strikethrough(token.status == TranscriptWordDiffStatus::Removed)])
    .size(11);

    match token.status {
        TranscriptWordDiffStatus::Added => container(label)
            .padding([1, 3])
            .style(diff_word_added_chip_style)
            .into(),
        TranscriptWordDiffStatus::Removed => container(label)
            .padding([1, 3])
            .style(diff_word_removed_chip_style)
            .into(),
        TranscriptWordDiffStatus::Unchanged | TranscriptWordDiffStatus::Unknown => label.into(),
    }
}

fn view_low_confidence_transcript_text<'a>(
    segment_id: String,
    text_content: String,
    low_confidence_word_tokens: Vec<LowConfidenceWordToken>,
    acknowledged_words: BTreeSet<usize>,
    is_read_only: bool,
) -> Element<'a, Message> {
    let mut words = Row::new().spacing(4).width(Fill).align_y(Alignment::End);
    let low_confidence_words = low_confidence_word_tokens
        .into_iter()
        .map(|word| (word.index, word))
        .collect::<std::collections::BTreeMap<_, _>>();

    for (token_index, token) in text_content.split_whitespace().enumerate() {
        if let Some(word) = low_confidence_words.get(&token_index) {
            words = words.push(view_low_confidence_word_token(
                segment_id.clone(),
                word,
                acknowledged_words.contains(&word.index),
                is_read_only,
            ));
        } else {
            words = words.push(text(token.to_string()).size(12));
        }
    }

    container(words.wrap())
        .padding([2, 12])
        .width(Fill)
        .style(low_confidence_text_shell_style)
        .into()
}

fn low_confidence_word_tokens(word_views: &[TranscriptWordView]) -> Vec<LowConfidenceWordToken> {
    word_views
        .iter()
        .filter(|word| word.is_low_confidence)
        .map(|word| LowConfidenceWordToken {
            index: word.index,
            text: word.text.clone(),
            start_seconds: word.start_seconds,
        })
        .collect()
}

fn view_low_confidence_word_token<'a>(
    segment_id: String,
    word: &LowConfidenceWordToken,
    is_acknowledged: bool,
    is_read_only: bool,
) -> Element<'a, Message> {
    if is_acknowledged || is_read_only {
        return view_plain_low_confidence_word_token(word);
    }

    let word_text = container(text(word.text.clone()).size(12))
        .padding([0, 1])
        .style(low_confidence_word_dotted_style);

    let word_text: Element<'a, Message> = if let Some(start_seconds) = word.start_seconds {
        button(word_text)
            .style(super::super::super::styles::table_plain_button_style)
            .padding(0)
            .on_press(Message::SeekToTime(start_seconds))
            .into()
    } else {
        word_text.into()
    };

    stack![
        column![
            Space::with_height(Length::Fixed(LOW_CONFIDENCE_ACK_BUTTON_SIZE)),
            word_text,
        ]
        .spacing(0),
        row![
            Space::with_width(Fill),
            view_low_confidence_ack_button(segment_id, word.index),
            Space::with_width(Fill),
        ]
        .width(Fill),
    ]
    .into()
}

fn view_plain_low_confidence_word_token<'a>(word: &LowConfidenceWordToken) -> Element<'a, Message> {
    text(word.text.clone()).size(12).into()
}

fn view_low_confidence_ack_button<'a>(
    segment_id: String,
    word_index: usize,
) -> Element<'a, Message> {
    let icon = Svg::new(LOW_CONFIDENCE_ACK_ICON_HANDLE.clone())
        .width(Length::Fixed(LOW_CONFIDENCE_ACK_ICON_SIZE))
        .height(Length::Fixed(LOW_CONFIDENCE_ACK_ICON_SIZE))
        .style(|_, _| svg::Style {
            color: Some(Color::WHITE),
        });

    button(icon)
        .padding(0)
        .width(Length::Fixed(LOW_CONFIDENCE_ACK_BUTTON_SIZE))
        .height(Length::Fixed(LOW_CONFIDENCE_ACK_BUTTON_SIZE))
        .style(low_confidence_ack_button_style)
        .on_press(Message::AcknowledgeLowConfidenceWord(
            segment_id, word_index,
        ))
        .into()
}

fn word_diff_token_color(status: TranscriptWordDiffStatus) -> Color {
    match status {
        TranscriptWordDiffStatus::Unchanged => Color::from_rgb8(22, 26, 31),
        TranscriptWordDiffStatus::Added => Color::from_rgb8(21, 128, 61),
        TranscriptWordDiffStatus::Removed => Color::from_rgb8(185, 28, 28),
        TranscriptWordDiffStatus::Unknown => Color::from_rgb8(75, 85, 99),
    }
}

fn hsl_color(token: &str) -> Option<Color> {
    let body = token.strip_prefix("hsl(")?.strip_suffix(')')?;
    let parts = body.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }

    let hue = parts[0].parse::<f32>().ok()?;
    let saturation = parts[1].strip_suffix('%')?.parse::<f32>().ok()? / 100.0;
    let lightness = parts[2].strip_suffix('%')?.parse::<f32>().ok()? / 100.0;
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let hue_prime = (hue.rem_euclid(360.0)) / 60.0;
    let x = chroma * (1.0 - (hue_prime % 2.0 - 1.0).abs());
    let (red, green, blue) = match hue_prime {
        value if (0.0..1.0).contains(&value) => (chroma, x, 0.0),
        value if (1.0..2.0).contains(&value) => (x, chroma, 0.0),
        value if (2.0..3.0).contains(&value) => (0.0, chroma, x),
        value if (3.0..4.0).contains(&value) => (0.0, x, chroma),
        value if (4.0..5.0).contains(&value) => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let match_value = lightness - chroma / 2.0;

    Some(Color::from_rgb(
        red + match_value,
        green + match_value,
        blue + match_value,
    ))
}

#[cfg(test)]
fn babel_transcript_key_binding(
    segment_id: &str,
    tag_suggestions_visible: bool,
    event: iced::widget::text_editor::KeyPress,
) -> Option<iced::widget::text_editor::Binding<Message>> {
    babel_transcript_key_binding_with_selection(segment_id, tag_suggestions_visible, false, event)
}

fn babel_transcript_key_binding_with_selection(
    segment_id: &str,
    tag_suggestions_visible: bool,
    has_text_selection: bool,
    event: iced::widget::text_editor::KeyPress,
) -> Option<iced::widget::text_editor::Binding<Message>> {
    if event.status != iced::widget::text_editor::Status::Focused {
        return iced::widget::text_editor::Binding::from_key_press(event);
    }

    if tag_suggestions_visible {
        match event.key.as_ref() {
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
                return Some(iced::widget::text_editor::Binding::Custom(
                    Message::TranscriptTagSuggestionKeyPressed(
                        segment_id.to_string(),
                        TagSuggestionKeyboardAction::Next,
                    ),
                ));
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
                return Some(iced::widget::text_editor::Binding::Custom(
                    Message::TranscriptTagSuggestionKeyPressed(
                        segment_id.to_string(),
                        TagSuggestionKeyboardAction::Previous,
                    ),
                ));
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter)
            | iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab) => {
                return Some(iced::widget::text_editor::Binding::Custom(
                    Message::TranscriptTagSuggestionKeyPressed(
                        segment_id.to_string(),
                        TagSuggestionKeyboardAction::Accept,
                    ),
                ));
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                return Some(iced::widget::text_editor::Binding::Custom(
                    Message::TranscriptTagSuggestionKeyPressed(
                        segment_id.to_string(),
                        TagSuggestionKeyboardAction::Dismiss,
                    ),
                ));
            }
            _ => {}
        }
    }

    if event.modifiers.command() && !event.modifiers.alt() {
        if transcript_key_character_is(&event.key, "a", "\u{1}") {
            return Some(iced::widget::text_editor::Binding::SelectAll);
        }
        if transcript_key_character_is(&event.key, "c", "\u{3}") {
            return Some(iced::widget::text_editor::Binding::Copy);
        }
        if transcript_key_character_is(&event.key, "x", "\u{18}") {
            return Some(iced::widget::text_editor::Binding::Cut);
        }
        if transcript_key_character_is(&event.key, "v", "\u{16}") {
            return Some(iced::widget::text_editor::Binding::Paste);
        }
        if transcript_key_character_is(&event.key, "z", "\u{1a}") {
            return Some(iced::widget::text_editor::Binding::Custom(
                if event.modifiers.shift() {
                    Message::SegmentTextRedo(segment_id.to_string())
                } else {
                    Message::SegmentTextUndo(segment_id.to_string())
                },
            ));
        }
        if !event.modifiers.shift() && transcript_key_character_is(&event.key, "y", "\u{19}") {
            return Some(iced::widget::text_editor::Binding::Custom(
                Message::SegmentTextRedo(segment_id.to_string()),
            ));
        }
    }

    if transcript_should_swallow_modified_text_input(&event) {
        return Some(iced::widget::text_editor::Binding::Sequence(vec![]));
    }

    if has_text_selection
        && !event.modifiers.command()
        && !event.modifiers.control()
        && !event.modifiers.alt()
        && !event.modifiers.logo()
        && !event.modifiers.shift()
        && let Some(digit) = transcript_digit_key(&event.key)
    {
        return Some(iced::widget::text_editor::Binding::Custom(
            Message::ConvertSelectionToSkazWithDigit(segment_id.to_string(), digit),
        ));
    }

    match event.key.as_ref() {
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Copy) => {
            return Some(iced::widget::text_editor::Binding::Copy);
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Cut) => {
            return Some(iced::widget::text_editor::Binding::Cut);
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Paste) => {
            return Some(iced::widget::text_editor::Binding::Paste);
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Undo) => {
            return Some(iced::widget::text_editor::Binding::Custom(
                Message::SegmentTextUndo(segment_id.to_string()),
            ));
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Redo) => {
            return Some(iced::widget::text_editor::Binding::Custom(
                Message::SegmentTextRedo(segment_id.to_string()),
            ));
        }
        _ => {}
    }

    if matches!(
        event.key.as_ref(),
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
    ) && !event.modifiers.command()
        && !event.modifiers.control()
        && !event.modifiers.alt()
        && !event.modifiers.logo()
        && !event.modifiers.shift()
    {
        return Some(iced::widget::text_editor::Binding::Sequence(vec![
            iced::widget::text_editor::Binding::Unfocus,
            iced::widget::text_editor::Binding::Custom(Message::TranscriptEditorEscapePressed(
                segment_id.to_string(),
            )),
        ]));
    }

    if matches!(
        event.key.as_ref(),
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter)
    ) {
        return Some(iced::widget::text_editor::Binding::Unfocus);
    }

    if matches!(
        event.key.as_ref(),
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab)
    ) && !event.modifiers.command()
        && !event.modifiers.control()
        && !event.modifiers.alt()
        && !event.modifiers.logo()
    {
        return Some(iced::widget::text_editor::Binding::Custom(
            if event.modifiers.shift() {
                Message::FocusPrevious
            } else {
                Message::FocusNext
            },
        ));
    }

    iced::widget::text_editor::Binding::from_key_press(event)
}

fn transcript_digit_key(key: &iced::keyboard::Key) -> Option<char> {
    match key.as_ref() {
        iced::keyboard::Key::Character(value) if value.len() == 1 => {
            value.chars().next().filter(char::is_ascii_digit)
        }
        _ => None,
    }
}

fn transcript_key_character_is(
    key: &iced::keyboard::Key,
    expected: &str,
    control_character: &str,
) -> bool {
    match key.as_ref() {
        iced::keyboard::Key::Character(value) => {
            value.eq_ignore_ascii_case(expected) || value == control_character
        }
        _ => false,
    }
}

fn transcript_should_swallow_modified_text_input(
    event: &iced::widget::text_editor::KeyPress,
) -> bool {
    if !event.text.as_ref().is_some_and(|text| {
        text.chars()
            .any(|character| !character.is_control() && !character.is_whitespace())
    }) {
        return false;
    }

    event.modifiers.alt()
        || event.modifiers.control()
        || event.modifiers.logo()
        || event.modifiers.command()
}

#[cfg(test)]
mod tests {
    use iced::keyboard::{Key, Modifiers, key::Named};
    use iced::mouse;
    use iced::widget::text_editor::{Binding, KeyPress, Status};

    use super::{Message, TagSuggestionKeyboardAction};

    fn key_press(key: Key, modifiers: Modifiers) -> KeyPress {
        key_press_with_status(key, modifiers, Status::Focused)
    }

    fn key_press_with_status(key: Key, modifiers: Modifiers, status: Status) -> KeyPress {
        KeyPress {
            key,
            modifiers,
            text: None,
            status,
        }
    }

    fn key_press_with_text(key: Key, modifiers: Modifiers, text: &str) -> KeyPress {
        KeyPress {
            key,
            modifiers,
            text: Some(text.into()),
            status: Status::Focused,
        }
    }

    #[test]
    fn transcript_text_wheel_delta_matches_outer_scrollable_direction() {
        assert_eq!(
            super::transcript_scroll_delta_y(mouse::ScrollDelta::Lines { x: 0.0, y: 2.0 }),
            -120.0
        );
        assert_eq!(
            super::transcript_scroll_delta_y(mouse::ScrollDelta::Pixels { x: 0.0, y: -42.0 }),
            42.0
        );
    }

    #[test]
    fn babel_transcript_editor_enter_blurs_instead_of_inserting_newline() {
        let binding = super::babel_transcript_key_binding(
            "annotation-1",
            false,
            key_press(Key::Named(Named::Enter), Modifiers::empty()),
        );

        assert_eq!(binding, Some(Binding::Unfocus));
    }

    #[test]
    fn babel_transcript_editor_escape_unfocuses_and_runs_helper_focus_workflow() {
        let binding = super::babel_transcript_key_binding(
            "annotation-1",
            false,
            key_press(Key::Named(Named::Escape), Modifiers::empty()),
        );

        assert_eq!(
            binding,
            Some(Binding::Sequence(vec![
                Binding::Unfocus,
                Binding::Custom(Message::TranscriptEditorEscapePressed(
                    "annotation-1".to_string()
                )),
            ]))
        );
    }

    #[test]
    fn babel_transcript_editor_keeps_browser_edit_chords() {
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("a".into()), Modifiers::COMMAND),
            ),
            Some(Binding::SelectAll)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("c".into()), Modifiers::COMMAND),
            ),
            Some(Binding::Copy)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("x".into()), Modifiers::COMMAND),
            ),
            Some(Binding::Cut)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("v".into()), Modifiers::COMMAND),
            ),
            Some(Binding::Paste)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("\u{1}".into()), Modifiers::COMMAND),
            ),
            Some(Binding::SelectAll)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("\u{3}".into()), Modifiers::COMMAND),
            ),
            Some(Binding::Copy)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("\u{18}".into()), Modifiers::COMMAND),
            ),
            Some(Binding::Cut)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("\u{16}".into()), Modifiers::COMMAND),
            ),
            Some(Binding::Paste)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Copy), Modifiers::empty()),
            ),
            Some(Binding::Copy)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Cut), Modifiers::empty()),
            ),
            Some(Binding::Cut)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Paste), Modifiers::empty()),
            ),
            Some(Binding::Paste)
        );
    }

    #[test]
    fn babel_transcript_editor_digit_over_selection_routes_skaz_conversion_only_with_selection() {
        assert_eq!(
            super::babel_transcript_key_binding_with_selection(
                "annotation-1",
                false,
                true,
                key_press(Key::Character("5".into()), Modifiers::empty()),
            ),
            Some(Binding::Custom(Message::ConvertSelectionToSkazWithDigit(
                "annotation-1".to_string(),
                '5'
            )))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("5".into()), Modifiers::empty()),
            ),
            Binding::from_key_press(key_press(Key::Character("5".into()), Modifiers::empty()))
        );
    }

    #[test]
    fn babel_transcript_editor_swallows_modified_hotkey_text_while_focused() {
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press_with_text(Key::Character("x".into()), Modifiers::ALT, "x"),
            ),
            Some(Binding::Sequence(vec![]))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press_with_text(Key::Character("1".into()), Modifiers::ALT, "1"),
            ),
            Some(Binding::Sequence(vec![]))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press_with_text(Key::Character("f".into()), Modifiers::COMMAND, "f"),
            ),
            Some(Binding::Sequence(vec![]))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press_with_text(Key::Character("A".into()), Modifiers::SHIFT, "A"),
            ),
            Some(Binding::Insert('A'))
        );
    }

    #[test]
    fn babel_transcript_editor_does_not_capture_browser_edit_keys_when_unfocused() {
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press_with_status(
                    Key::Character("c".into()),
                    Modifiers::COMMAND,
                    Status::Active
                ),
            ),
            None
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press_with_status(Key::Named(Named::Undo), Modifiers::empty(), Status::Active),
            ),
            None
        );
    }

    #[test]
    fn babel_transcript_editor_routes_undo_redo_to_local_draft_history() {
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("z".into()), Modifiers::COMMAND),
            ),
            Some(Binding::Custom(Message::SegmentTextUndo(
                "annotation-1".to_string()
            )))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(
                    Key::Character("z".into()),
                    Modifiers::COMMAND | Modifiers::SHIFT
                ),
            ),
            Some(Binding::Custom(Message::SegmentTextRedo(
                "annotation-1".to_string()
            )))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Character("y".into()), Modifiers::COMMAND),
            ),
            Some(Binding::Custom(Message::SegmentTextRedo(
                "annotation-1".to_string()
            )))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Undo), Modifiers::empty()),
            ),
            Some(Binding::Custom(Message::SegmentTextUndo(
                "annotation-1".to_string()
            )))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Redo), Modifiers::empty()),
            ),
            Some(Binding::Custom(Message::SegmentTextRedo(
                "annotation-1".to_string()
            )))
        );
    }

    #[test]
    fn babel_transcript_editor_tabs_through_focus_like_browser_forms() {
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Tab), Modifiers::empty()),
            ),
            Some(Binding::Custom(Message::FocusNext))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Tab), Modifiers::SHIFT),
            ),
            Some(Binding::Custom(Message::FocusPrevious))
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Tab), Modifiers::COMMAND),
            ),
            None
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                false,
                key_press(Key::Named(Named::Tab), Modifiers::ALT),
            ),
            None
        );
    }

    #[test]
    fn babel_transcript_editor_routes_tag_suggestion_keys_like_recovered_babel() {
        let custom = |action| {
            Some(Binding::Custom(Message::TranscriptTagSuggestionKeyPressed(
                "annotation-1".to_string(),
                action,
            )))
        };

        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                true,
                key_press(Key::Named(Named::ArrowDown), Modifiers::empty()),
            ),
            custom(TagSuggestionKeyboardAction::Next)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                true,
                key_press(Key::Named(Named::ArrowUp), Modifiers::empty()),
            ),
            custom(TagSuggestionKeyboardAction::Previous)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                true,
                key_press(Key::Named(Named::Enter), Modifiers::empty()),
            ),
            custom(TagSuggestionKeyboardAction::Accept)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                true,
                key_press(Key::Named(Named::Tab), Modifiers::empty()),
            ),
            custom(TagSuggestionKeyboardAction::Accept)
        );
        assert_eq!(
            super::babel_transcript_key_binding(
                "annotation-1",
                true,
                key_press(Key::Named(Named::Escape), Modifiers::empty()),
            ),
            custom(TagSuggestionKeyboardAction::Dismiss)
        );
    }

    #[test]
    fn parses_recovered_babel_reviewer_hsl_color() {
        let color = super::hsl_color("hsl(49, 70%, 30%)").expect("valid hsl");

        assert!((color.r - 0.51).abs() < 0.01);
        assert!((color.g - 0.44).abs() < 0.01);
        assert!((color.b - 0.09).abs() < 0.01);
    }
}
