//! Immediate-mode painters for the timeline surfaces (lane bodies, ruler, scrollbar,
//! overview). Lane waveforms share the peak-preserving line representation (see
//! [`studio_waveform::LineWaveform`]); the overview draws a per-frame column scan of its
//! peak buffer.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use exaudio_extension::document::Severity;
use gpui_kit::*;
use studio_editor::timeline_model::{
    OverviewWaveformBlock, TimeRange, TimelineRulerNotchKind, TimelineRulerView, WaveformBlock,
};
use studio_editor::transcript_model::format_mm_ss;
use studio_waveform::{LineFrame, LineView, LineWaveform, WaveformRenderModel, js_round};

use super::geometry::{Thumb, selection_chip_left, selection_length_label, x_at};

// Sizes below are px at 100% interface zoom; painters multiply them by
// `crate::zoom::ui_scale(window)`. Positions derived from the surface bounds are not scaled.
/// Width of the magnifier shown while a region boundary is dragged.
const MAGNIFIER_WIDTH: f32 = 180.0;
const MAGNIFIER_MAX_HEIGHT: f32 = 150.0;
const MAGNIFIER_SCALE: f32 = 3.0;
const MAGNIFIER_INSET: f32 = 6.0;
const LABEL_LINE: f32 = 15.;
const TIME_SIZE: f32 = 10.;
const CHIP_PAD: f32 = 4.0;

fn shape(text: SharedString, font_size: Pixels, color: Hsla, window: &Window) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, font_size, &[run], None)
}

