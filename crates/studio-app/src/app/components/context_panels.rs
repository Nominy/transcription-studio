use std::sync::LazyLock;

use iced::widget::svg::{self, Handle as SvgHandle, Svg};
use iced::widget::{Column, Space, button, column, container, row, text, tooltip};
use iced::{Alignment, Color, Element, Fill, Length};
use studio_app::{
    CollapsiblePanelModel, CollapsiblePanelToggleIcon, OriginalScriptPanelModel,
    ScriptContextPanelModel, SubmissionCopyIcon, SubmissionIdModel,
};
use studio_core::Project;

use super::super::styles::{subtle_panel_style, table_plain_button_style};
use super::super::{Message, StudioState};

const PANEL_TOGGLE_SIZE: f32 = 22.0;
const PANEL_ICON_SIZE: f32 = 14.0;
const COPY_BUTTON_HEIGHT: f32 = 24.0;
const COPY_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>"#;
const CHECK_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>"#;
const SHOW_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="M5 12h14"/></svg>"#;
const HIDE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/></svg>"#;
static COPY_ICON_HANDLE: LazyLock<SvgHandle> = LazyLock::new(|| SvgHandle::from_memory(COPY_ICON));
static CHECK_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(CHECK_ICON));
static SHOW_ICON_HANDLE: LazyLock<SvgHandle> = LazyLock::new(|| SvgHandle::from_memory(SHOW_ICON));
static HIDE_ICON_HANDLE: LazyLock<SvgHandle> = LazyLock::new(|| SvgHandle::from_memory(HIDE_ICON));

pub(super) fn view_context_panels<'a>(
    state: &'a StudioState,
    project: &'a Project,
) -> Option<Element<'a, Message>> {
    let mut panels = Column::new().spacing(8).width(Fill);
    let mut has_panel = false;

    if let Some(script_context) = state.script_context_panel(project) {
        panels = panels.push(view_script_context_panel(script_context));
        has_panel = true;
    }

    if let Some(original_script) = state.original_script_panel(project) {
        panels = panels.push(view_original_script_panel(original_script));
        has_panel = true;
    }

    has_panel.then(|| container(panels).width(Fill).into())
}

fn view_script_context_panel(model: ScriptContextPanelModel) -> Element<'static, Message> {
    let mut body = Column::new().spacing(8).width(Fill);
    if model.panel.content_visible {
        if let Some(emotion) = model.emotion.filter(|value| !value.trim().is_empty()) {
            body = body.push(view_context_field("Emotion".to_string(), emotion));
        }

        for field in model.fields {
            body = body.push(view_context_field(field.label, field.value));
        }

        if let Some(submission) = model.submission {
            body = body.push(view_submission_id(submission));
        }
    }

    view_collapsible_panel(model.panel, body)
}

fn view_original_script_panel(model: OriginalScriptPanelModel) -> Element<'static, Message> {
    let mut body = Column::new().spacing(8).width(Fill);
    if model.panel.content_visible {
        body = body.push(
            text(model.script)
                .size(12)
                .line_height(iced::widget::text::LineHeight::Relative(1.35))
                .width(Fill),
        );
    }

    view_collapsible_panel(model.panel, body)
}

fn view_collapsible_panel(
    panel: CollapsiblePanelModel,
    body: Column<'static, Message>,
) -> Element<'static, Message> {
    let content = column![view_panel_header(&panel), body]
        .spacing(8)
        .width(Fill);

    container(content)
        .padding(10)
        .width(Fill)
        .style(subtle_panel_style)
        .into()
}

fn view_panel_header(panel: &CollapsiblePanelModel) -> Element<'static, Message> {
    row![
        text(panel.title.clone()).size(13).width(Fill),
        Space::with_width(Fill),
        view_toggle_button(panel),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill)
    .into()
}

fn view_toggle_button(panel: &CollapsiblePanelModel) -> Element<'static, Message> {
    let icon = match panel.toggle_icon {
        CollapsiblePanelToggleIcon::Show => SHOW_ICON_HANDLE.clone(),
        CollapsiblePanelToggleIcon::Hide => HIDE_ICON_HANDLE.clone(),
    };
    let button = button(view_svg_icon(icon, Color::from_rgb8(75, 85, 99)))
        .style(table_plain_button_style)
        .padding(0)
        .width(Length::Fixed(PANEL_TOGGLE_SIZE))
        .height(Length::Fixed(PANEL_TOGGLE_SIZE))
        .on_press(Message::ToggleContextPanel(panel.storage_key.clone()));

    tooltip(
        button,
        container(text(panel.toggle_tooltip.clone()).size(12)).padding(6),
        tooltip::Position::FollowCursor,
    )
    .style(container::rounded_box)
    .into()
}

fn view_context_field(label: String, value: String) -> Element<'static, Message> {
    column![
        text(label).size(11).color(Color::from_rgb8(100, 116, 139)),
        text(value).size(12).width(Fill),
    ]
    .spacing(2)
    .width(Fill)
    .into()
}

fn view_submission_id(submission: SubmissionIdModel) -> Element<'static, Message> {
    let icon = match submission.icon {
        SubmissionCopyIcon::Copy => COPY_ICON_HANDLE.clone(),
        SubmissionCopyIcon::Check => CHECK_ICON_HANDLE.clone(),
    };
    let label = match submission.icon {
        SubmissionCopyIcon::Copy => "Copy",
        SubmissionCopyIcon::Check => "Copied!",
    };
    let copy_button = button(
        row![
            view_svg_icon(icon, Color::from_rgb8(75, 85, 99)),
            text(label).size(12),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .style(table_plain_button_style)
    .padding([0, 4])
    .height(Length::Fixed(COPY_BUTTON_HEIGHT))
    .on_press(Message::CopySubmissionId(submission.clipboard_text.clone()));

    row![
        column![
            text(submission.label)
                .size(11)
                .color(Color::from_rgb8(100, 116, 139)),
            text(submission.submission_id).size(12).width(Fill),
        ]
        .spacing(2)
        .width(Fill),
        copy_button,
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill)
    .into()
}

fn view_svg_icon(icon: SvgHandle, color: Color) -> Element<'static, Message> {
    Svg::new(icon)
        .width(Length::Fixed(PANEL_ICON_SIZE))
        .height(Length::Fixed(PANEL_ICON_SIZE))
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}
