//! Babel's browser waveform, reproduced bit for bit.
//!
//! Babel's editor streams each speaker WAV and folds it, as it arrives, into an 8 kHz mono
//! *display signal*: the channel average through a 4th-order Butterworth low-pass at 3.6 kHz,
//! linearly interpolated down to 8 kHz. It hands that signal to wavesurfer.js as peaks
//! (`load(url, [samples], duration)`), and wavesurfer's line renderer draws it with
//! `barHeight` = the lane's vertical zoom (20 by default) and `normalize: false`:
//!
//! - the content is `ceil(duration * minPxPerSec)` CSS px wide, cut into canvases one scroll
//!   viewport (`clientWidth`, at most 8000 px) wide; a canvas is `round(css px * pixelRatio)`
//!   device px and draws only its slice `floor(offset / width * len)..floor(end / width * len)`;
//! - inside a canvas, sample `e` belongs to the device column starting at `round(e * width /
//!   n)`; each column puts one vertex at its left edge, `half ∓ (round(peak * half *
//!   barHeight) || 1)`, and the outline returns to the centre at the canvas' right edge. Top
//!   and bottom are mirrored subpaths filled together; the canvas clips what overshoots.
//!
//! [`DisplaySignal`] is the first step and [`LineWaveform`] the second. Canvas columns depend
//! only on the zoom, so they are cached per zoom: panning and playback rebuild only the
//! outline of the visible columns.

use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
};

use crate::DecodedAudio;

/// Sample rate of Babel's display signal.
pub const DISPLAY_SAMPLE_RATE: u32 = 8_000;
const LOW_PASS_HZ: f64 = 3_600.0;
/// Q of the two biquads of a 4th-order Butterworth low-pass, as Babel rounds them.
const BUTTERWORTH_Q: [f64; 2] = [0.5412, 1.3066];
/// wavesurfer's canvas width cap (CSS px).
const MAX_CANVAS_WIDTH: f64 = 8_000.0;
/// Canvases kept per zoom: the visible pair plus panning slack.
const CACHED_CANVASES: usize = 32;

/// The 8 kHz mono signal Babel's wavesurfer draws, with the source's true duration (the
/// signal is `ceil(8000 * duration)` samples long).
#[derive(Clone, Debug)]
pub struct DisplaySignal {
    samples: Arc<[f32]>,
    duration_seconds: f64,
}

impl PartialEq for DisplaySignal {
    fn eq(&self, other: &Self) -> bool {
        self.duration_seconds == other.duration_seconds
            && (Arc::ptr_eq(&self.samples, &other.samples) || self.samples == other.samples)
    }
}

impl DisplaySignal {
    /// Babel's streaming downsampler over decoded PCM.
    pub fn from_decoded(audio: &DecodedAudio) -> Self {
        let sample_rate = f64::from(audio.sample_rate());
        let duration_seconds = audio.duration();
        let length = (f64::from(DISPLAY_SAMPLE_RATE) * duration_seconds).ceil() as usize;
        let mut samples = vec![0.0_f32; length];
        let step = sample_rate / f64::from(DISPLAY_SAMPLE_RATE);
        let mut filters = BUTTERWORTH_Q.map(|q| Biquad::low_pass(sample_rate, LOW_PASS_HZ, q));
        let filtered = sample_rate > f64::from(DISPLAY_SAMPLE_RATE);
        let channels: Vec<&[f32]> = (0..audio.number_of_channels())
            .filter_map(|channel| audio.channel_data(channel))
            .collect();
        let channel_count = channels.len() as f64;

        let mut previous = 0.0_f64;
        let mut next = 0_usize;
        for frame in 0..audio.frame_count() {
            let mut sum = 0.0_f64;
            for channel in &channels {
                sum += f64::from(channel[frame]);
            }
            let mut value = sum / channel_count;
            if filtered {
                for filter in &mut filters {
                    value = filter.process(value);
                }
            }
            let input = frame as f64;
            while next < length && next as f64 * step <= input {
                samples[next] = if frame == 0 {
                    value
                } else {
                    previous + (value - previous) * (next as f64 * step - (input - 1.0))
                } as f32;
                next += 1;
            }
            previous = value;
        }

        Self {
            samples: samples.into(),
            duration_seconds,
        }
    }