/// Small rounded label (timestamps and cut selection chips over the waveform).
fn paint_chip(
    line: &ShapedLine,
    origin: Point<Pixels>,
    border: Hsla,
    palette: Palette,
    scale: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let line_height = px(LABEL_LINE * scale);
    window.paint_quad(quad(
        Bounds::new(
            origin,
            size(line.width() + px(2.0 * CHIP_PAD * scale), line_height),
        ),
        px(3. * scale),
        palette.popover,
        px(1.),
        border,
        BorderStyle::Solid,
    ));
    let _ = line.paint(
        origin + point(px(CHIP_PAD * scale), px(0.)),
        line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// One channel of peaks covering `start_seconds..end_seconds` uniformly.
#[derive(Clone)]
pub struct PeakStrip {
    pub peaks: Arc<[f32]>,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl PeakStrip {
    /// The whole-file peaks of an overview render model.
    pub fn overview(model: &WaveformRenderModel) -> Option<Self> {
        let channel = model.channels.first()?;
        (!channel.visible_peaks.is_empty()).then(|| Self {
            peaks: Arc::clone(&channel.visible_peaks),
            start_seconds: model.visible_start_seconds,
            end_seconds: model.visible_end_seconds,
        })
    }

    /// Loudest absolute peak in `t0..t1`, `None` outside the strip.
    fn max_between(&self, t0: f64, t1: f64) -> Option<f32> {
        let count = self.peaks.len();
        let span = self.end_seconds - self.start_seconds;
        if count == 0 || span <= 0.0 || t1 <= self.start_seconds || t0 >= self.end_seconds {
            return None;
        }
        let scale = count as f64 / span;
        let first = (((t0 - self.start_seconds) * scale).floor().max(0.0) as usize).min(count - 1);
        let last = (((t1 - self.start_seconds) * scale).ceil().max(0.0) as usize)
            .min(count)
            .max(first + 1);
        Some(
            self.peaks[first..last]
                .iter()
                .fold(0.0_f32, |max, peak| max.max(peak.abs())),
        )
    }
}

/// Theme-derived colors shared by every timeline surface.
#[derive(Clone, Copy)]
pub struct Palette {
    pub text: Hsla,
    pub grid: Hsla,
    /// The not-yet-played waveform.
    pub unplayed: Hsla,
    pub playhead: Hsla,
    pub hover: Hsla,
    pub accent: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub info: Hsla,
    pub popover: Hsla,
    pub muted: Hsla,
}

/// Per-region decorations derived from transcript rows (lint state, diff change).
#[derive(Clone, Copy, Default)]
pub struct RegionMark {
    pub lint: Option<Severity>,
    pub changed: bool,
}

pub type RegionMarks = Rc<HashMap<String, RegionMark>>;

/// Everything a lane body paints in one frame.
pub struct LaneScene {
    pub viewport: TimeRange,
    pub playhead_seconds: f64,
    pub color: Hsla,
    pub collapsed: bool,
    /// Lane vertical zoom.
    pub bar_height: f32,
    pub waveform: Option<Arc<LineWaveform>>,
    pub blocks: Arc<[WaveformBlock]>,
    pub selected: Option<String>,
    pub hovered: Option<String>,
    pub marks: RegionMarks,
    /// The cut selection on this lane and whether playback loops over it.
    pub overlay: Option<(TimeRange, bool)>,
    pub minimum_cut_duration_seconds: f64,
    pub hover_seconds: Option<f64>,
    pub autoscroll_anchor: Option<f32>,
    pub magnifier_seconds: Option<f64>,
    pub palette: Palette,
}

/// The part of `bounds` between `x0..x1` (px from its left edge, clamped to its width) and
/// `y0..y0 + h` (px from its top), at least 1 px wide.
fn column(bounds: Bounds<Pixels>, x0: f32, x1: f32, y0: f32, h: f32) -> Bounds<Pixels> {
    let width = f32::from(bounds.size.width);
    let x0 = x0.clamp(0.0, width);
    let x1 = x1.clamp(0.0, width);
    Bounds::new(
        point(bounds.left() + px(x0), bounds.top() + px(y0)),
        size(px((x1 - x0).max(1.0)), px(h)),
    )
}

pub fn paint_lane(bounds: Bounds<Pixels>, scene: &LaneScene, window: &mut Window, cx: &mut App) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width <= 1.0 || height <= 1.0 {
        return;
    }
    let palette = scene.palette;
    let s = crate::zoom::ui_scale(window);
    let x_of = |seconds: f64| x_at(scene.viewport, seconds, width);

    paint_region_fills(window, bounds, scene.viewport, scene);

    let playhead_x = x_of(scene.playhead_seconds);
    if !scene.collapsed
        && let Some(waveform) = scene.waveform.as_deref()
        && let Some(frame) = waveform.frame(&line_view(window, bounds, waveform, scene.viewport))
    {
        let split = js_round(frame.viewport_x_of(scene.playhead_seconds));
        paint_line(
            window,
            bounds,
            &frame,
            f64::from(scene.bar_height),
            split,
            scene.color,
            palette.unplayed,
        );
    }

    paint_region_decorations(window, bounds, scene.viewport, scene, s, cx);

    if let Some(seconds) = scene.hover_seconds {
        let x = x_of(seconds);
        if (0.0..=width).contains(&x) {
            window.paint_quad(fill(column(bounds, x, x + 1.0, 0.0, height), palette.hover));
            if !scene.collapsed {
                let label = shape(
                    format_mm_ss(seconds).into(),
                    px(TIME_SIZE * s),
                    palette.text,
                    window,
                );
                let chip_width = f32::from(label.width()) + 2.0 * CHIP_PAD * s;
                let left = if x + 6.0 * s + chip_width > width {
                    x - 6.0 * s - chip_width
                } else {
                    x + 6.0 * s
                };
                paint_chip(
                    &label,
                    point(bounds.left() + px(left), bounds.top() + px(20. * s)),
                    palette.grid,
                    palette,
                    s,
                    window,
                    cx,
                );
            }
        }
    }

    if (0.0..=width).contains(&playhead_x) {
        window.paint_quad(fill(
            column(bounds, playhead_x - 0.75, playhead_x + 0.75, 0.0, height),
            palette.playhead,
        ));
    }

    if let Some(anchor) = scene.autoscroll_anchor {
        let x = anchor.clamp(0.0, 1.0) * width;
        window.paint_quad(fill(
            column(bounds, x, x + 1.0, 0.0, height),
            palette.accent,
        ));
        window.paint_quad(quad(
            column(bounds, x - 6.0 * s, x + 6.0 * s, 4.0 * s, 12.0 * s),
            px(6. * s),
            palette.accent,
            px(0.),
            transparent_black(),
            BorderStyle::Solid,
        ));
    }

    if let Some(seconds) = scene.magnifier_seconds
        && let Some(waveform) = scene.waveform.as_deref()
    {
        paint_magnifier(window, bounds, scene, waveform, seconds, s, cx);
    }
}

/// What sits under the waveform, so the peaks stay legible: the region fills (stronger when
/// hovered/selected), for `viewport` drawn across `bounds`.
fn paint_region_fills(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    viewport: TimeRange,
    scene: &LaneScene,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let x_of = |seconds: f64| x_at(viewport, seconds, width);
    for block in scene.blocks.iter() {
        let x0 = x_of(block.start_seconds);
        let x1 = x_of(block.end_seconds);
        if x1 < 0.0 || x0 > width {
            continue;
        }
        let id = Some(block.segment_id.as_str());
        let alpha = if scene.selected.as_deref() == id {
            0.26
        } else if scene.hovered.as_deref() == id {
            0.18
        } else {
            0.10
        };
        window.paint_quad(fill(
            column(bounds, x0, x1, 0.0, height),
            scene.color.opacity(alpha),
        ));
    }
}

/// What sits over the waveform, for `viewport` drawn across `bounds`: region edges (dashed
/// outline for annotations), the selection frame, lint/diff bars and the hovered/selected
/// region's time chips when there is room, and the cut selection (accent edges, its length
/// chip and a "Loop" chip before it while playback loops over it).
fn paint_region_decorations(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    viewport: TimeRange,
    scene: &LaneScene,
    s: f32,
    cx: &mut App,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let palette = scene.palette;
    let x_of = |seconds: f64| x_at(viewport, seconds, width);
    for block in scene.blocks.iter() {
        let x0 = x_of(block.start_seconds);
        let x1 = x_of(block.end_seconds);
        if x1 < 0.0 || x0 > width {
            continue;
        }
        let id = block.segment_id.as_str();
        let selected = scene.selected.as_deref() == Some(id);
        let hovered = scene.hovered.as_deref() == Some(id);
        let area = column(bounds, x0, x1, 0.0, height);
        if block.is_transcription {
            for edge in [x0, x1] {
                if (0.0..=width).contains(&edge) {
                    window.paint_quad(fill(
                        column(bounds, edge - 0.5, edge + 0.5, 0.0, height),
                        scene.color.opacity(0.7),
                    ));
                }
            }
        } else {
            window.paint_quad(outline(area, scene.color.opacity(0.8), BorderStyle::Dashed));
        }
        if selected {
            window.paint_quad(quad(
                area,
                px(2.),
                transparent_black(),
                px(2.),
                scene.color,
                BorderStyle::Solid,
            ));
        }
        let mark = scene.marks.get(id).copied().unwrap_or_default();
        let lint_color = match mark.lint {
            Some(Severity::Error) => Some(palette.danger),
            Some(Severity::Warning) => Some(palette.warning),
            _ => None,
        };
        if let Some(color) = lint_color {
            window.paint_quad(fill(column(bounds, x0, x1, 0.0, 3.0 * s), color));
        }
        if mark.changed {
            window.paint_quad(fill(
                column(bounds, x0, x1, height - 3.0 * s, 3.0 * s),
                palette.info,
            ));
        }

        let visible_width = x1.min(width) - x0.max(0.0);
        if (selected || hovered)
            && !scene.collapsed
            && visible_width > 96.0 * s
            && height > 48.0 * s
        {
            let top = bounds.bottom() - px((LABEL_LINE + 4.) * s);
            if x0 >= 0.0 {
                let start = shape(
                    format_mm_ss(block.start_seconds).into(),
                    px(TIME_SIZE * s),
                    palette.text,
                    window,
                );
                paint_chip(
                    &start,
                    point(bounds.left() + px(x0 + 4.0 * s), top),
                    palette.grid,
                    palette,
                    s,
                    window,
                    cx,
                );
            }
            if x1 <= width {
                let end = shape(
                    format_mm_ss(block.end_seconds).into(),
                    px(TIME_SIZE * s),
                    palette.text,
                    window,
                );
                let left = x1 - (4.0 + 2.0 * CHIP_PAD) * s - f32::from(end.width());
                paint_chip(
                    &end,
                    point(bounds.left() + px(left), top),
                    palette.grid,
                    palette,
                    s,
                    window,
                    cx,
                );
            }
        }
    }

    if let Some((range, looping)) = scene.overlay {
        let (x0, x1) = (x_of(range.start_seconds), x_of(range.end_seconds));
        if x1 > 0.0 && x0 < width {
            // Green: long enough to cut; red: loop only.
            let color = if range.duration_seconds() >= scene.minimum_cut_duration_seconds {
                palette.success
            } else {
                palette.danger
            };
            let half_edge = if looping { 1.5 * s } else { 1.0 };
            let area = column(bounds, x0, x1, 0.0, height);
            window.paint_quad(fill(area, color.opacity(0.18)));
            for edge in [x0, x1] {
                window.paint_quad(fill(
                    column(bounds, edge - half_edge, edge + half_edge, 0.0, height),
                    color,
                ));
            }
            // The length chip (after "Loop" while looping) sits inside the selection when it
            // fits, else just outside an edge; text stays neutral so red/green both read.
            let chip_width = |line: &ShapedLine| f32::from(line.width()) + 2.0 * CHIP_PAD * s;
            let gap = 3.0 * s;
            let loop_chip = looping.then(|| {
                let label = crate::screens::ui::localized(cx, "Loop", "Повтор");
                shape(label.into(), px(TIME_SIZE * s), color, window)
            });
            let length_chip = shape(
                selection_length_label(
                    range.duration_seconds(),
                    crate::screens::ui::localized(cx, "s", "с"),
                )
                .into(),
                px(TIME_SIZE * s),
                palette.text,
                window,
            );
            let group_width = loop_chip
                .as_ref()
                .map_or(0.0, |chip| chip_width(chip) + gap)
                + chip_width(&length_chip);
            if let Some(mut left) = selection_chip_left(x0, x1, width, group_width, 4.0 * s) {
                let top = bounds.top() + px(4. * s);
                for chip in loop_chip.iter().chain([&length_chip]) {
                    paint_chip(
                        chip,
                        point(bounds.left() + px(left), top),
                        color,
                        palette,
                        s,
                        window,
                        cx,
                    );
                    left += chip_width(chip) + gap;
                }
            }
        }
    }
}

/// Zoomed inset around the dragged boundary: the lane body (regions under and over the
/// waveform, as in the lane) at `MAGNIFIER_SCALE`× around `seconds`.
fn paint_magnifier(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    scene: &LaneScene,
    waveform: &LineWaveform,
    seconds: f64,
    scale: f32,
    cx: &mut App,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let inset = MAGNIFIER_INSET * scale;
    let box_width = (MAGNIFIER_WIDTH * scale).min(width - 2.0 * inset);
    let box_height = (MAGNIFIER_MAX_HEIGHT * scale).min(height - 2.0 * inset);
    if box_width < 40.0 * scale || box_height < 24.0 * scale {
        return;
    }
    let x = x_at(scene.viewport, seconds, width);
    let left = (x - box_width / 2.0).clamp(inset, width - box_width - inset);
    let area = Bounds::new(
        point(bounds.left() + px(left), bounds.top() + px(inset)),
        size(px(box_width), px(box_height)),
    );
    let radius = px(6. * scale);
    window.paint_quad(quad(
        area,
        radius,
        scene.palette.popover,
        px(0.),
        transparent_black(),
        BorderStyle::Solid,
    ));
    let span = scene.viewport.duration_seconds() * f64::from(box_width / MAGNIFIER_SCALE)
        / f64::from(width);
    let range = TimeRange {
        start_seconds: seconds - span / 2.0,
        end_seconds: seconds + span / 2.0,
    };
    window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
        paint_region_fills(window, area, range, scene);
        if let Some(frame) = waveform.frame_uncached(&line_view(window, area, waveform, range)) {
            paint_line(
                window,
                area,
                &frame,
                f64::from(scene.bar_height * MAGNIFIER_SCALE),
                f64::INFINITY,
                scene.color,
                scene.color,
            );
        }
        paint_region_decorations(window, area, range, scene, scale, cx);
        let center = area.center().x;
        window.paint_quad(fill(
            Bounds::new(
                point(center - px(0.5), area.top()),
                size(px(1.), area.size.height),
            ),
            scene.palette.playhead,
        ));
    });
    // The frame goes on top, so regions running into the box's edges never cover it.
    window.paint_quad(quad(
        area,
        radius,
        transparent_black(),
        px(1.),
        scene.palette.grid,
        BorderStyle::Solid,
    ));
}

