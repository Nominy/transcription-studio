use std::{
    hash::{Hash, Hasher},
    sync::LazyLock,
};

use iced::widget::svg::{self, Handle as SvgHandle, Svg};
use iced::widget::{Column, Space, button, column, container, lazy, row, text};
use iced::{Color, Element, Fill, Length};
use studio_app::{
    EditorSession, TranscriptActionCommand, TranscriptActionMenuEntryView,
    TranscriptActionMenuItemView, TranscriptActionMenuView, TranscriptRowView,
};
use studio_core::Project;

use super::super::super::Message;
use super::super::super::styles::{
    action_menu_delete_button_style, action_menu_item_button_style, action_menu_popup_style,
    action_menu_separator_style, table_plain_button_style,
};

const ACTION_MENU_ICON_SIZE: f32 = 16.0;
const ACTION_MENU_TRIGGER_WIDTH: f32 = 20.0;
const ACTION_MENU_POPUP_WIDTH: f32 = 192.0;
const ACTION_MENU_POPUP_ITEM_HEIGHT: f32 = 28.0;
const ACTION_MENU_ITEM_ICON_SIZE: f32 = 12.0;
const ACTION_MENU_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black"><circle cx="12" cy="5" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="12" cy="19" r="2"/></svg>"#;
const ACTION_MENU_ICON_ADD: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="M12 5v14"/></svg>"#;
const ACTION_MENU_ICON_MERGE_ABOVE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>"#;
const ACTION_MENU_ICON_MERGE_BELOW: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 5 7 7-7 7"/><path d="M5 12h14"/></svg>"#;
const ACTION_MENU_ICON_DELETE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/></svg>"#;
static ACTION_MENU_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(ACTION_MENU_ICON));
static ACTION_MENU_ICON_ADD_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(ACTION_MENU_ICON_ADD));
static ACTION_MENU_ICON_MERGE_ABOVE_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(ACTION_MENU_ICON_MERGE_ABOVE));
static ACTION_MENU_ICON_MERGE_BELOW_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(ACTION_MENU_ICON_MERGE_BELOW));
static ACTION_MENU_ICON_DELETE_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(ACTION_MENU_ICON_DELETE));

struct ActionCellDependency {
    segment_id: String,
    is_open: bool,
    menu: Option<TranscriptActionMenuView>,
}

impl std::hash::Hash for ActionCellDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.segment_id.hash(state);
        self.is_open.hash(state);
        if let Some(menu) = &self.menu {
            menu.segment_id.hash(state);
            menu.items.len().hash(state);
            for entry in &menu.items {
                hash_action_entry(entry, state);
            }
        }
    }
}

pub(super) fn view_action_cell(
    project: &Project,
    editor: &EditorSession,
    row_view: &TranscriptRowView,
    is_open: bool,
) -> Element<'static, Message> {
    view_lazy_action_cell(ActionCellDependency {
        segment_id: row_view.segment_id.clone(),
        is_open,
        menu: is_open.then(|| editor.transcript_action_menu(project, row_view)),
    })
}

fn view_lazy_action_cell(dependency: ActionCellDependency) -> Element<'static, Message> {
    lazy(dependency, view_action_cell_content).into()
}

fn view_action_cell_content(dependency: &ActionCellDependency) -> Element<'static, Message> {
    let toggle = action_menu_svg_trigger(dependency.segment_id.clone());

    if !dependency.is_open {
        return container(align_action_trigger(toggle))
            .width(Length::Fixed(super::ACTION_COLUMN_WIDTH))
            .into();
    }

    let menu = dependency
        .menu
        .as_ref()
        .expect("open action cells carry their popup menu");
    let popup = view_action_popup_menu(&menu.segment_id, menu.items.clone());
    let open_trigger =
        container(align_action_trigger(toggle)).align_right(Length::Fixed(ACTION_MENU_POPUP_WIDTH));

    container(
        column![
            container(popup).align_right(Length::Fixed(ACTION_MENU_POPUP_WIDTH)),
            open_trigger
        ]
        .spacing(2),
    )
    .width(Length::Fixed(ACTION_MENU_POPUP_WIDTH))
    .into()
}

fn hash_action_entry<H: Hasher>(entry: &TranscriptActionMenuEntryView, state: &mut H) {
    match entry {
        TranscriptActionMenuEntryView::Item(item) => {
            1_u8.hash(state);
            hash_action_command(&item.command, state);
            item.label.hash(state);
            item.enabled.hash(state);
        }
        TranscriptActionMenuEntryView::Separator => {
            2_u8.hash(state);
        }
    }
}

fn hash_action_command<H: Hasher>(command: &TranscriptActionCommand, state: &mut H) {
    let key = match command {
        TranscriptActionCommand::Edit => 1_u8,
        TranscriptActionCommand::MergeAbove => 2,
        TranscriptActionCommand::MergeBelow => 3,
        TranscriptActionCommand::CreateBefore => 4,
        TranscriptActionCommand::CreateAfter => 5,
        TranscriptActionCommand::Delete => 6,
    };
    key.hash(state);
}

