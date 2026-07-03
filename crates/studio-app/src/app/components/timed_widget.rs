use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::event::{self, Event};
use iced::{Element, Length, Rectangle, Renderer, Size, Theme, Vector};

use super::super::Message;

const TRACE_WIDGET_TIMES_ENV: &str = "STUDIO_TRACE_WIDGET_TIMES";
const TRACE_WIDGET_TIMES_FRAMES_ENV: &str = "STUDIO_TRACE_WIDGET_TIMES_FRAMES";
const DEFAULT_TRACE_WIDGET_TIMES_FRAMES: u64 = 120;

static WIDGET_TIMING_STATS: LazyLock<Mutex<BTreeMap<&'static str, WidgetTimingStats>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static WIDGET_TIMING_TRACE_ENABLED: LazyLock<bool> =
    LazyLock::new(|| std::env::var(TRACE_WIDGET_TIMES_ENV).is_ok_and(|value| value != "0"));
static WIDGET_TIMING_TRACE_FRAMES: LazyLock<u64> = LazyLock::new(|| {
    std::env::var(TRACE_WIDGET_TIMES_FRAMES_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TRACE_WIDGET_TIMES_FRAMES)
});

#[derive(Debug, Default)]
struct WidgetTimingStats {
    layout_count: u64,
    draw_count: u64,
    layout_total: Duration,
    draw_total: Duration,
    layout_max: Duration,
    draw_max: Duration,
}

pub(crate) fn view_timed<'a>(
    label: &'static str,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    if !widget_timing_trace_enabled() {
        return content;
    }

    Element::new(TimedWidget { label, content })
}

struct TimedWidget<'a> {
    label: &'static str,
    content: Element<'a, Message>,
}

impl<'a> Widget<Message, Theme, Renderer> for TimedWidget<'a> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.content.as_widget())]
    }

    fn diff(&self, tree: &mut Tree) {
        if let Some(child) = tree.children.get_mut(0) {
            child.diff(self.content.as_widget());
        } else {
            tree.children.push(Tree::new(self.content.as_widget()));
        }
        tree.children.truncate(1);
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let started = Instant::now();
        let layout = self.content.as_widget().layout(
            tree.children
                .get_mut(0)
                .expect("timed widget should have one child tree"),
            renderer,
            limits,
        );
        record_widget_timing(self.label, WidgetTimingPhase::Layout, started.elapsed());
        layout
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget().operate(
            tree.children
                .get_mut(0)
                .expect("timed widget should have one child tree"),
            layout,
            renderer,
            operation,
        );
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        self.content.as_widget_mut().on_event(
            tree.children
                .get_mut(0)
                .expect("timed widget should have one child tree"),
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            tree.children
                .first()
                .expect("timed widget should have one child tree"),
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let started = Instant::now();
        self.content.as_widget().draw(
            tree.children
                .first()
                .expect("timed widget should have one child tree"),
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        record_widget_timing(self.label, WidgetTimingPhase::Draw, started.elapsed());
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            tree.children
                .get_mut(0)
                .expect("timed widget should have one child tree"),
            layout,
            renderer,
            translation,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WidgetTimingPhase {
    Layout,
    Draw,
}

fn record_widget_timing(label: &'static str, phase: WidgetTimingPhase, elapsed: Duration) {
    let Ok(mut stats_by_label) = WIDGET_TIMING_STATS.lock() else {
        return;
    };
    let stats = stats_by_label.entry(label).or_default();
    match phase {
        WidgetTimingPhase::Layout => {
            stats.layout_count += 1;
            stats.layout_total += elapsed;
            stats.layout_max = stats.layout_max.max(elapsed);
        }
        WidgetTimingPhase::Draw => {
            stats.draw_count += 1;
            stats.draw_total += elapsed;
            stats.draw_max = stats.draw_max.max(elapsed);
            if stats.draw_count % widget_timing_trace_frames() == 0 {
                eprintln!("{}", widget_timing_runtime_trace_line(label, stats));
            }
        }
    }
}

fn widget_timing_runtime_trace_line(label: &str, stats: &WidgetTimingStats) -> String {
    format!(
        "Transcription Studio widget timing: label={label} layout_count={} layout_total_ms={:.3} layout_avg_ms={:.3} layout_max_ms={:.3} draw_count={} draw_total_ms={:.3} draw_avg_ms={:.3} draw_max_ms={:.3}",
        stats.layout_count,
        duration_ms(stats.layout_total),
        duration_average_ms(stats.layout_total, stats.layout_count),
        duration_ms(stats.layout_max),
        stats.draw_count,
        duration_ms(stats.draw_total),
        duration_average_ms(stats.draw_total, stats.draw_count),
        duration_ms(stats.draw_max),
    )
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn duration_average_ms(total: Duration, count: u64) -> f64 {
    if count == 0 {
        return 0.0;
    }

    duration_ms(total) / count as f64
}

fn widget_timing_trace_enabled() -> bool {
    *WIDGET_TIMING_TRACE_ENABLED
}

fn widget_timing_trace_frames() -> u64 {
    *WIDGET_TIMING_TRACE_FRAMES
}