    /// Peaks handed to wavesurfer as they are (`load(url, [peaks], duration)`): like
    /// wavesurfer's `createBuffer`, a channel reaching beyond ±1 is scaled to its maximum.
    pub fn from_peaks(mut peaks: Vec<f32>, duration_seconds: f64) -> Result<Self, String> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err("display signal duration must be greater than zero".to_string());
        }
        if peaks.is_empty() {
            return Err("display signal cannot be empty".to_string());
        }
        if peaks.iter().any(|peak| *peak > 1.0 || *peak < -1.0) {
            let max = peaks.iter().fold(0.0_f32, |max, peak| max.max(peak.abs()));
            for peak in &mut peaks {
                *peak = (f64::from(*peak) / f64::from(max)) as f32;
            }
        }
        Ok(Self {
            samples: peaks.into(),
            duration_seconds,
        })
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn samples_shared(&self) -> Arc<[f32]> {
        Arc::clone(&self.samples)
    }

    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of_val(self.samples.as_ref())
    }
}

/// RBJ low-pass biquad in Babel's (direct form I, f64) formulation.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn low_pass(sample_rate: f64, cutoff_hz: f64, q: f64) -> Self {
        let omega = 2.0 * std::f64::consts::PI * cutoff_hz / sample_rate;
        let cos = omega.cos();
        let alpha = omega.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b0 = (1.0 - cos) / 2.0 / a0;
        Self {
            b0,
            b1: (1.0 - cos) / a0,
            b2: b0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, input: f64) -> f64 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

/// Where and how large a lane's wavesurfer is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineView {
    /// Zoom: CSS px per second (`minPxPerSec`).
    pub min_px_per_sec: f64,
    /// Scroll viewport width in CSS px (`clientWidth`, a whole number).
    pub client_width: f64,
    /// Canvas height in CSS px.
    pub height: f64,
    /// Device px per CSS px (`devicePixelRatio`; wavesurfer never uses less than 1).
    pub pixel_ratio: f64,
    /// Horizontal scroll in CSS px (`scrollLeft`).
    pub scroll_left: f64,
}

impl LineView {
    /// The view showing `start..end` seconds of a `duration`-second waveform across `width`
    /// CSS px. Positions follow wavesurfer (`x = t / duration * ceil(duration *
    /// minPxPerSec)`), so the scroll is the content x of `start`, snapped to device px like
    /// the browser's `scrollLeft`.
    pub fn of_window(
        duration_seconds: f64,
        (start_seconds, end_seconds): (f64, f64),
        (width, height): (f64, f64),
        pixel_ratio: f64,
    ) -> Self {
        let mut view = Self {
            min_px_per_sec: width / (end_seconds - start_seconds),
            client_width: js_round(width),
            height,
            pixel_ratio,
            scroll_left: 0.0,
        };
        let ratio = view.pixel_ratio();
        let scroll_width = (duration_seconds * view.min_px_per_sec).ceil();
        view.scroll_left =
            js_round(start_seconds / duration_seconds * scroll_width * ratio) / ratio;
        view
    }

    fn pixel_ratio(&self) -> f64 {
        if self.pixel_ratio.is_finite() {
            self.pixel_ratio.max(1.0)
        } else {
            1.0
        }
    }
}

/// wavesurfer's canvas split of the content at one zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasLayout {
    /// `ceil(duration * minPxPerSec)` CSS px.
    pub scroll_width: f64,
    /// The width the canvases split (`scroll_width * ratio / ratio`, as wavesurfer computes it).
    pub total_width: f64,
    /// Width of every canvas but the last (CSS px).
    pub canvas_width: f64,
    pub canvas_count: usize,
    pub pixel_ratio: f64,
}

