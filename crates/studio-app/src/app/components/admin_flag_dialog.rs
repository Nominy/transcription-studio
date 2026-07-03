use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Element, Fill, Length};
use studio_app::AdminFlagControlView;

use super::super::Message;
use super::super::styles::modal_surface_style;

const ADMIN_FLAG_DIALOG_WIDTH: f32 = 460.0;
const ADMIN_FLAG_REASON_INPUT_ID: &str = "babel-admin-flag-reason-input";

pub(crate) fn admin_flag_reason_input_id() -> text_input::Id {
    text_input::Id::new(ADMIN_FLAG_REASON_INPUT_ID)
}

pub(super) fn view_admin_flag_dialog<'a>(admin_flag: AdminFlagControlView) -> Element<'a, Message> {
    let mut reason_input = text_input(&admin_flag.reason_placeholder, &admin_flag.reason_value)
        .id(admin_flag_reason_input_id())
        .on_input_maybe((!admin_flag.reason_disabled).then_some(Message::AdminFlagReasonChanged))
        .padding(6)
        .width(Fill);
    if !admin_flag.confirm_disabled {
        reason_input = reason_input.on_submit(Message::ConfirmAdminFlag);
    }

    container(
        column![
            text(admin_flag.title).size(16),
            text(admin_flag.description).size(13),
            column![text(admin_flag.reason_label).size(12), reason_input,].spacing(4),
            row![
                Space::with_width(Fill),
                button(text(admin_flag.cancel_label)).on_press_maybe(
                    (!admin_flag.cancel_disabled).then_some(Message::CancelAdminFlag)
                ),
                button(text(admin_flag.confirm_label)).on_press_maybe(
                    (!admin_flag.confirm_disabled).then_some(Message::ConfirmAdminFlag)
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(16)
    .width(Length::Fixed(ADMIN_FLAG_DIALOG_WIDTH))
    .style(modal_surface_style)
    .into()
}
