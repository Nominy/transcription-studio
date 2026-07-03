use iced::widget::{Space, button, column, container, row, text};
use iced::{Element, Fill, Length};

use super::super::Message;
use super::super::styles::modal_surface_style;

const EXIT_CONFIRM_DIALOG_WIDTH: f32 = 440.0;

pub(super) fn view_exit_confirm_dialog<'a>() -> Element<'a, Message> {
    container(
        column![
            text("Unsaved changes").size(16),
            text(
                "This task has unsaved changes. If you close the app now, \
                 your latest edits will be lost."
            )
            .size(13),
            row![
                Space::with_width(Fill),
                button(text("Keep Editing")).on_press(Message::CancelExit),
                button(text("Discard and Exit")).on_press(Message::ConfirmExitDiscardingChanges),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(16)
    .width(Length::Fixed(EXIT_CONFIRM_DIALOG_WIDTH))
    .style(modal_surface_style)
    .into()
}
