use std::{
    hash::Hasher,
    sync::{Arc, LazyLock},
};

use iced::mouse;
use iced::widget::svg::{self, Handle as SvgHandle, Svg};
use iced::widget::{
    Row, Space, button, column, container, lazy, mouse_area, opaque, responsive, row, stack, text,
    text_input, tooltip,
};
use iced::{Color, Element, Fill, Length, Size};
use studio_app::{
    EditorSession, TimeRange, TimelineLaneView, TimelineOverlayView, TimelineScrollbarView,
    WaveformBlock, format_mm_ss,
};
use studio_waveform::WaveformHitTarget;

use super::super::super::Message;
use super::super::super::state::WaveformMagnifierOverlay;
use super::super::super::styles::{
    action_menu_popup_style, hover_cursor_style, hover_label_style, overlay_range_style, portion,
    table_plain_button_style, waveform_backdrop_style, waveform_magnifier_center_line_style,
    waveform_magnifier_style, waveform_region_label_edit_tooltip_style,
    waveform_region_timestamp_label_style,
};
use super::super::timed_widget::view_timed;
use super::super::wavesurfer_canvas::{
    view_waveform_playhead_canvas, view_waveform_scrollbar_canvas,
    view_waveform_selected_region_border_canvas, view_wavesurfer_line_waveform_window_canvas,
    view_wavesurfer_region_fill_window_canvas,
    view_wavesurfer_segment_waveforms_with_region_fill_canvas,
};
use super::ruler;

const SCROLLBAR_THUMB_HEIGHT: f32 = 7.0;
const SCROLLBAR_BOTTOM_OFFSET: f32 = 4.0;
const SCROLLBAR_THUMB_MIN_PERCENT: f32 = 2.4;
const SYNTHETIC_REGION_PEAK_SCALE: f64 = 0.045;
const REGION_HANDLE_HIT_ZONE_WIDTH: f32 = 12.0;
const REGION_TIMESTAMP_TOOLTIP_TOP_OFFSET: f32 = -22.0;
const REGION_TIMESTAMP_TOOLTIP_HEIGHT: f32 = REGION_TIMESTAMP_TOOLTIP_TOP_OFFSET.abs();
const REGION_LABEL_TOOLTIP_WIDTH: f32 = 300.0;
const REGION_LABEL_TOOLTIP_GAP: f32 = 12.0;
const REGION_EDIT_PENCIL_ICON_SIZE: f32 = 14.0;
const TIMELINE_WAVEFORM_LAYER_CAPACITY: usize = 11;
const ATTACHED_CREATE_REGION_POPUP_WIDTH: f32 = 384.0;
const ATTACHED_CREATE_REGION_POPUP_TOP_OFFSET: f32 = 18.0;
const WAVEFORM_MAGNIFIER_INSET_PX: f32 = 6.0;
const WAVEFORM_MAGNIFIER_HEADER_HEIGHT: f32 = 22.0;
const WAVEFORM_MAGNIFIER_MIN_WIDTH_PX: f32 = 72.0;
const CREATE_PREVIEW_SEGMENT_ID: &str = "__studio_create_preview__";
const REGION_EDIT_PENCIL_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/><path d="m15 5 4 4"/></svg>"#;
static REGION_EDIT_PENCIL_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(REGION_EDIT_PENCIL_ICON));

#[derive(Clone, Copy, Debug)]
struct WaveformLaneMetrics {
    body_height: f32,
    vertical_scale: f32,
}

struct WaveformBodyData<'a> {
    lane_id: String,
    color_token: String,
    selected_segment_id: Option<String>,
    suppress_region_affordances: bool,
    blocks: Arc<[WaveformBlock]>,
    strip_blocks: Arc<[WaveformBlock]>,
    overlay: Option<TimelineOverlayView>,
    scrollbar: TimelineScrollbarView,
    metrics: WaveformLaneMetrics,
    playhead_percent: f32,
    hover: Option<WaveformHoverOverlay>,
    magnifier: Option<WaveformMagnifierOverlay>,
    timeline_autoscroll_anchor_fraction: Option<f32>,
    attached_create_region_popup: Option<AttachedCreateRegionPopup<'a>>,
}

struct SyntheticBaseWaveformDependency {
    blocks: Arc<[WaveformBlock]>,
    color_token: String,
    metrics: WaveformLaneMetrics,
}

struct EngineWaveformStripDependency {
    scroll_strip_peaks: Arc<[f32]>,
    color_token: String,
    metrics: WaveformLaneMetrics,
    source_start_seconds: f64,
    source_end_seconds: f64,
    visible_start_seconds: f64,
    visible_end_seconds: f64,
}

struct EngineWaveformMagnifierDependency {
    scroll_strip_peaks: Arc<[f32]>,
    color_token: String,
    body_height: f32,
    vertical_scale: f32,
    source_start_seconds: f64,
    source_end_seconds: f64,
    visible_start_seconds: f64,
    visible_end_seconds: f64,
}

#[derive(Clone, Copy, Debug)]
struct WaveformMagnifierLayout {
    left_px: f32,
    top_px: f32,
    width_px: f32,
    height_px: f32,
    waveform_height_px: f32,
}

struct EngineRegionFillDependency {
    blocks: Arc<[WaveformBlock]>,
    color_token: String,
    metrics: WaveformLaneMetrics,
    visible_start_seconds: f64,
    visible_end_seconds: f64,
}

struct SelectedRegionBorderDependency {
    blocks: Arc<[WaveformBlock]>,
    selected_segment_id: String,
    color_token: String,
    metrics: WaveformLaneMetrics,
}

struct RegionAffordanceOverlayDependency {
    blocks: Arc<[WaveformBlock]>,
    selected_segment_id: Option<String>,
    hovered_segment_id: Option<String>,
    color_token: String,
    viewport: TimeRange,
    metrics: WaveformLaneMetrics,
    suppress_region_affordances: bool,
}

struct EngineWaveformData {
    visible_start_seconds: f64,
    visible_end_seconds: f64,
    scroll_strip_start_seconds: f64,
    scroll_strip_end_seconds: f64,
    scroll_strip_peaks: Arc<[f32]>,
}