impl CanvasLayout {
    pub fn new(duration_seconds: f64, view: &LineView) -> Option<Self> {
        let pixel_ratio = view.pixel_ratio();
        let scroll_width = (duration_seconds * view.min_px_per_sec).ceil();
        if !scroll_width.is_finite() || scroll_width <= 0.0 {
            return None;
        }
        let total_width = scroll_width * pixel_ratio / pixel_ratio;
        let canvas_width = MAX_CANVAS_WIDTH.min(view.client_width).min(total_width);
        if !canvas_width.is_finite() || canvas_width <= 0.0 {
            return None;
        }
        Some(Self {
            scroll_width,
            total_width,
            canvas_width,
            canvas_count: (total_width / canvas_width).ceil() as usize,
            pixel_ratio,
        })
    }

    /// CSS `(offset, width)` of canvas `index`.
    pub fn canvas_span(&self, index: usize) -> (f64, f64) {
        let offset = index as f64 * self.canvas_width;
        (offset, (self.total_width - offset).min(self.canvas_width))
    }

    /// Canvases overlapping `left..left + width` (CSS px of content).
    pub fn canvases_between(&self, left: f64, width: f64) -> Range<usize> {
        let first = (left / self.canvas_width).floor().max(0.0) as usize;
        let end = ((left + width) / self.canvas_width).ceil().max(0.0) as usize;
        first.min(self.canvas_count)..end.min(self.canvas_count)
    }

    /// Samples of a `length`-sample signal that canvas `index` draws.
    pub fn canvas_samples(&self, index: usize, length: usize) -> Range<usize> {
        let (offset, width) = self.canvas_span(index);
        let length_f = length as f64;
        let start = ((offset / self.total_width * length_f).floor() as usize).min(length);
        let end = (((offset + width) / self.total_width * length_f).floor() as usize).min(length);
        start..end.max(start)
    }
}

/// One device column of a canvas: its left edge and the loudest |sample| in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineColumn {
    pub x: u32,
    pub peak: f32,
}

/// The columns of one canvas, left to right; the outline returns to the centre at `end_x`.
#[derive(Clone, Debug, PartialEq)]
pub struct LineCanvasColumns {
    pub columns: Vec<LineColumn>,
    pub end_x: u32,
}

impl LineCanvasColumns {
    /// wavesurfer's `renderLineWaveform` column scan of one canvas' samples.
    pub fn scan(samples: &[f32], device_width: u32) -> Self {
        let count = samples.len();
        let per_sample = if count > 0 {
            f64::from(device_width) / count as f64
        } else {
            0.0
        };
        let mut columns = Vec::with_capacity((device_width as usize).min(count));
        let mut column_x = 0_u32;
        let mut peak = 0.0_f32;
        for index in 0..=count {
            let x = js_round(index as f64 * per_sample) as u32;
            if x > column_x {
                columns.push(LineColumn { x: column_x, peak });
                column_x = x;
                peak = 0.0;
            }
            if let Some(sample) = samples.get(index) {
                let magnitude = sample.abs();
                if magnitude > peak {
                    peak = magnitude;
                }
            }
        }
        Self {
            columns,
            end_x: column_x,
        }
    }

    /// Columns to draw for the device range `x0..x1`, with one neighbour on each side so the
    /// slopes into the range stay exact.
    pub fn columns_between(&self, x0: f64, x1: f64) -> Range<usize> {
        let first = self
            .columns
            .partition_point(|column| f64::from(column.x) <= x0)
            .saturating_sub(1);
        let end = self
            .columns
            .partition_point(|column| f64::from(column.x) < x1)
            .saturating_add(1)
            .min(self.columns.len());
        first..end.max(first)
    }
}

