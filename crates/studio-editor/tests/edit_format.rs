//! Time, playback-speed and zoom conversions (ported from the iced app's formatting.rs).

use studio_editor::edit::format::{
    MAX_ZOOM_FACTOR, MIN_ZOOM_FACTOR, SPEED_OPTIONS, ZOOM_SLIDER_MAX, ZOOM_SLIDER_MIN,
    format_speed, format_time, parse_speed, parse_time, slider_from_zoom_factor, speed_labels,
    zoom_after_wheel_pixels, zoom_factor_from_slider,
};

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-5,
        "expected {expected}, got {actual}"
    );
}

// ---- time -------------------------------------------------------------------------------

#[test]
fn format_time_uses_two_digit_minutes_seconds_and_centiseconds() {
    assert_eq!(format_time(0.0), "00:00.00");
    assert_eq!(format_time(1.23), "00:01.23");
    assert_eq!(format_time(61.5), "01:01.50");
    assert_eq!(format_time(754.32), "12:34.32");
    assert_eq!(format_time(5999.99), "99:59.99");
}

#[test]
fn format_time_rounds_centiseconds_and_carries_into_minutes() {
    assert_eq!(format_time(1.234), "00:01.23");
    assert_eq!(format_time(1.236), "00:01.24");
    assert_eq!(format_time(59.999), "01:00.00");
}

#[test]
fn format_time_clamps_negative_and_marks_non_finite() {
    assert_eq!(format_time(-5.0), "00:00.00");
    assert_eq!(format_time(f64::NAN), "--:--.--");
    assert_eq!(format_time(f64::INFINITY), "--:--.--");
}

#[test]
fn parse_time_accepts_strict_minutes_seconds_centiseconds() {
    assert_eq!(parse_time("00:00.00"), Some(0.0));
    assert_eq!(parse_time("01:01.50"), Some(61.5));
    assert_eq!(parse_time("1:01.50"), Some(61.5));
    assert_eq!(parse_time("99:59.99"), Some(99.0 * 60.0 + 59.0 + 0.99));
    assert_eq!(parse_time(" 02:03.04 "), Some(123.0 + 0.04));
}

#[test]
fn parse_time_rejects_other_shapes() {
    for value in [
        "",
        "abc",
        "1.5",
        "01:00",
        ":00.00",
        "1:1.50",
        "01:60.00",
        "1000:00.00",
        "01:00.5",
        "01:00.500",
        "01:00,50",
        "01:0a.00",
        "0a:00.00",
        "-1:00.00",
        "+1:00.00",
        "01:-1.00",
        "01:00.-1",
        "01:02:03.00",
    ] {
        assert_eq!(parse_time(value), None, "`{value}` must be rejected");
    }
}

#[test]
fn long_recording_times_round_trip_with_three_digit_minutes() {
    let formatted = format_time(6_000.5);
    assert_eq!(formatted, "100:00.50");
    assert_eq!(parse_time(&formatted), Some(6_000.5));
}

#[test]
fn parse_of_formatted_time_round_trips_at_centisecond_precision() {
    for seconds in [0.0, 1.23, 59.99, 60.0, 61.5, 754.32, 3599.99, 5999.99] {
        let parsed = parse_time(&format_time(seconds)).expect("formatted time parses");
        assert!(
            (parsed - seconds).abs() < 1e-9,
            "{seconds} round-tripped to {parsed}"
        );
    }
}

#[test]
fn format_of_parsed_time_normalizes_to_two_digit_minutes() {
    for text in ["00:00.00", "01:01.50", "12:34.32", "99:59.99"] {
        assert_eq!(format_time(parse_time(text).expect("valid")), text);
    }
    assert_eq!(
        format_time(parse_time("1:01.50").expect("valid")),
        "01:01.50"
    );
}

// ---- speed ------------------------------------------------------------------------------

#[test]
fn speed_options_are_the_babel_steps() {
    assert_eq!(SPEED_OPTIONS, &[0.25, 0.5, 0.75, 1.0, 1.5, 2.0]);
}

#[test]
fn speed_labels_drop_trailing_zeros() {
    assert_eq!(
        speed_labels(),
        vec!["0.25x", "0.5x", "0.75x", "1x", "1.5x", "2x"]
    );
}

#[test]
fn format_speed_handles_values_outside_the_options() {
    assert_eq!(format_speed(1.0), "1x");
    assert_eq!(format_speed(1.25), "1.25x");
    assert_eq!(format_speed(3.0), "3x");
    assert_eq!(format_speed(0.1), "0.1x");
}

#[test]
fn parse_speed_accepts_labels_and_bare_numbers() {
    assert_eq!(parse_speed("1.5x"), Some(1.5));
    assert_eq!(parse_speed("0.75"), Some(0.75));
    assert_eq!(parse_speed(" 2x "), Some(2.0));
    assert_eq!(parse_speed("1x"), Some(1.0));
}

#[test]
fn parse_speed_rejects_non_positive_and_non_finite_values() {
    for value in ["", "x", "abc", "0x", "0", "-1x", "inf", "NaNx"] {
        assert_eq!(parse_speed(value), None, "`{value}` must be rejected");
    }
}

