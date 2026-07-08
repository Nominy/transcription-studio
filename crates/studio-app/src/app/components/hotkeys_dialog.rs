use iced::widget::{Column, Row, Space, button, column, container, row, scrollable, text};
use iced::{Element, Fill, Length};
#[cfg(test)]
use studio_app::CapabilityRegistry;

use super::super::keybinds::{HOTKEY_DEFINITIONS, HotkeyDefinition};
use super::super::styles::{kbd_style, modal_surface_style};
use super::super::{Message, StudioState};

const HOTKEYS_DIALOG_WIDTH: f32 = 760.0;
const HOTKEYS_DIALOG_HEIGHT: f32 = 620.0;

#[cfg(test)]
pub(crate) fn recovered_hotkey_registry()
-> Vec<(&'static str, Vec<(Vec<&'static str>, &'static str)>)> {
    CapabilityRegistry::default().hotkey_sections()
}

pub(super) fn view_hotkeys_dialog<'a>(state: &'a StudioState) -> Element<'a, Message> {
    let mut sections = Column::new().spacing(14);
    let mut start = 0;
    while start < HOTKEY_DEFINITIONS.len() {
        let section = HOTKEY_DEFINITIONS[start].section;
        let mut end = start + 1;
        while end < HOTKEY_DEFINITIONS.len() && HOTKEY_DEFINITIONS[end].section == section {
            end += 1;
        }
        sections = sections.push(view_section(
            state,
            section,
            &HOTKEY_DEFINITIONS[start..end],
        ));
        start = end;
    }

    let capture_hint = if state.hotkey_capture.is_some() {
        text("Listening for a physical key code. Press the desired key combination, or Cancel.")
            .size(12)
    } else {
        text("Shortcuts use physical key codes, so the same key positions work across keyboard languages.")
            .size(12)
    };

    container(
        column![
            row![
                column![text("Keyboard Shortcuts").size(16), capture_hint,].spacing(4),
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

fn view_section<'a>(
    state: &'a StudioState,
    title: &'static str,
    definitions: &'static [HotkeyDefinition],
) -> Element<'a, Message> {
    let mut rows = Column::new().spacing(4).push(text(title).size(13));
    for definition in definitions {
        rows = rows.push(view_shortcut_row(state, definition));
    }
    rows.into()
}

fn view_shortcut_row<'a>(
    state: &'a StudioState,
    definition: &HotkeyDefinition,
) -> Element<'a, Message> {
    let is_capturing = state.hotkey_capture == Some(definition.action);
    let mut description = Column::new()
        .spacing(2)
        .push(text(definition.description).size(12));
    if is_capturing {
        description = description.push(text("Press new key combination...").size(11));
    }

    let mut keys = Row::new().spacing(4).align_y(iced::Alignment::Center);
    for key in state.hotkey_bindings.labels_for(definition.action) {
        keys = keys.push(container(text(key).size(11)).padding(4).style(kbd_style));
    }
    if state.hotkey_bindings.is_overridden(definition.action) {
        keys = keys.push(text("custom").size(11));
    }

    let change = if is_capturing {
        button(text("Cancel")).on_press(Message::CancelHotkeyCapture)
    } else {
        button(text("Change")).on_press(Message::StartHotkeyCapture(definition.action))
    };
    let reset = button(text("Reset")).on_press_maybe(
        state
            .hotkey_bindings
            .is_overridden(definition.action)
            .then_some(Message::ResetHotkeyBinding(definition.action)),
    );

    container(
        row![description.width(Fill), keys, change, reset,]
            .spacing(8)
            .align_y(iced::Alignment::Center),
    )
    .padding(2)
    .width(Fill)
    .into()
}