/// One canvas placed in a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct LineFrameCanvas {
    pub index: usize,
    /// CSS px from the content start (`style.left`).
    pub left: f64,
    /// CSS width.
    pub width: f64,
    pub device_width: u32,
    pub device_height: u32,
    /// Device x of the canvas' left edge relative to the viewport's left edge.
    pub viewport_x: f64,
    /// Device px per canvas px (canvases are stretched to their CSS size).
    pub x_scale: f64,
    pub y_scale: f64,
    pub columns: Arc<LineCanvasColumns>,
}

impl LineFrameCanvas {
    /// Vertical centre in canvas px.
    pub fn half(&self) -> f64 {
        f64::from(self.device_height) / 2.0
    }

    /// Distance of a column's vertex from the centre in canvas px (at least one).
    pub fn amplitude(&self, peak: f32, bar_height: f64) -> f64 {
        let amplitude = js_round(f64::from(peak) * self.half() * bar_height);
        if amplitude == 0.0 || amplitude.is_nan() {
            1.0
        } else {
            amplitude
        }
    }

    /// One half of the outline in canvas px, as wavesurfer strokes it: up from the centre at
    /// the first column's edge, through each column's vertex, back to the centre where the
    /// next column starts (the canvas' right edge after the last column). `direction` is -1
    /// for the top half and 1 for the bottom half.
    pub fn outline(
        &self,
        columns: Range<usize>,
        bar_height: f64,
        direction: f64,
    ) -> impl Iterator<Item = (f64, f64)> + '_ {
        let half = self.half();
        let all = &self.columns.columns;
        let start_x = all.get(columns.start).map_or(0, |column| column.x);
        let end_x = all
            .get(columns.end)
            .map_or(self.columns.end_x, |column| column.x);
        std::iter::once((f64::from(start_x), half))
            .chain(all[columns].iter().map(move |column| {
                (
                    f64::from(column.x),
                    half + self.amplitude(column.peak, bar_height) * direction,
                )
            }))
            .chain(std::iter::once((f64::from(end_x), half)))
    }
}

/// The canvases of a viewport.
#[derive(Clone, Debug, PartialEq)]
pub struct LineFrame {
    pub layout: CanvasLayout,
    pub canvases: Vec<LineFrameCanvas>,
    scroll_left: f64,
    duration_seconds: f64,
}

impl LineFrame {
    /// Device x of `seconds` relative to the viewport's left edge (wavesurfer's progress and
    /// cursor position).
    pub fn viewport_x_of(&self, seconds: f64) -> f64 {
        (seconds / self.duration_seconds * self.layout.scroll_width - self.scroll_left)
            * self.layout.pixel_ratio
    }
}

/// A lane's display signal with its per-zoom canvas columns.
#[derive(Debug)]
pub struct LineWaveform {
    signal: DisplaySignal,
    canvases: Mutex<CanvasCache>,
}

#[derive(Debug, Default)]
struct CanvasCache {
    key: Option<[u64; 3]>,
    canvases: HashMap<usize, Arc<LineCanvasColumns>>,
}

/// Equal when drawing the same signal (the canvas cache is derived state).
impl PartialEq for LineWaveform {
    fn eq(&self, other: &Self) -> bool {
        self.signal == other.signal
    }
}

impl LineWaveform {
    pub fn new(signal: DisplaySignal) -> Self {
        Self {
            signal,
            canvases: Mutex::default(),
        }
    }

    pub fn signal(&self) -> &DisplaySignal {
        &self.signal
    }

    pub fn duration_seconds(&self) -> f64 {
        self.signal.duration_seconds
    }

