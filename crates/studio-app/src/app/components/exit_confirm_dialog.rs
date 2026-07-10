use iced::widget::{Space, button, column, container, row, text};
use iced::{Color, Element, Fill, Length};

use super::super::Message;
use super::super::styles::modal_surface_style;

const EXIT_CONFIRM_DIALOG_WIDTH: f32 = 440.0;

pub(super) fn view_exit_confirm_dialog<'a>(
    is_saving: bool,
    save_error: Option<&'a str>,
) -> Element<'a, Message> {
    let mut content = column![
        text("Unsaved changes").size(16),
        text(if is_saving {
            "Saving your latest changes before leaving..."
        } else {
            "This task has unsaved changes. Save them before leaving, or discard them."
        })
        .size(13),
    ]
    .spacing(8);
    if let Some(error) = save_error {
        content = content.push(
            text(format!("Save failed: {error}"))
                .size(12)
                .color(Color::from_rgb8(185, 28, 28)),
        );
    }
    content = content.push(
        row![
            Space::with_width(Fill),
            button(text("Keep Editing")).on_press(Message::CancelPendingLeave),
            button(text("Discard Changes")).on_press(Message::DiscardAndContinuePendingLeave),
            button(text(if is_saving {
                "Saving..."
            } else {
                "Save & Continue"
            }))
            .on_press_maybe((!is_saving).then_some(Message::SaveAndContinuePendingLeave)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    );

    container(content)
        .padding(16)
        .width(Length::Fixed(EXIT_CONFIRM_DIALOG_WIDTH))
        .style(modal_surface_style)
        .into()
}