impl std::hash::Hash for SyntheticBaseWaveformDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.blocks.len().hash(state);
        for block in self.blocks.iter() {
            block.segment_id.hash(state);
            block.left_percent.to_bits().hash(state);
            block.width_percent.to_bits().hash(state);
            (Arc::as_ptr(&block.peaks) as *const f32 as usize).hash(state);
            block.peaks.len().hash(state);
        }
        self.color_token.hash(state);
        self.metrics.body_height.to_bits().hash(state);
        self.metrics.vertical_scale.to_bits().hash(state);
    }
}

impl std::hash::Hash for EngineWaveformStripDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.scroll_strip_peaks).hash(state);
        self.scroll_strip_peaks.len().hash(state);
        self.color_token.hash(state);
        self.metrics.body_height.to_bits().hash(state);
        self.metrics.vertical_scale.to_bits().hash(state);
        self.source_start_seconds.to_bits().hash(state);
        self.source_end_seconds.to_bits().hash(state);
        self.visible_start_seconds.to_bits().hash(state);
        self.visible_end_seconds.to_bits().hash(state);
    }
}

impl std::hash::Hash for EngineWaveformMagnifierDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.scroll_strip_peaks).hash(state);
        self.scroll_strip_peaks.len().hash(state);
        self.color_token.hash(state);
        self.body_height.to_bits().hash(state);
        self.vertical_scale.to_bits().hash(state);
        self.source_start_seconds.to_bits().hash(state);
        self.source_end_seconds.to_bits().hash(state);
        self.visible_start_seconds.to_bits().hash(state);
        self.visible_end_seconds.to_bits().hash(state);
    }
}

impl std::hash::Hash for EngineRegionFillDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.blocks.len().hash(state);
        for block in self.blocks.iter() {
            block.segment_id.hash(state);
            block.start_seconds.to_bits().hash(state);
            block.end_seconds.to_bits().hash(state);
        }
        self.color_token.hash(state);
        self.metrics.body_height.to_bits().hash(state);
        self.visible_start_seconds.to_bits().hash(state);
        self.visible_end_seconds.to_bits().hash(state);
    }
}

impl std::hash::Hash for SelectedRegionBorderDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.blocks.len().hash(state);
        for block in self.blocks.iter() {
            block.segment_id.hash(state);
            block.left_percent.to_bits().hash(state);
            block.width_percent.to_bits().hash(state);
        }
        self.selected_segment_id.hash(state);
        self.color_token.hash(state);
        self.metrics.body_height.to_bits().hash(state);
    }
}

impl std::hash::Hash for RegionAffordanceOverlayDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.blocks.len().hash(state);
        for block in self.blocks.iter() {
            block.segment_id.hash(state);
            block.left_percent.to_bits().hash(state);
            block.width_percent.to_bits().hash(state);
            if self.selected_segment_id.as_deref() == Some(block.segment_id.as_str())
                || self.hovered_segment_id.as_deref() == Some(block.segment_id.as_str())
            {
                block.tooltip.hash(state);
            }
        }
        self.selected_segment_id.hash(state);
        self.hovered_segment_id.hash(state);
        self.color_token.hash(state);
        self.viewport.start_seconds.to_bits().hash(state);
        self.viewport.end_seconds.to_bits().hash(state);
        self.metrics.body_height.to_bits().hash(state);
        self.metrics.vertical_scale.to_bits().hash(state);
        self.suppress_region_affordances.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct WaveformHoverOverlay {
    pub(super) time_seconds: f64,
    pub(super) region_id: Option<String>,
    pub(super) region_label: Option<String>,
    pub(super) region_tooltip: Option<String>,
    pub(super) target: WaveformHitTarget,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AttachedCreateRegionPopup<'a> {
    pub(super) start_seconds: f64,
    pub(super) end_seconds: f64,
    pub(super) text: &'a str,
}

pub(super) fn view_lane_waveform<'a>(
    editor: &'a EditorSession,
    lane: &TimelineLaneView,
    hover: Option<WaveformHoverOverlay>,
    suppress_region_affordances: bool,
    timeline_autoscroll_anchor_fraction: Option<f32>,
    magnifier: Option<WaveformMagnifierOverlay>,
    attached_create_region_popup: Option<AttachedCreateRegionPopup<'a>>,
) -> Element<'a, Message> {
    let metrics = WaveformLaneMetrics {
        body_height: (lane.track_height - ruler::RULER_HEIGHT).max(1.0),
        vertical_scale: lane.vertical_zoom,
    };
    let selected_segment_id = editor.selected_segment_id.clone();
    let viewport_duration =
        (editor.viewport.end_seconds - editor.viewport.start_seconds).max(0.001);
    let playhead_percent =
        (((editor.playhead_seconds - editor.viewport.start_seconds) / viewport_duration) * 100.0)
            .clamp(0.0, 100.0) as f32;
    let body_data = WaveformBodyData {
        lane_id: lane.lane_id.clone(),
        color_token: lane.color_token.clone(),
        selected_segment_id,
        suppress_region_affordances,
        blocks: Arc::clone(&lane.blocks),
        strip_blocks: Arc::clone(&lane.strip_blocks),
        overlay: lane.overlay,
        scrollbar: lane.scrollbar,
        metrics,
        playhead_percent,
        hover,
        magnifier,
        timeline_autoscroll_anchor_fraction,
        attached_create_region_popup,
    };

    let waveform_body: Element<'a, Message> = if let Some(model) = lane.waveform_model.as_ref() {
        let channel = model.channels.first();
        let engine = EngineWaveformData {
            visible_start_seconds: model.visible_start_seconds,
            visible_end_seconds: model.visible_end_seconds,
            scroll_strip_start_seconds: channel
                .map(|channel| channel.scroll_strip_start_seconds)
                .unwrap_or(model.visible_start_seconds),
            scroll_strip_end_seconds: channel
                .map(|channel| channel.scroll_strip_end_seconds)
                .unwrap_or(model.visible_end_seconds),
            scroll_strip_peaks: channel
                .map(|channel| Arc::clone(&channel.scroll_strip_peaks))
                .unwrap_or_else(|| Arc::<[f32]>::from(Vec::new())),
        };
        view_engine_waveform_body(editor, body_data, engine)
    } else {
        view_synthetic_waveform_body(editor, body_data)
    };

    container(
        column![
            view_timed("lane-ruler", ruler::view_lane_time_ruler(editor),),
            view_timed("lane-waveform-body", waveform_body)
        ]
        .spacing(0),
    )
    .width(Fill)
    .height(Length::Fixed(lane.track_height))
    .style(waveform_backdrop_style)
    .into()
}