    /// The canvases visible in `view`, columns computed once per zoom.
    pub fn frame(&self, view: &LineView) -> Option<LineFrame> {
        let layout = CanvasLayout::new(self.signal.duration_seconds, view)?;
        let key = [
            layout.total_width.to_bits(),
            layout.canvas_width.to_bits(),
            layout.pixel_ratio.to_bits(),
        ];
        let mut cache = self
            .canvases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.key != Some(key) {
            cache.key = Some(key);
            cache.canvases.clear();
        }
        let visible = layout.canvases_between(view.scroll_left, view.client_width);
        if cache.canvases.len() + visible.len() > CACHED_CANVASES {
            cache.canvases.retain(|index, _| visible.contains(index));
        }
        Some(self.build_frame(layout, view, |index, device_width| {
            Arc::clone(
                cache
                    .canvases
                    .entry(index)
                    .or_insert_with(|| self.scan_canvas(&layout, index, device_width)),
            )
        }))
    }

    /// Like [`Self::frame`] for one-off views (a magnifier), leaving the zoom cache alone.
    pub fn frame_uncached(&self, view: &LineView) -> Option<LineFrame> {
        let layout = CanvasLayout::new(self.signal.duration_seconds, view)?;
        Some(self.build_frame(layout, view, |index, device_width| {
            self.scan_canvas(&layout, index, device_width)
        }))
    }

    fn scan_canvas(
        &self,
        layout: &CanvasLayout,
        index: usize,
        device_width: u32,
    ) -> Arc<LineCanvasColumns> {
        let range = layout.canvas_samples(index, self.signal.samples.len());
        Arc::new(LineCanvasColumns::scan(
            &self.signal.samples[range],
            device_width,
        ))
    }

    fn build_frame(
        &self,
        layout: CanvasLayout,
        view: &LineView,
        mut columns_of: impl FnMut(usize, u32) -> Arc<LineCanvasColumns>,
    ) -> LineFrame {
        let pixel_ratio = layout.pixel_ratio;
        let device_height = js_round(view.height * pixel_ratio).max(0.0) as u32;
        let canvases = layout
            .canvases_between(view.scroll_left, view.client_width)
            .filter_map(|index| {
                let (offset, width) = layout.canvas_span(index);
                if width <= 0.0 {
                    return None;
                }
                let device_width = js_round(width * pixel_ratio).max(0.0) as u32;
                let left = js_round(offset);
                Some(LineFrameCanvas {
                    index,
                    left,
                    width,
                    device_width,
                    device_height,
                    viewport_x: js_round((left - view.scroll_left) * pixel_ratio),
                    x_scale: if device_width > 0 {
                        width * pixel_ratio / f64::from(device_width)
                    } else {
                        1.0
                    },
                    y_scale: if device_height > 0 {
                        view.height * pixel_ratio / f64::from(device_height)
                    } else {
                        1.0
                    },
                    columns: columns_of(index, device_width),
                })
            })
            .collect();
        LineFrame {
            layout,
            canvases,
            scroll_left: view.scroll_left,
            duration_seconds: self.signal.duration_seconds,
        }
    }
}

