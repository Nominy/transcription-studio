use std::{
    cell::Cell,
    sync::{Arc, LazyLock},
};

use iced::advanced::renderer::Renderer as _;
use iced::advanced::{Layout, Widget, layout, mouse as advanced_mouse, renderer};
use iced::mouse;
use iced::widget::canvas;
use iced::{Color, Element, Fill, Length, Point, Rectangle, Renderer, Size, Theme, border};
use studio_app::{OverviewWaveformBlock, WaveformBlock};

use super::super::Message;
use super::super::styles::{color_for_token, portion, region_border_color_for_token};
use super::playhead_line::view_playhead_line;

static WAVEFORM_CANVAS_CACHE_GROUP: LazyLock<canvas::Group> = LazyLock::new(canvas::Group::unique);
const WAVEFORM_REGION_BORDER_WIDTH: f32 = 2.0;

pub(super) fn view_wavesurfer_line_waveform<'a>(
    peaks: Arc<[f64]>,
    color: Color,
    height: f32,
    vertical_scale: f32,
) -> Element<'a, Message> {
    canvas(WaveformCanvas {
        peaks: WaveformPeaks::F64(peaks),
        color,
        vertical_scale,
    })
    .height(Length::Fixed(height))
    .width(Fill)
    .into()
}

pub(super) fn view_wavesurfer_line_waveform_window_canvas<'a>(
    peaks: Arc<[f32]>,
    color_token: &str,
    height: f32,
    vertical_scale: f32,
    source_start_seconds: f64,
    source_end_seconds: f64,
    visible_start_seconds: f64,
    visible_end_seconds: f64,
) -> Element<'a, Message> {
    canvas(WaveformWindowCanvas {
        peaks,
        color: continuous_waveform_color(color_token),
        vertical_scale,
        time_window: PeakTimeWindow {
            source_start_seconds,
            source_end_seconds,
            visible_start_seconds,
            visible_end_seconds,
        },
    })
    .height(Length::Fixed(height))
    .width(Fill)
    .into()
}

pub(super) fn view_wavesurfer_region_fill_window_canvas<'a>(
    blocks: Arc<[WaveformBlock]>,
    color_token: &str,
    height: f32,
    visible_start_seconds: f64,
    visible_end_seconds: f64,
) -> Element<'a, Message> {
    canvas(RegionFillCanvas {
        blocks,
        fill_color: waveform_region_fill_color(color_token),
        border_color: waveform_region_border_color(color_token),
        time_window: RegionFillTimeWindow {
            start_seconds: visible_start_seconds,
            end_seconds: visible_end_seconds,
        },
    })
    .height(Length::Fixed(height))
    .width(Fill)
    .into()
}

pub(super) fn view_wavesurfer_segment_waveforms_with_region_fill_canvas<'a>(
    blocks: Arc<[WaveformBlock]>,
    peak_scale: f64,
    color_token: &str,
    height: f32,
    vertical_scale: f32,
) -> Element<'a, Message> {
    canvas(SegmentWaveformsWithRegionFillCanvas {
        blocks,
        peak_scale,
        waveform_color: synthetic_waveform_color(color_token),
        fill_color: waveform_region_fill_color(color_token),
        border_color: waveform_region_border_color(color_token),
        vertical_scale,
    })
    .height(Length::Fixed(height))
    .width(Fill)
    .into()
}

pub(super) fn view_waveform_selected_region_border_canvas<'a>(
    blocks: Arc<[WaveformBlock]>,
    color_token: &str,
    selected_segment_id: String,
    height: f32,
) -> Element<'a, Message> {
    canvas(SelectedRegionBorderCanvas {
        blocks,
        selected_segment_id,
        selected_border_color: region_border_color_for_token(color_token),
    })
    .height(Length::Fixed(height))
    .width(Fill)
    .into()
}

pub(super) fn view_overview_blocks_canvas<'a>(
    blocks: Arc<[OverviewWaveformBlock]>,
) -> Element<'a, Message> {
    canvas(OverviewBlocksCanvas { blocks })
        .height(Length::Fixed(30.0))
        .width(Fill)
        .into()
}

pub(super) fn view_overview_window_quad<'a>(
    viewport_left_percent: f32,
    viewport_width_percent: f32,
) -> Element<'a, Message> {
    Element::new(OverviewWindowQuad {
        viewport_left_percent,
        viewport_width_percent,
    })
}

