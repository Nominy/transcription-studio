use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    mem::size_of,
    sync::Arc,
};

use crate::timeline_model::TimeRange;
use crate::transcript_model::TranscriptAnnotationKind;
use studio_core::Segment;
use studio_waveform::{
    DecodedAudio, PeakCache, WaveViewport, WaveformRenderModel, WaveformRenderOptions,
    WaveformSource,
};

const WAVEFORM_PEAK_COUNT: usize = 256;
const TIMELINE_RENDER_PEAK_COUNT: usize = 2_048;
const TIMELINE_RENDER_WIDTH_PX: f64 = 2_000.0;
const TIMELINE_RENDER_MODEL_CACHE_CAPACITY: usize = 8;
// Timeline render-model retention is bounded by entry count and by an estimated byte
// budget so zoom churn cannot pin unbounded strip/sample allocations per lane.
// Together with the `PeakCache` byte budget (16 MiB), this keeps each lane's
// waveform caches at or below ~20 MiB regardless of zoom history.
const TIMELINE_RENDER_MODEL_CACHE_BYTE_BUDGET: usize = 4 * 1024 * 1024;
// Rough per-entry struct overhead (model fields, Arc headers, cursor/region
// geometry) used by the cache byte estimator; sample buffers are counted exactly.
const CACHED_TIMELINE_MODEL_BASE_BYTES: usize = 512;
const CACHED_TIMELINE_MODEL_CHANNEL_BYTES: usize = 128;
const OVERVIEW_RENDER_PEAK_COUNT: usize = 1_024;
const OVERVIEW_RENDER_WIDTH_PX: f64 = 1_000.0;
// Segment-derived caches retain only the current variant for each stable segment ID.
// These caps also bound retention when segments are deleted or IDs churn over a long session.
const SEGMENT_PEAK_CACHE_CAPACITY: usize = 512;
const WAVEFORM_BLOCK_STATIC_CACHE_CAPACITY: usize = 512;

#[derive(Debug)]
pub struct LaneWaveform {
    source: WaveformSource,
    peak_cache: RefCell<PeakCache>,
    render_cache: RefCell<LaneWaveformRenderCache>,
    segment_peak_cache: RefCell<SegmentPeakCache>,
    block_static_cache: RefCell<WaveformBlockStaticCache>,
}

impl Clone for LaneWaveform {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            peak_cache: RefCell::new(PeakCache::new(self.source.clone())),
            render_cache: RefCell::new(LaneWaveformRenderCache::default()),
            segment_peak_cache: RefCell::new(SegmentPeakCache::default()),
            block_static_cache: RefCell::new(WaveformBlockStaticCache::default()),
        }
    }
}

impl PartialEq for LaneWaveform {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl LaneWaveform {
    pub fn from_decoded(decoded: DecodedAudio) -> Self {
        let source = WaveformSource::from_decoded(decoded);
        Self {
            peak_cache: RefCell::new(PeakCache::new(source.clone())),
            source,
            render_cache: RefCell::new(LaneWaveformRenderCache::default()),
            segment_peak_cache: RefCell::new(SegmentPeakCache::default()),
            block_static_cache: RefCell::new(WaveformBlockStaticCache::default()),
        }
    }

    pub fn from_precomputed_peaks(
        peaks: Vec<Vec<f64>>,
        duration_seconds: f64,
    ) -> Result<Self, String> {
        let source = WaveformSource::from_precomputed_peaks(peaks, duration_seconds)?;
        Ok(Self {
            peak_cache: RefCell::new(PeakCache::new(source.clone())),
            source,
            render_cache: RefCell::new(LaneWaveformRenderCache::default()),
            segment_peak_cache: RefCell::new(SegmentPeakCache::default()),
            block_static_cache: RefCell::new(WaveformBlockStaticCache::default()),
        })
    }

    pub fn decoded_audio(&self) -> Option<&DecodedAudio> {
        match &self.source {
            WaveformSource::Decoded(decoded) => Some(decoded.as_ref()),
            WaveformSource::Peaks(_) => None,
        }
    }

