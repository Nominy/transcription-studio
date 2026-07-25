use std::{cell::Cell, sync::LazyLock};

use iced::widget::{canvas, text};
use iced::{
    Color, Element, Fill, Font, Length, Pixels, Point, Rectangle, Renderer, Theme, alignment, mouse,
};
use studio_app::{
    EditorSession, TimeRange, TimelineRulerNotchKind, build_babel_timeline_ruler_for_viewport,
};

use super::super::super::Message;
use super::super::wavesurfer_canvas::quantized_window_key;

pub(super) const RULER_HEIGHT: f32 = 20.0;
const RULER_TEXT_INDENT_PX: f32 = 3.0;
const RULER_FONT_SIZE: f32 = 10.0;
const RULER_LABEL_OVERFLOW_PX: f32 = 64.0;
const RULER_LINE_WIDTH: f32 = 1.0;

static RULER_CANVAS_CACHE_GROUP: LazyLock<canvas::Group> = LazyLock::new(canvas::Group::unique);

#[derive(Clone, Copy, Debug)]
struct RulerCanvas {
    viewport: TimeRange,
    min_px_per_sec: f64,
}

#[derive(Debug)]
struct RulerCanvasState {
    cache_key: Cell<u64>,
    cache: canvas::Cache,
}

impl Default for RulerCanvasState {
    fn default() -> Self {
        Self {
            cache_key: Cell::new(0),
            cache: canvas::Cache::with_group(*RULER_CANVAS_CACHE_GROUP),
        }
    }
}

impl RulerCanvasState {
    fn clear_if_key_changed(&self, key: u64) {
        if self.cache_key.get() != key {
            self.cache.clear();
            self.cache_key.set(key);
        }
    }
}

pub(super) fn view_lane_time_ruler(editor: &EditorSession) -> Element<'_, Message> {
    let viewport = editor.viewport;
    let min_px_per_sec = editor.timeline_min_px_per_sec();
    view_ruler_canvas(viewport, min_px_per_sec)
}

fn view_ruler_canvas(viewport: TimeRange, min_px_per_sec: f64) -> Element<'static, Message> {
    canvas(RulerCanvas {
        viewport,
        min_px_per_sec,
    })
    .width(Fill)
    .height(Length::Fixed(RULER_HEIGHT))
    .into()
}

impl canvas::Program<Message> for RulerCanvas {
    type State = RulerCanvasState;

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
            let ruler = build_babel_timeline_ruler_for_viewport(
                self.viewport,
                bounds.width as f64,
                self.min_px_per_sec,
            );

            for notch in ruler.notches {
                let x = notch.left_px as f32;
                if !(-RULER_LABEL_OVERFLOW_PX..=bounds.width + RULER_LABEL_OVERFLOW_PX).contains(&x)
                {
                    continue;
                }

                let line_height = match notch.kind {
                    TimelineRulerNotchKind::Primary | TimelineRulerNotchKind::Secondary => {
                        RULER_HEIGHT
                    }
                    TimelineRulerNotchKind::Tick => RULER_HEIGHT / 2.0,
                };
                let opacity = match notch.kind {
                    TimelineRulerNotchKind::Primary => 1.0,
                    TimelineRulerNotchKind::Secondary | TimelineRulerNotchKind::Tick => 0.25,
                };
                let line = canvas::Path::line(Point::new(x, 0.0), Point::new(x, line_height));
                frame.stroke(
                    &line,
                    canvas::Stroke::default()
                        .with_color(ruler_color(opacity))
                        .with_width(RULER_LINE_WIDTH),
                );

                if let Some(label) = notch.label {
                    frame.fill_text(canvas::Text {
                        content: label,
                        position: Point::new(x + RULER_TEXT_INDENT_PX, 0.0),
                        color: ruler_color(1.0),
                        size: Pixels(RULER_FONT_SIZE),
                        line_height: text::LineHeight::Relative(1.0),
                        font: Font::DEFAULT,
                        horizontal_alignment: alignment::Horizontal::Left,
                        vertical_alignment: alignment::Vertical::Top,
                        shaping: text::Shaping::Basic,
                    });
                }
            }
        });

        vec![geometry]
    }
}

impl RulerCanvas {
    fn cache_key(&self, pixel_width: f32) -> u64 {
        let (viewport_start_bucket, viewport_scale_bucket) = quantized_window_key(
            self.viewport.start_seconds,
            self.viewport.end_seconds,
            f64::from(pixel_width),
        );
        let key = hash_u64(0xcbf2_9ce4_8422_2325, viewport_start_bucket);
        let key = hash_u64(key, viewport_scale_bucket);
        hash_u64(key, self.min_px_per_sec.to_bits())
    }
}

fn hash_u64(state: u64, value: u64) -> u64 {
    state.wrapping_mul(0x100_0000_01b3).wrapping_add(value)
}

fn ruler_color(alpha: f32) -> Color {
    Color {
        a: alpha,
        ..Color::from_rgb8(82, 82, 82)
    }
}
