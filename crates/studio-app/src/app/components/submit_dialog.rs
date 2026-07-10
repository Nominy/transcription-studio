use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Color, Element, Fill, Font, Length, font};
use studio_app::{SubmitControlView, SubmitDialogDescriptionTone};

use super::super::Message;
use super::super::styles::{
    modal_surface_style, primary_button_style, secondary_button_style, warning_pill_style,
};

const SUBMIT_DIALOG_WIDTH: f32 = 440.0;
const SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

pub(super) fn view_submit_dialog(submit: Arc<SubmitControlView>) -> Element<'static, Message> {
    let description = match submit.dialog_description_tone {
        SubmitDialogDescriptionTone::Default => text(submit.dialog_description.clone()).size(13),
        SubmitDialogDescriptionTone::Error => text(submit.dialog_description.clone())
            .size(13)
            .color(Color::from_rgb8(185, 28, 28)),
    };
    let title_row = row![
        text(submit.dialog_title.clone()).size(18).font(SEMIBOLD),
        Space::with_width(Fill),
        button(text(submit.close_label.clone()).size(12))
            .style(secondary_button_style)
            .on_press_maybe((!submit.cancel_disabled).then_some(Message::CancelSubmitReview)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let mut content = column![
        title_row,
        description,
        row![
            Space::with_width(Fill),
            button(text(submit.cancel_label.clone()).size(13))
                .style(secondary_button_style)
                .on_press_maybe((!submit.cancel_disabled).then_some(Message::CancelSubmitReview)),
            button(text(submit.confirm_label.clone()).size(13))
                .style(primary_button_style)
                .on_press_maybe((!submit.confirm_disabled).then_some(Message::SubmitReview)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(14);

    if submit.show_pending_overlay {
        content = content.push(
            container(text(submit.pending_overlay_text.clone()).size(12))
                .padding([6, 10])
                .style(warning_pill_style),
        );
    }

    container(content)
        .padding(22)
        .width(Length::Fixed(SUBMIT_DIALOG_WIDTH))
        .style(modal_surface_style)
        .into()
}