pub(super) fn view_overview_playhead_canvas<'a>(playhead_percent: f32) -> Element<'a, Message> {
    view_playhead_line(playhead_percent, 30.0, 2.0, Color::from_rgb8(49, 160, 252))
}

pub(super) fn view_waveform_playhead_canvas<'a>(
    playhead_percent: f32,
    height: f32,
) -> Element<'a, Message> {
    view_playhead_line(playhead_percent, height, 1.0, Color::from_rgb8(20, 20, 20))
}

pub(super) fn view_waveform_scrollbar_canvas<'a>(
    left_percent: f32,
    width_percent: f32,
    height: f32,
) -> Element<'a, Message> {
    canvas(WaveformScrollbarCanvas {
        left_percent,
        width_percent,
    })
    .height(Length::Fixed(height))
    .width(Fill)
    .into()
}

pub(super) fn continuous_waveform_color(color_token: &str) -> Color {
    Color {
        a: 0.28,
        ..color_for_token(color_token)
    }
}

pub(super) fn synthetic_waveform_color(color_token: &str) -> Color {
    Color {
        a: 0.64,
        ..color_for_token(color_token)
    }
}

fn waveform_region_fill_color(color_token: &str) -> Color {
    Color {
        a: 0.25,
        ..color_for_token(color_token)
    }
}

fn waveform_region_border_color(color_token: &str) -> Color {
    region_border_color_for_token(color_token)
}

#[derive(Clone, Debug)]
enum WaveformPeaks {
    F64(Arc<[f64]>),
    F32(Arc<[f32]>),
}

#[derive(Clone, Debug)]
struct WaveformCanvas {
    peaks: WaveformPeaks,
    color: Color,
    vertical_scale: f32,
}

#[derive(Clone, Debug)]
struct WaveformWindowCanvas {
    peaks: Arc<[f32]>,
    color: Color,
    vertical_scale: f32,
    time_window: PeakTimeWindow,
}

#[derive(Clone, Debug)]
struct RegionFillCanvas {
    blocks: Arc<[WaveformBlock]>,
    fill_color: Color,
    border_color: Color,
    time_window: RegionFillTimeWindow,
}

#[derive(Clone, Debug)]
struct SegmentWaveformsWithRegionFillCanvas {
    blocks: Arc<[WaveformBlock]>,
    peak_scale: f64,
    waveform_color: Color,
    fill_color: Color,
    border_color: Color,
    vertical_scale: f32,
}

#[derive(Clone, Debug)]
struct SelectedRegionBorderCanvas {
    blocks: Arc<[WaveformBlock]>,
    selected_segment_id: String,
    selected_border_color: Color,
}

#[derive(Clone, Debug)]
struct OverviewBlocksCanvas {
    blocks: Arc<[OverviewWaveformBlock]>,
}

#[derive(Clone, Debug)]
struct OverviewWindowQuad {
    viewport_left_percent: f32,
    viewport_width_percent: f32,
}

#[derive(Clone, Debug)]
struct WaveformScrollbarCanvas {
    left_percent: f32,
    width_percent: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PeakTimeWindow {
    source_start_seconds: f64,
    source_end_seconds: f64,
    visible_start_seconds: f64,
    visible_end_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RegionFillTimeWindow {
    start_seconds: f64,
    end_seconds: f64,
}

#[derive(Debug)]
struct WaveformCanvasState {
    cache_key: Cell<u64>,
    cache: canvas::Cache,
}

impl Default for WaveformCanvasState {
    fn default() -> Self {
        Self {
            cache_key: Cell::new(0),
            cache: canvas::Cache::with_group(*WAVEFORM_CANVAS_CACHE_GROUP),
        }
    }
}

impl WaveformCanvasState {
    fn clear_if_key_changed(&self, key: u64) {
        if self.cache_key.get() != key {
            self.cache.clear();
            self.cache_key.set(key);
        }
    }
}

impl canvas::Program<Message> for WaveformCanvas {
    type State = WaveformCanvasState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        state.clear_if_key_changed(self.cache_key());
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let waveform_path = canvas::Path::new(|builder| {
                append_wavesurfer_line_points(
                    builder,
                    &self.peaks,
                    bounds.width,
                    bounds.height,
                    self.vertical_scale,
                );
            });
            frame.fill(&waveform_path, self.color);
        });

        vec![geometry]
    }
}