fn view_synthetic_waveform_body<'a>(
    editor: &'a EditorSession,
    data: WaveformBodyData<'a>,
) -> Element<'a, Message> {
    let viewport = editor.viewport;
    let hit_area = responsive(move |size: Size| {
        let width = size.width.max(1.0);
        let hover_lane_id = data.lane_id.clone();
        let press_lane_id = data.lane_id.clone();
        let middle_press_lane_id = data.lane_id.clone();
        let release_lane_id = data.lane_id.clone();
        let hit_blocks = Arc::clone(&data.blocks);
        let selected_segment_id =
            visible_segment_id_for_blocks(&data.blocks, data.selected_segment_id.as_deref());
        let hovered_segment_id = visible_segment_id_for_blocks(
            &data.blocks,
            data.hover
                .as_ref()
                .and_then(|hover| hover.region_id.as_deref()),
        );
        let selected_segment_id = active_region_segment_id_for_blocks(
            &data.blocks,
            selected_segment_id.as_deref(),
            hovered_segment_id.as_deref(),
        );
        let mut layers: Vec<Element<'a, Message>> =
            Vec::with_capacity(TIMELINE_WAVEFORM_LAYER_CAPACITY);
        layers.push(view_lazy_synthetic_base_waveform(
            SyntheticBaseWaveformDependency {
                blocks: Arc::clone(&data.blocks),
                color_token: data.color_token.clone(),
                metrics: data.metrics,
            },
        ));
        if let Some(selected_segment_id) = selected_segment_id.clone() {
            layers.push(view_lazy_selected_region_border_overlay(
                SelectedRegionBorderDependency {
                    blocks: Arc::clone(&data.blocks),
                    selected_segment_id,
                    color_token: data.color_token.clone(),
                    metrics: data.metrics,
                },
            ));
        }
        if !data.suppress_region_affordances
            && (selected_segment_id.is_some() || hovered_segment_id.is_some())
        {
            layers.push(view_lazy_region_affordance_overlay(
                RegionAffordanceOverlayDependency {
                    blocks: Arc::clone(&data.blocks),
                    selected_segment_id,
                    hovered_segment_id,
                    color_token: data.color_token.clone(),
                    viewport,
                    metrics: data.metrics,
                    suppress_region_affordances: data.suppress_region_affordances,
                },
            ));
        }
        if let Some(overlay) = data.overlay {
            layers.push(view_timeline_overlay(overlay, data.metrics.body_height));
        }
        layers.push(view_timed(
            "waveform-playhead",
            view_waveform_playhead_canvas(data.playhead_percent, data.metrics.body_height),
        ));
        if let Some(cursor) =
            view_hover_cursor(viewport, data.hover.as_ref(), data.metrics.body_height)
        {
            layers.push(cursor);
        }
        if let Some(magnifier) = data.magnifier {
            let layout = waveform_magnifier_layout(magnifier, width, data.metrics.body_height);
            layers.push(view_waveform_magnifier(
                magnifier,
                layout,
                None,
                data.metrics.body_height,
            ));
        }
        if let Some(anchor_fraction) = data.timeline_autoscroll_anchor_fraction {
            layers.push(view_timeline_autoscroll_marker(
                anchor_fraction,
                data.metrics.body_height,
            ));
        }
        if let Some(popup) = data.attached_create_region_popup {
            layers.push(view_attached_create_region_popup(
                viewport,
                popup,
                data.metrics.body_height,
            ));
        }
        layers.push(view_timed(
            "waveform-scrollbar",
            view_waveform_scrollbar(data.scrollbar, data.metrics.body_height),
        ));
        let content: Element<'a, Message> = stack(layers)
            .height(Length::Fixed(data.metrics.body_height))
            .width(Fill)
            .into();
        let interaction = waveform_interaction(
            data.timeline_autoscroll_anchor_fraction.is_some(),
            data.hover.as_ref(),
        );

        mouse_area(
            container(content)
                .width(Fill)
                .height(Length::Fixed(data.metrics.body_height)),
        )
        .on_move(move |point| {
            let fraction = (point.x / width).clamp(0.0, 1.0);
            Message::WaveformHitHovered(
                hover_lane_id.clone(),
                synthetic_hit_test(&hit_blocks, viewport, fraction, width),
            )
        })
        .on_exit(Message::WaveformHitExited(release_lane_id.clone()))
        .on_press(Message::WaveformHitPressed(press_lane_id))
        .on_release(Message::WaveformHitReleased(release_lane_id))
        .on_middle_press(Message::TimelineAutoscrollPressed(middle_press_lane_id))
        .on_middle_release(Message::TimelineAutoscrollReleased)
        .interaction(interaction)
        .into()
    });

    container(hit_area)
        .height(Length::Fixed(data.metrics.body_height))
        .width(Fill)
        .into()
}