    pub fn decoded_audio_shared(&self) -> Option<Arc<DecodedAudio>> {
        match &self.source {
            WaveformSource::Decoded(decoded) => Some(Arc::clone(decoded)),
            WaveformSource::Peaks(_) => None,
        }
    }

    pub fn duration_seconds(&self) -> f64 {
        self.source.duration()
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
        WaveformBlock::from_static_block(segment, viewport, static_block)
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
    ) -> Arc<[WaveformBlock]> {
        segments
            .into_iter()
            .map(|segment| self.block_for_segment_with_static_source(segment, segment, viewport))
            .collect::<Vec<_>>()
            .into()
    }

    pub fn render_model(
        &self,
        segments: &[&Segment],
        viewport: TimeRange,
        playhead_seconds: f64,
    ) -> Option<WaveformRenderModel> {
        let _ = segments;
        let key = WaveformRenderCacheKey::timeline(viewport);
        // Bind the cached model first: the `if let` scrutinee temporary would
        // otherwise hold the RefCell borrow across the body, and the re-store
        // below borrows the same cell again.
        let cached_model = self.render_cache.borrow_mut().timeline_model(&key);
        if let Some(model) = cached_model {
            if model.visible_start_seconds.to_bits() == viewport.start_seconds.to_bits()
                && model.visible_end_seconds.to_bits() == viewport.end_seconds.to_bits()
            {
                return Some(with_cursor(model, playhead_seconds, 1.0));
            }
            let model = retarget_timeline_visible_window(model, viewport, playhead_seconds);
            // Keep the retargeted model in the cache so the next frame clones the
            // shifted strip instead of resampling a fresh bounded strip every frame
            // once playback walks beyond the previous strip window.
            self.render_cache
                .borrow_mut()
                .store_timeline_model(key, model.clone());
            return Some(model);
        }
        let visible_duration = viewport.duration_seconds();
        let min_px_per_sec = TIMELINE_RENDER_WIDTH_PX / visible_duration;
        let mut wave_viewport = WaveViewport::new(
            self.source.duration(),
            TIMELINE_RENDER_WIDTH_PX,
            min_px_per_sec,
            false,
            1.0,
        );
        wave_viewport.set_scroll_time(viewport.start_seconds);

        let model = {
            let mut peak_cache = self.peak_cache.borrow_mut();
            WaveformRenderModel::from_peak_cache_full_resolution(
                &mut peak_cache,
                wave_viewport,
                &[],
                playhead_seconds,
                WaveformRenderOptions {
                    channels: 1,
                    peak_count: TIMELINE_RENDER_PEAK_COUNT,
                    precision: 10_000.0,
                    ..WaveformRenderOptions::default()
                },
            )
            .ok()?
        };
        let model = retarget_timeline_visible_window(model, viewport, playhead_seconds);
        self.render_cache
            .borrow_mut()
            .store_timeline_model(key, model.clone());
        Some(model)
    }

