use super::TimeRange;

const DEFAULT_RULER_WIDTH_PX: f64 = 1_000.0;
const MAX_VISIBLE_NOTCHES: usize = 10_000;
const EPSILON: f64 = 0.000_001;

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineRulerView {
    pub notches: Vec<TimelineRulerNotchView>,
    pub visible_start_seconds: f64,
    pub visible_end_seconds: f64,
    pub pixels_per_second: f64,
}

impl TimelineRulerView {
    pub fn labels(&self) -> impl Iterator<Item = &TimelineRulerNotchView> {
        self.notches.iter().filter(|notch| notch.label.is_some())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineRulerNotchView {
    pub time_seconds: f64,
    pub left_px: f64,
    pub left_percent: f32,
    pub kind: TimelineRulerNotchKind,
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineRulerNotchKind {
    Primary,
    Secondary,
    Tick,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineRulerIntervals {
    pub time_interval_seconds: f64,
    pub primary_label_interval_seconds: f64,
    pub secondary_label_interval_seconds: f64,
}

impl TimelineRulerIntervals {
    pub fn from_babel_min_px_per_sec(min_px_per_sec: f64) -> Self {
        let min_px_per_sec = finite_non_negative(min_px_per_sec);
        if min_px_per_sec >= 200.0 {
            Self {
                time_interval_seconds: 0.1,
                primary_label_interval_seconds: 1.0,
                secondary_label_interval_seconds: 0.5,
            }
        } else if min_px_per_sec >= 100.0 {
            Self {
                time_interval_seconds: 0.25,
                primary_label_interval_seconds: 2.0,
                secondary_label_interval_seconds: 1.0,
            }
        } else if min_px_per_sec >= 50.0 {
            Self {
                time_interval_seconds: 0.5,
                primary_label_interval_seconds: 5.0,
                secondary_label_interval_seconds: 2.5,
            }
        } else {
            Self {
                time_interval_seconds: 1.0,
                primary_label_interval_seconds: 10.0,
                secondary_label_interval_seconds: 5.0,
            }
        }
    }

    pub fn from_wavesurfer_defaults(pixels_per_second: f64) -> Self {
        let pixels_per_second = finite_non_negative(pixels_per_second);
        Self {
            time_interval_seconds: default_time_interval(pixels_per_second),
            primary_label_interval_seconds: default_primary_label_interval(pixels_per_second),
            secondary_label_interval_seconds: default_secondary_label_interval(pixels_per_second),
        }
    }
}

pub fn build_babel_timeline_ruler(
    duration_seconds: f64,
    scroll_left_px: f64,
    viewport_width_px: f64,
    min_px_per_sec: f64,
) -> TimelineRulerView {
    build_timeline_ruler(
        duration_seconds,
        scroll_left_px,
        viewport_width_px,
        min_px_per_sec,
        TimelineRulerIntervals::from_babel_min_px_per_sec(min_px_per_sec),
        0.0,
    )
}

pub fn build_timeline_ruler_for_viewport(
    viewport: TimeRange,
    viewport_width_px: f64,
) -> TimelineRulerView {
    let visible_duration = viewport.duration_seconds().max(EPSILON);
    let viewport_width_px = finite_positive(viewport_width_px, DEFAULT_RULER_WIDTH_PX);
    let pixels_per_second = viewport_width_px / visible_duration;
    build_timeline_ruler(
        viewport.end_seconds.max(visible_duration),
        viewport.start_seconds * pixels_per_second,
        viewport_width_px,
        pixels_per_second,
        TimelineRulerIntervals::from_wavesurfer_defaults(pixels_per_second),
        0.0,
    )
}

pub fn build_babel_timeline_ruler_for_viewport(
    viewport: TimeRange,
    viewport_width_px: f64,
    min_px_per_sec: f64,
) -> TimelineRulerView {
    let visible_duration = viewport.duration_seconds().max(EPSILON);
    let viewport_width_px = finite_positive(viewport_width_px, DEFAULT_RULER_WIDTH_PX);
    let pixels_per_second = viewport_width_px / visible_duration;
    build_timeline_ruler(
        viewport.end_seconds.max(visible_duration),
        viewport.start_seconds * pixels_per_second,
        viewport_width_px,
        pixels_per_second,
        TimelineRulerIntervals::from_babel_min_px_per_sec(min_px_per_sec),
        0.0,
    )
}

pub fn format_babel_timeline_time(seconds: f64) -> String {
    let millis = (finite_non_negative(seconds) * 1_000.0).round() as u64;
    let minutes = millis / 60_000;
    let remaining_seconds = (millis % 60_000) / 1_000;
    let remaining_millis = millis % 1_000;
    if minutes > 0 {
        format!("{minutes}:{remaining_seconds:02}m")
    } else {
        format!("{remaining_seconds}.{remaining_millis:02}s")
    }
}

fn build_timeline_ruler(
    duration_seconds: f64,
    scroll_left_px: f64,
    viewport_width_px: f64,
    pixels_per_second: f64,
    intervals: TimelineRulerIntervals,
    time_offset_seconds: f64,
) -> TimelineRulerView {
    let duration_seconds = finite_non_negative(duration_seconds);
    let viewport_width_px = finite_positive(viewport_width_px, DEFAULT_RULER_WIDTH_PX);
    let pixels_per_second = finite_positive(pixels_per_second, 1.0);
    let scroll_left_px = finite_non_negative(scroll_left_px);
    let visible_start_seconds = (scroll_left_px / pixels_per_second).clamp(0.0, duration_seconds);
    let visible_end_seconds = ((scroll_left_px + viewport_width_px) / pixels_per_second)
        .clamp(visible_start_seconds, duration_seconds);
    let visible_duration = (visible_end_seconds - visible_start_seconds).max(EPSILON);
    let tick_interval = finite_positive(intervals.time_interval_seconds, 1.0);

    let mut notches = Vec::new();
    let mut time_seconds = (visible_start_seconds / tick_interval).ceil() * tick_interval;
    if time_seconds < visible_start_seconds - EPSILON {
        time_seconds += tick_interval;
    }

    while time_seconds < visible_end_seconds + EPSILON && notches.len() < MAX_VISIBLE_NOTCHES {
        let kind = notch_kind(time_seconds, intervals);
        let label = if kind == TimelineRulerNotchKind::Tick {
            None
        } else {
            Some(format_babel_timeline_time(time_seconds))
        };
        let left_px = (round_to_two_decimals(time_seconds + time_offset_seconds)
            * pixels_per_second)
            - scroll_left_px;
        let left_percent = ((time_seconds - visible_start_seconds) / visible_duration * 100.0)
            .clamp(0.0, 100.0) as f32;
        notches.push(TimelineRulerNotchView {
            time_seconds: round_to_six_decimals(time_seconds),
            left_px,
            left_percent,
            kind,
            label,
        });
        time_seconds += tick_interval;
    }

    TimelineRulerView {
        notches,
        visible_start_seconds,
        visible_end_seconds,
        pixels_per_second,
    }
}

fn notch_kind(time_seconds: f64, intervals: TimelineRulerIntervals) -> TimelineRulerNotchKind {
    if is_rounded_multiple(time_seconds, intervals.primary_label_interval_seconds) {
        TimelineRulerNotchKind::Primary
    } else if is_rounded_multiple(time_seconds, intervals.secondary_label_interval_seconds) {
        TimelineRulerNotchKind::Secondary
    } else {
        TimelineRulerNotchKind::Tick
    }
}

fn is_rounded_multiple(time_seconds: f64, interval_seconds: f64) -> bool {
    let interval = (finite_positive(interval_seconds, 1.0) * 100.0).round() as i64;
    if interval <= 0 {
        return false;
    }
    let time = (finite_non_negative(time_seconds) * 100.0).round() as i64;
    time % interval == 0
}

fn default_time_interval(pixels_per_second: f64) -> f64 {
    if pixels_per_second >= 25.0 {
        1.0
    } else if pixels_per_second * 5.0 >= 25.0 {
        5.0
    } else if pixels_per_second * 15.0 >= 25.0 {
        15.0
    } else {
        (0.5 / pixels_per_second).ceil() * 60.0
    }
}

fn default_primary_label_interval(pixels_per_second: f64) -> f64 {
    if pixels_per_second >= 25.0 {
        10.0
    } else if pixels_per_second * 5.0 >= 25.0 {
        6.0
    } else {
        4.0
    }
}

fn default_secondary_label_interval(pixels_per_second: f64) -> f64 {
    if pixels_per_second >= 25.0 { 5.0 } else { 2.0 }
}

fn round_to_two_decimals(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round_to_six_decimals(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_positive(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TimelineRulerIntervals, TimelineRulerNotchKind, build_babel_timeline_ruler,
        build_babel_timeline_ruler_for_viewport, build_timeline_ruler_for_viewport,
        format_babel_timeline_time,
    };
    use crate::timeline_model::TimeRange;

    #[test]
    fn babel_interval_overrides_match_editor_wrapper() {
        assert_eq!(
            TimelineRulerIntervals::from_babel_min_px_per_sec(200.0),
            TimelineRulerIntervals {
                time_interval_seconds: 0.1,
                primary_label_interval_seconds: 1.0,
                secondary_label_interval_seconds: 0.5,
            }
        );
        assert_eq!(
            TimelineRulerIntervals::from_babel_min_px_per_sec(100.0),
            TimelineRulerIntervals {
                time_interval_seconds: 0.25,
                primary_label_interval_seconds: 2.0,
                secondary_label_interval_seconds: 1.0,
            }
        );
    }

    #[test]
    fn wavesurfer_default_intervals_follow_pixels_per_second_rules() {
        assert_eq!(
            TimelineRulerIntervals::from_wavesurfer_defaults(25.0).time_interval_seconds,
            1.0
        );
        assert_eq!(
            TimelineRulerIntervals::from_wavesurfer_defaults(5.0).time_interval_seconds,
            5.0
        );
        assert_eq!(
            TimelineRulerIntervals::from_wavesurfer_defaults(2.0).time_interval_seconds,
            15.0
        );
        assert_eq!(
            TimelineRulerIntervals::from_wavesurfer_defaults(0.1).time_interval_seconds,
            300.0
        );
    }

    #[test]
    fn babel_timeline_ruler_emits_visible_primary_secondary_and_tick_notches() {
        let ruler = build_babel_timeline_ruler(10.0, 240.0, 200.0, 200.0);

        assert_eq!(ruler.visible_start_seconds, 1.2);
        assert_eq!(ruler.visible_end_seconds, 2.2);
        assert_eq!(ruler.pixels_per_second, 200.0);
        assert!(ruler.notches.iter().any(|notch| {
            notch.time_seconds == 1.5
                && notch.kind == TimelineRulerNotchKind::Secondary
                && notch.label.as_deref() == Some("1.500s")
        }));
        assert!(ruler.notches.iter().any(|notch| {
            notch.time_seconds == 2.0
                && notch.kind == TimelineRulerNotchKind::Primary
                && notch.label.as_deref() == Some("2.00s")
                && (notch.left_percent - 80.0).abs() < 0.001
        }));
        assert!(ruler.notches.iter().any(|notch| {
            notch.time_seconds == 1.6
                && notch.kind == TimelineRulerNotchKind::Tick
                && notch.label.is_none()
        }));
    }

    #[test]
    fn viewport_adapter_uses_visible_width_as_wavesurfer_scroll_window() {
        let ruler = build_timeline_ruler_for_viewport(
            TimeRange {
                start_seconds: 60.0,
                end_seconds: 120.0,
            },
            1_200.0,
        );

        assert_eq!(ruler.visible_start_seconds, 60.0);
        assert_eq!(ruler.visible_end_seconds, 120.0);
        assert_eq!(ruler.pixels_per_second, 20.0);
        assert!(ruler.labels().all(|notch| notch.label.is_some()));
    }

    #[test]
    fn viewport_adapter_can_use_babel_interval_tiers() {
        let ruler = build_babel_timeline_ruler_for_viewport(
            TimeRange {
                start_seconds: 1.2,
                end_seconds: 2.2,
            },
            200.0,
            200.0,
        );

        assert!(ruler.notches.iter().any(|notch| {
            notch.time_seconds == 1.5
                && notch.kind == TimelineRulerNotchKind::Secondary
                && notch.label.as_deref() == Some("1.500s")
        }));
        assert!(ruler.notches.iter().any(|notch| {
            notch.time_seconds == 2.0
                && notch.kind == TimelineRulerNotchKind::Primary
                && notch.label.as_deref() == Some("2.00s")
        }));
    }

    #[test]
    fn babel_timeline_time_formatter_matches_callback_shape() {
        assert_eq!(format_babel_timeline_time(2.0), "2.00s");
        assert_eq!(format_babel_timeline_time(1.5), "1.500s");
        assert_eq!(format_babel_timeline_time(74.25), "1:14m");
    }
}