/// The wavesurfer view of `range` across `bounds`. Bounds are logical px; at interface zoom
/// `z` scales logical px relative to the document viewport; effective px are logical px / `z` and
/// the device pixel ratio is the window scale × `z`.
fn line_view(
    window: &Window,
    bounds: Bounds<Pixels>,
    waveform: &LineWaveform,
    range: TimeRange,
) -> LineView {
    let zoom = f64::from(crate::zoom::ui_scale(window));
    LineView::of_window(
        waveform.duration_seconds(),
        (range.start_seconds, range.end_seconds),
        (
            f64::from(bounds.size.width) / zoom,
            f64::from(bounds.size.height) / zoom,
        ),
        f64::from(window.scale_factor()) * zoom,
    )
}

/// Fills a wavesurfer frame's outline with the viewport's left edge at `bounds.left()`, on
/// device pixels as the browser places its canvases, clipped to the canvas height. Device
/// columns left of `split` (device px from `bounds.left()`) take `played`, the rest
/// `unplayed` (wavesurfer's progress overlay).
fn paint_line(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    frame: &LineFrame,
    bar_height: f64,
    split: f64,
    played: Hsla,
    unplayed: Hsla,
) {
    let scale = f64::from(window.scale_factor());
    let origin_x = js_round(f64::from(bounds.left()) * scale);
    let origin_y = js_round(f64::from(bounds.top()) * scale);
    let device_width = f64::from(bounds.size.width) * scale;
    let mut builder = PathBuilder::fill();
    let mut canvas_height = 0.0_f64;
    for canvas in &frame.canvases {
        let columns = canvas.columns.columns_between(
            -canvas.viewport_x / canvas.x_scale,
            (device_width - canvas.viewport_x) / canvas.x_scale,
        );
        if columns.is_empty() {
            continue;
        }
        canvas_height = canvas_height.max(f64::from(canvas.device_height) * canvas.y_scale);
        let to_point = |(x, y): (f64, f64)| {
            point(
                px(((origin_x + canvas.viewport_x + x * canvas.x_scale) / scale) as f32),
                px(((origin_y + y * canvas.y_scale) / scale) as f32),
            )
        };
        for direction in [-1.0, 1.0] {
            let mut outline = canvas
                .outline(columns.clone(), bar_height, direction)
                .map(to_point);
            if let Some(first) = outline.next() {
                builder.move_to(first);
                for vertex in outline {
                    builder.line_to(vertex);
                }
                builder.close();
            }
        }
    }
    let Ok(path) = builder.build() else {
        return;
    };

    // One outline, two colours: clip it at the progress edge instead of building it twice.
    let x_at = |device_x: f64| px(((origin_x + device_x.clamp(0.0, device_width)) / scale) as f32);
    let (left, split_x, right) = (x_at(0.0), x_at(split), x_at(device_width));
    let top = px((origin_y / scale) as f32);
    let height = px((canvas_height / scale) as f32);
    if split_x > left {
        let area = Bounds::new(point(left, top), size(split_x - left, height));
        window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
            window.paint_path(path.clone(), played);
        });
    }
    if split_x < right {
        let area = Bounds::new(point(split_x, top), size(right - split_x, height));
        window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
            window.paint_path(path, unplayed);
        });
    }
}