    pub fn overview_render_model(&self, playhead_seconds: f64) -> Option<WaveformRenderModel> {
        let key = WaveformRenderCacheKey::overview();
        if let Some(model) = self.render_cache.borrow_mut().overview_model(&key) {
            return Some(with_cursor(model, playhead_seconds, 2.0));
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
            .store_overview_model(key, model.clone());
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

fn retarget_timeline_visible_window(
    model: WaveformRenderModel,
    viewport: TimeRange,
    playhead_seconds: f64,
) -> WaveformRenderModel {
    model.with_visible_window_for_scroll_strip(
        viewport.start_seconds,
        viewport.end_seconds,
        &[],
        playhead_seconds,
        1.0,
    )
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
    label: String,
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
            label: waveform_region_label(&segment.text),
            tooltip: segment.text.clone(),
            peaks,
            peak_source,
        }
    }

    fn synthetic(segment: &Segment) -> Self {
        Self {
            segment_id: segment.id.clone(),
            label: waveform_region_label(&segment.text),
            tooltip: segment.text.clone(),
            peaks: Arc::<[f32]>::from(waveform_peaks(&segment.id, WAVEFORM_PEAK_COUNT)),
            peak_source: WaveformPeakSource::Synthetic,
        }
    }
}

#[derive(Clone, Debug)]
struct LaneWaveformRenderCache {
    timeline: Vec<CachedWaveformRenderModel>,
    timeline_cached_bytes: usize,
    timeline_byte_budget: usize,
    overview: Option<CachedWaveformRenderModel>,
    stats: WaveformRenderCacheStats,
}

impl Default for LaneWaveformRenderCache {
    fn default() -> Self {
        Self {
            timeline: Vec::new(),
            timeline_cached_bytes: 0,
            timeline_byte_budget: TIMELINE_RENDER_MODEL_CACHE_BYTE_BUDGET,
            overview: None,
            stats: WaveformRenderCacheStats::default(),
        }
    }
}

impl LaneWaveformRenderCache {
    fn timeline_model(&mut self, key: &WaveformRenderCacheKey) -> Option<WaveformRenderModel> {
        if let Some(index) = self.timeline.iter().position(|cached| cached.key == *key) {
            self.stats.hits += 1;
            let cached = self.timeline.remove(index);
            let model = cached.model.clone();
            self.timeline.push(cached);
            return Some(model);
        }

        self.stats.misses += 1;
        None
    }

    fn overview_model(&mut self, key: &WaveformRenderCacheKey) -> Option<WaveformRenderModel> {
        if let Some(cached) = self.overview.as_ref()
            && cached.key == *key
        {
            self.stats.hits += 1;
            return Some(cached.model.clone());
        }

        self.stats.misses += 1;
        None
    }

    fn store_timeline_model(&mut self, key: WaveformRenderCacheKey, model: WaveformRenderModel) {
        if let Some(index) = self.timeline.iter().position(|cached| cached.key == key) {
            let removed = self.timeline.remove(index);
            self.timeline_cached_bytes = self
                .timeline_cached_bytes
                .saturating_sub(removed.estimated_bytes);
        }
        let estimated_bytes = cached_timeline_render_model_bytes(&model);
        self.timeline_cached_bytes = self.timeline_cached_bytes.saturating_add(estimated_bytes);
        self.timeline.push(CachedWaveformRenderModel {
            key,
            model,
            estimated_bytes,
        });

        // Evict least-recently-used entries (front) until within both bounds; the
        // just-stored entry is kept even if it alone exceeds the byte budget.
        while self.timeline.len() > 1
            && (self.timeline.len() > TIMELINE_RENDER_MODEL_CACHE_CAPACITY
                || self.timeline_cached_bytes > self.timeline_byte_budget)
        {
            let evicted = self.timeline.remove(0);
            self.timeline_cached_bytes = self
                .timeline_cached_bytes
                .saturating_sub(evicted.estimated_bytes);
        }
    }

    fn store_overview_model(&mut self, key: WaveformRenderCacheKey, model: WaveformRenderModel) {
        let estimated_bytes = cached_timeline_render_model_bytes(&model);
        self.overview = Some(CachedWaveformRenderModel {
            key,
            model,
            estimated_bytes,
        });
    }
}

/// Estimated retained bytes for one cached timeline render model. Counts the
/// per-model f32 sample buffers (visible + scroll-strip peaks) exactly; the f64
/// backing peaks are shared with `PeakCache`, which tracks them under its own byte
/// budget, so they are not double-counted here.
fn cached_timeline_render_model_bytes(model: &WaveformRenderModel) -> usize {
    let sample_count = model
        .channels
        .iter()
        .map(|channel| {
            channel
                .visible_peaks
                .len()
                .saturating_add(channel.scroll_strip_peaks.len())
        })
        .sum::<usize>();
    CACHED_TIMELINE_MODEL_BASE_BYTES
        .saturating_add(
            model
                .channels
                .len()
                .saturating_mul(CACHED_TIMELINE_MODEL_CHANNEL_BYTES),
        )
        .saturating_add(sample_count.saturating_mul(size_of::<f32>()))
}

#[derive(Clone, Debug)]
struct CachedWaveformRenderModel {
    key: WaveformRenderCacheKey,
    model: WaveformRenderModel,
    estimated_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WaveformRenderCacheKey {
    kind: WaveformRenderCacheKind,
    duration_seconds_bits: u64,
}

impl WaveformRenderCacheKey {
    fn timeline(viewport: TimeRange) -> Self {
        Self {
            kind: WaveformRenderCacheKind::Timeline,
            duration_seconds_bits: viewport.duration_seconds().to_bits(),
        }
    }

