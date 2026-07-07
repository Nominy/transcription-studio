use std::{hash::Hasher, sync::LazyLock};

use iced::widget::svg::{self, Handle as SvgHandle, Svg};
use iced::widget::{
    Space, button, column, container, lazy, responsive, row, text, text_input, tooltip,
};
use iced::{Color, Element, Fill, Length, Size};
use studio_app::{EditorSession, TimelineLaneView};

use super::super::super::Message;
use super::super::super::state::WaveformMagnifierOverlay;
use super::super::super::styles::{
    color_for_token, lane_label_style, lane_vertical_zoom_slider_rail_style,
    lane_vertical_zoom_slider_thumb_style, outline_button_style, waveform_backdrop_style,
};
use super::super::drag_capture::view_vertical_drag_capture;
use super::super::timed_widget::view_timed;

const LANE_EDGE_BORDER_WIDTH: f32 = 2.5;
const LANE_CONTROL_RAIL_WIDTH: f32 = 176.0;
const LANE_VERTICAL_ZOOM_SLIDER_WIDTH: f32 = 8.0;
const LANE_VERTICAL_ZOOM_SLIDER_THUMB_SIZE: f32 = 7.0;
const LANE_VERTICAL_ZOOM_SLIDER_MIN: f32 = 0.3;
const LANE_VERTICAL_ZOOM_SLIDER_MAX: f32 = 1000.0;
const LANE_ICON_BUTTON_WIDTH: f32 = 28.0;
const LANE_ICON_BUTTON_HEIGHT: f32 = 28.0;
const LANE_ICON_SIZE: f32 = 12.0;
const LANE_ICON_EYE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>"#;
const LANE_ICON_EYE_OFF: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3l18 18"/><path d="M10.6 10.6a2 2 0 0 0 2.8 2.8"/><path d="M9.9 4.3A10.8 10.8 0 0 1 12 4c6.5 0 10 8 10 8a18.5 18.5 0 0 1-2.9 4.2"/><path d="M6.6 6.6C3.6 8.7 2 12 2 12s3.5 8 10 8c1.6 0 3-.3 4.3-.9"/></svg>"#;
const LANE_ICON_MIC: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><path d="M12 19v3"/><path d="M8 22h8"/></svg>"#;
const LANE_ICON_MIC_OFF: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3l18 18"/><path d="M9 9v3a3 3 0 0 0 5.1 2.1"/><path d="M15 9.3V5a3 3 0 0 0-5.1-2.1"/><path d="M19 10v2a7 7 0 0 1-.8 3.2"/><path d="M5 10v2a7 7 0 0 0 10.7 6"/><path d="M12 19v3"/><path d="M8 22h8"/></svg>"#;
static LANE_ICON_EYE_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(LANE_ICON_EYE));
static LANE_ICON_EYE_OFF_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(LANE_ICON_EYE_OFF));
static LANE_ICON_MIC_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(LANE_ICON_MIC));
static LANE_ICON_MIC_OFF_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(LANE_ICON_MIC_OFF));

struct LaneControlRailDependency {
    lane_id: String,
    label: String,
    is_collapsed: bool,
    is_muted: bool,
    track_height_bits: u32,
    vertical_zoom_bits: u32,
    render_solo_controls: bool,
    render_vertical_zoom_slider: bool,
    collapse_tooltip: String,
    solo_aria_label: String,
    vertical_zoom_input: Option<String>,
}

impl LaneControlRailDependency {
    fn from_lane(lane: &TimelineLaneView, vertical_zoom_input: Option<&str>) -> Self {
        Self {
            lane_id: lane.lane_id.clone(),
            label: lane.label.clone(),
            is_collapsed: lane.is_collapsed,
            is_muted: lane.is_muted,
            track_height_bits: lane.track_height.to_bits(),
            vertical_zoom_bits: lane.vertical_zoom.to_bits(),
            render_solo_controls: lane.viewport_state.render_solo_controls,
            render_vertical_zoom_slider: lane.viewport_state.render_vertical_zoom_slider,
            collapse_tooltip: lane.chrome.collapse_tooltip.clone(),
            solo_aria_label: lane.chrome.solo_aria_label.clone(),
            vertical_zoom_input: vertical_zoom_input.map(str::to_string),
        }
    }

    fn track_height(&self) -> f32 {
        f32::from_bits(self.track_height_bits)
    }

    fn vertical_zoom(&self) -> f32 {
        f32::from_bits(self.vertical_zoom_bits)
    }
}

