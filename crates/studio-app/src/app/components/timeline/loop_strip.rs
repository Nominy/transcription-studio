use iced::mouse;
use iced::widget::{Space, container, mouse_area, responsive, row, stack, text};
use iced::{Element, Fill, Length, Size};
use studio_app::{EditorSession, LoopStripClientMetrics};
use studio_core::Project;

use super::super::super::Message;
use super::super::super::styles::{loop_handle_style, loop_range_style, loop_strip_style, portion};

const LOOP_STRIP_HEIGHT: f32 = 24.0;
const LOOP_LABEL_WIDTH: f32 = 176.0;

pub(super) fn view_loop_strip<'a>(
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let strip = editor.loop_strip(project);
    let viewport = editor.viewport;
    let duration = editor.timeline_bounds(project).duration_seconds();
    let label = text(format_loop_label(strip.has_range, strip.is_enabled)).size(12);

    let body = responsive(move |size: Size| {
        let width = size.width.max(1.0);
        let move_strip = strip;
        let metrics = LoopStripClientMetrics::for_viewport(viewport, duration, width);
        let content: Element<'a, Message> =
            stack(vec![view_loop_range(strip), view_loop_handles(strip)])
                .height(Length::Fixed(LOOP_STRIP_HEIGHT))
                .width(Fill)
                .into();

        mouse_area(
            container(content)
                .width(Fill)
                .height(Length::Fixed(LOOP_STRIP_HEIGHT)),
        )
        .on_move(move |point| {
            Message::LoopStripHovered(move_strip.hit_test_client_x(
                viewport,
                Some(metrics),
                point.x as f64,
            ))
        })
        .on_press(Message::LoopStripPressed)
        .on_release(Message::LoopStripReleased)
        .interaction(mouse::Interaction::Pointer)
        .into()
    });

    row![
        container(label)
            .padding([0, 12])
            .width(Length::Fixed(LOOP_LABEL_WIDTH))
            .height(Length::Fixed(LOOP_STRIP_HEIGHT))
            .style(loop_strip_style),
        container(body)
            .width(Fill)
            .height(Length::Fixed(LOOP_STRIP_HEIGHT))
            .style(loop_strip_style),
    ]
    .spacing(0)
    .into()
}

fn format_loop_label(has_range: bool, is_enabled: bool) -> &'static str {
    if has_range && !is_enabled {
        "Loop (off)"
    } else {
        "Loop"
    }
}

fn view_loop_range<'a>(strip: studio_app::LoopStripView) -> Element<'a, Message> {
    if !strip.has_range || strip.width_percent <= 0.0 {
        return Space::with_width(Fill).into();
    }

    let is_enabled = strip.is_enabled;
    row![
        Space::with_width(Length::FillPortion(portion(strip.left_percent))),
        container(text(""))
            .height(Length::Fixed(LOOP_STRIP_HEIGHT))
            .width(Length::FillPortion(portion(strip.width_percent)))
            .style(move |_| loop_range_style(is_enabled)),
        Space::with_width(Length::FillPortion(portion(
            100.0 - strip.left_percent - strip.width_percent
        ))),
    ]
    .width(Fill)
    .into()
}

fn view_loop_handles<'a>(strip: studio_app::LoopStripView) -> Element<'a, Message> {
    if !strip.has_range || strip.width_percent <= 0.0 {
        return Space::with_width(Fill).into();
    }

    row![
        Space::with_width(Length::FillPortion(portion(strip.left_percent))),
        container(text(""))
            .height(Length::Fixed(LOOP_STRIP_HEIGHT))
            .width(Length::Fixed(4.0))
            .style(loop_handle_style),
        Space::with_width(Length::FillPortion(portion(strip.width_percent))),
        container(text(""))
            .height(Length::Fixed(LOOP_STRIP_HEIGHT))
            .width(Length::Fixed(4.0))
            .style(loop_handle_style),
        Space::with_width(Length::FillPortion(portion(
            100.0 - strip.left_percent - strip.width_percent
        ))),
    ]
    .width(Fill)
    .into()
}
