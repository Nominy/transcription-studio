use iced::widget::{Space, button, column, container, row, text};
use iced::{Element, Fill, Font, Length, font};
use studio_app::SkipChunkControlView;

use super::super::Message;
use super::super::styles::{modal_surface_style, primary_button_style, secondary_button_style};

const SKIP_CHUNK_DIALOG_WIDTH: f32 = 440.0;
const SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

pub(super) fn view_skip_chunk_dialog<'a>(skip_chunk: SkipChunkControlView) -> Element<'a, Message> {
    container(
        column![
            text(skip_chunk.title).size(18).font(SEMIBOLD),
            text(skip_chunk.description).size(13),
            row![
                Space::with_width(Fill),
                button(text(skip_chunk.cancel_label).size(13))
                    .style(secondary_button_style)
                    .on_press_maybe(
                        (!skip_chunk.cancel_disabled).then_some(Message::CancelSkipChunk)
                    ),
                button(text(skip_chunk.confirm_label).size(13))
                    .style(primary_button_style)
                    .on_press_maybe(
                        (!skip_chunk.confirm_disabled).then_some(Message::ConfirmSkipChunk)
                    ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(14),
    )
    .padding(22)
    .width(Length::Fixed(SKIP_CHUNK_DIALOG_WIDTH))
    .style(modal_surface_style)
    .into()
}