/// Draws an overview row: one value per device pixel (the loudest absolute peak in that
/// column), joined into a filled outline mirrored around the vertical centre.
fn paint_overview_peaks(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    strip: &PeakStrip,
    range: TimeRange,
    color: Hsla,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let columns = (width * window.scale_factor().max(1.0)).ceil().max(1.0) as usize;
    let column_width = width / columns as f32;
    let step = range.duration_seconds() / columns as f64;
    let half = (height / 2.0 - 1.0).max(0.5);
    let middle = f32::from(bounds.top()) + height / 2.0;
    let left = f32::from(bounds.left());
    // Silence keeps a hairline of one device pixel.
    let floor = 0.5 / window.scale_factor().max(1.0);
    let amplitudes: Vec<Option<f32>> = (0..columns)
        .map(|index| {
            let t0 = range.start_seconds + step * index as f64;
            strip
                .max_between(t0, t0 + step)
                .map(|peak| (peak.min(1.0) * half).max(floor))
        })
        .collect();

    // One closed outline per contiguous run of columns inside the strip: across the top
    // through the column centres (squared off at the run's outer edges), back along the bottom.
    let mut builder = PathBuilder::fill();
    let mut index = 0;
    while index < columns {
        if amplitudes[index].is_none() {
            index += 1;
            continue;
        }
        let start = index;
        while index < columns && amplitudes[index].is_some() {
            index += 1;
        }
        let run = &amplitudes[start..index];
        let x_center = |offset: usize| left + ((start + offset) as f32 + 0.5) * column_width;
        let (x_start, x_end) = (
            left + start as f32 * column_width,
            left + index as f32 * column_width,
        );
        let first = run[0].unwrap_or(floor);
        let last = run[run.len() - 1].unwrap_or(floor);
        builder.move_to(point(px(x_start), px(middle - first)));
        for (offset, amplitude) in run.iter().enumerate() {
            builder.line_to(point(
                px(x_center(offset)),
                px(middle - amplitude.unwrap_or(floor)),
            ));
        }
        builder.line_to(point(px(x_end), px(middle - last)));
        builder.line_to(point(px(x_end), px(middle + last)));
        for (offset, amplitude) in run.iter().enumerate().rev() {
            builder.line_to(point(
                px(x_center(offset)),
                px(middle + amplitude.unwrap_or(floor)),
            ));
        }
        builder.line_to(point(px(x_start), px(middle + first)));
        builder.close();
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Notches along the bottom of the ruler with their time labels, plus the playhead head.
/// Labels that would overlap a neighbour are skipped, primary labels first.
pub fn paint_ruler(
    bounds: Bounds<Pixels>,
    ruler: &TimelineRulerView,
    playhead_x: f32,
    palette: Palette,
    window: &mut Window,
    cx: &mut App,
) {
    let s = crate::zoom::ui_scale(window);
    let height = f32::from(bounds.size.height);
    window.paint_quad(fill(
        Bounds::new(
            point(bounds.left(), bounds.bottom() - px(1.)),
            size(bounds.size.width, px(1.)),
        ),
        palette.grid,
    ));
    for notch in &ruler.notches {
        let tick = match notch.kind {
            TimelineRulerNotchKind::Primary => height * 0.55,
            TimelineRulerNotchKind::Secondary => height * 0.35,
            TimelineRulerNotchKind::Tick => height * 0.2,
        };
        window.paint_quad(fill(
            Bounds::new(
                point(
                    bounds.left() + px(notch.left_px as f32),
                    bounds.bottom() - px(tick),
                ),
                size(px(1.), px(tick)),
            ),
            palette.grid,
        ));
    }
    let gap = 6.0 * s;
    let mut placed: Vec<(f32, f32)> = Vec::new();
    for primary in [true, false] {
        for notch in &ruler.notches {
            let Some(label) = notch.label.as_ref() else {
                continue;
            };
            if (notch.kind == TimelineRulerNotchKind::Primary) != primary {
                continue;
            }
            let line = shape(
                label.clone().into(),
                px(TIME_SIZE * s),
                palette.text,
                window,
            );
            let left = notch.left_px as f32 + 3.0 * s;
            let right = left + f32::from(line.width());
            if placed
                .iter()
                .any(|&(l, r)| left < r + gap && l < right + gap)
            {
                continue;
            }
            placed.push((left, right));
            let _ = line.paint(
                point(bounds.left() + px(left), bounds.top() + px(s)),
                px(12. * s),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
    let width = f32::from(bounds.size.width);
    if (0.0..=width).contains(&playhead_x) {
        let x = bounds.left() + px(playhead_x);
        window.paint_quad(fill(
            Bounds::new(
                point(x - px(0.75), bounds.top()),
                size(px(1.5), bounds.size.height),
            ),
            palette.playhead,
        ));
        window.paint_quad(quad(
            Bounds::new(
                point(x - px(5. * s), bounds.top()),
                size(px(10. * s), px(8. * s)),
            ),
            Corners {
                top_left: px(s),
                top_right: px(s),
                bottom_left: px(5. * s),
                bottom_right: px(5. * s),
            },
            palette.playhead,
            px(0.),
            transparent_black(),
            BorderStyle::Solid,
        ));
    }
}

/// One overview lane row: its peaks (if decoded) and colour.
pub struct OverviewLane {
    pub row: usize,
    pub color: Hsla,
    pub peaks: PeakStrip,
}

pub struct OverviewScene {
    pub bounds_range: TimeRange,
    pub rows: usize,
    pub lanes: Rc<[OverviewLane]>,
    pub blocks: Arc<[OverviewWaveformBlock]>,
    pub block_colors: Rc<[Hsla]>,
    pub thumb: Option<Thumb>,
    pub playhead_fraction: f32,
    pub palette: Palette,
}

/// Paints the overview and returns the device-pixel edges of its markers.
pub fn paint_overview(
    bounds: Bounds<Pixels>,
    scene: &OverviewScene,
    window: &mut Window,
) -> MarkerEdges {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let rows = scene.rows.max(1);
    let row_height = height / rows as f32;
    let row_bounds = |row: usize| {
        Bounds::new(
            point(bounds.left(), bounds.top() + px(row as f32 * row_height)),
            size(bounds.size.width, px(row_height)),
        )
    };
    for (block, color) in scene.blocks.iter().zip(scene.block_colors.iter()) {
        let row = row_bounds(block.lane_row.min(rows - 1));
        window.paint_quad(fill(
            Bounds::new(
                point(
                    row.left() + px(block.left_percent / 100.0 * width),
                    row.top(),
                ),
                size(
                    px((block.width_percent / 100.0 * width).max(1.0)),
                    row.size.height,
                ),
            ),
            color.opacity(0.22),
        ));
    }
    for lane in scene.lanes.iter() {
        paint_overview_peaks(
            window,
            row_bounds(lane.row.min(rows - 1)),
            &lane.peaks,
            scene.bounds_range,
            lane.color.opacity(0.9),
        );
    }
    let markers = OverviewMarkers::new(bounds, scene.thumb, scene.playhead_fraction);
    if let Some(area) = markers.thumb {
        window.paint_quad(quad(
            area,
            px(3. * crate::zoom::ui_scale(window)),
            scene.palette.playhead.opacity(0.08),
            px(1.),
            scene.palette.playhead.opacity(0.55),
            BorderStyle::Solid,
        ));
    }
    window.paint_quad(fill(markers.playhead, scene.palette.playhead));
    markers.device_edges(window.scale_factor())
}

/// Horizontal device-pixel edges of the overview's viewport window (if shown) and playhead.
pub type MarkerEdges = (Option<[f32; 2]>, [f32; 2]);

/// The overview's viewport window (when part of the timeline is hidden) and playhead.
pub struct OverviewMarkers {
    pub thumb: Option<Bounds<Pixels>>,
    pub playhead: Bounds<Pixels>,
}

impl OverviewMarkers {
    pub fn new(bounds: Bounds<Pixels>, thumb: Option<Thumb>, playhead_fraction: f32) -> Self {
        let width = f32::from(bounds.size.width);
        let thumb = thumb.map(|thumb| {
            Bounds::new(
                point(bounds.left() + px(thumb.drawn_left() * width), bounds.top()),
                size(px(thumb.drawn_width() * width), bounds.size.height),
            )
        });
        let x = playhead_fraction.clamp(0.0, 1.0) * width;
        let playhead = Bounds::new(
            point(bounds.left() + px(x - 1.0), bounds.top()),
            size(px(2.), bounds.size.height),
        );
        Self { thumb, playhead }
    }

    /// Horizontal device-pixel edges the markers snap to when painted (gpui rounds quad edges
    /// half toward zero). Equal edges paint identical frames.
    pub fn device_edges(&self, scale_factor: f32) -> MarkerEdges {
        let snap = |x: Pixels| {
            let value = f32::from(x) * scale_factor;
            (value.abs() - 0.5).ceil().copysign(value)
        };
        let edges = |bounds: Bounds<Pixels>| [snap(bounds.left()), snap(bounds.right())];
        (self.thumb.map(edges), edges(self.playhead))
    }
}

/// Horizontal scrollbar thumb.
pub fn paint_scrollbar(
    bounds: Bounds<Pixels>,
    thumb: Thumb,
    active: bool,
    palette: Palette,
    window: &mut Window,
) {
    let s = crate::zoom::ui_scale(window);
    let width = f32::from(bounds.size.width);
    let thickness = px(6. * s);
    window.paint_quad(quad(
        Bounds::new(
            point(
                bounds.left() + px(thumb.drawn_left() * width),
                bounds.center().y - thickness / 2.0,
            ),
            size(px(thumb.drawn_width() * width), thickness),
        ),
        px(3. * s),
        palette.muted.opacity(if active { 0.9 } else { 0.6 }),
        px(0.),
        transparent_black(),
        BorderStyle::Solid,
    ));
}