fn align_action_trigger(trigger: Element<'static, Message>) -> Element<'static, Message> {
    row![Space::with_width(Fill), trigger]
        .width(Length::Fixed(super::ACTION_COLUMN_WIDTH))
        .align_y(iced::Alignment::Center)
        .into()
}

fn action_menu_svg_trigger(segment_id: String) -> Element<'static, Message> {
    let icon = Svg::new(ACTION_MENU_ICON_HANDLE.clone())
        .width(Length::Fixed(ACTION_MENU_ICON_SIZE))
        .height(Length::Fixed(ACTION_MENU_ICON_SIZE))
        .style(|_, _| svg::Style {
            color: Some(Color::from_rgb8(75, 85, 99)),
        });
    let trigger = row![Space::with_width(Fill), icon]
        .width(Fill)
        .align_y(iced::Alignment::Center);

    button(trigger)
        .style(table_plain_button_style)
        .on_press(Message::ToggleActions(segment_id))
        .padding(0)
        .width(Length::Fixed(super::ACTION_COLUMN_WIDTH))
        .height(Length::Fixed(ACTION_MENU_TRIGGER_WIDTH))
        .into()
}

fn view_action_popup_menu(
    segment_id: &str,
    entries: Vec<TranscriptActionMenuEntryView>,
) -> Element<'static, Message> {
    let mut menu_column = Column::new()
        .spacing(0)
        .padding([4, 4])
        .width(Length::Fixed(ACTION_MENU_POPUP_WIDTH));

    for entry in entries {
        menu_column = menu_column.push(view_action_entry(segment_id, entry));
    }

    container(menu_column)
        .width(Length::Fixed(ACTION_MENU_POPUP_WIDTH))
        .style(action_menu_popup_style)
        .into()
}

fn view_action_entry(
    segment_id: &str,
    entry: TranscriptActionMenuEntryView,
) -> Element<'static, Message> {
    match entry {
        TranscriptActionMenuEntryView::Item(item) => view_action_button(segment_id, item),
        TranscriptActionMenuEntryView::Separator => container(text(""))
            .height(Length::Fixed(1.0))
            .width(Fill)
            .style(action_menu_separator_style)
            .into(),
    }
}

fn view_action_button(
    segment_id: &str,
    item: TranscriptActionMenuItemView,
) -> Element<'static, Message> {
    let is_delete = item.command == TranscriptActionCommand::Delete;
    let icon = view_action_button_icon(&item.command, item.enabled);
    let content = row![icon, text(item.label).size(12).width(Fill)]
        .spacing(8)
        .align_y(iced::Alignment::Center);
    let action_button = button(content)
        .style(if is_delete {
            action_menu_delete_button_style
        } else {
            action_menu_item_button_style
        })
        .padding([0, 8])
        .height(Length::Fixed(ACTION_MENU_POPUP_ITEM_HEIGHT))
        .width(Fill);

    if item.enabled {
        action_button
            .on_press(action_message(&item.command, segment_id))
            .into()
    } else {
        action_button.into()
    }
}

fn view_action_button_icon(
    command: &TranscriptActionCommand,
    enabled: bool,
) -> Element<'static, Message> {
    let icon: SvgHandle = match command {
        TranscriptActionCommand::Edit
        | TranscriptActionCommand::CreateBefore
        | TranscriptActionCommand::CreateAfter => ACTION_MENU_ICON_ADD_HANDLE.clone(),
        TranscriptActionCommand::MergeAbove => ACTION_MENU_ICON_MERGE_ABOVE_HANDLE.clone(),
        TranscriptActionCommand::MergeBelow => ACTION_MENU_ICON_MERGE_BELOW_HANDLE.clone(),
        TranscriptActionCommand::Delete => ACTION_MENU_ICON_DELETE_HANDLE.clone(),
    };
    let color = if !enabled {
        Color::from_rgb8(163, 163, 163)
    } else if *command == TranscriptActionCommand::Delete {
        Color::from_rgb8(220, 38, 38)
    } else {
        Color::from_rgb8(82, 82, 82)
    };

    Svg::new(icon)
        .width(Length::Fixed(ACTION_MENU_ITEM_ICON_SIZE))
        .height(Length::Fixed(ACTION_MENU_ITEM_ICON_SIZE))
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

fn action_message(command: &TranscriptActionCommand, segment_id: &str) -> Message {
    let segment_id = segment_id.to_string();
    match command {
        TranscriptActionCommand::Edit => Message::RequestEditAnnotation(segment_id),
        TranscriptActionCommand::MergeAbove => Message::MergeSegmentAbove(segment_id),
        TranscriptActionCommand::MergeBelow => Message::MergeSegmentBelow(segment_id),
        TranscriptActionCommand::CreateBefore => Message::CreateSegmentBefore(segment_id),
        TranscriptActionCommand::CreateAfter => Message::CreateSegmentAfter(segment_id),
        TranscriptActionCommand::Delete => Message::DeleteSegment(segment_id),
    }
}
