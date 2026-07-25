use std::{
    cell::Cell,
    ops::Range,
    sync::{Arc, LazyLock},
};

use iced::advanced::renderer::Renderer as _;
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Layout, Widget, layout, mouse as advanced_mouse, renderer};
use iced::mouse;
use iced::widget::canvas;
use iced::{Color, Element, Fill, Length, Point, Rectangle, Renderer, Size, Theme, border};
use studio_app::{OverviewWaveformBlock, WaveformBlock};

use super::super::Message;
use super::super::styles::{color_for_token, region_border_color_for_token};
use super::playhead_line::view_playhead_line;

static WAVEFORM_CANVAS_CACHE_GROUP: LazyLock<canvas::Group> = LazyLock::new(canvas::Group::unique);
const WAVEFORM_REGION_BORDER_WIDTH: f32 = 2.0;
const OVERVIEW_BLOCKS_VERTICAL_PADDING: f32 = 2.0;

pub(super) fn view_wavesurfer_line_waveform<'a>(
    peaks: Arc<[f64]>,
    color: Color,
    height: f32,
    vertical_scale: f32,
    span_fraction: f32,
) -> Element<'a, Message> {
    canvas(WaveformCanvas {
        peaks: WaveformPeaks::F64(peaks),
        color,
        vertical_scale,
        span_fraction,
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
    source_window: Range<f64>,
    visible_window: Range<f64>,
) -> Element<'a, Message> {
    view_wavesurfer_line_waveform_window_canvas_with_color(
        peaks,
        waveform_color(color_token),
        height,
        vertical_scale,
        source_window,
        visible_window,
    )
}

pub(super) fn view_wavesurfer_unplayed_line_waveform_window_canvas<'a>(
    peaks: Arc<[f32]>,
    height: f32,
    vertical_scale: f32,
    source_window: Range<f64>,
    visible_window: Range<f64>,
) -> Element<'a, Message> {
    view_wavesurfer_line_waveform_window_canvas_with_color(
        peaks,
        waveform_unplayed_color(),
        height,
        vertical_scale,
        source_window,
        visible_window,
    )
}

fn view_wavesurfer_line_waveform_window_canvas_with_color<'a>(
    peaks: Arc<[f32]>,
    color: Color,
    height: f32,
    vertical_scale: f32,
    source_window: Range<f64>,
    visible_window: Range<f64>,
) -> Element<'a, Message> {
    canvas(WaveformWindowCanvas {
        peaks,
        color,
        vertical_scale,
        time_window: PeakTimeWindow {
            source_start_seconds: source_window.start,
            source_end_seconds: source_window.end,
            visible_start_seconds: visible_window.start,
            visible_end_seconds: visible_window.end,
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

pub(super) fn view_wavesurfer_segment_waveforms_canvas<'a>(
    blocks: Arc<[WaveformBlock]>,
    peak_scale: f64,
    color_token: &str,
    height: f32,
    vertical_scale: f32,
) -> Element<'a, Message> {
    view_wavesurfer_segment_waveforms_canvas_with_color(
        blocks,
        peak_scale,
        waveform_color(color_token),
        height,
        vertical_scale,
    )
}

pub(super) fn view_wavesurfer_unplayed_segment_waveforms_canvas<'a>(
    blocks: Arc<[WaveformBlock]>,
    peak_scale: f64,
    height: f32,
    vertical_scale: f32,
) -> Element<'a, Message> {
    view_wavesurfer_segment_waveforms_canvas_with_color(
        blocks,
        peak_scale,
        waveform_unplayed_color(),
        height,
        vertical_scale,
    )
}

fn view_wavesurfer_segment_waveforms_canvas_with_color<'a>(
    blocks: Arc<[WaveformBlock]>,
    peak_scale: f64,
    waveform_color: Color,
    height: f32,
    vertical_scale: f32,
) -> Element<'a, Message> {
    canvas(SegmentWaveformsCanvas {
        blocks,
        peak_scale,
        waveform_color,
        vertical_scale,
    })
    .height(Length::Fixed(height))
    .width(Fill)
    .into()
}

