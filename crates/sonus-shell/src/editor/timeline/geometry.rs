//! Pure timeline geometry: time ↔ x mapping, waveform hit testing, scrollbar thumbs and the
//! cut selection's length chip.

use studio_editor::timeline_model::{TimeRange, WaveformBlock};
use studio_waveform::{WaveformHit, WaveformHitTarget};

/// Distance (px at 100% interface zoom) from a visible region edge that grabs its resize
/// handle.
pub const HANDLE_HIT_PX: f32 = 6.0;
/// Smallest drawn scrollbar/overview thumb, as a fraction of the track.
pub const MIN_THUMB_FRACTION: f32 = 0.024;

/// Seconds at `x` px inside a surface of `width` px showing `range`.
pub fn time_at(range: TimeRange, x: f32, width: f32) -> f64 {
    range.start_seconds + range.duration_seconds() * fraction_at(x, width) as f64
}

/// Horizontal position (px) of `seconds` inside a surface of `width` px showing `range`.
/// Not clamped: callers clip.
pub fn x_at(range: TimeRange, seconds: f64, width: f32) -> f32 {
    ((seconds - range.start_seconds) / range.duration_seconds() * width as f64) as f32
}

/// `x / width` clamped to `0..=1`.
pub fn fraction_at(x: f32, width: f32) -> f32 {
    if width <= 0.0 || !x.is_finite() {
        return 0.0;
    }
    (x / width).clamp(0.0, 1.0)
}

/// Maps a pointer at `x` over a lane to a [`WaveformHit`] against the lane's region blocks.
///
/// Visible region edges within `handle_hit` px win over bodies (nearest edge first), so a
/// boundary shared by two touching regions stays grabbable; otherwise the topmost (last drawn)
/// body under the pointer is hit.
pub fn hit_test(
    blocks: &[WaveformBlock],
    viewport: TimeRange,
    x: f32,
    width: f32,
    handle_hit: f32,
) -> WaveformHit {
    let x = x.clamp(0.0, width.max(0.0));
    let time_seconds = time_at(viewport, x, width);

    let mut handle: Option<(f32, &WaveformBlock, WaveformHitTarget)> = None;
    for block in blocks {
        let edges = [
            (block.start_seconds, WaveformHitTarget::RegionStartHandle),
            (block.end_seconds, WaveformHitTarget::RegionEndHandle),
        ];
        for (seconds, target) in edges {
            if seconds < viewport.start_seconds || seconds > viewport.end_seconds {
                continue;
            }
            let distance = (x_at(viewport, seconds, width) - x).abs();
            if distance <= handle_hit && handle.is_none_or(|(best, ..)| distance < best) {
                handle = Some((distance, block, target));
            }
        }
    }
    if let Some((_, block, target)) = handle {
        return WaveformHit {
            target,
            region_id: Some(block.segment_id.clone()),
            time_seconds,
        };
    }

    let body = blocks
        .iter()
        .rev()
        .find(|block| block.start_seconds <= time_seconds && time_seconds <= block.end_seconds);
    match body {
        Some(block) => WaveformHit {
            target: WaveformHitTarget::RegionBody,
            region_id: Some(block.segment_id.clone()),
            time_seconds,
        },
        None => WaveformHit {
            target: WaveformHitTarget::Waveform,
            region_id: None,
            time_seconds,
        },
    }
}

/// Cut selection length shown on its chip: hundredths under 10 s (`0.30 s`), tenths under a
/// minute (`12.5 s`), then `m:ss.s`. `unit` is the localized seconds suffix.
pub fn selection_length_label(seconds: f64, unit: &str) -> String {
    let seconds = seconds.max(0.0);
    if seconds < 9.995 {
        format!("{seconds:.2} {unit}")
    } else if seconds < 59.95 {
        format!("{seconds:.1} {unit}")
    } else {
        let tenths = (seconds * 10.0).round() as u64;
        format!("{}:{:02}.{}", tenths / 600, tenths % 600 / 10, tenths % 10)
    }
}

/// Left edge (px) of the cut selection's `chips_width` px chip group, the selection spanning
/// `x0..x1` over a surface `width` px wide: inside its visible part, `pad` from its left edge,
/// when the group fits there; else just past its right edge; else just before its left edge;
/// `None` (hidden) when the group fits nowhere on the surface.
pub fn selection_chip_left(
    x0: f32,
    x1: f32,
    width: f32,
    chips_width: f32,
    pad: f32,
) -> Option<f32> {
    let (left, right) = (x0.max(0.0), x1.min(width));
    if right - left >= chips_width + 2.0 * pad {
        return Some(left + pad);
    }
    let after = x1 + pad;
    if after >= 0.0 && after + chips_width <= width {
        return Some(after);
    }
    let before = x0 - pad - chips_width;
    (before >= 0.0 && x0 - pad <= width).then_some(before)
}

/// A viewport thumb on a track spanning `bounds` (scrollbar, overview window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thumb {
    /// Left edge, fraction of the track.
    pub left: f32,
    /// Real width (viewport share), fraction of the track.
    pub width: f32,
}

impl Thumb {
    /// `None` when the whole timeline is visible.
    pub fn for_viewport(bounds: TimeRange, viewport: TimeRange) -> Option<Self> {
        let total = bounds.duration_seconds();
        let visible = viewport.duration_seconds().min(total);
        if visible >= total {
            return None;
        }
        Some(Self {
            left: ((viewport.start_seconds - bounds.start_seconds) / total).clamp(0.0, 1.0) as f32,
            width: (visible / total).clamp(0.0, 1.0) as f32,
        })
    }

