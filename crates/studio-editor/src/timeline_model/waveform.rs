use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use crate::timeline_model::{LaneColor, TimeRange};
use exaudio_extension::document::DocumentEnvironment;
use studio_core::Segment;
use studio_waveform::{
    DISPLAY_SAMPLE_RATE, DecodedAudio, DisplaySignal, LineWaveform, PeakCache, WaveViewport,
    WaveformRenderModel, WaveformRenderOptions, WaveformSource,
};

const WAVEFORM_PEAK_COUNT: usize = 256;
const OVERVIEW_RENDER_PEAK_COUNT: usize = 1_024;
const OVERVIEW_RENDER_WIDTH_PX: f64 = 1_000.0;
// Segment-derived caches retain only the current variant for each stable segment ID.
// These caps also bound retention when segments are deleted or IDs churn over a long session.
const SEGMENT_PEAK_CACHE_CAPACITY: usize = 512;
const WAVEFORM_BLOCK_STATIC_CACHE_CAPACITY: usize = 512;

/// A lane's audio as the editor draws it. The timeline paints its display signal (see
/// [`DisplaySignal`]); the overview and segment peaks read the same signal, so only
/// analysis (silence, drafting) needs the full-rate PCM, kept unless built `display_only`.
#[derive(Debug)]
pub struct LaneWaveform {
    audio: Option<Arc<DecodedAudio>>,
    line: Arc<LineWaveform>,
    source: WaveformSource,
    peak_cache: RefCell<PeakCache>,
    render_cache: RefCell<LaneWaveformRenderCache>,
    segment_peak_cache: RefCell<SegmentPeakCache>,
    block_static_cache: RefCell<WaveformBlockStaticCache>,
}

impl Clone for LaneWaveform {
    fn clone(&self) -> Self {
        Self::with_parts(
            self.audio.clone(),
            Arc::clone(&self.line),
            self.source.clone(),
        )
    }
}

impl PartialEq for LaneWaveform {
    fn eq(&self, other: &Self) -> bool {
        self.audio == other.audio && self.line.signal() == other.line.signal()
    }
}

impl LaneWaveform {
    /// Decoded lane audio; builds the display signal (O(frames): call off the UI thread).
    pub fn from_decoded(decoded: DecodedAudio) -> Self {
        let signal = DisplaySignal::from_decoded(&decoded);
        Self::from_signal(signal, Some(Arc::new(decoded)))
    }

    /// Like [`Self::from_decoded`] without keeping the PCM (`decoded_audio()` is `None`).
    pub fn display_only(decoded: &DecodedAudio) -> Self {
        Self::from_signal(DisplaySignal::from_decoded(decoded), None)
    }

    /// Peaks drawn as they are, like wavesurfer's `load(url, [peaks], duration)` (first
    /// channel).
    pub fn from_precomputed_peaks(
        peaks: Vec<Vec<f64>>,
        duration_seconds: f64,
    ) -> Result<Self, String> {
        let first = peaks
            .first()
            .map(|channel| channel.iter().map(|peak| *peak as f32).collect())
            .unwrap_or_default();
        let signal = DisplaySignal::from_peaks(first, duration_seconds)?;
        let source = WaveformSource::from_precomputed_peaks(peaks, duration_seconds)?;
        Ok(Self::with_parts(
            None,
            Arc::new(LineWaveform::new(signal)),
            source,
        ))
    }

    fn from_signal(signal: DisplaySignal, audio: Option<Arc<DecodedAudio>>) -> Self {
        // A decoded lane has at least one frame, so its display signal has a sample.
        let display_audio =
            DecodedAudio::from_shared_channels(DISPLAY_SAMPLE_RATE, vec![signal.samples_shared()])
                .expect("display signal of decoded audio is never empty");
        Self::with_parts(
            audio,
            Arc::new(LineWaveform::new(signal)),
            WaveformSource::from_decoded(display_audio),
        )
    }