pub(super) fn view_waveform_progress_clip<'a>(
    content: Element<'a, Message>,
    playhead_percent: f32,
) -> Element<'a, Message> {
    Element::new(WaveformProgressClip {
        content,
        progress_fraction: (playhead_percent / 100.0).clamp(0.0, 1.0),
    })
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
    lane_rows: usize,
) -> Element<'a, Message> {
    canvas(OverviewBlocksCanvas { blocks, lane_rows })
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
    let (width, color) = overview_playhead_style();
    view_playhead_line(playhead_percent, 30.0, width, color)
}

pub(super) fn view_waveform_playhead_canvas<'a>(
    playhead_percent: f32,
    height: f32,
) -> Element<'a, Message> {
    let (width, color) = waveform_playhead_style();
    view_playhead_line(playhead_percent, height, width, color)
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

pub(super) fn waveform_color(color_token: &str) -> Color {
    color_for_token(color_token)
}

fn waveform_unplayed_color() -> Color {
    Color::from_rgb8(112, 112, 112)
}

fn waveform_playhead_style() -> (f32, Color) {
    (1.0, Color::from_rgb8(0, 0, 0))
}

fn overview_playhead_style() -> (f32, Color) {
    (2.0, Color::from_rgb8(49, 160, 252))
}

fn overview_block_color(color_token: &str) -> Color {
    Color {
        a: 0.25,
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
    /// Fraction of the widget width the peaks span, so audio shorter or
    /// longer than the shared time axis stays aligned with it.
    span_fraction: f32,
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
struct SegmentWaveformsCanvas {
    blocks: Arc<[WaveformBlock]>,
    peak_scale: f64,
    waveform_color: Color,
    vertical_scale: f32,
}

struct WaveformProgressClip<'a> {
    content: Element<'a, Message>,
    progress_fraction: f32,
}

impl Widget<Message, Theme, Renderer> for WaveformProgressClip<'_> {
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: advanced_mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let clip_bounds = waveform_progress_clip_bounds(layout.bounds(), self.progress_fraction);
        let Some(clip_bounds) = clip_bounds.intersection(viewport) else {
            return;
        };
        if clip_bounds.width <= 0.0 || clip_bounds.height <= 0.0 {
            return;
        }

        renderer.with_layer(clip_bounds, |renderer| {
            self.content.as_widget().draw(
                tree,
                renderer,
                theme,
                style,
                layout,
                cursor,
                &clip_bounds,
            );
        });
    }
}

fn waveform_progress_clip_bounds(bounds: Rectangle, progress_fraction: f32) -> Rectangle {
    let progress_fraction = if progress_fraction.is_finite() {
        progress_fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    Rectangle {
        width: bounds.width * progress_fraction,
        ..bounds
    }
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
    lane_rows: usize,
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
                    bounds.width * self.span_fraction.max(0.0),
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
        key = hash_u32(key, self.vertical_scale.to_bits());
        hash_u32(key, self.span_fraction.to_bits())
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
        state.clear_if_key_changed(self.cache_key(bounds.width));
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
    fn cache_key(&self, pixel_width: f32) -> u64 {
        let peaks_key = WaveformPeaks::F32(Arc::clone(&self.peaks)).cache_key();
        let mut key = hash_u64(peaks_key, self.time_window.source_start_seconds.to_bits());
        key = hash_u64(key, self.time_window.source_end_seconds.to_bits());
        let (visible_start_bucket, visible_scale_bucket) = quantized_window_key(
            self.time_window.visible_start_seconds,
            self.time_window.visible_end_seconds,
            f64::from(pixel_width),
        );
        key = hash_u64(key, visible_start_bucket);
        key = hash_u64(key, visible_scale_bucket);
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
        state.clear_if_key_changed(self.cache_key(bounds.width));
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
    fn cache_key(&self, pixel_width: f32) -> u64 {
        let mut key = hash_u64(0xa6d3_4b92_c0de_f111, self.blocks.len() as u64);
        let (window_start_bucket, window_scale_bucket) = quantized_window_key(
            self.time_window.start_seconds,
            self.time_window.end_seconds,
            f64::from(pixel_width),
        );
        key = hash_u64(key, window_start_bucket);
        key = hash_u64(key, window_scale_bucket);
        for block in self.blocks.iter() {
            key = hash_str(key, block.segment_id.as_str());
            key = hash_u64(key, block.start_seconds.to_bits());
            key = hash_u64(key, block.end_seconds.to_bits());
        }
        key = hash_color(key, &self.fill_color);
        hash_color(key, &self.border_color)
    }
}

impl canvas::Program<Message> for SegmentWaveformsCanvas {
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
        });

        vec![geometry]
    }
}

