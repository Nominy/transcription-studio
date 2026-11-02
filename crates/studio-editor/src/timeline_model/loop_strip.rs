use crate::timeline_model::TimeRange;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopStripView {
    pub has_range: bool,
    pub is_enabled: bool,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub left_percent: f32,
    pub width_percent: f32,
    pub playhead_percent: f32,
}

impl LoopStripView {
    pub fn hit_test(self, viewport: TimeRange, fraction: f32) -> LoopStripHit {
        hit_loop_strip(self, viewport, fraction)
    }

    pub fn hit_test_client_x(
        self,
        viewport: TimeRange,
        metrics: Option<LoopStripClientMetrics>,
        client_x: f64,
    ) -> LoopStripHit {
        hit_loop_strip_at_time(self, viewport, client_x_to_loop_time(metrics, client_x))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopStripClientMetrics {
    pub container_left: f64,
    pub scroll_left: f64,
    pub duration: f64,
    pub zoom_value: f64,
}

impl LoopStripClientMetrics {
    pub fn for_viewport(viewport: TimeRange, duration: f64, visible_width: f32) -> Self {
        let viewport_duration = viewport.duration_seconds().max(f64::EPSILON);
        let zoom_value = visible_width.max(1.0) as f64 / viewport_duration;
        Self {
            container_left: 0.0,
            scroll_left: viewport.start_seconds * zoom_value,
            duration,
            zoom_value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopStripHit {
    pub target: LoopStripHitTarget,
    pub time_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopStripHitTarget {
    Empty,
    StartHandle,
    RangeBody,
    EndHandle,
}

pub fn client_x_to_loop_time(metrics: Option<LoopStripClientMetrics>, client_x: f64) -> f64 {
    let Some(metrics) = metrics else {
        return 0.0;
    };
    let duration = if metrics.duration.is_finite() {
        metrics.duration.max(0.0)
    } else {
        0.0
    };
    if !client_x.is_finite() || !metrics.zoom_value.is_finite() || metrics.zoom_value <= 0.0 {
        return 0.0;
    }

    let pixel_offset = client_x - metrics.container_left + metrics.scroll_left;
    (pixel_offset / metrics.zoom_value).clamp(0.0, duration)
}

pub(crate) fn build_loop_strip(
    is_enabled: bool,
    loop_range: Option<TimeRange>,
    viewport: TimeRange,
    playhead_seconds: f64,
) -> LoopStripView {
    let viewport_duration = viewport.duration_seconds();
    let has_range = loop_range.is_some();
    let loop_range = loop_range.unwrap_or(viewport);
    let clipped_start = loop_range.start_seconds.max(viewport.start_seconds);
    let clipped_end = loop_range.end_seconds.min(viewport.end_seconds);
    let visible_duration = if has_range {
        (clipped_end - clipped_start).max(0.0)
    } else {
        0.0
    };

    LoopStripView {
        has_range,
        is_enabled,
        start_seconds: loop_range.start_seconds,
        end_seconds: loop_range.end_seconds,
        left_percent: if has_range && visible_duration > 0.0 {
            percent_between(clipped_start, viewport, viewport_duration)
        } else {
            0.0
        },
        width_percent: if has_range && visible_duration > 0.0 {
            (visible_duration / viewport_duration * 100.0).clamp(0.0, 100.0) as f32
        } else {
            0.0
        },
        playhead_percent: percent_between(playhead_seconds, viewport, viewport_duration),
    }
}

pub(crate) fn hit_loop_strip(
    strip: LoopStripView,
    viewport: TimeRange,
    fraction: f32,
) -> LoopStripHit {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let time_seconds = viewport.start_seconds + viewport.duration_seconds() * fraction as f64;
    hit_loop_strip_at_time(strip, viewport, time_seconds)
}

fn hit_loop_strip_at_time(
    strip: LoopStripView,
    viewport: TimeRange,
    time_seconds: f64,
) -> LoopStripHit {
    let viewport_duration = viewport.duration_seconds();
    let fraction = if time_seconds.is_finite() && viewport_duration > 0.0 {
        ((time_seconds - viewport.start_seconds) / viewport_duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let time_seconds = if time_seconds.is_finite() {
        time_seconds
    } else {
        viewport.start_seconds
    };
    if !strip.has_range || strip.width_percent <= 0.0 {
        return LoopStripHit {
            target: LoopStripHitTarget::Empty,
            time_seconds,
        };
    }

    let pointer_percent = fraction * 100.0;
    let start_percent = strip.left_percent;
    let end_percent = strip.left_percent + strip.width_percent;
    if pointer_percent < start_percent || pointer_percent > end_percent {
        return LoopStripHit {
            target: LoopStripHitTarget::Empty,
            time_seconds,
        };
    }

    let handle_width_percent = (strip.width_percent * 0.2).clamp(0.15, 1.5);
    let target = if (pointer_percent - start_percent).abs() <= handle_width_percent {
        LoopStripHitTarget::StartHandle
    } else if (pointer_percent - end_percent).abs() <= handle_width_percent {
        LoopStripHitTarget::EndHandle
    } else {
        LoopStripHitTarget::RangeBody
    };

    LoopStripHit {
        target,
        time_seconds,
    }
}

fn percent_between(seconds: f64, viewport: TimeRange, viewport_duration: f64) -> f32 {
    (((seconds - viewport.start_seconds) / viewport_duration) * 100.0).clamp(0.0, 100.0) as f32
}