/// `Math.round`: halves round towards +∞.
pub fn js_round(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_round_rounds_halves_up() {
        assert_eq!(js_round(0.5), 1.0);
        assert_eq!(js_round(-0.5), 0.0);
        assert_eq!(js_round(2.4999), 2.0);
        assert_eq!(js_round(-2.5), -2.0);
    }

    #[test]
    fn column_scan_puts_each_sample_in_the_column_its_index_rounds_to() {
        // Four samples over ten device px: columns start at round(e * 2.5) = 0, 3, 5, 8.
        let columns = LineCanvasColumns::scan(&[0.1, -0.4, 0.2, 0.3], 10);
        assert_eq!(
            columns.columns,
            vec![
                LineColumn { x: 0, peak: 0.1 },
                LineColumn { x: 3, peak: 0.4 },
                LineColumn { x: 5, peak: 0.2 },
                LineColumn { x: 8, peak: 0.3 },
            ]
        );
        assert_eq!(columns.end_x, 10);

        // More samples than px: the loudest of each column wins.
        let columns = LineCanvasColumns::scan(&[0.1, -0.5, 0.2, 0.05, 0.3, -0.1], 2);
        assert_eq!(
            columns.columns,
            vec![
                LineColumn { x: 0, peak: 0.5 },
                LineColumn { x: 1, peak: 0.3 },
            ]
        );
    }

    #[test]
    fn amplitudes_round_and_keep_a_one_pixel_floor() {
        let canvas = LineFrameCanvas {
            index: 0,
            left: 0.0,
            width: 10.0,
            device_width: 10,
            device_height: 150,
            viewport_x: 0.0,
            x_scale: 1.0,
            y_scale: 1.0,
            columns: Arc::new(LineCanvasColumns::scan(&[], 10)),
        };
        assert_eq!(canvas.half(), 75.0);
        assert_eq!(canvas.amplitude(0.0, 20.0), 1.0);
        assert_eq!(canvas.amplitude(0.0003, 20.0), 1.0);
        assert_eq!(canvas.amplitude(0.001, 20.0), 2.0); // 1.5 rounds up
        assert_eq!(canvas.amplitude(0.5, 20.0), 750.0); // overshoot; the canvas clips it
    }

    #[test]
    fn canvases_are_one_viewport_wide_and_slice_their_samples() {
        let view = LineView {
            min_px_per_sec: 100.0,
            client_width: 300.0,
            height: 150.0,
            pixel_ratio: 1.25,
            scroll_left: 250.0,
        };
        let layout = CanvasLayout::new(10.005, &view).expect("layout");
        assert_eq!(layout.scroll_width, 1001.0);
        assert_eq!(layout.canvas_width, 300.0);
        assert_eq!(layout.canvas_count, 4);
        assert_eq!(layout.canvas_span(3), (900.0, 101.0));
        assert_eq!(layout.canvases_between(250.0, 300.0), 0..2);
        assert_eq!(layout.canvas_samples(1, 80_040), 23_988..47_976);
    }

    #[test]
    fn frame_places_visible_canvases_and_reuses_columns_while_panning() {
        let signal = DisplaySignal::from_peaks(vec![0.5; 8_000], 1.0).expect("signal");
        let waveform = LineWaveform::new(signal);
        let mut view = LineView {
            min_px_per_sec: 1_000.0,
            client_width: 400.0,
            height: 100.0,
            pixel_ratio: 2.0,
            scroll_left: 300.0,
        };
        let frame = waveform.frame(&view).expect("frame");
        assert_eq!(
            frame
                .canvases
                .iter()
                .map(|canvas| (canvas.index, canvas.viewport_x, canvas.device_width))
                .collect::<Vec<_>>(),
            vec![(0, -600.0, 800), (1, 200.0, 800)]
        );
        assert_eq!(frame.viewport_x_of(0.5), 400.0);

        view.scroll_left = 350.0;
        let panned = waveform.frame(&view).expect("frame");
        assert!(Arc::ptr_eq(
            &frame.canvases[1].columns,
            &panned.canvases[1].columns
        ));
    }

    #[test]
    fn display_signal_keeps_the_true_duration_and_8_khz_length() {
        let audio =
            DecodedAudio::from_f32_channels(24_000, vec![vec![0.25; 24_001], vec![-0.25; 24_001]])
                .expect("audio");
        let signal = DisplaySignal::from_decoded(&audio);
        assert_eq!(signal.duration_seconds(), 24_001.0 / 24_000.0);
        assert_eq!(signal.samples().len(), 8_001);
        // Opposite channels average to silence.
        assert!(signal.samples().iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn peaks_beyond_unity_are_scaled_like_wavesurfer_create_buffer() {
        let signal = DisplaySignal::from_peaks(vec![0.5, -2.0, 1.0], 1.0).expect("signal");
        assert_eq!(signal.samples(), &[0.25, -1.0, 0.5]);
    }
}