impl WaveformCanvas {
    fn cache_key(&self) -> u64 {
        let mut key = self.peaks.cache_key();
        key = hash_u32(key, self.color.r.to_bits());
        key = hash_u32(key, self.color.g.to_bits());
        key = hash_u32(key, self.color.b.to_bits());
        key = hash_u32(key, self.color.a.to_bits());
        hash_u32(key, self.vertical_scale.to_bits())
    }
}

impl canvas::Program<Message> for WaveformWindowCanvas {
    type State = WaveformCanvasState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        state.clear_if_key_changed(self.cache_key());
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let waveform_path = canvas::Path::new(|builder| {
                append_wavesurfer_line_points_f32_window(
                    builder,
                    &self.peaks,
                    self.time_window,
                    bounds.width,
                    bounds.height,
                    self.vertical_scale,
                );
            });
            frame.fill(&waveform_path, self.color);
        });

        vec![geometry]
    }
}

impl WaveformWindowCanvas {
    fn cache_key(&self) -> u64 {
        let peaks_key = WaveformPeaks::F32(Arc::clone(&self.peaks)).cache_key();
        let mut key = hash_u64(peaks_key, self.time_window.source_start_seconds.to_bits());
        key = hash_u64(key, self.time_window.source_end_seconds.to_bits());
        key = hash_u64(key, self.time_window.visible_start_seconds.to_bits());
        key = hash_u64(key, self.time_window.visible_end_seconds.to_bits());
        key = hash_color(key, &self.color);
        hash_u32(key, self.vertical_scale.to_bits())
    }
}

impl WaveformPeaks {
    fn cache_key(&self) -> u64 {
        match self {
            Self::F64(peaks) => {
                let ptr = Arc::as_ptr(peaks) as *const f64 as usize as u64;
                hash_u64(hash_u64(0xcbf2_9ce4_8422_2325, ptr), peaks.len() as u64)
            }
            Self::F32(peaks) => {
                let ptr = Arc::as_ptr(peaks) as *const f32 as usize as u64;
                let key = hash_u64(hash_u64(0xcbf2_9ce4_8422_2326, ptr), peaks.len() as u64);
                hash_u64(key, peaks.len() as u64)
            }
        }
    }
}

impl canvas::Program<Message> for RegionFillCanvas {
    type State = WaveformCanvasState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        state.clear_if_key_changed(self.cache_key());
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            draw_region_fills_for_time_window(
                frame,
                &self.blocks,
                bounds.width,
                bounds.height,
                self.fill_color,
                self.border_color,
                self.time_window,
            );
        });

        vec![geometry]
    }
}

impl RegionFillCanvas {
    fn cache_key(&self) -> u64 {
        let mut key = hash_u64(0xa6d3_4b92_c0de_f111, self.blocks.len() as u64);
        key = hash_u64(key, self.time_window.start_seconds.to_bits());
        key = hash_u64(key, self.time_window.end_seconds.to_bits());
        for block in self.blocks.iter() {
            key = hash_str(key, block.segment_id.as_str());
            key = hash_u64(key, block.start_seconds.to_bits());
            key = hash_u64(key, block.end_seconds.to_bits());
        }
        key = hash_color(key, &self.fill_color);
        hash_color(key, &self.border_color)
    }
}

impl canvas::Program<Message> for SegmentWaveformsWithRegionFillCanvas {
    type State = WaveformCanvasState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        state.clear_if_key_changed(self.cache_key());
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let waveform_path = canvas::Path::new(|builder| {
                append_wavesurfer_segment_waveforms_f32(
                    builder,
                    &self.blocks,
                    self.peak_scale,
                    bounds.width,
                    bounds.height,
                    self.vertical_scale,
                );
            });
            frame.fill(&waveform_path, self.waveform_color);
            draw_region_fills(
                frame,
                &self.blocks,
                bounds.width,
                bounds.height,
                self.fill_color,
                self.border_color,
            );
        });

        vec![geometry]
    }
}

impl SegmentWaveformsWithRegionFillCanvas {
    fn cache_key(&self) -> u64 {
        let mut key = hash_u64(0xcdf2_9ce4_8422_2325, self.blocks.len() as u64);
        for block in self.blocks.iter() {
            key = hash_str(key, block.segment_id.as_str());
            key = hash_u32(key, block.left_percent.to_bits());
            key = hash_u32(key, block.width_percent.to_bits());
            let peaks_ptr = Arc::as_ptr(&block.peaks) as *const f32 as usize as u64;
            key = hash_u64(key, peaks_ptr);
            key = hash_u64(key, block.peaks.len() as u64);
        }
        key = hash_u64(key, self.peak_scale.to_bits());
        key = hash_color(key, &self.waveform_color);
        key = hash_color(key, &self.fill_color);
        key = hash_color(key, &self.border_color);
        hash_u32(key, self.vertical_scale.to_bits())
    }
}

