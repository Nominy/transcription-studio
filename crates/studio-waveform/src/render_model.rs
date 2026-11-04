use rayon::prelude::*;
use std::sync::Arc;

use crate::{
    PeakCache, PeakExportError, ProgressGeometry, Region, RendererLayout, ScrollWindow,
    WaveViewport, WaveformSource,
};

const MAX_CACHED_RENDER_PEAKS: usize = 1_048_576;
const PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveformRenderOptions {
    pub channels: usize,
    pub peak_count: usize,
    pub precision: f64,
    pub cursor_width_px: f64,
    pub handle_width_px: f64,
}

impl Default for WaveformRenderOptions {
    fn default() -> Self {
        Self {
            channels: 1,
            peak_count: 256,
            precision: 10_000.0,
            cursor_width_px: 1.0,
            handle_width_px: 6.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformRenderModel {
    pub layout: RendererLayout,
    pub visible_start_seconds: f64,
    pub visible_end_seconds: f64,
    pub viewport_width_px: f64,
    pub cursor: ProgressGeometry,
    pub channels: Arc<[WaveformChannelRender]>,
    pub regions: Arc<[WaveformRegionRender]>,
    handle_width_px: f64,
}

impl WaveformRenderModel {
    pub fn from_source(
        source: &WaveformSource,
        viewport: WaveViewport,
        regions: &[Region],
        current_seconds: f64,
        options: WaveformRenderOptions,
    ) -> Result<Self, PeakExportError> {
        let scroll_window = viewport.scroll_window();
        let peaks = source.export_range_peaks(
            scroll_window.visible_start_seconds,
            scroll_window.visible_end_seconds,
            options.channels,
            options.peak_count,
            options.precision,
        )?;
        Self::from_render_parts(
            viewport.layout(),
            scroll_window,
            peaks,
            regions,
            current_seconds,
            options,
        )
    }

    pub fn from_peak_cache(
        peak_cache: &mut PeakCache,
        viewport: WaveViewport,
        regions: &[Region],
        current_seconds: f64,
        options: WaveformRenderOptions,
    ) -> Result<Self, PeakExportError> {
        let scroll_window = viewport.scroll_window();
        let resolution_length =
            cached_render_peak_resolution(viewport.layout(), options.peak_count);
        let peaks = peak_cache.visible_peaks_from_cached_resolution(
            scroll_window.visible_start_seconds,
            scroll_window.visible_end_seconds,
            options.channels,
            options.peak_count,
            options.precision,
            resolution_length,
        )?;
        Self::from_render_parts(
            viewport.layout(),
            scroll_window,
            peaks,
            regions,
            current_seconds,
            options,
        )
    }

    fn from_render_parts(
        layout: RendererLayout,
        scroll_window: ScrollWindow,
        peaks: Vec<Vec<f64>>,
        regions: &[Region],
        current_seconds: f64,
        options: WaveformRenderOptions,
    ) -> Result<Self, PeakExportError> {
        let visible_start_seconds = scroll_window.visible_start_seconds;
        let visible_end_seconds = scroll_window.visible_end_seconds;
        let visible_duration_seconds = (visible_end_seconds - visible_start_seconds).max(0.001);
        let viewport_width_px =
            (scroll_window.scroll_right_px - scroll_window.scroll_left_px).max(1.0);
        let cursor = ProgressGeometry::from_wavesurfer_rules(
            current_seconds - visible_start_seconds,
            visible_duration_seconds,
            options.cursor_width_px,
        );

        Ok(Self {
            layout,
            visible_start_seconds,
            visible_end_seconds,
            viewport_width_px,
            cursor,
            channels: peaks
                .into_iter()
                .enumerate()
                .map(|(channel_index, peaks)| {
                    WaveformChannelRender::from_window(
                        channel_index,
                        Arc::from(peaks),
                        visible_start_seconds,
                        visible_end_seconds,
                        visible_start_seconds,
                        visible_end_seconds,
                        options.peak_count,
                    )
                })
                .collect::<Vec<_>>()
                .into(),
            regions: regions
                .iter()
                .filter_map(|region| {
                    WaveformRegionRender::from_region(
                        region,
                        visible_start_seconds,
                        visible_end_seconds,
                    )
                })
                .collect::<Vec<_>>()
                .into(),
            handle_width_px: options.handle_width_px.max(0.0),
        })
    }

    pub fn hit_test(&self, x_px: f64, y_percent: f64) -> WaveformHit {
        self.hit_tester().hit_test(x_px, y_percent)
    }

    pub fn with_playhead_cursor(mut self, current_seconds: f64, cursor_width_px: f64) -> Self {
        let visible_duration_seconds =
            (self.visible_end_seconds - self.visible_start_seconds).max(0.001);
        self.cursor = ProgressGeometry::from_wavesurfer_rules(
            current_seconds - self.visible_start_seconds,
            visible_duration_seconds,
            cursor_width_px,
        );
        self
    }

    pub fn with_visible_window(
        mut self,
        visible_start_seconds: f64,
        visible_end_seconds: f64,
        regions: &[Region],
        current_seconds: f64,
        cursor_width_px: f64,
    ) -> Self {
        let visible_start_seconds = finite_non_negative(visible_start_seconds);
        let visible_end_seconds =
            finite_non_negative(visible_end_seconds).max(visible_start_seconds);
        let visible_duration_seconds = (visible_end_seconds - visible_start_seconds).max(0.001);
        self.visible_start_seconds = visible_start_seconds;
        self.visible_end_seconds = visible_end_seconds;
        self.cursor = ProgressGeometry::from_wavesurfer_rules(
            current_seconds - visible_start_seconds,
            visible_duration_seconds,
            cursor_width_px,
        );
        self.channels = self
            .channels
            .iter()
            .map(|channel| channel.with_visible_window(visible_start_seconds, visible_end_seconds))
            .collect::<Vec<_>>()
            .into();
        self.regions = regions
            .iter()
            .filter_map(|region| {
                WaveformRegionRender::from_region(
                    region,
                    visible_start_seconds,
                    visible_end_seconds,
                )
            })
            .collect::<Vec<_>>()
            .into();
        self
    }

    pub fn hit_tester(&self) -> WaveformHitTester {
        WaveformHitTester {
            visible_start_seconds: self.visible_start_seconds,
            visible_end_seconds: self.visible_end_seconds,
            viewport_width_px: self.viewport_width_px,
            regions: Arc::clone(&self.regions),
            handle_width_px: self.handle_width_px,
        }
    }
}

fn cached_render_peak_resolution(layout: RendererLayout, peak_count: usize) -> usize {
    let wrapper_width = if layout.wrapper_width_px.is_finite() {
        layout.wrapper_width_px.ceil().max(peak_count as f64)
    } else {
        peak_count as f64
    };
    (wrapper_width as usize).clamp(peak_count.max(1), MAX_CACHED_RENDER_PEAKS)
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformHitTester {
    visible_start_seconds: f64,
    visible_end_seconds: f64,
    viewport_width_px: f64,
    regions: Arc<[WaveformRegionRender]>,
    handle_width_px: f64,
}

impl WaveformHitTester {
    pub fn hit_test(&self, x_px: f64, y_percent: f64) -> WaveformHit {
        let x_px = finite_non_negative(x_px).clamp(0.0, self.viewport_width_px);
        let x_percent = if self.viewport_width_px > 0.0 {
            x_px / self.viewport_width_px * 100.0
        } else {
            0.0
        };
        let time_seconds = self.visible_start_seconds
            + x_px / self.viewport_width_px
                * (self.visible_end_seconds - self.visible_start_seconds);

        for region in self.regions.iter().rev() {
            if !region.contains_y(y_percent) || !region.contains_x(x_percent) {
                continue;
            }

            let start_x = region.left_percent / 100.0 * self.viewport_width_px;
            let end_x = (100.0 - region.right_percent) / 100.0 * self.viewport_width_px;
            if (x_px - start_x).abs() <= self.handle_width_px {
                return WaveformHit {
                    target: WaveformHitTarget::RegionStartHandle,
                    region_id: Some(region.id.clone()),
                    time_seconds,
                };
            }
            if (x_px - end_x).abs() <= self.handle_width_px {
                return WaveformHit {
                    target: WaveformHitTarget::RegionEndHandle,
                    region_id: Some(region.id.clone()),
                    time_seconds,
                };
            }
            return WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some(region.id.clone()),
                time_seconds,
            };
        }

        WaveformHit {
            target: WaveformHitTarget::Waveform,
            region_id: None,
            time_seconds,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformChannelRender {
    pub channel_index: usize,
    pub peaks: Arc<[f64]>,
    pub visible_peaks: Arc<[f32]>,
    pub peak_start_seconds: f64,
    pub peak_end_seconds: f64,
    pub visible_peak_count: usize,
}

impl WaveformChannelRender {
    fn from_window(
        channel_index: usize,
        peaks: Arc<[f64]>,
        peak_start_seconds: f64,
        peak_end_seconds: f64,
        visible_start_seconds: f64,
        visible_end_seconds: f64,
        visible_peak_count: usize,
    ) -> Self {
        let visible_peaks = sample_visible_peaks(
            &peaks,
            peak_start_seconds,
            peak_end_seconds,
            visible_start_seconds,
            visible_end_seconds,
            visible_peak_count,
        );

        Self {
            channel_index,
            peaks,
            visible_peaks,
            peak_start_seconds,
            peak_end_seconds,
            visible_peak_count,
        }
    }

    fn with_visible_window(&self, visible_start_seconds: f64, visible_end_seconds: f64) -> Self {
        let visible_peaks = sample_visible_peaks(
            &self.peaks,
            self.peak_start_seconds,
            self.peak_end_seconds,
            visible_start_seconds,
            visible_end_seconds,
            self.visible_peak_count,
        );

        Self {
            channel_index: self.channel_index,
            peaks: Arc::clone(&self.peaks),
            visible_peaks,
            peak_start_seconds: self.peak_start_seconds,
            peak_end_seconds: self.peak_end_seconds,
            visible_peak_count: self.visible_peak_count,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformRegionRender {
    pub id: String,
    pub left_percent: f64,
    pub right_percent: f64,
    pub top_percent: f64,
    pub height_percent: f64,
    pub is_marker: bool,
}

impl WaveformRegionRender {
    fn from_region(
        region: &Region,
        visible_start_seconds: f64,
        visible_end_seconds: f64,
    ) -> Option<Self> {
        if region.end_seconds < visible_start_seconds || region.start_seconds > visible_end_seconds
        {
            return None;
        }

        let visible_duration = (visible_end_seconds - visible_start_seconds).max(0.001);
        let clipped_start = region.start_seconds.max(visible_start_seconds);
        let clipped_end = region.end_seconds.min(visible_end_seconds);
        let channel_geometry = region.geometry();

        Some(Self {
            id: region.id.clone(),
            left_percent: (clipped_start - visible_start_seconds) / visible_duration * 100.0,
            right_percent: (visible_end_seconds - clipped_end) / visible_duration * 100.0,
            top_percent: channel_geometry.top_percent,
            height_percent: channel_geometry.height_percent,
            is_marker: region.start_seconds == region.end_seconds,
        })
    }

    fn contains_x(&self, x_percent: f64) -> bool {
        x_percent >= self.left_percent && x_percent <= 100.0 - self.right_percent
    }

    fn contains_y(&self, y_percent: f64) -> bool {
        y_percent >= self.top_percent && y_percent <= self.top_percent + self.height_percent
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformHit {
    pub target: WaveformHitTarget,
    pub region_id: Option<String>,
    pub time_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveformHitTarget {
    Waveform,
    RegionBody,
    RegionStartHandle,
    RegionEndHandle,
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn sample_visible_peaks(
    peaks: &[f64],
    peak_start_seconds: f64,
    peak_end_seconds: f64,
    visible_start_seconds: f64,
    visible_end_seconds: f64,
    sample_count: usize,
) -> Arc<[f32]> {
    let sample_count = sample_count.max(1);
    let visible_duration = (visible_end_seconds - visible_start_seconds).max(f64::EPSILON);
    let sample_peak = |index: usize| {
        sample_visible_peak(
            peaks,
            peak_start_seconds,
            peak_end_seconds,
            visible_start_seconds,
            visible_duration,
            sample_count,
            index,
        )
    };
    let visible_peaks = if sample_count >= PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN {
        (0..sample_count)
            .into_par_iter()
            .map(sample_peak)
            .collect::<Vec<_>>()
    } else {
        (0..sample_count).map(sample_peak).collect::<Vec<_>>()
    };
    Arc::from(visible_peaks)
}

fn sample_visible_peak(
    peaks: &[f64],
    peak_start_seconds: f64,
    peak_end_seconds: f64,
    visible_start_seconds: f64,
    visible_duration: f64,
    sample_count: usize,
    index: usize,
) -> f32 {
    let start_ratio = index as f64 / sample_count as f64;
    let end_ratio = (index + 1) as f64 / sample_count as f64;
    let sample_start = visible_start_seconds + visible_duration * start_ratio;
    let sample_end = visible_start_seconds + visible_duration * end_ratio.min(1.0);
    peak_window_abs_max(
        peaks,
        peak_start_seconds,
        peak_end_seconds,
        sample_start,
        sample_end,
    )
}

fn peak_window_abs_max(
    peaks: &[f64],
    peak_start_seconds: f64,
    peak_end_seconds: f64,
    sample_start_seconds: f64,
    sample_end_seconds: f64,
) -> f32 {
    if peaks.is_empty() || peak_start_seconds >= peak_end_seconds {
        return 0.0;
    }

    let peak_duration = peak_end_seconds - peak_start_seconds;
    let start_ratio = ((sample_start_seconds - peak_start_seconds) / peak_duration).clamp(0.0, 1.0);
    let end_ratio = ((sample_end_seconds - peak_start_seconds) / peak_duration).clamp(0.0, 1.0);
    let start_index = (start_ratio * peaks.len() as f64).floor() as usize;
    let mut end_index = (end_ratio * peaks.len() as f64).ceil() as usize;
    let start_index = start_index.min(peaks.len());
    end_index = end_index.min(peaks.len());
    if end_index <= start_index && start_index < peaks.len() {
        end_index = start_index + 1;
    }

    peaks[start_index..end_index]
        .iter()
        .map(|peak| peak.abs().clamp(0.0, 1.0) as f32)
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{RegionParams, RegionsPlugin};

    #[test]
    fn hit_tester_reuses_render_model_regions_without_cloning_geometry() {
        let source = WaveformSource::from_precomputed_peaks(vec![vec![0.0; 8]], 4.0).unwrap();
        let viewport = WaveViewport::new(4.0, 400.0, 0.0, true, 1.0);
        let mut regions = RegionsPlugin::new(4.0, 1).unwrap();
        regions
            .add_region(RegionParams::new("annotation-1", 1.0, Some(3.0)))
            .unwrap();
        let model = WaveformRenderModel::from_source(
            &source,
            viewport,
            regions.get_regions(),
            0.0,
            WaveformRenderOptions::default(),
        )
        .unwrap();

        let hit_tester = model.hit_tester();

        assert!(Arc::ptr_eq(&model.regions, &hit_tester.regions));
        assert_eq!(
            hit_tester.hit_test(200.0, 25.0).target,
            WaveformHitTarget::RegionBody
        );
    }

    #[test]
    fn visible_window_retargets_presampled_channel_peaks_without_replacing_full_peak_storage() {
        let source =
            WaveformSource::from_precomputed_peaks(vec![vec![0.0, 0.25, 0.5, 0.75, 1.0]], 5.0)
                .unwrap();
        let mut peak_cache = PeakCache::new(source);
        let mut viewport = WaveViewport::new(5.0, 100.0, 20.0, false, 1.0);
        viewport.set_scroll_time(0.0);
        let model = WaveformRenderModel::from_peak_cache(
            &mut peak_cache,
            viewport,
            &[],
            0.0,
            WaveformRenderOptions {
                channels: 1,
                peak_count: 4,
                precision: 10_000.0,
                ..WaveformRenderOptions::default()
            },
        )
        .unwrap();
        let first_channel = model.channels.first().unwrap();
        let first_full_peaks = Arc::clone(&first_channel.peaks);
        let first_visible_peaks = Arc::clone(&first_channel.visible_peaks);

        let retargeted = model.with_visible_window(1.0, 4.0, &[], 2.0, 1.0);
        let retargeted_channel = retargeted.channels.first().unwrap();

        assert!(Arc::ptr_eq(&first_full_peaks, &retargeted_channel.peaks));
        assert!(!Arc::ptr_eq(
            &first_visible_peaks,
            &retargeted_channel.visible_peaks
        ));
        assert_eq!(retargeted_channel.visible_peak_count, 4);
        assert_eq!(retargeted_channel.visible_peaks.len(), 4);
    }

    #[test]
    fn visible_peak_sampling_uses_parallel_iterator_for_large_windows() {
        let source = include_str!("render_model.rs");
        let imports_and_constants = source
            .split("#[derive(Clone, Copy, Debug, PartialEq)]")
            .next()
            .expect("render model options should follow imports and constants");
        let sampling_helper = source
            .split("fn sample_visible_peaks(")
            .nth(1)
            .expect("peak sampling helper should exist")
            .split("\nfn sample_visible_peak(")
            .next()
            .expect("single-peak sampler should follow peak sampling helper");

        assert!(imports_and_constants.contains("use rayon::prelude::*;"));
        assert!(imports_and_constants.contains("const PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN: usize"));
        assert!(sampling_helper.contains("sample_count >= PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN"));
        assert!(
            sampling_helper.contains("into_par_iter()"),
            "large visible windows reach PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN and must fan bucket sampling across CPU cores"
        );
    }
}
