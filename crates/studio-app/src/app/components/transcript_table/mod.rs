mod actions;
mod header;
mod lint;
mod row;

use std::collections::BTreeSet;

use iced::widget::{Column, Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Fill, Length};
use studio_app::{EditorSession, TranscriptEmptyStateView, TranscriptRowKind, TranscriptRowView};
use studio_core::Project;

use super::super::find;
use super::super::styles::{
    babel_transcript_card_style, outline_button_style, table_plain_text_input_style,
};
use super::super::{Message, StudioState};
use super::transport_strip;

const ID_COLUMN_WIDTH: f32 = 48.0;
const SPEAKER_COLUMN_WIDTH: f32 = 96.0;
const TIME_COLUMN_WIDTH: f32 = 96.0;
const ACTION_COLUMN_WIDTH: f32 = 64.0;
const LINTER_COLUMN_WIDTH: f32 = 56.0;
const REVIEWER_COLUMN_WIDTH: f32 = 144.0;
const SCROLLBAR_GUTTER_WIDTH: f32 = 16.0;

pub(super) fn view_transcript_workspace<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let row_views = state.transcript_rows_snapshot_for_render(project, editor);
    let transcript_layout = state.transcript_layout_for_render(&row_views, editor);
    let virtual_window = transcript_layout.virtual_window;
    let visible_cells = editor.transcript_visible_cells(project);
    let find_match_segment_ids = if state.find_bar_open {
        state
            .find_match_segment_ids(project, editor)
            .into_iter()
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    let mut rows = Column::new().spacing(0);

    if row_views.is_empty() {
        rows = rows.push(view_empty_state(editor.transcript_empty_state(project)));
    }

    if !row_views.is_empty() {
        if let Some(pinned) = virtual_window.pinned_row_before_window() {
            let row = view_any_transcript_row(
                state,
                project,
                editor,
                &find_match_segment_ids,
                &row_views[pinned.index],
            );
            rows = push_pinned_row_island(rows, pinned, row);
        } else if virtual_window.before_height > 0.0 {
            rows = rows.push(Space::with_height(Length::Fixed(
                virtual_window.before_height,
            )));
        }

        for row_view in row_views
            .iter()
            .skip(virtual_window.start_index)
            .take(virtual_window.rendered_len())
        {
            if let Some(row) =
                view_any_transcript_row(state, project, editor, &find_match_segment_ids, row_view)
            {
                rows = rows.push(row);
            }
        }

        if let Some(pinned) = virtual_window.pinned_row_after_window() {
            let row = view_any_transcript_row(
                state,
                project,
                editor,
                &find_match_segment_ids,
                &row_views[pinned.index],
            );
            rows = push_pinned_row_island(rows, pinned, row);
        } else if virtual_window.after_height > 0.0 {
            rows = rows.push(Space::with_height(Length::Fixed(
                virtual_window.after_height,
            )));
        }
    }

    let mut table_content = Column::new().spacing(0);
    if state.find_bar_open {
        table_content = table_content.push(view_find_bar(state, project, editor));
    }
    let table = table_content
        .push(header::view_table_header(visible_cells))
        .push(
            scrollable(rows)
                .id(find::transcript_scroll_id())
                .on_scroll(|viewport| {
                    let offset = viewport.absolute_offset();
                    let bounds = viewport.bounds();
                    Message::TranscriptScrolled {
                        offset_y: offset.y,
                        viewport_height: bounds.height,
                    }
                })
                .height(Fill),
        )
        .width(Fill)
        .height(Fill);
    let body: Element<'a, Message> = table.into();

    container(
        column![
            transport_strip::view_transport_strip(state, project, editor),
            body
        ]
        .spacing(8)
        .height(Fill),
    )
    .padding(12)
    .width(Fill)
    .height(Fill)
    .style(babel_transcript_card_style)
    .into()
}

fn view_empty_state<'a>(empty_state: TranscriptEmptyStateView) -> Element<'a, Message> {
    let _recovered_col_span = empty_state.col_span;

    container(
        column![
            text(empty_state.title).size(16),
            text(empty_state.description).size(12).width(Fill),
        ]
        .spacing(8)
        .align_x(Alignment::Center),
    )
    .padding([24, 8])
    .width(Fill)
    .into()
}

fn view_find_bar<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let summary = state
        .find_match_summary(project, editor)
        .unwrap_or_else(|| "Type to find".to_string());

    container(
        row![
            text("Find").size(12),
            text_input("Find in transcript", &state.find_query)
                .id(find::find_input_id())
                .on_input(Message::FindQueryChanged)
                .on_submit(Message::FindNext)
                .padding([4, 8])
                .size(12)
                .style(table_plain_text_input_style)
                .width(220.0),
            text(summary).size(12),
            Space::with_width(Fill),
            button(text("Previous").size(12))
                .style(outline_button_style)
                .on_press(Message::FindPrevious),
            button(text("Next").size(12))
                .style(outline_button_style)
                .on_press(Message::FindNext),
            button(text("Close").size(12))
                .style(outline_button_style)
                .on_press(Message::CloseFind),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    )
    .padding([0, 0])
    .width(Fill)
    .into()
}

pub(super) fn view_scrollbar_gutter<'a>() -> Element<'a, Message> {
    Space::with_width(Length::Fixed(SCROLLBAR_GUTTER_WIDTH)).into()
}

fn view_any_transcript_row<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
    find_match_segment_ids: &BTreeSet<String>,
    row_view: &TranscriptRowView,
) -> Option<Element<'a, Message>> {
    if row_view.row_kind != TranscriptRowKind::Annotation {
        Some(row::view_modified_transcript_row(row_view))
    } else {
        state.segment_drafts.get(&row_view.segment_id).map(|draft| {
            let is_find_match = find_match_segment_ids.contains(&row_view.segment_id);
            row::view_transcript_row(state, project, editor, row_view, draft, is_find_match)
        })
    }
}

fn push_pinned_row_island<'a>(
    mut rows: Column<'a, Message>,
    pinned: super::super::transcript_virtualization::PinnedVirtualRow,
    row: Option<Element<'a, Message>>,
) -> Column<'a, Message> {
    if pinned.leading_height > 0.0 {
        rows = rows.push(Space::with_height(Length::Fixed(pinned.leading_height)));
    }
    if let Some(row) = row {
        rows = rows.push(row);
    }
    if pinned.trailing_height > 0.0 {
        rows = rows.push(Space::with_height(Length::Fixed(pinned.trailing_height)));
    }
    rows
}