impl canvas::Program<Message> for SelectedRegionBorderCanvas {
    type State = WaveformCanvasState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        state.clear_if_key_changed(self.cache_key());
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let selected_segment_id = self.selected_segment_id.as_str();
            let radius = border::Radius::from(1.0);
            for (block, block_width) in self
                .blocks
                .iter()
                .zip(synthetic_waveform_block_widths(&self.blocks, bounds.width))
            {
                if selected_segment_id != block.segment_id.as_str() {
                    continue;
                }
                let region_path = canvas::Path::rounded_rectangle(
                    Point::new(block_width.left_px, 0.0),
                    Size::new(block_width.width_px, bounds.height),
                    radius,
                );
                frame.stroke(
                    &region_path,
                    canvas::Stroke::default()
                        .with_width(2.0)
                        .with_color(self.selected_border_color),
                );
                break;
            }
        });

        vec![geometry]
    }
}

impl SelectedRegionBorderCanvas {
    fn cache_key(&self) -> u64 {
        let mut key = hash_u64(0x9ce4_8422_2325_cbf3, self.blocks.len() as u64);
        for block in self.blocks.iter() {
            key = hash_str(key, block.segment_id.as_str());
            key = hash_u32(key, block.left_percent.to_bits());
            key = hash_u32(key, block.width_percent.to_bits());
        }
        key = hash_u32(key, self.selected_border_color.r.to_bits());
        key = hash_u32(key, self.selected_border_color.g.to_bits());
        key = hash_u32(key, self.selected_border_color.b.to_bits());
        key = hash_u32(key, self.selected_border_color.a.to_bits());
        hash_optional_str(key, Some(self.selected_segment_id.as_str()))
    }
}

impl canvas::Program<Message> for OverviewBlocksCanvas {
    type State = WaveformCanvasState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        state.clear_if_key_changed(self.cache_key());
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            for (block, block_width) in self
                .blocks
                .iter()
                .zip(overview_block_widths(&self.blocks, bounds.width))
            {
                frame.fill_rectangle(
                    Point::new(block_width.left_px, 2.0),
                    Size::new(block_width.width_px, 24.0),
                    continuous_waveform_color(&block.color_token),
                );
            }
        });

        vec![geometry]
    }
}

impl OverviewBlocksCanvas {
    fn cache_key(&self) -> u64 {
        let ptr = Arc::as_ptr(&self.blocks) as *const OverviewWaveformBlock as usize as u64;
        hash_u64(
            hash_u64(0x8422_2325_cbf2_9ce4, ptr),
            self.blocks.len() as u64,
        )
    }
}

impl Widget<Message, Theme, Renderer> for OverviewWindowQuad {
    fn size(&self) -> Size<Length> {
        Size::new(Fill, Length::Fixed(30.0))
    }

    fn layout(
        &self,
        _tree: &mut iced::advanced::widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(
            Fill,
            Length::Fixed(30.0),
            Size::new(limits.max().width, 30.0),
        );
        layout::Node::new(size)
    }

    fn draw(
        &self,
        _tree: &iced::advanced::widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: advanced_mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let (window_left, window_width) = overview_window_rect(
            self.viewport_left_percent,
            self.viewport_width_percent,
            bounds.width,
        );
        if bounds.height <= 0.0 || window_width <= 0.0 {
            return;
        }

        let window_bounds = Rectangle {
            x: bounds.x + window_left,
            y: bounds.y,
            width: window_width,
            height: bounds.height,
        };
        if window_bounds.intersection(viewport).is_none() {
            return;
        }

        renderer.fill_quad(
            renderer::Quad {
                bounds: window_bounds,
                border: border::rounded(1)
                    .width(2)
                    .color(Color::from_rgb8(49, 160, 252)),
                ..renderer::Quad::default()
            },
            Color {
                a: 0.10,
                ..Color::from_rgb8(100, 181, 246)
            },
        );
    }
}

