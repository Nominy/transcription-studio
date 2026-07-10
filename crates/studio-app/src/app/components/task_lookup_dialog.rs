use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Element, Fill, Font, Length, font};
use studio_app::TaskLookupState;

use super::super::Message;
use super::super::styles::{modal_surface_style, primary_button_style, secondary_button_style};

const TASK_LOOKUP_DIALOG_WIDTH: f32 = 480.0;
const SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

pub(super) fn view_task_lookup_dialog<'a>(state: &'a TaskLookupState) -> Element<'a, Message> {
    if state.confirming_edit {
        let confirm = state.confirm_edit_model();
        return container(
            column![
                text(confirm.title).size(18).font(SEMIBOLD),
                text(confirm.body).size(13),
                row![
                    Space::with_width(Fill),
                    button(text(confirm.cancel_label).size(13))
                        .style(secondary_button_style)
                        .on_press(Message::TaskLookupCancelConfirmEdit),
                    button(text(confirm.confirm_label).size(13))
                        .style(primary_button_style)
                        .on_press(Message::TaskLookupConfirmEdit),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(14),
        )
        .padding(22)
        .width(Length::Fixed(TASK_LOOKUP_DIALOG_WIDTH))
        .style(modal_surface_style)
        .into();
    }

    let select = state.select_action_model();
    let mut content = column![
        text("Select Action").size(18).font(SEMIBOLD),
        text(select.description).size(13),
    ]
    .spacing(8);

    if select.show_processed_transcription_input {
        let mut processed_input = text_input(
            &select.processed_transcription_placeholder,
            &select.processed_transcription_id,
        )
        .on_input(Message::TaskLookupProcessedTranscriptionChanged)
        .padding(10)
        .size(13)
        .width(Fill);
        if !select.claim_stitched_disabled {
            processed_input = processed_input.on_submit(Message::TaskLookupClickClaimStitched);
        }
        content = content.push(processed_input);
    }

    let mut review_action_input =
        text_input(&select.review_action_placeholder, &select.review_action_id)
            .on_input(Message::TaskLookupReviewActionChanged)
            .padding(10)
            .size(13)
            .width(Fill);
    if !select.read_only_disabled {
        review_action_input = review_action_input.on_submit(Message::TaskLookupClickReadOnly);
    }
    content = content.push(review_action_input);

    content = content.push(
        row![
            Space::with_width(Fill),
            button(text("Claim Stitched").size(13))
                .style(secondary_button_style)
                .on_press_maybe(
                    (!select.claim_stitched_disabled)
                        .then_some(Message::TaskLookupClickClaimStitched),
                ),
            button(text("View Read-Only").size(13))
                .style(primary_button_style)
                .on_press_maybe(
                    (!select.read_only_disabled).then_some(Message::TaskLookupClickReadOnly),
                ),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    );

    container(content)
        .padding(22)
        .width(Length::Fixed(TASK_LOOKUP_DIALOG_WIDTH))
        .style(modal_surface_style)
        .into()
}