fn view_region_affordance_overlay<'a>(
    blocks: &'a [WaveformBlock],
    selected_segment_id: Option<&str>,
    hovered_segment_id: Option<&str>,
    color_token: &'a str,
    viewport: TimeRange,
    metrics: WaveformLaneMetrics,
    suppress_region_affordances: bool,
) -> Element<'static, Message> {
    if suppress_region_affordances {
        return Space::with_width(Fill)
            .height(Length::Fixed(metrics.body_height))
            .into();
    }

    let mut row = Row::new().spacing(0).width(Fill);
    let mut cursor_percent = 0.0_f32;
    let mut pending_space_portions = 0_u16;
    let mut has_affordance = false;

    for block in blocks {
        let gap = (block.left_percent - cursor_percent).max(0.0);
        if gap > 0.0 {
            pending_space_portions = pending_space_portions.saturating_add(portion(gap));
        }
        let is_selected = selected_segment_id == Some(block.segment_id.as_str());
        let is_hovered = hovered_segment_id == Some(block.segment_id.as_str());
        let next_cursor = (block.left_percent + block.width_percent).min(100.0);
        if is_selected || is_hovered {
            if pending_space_portions > 0 {
                row = row.push(Space::with_width(Length::FillPortion(
                    pending_space_portions,
                )));
                pending_space_portions = 0;
            }
            row = row.push(view_waveform_region_affordance_block(
                block,
                color_token,
                viewport,
                metrics,
            ));
            has_affordance = true;
        } else {
            pending_space_portions =
                pending_space_portions.saturating_add(portion(block.width_percent));
        }
        cursor_percent = next_cursor;
    }
    if cursor_percent < 100.0 {
        pending_space_portions =
            pending_space_portions.saturating_add(portion(100.0 - cursor_percent));
    }
    if !has_affordance {
        return Space::with_width(Fill)
            .height(Length::Fixed(metrics.body_height))
            .into();
    }
    if pending_space_portions > 0 {
        row = row.push(Space::with_width(Length::FillPortion(
            pending_space_portions,
        )));
    }

    row.into()
}

fn visible_segment_id_for_blocks(
    blocks: &[WaveformBlock],
    segment_id: Option<&str>,
) -> Option<String> {
    let segment_id = segment_id?;
    blocks
        .iter()
        .any(|block| block.segment_id.as_str() == segment_id)
        .then(|| segment_id.to_string())
}

fn active_region_segment_id_for_blocks(
    blocks: &[WaveformBlock],
    selected_segment_id: Option<&str>,
    hovered_segment_id: Option<&str>,
) -> Option<String> {
    visible_segment_id_for_blocks(blocks, hovered_segment_id)
        .or_else(|| visible_segment_id_for_blocks(blocks, selected_segment_id))
        .or_else(|| create_preview_segment_id_for_blocks(blocks))
}

fn create_preview_segment_id_for_blocks(blocks: &[WaveformBlock]) -> Option<String> {
    blocks
        .iter()
        .find(|block| block.segment_id == CREATE_PREVIEW_SEGMENT_ID)
        .map(|block| block.segment_id.clone())
}

fn view_lazy_selected_region_border_overlay(
    dependency: SelectedRegionBorderDependency,
) -> Element<'static, Message> {
    lazy(dependency, |dependency| {
        view_waveform_selected_region_border_canvas(
            Arc::clone(&dependency.blocks),
            &dependency.color_token,
            dependency.selected_segment_id.clone(),
            dependency.metrics.body_height,
        )
    })
    .into()
}

fn view_lazy_region_affordance_overlay(
    dependency: RegionAffordanceOverlayDependency,
) -> Element<'static, Message> {
    lazy(dependency, |dependency| {
        view_region_affordance_overlay(
            &dependency.blocks,
            dependency.selected_segment_id.as_deref(),
            dependency.hovered_segment_id.as_deref(),
            &dependency.color_token,
            dependency.viewport,
            dependency.metrics,
            dependency.suppress_region_affordances,
        )
    })
    .into()
}

fn view_timeline_overlay<'a>(
    overlay: TimelineOverlayView,
    body_height: f32,
) -> Element<'static, Message> {
    let right_percent = (100.0 - overlay.left_percent - overlay.width_percent).max(0.0);

    row![
        Space::with_width(Length::FillPortion(portion(overlay.left_percent))),
        container(text(""))
            .height(Length::Fixed(body_height))
            .width(Length::FillPortion(portion(overlay.width_percent)))
            .style(overlay_range_style),
        Space::with_width(Length::FillPortion(portion(right_percent))),
    ]
    .width(Fill)
    .into()
}

fn synthetic_hit_test(
    blocks: &[WaveformBlock],
    viewport: studio_app::TimeRange,
    fraction: f32,
    width: f32,
) -> studio_waveform::WaveformHit {
    let percent = fraction * 100.0;
    let handle_percent = ((REGION_HANDLE_HIT_ZONE_WIDTH / 2.0) / width.max(1.0) * 100.0).max(0.4);
    let time_seconds = viewport.start_seconds + viewport.duration_seconds() * fraction as f64;

    for block in blocks {
        let left = block.left_percent;
        let right = block.left_percent + block.width_percent;
        if (percent - left).abs() <= handle_percent {
            return studio_waveform::WaveformHit {
                target: studio_waveform::WaveformHitTarget::RegionStartHandle,
                region_id: Some(block.segment_id.clone()),
                time_seconds,
            };
        }
        if (percent - right).abs() <= handle_percent {
            return studio_waveform::WaveformHit {
                target: studio_waveform::WaveformHitTarget::RegionEndHandle,
                region_id: Some(block.segment_id.clone()),
                time_seconds,
            };
        }
        if left <= percent && percent <= right {
            return studio_waveform::WaveformHit {
                target: studio_waveform::WaveformHitTarget::RegionBody,
                region_id: Some(block.segment_id.clone()),
                time_seconds,
            };
        }
    }

    studio_waveform::WaveformHit {
        target: studio_waveform::WaveformHitTarget::Waveform,
        region_id: None,
        time_seconds,
    }
}

