#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferedPlaybackDirection {
    Before,
    After,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferedPlaybackRange {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub timeout_ms: f64,
}

pub fn compute_buffered_playback_range(
    time_seconds: f64,
    buffer_ms: f64,
    direction: BufferedPlaybackDirection,
) -> Option<BufferedPlaybackRange> {
    if !time_seconds.is_finite() || !buffer_ms.is_finite() {
        return None;
    }

    let seconds = buffer_ms.abs() / 1000.0;
    let (start_seconds, end_seconds) = match direction {
        BufferedPlaybackDirection::Before => ((time_seconds - seconds).max(0.0), time_seconds),
        BufferedPlaybackDirection::After => (time_seconds, time_seconds + seconds),
    };

    Some(BufferedPlaybackRange {
        start_seconds,
        end_seconds,
        timeout_ms: (end_seconds - start_seconds) * 1000.0 + 200.0,
    })
}
