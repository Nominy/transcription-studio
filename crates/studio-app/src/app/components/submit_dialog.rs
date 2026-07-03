use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Color, Element, Fill, Length};
use studio_app::{SubmitControlView, SubmitDialogDescriptionTone};

use super::super::Message;
use super::super::styles::modal_surface_style;

const SUBMIT_DIALOG_WIDTH: f32 = 440.0;

pub(super) fn view_submit_dialog(submit: Arc<SubmitControlView>) -> Element<'static, Message> {
    let description = match submit.dialog_description_tone {
        SubmitDialogDescriptionTone::Default => text(submit.dialog_description.clone()).size(13),
        SubmitDialogDescriptionTone::Error => text(submit.dialog_description.clone())
            .size(13)
            .color(Color::from_rgb8(185, 28, 28)),
    };
    let title_row = row![
        text(submit.dialog_title.clone()).size(16),
        Space::with_width(Fill),
        button(text(submit.close_label.clone()))
            .on_press_maybe((!submit.cancel_disabled).then_some(Message::CancelSubmitReview)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let mut content = column![
        title_row,
        description,
        row![
            Space::with_width(Fill),
            button(text(submit.cancel_label.clone()))
                .on_press_maybe((!submit.cancel_disabled).then_some(Message::CancelSubmitReview)),
            button(text(submit.confirm_label.clone()))
                .on_press_maybe((!submit.confirm_disabled).then_some(Message::SubmitReview)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(8);

    if submit.show_pending_overlay {
        content = content.push(text(submit.pending_overlay_text.clone()).size(12));
    }

    container(content)
        .padding(16)
        .width(Length::Fixed(SUBMIT_DIALOG_WIDTH))
        .style(modal_surface_style)
        .into()
}
