use std::{fs, path::Path};

use studio_app::format_mm_ss;
use studio_core::Segment;

use super::SegmentDraft;

pub(crate) const BABEL_ZOOM_SLIDER_MIN: f32 = 10.0;
pub(crate) const BABEL_ZOOM_SLIDER_MAX: f32 = 2000.0;
pub(crate) const BABEL_ZOOM_DISPLAY_DIVISOR: f32 = 200.0;
const TIMELINE_WHEEL_ZOOM_OCTAVE_PIXELS: f32 = 600.0;
pub(crate) const BABEL_SPEED_OPTIONS: &[f32] = &[0.25, 0.5, 0.75, 1.0, 1.5, 2.0];
pub(crate) fn parse_seconds(_label: &str, value: &str) -> Result<f64, String> {
    let trimmed = value.trim();
    let Some((minutes, seconds)) = trimmed.split_once(':') else {
        return Err("Invalid format".to_string());
    };
    if minutes.is_empty()
        || minutes.len() > 2
        || !minutes.chars().all(|character| character.is_ascii_digit())
    {
        return Err("Invalid format".to_string());
    }
    let Some((whole_seconds, centiseconds)) = seconds.split_once('.') else {
        return Err("Invalid format".to_string());
    };
    if whole_seconds.len() != 2
        || centiseconds.len() != 2
        || !whole_seconds
            .chars()
            .all(|character| character.is_ascii_digit())
        || !centiseconds
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err("Invalid format".to_string());
    }

    let minutes = minutes
        .parse::<u64>()
        .map_err(|_| "Invalid format".to_string())?;
    let seconds = whole_seconds
        .parse::<u64>()
        .map_err(|_| "Invalid format".to_string())?;
    if seconds >= 60 {
        return Err("Invalid format".to_string());
    }
    let centiseconds = centiseconds
        .parse::<u64>()
        .map_err(|_| "Invalid format".to_string())?;

    Ok(minutes as f64 * 60.0 + seconds as f64 + centiseconds as f64 / 100.0)
}

pub(crate) fn parse_draft_times(
    segment: &Segment,
    draft: &SegmentDraft,
) -> Result<(f64, f64), String> {
    let start_seconds = if draft.start_seconds == format_seconds_for_edit(segment.start_seconds) {
        segment.start_seconds
    } else {
        parse_seconds("Start", &draft.start_seconds)?
    };
    let end_seconds = if draft.end_seconds == format_seconds_for_edit(segment.end_seconds) {
        segment.end_seconds
    } else {
        parse_seconds("End", &draft.end_seconds)?
    };
    Ok((start_seconds, end_seconds))
}

pub(crate) fn parse_speed(value: &str) -> Option<f32> {
    value.trim_end_matches('x').parse::<f32>().ok()
}

pub(crate) fn format_speed(speed: f32) -> String {
    if (speed - 1.0).abs() < f32::EPSILON {
        "1x".to_string()
    } else {
        let mut label = format!("{speed:.2}");
        while label.contains('.') && label.ends_with('0') {
            label.pop();
        }
        if label.ends_with('.') {
            label.pop();
        }
        format!("{label}x")
    }
}

pub(crate) fn babel_speed_labels() -> Vec<String> {
    BABEL_SPEED_OPTIONS
        .iter()
        .map(|speed| format_speed(*speed))
        .collect()
}

pub(crate) fn zoom_factor_from_babel_slider(value: f32) -> f32 {
    (value / BABEL_ZOOM_DISPLAY_DIVISOR).clamp(
        BABEL_ZOOM_SLIDER_MIN / BABEL_ZOOM_DISPLAY_DIVISOR,
        BABEL_ZOOM_SLIDER_MAX / BABEL_ZOOM_DISPLAY_DIVISOR,
    )
}

pub(crate) fn babel_slider_from_zoom_factor(zoom: f32) -> f32 {
    (zoom * BABEL_ZOOM_DISPLAY_DIVISOR).clamp(BABEL_ZOOM_SLIDER_MIN, BABEL_ZOOM_SLIDER_MAX)
}

pub(crate) fn zoom_factor_after_timeline_wheel_pixels(
    current_zoom: f32,
    delta_y_pixels: f32,
) -> f32 {
    let zoom = if current_zoom.is_finite() {
        current_zoom
    } else {
        1.0
    }
    .clamp(
        BABEL_ZOOM_SLIDER_MIN / BABEL_ZOOM_DISPLAY_DIVISOR,
        BABEL_ZOOM_SLIDER_MAX / BABEL_ZOOM_DISPLAY_DIVISOR,
    );
    if !delta_y_pixels.is_finite() {
        return zoom;
    }

    let scale = 2.0_f32.powf(-delta_y_pixels / TIMELINE_WHEEL_ZOOM_OCTAVE_PIXELS);
    (zoom * scale).clamp(
        BABEL_ZOOM_SLIDER_MIN / BABEL_ZOOM_DISPLAY_DIVISOR,
        BABEL_ZOOM_SLIDER_MAX / BABEL_ZOOM_DISPLAY_DIVISOR,
    )
}

pub(crate) fn format_seconds_for_edit(value: f64) -> String {
    format_mm_ss(value)
}

pub(crate) fn write_text_file(path: &str, contents: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return Err("path is empty".to_string());
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create `{}`: {error}", parent.display()))?;
    }

    fs::write(path, contents)
        .map_err(|error| format!("failed to write `{}`: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::zoom_factor_after_timeline_wheel_pixels;

    #[test]
    fn timeline_wheel_zoom_uses_pixel_octaves_and_clamps_invalid_input() {
        let zoom = zoom_factor_after_timeline_wheel_pixels(1.0, -120.0);
        let expected = 2.0_f32.powf(120.0 / 600.0);
        assert!((zoom - expected).abs() < 0.000_001);

        assert_eq!(zoom_factor_after_timeline_wheel_pixels(1.0, 10_000.0), 0.05);
        assert_eq!(
            zoom_factor_after_timeline_wheel_pixels(1.0, -10_000.0),
            10.0
        );
        assert_eq!(
            zoom_factor_after_timeline_wheel_pixels(1.25, f32::INFINITY),
            1.25
        );
        assert_eq!(
            zoom_factor_after_timeline_wheel_pixels(1.25, f32::NAN),
            1.25
        );
    }
}
