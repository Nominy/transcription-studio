use iced::widget::{Row, container, text};
use iced::{Element, Fill, Length};
use studio_app::TranscriptRowVisibleCells;

use super::super::super::Message;

const ID_COLUMN: &str = "ID";
const SPEAKER_COLUMN: &str = "Speaker";
const START_COLUMN: &str = "Start (mm:ss)";
const END_COLUMN: &str = "End (mm:ss)";
const REVIEWER_COLUMN: &str = "Reviewer";
const TEXT_COLUMN: &str = "Text";
const LINTER_COLUMN: &str = "Linter";
const ACTIONS_COLUMN: &str = "Actions";

pub(super) fn view_table_header<'a>(
    visible_cells: TranscriptRowVisibleCells,
) -> Element<'a, Message> {
    let mut header = Row::new().spacing(0).padding([4, 0]);

    if visible_cells.id {
        header = header.push(header_cell(
            ID_COLUMN,
            Length::Fixed(super::ID_COLUMN_WIDTH),
        ));
    }
    if visible_cells.track {
        header = header.push(header_cell(
            SPEAKER_COLUMN,
            Length::Fixed(super::SPEAKER_COLUMN_WIDTH),
        ));
    }
    if visible_cells.start {
        header = header.push(header_cell(
            START_COLUMN,
            Length::Fixed(super::TIME_COLUMN_WIDTH),
        ));
    }
    if visible_cells.end {
        header = header.push(header_cell(
            END_COLUMN,
            Length::Fixed(super::TIME_COLUMN_WIDTH),
        ));
    }
    if visible_cells.reviewer {
        header = header.push(header_cell(
            REVIEWER_COLUMN,
            Length::Fixed(super::REVIEWER_COLUMN_WIDTH),
        ));
    }
    if visible_cells.text {
        header = header.push(header_cell(TEXT_COLUMN, Fill));
    }
    if visible_cells.linter {
        header = header.push(header_cell(
            LINTER_COLUMN,
            Length::Fixed(super::LINTER_COLUMN_WIDTH),
        ));
    }
    if visible_cells.actions {
        header = header.push(header_cell(
            ACTIONS_COLUMN,
            Length::Fixed(super::ACTION_COLUMN_WIDTH),
        ));
        header = header.push(super::view_scrollbar_gutter());
    }

    header.into()
}

fn header_cell<'a>(label: &'static str, width: impl Into<Length>) -> Element<'a, Message> {
    container(text(label).size(12))
        .padding([0, 8])
        .width(width)
        .into()
}

#[cfg(test)]
pub(super) fn visible_column_labels(
    show_reviewers: bool,
    hide_timestamps: bool,
) -> Vec<&'static str> {
    let visible_cells = TranscriptRowVisibleCells {
        id: !hide_timestamps,
        track: !hide_timestamps,
        start: !hide_timestamps,
        end: !hide_timestamps,
        reviewer: show_reviewers,
        text: true,
        linter: true,
        actions: !hide_timestamps,
    };
    let mut columns = Vec::new();
    if visible_cells.id {
        columns.push(ID_COLUMN);
    }
    if visible_cells.track {
        columns.push(SPEAKER_COLUMN);
    }
    if visible_cells.start {
        columns.push(START_COLUMN);
    }
    if visible_cells.end {
        columns.push(END_COLUMN);
    }
    if visible_cells.reviewer {
        columns.push(REVIEWER_COLUMN);
    }
    if visible_cells.text {
        columns.push(TEXT_COLUMN);
    }
    if visible_cells.linter {
        columns.push(LINTER_COLUMN);
    }
    if visible_cells.actions {
        columns.push(ACTIONS_COLUMN);
    }
    columns
}

#[cfg(test)]
mod tests {
    #[test]
    fn uses_babel_visible_columns_without_split_column() {
        assert_eq!(
            super::visible_column_labels(false, false),
            vec![
                "ID",
                "Speaker",
                "Start (mm:ss)",
                "End (mm:ss)",
                "Text",
                "Linter",
                "Actions"
            ]
        );
    }

    #[test]
    fn can_include_babel_reviewer_column_when_enabled() {
        assert_eq!(
            super::visible_column_labels(true, false),
            vec![
                "ID",
                "Speaker",
                "Start (mm:ss)",
                "End (mm:ss)",
                "Reviewer",
                "Text",
                "Linter",
                "Actions"
            ]
        );
    }

    #[test]
    fn hides_recovered_babel_timestamp_columns_when_enabled() {
        assert_eq!(
            super::visible_column_labels(true, true),
            vec!["Reviewer", "Text", "Linter"]
        );
    }
}