fn view_engine_waveform_body<'a>(
    editor: &'a EditorSession,
    data: WaveformBodyData<'a>,
    engine: EngineWaveformData,
) -> Element<'a, Message> {
    let viewport = editor.viewport;
    let hit_area = responsive(move |size: Size| {
        let hit_width = size.width.max(1.0);
        let hit_blocks = Arc::clone(&data.blocks);
        let hover_lane_id = data.lane_id.clone();
        let press_lane_id = data.lane_id.clone();
        let middle_press_lane_id = data.lane_id.clone();
        let release_lane_id = data.lane_id.clone();
        let selected_segment_id =
            visible_segment_id_for_blocks(&data.blocks, data.selected_segment_id.as_deref());
        let hovered_segment_id = visible_segment_id_for_blocks(
            &data.blocks,
            data.hover
                .as_ref()
                .and_then(|hover| hover.region_id.as_deref()),
        );
        let selected_segment_id = active_region_segment_id_for_blocks(
            &data.blocks,
            selected_segment_id.as_deref(),
            hovered_segment_id.as_deref(),
        );
        let mut layers: Vec<Element<'a, Message>> =
            Vec::with_capacity(TIMELINE_WAVEFORM_LAYER_CAPACITY);
        layers.push(view_timed(
            "engine-waveform-strip",
            view_lazy_engine_waveform_strip(EngineWaveformStripDependency {
                scroll_strip_peaks: Arc::clone(&engine.scroll_strip_peaks),
                color_token: data.color_token.clone(),
                metrics: data.metrics,
                source_start_seconds: engine.scroll_strip_start_seconds,
                source_end_seconds: engine.scroll_strip_end_seconds,
                visible_start_seconds: engine.visible_start_seconds,
                visible_end_seconds: engine.visible_end_seconds,
            }),
        ));
        layers.push(view_timed(
            "engine-region-fill-strip",
            view_lazy_engine_region_fill_strip(EngineRegionFillDependency {
                blocks: Arc::clone(&data.strip_blocks),
                color_token: data.color_token.clone(),
                metrics: data.metrics,
                visible_start_seconds: engine.visible_start_seconds,
                visible_end_seconds: engine.visible_end_seconds,
            }),
        ));
        if let Some(selected_segment_id) = selected_segment_id.clone() {
            layers.push(view_lazy_selected_region_border_overlay(
                SelectedRegionBorderDependency {
                    blocks: Arc::clone(&data.blocks),
                    selected_segment_id,
                    color_token: data.color_token.clone(),
                    metrics: data.metrics,
                },
            ));
        }
        if !data.suppress_region_affordances
            && (selected_segment_id.is_some() || hovered_segment_id.is_some())
        {
            layers.push(view_lazy_region_affordance_overlay(
                RegionAffordanceOverlayDependency {
                    blocks: Arc::clone(&data.blocks),
                    selected_segment_id,
                    hovered_segment_id,
                    color_token: data.color_token.clone(),
                    viewport,
                    metrics: data.metrics,
                    suppress_region_affordances: data.suppress_region_affordances,
                },
            ));
        }
        if let Some(overlay) = data.overlay {
            layers.push(view_timeline_overlay(overlay, data.metrics.body_height));
        }
        layers.push(view_timed(
            "waveform-playhead",
            view_waveform_playhead_canvas(data.playhead_percent, data.metrics.body_height),
        ));
        if let Some(cursor) =
            view_hover_cursor(viewport, data.hover.as_ref(), data.metrics.body_height)
        {
            layers.push(cursor);
        }
        if let Some(magnifier) = data.magnifier {
            let layout = waveform_magnifier_layout(magnifier, hit_width, data.metrics.body_height);
            let magnified_waveform = if engine.scroll_strip_peaks.is_empty() {
                None
            } else {
                let (visible_start_seconds, visible_end_seconds) = waveform_magnifier_time_window(
                    viewport,
                    magnifier,
                    hit_width,
                    layout.width_px,
                    engine.scroll_strip_start_seconds,
                    engine.scroll_strip_end_seconds,
                );
                Some(view_lazy_engine_waveform_magnifier(
                    EngineWaveformMagnifierDependency {
                        scroll_strip_peaks: Arc::clone(&engine.scroll_strip_peaks),
                        color_token: data.color_token.clone(),
                        body_height: layout.waveform_height_px,
                        vertical_scale: data.metrics.vertical_scale * magnifier.scale,
                        source_start_seconds: engine.scroll_strip_start_seconds,
                        source_end_seconds: engine.scroll_strip_end_seconds,
                        visible_start_seconds,
                        visible_end_seconds,
                    },
                ))
            };
            layers.push(view_waveform_magnifier(
                magnifier,
                layout,
                magnified_waveform,
                data.metrics.body_height,
            ));
        }
        if let Some(anchor_fraction) = data.timeline_autoscroll_anchor_fraction {
            layers.push(view_timeline_autoscroll_marker(
                anchor_fraction,
                data.metrics.body_height,
            ));
        }
        if let Some(popup) = data.attached_create_region_popup {
            layers.push(view_attached_create_region_popup(
                viewport,
                popup,
                data.metrics.body_height,
            ));
        }
        layers.push(view_timed(
            "waveform-scrollbar",
            view_waveform_scrollbar(data.scrollbar, data.metrics.body_height),
        ));
        let content: Element<'a, Message> = stack(layers)
            .height(Length::Fixed(data.metrics.body_height))
            .width(Fill)
            .into();
        let interaction = waveform_interaction(
            data.timeline_autoscroll_anchor_fraction.is_some(),
            data.hover.as_ref(),
        );

        mouse_area(
            container(content)
                .width(Fill)
                .height(Length::Fixed(data.metrics.body_height)),
        )
        .on_move(move |point| {
            let fraction = (point.x / hit_width).clamp(0.0, 1.0);
            Message::WaveformHitHovered(
                hover_lane_id.clone(),
                synthetic_hit_test(&hit_blocks, viewport, fraction, hit_width),
            )
        })
        .on_exit(Message::WaveformHitExited(release_lane_id.clone()))
        .on_press(Message::WaveformHitPressed(press_lane_id))
        .on_release(Message::WaveformHitReleased(release_lane_id))
        .on_middle_press(Message::TimelineAutoscrollPressed(middle_press_lane_id))
        .on_middle_release(Message::TimelineAutoscrollReleased)
        .interaction(interaction)
        .into()
    });

    container(hit_area)
        .height(Length::Fixed(data.metrics.body_height))
        .width(Fill)
        .into()
}