impl std::hash::Hash for LaneControlRailDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.lane_id.hash(state);
        self.label.hash(state);
        self.is_collapsed.hash(state);
        self.is_muted.hash(state);
        self.track_height_bits.hash(state);
        self.vertical_zoom_bits.hash(state);
        self.render_solo_controls.hash(state);
        self.render_vertical_zoom_slider.hash(state);
        self.collapse_tooltip.hash(state);
        self.solo_aria_label.hash(state);
        self.vertical_zoom_input.hash(state);
    }
}

pub(crate) fn lane_vertical_zoom_input_id() -> text_input::Id {
    text_input::Id::new("lane-vertical-zoom-input")
}

pub(super) fn view_timeline_lane<'a>(
    editor: &'a EditorSession,
    lane: &TimelineLaneView,
    hover: Option<super::waveform::WaveformHoverOverlay>,
    suppress_region_affordances: bool,
    timeline_autoscroll_active: Option<f32>,
    magnifier: Option<WaveformMagnifierOverlay>,
    attached_create_region_popup: Option<super::waveform::AttachedCreateRegionPopup<'a>>,
    vertical_zoom_input: Option<&str>,
) -> Element<'a, Message> {
    let track_height = lane.track_height;
    let color_token = lane.color_token.clone();
    let control_rail = view_lane_control_rail(lane, vertical_zoom_input);
    let waveform: Element<'a, Message> = if lane.is_collapsed {
        let viewport_height = lane.viewport_state.container_height;
        container(Space::with_height(Length::Fixed(viewport_height)))
            .width(Fill)
            .height(Length::Fixed(viewport_height))
            .style(waveform_backdrop_style)
            .into()
    } else {
        super::waveform::view_lane_waveform(
            editor,
            lane,
            hover,
            suppress_region_affordances,
            timeline_autoscroll_active,
            magnifier,
            attached_create_region_popup,
        )
    };
    let waveform = view_timed("lane-waveform", waveform);

    row![
        lane_edge_border(color_token.clone(), track_height),
        control_rail,
        waveform,
        lane_edge_border(color_token, track_height),
    ]
    .spacing(0)
    .into()
}

fn lane_edge_border<'a>(color_token: String, track_height: f32) -> Element<'a, Message> {
    container(text(""))
        .width(Length::Fixed(LANE_EDGE_BORDER_WIDTH))
        .height(Length::Fixed(track_height))
        .style(move |_| {
            iced::widget::container::Style::default().background(color_for_token(&color_token))
        })
        .into()
}

fn view_lane_control_rail(
    lane: &TimelineLaneView,
    vertical_zoom_input: Option<&str>,
) -> Element<'static, Message> {
    view_lazy_lane_control_rail(LaneControlRailDependency::from_lane(
        lane,
        vertical_zoom_input,
    ))
}

fn view_lazy_lane_control_rail(dependency: LaneControlRailDependency) -> Element<'static, Message> {
    lazy(dependency, view_lane_control_rail_content).into()
}

fn view_lane_control_rail_content(
    dependency: &LaneControlRailDependency,
) -> Element<'static, Message> {
    let track_height = dependency.track_height();
    let label = container(view_lane_label(dependency))
        .padding([8, 12])
        .width(Fill)
        .height(Length::Fixed(track_height));

    let rail_content = if dependency.render_vertical_zoom_slider {
        row![label, view_lane_vertical_zoom_slider(dependency)].spacing(0)
    } else {
        row![label].spacing(0)
    };

    container(rail_content)
        .width(Length::Fixed(LANE_CONTROL_RAIL_WIDTH))
        .height(Length::Fixed(track_height))
        .style(lane_label_style)
        .into()
}

