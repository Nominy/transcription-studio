use iced::widget::{button, column, container, text};
use iced::{Alignment, Element, Fill, Length};
use studio_app::NoWorkModalModel;

use super::super::Message;
use super::super::styles::{modal_surface_style, outline_button_style};

const NO_WORK_DIALOG_WIDTH: f32 = 420.0;

pub(super) fn view_no_work_dialog(model: &NoWorkModalModel) -> Element<'static, Message> {
    if !model.open {
        return container(column![])
            .width(Fill)
            .height(Length::Shrink)
            .into();
    }

    container(
        column![
            text(model.title.clone()).size(18),
            text(model.description.clone()).size(13),
            button(text(model.button_label.clone()).size(13))
                .style(outline_button_style)
                .on_press(Message::BackToProjects)
                .padding([6, 12])
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
    .padding(20)
    .width(Length::Fixed(NO_WORK_DIALOG_WIDTH))
    .style(modal_surface_style)
    .into()
}
