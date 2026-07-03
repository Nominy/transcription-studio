use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Length};
use studio_app::FEEDBACK_MAX_COMMENT_LENGTH;

use super::super::styles::subtle_panel_style;
use super::super::{FeedbackPanelState, Message};

const FEEDBACK_PANEL_WIDTH: f32 = 420.0;

pub(super) fn view_feedback_panel<'a>(panel: &'a FeedbackPanelState) -> Element<'a, Message> {
    let mut body = column![].spacing(12);
    if panel.is_loading {
        body = body.push(text(if panel.read_only {
            "Loading feedback..."
        } else {
            "Loading draft..."
        }));
    } else if panel.read_only && panel.draft.entries.is_empty() {
        body = body.push(text("No feedback exists"));
    } else {
        for field in &panel.rubric_fields {
            if let Some(entry) = panel.draft.entries.get(field.key) {
                body = body.push(view_rubric_card(
                    field.key,
                    field.label,
                    field.description.unwrap_or_default(),
                    entry.rating,
                    &entry.comment,
                    panel.read_only,
                    panel.is_tts_project,
                ));
            }
        }

        if panel.has_other_feedback {
            body = body.push(view_other_feedback(
                &panel.draft.other_feedback,
                panel.read_only,
            ));
        }
    }

    let footer: Element<'a, Message> = if panel.read_only {
        Space::with_height(Length::Shrink).into()
    } else {
        row![Space::with_width(Fill), text(panel.footer_label()).size(12)]
            .align_y(iced::Alignment::Center)
            .into()
    };

    container(
        column![
            row![
                column![
                    text(panel.title()).size(16),
                    text(panel.description()).size(12),
                    autosave_status(panel),
                ]
                .spacing(4),
                Space::with_width(Fill),
                button(text("x")).on_press(Message::CloseFeedbackPanel),
            ]
            .align_y(iced::Alignment::Start),
            scrollable(body).height(Fill),
            footer,
        ]
        .spacing(12),
    )
    .padding(16)
    .width(Length::Fixed(FEEDBACK_PANEL_WIDTH))
    .height(Fill)
    .style(subtle_panel_style)
    .into()
}

fn autosave_status(panel: &FeedbackPanelState) -> Element<'static, Message> {
    if panel.read_only {
        return Space::with_height(Length::Shrink).into();
    }
    if panel.is_saving {
        text("Saving...").size(12).into()
    } else if panel.last_saved.is_some() {
        text("Saved").size(12).into()
    } else {
        text("Draft auto-saves").size(12).into()
    }
}

fn view_rubric_card<'a>(
    field_key: &'static str,
    label: &'static str,
    description: &'static str,
    rating: u8,
    comment: &'a str,
    read_only: bool,
    is_tts_project: bool,
) -> Element<'a, Message> {
    let mut rating_row = row![].spacing(6);
    for option in 1..=3 {
        let label = if option == rating {
            format!("{option} selected")
        } else if is_tts_project {
            format!("{option}")
        } else {
            format!("{option}")
        };
        rating_row = rating_row.push(button(text(label).size(12)).on_press_maybe(
            (!read_only).then_some(Message::FeedbackRatingChanged(
                field_key.to_string(),
                option,
            )),
        ));
    }

    let comment_required = !read_only && comment.trim().is_empty();
    let counter = format!("{}/{}", comment.len(), FEEDBACK_MAX_COMMENT_LENGTH);
    container(
        column![
            text(label).size(14),
            text(description).size(12),
            text("Rating *").size(12),
            rating_row,
            text("Comment *").size(12),
            text_input("Provide specific feedback...", comment)
                .on_input_maybe((!read_only).then_some(move |value| {
                    Message::FeedbackCommentChanged(field_key.to_string(), value)
                }))
                .padding(6)
                .width(Fill),
            row![
                if comment_required {
                    text("Comment required").size(12)
                } else {
                    text("").size(12)
                },
                Space::with_width(Fill),
                text(counter).size(12),
            ]
        ]
        .spacing(6),
    )
    .padding(10)
    .width(Fill)
    .style(subtle_panel_style)
    .into()
}

fn view_other_feedback<'a>(value: &'a str, read_only: bool) -> Element<'a, Message> {
    let counter = format!("{}/{}", value.len(), FEEDBACK_MAX_COMMENT_LENGTH);
    container(
        column![
            text("Other Feedback").size(14),
            text("Any additional comments or observations (optional)").size(12),
            text_input("Additional feedback...", value)
                .on_input_maybe((!read_only).then_some(Message::FeedbackOtherFeedbackChanged))
                .padding(6)
                .width(Fill),
            row![Space::with_width(Fill), text(counter).size(12)]
        ]
        .spacing(6),
    )
    .padding(10)
    .width(Fill)
    .style(subtle_panel_style)
    .into()
}