    fn with_parts(
        audio: Option<Arc<DecodedAudio>>,
        line: Arc<LineWaveform>,
        source: WaveformSource,
    ) -> Self {
        Self {
            audio,
            line,
            peak_cache: RefCell::new(PeakCache::new(source.clone())),
            source,
            render_cache: RefCell::new(LaneWaveformRenderCache::default()),
            segment_peak_cache: RefCell::new(SegmentPeakCache::default()),
            block_static_cache: RefCell::new(WaveformBlockStaticCache::default()),
        }
    }

    /// Full-rate PCM for analysis; `None` for display-only and peak-backed waveforms.
    pub fn decoded_audio(&self) -> Option<&DecodedAudio> {
        self.audio.as_deref()
    }

    pub fn decoded_audio_shared(&self) -> Option<Arc<DecodedAudio>> {
        self.audio.clone()
    }

    /// The timeline's waveform (shared; its canvas cache survives clones).
    pub fn line(&self) -> Arc<LineWaveform> {
        Arc::clone(&self.line)
    }

    pub fn duration_seconds(&self) -> f64 {
        self.line.duration_seconds()
    }

    /// Bytes of sample data this waveform keeps alive: the display signal plus any PCM.
    pub fn retained_bytes(&self) -> usize {
        let pcm = self.audio.as_ref().map_or(0, |audio| {
            audio.frame_count() * audio.number_of_channels() * std::mem::size_of::<f32>()
        });
        self.line.signal().retained_bytes() + pcm
    }

    pub fn peaks_for_segment(&self, segment: &Segment, count: usize) -> Option<Arc<[f32]>> {
        self.segment_peak_cache.borrow_mut().peaks_for_segment(
            &self.source,
            segment,
            count,
            10_000.0,
        )
    }

    pub(super) fn block_for_segment_with_static_source(
        &self,
        segment: &Segment,
        static_source_segment: &Segment,
        viewport: TimeRange,
        is_transcription: bool,
    ) -> WaveformBlock {
        let static_block = {
            let mut block_cache = self.block_static_cache.borrow_mut();
            let mut peak_cache = self.segment_peak_cache.borrow_mut();
            block_cache.static_block_for_segment(
                &self.source,
                &mut peak_cache,
                static_source_segment,
            )
        };
        WaveformBlock::from_static_block(segment, viewport, static_block, is_transcription)
    }

    pub fn prewarm_blocks_for_segments<'a>(
        &self,
        segments: impl IntoIterator<Item = &'a Segment>,
    ) -> usize {
        let mut block_cache = self.block_static_cache.borrow_mut();
        let mut peak_cache = self.segment_peak_cache.borrow_mut();
        segments
            .into_iter()
            .filter(|segment| {
                block_cache.prewarm_static_block_for_segment(&self.source, &mut peak_cache, segment)
            })
            .count()
    }

    pub fn blocks_for_segments<'a>(
        &self,
        segments: impl IntoIterator<Item = &'a Segment>,
        viewport: TimeRange,
        environment: &DocumentEnvironment,
    ) -> Arc<[WaveformBlock]> {
        segments
            .into_iter()
            .map(|segment| {
                self.block_for_segment_with_static_source(
                    segment,
                    segment,
                    viewport,
                    environment.is_text_segment(segment),
                )
            })
            .collect::<Vec<_>>()
            .into()
    }

    pub fn overview_render_model(&self, playhead_seconds: f64) -> Option<WaveformRenderModel> {
        if let Some(model) = self.render_cache.borrow_mut().overview_model() {
            return Some(model.with_playhead_cursor(playhead_seconds, 2.0));
        }

        let viewport = WaveViewport::new(
            self.source.duration(),
            OVERVIEW_RENDER_WIDTH_PX,
            0.0,
            true,
            1.0,
        );

        let model = {
            let mut peak_cache = self.peak_cache.borrow_mut();
            WaveformRenderModel::from_peak_cache(
                &mut peak_cache,
                viewport,
                &[],
                playhead_seconds,
                WaveformRenderOptions {
                    channels: 1,
                    peak_count: OVERVIEW_RENDER_PEAK_COUNT,
                    precision: 10_000.0,
                    cursor_width_px: 2.0,
                    ..WaveformRenderOptions::default()
                },
            )
            .ok()?
        };
        self.render_cache
            .borrow_mut()
            .store_overview_model(model.clone());
        Some(model)
    }

    pub fn render_cache_stats(&self) -> WaveformRenderCacheStats {
        self.render_cache.borrow().stats
    }

    pub fn segment_peak_cache_stats(&self) -> WaveformRenderCacheStats {
        self.segment_peak_cache.borrow().stats
    }

    pub fn cached_segment_peak_count(&self) -> usize {
        self.segment_peak_cache.borrow().entries.len()
    }

    pub fn cached_static_block_count(&self) -> usize {
        self.block_static_cache.borrow().entries.len()
    }

    pub fn cached_peak_resolution_count(&self) -> usize {
        self.peak_cache.borrow().cached_resolution_count()
    }
}

