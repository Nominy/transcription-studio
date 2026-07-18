use iced::widget::{Space, button, column, container, row, scrollable, text, text_editor};
use iced::{Element, Fill, Font, Length, Theme, font};
use studio_app::FEEDBACK_MAX_COMMENT_LENGTH;

use super::super::review_backend_client::ReviewBackendRequest;
use super::super::styles::{
    error_pill_style, feedback_card_style, feedback_footer_style, feedback_panel_style,
    feedback_rating_button_style, primary_button_style, secondary_button_style, success_pill_style,
    warning_pill_style,
};
use super::super::{FeedbackPanelState, Message, StudioState};

const FEEDBACK_PANEL_WIDTH: f32 = 512.0;
const CONTROL_HEIGHT: f32 = 40.0;
const COMMENT_EDITOR_HEIGHT: f32 = 104.0;
const SPACING_XS: u16 = 4;
const SPACING_SM: u16 = 8;
const SPACING_MD: u16 = 16;
const SPACING_LG: u16 = 24;
const SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

pub(super) fn view_feedback_panel<'a>(
    state: &'a StudioState,
    panel: &'a FeedbackPanelState,
) -> Element<'a, Message> {
    let body = view_feedback_body(panel);
    let generate_pending = reviewer_generate_pending(state);
    let magic_review_disabled =
        panel.read_only || panel.is_loading || panel.bootstrap_error.is_some() || generate_pending;

    let magic_review = button(text("Magic Review").size(13).font(SEMIBOLD))
        .style(primary_button_style)
        .on_press_maybe((!magic_review_disabled).then_some(Message::ReviewerGenerateReview))
        .padding([SPACING_SM, 12])
        .height(Length::Fixed(CONTROL_HEIGHT));

    let header = container(
        column![
            row![
                column![
                    text(panel.title()).size(20).font(SEMIBOLD),
                    text(panel.description()).size(13),
                ]
                .spacing(SPACING_XS)
                .width(Fill),
                button(text("Close").size(12))
                    .style(secondary_button_style)
                    .on_press(Message::CloseFeedbackPanel)
                    .padding([SPACING_SM, 12]),
            ]
            .spacing(SPACING_MD)
            .align_y(iced::Alignment::Start),
            row![
                autosave_status(panel),
                if generate_pending {
                    text("Magic Review is generating feedback.").size(11)
                } else {
                    text("").size(11)
                },
                Space::with_width(Fill),
                magic_review,
            ]
            .spacing(SPACING_SM)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(SPACING_MD),
    )
    .padding([20, SPACING_LG])
    .width(Fill);

    container(
        column![
            header,
            scrollable(container(body).padding([0, SPACING_LG]).width(Fill)).height(Fill),
            completion_footer(panel),
        ]
        .spacing(0),
    )
    .width(Length::Fixed(FEEDBACK_PANEL_WIDTH))
    .height(Fill)
    .style(feedback_panel_style)
    .into()
}