fn view_lane_label(dependency: &LaneControlRailDependency) -> Element<'static, Message> {
    let collapse_id = dependency.lane_id.clone();
    let visibility_icon = if dependency.is_collapsed {
        LANE_ICON_EYE_HANDLE.clone()
    } else {
        LANE_ICON_EYE_OFF_HANDLE.clone()
    };

    let mut controls = column![
        row![
            text(dependency.label.clone()).size(14).width(Fill),
            lane_svg_icon_button(
                visibility_icon,
                dependency.collapse_tooltip.clone(),
                Some(Message::ToggleLaneCollapse(collapse_id)),
            ),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .height(Fill)
    .spacing(8);

    if let Some(value) = &dependency.vertical_zoom_input {
        controls = controls.push(
            row![
                text_input("zoom", value)
                    .id(lane_vertical_zoom_input_id())
                    .on_input(Message::LaneVerticalZoomInputChanged)
                    .on_submit(Message::SubmitLaneVerticalZoomInput)
                    .size(12)
                    .padding(4)
                    .width(Length::Fixed(64.0)),
                text("x").size(12),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        );
    }

    if dependency.render_solo_controls {
        let mute_id = dependency.lane_id.clone();
        let mute_icon = if dependency.is_muted {
            LANE_ICON_MIC_OFF_HANDLE.clone()
        } else {
            LANE_ICON_MIC_HANDLE.clone()
        };
        controls = controls
            .push(Space::with_height(Fill))
            .push(
                row![
                    text("Mute").size(12).width(Fill),
                    lane_svg_icon_button(
                        mute_icon,
                        dependency.solo_aria_label.clone(),
                        Some(Message::ToggleLaneMute(mute_id)),
                    ),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .push(Space::with_height(Length::Fixed(
                LANE_VERTICAL_ZOOM_SLIDER_THUMB_SIZE,
            )));
    }

    controls.into()
}

fn view_lane_vertical_zoom_slider(
    dependency: &LaneControlRailDependency,
) -> Element<'static, Message> {
    let lane_id = dependency.lane_id.clone();
    let hover_lane_id = lane_id.clone();
    let press_lane_id = lane_id.clone();
    let top_fraction = lane_vertical_zoom_slider_top_fraction(dependency.vertical_zoom());
    let track_height = dependency.track_height();
    let handle = responsive(move |size: Size| {
        let height = size.height.max(1.0);
        let available_height = (height - LANE_VERTICAL_ZOOM_SLIDER_THUMB_SIZE).max(0.0);
        let thumb_top_offset = available_height * top_fraction;
        let thumb_bottom_offset = available_height - thumb_top_offset;
        let move_lane_id = hover_lane_id.clone();
        let press_lane_id = press_lane_id.clone();
        view_vertical_drag_capture(
            container(view_lane_vertical_zoom_slider_body(
                thumb_top_offset,
                thumb_bottom_offset,
            ))
            .width(Length::Fixed(LANE_VERTICAL_ZOOM_SLIDER_WIDTH))
            .height(Length::Fixed(height))
            .style(lane_vertical_zoom_slider_rail_style)
            .into(),
            move |fraction, height| Message::LaneVerticalZoomSliderPointerMoved {
                lane_id: move_lane_id.clone(),
                fraction,
                height,
            },
            Message::LaneVerticalZoomSliderPressed(press_lane_id.clone()),
            Message::LaneVerticalZoomSliderReleased,
            None,
        )
        .into()
    });

    container(handle)
        .width(Length::Fixed(LANE_VERTICAL_ZOOM_SLIDER_WIDTH))
        .height(Length::Fixed(track_height))
        .into()
}

fn lane_vertical_zoom_slider_top_fraction(vertical_zoom: f32) -> f32 {
    let range = LANE_VERTICAL_ZOOM_SLIDER_MAX - LANE_VERTICAL_ZOOM_SLIDER_MIN;
    if !vertical_zoom.is_finite() || range <= 0.0 {
        return 0.0;
    }

    ((LANE_VERTICAL_ZOOM_SLIDER_MAX
        - vertical_zoom.clamp(LANE_VERTICAL_ZOOM_SLIDER_MIN, LANE_VERTICAL_ZOOM_SLIDER_MAX))
        / range)
        .clamp(0.0, 1.0)
}

fn view_lane_vertical_zoom_slider_body<'a>(
    thumb_top_offset: f32,
    thumb_bottom_offset: f32,
) -> Element<'a, Message> {
    column![
        Space::with_height(Length::Fixed(thumb_top_offset)),
        container(text(""))
            .width(Length::Fixed(LANE_VERTICAL_ZOOM_SLIDER_THUMB_SIZE))
            .height(Length::Fixed(LANE_VERTICAL_ZOOM_SLIDER_THUMB_SIZE))
            .style(lane_vertical_zoom_slider_thumb_style),
        Space::with_height(Length::Fixed(thumb_bottom_offset)),
    ]
    .align_x(iced::Alignment::Center)
    .width(Fill)
    .height(Fill)
    .into()
}

fn lane_svg_icon_button<'a>(
    icon: SvgHandle,
    label: String,
    message: Option<Message>,
) -> Element<'a, Message> {
    let icon = Svg::new(icon)
        .width(Length::Fixed(LANE_ICON_SIZE))
        .height(Length::Fixed(LANE_ICON_SIZE))
        .style(|_, _| svg::Style {
            color: Some(Color::from_rgb8(75, 85, 99)),
        });

    tooltip(
        button(icon)
            .style(outline_button_style)
            .on_press_maybe(message)
            .padding(0)
            .width(Length::Fixed(LANE_ICON_BUTTON_WIDTH))
            .height(Length::Fixed(LANE_ICON_BUTTON_HEIGHT)),
        container(text(label).size(12)).padding(6),
        tooltip::Position::FollowCursor,
    )
    .style(container::rounded_box)
    .into()
}