impl canvas::Program<Message> for WaveformScrollbarCanvas {
    type State = WaveformCanvasState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        state.clear_if_key_changed(self.cache_key());
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let (left_px, width_px) =
                waveform_scrollbar_thumb_rect(self.left_percent, self.width_percent, bounds.width);
            let thumb_path = canvas::Path::rounded_rectangle(
                Point::new(left_px, 0.0),
                Size::new(width_px, bounds.height),
                border::Radius::from(999.0),
            );
            frame.fill(&thumb_path, Color::from_rgb8(116, 116, 116));
        });

        vec![geometry]
    }
}

impl WaveformScrollbarCanvas {
    fn cache_key(&self) -> u64 {
        let key = hash_u32(0x1b3_2328, self.left_percent.to_bits());
        hash_u32(key, self.width_percent.to_bits())
    }
}

fn hash_optional_str(key: u64, value: Option<&str>) -> u64 {
    match value {
        Some(value) => hash_str(hash_u64(key, 1), value),
        None => hash_u64(key, 0),
    }
}

fn hash_str(key: u64, value: &str) -> u64 {
    value
        .bytes()
        .fold(key, |key, byte| hash_u32(key, u32::from(byte)))
}

fn hash_u32(state: u64, value: u32) -> u64 {
    hash_u64(state, value as u64)
}

fn hash_u64(state: u64, value: u64) -> u64 {
    state.wrapping_mul(0x100_0000_01b3).wrapping_add(value)
}

fn hash_color(mut key: u64, color: &Color) -> u64 {
    key = hash_u32(key, color.r.to_bits());
    key = hash_u32(key, color.g.to_bits());
    key = hash_u32(key, color.b.to_bits());
    hash_u32(key, color.a.to_bits())
}

fn draw_region_fills(
    frame: &mut canvas::Frame<Renderer>,
    blocks: &[WaveformBlock],
    width: f32,
    height: f32,
    fill_color: Color,
    border_color: Color,
) {
    let radius = border::Radius::from(1.0);
    for block_width in synthetic_waveform_block_widths(blocks, width) {
        let region_path = canvas::Path::rounded_rectangle(
            Point::new(block_width.left_px, 0.0),
            Size::new(block_width.width_px, height),
            radius,
        );
        frame.fill(&region_path, fill_color);
        frame.stroke(
            &region_path,
            canvas::Stroke::default()
                .with_width(WAVEFORM_REGION_BORDER_WIDTH)
                .with_color(border_color),
        );
    }
}

fn draw_region_fills_for_time_window(
    frame: &mut canvas::Frame<Renderer>,
    blocks: &[WaveformBlock],
    width: f32,
    height: f32,
    fill_color: Color,
    border_color: Color,
    time_window: RegionFillTimeWindow,
) {
    let radius = border::Radius::from(1.0);
    for block_width in time_window_waveform_block_widths(blocks, width, time_window) {
        let region_path = canvas::Path::rounded_rectangle(
            Point::new(block_width.left_px, 0.0),
            Size::new(block_width.width_px, height),
            radius,
        );
        frame.fill(&region_path, fill_color);
        frame.stroke(
            &region_path,
            canvas::Stroke::default()
                .with_width(WAVEFORM_REGION_BORDER_WIDTH)
                .with_color(border_color),
        );
    }
}

fn append_wavesurfer_line_points(
    builder: &mut canvas::path::Builder,
    peaks: &WaveformPeaks,
    width: f32,
    height: f32,
    vertical_scale: f32,
) {
    match peaks {
        WaveformPeaks::F64(peaks) => {
            append_wavesurfer_line_points_f64(builder, peaks, width, height, vertical_scale);
        }
        WaveformPeaks::F32(peaks) => {
            append_wavesurfer_line_points_f32(builder, peaks, width, height, vertical_scale);
        }
    }
}

fn append_wavesurfer_line_points_f64(
    builder: &mut canvas::path::Builder,
    peaks: &[f64],
    width: f32,
    height: f32,
    vertical_scale: f32,
) {
    append_wavesurfer_line_points_with(
        builder,
        0.0,
        peaks.len(),
        width,
        height,
        vertical_scale,
        |index| {
            peaks
                .get(index)
                .copied()
                .unwrap_or(0.0)
                .abs()
                .clamp(0.0, 1.0) as f32
        },
    );
}

fn append_wavesurfer_line_points_f32(
    builder: &mut canvas::path::Builder,
    peaks: &[f32],
    width: f32,
    height: f32,
    vertical_scale: f32,
) {
    append_wavesurfer_line_points_with(
        builder,
        0.0,
        peaks.len(),
        width,
        height,
        vertical_scale,
        |index| peaks.get(index).copied().unwrap_or(0.0),
    );
}