fn view_feedback_body<'a>(panel: &'a FeedbackPanelState) -> Element<'a, Message> {
    let mut body = column![].spacing(SPACING_MD).width(Fill);
    if let Some(error) = panel.bootstrap_error.as_deref() {
        body = body.push(
            container(
                column![
                    text("The feedback form could not be loaded.")
                        .size(15)
                        .font(SEMIBOLD),
                    text(error).size(12),
                    button(text("Retry loading").size(12))
                        .style(secondary_button_style)
                        .on_press(Message::RetryFeedbackBootstrap)
                        .padding([SPACING_SM, 12]),
                ]
                .spacing(SPACING_SM),
            )
            .padding(SPACING_MD)
            .width(Fill)
            .style(feedback_card_style),
        );
    } else if panel.is_loading {
        body = body.push(
            container(
                column![
                    text(if panel.read_only {
                        "Loading feedback"
                    } else {
                        "Loading rubric categories"
                    })
                    .size(15)
                    .font(SEMIBOLD),
                    text("Your feedback form will appear here when it is ready.").size(12),
                ]
                .spacing(SPACING_XS),
            )
            .padding(SPACING_MD)
            .width(Fill)
            .style(feedback_card_style),
        );
    } else if panel.read_only && panel.draft.entries.is_empty() {
        body = body.push(
            container(
                column![
                    text("No feedback was provided").size(15).font(SEMIBOLD),
                    text("This completed review does not contain rubric feedback.").size(12),
                ]
                .spacing(SPACING_XS),
            )
            .padding(SPACING_MD)
            .width(Fill)
            .style(feedback_card_style),
        );
    } else if panel.rubric_fields.is_empty() {
        body = body.push(
            container(
                column![
                    text("No rubric categories are available")
                        .size(15)
                        .font(SEMIBOLD),
                    text("Retry loading the form before completing this review.").size(12),
                ]
                .spacing(SPACING_XS),
            )
            .padding(SPACING_MD)
            .width(Fill)
            .style(feedback_card_style),
        );
    } else {
        for field in &panel.rubric_fields {
            if let (Some(entry), Some(comment_content)) = (
                panel.draft.entries.get(field.key),
                panel.comment_content(field.key),
            ) {
                body = body.push(view_rubric_card(
                    field.key,
                    field.label,
                    field.description.unwrap_or_default(),
                    entry.rating,
                    &entry.comment,
                    comment_content,
                    panel.read_only,
                ));
            }
        }

        if panel.has_other_feedback
            && let Some(other_feedback_content) = panel.other_feedback_content()
        {
            body = body.push(view_other_feedback(
                &panel.draft.other_feedback,
                other_feedback_content,
                panel.read_only,
            ));
        }
    }

    container(body).padding([SPACING_SM, 0]).width(Fill).into()
}

fn autosave_status(panel: &FeedbackPanelState) -> Element<'static, Message> {
    if panel.read_only {
        return status_pill("Read-only", warning_pill_style);
    }
    if panel.is_loading {
        return status_pill("Loading feedback", warning_pill_style);
    }
    if panel.bootstrap_error.is_some() {
        return status_pill("Feedback unavailable", error_pill_style);
    }
    if let Some(error) = panel.last_autosave_error.as_deref() {
        return container(
            column![
                text(format!("Autosave failed: {error}"))
                    .size(11)
                    .font(SEMIBOLD),
                text("Edit again to retry automatically.").size(11),
            ]
            .spacing(SPACING_XS),
        )
        .padding([SPACING_XS, SPACING_SM])
        .width(Fill)
        .style(error_pill_style)
        .into();
    }
    if panel.is_saving {
        status_pill("Autosaving", warning_pill_style)
    } else if panel.last_saved.is_some() {
        status_pill("All changes saved", success_pill_style)
    } else {
        status_pill("Autosave is on", success_pill_style)
    }
}

fn status_pill(
    label: &'static str,
    style: fn(&Theme) -> iced::widget::container::Style,
) -> Element<'static, Message> {
    container(text(label).size(11))
        .padding([SPACING_XS, SPACING_SM])
        .style(style)
        .into()
}

