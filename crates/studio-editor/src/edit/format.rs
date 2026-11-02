//! Time, playback-speed and zoom value conversions shared by the editing UI.

use crate::transcript_model::format_mm_ss;

/// Lowest zoom-slider value (Babel slider units).
pub const ZOOM_SLIDER_MIN: f32 = 10.0;
/// Highest zoom-slider value (Babel slider units).
pub const ZOOM_SLIDER_MAX: f32 = 2000.0;
/// Slider units per zoom factor of `1.0`.
pub const ZOOM_DISPLAY_DIVISOR: f32 = 200.0;
/// Pixels of vertical wheel travel that double (or halve) the zoom.
const TIMELINE_WHEEL_ZOOM_OCTAVE_PIXELS: f32 = 600.0;
/// Playback speeds offered by the speed picker.
pub const SPEED_OPTIONS: &[f32] = &[0.25, 0.5, 0.75, 1.0, 1.5, 2.0];

/// Smallest zoom factor reachable from the slider or the wheel.
pub const MIN_ZOOM_FACTOR: f32 = ZOOM_SLIDER_MIN / ZOOM_DISPLAY_DIVISOR;
/// Largest zoom factor reachable from the slider or the wheel.
pub const MAX_ZOOM_FACTOR: f32 = ZOOM_SLIDER_MAX / ZOOM_DISPLAY_DIVISOR;

/// Formats seconds as the editable `MM:SS.cc` timestamp used by every time cell.
pub fn format_time(seconds: f64) -> String {
    format_mm_ss(seconds)
}

/// Parses a strict `M:SS.cc` / `MM:SS.cc` / `MMM:SS.cc` timestamp (the shapes [`format_time`]
/// produces). Returns `None` for any other shape.
pub fn parse_time(value: &str) -> Option<f64> {
    let (minutes, seconds) = value.trim().split_once(':')?;
    if minutes.is_empty() || minutes.len() > 3 || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (whole_seconds, centiseconds) = seconds.split_once('.')?;
    if whole_seconds.len() != 2
        || centiseconds.len() != 2
        || !whole_seconds.bytes().all(|b| b.is_ascii_digit())
        || !centiseconds.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }

    let minutes = minutes.parse::<u64>().ok()?;
    let seconds = whole_seconds.parse::<u64>().ok()?;
    if seconds >= 60 {
        return None;
    }
    let centiseconds = centiseconds.parse::<u64>().ok()?;
    Some(minutes as f64 * 60.0 + seconds as f64 + centiseconds as f64 / 100.0)
}

/// Parses a speed label such as `1.5x` or `0.75`.
pub fn parse_speed(value: &str) -> Option<f32> {
    value
        .trim()
        .trim_end_matches('x')
        .parse::<f32>()
        .ok()
        .filter(|speed| speed.is_finite() && *speed > 0.0)
}

/// Formats a speed for the speed picker (`1x`, `0.75x`, `1.5x`).
pub fn format_speed(speed: f32) -> String {
    if (speed - 1.0).abs() < f32::EPSILON {
        return "1x".to_string();
    }
    let mut label = format!("{speed:.2}");
    while label.contains('.') && label.ends_with('0') {
        label.pop();
    }
    if label.ends_with('.') {
        label.pop();
    }
    format!("{label}x")
}

/// Labels for [`SPEED_OPTIONS`], in order.
pub fn speed_labels() -> Vec<String> {
    SPEED_OPTIONS
        .iter()
        .map(|speed| format_speed(*speed))
        .collect()
}

/// Converts a zoom-slider value into the editor zoom factor.
pub fn zoom_factor_from_slider(value: f32) -> f32 {
    (value / ZOOM_DISPLAY_DIVISOR).clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR)
}

/// Converts an editor zoom factor into the zoom-slider value.
pub fn slider_from_zoom_factor(zoom: f32) -> f32 {
    (zoom * ZOOM_DISPLAY_DIVISOR).clamp(ZOOM_SLIDER_MIN, ZOOM_SLIDER_MAX)
}

/// Zoom factor after a `secondary`-modified vertical wheel gesture over the timeline.
/// Scrolling up (negative delta) zooms in; one octave per 600px; invalid input keeps the zoom.
pub fn zoom_after_wheel_pixels(current_zoom: f32, delta_y_pixels: f32) -> f32 {
    let zoom = if current_zoom.is_finite() {
        current_zoom
    } else {
        1.0
    }
    .clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
    if !delta_y_pixels.is_finite() {
        return zoom;
    }
    let scale = 2.0_f32.powf(-delta_y_pixels / TIMELINE_WHEEL_ZOOM_OCTAVE_PIXELS);
    (zoom * scale).clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR)
}