fn append_wavesurfer_line_points_f32_window(
    builder: &mut canvas::path::Builder,
    peaks: &[f32],
    time_window: PeakTimeWindow,
    width: f32,
    height: f32,
    vertical_scale: f32,
) {
    let Some((start_index, end_index)) = peak_indices_for_time_window(peaks.len(), time_window)
    else {
        return;
    };
    append_wavesurfer_line_points_with(
        builder,
        0.0,
        end_index - start_index,
        width,
        height,
        vertical_scale,
        |index| peaks.get(start_index + index).copied().unwrap_or(0.0),
    );
}

fn peak_indices_for_time_window(
    peak_count: usize,
    time_window: PeakTimeWindow,
) -> Option<(usize, usize)> {
    if peak_count == 0 {
        return None;
    }
    let source_duration = time_window.source_end_seconds - time_window.source_start_seconds;
    if !source_duration.is_finite() || source_duration <= 0.0 {
        return None;
    }
    let visible_start = time_window
        .visible_start_seconds
        .max(time_window.source_start_seconds)
        .min(time_window.source_end_seconds);
    let visible_end = time_window
        .visible_end_seconds
        .max(time_window.source_start_seconds)
        .min(time_window.source_end_seconds);
    if !visible_start.is_finite() || !visible_end.is_finite() || visible_start >= visible_end {
        return None;
    }

    let start_ratio =
        ((visible_start - time_window.source_start_seconds) / source_duration).clamp(0.0, 1.0);
    let end_ratio =
        ((visible_end - time_window.source_start_seconds) / source_duration).clamp(0.0, 1.0);
    let start_index = (start_ratio * peak_count as f64).floor() as usize;
    let mut end_index = (end_ratio * peak_count as f64).ceil() as usize;
    let start_index = start_index.min(peak_count);
    end_index = end_index.min(peak_count);
    if end_index <= start_index && start_index < peak_count {
        end_index = start_index + 1;
    }

    (end_index > start_index).then_some((start_index, end_index))
}

fn append_wavesurfer_line_points_f32_scaled_at(
    builder: &mut canvas::path::Builder,
    peaks: &[f32],
    peak_scale: f64,
    x_origin: f32,
    width: f32,
    height: f32,
    vertical_scale: f32,
) {
    append_wavesurfer_line_points_with(
        builder,
        x_origin,
        peaks.len(),
        width,
        height,
        vertical_scale,
        |index| {
            peaks
                .get(index)
                .copied()
                .map(|peak| f64::from(peak) * peak_scale)
                .unwrap_or(0.0)
                .abs()
                .clamp(0.0, 1.0) as f32
        },
    );
}

fn append_wavesurfer_segment_waveforms_f32(
    builder: &mut canvas::path::Builder,
    blocks: &[WaveformBlock],
    peak_scale: f64,
    width: f32,
    height: f32,
    vertical_scale: f32,
) {
    for (block, block_width) in blocks
        .iter()
        .zip(synthetic_waveform_block_widths(blocks, width))
    {
        append_wavesurfer_line_points_f32_scaled_at(
            builder,
            &block.peaks,
            peak_scale,
            block_width.left_px,
            block_width.width_px,
            height,
            vertical_scale,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SyntheticWaveformBlockWidth {
    left_px: f32,
    width_px: f32,
}

#[derive(Clone, Debug)]
struct SyntheticWaveformBlockWidths<'a> {
    blocks: &'a [WaveformBlock],
    width: f32,
    index: usize,
}

#[derive(Clone, Debug)]
struct TimeWindowWaveformBlockWidths<'a> {
    blocks: &'a [WaveformBlock],
    width: f32,
    time_window: RegionFillTimeWindow,
    index: usize,
}

#[derive(Clone, Debug)]
struct OverviewBlockWidths<'a> {
    blocks: &'a [OverviewWaveformBlock],
    width: f32,
    total_portion: f32,
    index: usize,
    cursor_percent: f32,
    cursor_px: f32,
}

fn synthetic_waveform_block_widths(
    blocks: &[WaveformBlock],
    width: f32,
) -> SyntheticWaveformBlockWidths<'_> {
    SyntheticWaveformBlockWidths {
        blocks,
        width,
        index: 0,
    }
}