fn view_waveform_scrollbar(
    scrollbar: TimelineScrollbarView,
    body_height: f32,
) -> Element<'static, Message> {
    if !scrollbar.is_visible {
        return Space::with_height(Length::Fixed(body_height)).into();
    }

    let thumb_width = scrollbar
        .width_percent
        .clamp(0.0, 100.0)
        .max(SCROLLBAR_THUMB_MIN_PERCENT);
    let left_percent = scrollbar
        .left_percent
        .clamp(0.0, 100.0 - thumb_width)
        .max(0.0);
    let thumb = responsive(move |size: Size| {
        let width = size.width.max(1.0);

        mouse_area(view_waveform_scrollbar_canvas(
            left_percent,
            thumb_width,
            SCROLLBAR_THUMB_HEIGHT,
        ))
        .on_move(move |point| Message::TimelineScrollbarHovered((point.x / width).clamp(0.0, 1.0)))
        .on_press(Message::TimelineScrollbarPressed)
        .on_release(Message::TimelineScrollbarReleased)
        .interaction(mouse::Interaction::Pointer)
        .into()
    });

    column![
        Space::with_height(Length::Fixed(
            (body_height - SCROLLBAR_THUMB_HEIGHT - SCROLLBAR_BOTTOM_OFFSET).max(0.0)
        )),
        thumb,
        Space::with_height(Length::Fixed(SCROLLBAR_BOTTOM_OFFSET)),
    ]
    .width(Fill)
    .height(Length::Fixed(body_height))
    .into()
}

fn waveform_interaction(
    autoscroll_active: bool,
    hover: Option<&WaveformHoverOverlay>,
) -> mouse::Interaction {
    if autoscroll_active {
        return autoscroll_interaction(true);
    }

    if matches!(
        hover.map(|hover| hover.target),
        Some(WaveformHitTarget::RegionStartHandle | WaveformHitTarget::RegionEndHandle)
    ) {
        return mouse::Interaction::ResizingHorizontally;
    }

    autoscroll_interaction(false)
}

fn autoscroll_interaction(active: bool) -> mouse::Interaction {
    if active {
        mouse::Interaction::Grabbing
    } else {
        mouse::Interaction::Pointer
    }
}

fn view_timeline_autoscroll_marker(
    anchor_fraction: f32,
    body_height: f32,
) -> Element<'static, Message> {
    let anchor_percent = (anchor_fraction.clamp(0.0, 1.0) * 100.0).clamp(0.0, 100.0);
    let right_percent = (100.0 - anchor_percent).max(0.0);
    let marker_height = 12.0_f32.min(body_height.max(0.0));
    let line_height = (body_height - marker_height).max(0.0);
    let marker: Element<'static, Message> = column![
        container(text(""))
            .width(Length::Fixed(12.0))
            .height(Length::Fixed(marker_height))
            .style(hover_label_style),
        container(text(""))
            .width(Length::Fixed(1.0))
            .height(Length::Fixed(line_height))
            .style(hover_cursor_style),
    ]
    .align_x(iced::Alignment::Center)
    .width(Length::Fixed(12.0))
    .height(Length::Fixed(body_height))
    .into();

    row![
        Space::with_width(Length::FillPortion(portion(anchor_percent))),
        marker,
        Space::with_width(Length::FillPortion(portion(right_percent))),
    ]
    .width(Fill)
    .height(Length::Fixed(body_height))
    .into()
}