    fn overview() -> Self {
        Self {
            kind: WaveformRenderCacheKind::Overview,
            duration_seconds_bits: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaveformRenderCacheKind {
    Timeline,
    Overview,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaveformRenderCacheStats {
    pub hits: usize,
    pub misses: usize,
}

fn with_cursor(
    model: WaveformRenderModel,
    playhead_seconds: f64,
    cursor_width_px: f64,
) -> WaveformRenderModel {
    model.with_playhead_cursor(playhead_seconds, cursor_width_px)
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
    pub color_token: String,
    pub waveform_model: WaveformRenderModel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverviewWaveformBlock {
    pub color_token: String,
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
    pub label: String,
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
    ) -> Self {
        if let Some(lane_waveform) = lane_waveform {
            return lane_waveform.block_for_segment_with_static_source(
                segment,
                static_source_segment,
                viewport,
            );
        }

        Self::from_static_block(
            segment,
            viewport,
            WaveformBlockStatic::synthetic(static_source_segment),
        )
    }

    fn from_static_block(
        segment: &Segment,
        viewport: TimeRange,
        static_block: WaveformBlockStatic,
    ) -> Self {
        let (left_percent, width_percent) = waveform_block_geometry_percentages(segment, viewport);

        Self {
            segment_id: static_block.segment_id,
            label: static_block.label,
            tooltip: static_block.tooltip,
            is_transcription: TranscriptAnnotationKind::from_segment(segment).is_transcription(),
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

pub fn waveform_region_label(text: &str) -> String {
    const MAX_CHARS: usize = 28;
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }

    let mut label = normalized.chars().take(MAX_CHARS).collect::<String>();
    label.push_str("...");
    label
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
    use std::{mem::size_of, sync::Arc};

    use studio_core::Segment;

    use super::{
        CACHED_TIMELINE_MODEL_BASE_BYTES, CACHED_TIMELINE_MODEL_CHANNEL_BYTES, LaneWaveform,
        SEGMENT_PEAK_CACHE_CAPACITY, SegmentPeakCache, SegmentPeakCacheKey,
        TIMELINE_RENDER_MODEL_CACHE_BYTE_BUDGET, TIMELINE_RENDER_MODEL_CACHE_CAPACITY,
        TIMELINE_RENDER_PEAK_COUNT, TimeRange, WAVEFORM_BLOCK_STATIC_CACHE_CAPACITY, WaveformBlock,
        WaveformBlockStatic, WaveformBlockStaticCache, WaveformBlockStaticCacheKey,
        WaveformPeakSource, WaveformRenderCacheKey, cached_timeline_render_model_bytes,
    };

    #[test]
    fn waveform_block_carries_babel_region_label_and_full_tooltip_text() {
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
        );

        assert_eq!(block.label, "This is a long transcription...");
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

            let block =
                WaveformBlock::for_segment_with_static_source(&segment, &segment, viewport, None);
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

            let block = waveform.block_for_segment_with_static_source(&segment, &segment, viewport);
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
                    label: segment_id.clone(),
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
    fn waveform_block_geometry_hot_path_does_not_allocate_temporary_regions() {
        let source = include_str!("waveform.rs");
        let hot_path = source
            .split("fn from_static_block")
            .nth(1)
            .expect("waveform block hot path should exist")
            .split("pub fn waveform_region_label")
            .next()
            .expect("hot path should end before label helper");

        assert!(
            !hot_path.contains("Region::new("),
            "timeline playback rebuilds visible blocks while the viewport moves, so the block geometry hot path must not allocate temporary Region ids"
        );
        assert!(
            !hot_path.contains(".geometry()"),
            "timeline block geometry should be computed directly instead of constructing a throwaway RegionGeometry"
        );
    }

    fn render_cache_test_waveform() -> LaneWaveform {
        LaneWaveform::from_precomputed_peaks(vec![vec![0.5; 256]], 600.0)
            .expect("valid peak source")
    }

    fn viewport_with_duration(duration_seconds: f64) -> TimeRange {
        TimeRange {
            start_seconds: 0.0,
            end_seconds: duration_seconds,
        }
    }

    #[test]
    fn cached_timeline_render_model_bytes_counts_sample_buffers_and_overhead() {
        let waveform = render_cache_test_waveform();
        let model = waveform
            .render_model(&[], viewport_with_duration(10.0), 0.0)
            .expect("render model should build");
        let channel = model.channels.first().expect("model should have a channel");

        let expected = CACHED_TIMELINE_MODEL_BASE_BYTES
            + CACHED_TIMELINE_MODEL_CHANNEL_BYTES
            + (channel.visible_peaks.len() + channel.scroll_strip_peaks.len()) * size_of::<f32>();
        assert_eq!(cached_timeline_render_model_bytes(&model), expected);
        assert!(expected > channel.scroll_strip_peaks.len() * size_of::<f32>());
    }

    #[test]
    fn timeline_render_cache_keeps_at_most_capacity_entries() {
        let waveform = render_cache_test_waveform();
        for index in 0..=TIMELINE_RENDER_MODEL_CACHE_CAPACITY {
            let viewport = viewport_with_duration(10.0 + index as f64);
            waveform
                .render_model(&[], viewport, 0.0)
                .expect("render model should build");
        }

        let cache = waveform.render_cache.borrow();
        assert_eq!(cache.timeline.len(), TIMELINE_RENDER_MODEL_CACHE_CAPACITY);
        assert!(
            !cache.timeline.iter().any(|cached| cached.key
                == WaveformRenderCacheKey::timeline(viewport_with_duration(10.0))),
            "the oldest entry should be evicted once the count cap is exceeded"
        );
        let newest_duration = 10.0 + TIMELINE_RENDER_MODEL_CACHE_CAPACITY as f64;
        assert!(cache.timeline.iter().any(|cached| cached.key
            == WaveformRenderCacheKey::timeline(viewport_with_duration(newest_duration))));
        assert!(cache.timeline_cached_bytes <= TIMELINE_RENDER_MODEL_CACHE_BYTE_BUDGET);
    }

    #[test]
    fn timeline_render_cache_byte_budget_evicts_least_recently_used_entries() {
        let waveform = render_cache_test_waveform();
        let first_viewport = viewport_with_duration(10.0);
        waveform
            .render_model(&[], first_viewport, 0.0)
            .expect("first render model should build");
        let entry_bytes = waveform.render_cache.borrow().timeline_cached_bytes;
        assert!(entry_bytes > 0);
        // Keep the budget just above two entries so storing a third must evict.
        waveform.render_cache.borrow_mut().timeline_byte_budget = 2 * entry_bytes + 1;

        let second_viewport = viewport_with_duration(20.0);
        waveform
            .render_model(&[], second_viewport, 0.0)
            .expect("second render model should build");
        // Touch the first entry so the second becomes the eviction candidate.
        waveform
            .render_model(&[], first_viewport, 1.0)
            .expect("first viewport should hit the cache");
        let third_viewport = viewport_with_duration(30.0);
        waveform
            .render_model(&[], third_viewport, 0.0)
            .expect("third render model should build");

        let cache = waveform.render_cache.borrow();
        assert_eq!(cache.timeline.len(), 2);
        assert!(
            cache
                .timeline
                .iter()
                .any(|cached| cached.key == WaveformRenderCacheKey::timeline(first_viewport))
        );
        assert!(
            !cache
                .timeline
                .iter()
                .any(|cached| cached.key == WaveformRenderCacheKey::timeline(second_viewport)),
            "the least recently used entry should be evicted when the byte budget is exceeded"
        );
        assert!(
            cache
                .timeline
                .iter()
                .any(|cached| cached.key == WaveformRenderCacheKey::timeline(third_viewport))
        );
        assert!(cache.timeline_cached_bytes <= cache.timeline_byte_budget);
    }

    #[test]
    fn timeline_render_cache_retarget_replaces_entry_without_growing_bytes() {
        let waveform = render_cache_test_waveform();
        let viewport = viewport_with_duration(10.0);
        waveform
            .render_model(&[], viewport, 0.0)
            .expect("render model should build");
        let initial_bytes = waveform.render_cache.borrow().timeline_cached_bytes;

        // Same duration (same cache key) at a new scroll offset retargets and
        // re-stores the model; the cache must not double-count the replacement.
        let shifted = TimeRange {
            start_seconds: 5.0,
            end_seconds: 15.0,
        };
        waveform
            .render_model(&[], shifted, 5.0)
            .expect("retargeted render model should build");

        let cache = waveform.render_cache.borrow();
        assert_eq!(cache.timeline.len(), 1);
        assert_eq!(cache.timeline_cached_bytes, initial_bytes);
    }

    #[test]
    fn timeline_render_model_strip_stays_bounded_and_dense_across_zoom_levels() {
        let waveform = LaneWaveform::from_precomputed_peaks(vec![vec![0.5; 256]], 3_600.0)
            .expect("valid peak source");

        // Extreme zoom-in on long audio: a 0.5s window of one hour. The strip
        // previously sampled the whole source proportionally to the zoom ratio
        // (~4MB per entry); it must now stay bounded without losing density.
        let zoomed = waveform
            .render_model(
                &[],
                TimeRange {
                    start_seconds: 100.0,
                    end_seconds: 100.5,
                },
                100.25,
            )
            .expect("zoomed render model should build");
        let channel = zoomed
            .channels
            .first()
            .expect("model should have a channel");
        let strip_duration = channel.scroll_strip_end_seconds - channel.scroll_strip_start_seconds;
        assert!(channel.scroll_strip_peaks.len() <= 8 * TIMELINE_RENDER_PEAK_COUNT);
        assert!(strip_duration <= 8.0 * 0.5 + 1e-9);
        assert!(channel.scroll_strip_start_seconds <= 100.0);
        assert!(channel.scroll_strip_end_seconds >= 100.5);
        let visible_samples = channel.scroll_strip_peaks.len() as f64 * (0.5 / strip_duration);
        assert!(
            visible_samples >= TIMELINE_RENDER_PEAK_COUNT as f64 - 1.0,
            "the visible window must keep a full window of samples so the strip never renders sparse"
        );

        // Full zoom-out: the strip covers the whole source and never renders empty.
        let zoomed_out = waveform
            .render_model(
                &[],
                TimeRange {
                    start_seconds: 0.0,
                    end_seconds: 3_600.0,
                },
                1_800.0,
            )
            .expect("zoomed-out render model should build");
        let channel = zoomed_out
            .channels
            .first()
            .expect("model should have a channel");
        assert_eq!(channel.scroll_strip_start_seconds, 0.0);
        assert_eq!(channel.scroll_strip_end_seconds, 3_600.0);
        assert_eq!(channel.scroll_strip_peaks.len(), TIMELINE_RENDER_PEAK_COUNT);
    }
}