fn view_rubric_card<'a>(
    field_key: &'static str,
    label: &'static str,
    description: &'static str,
    rating: u8,
    comment: &'a str,
    comment_content: &'a text_editor::Content,
    read_only: bool,
) -> Element<'a, Message> {
    let mut rating_row = row![].spacing(SPACING_SM).width(Fill);
    for (option, choice_label) in [(1, "1 Good"), (2, "2 OK"), (3, "3 Needs Work")] {
        rating_row = rating_row.push(
            button(text(choice_label).size(12))
                .style(feedback_rating_button_style(option == rating))
                .on_press_maybe((!read_only).then_some(Message::FeedbackRatingChanged(
                    field_key.to_string(),
                    option,
                )))
                .padding(SPACING_SM)
                .height(Length::Fixed(CONTROL_HEIGHT))
                .width(Fill),
        );
    }

    let comment_complete = !comment.trim().is_empty();
    let counter = format!("{}/{}", comment.len(), FEEDBACK_MAX_COMMENT_LENGTH);
    let comment_status = if comment_complete {
        status_pill("Required · Complete", success_pill_style)
    } else {
        status_pill("Required · Comment needed", error_pill_style)
    };

    let mut comment_editor = text_editor(comment_content)
        .placeholder("Provide specific, actionable feedback...")
        .padding(12)
        .size(13)
        .height(Length::Fixed(COMMENT_EDITOR_HEIGHT));
    if !read_only {
        let field_key = field_key.to_string();
        comment_editor = comment_editor
            .on_action(move |action| Message::FeedbackCommentEdited(field_key.clone(), action));
    }

    container(
        column![
            row![
                text(label).size(15).font(SEMIBOLD),
                Space::with_width(Fill),
                status_pill("Required", warning_pill_style),
            ]
            .spacing(SPACING_SM)
            .align_y(iced::Alignment::Center),
            text(description).size(12),
            text("Rating *").size(12).font(SEMIBOLD),
            rating_row,
            row![
                text("Comment *").size(12).font(SEMIBOLD),
                Space::with_width(Fill),
                text(counter).size(11),
            ]
            .align_y(iced::Alignment::Center),
            comment_editor,
            comment_status,
        ]
        .spacing(SPACING_SM),
    )
    .padding(SPACING_MD)
    .width(Fill)
    .style(feedback_card_style)
    .into()
}

fn view_other_feedback<'a>(
    value: &'a str,
    content: &'a text_editor::Content,
    read_only: bool,
) -> Element<'a, Message> {
    let counter = format!("{}/{}", value.len(), FEEDBACK_MAX_COMMENT_LENGTH);
    let mut feedback_editor = text_editor(content)
        .placeholder("Add optional feedback...")
        .padding(12)
        .size(13)
        .height(Length::Fixed(COMMENT_EDITOR_HEIGHT));
    if !read_only {
        feedback_editor = feedback_editor.on_action(Message::FeedbackOtherFeedbackEdited);
    }
    container(
        column![
            row![
                text("Other Feedback").size(15).font(SEMIBOLD),
                Space::with_width(Fill),
                status_pill("Optional", success_pill_style),
            ]
            .spacing(SPACING_SM)
            .align_y(iced::Alignment::Center),
            text("Share any additional comments or observations.").size(12),
            row![
                text("Additional comments").size(12).font(SEMIBOLD),
                Space::with_width(Fill),
                text(counter).size(11),
            ]
            .align_y(iced::Alignment::Center),
            feedback_editor,
        ]
        .spacing(SPACING_SM),
    )
    .padding(SPACING_MD)
    .width(Fill)
    .style(feedback_card_style)
    .into()
}

fn completion_footer<'a>(panel: &FeedbackPanelState) -> Element<'a, Message> {
    let completed = panel
        .rubric_fields
        .iter()
        .filter(|field| {
            panel
                .draft
                .entries
                .get(field.key)
                .is_some_and(|entry| !entry.comment.trim().is_empty())
        })
        .count();
    let total = panel.rubric_fields.len();
    let completion = format!("{completed} of {total} required sections complete");
    let footer_status = if panel.read_only {
        "Feedback received"
    } else {
        panel.footer_label()
    };

    container(
        row![
            column![
                text("Completion").size(11).font(SEMIBOLD),
                text(completion).size(12),
            ]
            .spacing(SPACING_XS),
            Space::with_width(Fill),
            text(footer_status).size(12).font(SEMIBOLD),
        ]
        .spacing(SPACING_MD)
        .align_y(iced::Alignment::Center),
    )
    .padding([12, SPACING_LG])
    .width(Fill)
    .style(feedback_footer_style)
    .into()
}

fn reviewer_generate_pending(state: &StudioState) -> bool {
    state
        .review_backend_requests
        .iter()
        .chain(state.dispatched_review_backend_requests.iter())
        .any(|request| matches!(request, ReviewBackendRequest::ReviewGenerate(_)))
}