    /// Drawn width: never thinner than [`MIN_THUMB_FRACTION`].
    pub fn drawn_width(self) -> f32 {
        self.width.max(MIN_THUMB_FRACTION)
    }

    /// Drawn left edge, kept inside the track when the drawn width is widened.
    pub fn drawn_left(self) -> f32 {
        self.left.min(1.0 - self.drawn_width()).max(0.0)
    }

    pub fn contains(self, fraction: f32) -> bool {
        let left = self.drawn_left();
        left <= fraction && fraction <= left + self.drawn_width()
    }

    /// Pointer offset from the thumb's left edge kept during a drag. Pressing outside the
    /// thumb grabs its middle, so the thumb jumps under the pointer.
    pub fn grab_offset(self, fraction: f32) -> f32 {
        if self.contains(fraction) {
            fraction - self.left
        } else {
            self.width / 2.0
        }
    }

    /// Viewport start (seconds) for a pointer at `fraction` holding the thumb at `grab_offset`.
    pub fn viewport_start_at(bounds: TimeRange, fraction: f32, grab_offset: f32) -> f64 {
        bounds.start_seconds + bounds.duration_seconds() * f64::from(fraction - grab_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_editor::timeline_model::WaveformPeakSource;

    fn block(id: &str, start: f64, end: f64) -> WaveformBlock {
        WaveformBlock {
            segment_id: id.into(),
            tooltip: String::new(),
            is_transcription: true,
            start_seconds: start,
            end_seconds: end,
            left_percent: 0.0,
            width_percent: 0.0,
            peaks: Vec::new().into(),
            peak_source: WaveformPeakSource::Synthetic,
        }
    }

    const VIEW: TimeRange = TimeRange {
        start_seconds: 10.0,
        end_seconds: 20.0,
    };

    #[test]
    fn shared_boundary_grabs_the_nearest_edge_handle() {
        let blocks = [block("a", 11.0, 15.0), block("b", 15.0, 18.0)];
        // 1000 px over 10 s: 15 s sits at x = 500.
        let left_of_edge = hit_test(&blocks, VIEW, 498.0, 1000.0, HANDLE_HIT_PX);
        assert_eq!(left_of_edge.target, WaveformHitTarget::RegionEndHandle);
        assert_eq!(left_of_edge.region_id.as_deref(), Some("a"));

        let body = hit_test(&blocks, VIEW, 700.0, 1000.0, HANDLE_HIT_PX);
        assert_eq!(body.target, WaveformHitTarget::RegionBody);
        assert_eq!(body.region_id.as_deref(), Some("b"));
        assert!((body.time_seconds - 17.0).abs() < 1e-4);
    }

    #[test]
    fn edges_outside_the_viewport_are_not_handles() {
        let blocks = [block("a", 5.0, 12.0)];
        let hit = hit_test(&blocks, VIEW, 0.0, 1000.0, HANDLE_HIT_PX);
        assert_eq!(hit.target, WaveformHitTarget::RegionBody);
        let empty = hit_test(&blocks, VIEW, 900.0, 1000.0, HANDLE_HIT_PX);
        assert_eq!(empty.target, WaveformHitTarget::Waveform);
        assert_eq!(empty.region_id, None);
    }

    #[test]
    fn thumb_drag_keeps_the_grab_point_under_the_pointer() {
        let bounds = TimeRange {
            start_seconds: 0.0,
            end_seconds: 100.0,
        };
        let thumb = Thumb::for_viewport(bounds, VIEW).unwrap();
        let grab = thumb.grab_offset(0.15);
        assert!((grab - 0.05).abs() < 1e-6);
        let start = Thumb::viewport_start_at(bounds, 0.55, grab);
        assert!((start - 50.0).abs() < 1e-4);

        // Track click outside the thumb centres it on the pointer.
        let grab = thumb.grab_offset(0.8);
        assert!((Thumb::viewport_start_at(bounds, 0.8, grab) - 75.0).abs() < 1e-4);
        assert_eq!(Thumb::for_viewport(bounds, bounds), None);
    }

    #[test]
    fn selection_length_label_precision_follows_magnitude() {
        assert_eq!(selection_length_label(0.3, "s"), "0.30 s");
        assert_eq!(selection_length_label(1.254, "s"), "1.25 s");
        assert_eq!(selection_length_label(9.999, "s"), "10.0 s");
        assert_eq!(selection_length_label(42.34, "s"), "42.3 s");
        assert_eq!(selection_length_label(59.97, "s"), "1:00.0");
        assert_eq!(selection_length_label(125.26, "s"), "2:05.3");
        assert_eq!(selection_length_label(-1.0, "s"), "0.00 s");
    }

    #[test]
    fn selection_chips_sit_inside_then_after_then_before_the_selection() {
        // Wide enough: inside, padded from the visible left edge.
        assert_eq!(
            selection_chip_left(100.0, 300.0, 1000.0, 80.0, 4.0),
            Some(104.0)
        );
        assert_eq!(
            selection_chip_left(-50.0, 300.0, 1000.0, 80.0, 4.0),
            Some(4.0)
        );
        // Too narrow: just past the right edge.
        assert_eq!(
            selection_chip_left(100.0, 130.0, 1000.0, 80.0, 4.0),
            Some(134.0)
        );
        // No room on the right: just before the left edge.
        assert_eq!(
            selection_chip_left(900.0, 950.0, 1000.0, 80.0, 4.0),
            Some(816.0)
        );
        // Room nowhere: hidden.
        assert_eq!(selection_chip_left(40.0, 70.0, 100.0, 80.0, 4.0), None);
    }
}