fn time_window_waveform_block_widths(
    blocks: &[WaveformBlock],
    width: f32,
    time_window: RegionFillTimeWindow,
) -> TimeWindowWaveformBlockWidths<'_> {
    TimeWindowWaveformBlockWidths {
        blocks,
        width,
        time_window,
        index: 0,
    }
}

fn overview_block_widths(blocks: &[OverviewWaveformBlock], width: f32) -> OverviewBlockWidths<'_> {
    OverviewBlockWidths {
        blocks,
        width,
        total_portion: overview_block_total_portion(blocks).max(1) as f32,
        index: 0,
        cursor_percent: 0.0,
        cursor_px: 0.0,
    }
}

impl Iterator for SyntheticWaveformBlockWidths<'_> {
    type Item = SyntheticWaveformBlockWidth;

    fn next(&mut self) -> Option<Self::Item> {
        let block = self.blocks.get(self.index)?;
        self.index += 1;

        Some(SyntheticWaveformBlockWidth {
            left_px: self.width * block.left_percent.clamp(0.0, 100.0) / 100.0,
            width_px: self.width * block.width_percent.clamp(0.0, 100.0) / 100.0,
        })
    }
}

impl Iterator for TimeWindowWaveformBlockWidths<'_> {
    type Item = SyntheticWaveformBlockWidth;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let block = self.blocks.get(self.index)?;
            self.index += 1;
            let Some((left_percent, width_percent)) =
                time_window_block_percentages(block, self.time_window)
            else {
                continue;
            };

            return Some(SyntheticWaveformBlockWidth {
                left_px: self.width * left_percent.clamp(0.0, 100.0) / 100.0,
                width_px: self.width * width_percent.clamp(0.0, 100.0) / 100.0,
            });
        }
    }
}

impl Iterator for OverviewBlockWidths<'_> {
    type Item = SyntheticWaveformBlockWidth;

    fn next(&mut self) -> Option<Self::Item> {
        let block = self.blocks.get(self.index)?;
        self.index += 1;

        let gap = (block.left_percent - self.cursor_percent).max(0.0);
        if gap > 0.0 {
            self.cursor_px += self.width * f32::from(portion(gap)) / self.total_portion;
        }

        let block_width_px =
            self.width * f32::from(portion(block.width_percent)) / self.total_portion;
        let width = SyntheticWaveformBlockWidth {
            left_px: self.cursor_px,
            width_px: block_width_px,
        };
        self.cursor_px += block_width_px;
        self.cursor_percent = (block.left_percent + block.width_percent).min(100.0);

        Some(width)
    }
}

fn overview_window_rect(left_percent: f32, width_percent: f32, width: f32) -> (f32, f32) {
    let right_percent = 100.0 - left_percent - width_percent;
    let left_portion = f32::from(portion(left_percent));
    let width_portion = f32::from(portion(width_percent));
    let total_portion = (left_portion + width_portion + f32::from(portion(right_percent))).max(1.0);
    (
        width * left_portion / total_portion,
        width * width_portion / total_portion,
    )
}

fn waveform_scrollbar_thumb_rect(left_percent: f32, width_percent: f32, width: f32) -> (f32, f32) {
    let right_percent = (100.0 - left_percent - width_percent).max(0.0);
    let left_portion = f32::from(portion(left_percent));
    let width_portion = f32::from(portion(width_percent));
    let total_portion = (left_portion + width_portion + f32::from(portion(right_percent))).max(1.0);
    (
        width * left_portion / total_portion,
        width * width_portion / total_portion,
    )
}

fn time_window_block_percentages(
    block: &WaveformBlock,
    time_window: RegionFillTimeWindow,
) -> Option<(f32, f32)> {
    let window_duration = time_window.end_seconds - time_window.start_seconds;
    if !window_duration.is_finite() || window_duration <= 0.0 {
        return None;
    }

    let block_start = block.start_seconds.min(block.end_seconds);
    let block_end = block.start_seconds.max(block.end_seconds);
    if !block_start.is_finite()
        || !block_end.is_finite()
        || block_end < time_window.start_seconds
        || block_start > time_window.end_seconds
    {
        return None;
    }

    let clipped_start = block_start
        .max(time_window.start_seconds)
        .min(time_window.end_seconds);
    let clipped_end = block_end
        .max(time_window.start_seconds)
        .min(time_window.end_seconds);
    let left_percent =
        ((clipped_start - time_window.start_seconds) / window_duration * 100.0) as f32;
    let right_percent = ((time_window.end_seconds - clipped_end) / window_duration * 100.0) as f32;
    let width_percent = (100.0 - left_percent - right_percent).max(0.0);

    Some((
        left_percent.clamp(0.0, 100.0),
        width_percent.clamp(0.5, 100.0),
    ))
}

