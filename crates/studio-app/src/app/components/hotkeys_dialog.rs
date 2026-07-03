use iced::widget::{Column, Row, Space, button, column, container, row, scrollable, text};
use iced::{Element, Fill, Length};
use studio_app::{CapabilityHotkeySection, CapabilityRegistry};

use super::super::Message;
use super::super::styles::{kbd_style, modal_surface_style};

const HOTKEYS_DIALOG_WIDTH: f32 = 560.0;
const HOTKEYS_DIALOG_HEIGHT: f32 = 620.0;

#[cfg(test)]
pub(crate) fn recovered_hotkey_registry()
-> Vec<(&'static str, Vec<(Vec<&'static str>, &'static str)>)> {
    CapabilityRegistry::default().hotkey_sections()
}

pub(super) fn view_hotkeys_dialog<'a>() -> Element<'a, Message> {
    let mut sections = Column::new().spacing(14);
    let hotkey_sections = CapabilityRegistry::default().hotkey_sections();
    for section in &hotkey_sections {
        sections = sections.push(view_section(section));
    }

    container(
        column![
            row![
                column![
                    text("Keyboard Shortcuts").size(16),
                    text("Use these shortcuts to navigate and control the transcription workbench")
                        .size(12),
                ]
                .spacing(4),
                Space::with_width(Fill),
                button(text("Close")).on_press(Message::CloseHotkeys),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            scrollable(container(sections).padding([0, 8])).height(Fill),
        ]
        .spacing(14),
    )
    .padding(16)
    .width(Length::Fixed(HOTKEYS_DIALOG_WIDTH))
    .height(Length::Fixed(HOTKEYS_DIALOG_HEIGHT))
    .style(modal_surface_style)
    .into()
}

fn view_section<'a>(section: &CapabilityHotkeySection) -> Element<'a, Message> {
    let mut rows = Column::new().spacing(3).push(text(section.0).size(13));
    for shortcut in &section.1 {
        rows = rows.push(view_shortcut_row(shortcut));
    }
    rows.into()
}

fn view_shortcut_row<'a>(shortcut: &(Vec<&'static str>, &'static str)) -> Element<'a, Message> {
    let mut keys = Row::new().spacing(4).align_y(iced::Alignment::Center);
    for key in &shortcut.0 {
        keys = keys.push(container(text(*key).size(11)).padding(4).style(kbd_style));
    }

    container(
        row![text(shortcut.1).size(12).width(Fill), keys,]
            .spacing(12)
            .align_y(iced::Alignment::Center),
    )
    .padding(2)
    .width(Fill)
    .into()
}