fn view_attached_create_region_popup<'a>(
    viewport: TimeRange,
    popup: AttachedCreateRegionPopup<'a>,
    body_height: f32,
) -> Element<'a, Message> {
    let center_seconds = (popup.start_seconds + popup.end_seconds) / 2.0;
    if !center_seconds.is_finite()
        || center_seconds < viewport.start_seconds
        || center_seconds > viewport.end_seconds
    {
        return Space::with_height(Length::Fixed(body_height)).into();
    }

    let center_percent = (((center_seconds - viewport.start_seconds) / viewport.duration_seconds())
        * 100.0)
        .clamp(0.0, 100.0) as f32;
    let left_percent = (center_percent - 16.0).clamp(0.0, 100.0);
    let right_percent = (100.0 - left_percent).max(0.0);
    let popup_card = container(
        column![
            text("Add Region").size(13),
            text_input("Enter text for this region...", popup.text)
                .on_input(Message::CreateRegionTextChanged)
                .on_submit(Message::SubmitCreateRegion)
                .padding(8)
                .width(Fill),
            row![
                button(text("Cancel")).on_press(Message::CancelCreateRegion),
                button(text("Save"))
                    .on_press(Message::SubmitCreateRegion)
                    .width(Fill),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(10)
    .width(Length::Fixed(ATTACHED_CREATE_REGION_POPUP_WIDTH))
    .style(action_menu_popup_style);

    column![
        Space::with_height(Length::Fixed(
            ATTACHED_CREATE_REGION_POPUP_TOP_OFFSET.min(body_height.max(0.0))
        )),
        row![
            Space::with_width(Length::FillPortion(portion(left_percent))),
            opaque(popup_card),
            Space::with_width(Length::FillPortion(portion(right_percent))),
        ]
        .width(Fill),
    ]
    .height(Length::Fixed(body_height))
    .width(Fill)
    .into()
}

fn waveform_magnifier_layout(
    overlay: WaveformMagnifierOverlay,
    body_width: f32,
    body_height: f32,
) -> WaveformMagnifierLayout {
    let body_width = body_width.max(1.0);
    let panel_width = overlay
        .width_px
        .min(body_width)
        .max(WAVEFORM_MAGNIFIER_MIN_WIDTH_PX.min(body_width));
    let left_px = if body_width <= panel_width {
        0.0
    } else {
        (overlay.fraction.clamp(0.0, 1.0) * body_width - panel_width / 2.0)
            .clamp(0.0, body_width - panel_width)
    };
    let available_height = (body_height - WAVEFORM_MAGNIFIER_INSET_PX * 2.0).max(1.0);
    let panel_height = available_height.min(overlay.max_height_px.max(1.0));
    let top_px = WAVEFORM_MAGNIFIER_INSET_PX.min((body_height - panel_height).max(0.0));
    let waveform_height = (panel_height - WAVEFORM_MAGNIFIER_HEADER_HEIGHT).max(1.0);

    WaveformMagnifierLayout {
        left_px,
        top_px,
        width_px: panel_width,
        height_px: panel_height,
        waveform_height_px: waveform_height,
    }
}

fn waveform_magnifier_time_window(
    viewport: TimeRange,
    overlay: WaveformMagnifierOverlay,
    body_width: f32,
    panel_width: f32,
    source_start_seconds: f64,
    source_end_seconds: f64,
) -> (f64, f64) {
    let viewport_duration = viewport.duration_seconds().max(0.001);
    let body_width = body_width.max(1.0);
    let source_duration = (source_end_seconds - source_start_seconds).max(0.001);
    let window_duration = (viewport_duration
        * f64::from(panel_width.max(1.0) / body_width / overlay.scale.max(0.001)))
    .clamp(0.001, source_duration);
    let half_window = window_duration / 2.0;
    let min_start = source_start_seconds;
    let max_start = (source_end_seconds - window_duration).max(min_start);
    let start = (overlay.time_seconds - half_window).clamp(min_start, max_start);

    (start, start + window_duration)
}

fn view_waveform_magnifier(
    overlay: WaveformMagnifierOverlay,
    layout: WaveformMagnifierLayout,
    waveform: Option<Element<'static, Message>>,
    body_height: f32,
) -> Element<'static, Message> {
    let header = row![
        text(format!("{:.0}x", overlay.scale)).size(10),
        Space::with_width(Fill),
        text(format_mm_ss(overlay.time_seconds)).size(10),
    ]
    .align_y(iced::Alignment::Center);
    let header: Element<'static, Message> = container(header)
        .padding([2, 6])
        .height(Length::Fixed(WAVEFORM_MAGNIFIER_HEADER_HEIGHT))
        .width(Fill)
        .into();
    let waveform = waveform.unwrap_or_else(|| {
        container(text(""))
            .height(Length::Fixed(layout.waveform_height_px))
            .width(Fill)
            .into()
    });
    let waveform_preview: Element<'static, Message> = stack(vec![
        waveform,
        view_waveform_magnifier_center_line(layout.waveform_height_px),
    ])
    .height(Length::Fixed(layout.waveform_height_px))
    .width(Fill)
    .into();
    let panel = container(column![header, waveform_preview].spacing(0))
        .width(Length::Fixed(layout.width_px))
        .height(Length::Fixed(layout.height_px))
        .style(waveform_magnifier_style);

    row![
        Space::with_width(Length::Fixed(layout.left_px)),
        column![
            Space::with_height(Length::Fixed(layout.top_px)),
            opaque(panel)
        ]
        .spacing(0),
        Space::with_width(Fill),
    ]
    .width(Fill)
    .height(Length::Fixed(body_height))
    .into()
}

fn view_waveform_magnifier_center_line(height: f32) -> Element<'static, Message> {
    row![
        Space::with_width(Length::FillPortion(1)),
        container(text(""))
            .height(Length::Fixed(height))
            .width(Length::Fixed(1.0))
            .style(waveform_magnifier_center_line_style),
        Space::with_width(Length::FillPortion(1)),
    ]
    .width(Fill)
    .height(Length::Fixed(height))
    .into()
}

fn view_hover_cursor<'a>(
    viewport: TimeRange,
    hover: Option<&WaveformHoverOverlay>,
    body_height: f32,
) -> Option<Element<'static, Message>> {
    let hover = hover?;
    let hover_time_seconds = hover.time_seconds;
    if !hover_time_seconds.is_finite()
        || hover_time_seconds < viewport.start_seconds
        || hover_time_seconds > viewport.end_seconds
    {
        return None;
    }

    let cursor_percent =
        (((hover_time_seconds - viewport.start_seconds) / viewport.duration_seconds()) * 100.0)
            .clamp(0.0, 100.0) as f32;
    let right_percent = (100.0 - cursor_percent).max(0.0);
    let line: Element<'static, Message> = row![
        Space::with_width(Length::FillPortion(portion(cursor_percent))),
        container(text(""))
            .height(Length::Fixed(body_height))
            .width(Length::Fixed(1.0))
            .style(hover_cursor_style),
        Space::with_width(Length::FillPortion(portion(right_percent))),
    ]
    .width(Fill)
    .into();
    let time_label: Element<'static, Message> = row![
        Space::with_width(Length::FillPortion(portion(cursor_percent))),
        container(text(format_mm_ss(hover_time_seconds)).size(10))
            .padding(3)
            .style(hover_label_style),
        Space::with_width(Length::FillPortion(portion(right_percent))),
    ]
    .width(Fill)
    .into();
    let mut layers = vec![line, time_label];
    if let Some(region_label) = hover
        .region_label
        .as_ref()
        .filter(|label| !label.is_empty())
    {
        layers.push(view_hover_region_label(
            cursor_percent,
            right_percent,
            region_label.clone(),
            hover.region_tooltip.clone(),
            body_height,
        ));
    }

    Some(
        stack(layers)
            .height(Length::Fixed(body_height))
            .width(Fill)
            .into(),
    )
}

fn view_hover_region_label(
    cursor_percent: f32,
    right_percent: f32,
    region_label: String,
    region_tooltip: Option<String>,
    body_height: f32,
) -> Element<'static, Message> {
    let label = container(text(region_label).size(11))
        .padding([3, 6])
        .style(hover_label_style);
    let label: Element<'static, Message> = if let Some(region_tooltip) = region_tooltip {
        tooltip(
            label,
            container(text(region_tooltip).size(12).width(Length::Fixed(360.0)))
                .padding(8)
                .style(hover_label_style),
            tooltip::Position::FollowCursor,
        )
        .into()
    } else {
        label.into()
    };

    row![
        Space::with_width(Length::FillPortion(portion(cursor_percent))),
        container(column![Space::with_height(Length::Fixed(18.0)), label])
            .padding([0, 5])
            .height(Length::Fixed(body_height)),
        Space::with_width(Length::FillPortion(portion(right_percent))),
    ]
    .width(Fill)
    .into()
}