impl SegmentWaveformsCanvas {
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
            // Segment blocks tint each lane's own row (matching the stacked
            // per-lane waveforms above) like the timeline region fills, so the
            // waveforms stay legible underneath.
            let lane_rows = self.lane_rows.max(1);
            let content_top = OVERVIEW_BLOCKS_VERTICAL_PADDING;
            let content_height = (bounds.height - OVERVIEW_BLOCKS_VERTICAL_PADDING * 2.0).max(1.0);
            let row_height = content_height / lane_rows as f32;
            for (block, block_width) in self
                .blocks
                .iter()
                .zip(overview_block_widths(&self.blocks, bounds.width))
            {
                let row = block.lane_row.min(lane_rows - 1) as f32;
                frame.fill_rectangle(
                    Point::new(block_width.left_px, content_top + row * row_height),
                    Size::new(block_width.width_px, row_height),
                    overview_block_color(&block.color_token),
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
            hash_u64(
                hash_u64(0x8422_2325_cbf2_9ce4, ptr),
                self.blocks.len() as u64,
            ),
            self.lane_rows as u64,
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

/// Quantizes a visible time window into integer cache-key buckets at the
/// granularity that actually changes rendered pixels: the window position is
/// bucketed one bucket per pixel of `pixel_width` (the window is stretched
/// across that many pixels when drawn), and the zoom scale is bucketed
/// logarithmically at the same one-pixel granularity — a relative duration
/// change of `1 / pixel_width` is exactly what moves the window's right edge
/// by one pixel. Two viewports that render to identical pixels — e.g.
/// sub-pixel scroll or zoom drift between frames — produce identical keys, so
/// the canvas caches survive; any pan or zoom change of at least one pixel
/// moves at least one bucket and therefore produces a different key.
///
/// Bucketing the end offset instead of the scale would be useless: it is
/// always exactly `start_bucket + pixel_width`, so it carries no zoom
/// information at all and every window anchored at `0.0` would key
/// identically at every zoom level.
///
/// Degenerate inputs (non-finite values, an empty window, or a pixel width
/// below one) fall back to the raw `f64` bits so callers still get a correct —
/// merely conservative — key instead of a panic.
pub(super) fn quantized_window_key(
    visible_start_seconds: f64,
    visible_end_seconds: f64,
    pixel_width: f64,
) -> (u64, u64) {
    let duration_seconds = visible_end_seconds - visible_start_seconds;
    if !visible_start_seconds.is_finite()
        || !visible_end_seconds.is_finite()
        || !pixel_width.is_finite()
        || !duration_seconds.is_finite()
        || duration_seconds <= 0.0
        || pixel_width < 1.0
    {
        return (
            visible_start_seconds.to_bits(),
            visible_end_seconds.to_bits(),
        );
    }
    let seconds_per_pixel = duration_seconds / pixel_width;
    (
        (visible_start_seconds / seconds_per_pixel).round() as i64 as u64,
        (duration_seconds.ln() * pixel_width).round() as i64 as u64,
    )
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
    index: usize,
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
        index: 0,
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

        // Blocks come from multiple lanes and may overlap in time, so each one
        // is positioned absolutely from its own percentages rather than by
        // walking a shared gap cursor across the whole strip.
        let left_px = self.width * block.left_percent.clamp(0.0, 100.0) / 100.0;
        let width_px = (self.width * block.width_percent.clamp(0.0, 100.0) / 100.0)
            .min((self.width - left_px).max(0.0));

        Some(SyntheticWaveformBlockWidth {
            left_px,
            width_px: width_px.max(1.0),
        })
    }
}

fn overview_window_rect(left_percent: f32, width_percent: f32, width: f32) -> (f32, f32) {
    percent_rect_px(left_percent, width_percent, width)
}

fn waveform_scrollbar_thumb_rect(left_percent: f32, width_percent: f32, width: f32) -> (f32, f32) {
    percent_rect_px(left_percent, width_percent, width)
}

fn percent_rect_px(left_percent: f32, width_percent: f32, width: f32) -> (f32, f32) {
    let total_width = width.max(0.0);
    let left_fraction = if left_percent.is_finite() {
        (left_percent / 100.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let width_fraction = if width_percent.is_finite() {
        (width_percent / 100.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let left_px = total_width * left_fraction;
    let width_px = (total_width * width_fraction).min((total_width - left_px).max(0.0));
    (left_px, width_px)
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
            let next_x = index as f32 * x_scale;
            if next_x > x {
                let y_offset = (max_peak * half_height * vertical_scale).max(1.0);
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

    use iced::{Color, Rectangle};

    use studio_app::{OverviewWaveformBlock, WaveformBlock, WaveformPeakSource};

    use super::{
        RegionFillTimeWindow, overview_block_widths, overview_playhead_style, overview_window_rect,
        quantized_window_key, synthetic_waveform_block_widths, time_window_waveform_block_widths,
        waveform_color, waveform_playhead_style, waveform_progress_clip_bounds,
        waveform_scrollbar_thumb_rect, waveform_unplayed_color,
    };

    #[test]
    fn playback_uses_babel_progress_palette_and_distinct_playheads() {
        assert_eq!(waveform_color("blue"), Color::from_rgb8(100, 181, 246));
        assert_eq!(waveform_color("purple"), Color::from_rgb8(176, 131, 255));
        assert_eq!(waveform_unplayed_color(), Color::from_rgb8(112, 112, 112));

        let (playhead_width, playhead_color) = waveform_playhead_style();
        assert_eq!(playhead_width, 1.0);
        assert_eq!(playhead_color, Color::BLACK);

        let (overview_width, overview_color) = overview_playhead_style();
        assert_eq!(overview_width, 2.0);
        assert_eq!(overview_color, Color::from_rgb8(49, 160, 252));
    }

    #[test]
    fn progress_clip_tracks_playhead_without_changing_waveform_geometry() {
        let bounds = Rectangle {
            x: 10.0,
            y: 20.0,
            width: 800.0,
            height: 150.0,
        };

        assert_eq!(waveform_progress_clip_bounds(bounds, -1.0).width, 0.0);
        assert_eq!(waveform_progress_clip_bounds(bounds, 0.125).width, 100.0);
        assert_eq!(waveform_progress_clip_bounds(bounds, 0.5).width, 400.0);
        assert_eq!(waveform_progress_clip_bounds(bounds, 2.0).width, 800.0);
        assert_eq!(waveform_progress_clip_bounds(bounds, f32::NAN).width, 0.0);
    }

    fn overview_block(
        color_token: &str,
        lane_row: usize,
        left_percent: f32,
        width_percent: f32,
    ) -> OverviewWaveformBlock {
        OverviewWaveformBlock {
            color_token: color_token.to_string(),
            lane_row,
            left_percent,
            width_percent,
        }
    }

    #[test]
    fn overview_and_scrollbar_rects_preserve_subpercent_pixel_movement() {
        let (overview_left, overview_width) = overview_window_rect(0.125, 0.5, 800.0);
        assert!((overview_left - 1.0).abs() < 0.000_001);
        assert!((overview_width - 4.0).abs() < 0.000_001);

        let (scrollbar_left, scrollbar_width) = waveform_scrollbar_thumb_rect(0.25, 10.125, 800.0);
        assert!((scrollbar_left - 2.0).abs() < 0.000_001);
        assert!((scrollbar_width - 81.0).abs() < 0.000_001);
    }

    #[test]
    fn overview_block_widths_position_multi_lane_blocks_absolutely() {
        let blocks = [
            overview_block("blue", 0, 50.0, 10.0),
            // The second lane restarts at the left edge; a cumulative gap
            // cursor would misplace it, absolute positioning must not.
            overview_block("purple", 1, 10.0, 20.0),
        ];

        let widths = overview_block_widths(&blocks, 200.0).collect::<Vec<_>>();

        assert_eq!(widths.len(), 2);
        assert_eq!(widths[0].left_px, 100.0);
        assert_eq!(widths[0].width_px, 20.0);
        assert_eq!(widths[1].left_px, 20.0);
        assert_eq!(widths[1].width_px, 40.0);
    }

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
    fn quantized_window_key_reuses_buckets_for_sub_pixel_viewport_drift() {
        // A 10s window across a 500px strip means 0.02s per pixel.
        let base = quantized_window_key(100.0, 110.0, 500.0);

        assert_eq!(base, quantized_window_key(100.0, 110.0, 500.0));
        assert_eq!(
            base,
            quantized_window_key(100.004, 110.004, 500.0),
            "a 0.2px pan must reuse the cached tessellation"
        );
        assert_eq!(
            base,
            quantized_window_key(100.0, 110.0002, 500.0),
            "a 0.01px zoom drift must reuse the cached tessellation"
        );
    }

    #[test]
    fn quantized_window_key_changes_for_one_pixel_viewport_moves() {
        let base = quantized_window_key(100.0, 110.0, 500.0);

        assert_ne!(
            base,
            quantized_window_key(100.02, 110.02, 500.0),
            "a full-pixel pan (0.02s at 500px over 10s) must invalidate the cache"
        );
        assert_ne!(
            base,
            quantized_window_key(100.0, 110.02, 500.0),
            "a full-pixel zoom change must invalidate the cache"
        );
        assert_ne!(
            base,
            quantized_window_key(100.0, 110.0, 501.0),
            "a strip resize changes the seconds-per-pixel granularity and must invalidate"
        );
    }

    #[test]
    fn quantized_window_key_distinguishes_zoom_levels_anchored_at_the_timeline_start() {
        // `TimeRange::for_project` always anchors at 0.0 and `viewport_centered_on`
        // clamps to it, so zooming with the playhead near the start only moves the
        // window's end. A key that bucketed the end offset would be constant here —
        // the end bucket is always `start_bucket + pixel_width` — and the region-fill
        // overlays, whose dependency hash carries no other zoom-dependent input,
        // would stay tessellated at the previous zoom's pixel geometry.
        let keys = [60.0_f64, 30.0, 7.5, 3.1234, 0.5]
            .map(|end_seconds| quantized_window_key(0.0, end_seconds, 900.0));

        for (index, key) in keys.iter().enumerate() {
            for other in &keys[index + 1..] {
                assert_ne!(key, other, "each zoom level must produce a distinct key");
            }
        }
    }

    #[test]
    fn quantized_window_key_survives_degenerate_inputs_without_panicking() {
        let _ = quantized_window_key(0.0, 0.0, 500.0);
        let _ = quantized_window_key(20.0, 10.0, 500.0);
        let _ = quantized_window_key(10.0, 20.0, 0.0);
        let _ = quantized_window_key(10.0, 20.0, -3.0);
        let _ = quantized_window_key(10.0, 20.0, f64::NAN);
        let _ = quantized_window_key(f64::NAN, 20.0, 500.0);
        let _ = quantized_window_key(10.0, f64::INFINITY, 500.0);

        assert_ne!(
            quantized_window_key(1.0, 2.0, 0.0),
            quantized_window_key(1.0, 3.0, 0.0),
            "the degenerate fallback must keep distinct windows distinct"
        );
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
