pub fn format_mm_ss(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "--:--.--".to_string();
    }

    let clamped = seconds.max(0.0);
    let mut minutes = (clamped / 60.0).floor() as u64;
    let mut remaining_seconds = clamped - (minutes as f64 * 60.0);
    remaining_seconds = (remaining_seconds * 100.0).round() / 100.0;
    if remaining_seconds >= 60.0 {
        minutes += 1;
        remaining_seconds = 0.0;
    }

    format!("{minutes:02}:{remaining_seconds:05.2}")
}