fn view_waveform_region_affordance_block<'a>(
    block: &'a WaveformBlock,
    color_token: &'a str,
    viewport: TimeRange,
    metrics: WaveformLaneMetrics,
) -> Element<'static, Message> {
    let block_width_percent = block.width_percent;
    let mut layers = vec![view_region_boundary_timestamps(
        block,
        viewport,
        metrics.body_height,
    )];
    if !block.is_transcription {
        layers.push(view_region_label_edit_tooltip(
            block,
            color_token,
            metrics.body_height,
        ));
    }

    container(stack(layers))
        .height(Length::Fixed(metrics.body_height))
        .width(Length::FillPortion(portion(block_width_percent)))
        .into()
}

fn view_region_boundary_timestamps(
    block: &WaveformBlock,
    viewport: TimeRange,
    body_height: f32,
) -> Element<'static, Message> {
    let region_start_seconds = region_time_seconds(viewport, block.left_percent);
    let region_end_seconds = region_time_seconds(
        viewport,
        (block.left_percent + block.width_percent).min(100.0),
    );

    row![
        container(text(format_mm_ss(region_start_seconds)).size(10))
            .padding([2, 6])
            .height(Length::Fixed(REGION_TIMESTAMP_TOOLTIP_HEIGHT))
            .style(waveform_region_timestamp_label_style),
        Space::with_width(Fill),
        container(text(format_mm_ss(region_end_seconds)).size(10))
            .padding([2, 6])
            .height(Length::Fixed(REGION_TIMESTAMP_TOOLTIP_HEIGHT))
            .style(waveform_region_timestamp_label_style),
    ]
    .width(Fill)
    .height(Length::Fixed(body_height))
    .align_y(iced::Alignment::Start)
    .into()
}

fn view_region_label_edit_tooltip(
    block: &WaveformBlock,
    color_token: &str,
    body_height: f32,
) -> Element<'static, Message> {
    let color_token = color_token.to_string();
    let label_text = if block.tooltip.trim().is_empty() {
        "(empty)".to_string()
    } else {
        block.tooltip.clone()
    };
    let pencil = Svg::new(REGION_EDIT_PENCIL_ICON_HANDLE.clone())
        .width(Length::Fixed(REGION_EDIT_PENCIL_ICON_SIZE))
        .height(Length::Fixed(REGION_EDIT_PENCIL_ICON_SIZE))
        .style(|_, _| svg::Style {
            color: Some(Color::from_rgb8(102, 102, 102)),
        });

    let tooltip_content = row![text(label_text).size(13).width(Fill), pencil]
        .spacing(8)
        .align_y(iced::Alignment::Center);
    let edit_button = button(tooltip_content)
        .style(table_plain_button_style)
        .on_press(Message::RequestEditAnnotation(block.segment_id.clone()))
        .padding(0)
        .width(Length::Fixed(REGION_LABEL_TOOLTIP_WIDTH));
    let tooltip_top = (REGION_TIMESTAMP_TOOLTIP_HEIGHT + REGION_LABEL_TOOLTIP_GAP)
        .min((body_height - REGION_TIMESTAMP_TOOLTIP_HEIGHT).max(0.0));

    row![
        Space::with_width(Fill),
        column![
            Space::with_height(Length::Fixed(tooltip_top)),
            container(edit_button)
                .padding([8, 12])
                .width(Length::Fixed(REGION_LABEL_TOOLTIP_WIDTH))
                .style(move |_| waveform_region_label_edit_tooltip_style(&color_token)),
        ],
        Space::with_width(Fill),
    ]
    .width(Fill)
    .height(Length::Fixed(body_height))
    .into()
}

fn region_time_seconds(viewport: TimeRange, percent: f32) -> f64 {
    viewport.start_seconds
        + viewport.duration_seconds() * f64::from(percent.clamp(0.0, 100.0)) / 100.0
}

fn view_lazy_synthetic_base_waveform(
    dependency: SyntheticBaseWaveformDependency,
) -> Element<'static, Message> {
    lazy(dependency, |dependency| {
        view_wavesurfer_segment_waveforms_with_region_fill_canvas(
            Arc::clone(&dependency.blocks),
            SYNTHETIC_REGION_PEAK_SCALE,
            &dependency.color_token,
            dependency.metrics.body_height,
            dependency.metrics.vertical_scale,
        )
    })
    .into()
}

fn view_lazy_engine_waveform_strip(
    dependency: EngineWaveformStripDependency,
) -> Element<'static, Message> {
    lazy(dependency, |dependency| {
        view_wavesurfer_line_waveform_window_canvas(
            Arc::clone(&dependency.scroll_strip_peaks),
            &dependency.color_token,
            dependency.metrics.body_height,
            dependency.metrics.vertical_scale,
            dependency.source_start_seconds,
            dependency.source_end_seconds,
            dependency.visible_start_seconds,
            dependency.visible_end_seconds,
        )
    })
    .into()
}

fn view_lazy_engine_waveform_magnifier(
    dependency: EngineWaveformMagnifierDependency,
) -> Element<'static, Message> {
    lazy(dependency, |dependency| {
        view_wavesurfer_line_waveform_window_canvas(
            Arc::clone(&dependency.scroll_strip_peaks),
            &dependency.color_token,
            dependency.body_height,
            dependency.vertical_scale,
            dependency.source_start_seconds,
            dependency.source_end_seconds,
            dependency.visible_start_seconds,
            dependency.visible_end_seconds,
        )
    })
    .into()
}

fn view_lazy_engine_region_fill_strip(
    dependency: EngineRegionFillDependency,
) -> Element<'static, Message> {
    lazy(dependency, |dependency| {
        view_wavesurfer_region_fill_window_canvas(
            Arc::clone(&dependency.blocks),
            &dependency.color_token,
            dependency.metrics.body_height,
            dependency.visible_start_seconds,
            dependency.visible_end_seconds,
        )
    })
    .into()
}