fn overview_block_total_portion(blocks: &[OverviewWaveformBlock]) -> u32 {
    let mut total = 0_u32;
    let mut cursor = 0.0_f32;
    for block in blocks {
        let gap = (block.left_percent - cursor).max(0.0);
        if gap > 0.0 {
            total += u32::from(portion(gap));
        }
        total += u32::from(portion(block.width_percent));
        cursor = (block.left_percent + block.width_percent).min(100.0);
    }
    if cursor < 100.0 {
        total += u32::from(portion(100.0 - cursor));
    }
    total
}

fn append_wavesurfer_line_points_with(
    builder: &mut canvas::path::Builder,
    x_origin: f32,
    sample_count: usize,
    width: f32,
    height: f32,
    vertical_scale: f32,
    mut peak_at: impl FnMut(usize) -> f32,
) {
    let half_height = height / 2.0;
    let x_scale = if sample_count > 0 {
        width / sample_count as f32
    } else {
        0.0
    };

    for polarity in [-1.0_f32, 1.0_f32] {
        builder.move_to(Point::new(x_origin, half_height));
        let mut x = 0.0_f32;
        let mut max_peak = 0.0_f32;
        for index in 0..=sample_count {
            let next_x = (index as f32 * x_scale).round();
            if next_x > x {
                let y_offset = (max_peak * half_height * vertical_scale).round().max(1.0);
                builder.line_to(Point::new(x_origin + x, half_height + y_offset * polarity));
                x = next_x;
                max_peak = 0.0;
            }

            let peak = peak_at(index);
            if peak > max_peak {
                max_peak = peak;
            }
        }
        builder.line_to(Point::new(x_origin + x, half_height));
        builder.close();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use studio_app::{WaveformBlock, WaveformPeakSource};

    use super::{
        RegionFillTimeWindow, synthetic_waveform_block_widths, time_window_waveform_block_widths,
    };

    fn block(id: &str, start_seconds: f64, end_seconds: f64) -> WaveformBlock {
        block_with_geometry(id, start_seconds, end_seconds, 0.0, 10.0)
    }

    fn block_with_geometry(
        id: &str,
        start_seconds: f64,
        end_seconds: f64,
        left_percent: f32,
        width_percent: f32,
    ) -> WaveformBlock {
        WaveformBlock {
            segment_id: id.to_string(),
            label: id.to_string(),
            tooltip: id.to_string(),
            is_transcription: true,
            start_seconds,
            end_seconds,
            left_percent,
            width_percent,
            peaks: Arc::from([0.5_f32]),
            peak_source: WaveformPeakSource::Synthetic,
        }
    }

    #[test]
    fn time_window_region_fill_widths_skip_out_of_window_blocks_without_stopping() {
        let blocks = [
            block("visible-a", 1.0, 2.0),
            block("outside-after-window", 20.0, 21.0),
            block("visible-b", 3.0, 4.0),
        ];
        let widths = time_window_waveform_block_widths(
            &blocks,
            1_000.0,
            RegionFillTimeWindow {
                start_seconds: 0.0,
                end_seconds: 5.0,
            },
        )
        .collect::<Vec<_>>();

        assert_eq!(
            widths.len(),
            2,
            "decoded region fill must continue after out-of-window blocks because Babel segment order is not guaranteed to be time-sorted"
        );
        assert!(widths.iter().all(|width| width.width_px > 0.0));
        assert!(widths[1].left_px > widths[0].left_px + widths[0].width_px);
    }

    #[test]
    fn synthetic_region_widths_keep_fixed_far_edge_pixel_stable_while_dragging_other_edge() {
        let first = block_with_geometry("preview", 1.0, 2.0, 20.4, 29.9);
        let second = block_with_geometry("preview", 1.0, 2.0, 20.6, 29.7);
        let first_width = synthetic_waveform_block_widths(&[first], 1_000.0)
            .next()
            .unwrap();
        let second_width = synthetic_waveform_block_widths(&[second], 1_000.0)
            .next()
            .unwrap();

        assert!((first_width.left_px + first_width.width_px - 503.0).abs() < 0.001);
        assert!((second_width.left_px + second_width.width_px - 503.0).abs() < 0.001);
    }
}