#[test]
fn speed_labels_round_trip_through_parse_speed() {
    for (speed, label) in SPEED_OPTIONS.iter().zip(speed_labels()) {
        assert_eq!(parse_speed(&label), Some(*speed));
    }
}

// ---- zoom slider ------------------------------------------------------------------------

#[test]
fn zoom_limits_follow_slider_range() {
    assert_eq!(ZOOM_SLIDER_MIN, 10.0);
    assert_eq!(ZOOM_SLIDER_MAX, 2000.0);
    assert_close(MIN_ZOOM_FACTOR, 0.05);
    assert_close(MAX_ZOOM_FACTOR, 10.0);
}

#[test]
fn zoom_factor_from_slider_divides_by_two_hundred_and_clamps() {
    assert_eq!(zoom_factor_from_slider(200.0), 1.0);
    assert_eq!(zoom_factor_from_slider(400.0), 2.0);
    assert_eq!(zoom_factor_from_slider(ZOOM_SLIDER_MIN), MIN_ZOOM_FACTOR);
    assert_eq!(zoom_factor_from_slider(ZOOM_SLIDER_MAX), MAX_ZOOM_FACTOR);
    assert_eq!(zoom_factor_from_slider(1.0), MIN_ZOOM_FACTOR);
    assert_eq!(zoom_factor_from_slider(5000.0), MAX_ZOOM_FACTOR);
}

#[test]
fn slider_from_zoom_factor_multiplies_by_two_hundred_and_clamps() {
    assert_eq!(slider_from_zoom_factor(1.0), 200.0);
    assert_eq!(slider_from_zoom_factor(2.0), 400.0);
    assert_eq!(slider_from_zoom_factor(0.01), ZOOM_SLIDER_MIN);
    assert_eq!(slider_from_zoom_factor(20.0), ZOOM_SLIDER_MAX);
}

#[test]
fn zoom_slider_round_trips_inside_range() {
    for slider in [10.0, 50.0, 100.0, 200.0, 333.0, 1000.0, 2000.0] {
        let round_trip = slider_from_zoom_factor(zoom_factor_from_slider(slider));
        assert!(
            (round_trip - slider).abs() < 1e-3,
            "{slider} round-tripped to {round_trip}"
        );
    }
}

// ---- wheel zoom -------------------------------------------------------------------------

#[test]
fn timeline_wheel_zoom_uses_pixel_octaves_and_clamps_invalid_input() {
    let zoom = zoom_after_wheel_pixels(1.0, -120.0);
    let expected = 2.0_f32.powf(120.0 / 600.0);
    assert!((zoom - expected).abs() < 0.000_001);

    assert_eq!(zoom_after_wheel_pixels(1.0, 10_000.0), MIN_ZOOM_FACTOR);
    assert_eq!(zoom_after_wheel_pixels(1.0, -10_000.0), MAX_ZOOM_FACTOR);
    assert_eq!(zoom_after_wheel_pixels(1.25, f32::INFINITY), 1.25);
    assert_eq!(zoom_after_wheel_pixels(1.25, f32::NAN), 1.25);
}

#[test]
fn timeline_wheel_zoom_doubles_per_six_hundred_pixels_up_and_halves_down() {
    assert_close(zoom_after_wheel_pixels(1.0, -600.0), 2.0);
    assert_close(zoom_after_wheel_pixels(1.0, 600.0), 0.5);
    assert_close(zoom_after_wheel_pixels(2.0, -1200.0), 8.0);
    assert_eq!(zoom_after_wheel_pixels(1.5, 0.0), 1.5);
}

#[test]
fn timeline_wheel_zoom_in_then_out_restores_zoom() {
    let zoomed_in = zoom_after_wheel_pixels(1.0, -240.0);
    assert!(zoomed_in > 1.0);
    assert_close(zoom_after_wheel_pixels(zoomed_in, 240.0), 1.0);
}

#[test]
fn timeline_wheel_zoom_sanitizes_current_zoom() {
    assert_eq!(zoom_after_wheel_pixels(f32::NAN, 0.0), 1.0);
    assert_eq!(zoom_after_wheel_pixels(f32::INFINITY, 0.0), 1.0);
    assert_eq!(zoom_after_wheel_pixels(100.0, 0.0), MAX_ZOOM_FACTOR);
    assert_eq!(zoom_after_wheel_pixels(0.0, 0.0), MIN_ZOOM_FACTOR);
    assert_eq!(zoom_after_wheel_pixels(-3.0, f32::NAN), MIN_ZOOM_FACTOR);
}

#[test]
fn timeline_wheel_zoom_stays_clamped_at_limits() {
    assert_eq!(
        zoom_after_wheel_pixels(MAX_ZOOM_FACTOR, -600.0),
        MAX_ZOOM_FACTOR
    );
    assert_eq!(
        zoom_after_wheel_pixels(MIN_ZOOM_FACTOR, 600.0),
        MIN_ZOOM_FACTOR
    );
}
