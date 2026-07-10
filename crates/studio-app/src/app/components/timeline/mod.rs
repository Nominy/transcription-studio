mod lane;
mod loop_strip;
mod ruler;
mod waveform;

pub(crate) use lane::lane_vertical_zoom_input_id;

use iced::mouse;
use iced::widget::{Column, container, mouse_area};
use iced::{Element, Fill};
use studio_app::EditorSession;
use studio_core::Project;

use super::super::Message;
use super::super::StudioState;
use super::super::styles::editor_band_style;
use super::create_region_dialog;
use super::timed_widget::view_timed;
const TIMELINE_WHEEL_LINE_PIXELS: f32 = 60.0;

pub(super) fn view_timeline<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let mut lanes = Column::new().spacing(0);
    let lane_views = state.timeline_lanes_snapshot_for_render(project, editor);
    let suppress_region_affordances = state
        .feedback_panel
        .as_ref()
        .is_some_and(|panel| panel.open);

    lanes = lanes.push(view_timed(
        "loop-strip",
        loop_strip::view_loop_strip(project, editor),
    ));

    for lane_view in lane_views.iter() {
        let hover = state
            .waveform_hit_hover
            .as_ref()
            .filter(|_| !suppress_region_affordances)
            .filter(|hover| hover.lane_id == lane_view.lane_id)
            .map(|hover| waveform::WaveformHoverOverlay {
                time_seconds: hover.hit.time_seconds,
                region_id: hover.hit.region_id.clone(),
                region_label: hover.region_label.clone(),
                region_tooltip: hover.region_tooltip.clone(),
                target: hover.hit.target,
            });
        let timeline_autoscroll_anchor_fraction =
            state.timeline_autoscroll_active_for_lane(&lane_view.lane_id);
        let magnifier = state
            .waveform_magnifier_overlay_for_lane(&lane_view.lane_id)
            .filter(|_| !suppress_region_affordances);
        let attached_create_region_popup = state
            .create_region_dialog
            .as_ref()
            .filter(|dialog| create_region_dialog::is_attached_transcription_card(dialog))
            .filter(|dialog| dialog.selected_lane_id == lane_view.lane_id)
            .map(|dialog| waveform::AttachedCreateRegionPopup {
                start_seconds: dialog.start_seconds,
                end_seconds: dialog.end_seconds,
                text: dialog.text.as_str(),
            });
        let vertical_zoom_input = state
            .lane_vertical_zoom_input
            .as_ref()
            .filter(|input| input.lane_id == lane_view.lane_id)
            .map(|input| input.value.as_str());
        lanes = lanes.push(view_timed(
            "timeline-lane",
            lane::view_timeline_lane(
                editor,
                lane_view,
                lane::TimelineLanePresentation {
                    hover,
                    suppress_region_affordances,
                    timeline_autoscroll_active: timeline_autoscroll_anchor_fraction,
                    magnifier,
                    attached_create_region_popup,
                    vertical_zoom_input,
                },
            ),
        ));
    }

    mouse_area(
        container(lanes)
            .padding(0)
            .width(Fill)
            .style(editor_band_style),
    )
    .on_scroll(|delta| {
        let (delta_x, delta_y) = timeline_wheel_delta_pixels(delta);
        Message::TimelineWheelScrolled { delta_x, delta_y }
    })
    .into()
}

fn timeline_wheel_delta_pixels(delta: mouse::ScrollDelta) -> (f32, f32) {
    match delta {
        mouse::ScrollDelta::Lines { x, y } => (
            x * TIMELINE_WHEEL_LINE_PIXELS,
            y * TIMELINE_WHEEL_LINE_PIXELS,
        ),
        mouse::ScrollDelta::Pixels { x, y } => (x, y),
    }
}

#[cfg(test)]
mod tests {
    use iced::mouse;

    use super::timeline_wheel_delta_pixels;

    #[test]
    fn wheel_lines_normalize_to_pixels_while_pixel_deltas_pass_through() {
        assert_eq!(
            timeline_wheel_delta_pixels(mouse::ScrollDelta::Lines { x: 1.5, y: -2.0 }),
            (90.0, -120.0)
        );
        assert_eq!(
            timeline_wheel_delta_pixels(mouse::ScrollDelta::Pixels { x: 1.25, y: -0.5 }),
            (1.25, -0.5)
        );
    }
}