#[derive(Clone, Debug, Default)]
struct SegmentPeakCache {
    entries: BTreeMap<SegmentPeakCacheKey, Arc<[f32]>>,
    insertion_order: VecDeque<String>,
    stats: WaveformRenderCacheStats,
}

impl SegmentPeakCache {
    fn peaks_for_segment(
        &mut self,
        source: &WaveformSource,
        segment: &Segment,
        count: usize,
        precision: f64,
    ) -> Option<Arc<[f32]>> {
        let key = SegmentPeakCacheKey::new(segment, count, precision)?;
        if let Some(peaks) = self.entries.get(&key) {
            self.stats.hits += 1;
            return Some(Arc::clone(peaks));
        }

        self.stats.misses += 1;
        let peaks = source
            .export_range_peaks(
                segment.start_seconds,
                segment.end_seconds,
                1,
                count,
                precision,
            )
            .ok()
            .and_then(|channels| channels.into_iter().next())?
            .into_iter()
            .map(|peak| peak.abs().clamp(0.0, 1.0) as f32)
            .collect::<Vec<_>>();
        let peaks = Arc::<[f32]>::from(peaks);
        self.store(key, Arc::clone(&peaks));
        Some(peaks)
    }

    fn store(&mut self, key: SegmentPeakCacheKey, peaks: Arc<[f32]>) {
        let segment_id = key.segment_id.clone();
        self.entries
            .retain(|cached_key, _| cached_key.segment_id != segment_id);
        self.insertion_order
            .retain(|cached_segment_id| cached_segment_id != &segment_id);
        self.entries.insert(key, peaks);
        self.insertion_order.push_back(segment_id);

        while self.entries.len() > SEGMENT_PEAK_CACHE_CAPACITY {
            let Some(evicted_segment_id) = self.insertion_order.pop_front() else {
                break;
            };
            self.entries
                .retain(|cached_key, _| cached_key.segment_id != evicted_segment_id);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SegmentPeakCacheKey {
    segment_id: String,
    start_seconds_bits: u64,
    end_seconds_bits: u64,
    count: usize,
    precision_bits: u64,
}

impl SegmentPeakCacheKey {
    fn new(segment: &Segment, count: usize, precision: f64) -> Option<Self> {
        if count == 0
            || !precision.is_finite()
            || precision <= 0.0
            || !segment.start_seconds.is_finite()
            || !segment.end_seconds.is_finite()
            || segment.start_seconds >= segment.end_seconds
        {
            return None;
        }

        Some(Self {
            segment_id: segment.id.clone(),
            start_seconds_bits: segment.start_seconds.to_bits(),
            end_seconds_bits: segment.end_seconds.to_bits(),
            count,
            precision_bits: precision.to_bits(),
        })
    }
}

#[derive(Clone, Debug, Default)]
struct WaveformBlockStaticCache {
    entries: BTreeMap<WaveformBlockStaticCacheKey, WaveformBlockStatic>,
    insertion_order: VecDeque<String>,
}

impl WaveformBlockStaticCache {
    fn prewarm_static_block_for_segment(
        &mut self,
        source: &WaveformSource,
        segment_peak_cache: &mut SegmentPeakCache,
        segment: &Segment,
    ) -> bool {
        let key = WaveformBlockStaticCacheKey::from_segment(segment);
        if self.entries.contains_key(&key) {
            return false;
        }

        let block =
            WaveformBlockStatic::from_segment_with_peak_cache(source, segment_peak_cache, segment);
        self.store(key, block);
        true
    }

    fn static_block_for_segment(
        &mut self,
        source: &WaveformSource,
        segment_peak_cache: &mut SegmentPeakCache,
        segment: &Segment,
    ) -> WaveformBlockStatic {
        let key = WaveformBlockStaticCacheKey::from_segment(segment);
        if let Some(block) = self.entries.get(&key) {
            return block.clone();
        }

        let _ = self.prewarm_static_block_for_segment(source, segment_peak_cache, segment);
        self.entries
            .get(&key)
            .expect("prewarmed static waveform block should be stored")
            .clone()
    }

    fn store(&mut self, key: WaveformBlockStaticCacheKey, block: WaveformBlockStatic) {
        let segment_id = key.segment_id.clone();
        self.entries
            .retain(|cached_key, _| cached_key.segment_id != segment_id);
        self.insertion_order
            .retain(|cached_segment_id| cached_segment_id != &segment_id);
        self.entries.insert(key, block);
        self.insertion_order.push_back(segment_id);

        while self.entries.len() > WAVEFORM_BLOCK_STATIC_CACHE_CAPACITY {
            let Some(evicted_segment_id) = self.insertion_order.pop_front() else {
                break;
            };
            self.entries
                .retain(|cached_key, _| cached_key.segment_id != evicted_segment_id);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct WaveformBlockStaticCacheKey {
    segment_id: String,
    start_seconds_bits: u64,
    end_seconds_bits: u64,
    text: String,
}

impl WaveformBlockStaticCacheKey {
    fn from_segment(segment: &Segment) -> Self {
        Self {
            segment_id: segment.id.clone(),
            start_seconds_bits: segment.start_seconds.to_bits(),
            end_seconds_bits: segment.end_seconds.to_bits(),
            text: segment.text.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct WaveformBlockStatic {
    segment_id: String,
    tooltip: String,
    peaks: Arc<[f32]>,
    peak_source: WaveformPeakSource,
}

impl WaveformBlockStatic {
    fn from_segment_with_peak_cache(
        source: &WaveformSource,
        segment_peak_cache: &mut SegmentPeakCache,
        segment: &Segment,
    ) -> Self {
        let (peaks, peak_source) = segment_peak_cache
            .peaks_for_segment(source, segment, WAVEFORM_PEAK_COUNT, 10_000.0)
            .map(|peaks| (peaks, WaveformPeakSource::Audio))
            .unwrap_or_else(|| {
                (
                    Arc::<[f32]>::from(waveform_peaks(&segment.id, WAVEFORM_PEAK_COUNT)),
                    WaveformPeakSource::Synthetic,
                )
            });

        Self {
            segment_id: segment.id.clone(),
            tooltip: segment.text.clone(),
            peaks,
            peak_source,
        }
    }

    fn synthetic(segment: &Segment) -> Self {
        Self {
            segment_id: segment.id.clone(),
            tooltip: segment.text.clone(),
            peaks: Arc::<[f32]>::from(waveform_peaks(&segment.id, WAVEFORM_PEAK_COUNT)),
            peak_source: WaveformPeakSource::Synthetic,
        }
    }
}

/// The overview's render model (one per lane; the playhead cursor is reapplied per frame).
#[derive(Clone, Debug, Default)]
struct LaneWaveformRenderCache {
    overview: Option<WaveformRenderModel>,
    stats: WaveformRenderCacheStats,
}

impl LaneWaveformRenderCache {
    fn overview_model(&mut self) -> Option<WaveformRenderModel> {
        let model = self.overview.clone();
        if model.is_some() {
            self.stats.hits += 1;
        } else {
            self.stats.misses += 1;
        }
        model
    }

    fn store_overview_model(&mut self, model: WaveformRenderModel) {
        self.overview = Some(model);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaveformRenderCacheStats {
    pub hits: usize,
    pub misses: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverviewWaveformView {
    pub waveform_model: Option<WaveformRenderModel>,
    pub lane_waveform_models: Arc<[OverviewLaneWaveformView]>,
    pub blocks: Arc<[OverviewWaveformBlock]>,
    pub block_lane_rows: usize,
    pub peak_source: WaveformPeakSource,
    pub viewport_left_percent: f32,
    pub viewport_width_percent: f32,
    pub playhead_percent: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverviewLaneWaveformView {
    pub lane_id: String,
    pub color: LaneColor,
    pub waveform_model: WaveformRenderModel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverviewWaveformBlock {
    pub color: LaneColor,
    pub lane_row: usize,
    pub left_percent: f32,
    pub width_percent: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveformPeakSource {
    Audio,
    Synthetic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformBlock {
    pub segment_id: String,
    pub tooltip: String,
    pub is_transcription: bool,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub left_percent: f32,
    pub width_percent: f32,
    pub peaks: Arc<[f32]>,
    pub peak_source: WaveformPeakSource,
}

impl WaveformBlock {
    pub(super) fn for_segment_with_static_source(
        segment: &Segment,
        static_source_segment: &Segment,
        viewport: TimeRange,
        lane_waveform: Option<&LaneWaveform>,
        is_transcription: bool,
    ) -> Self {
        if let Some(lane_waveform) = lane_waveform {
            return lane_waveform.block_for_segment_with_static_source(
                segment,
                static_source_segment,
                viewport,
                is_transcription,
            );
        }

        Self::from_static_block(
            segment,
            viewport,
            WaveformBlockStatic::synthetic(static_source_segment),
            is_transcription,
        )
    }

    fn from_static_block(
        segment: &Segment,
        viewport: TimeRange,
        static_block: WaveformBlockStatic,
        is_transcription: bool,
    ) -> Self {
        let (left_percent, width_percent) = waveform_block_geometry_percentages(segment, viewport);

        Self {
            segment_id: static_block.segment_id,
            tooltip: static_block.tooltip,
            is_transcription,
            start_seconds: segment.start_seconds.min(segment.end_seconds),
            end_seconds: segment.start_seconds.max(segment.end_seconds),
            left_percent,
            width_percent,
            peaks: static_block.peaks,
            peak_source: static_block.peak_source,
        }
    }
}

fn waveform_block_geometry_percentages(segment: &Segment, viewport: TimeRange) -> (f32, f32) {
    let total_duration_seconds = viewport.duration_seconds();
    if !total_duration_seconds.is_finite() || total_duration_seconds <= 0.0 {
        return (0.0, 0.5);
    }

    let start_seconds = clamp_waveform_block_position(
        segment.start_seconds - viewport.start_seconds,
        total_duration_seconds,
    );
    let end_seconds = clamp_waveform_block_position(
        segment.end_seconds - viewport.start_seconds,
        total_duration_seconds,
    );
    let (start_seconds, end_seconds) = if start_seconds <= end_seconds {
        (start_seconds, end_seconds)
    } else {
        (end_seconds, start_seconds)
    };

    let left_percent = (start_seconds / total_duration_seconds * 100.0) as f32;
    let right_percent =
        ((total_duration_seconds - end_seconds) / total_duration_seconds * 100.0) as f32;
    let width_percent = (100.0 - left_percent - right_percent).max(0.0);

    (
        left_percent.clamp(0.0, 100.0),
        width_percent.clamp(0.5, 100.0),
    )
}

fn clamp_waveform_block_position(position: f64, total_duration_seconds: f64) -> f64 {
    if position.is_finite() {
        position.clamp(0.0, total_duration_seconds)
    } else {
        0.0
    }
}

fn waveform_peaks(seed: &str, count: usize) -> Vec<f32> {
    let mut hash = 0u32;
    for byte in seed.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }

    (0..count)
        .map(|index| {
            let value = hash
                .wrapping_add((index as u32).wrapping_mul(17))
                .wrapping_mul(1_103_515_245)
                .wrapping_add(12_345);
            0.18 + ((value % 100) as f32 / 100.0) * 0.78
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use studio_core::Segment;
    use studio_waveform::DecodedAudio;

    use super::{
        LaneWaveform, SEGMENT_PEAK_CACHE_CAPACITY, SegmentPeakCache, SegmentPeakCacheKey,
        TimeRange, WAVEFORM_BLOCK_STATIC_CACHE_CAPACITY, WaveformBlock, WaveformBlockStatic,
        WaveformBlockStaticCache, WaveformBlockStaticCacheKey, WaveformPeakSource,
    };

    #[test]
    fn waveform_block_carries_full_tooltip_text() {
        let segment = Segment {
            id: "annotation-1".to_string(),
            source_annotation_id: None,
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-1".to_string(),
            start_seconds: 1.0,
            end_seconds: 5.0,
            text: "This is a long transcription region that needs a compact waveform label."
                .to_string(),
            metadata: Default::default(),
        };

        let block = WaveformBlock::for_segment_with_static_source(
            &segment,
            &segment,
            TimeRange {
                start_seconds: 0.0,
                end_seconds: 10.0,
            },
            None,
            true,
        );

        assert_eq!(
            block.tooltip,
            "This is a long transcription region that needs a compact waveform label."
        );
    }

    #[test]
    fn waveform_block_geometry_matches_region_clamping_without_visible_change() {
        let cases = [
            (-1.0, 4.0, 0.0, 10.0),
            (1.0, 5.0, 0.0, 10.0),
            (7.0, 12.0, 0.0, 10.0),
            (12.0, 7.0, 0.0, 10.0),
        ];

        for (start_seconds, end_seconds, viewport_start, viewport_end) in cases {
            let segment = Segment {
                id: format!("annotation-{start_seconds}-{end_seconds}"),
                source_annotation_id: None,
                speaker_id: "speaker-1".to_string(),
                lane_id: "track-1".to_string(),
                start_seconds,
                end_seconds,
                text: "Geometry parity".to_string(),
                metadata: Default::default(),
            };
            let viewport = TimeRange {
                start_seconds: viewport_start,
                end_seconds: viewport_end,
            };

            let block = WaveformBlock::for_segment_with_static_source(
                &segment, &segment, viewport, None, true,
            );
            let region = studio_waveform::Region::new(
                &segment.id,
                segment.start_seconds - viewport.start_seconds,
                segment.end_seconds - viewport.start_seconds,
                viewport.duration_seconds(),
            )
            .expect("viewport duration should be valid");
            let geometry = region.geometry();
            let expected_left = (geometry.left_percent as f32).clamp(0.0, 100.0);
            let expected_width =
                (100.0 - geometry.left_percent as f32 - geometry.right_percent as f32)
                    .max(0.0)
                    .clamp(0.5, 100.0);

            assert_eq!(block.left_percent, expected_left);
            assert_eq!(block.width_percent, expected_width);
        }
    }

    #[test]
    fn segment_derived_caches_replace_variants_for_the_same_stable_id() {
        let waveform = LaneWaveform::from_precomputed_peaks(
            vec![vec![0.0, 0.25, 0.5, 0.75, 1.0, 0.5, 0.25, 0.0]],
            8.0,
        )
        .expect("valid peak source");
        let mut segment = Segment {
            id: "stable-segment".to_string(),
            source_annotation_id: None,
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-1".to_string(),
            start_seconds: 1.0,
            end_seconds: 4.0,
            text: "First text".to_string(),
            metadata: Default::default(),
        };
        let viewport = TimeRange {
            start_seconds: 0.0,
            end_seconds: 8.0,
        };

        for edit in 0..32 {
            segment.start_seconds = 1.0 + f64::from(edit) * 0.01;
            segment.end_seconds = 4.0 + f64::from(edit) * 0.01;
            segment.text = format!("Current text {edit}");

            let peaks = waveform
                .segment_peak_cache
                .borrow_mut()
                .peaks_for_segment(
                    &waveform.source,
                    &segment,
                    32 + edit as usize,
                    10_000.0 + f64::from(edit),
                )
                .expect("edited segment should produce peaks");
            assert_eq!(peaks.len(), 32 + edit as usize);

            let block =
                waveform.block_for_segment_with_static_source(&segment, &segment, viewport, true);
            assert_eq!(block.tooltip, segment.text);
            assert_eq!(block.start_seconds, segment.start_seconds);
            assert_eq!(block.end_seconds, segment.end_seconds);
        }

        let peak_cache = waveform.segment_peak_cache.borrow();
        assert_eq!(peak_cache.entries.len(), 1);
        assert_eq!(
            peak_cache
                .entries
                .keys()
                .next()
                .expect("current peak variant should remain")
                .segment_id,
            segment.id
        );
        let block_cache = waveform.block_static_cache.borrow();
        assert_eq!(block_cache.entries.len(), 1);
        let current_block = block_cache
            .entries
            .values()
            .next()
            .expect("current block variant should remain");
        assert_eq!(current_block.tooltip, segment.text);
    }

    #[test]
    fn segment_derived_cache_caps_evict_oldest_ids_and_keep_newest() {
        let mut peak_cache = SegmentPeakCache::default();
        let mut block_cache = WaveformBlockStaticCache::default();
        let total = SEGMENT_PEAK_CACHE_CAPACITY.max(WAVEFORM_BLOCK_STATIC_CACHE_CAPACITY) + 1;

        for index in 0..total {
            let segment_id = format!("segment-{index}");
            peak_cache.store(
                SegmentPeakCacheKey {
                    segment_id: segment_id.clone(),
                    start_seconds_bits: 0.0_f64.to_bits(),
                    end_seconds_bits: 1.0_f64.to_bits(),
                    count: 1,
                    precision_bits: 10_000.0_f64.to_bits(),
                },
                Arc::from([index as f32]),
            );
            block_cache.store(
                WaveformBlockStaticCacheKey {
                    segment_id: segment_id.clone(),
                    start_seconds_bits: 0.0_f64.to_bits(),
                    end_seconds_bits: 1.0_f64.to_bits(),
                    text: segment_id.clone(),
                },
                WaveformBlockStatic {
                    segment_id: segment_id.clone(),
                    tooltip: segment_id,
                    peaks: Arc::from([index as f32]),
                    peak_source: WaveformPeakSource::Audio,
                },
            );
        }

        assert_eq!(peak_cache.entries.len(), SEGMENT_PEAK_CACHE_CAPACITY);
        assert_eq!(
            block_cache.entries.len(),
            WAVEFORM_BLOCK_STATIC_CACHE_CAPACITY
        );
        assert!(
            !peak_cache
                .entries
                .keys()
                .any(|key| key.segment_id == "segment-0")
        );
        assert!(
            !block_cache
                .entries
                .keys()
                .any(|key| key.segment_id == "segment-0")
        );
        let newest = format!("segment-{}", total - 1);
        assert!(
            peak_cache
                .entries
                .keys()
                .any(|key| key.segment_id == newest)
        );
        assert!(
            block_cache
                .entries
                .keys()
                .any(|key| key.segment_id == newest)
        );
    }

    #[test]
    fn display_only_waveforms_drop_the_pcm_but_draw_the_same_signal() {
        let audio =
            DecodedAudio::from_f32_channels(16_000, vec![vec![0.25; 16_000]]).expect("valid audio");
        let full = LaneWaveform::from_decoded(audio.clone());
        let display = LaneWaveform::display_only(&audio);

        assert!(full.decoded_audio().is_some());
        assert!(display.decoded_audio().is_none());
        assert_eq!(full.line().signal(), display.line().signal());
        assert_eq!(full.duration_seconds(), 1.0);
        assert_eq!(display.duration_seconds(), 1.0);
        // 8 kHz display signal, plus the 16 kHz PCM when kept.
        assert_eq!(display.retained_bytes(), 8_000 * 4);
        assert_eq!(full.retained_bytes(), 8_000 * 4 + 16_000 * 4);
        assert!(display.overview_render_model(0.0).is_some());
    }
}
