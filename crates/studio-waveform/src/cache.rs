use std::{collections::BTreeMap, mem::size_of, sync::Arc};

use crate::{PeakExportError, WaveformSource, peaks::bucket_max_already_rounded};

// Resolution entries and their peak-sample payloads are independently bounded. The
// entry cap also bounds per-entry allocation overhead that is not included in the
// payload byte count. Together with the application-side timeline render-model byte
// budget (4 MiB per lane), this keeps a lane's waveform caches at or below ~20 MiB
// regardless of zoom history.
const MAX_CACHED_RESOLUTIONS: usize = 8;
const MAX_CACHED_PEAK_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct PeakCache {
    source: WaveformSource,
    resolutions: BTreeMap<PeakResolution, CachedResolution>,
    cached_peak_bytes: usize,
    recency_clock: u64,
}

impl PartialEq for PeakCache {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.resolutions.len() == other.resolutions.len()
            && self.resolutions.iter().all(|(key, cached)| {
                other
                    .resolutions
                    .get(key)
                    .is_some_and(|other_cached| cached.peaks == other_cached.peaks)
            })
    }
}

impl PeakCache {
    pub fn new(source: WaveformSource) -> Self {
        Self {
            source,
            resolutions: BTreeMap::new(),
            cached_peak_bytes: 0,
            recency_clock: 0,
        }
    }

    pub fn source(&self) -> &WaveformSource {
        &self.source
    }

    pub fn cached_resolution_count(&self) -> usize {
        self.resolutions.len()
    }

    pub fn peaks_for_resolution(
        &mut self,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        Ok(self
            .cached_peaks_for_resolution(max_length, precision)?
            .iter()
            .map(|channel| channel.to_vec())
            .collect())
    }

    fn cached_peaks_for_resolution(
        &mut self,
        max_length: usize,
        precision: f64,
    ) -> Result<&[Arc<[f64]>], PeakExportError> {
        let key = PeakResolution::new(max_length, precision)?;
        let last_used = self.next_recency();
        if let Some(cached) = self.resolutions.get_mut(&key) {
            cached.last_used = last_used;
        } else {
            let peaks = self.source.export_peaks(
                self.source.number_of_channels(),
                max_length,
                precision,
            )?;
            let peaks = peaks
                .into_iter()
                .map(Arc::<[f64]>::from)
                .collect::<Vec<_>>();
            let peak_bytes = peaks
                .iter()
                .map(|channel| channel.len())
                .sum::<usize>()
                .checked_mul(size_of::<f64>())
                .expect("allocated peak sample byte count fits in usize");

            self.cached_peak_bytes = self
                .cached_peak_bytes
                .checked_add(peak_bytes)
                .expect("allocated cached peak byte count fits in usize");
            self.resolutions.insert(
                key,
                CachedResolution {
                    peaks,
                    peak_bytes,
                    last_used,
                },
            );
            self.evict_after_inserting(key);
        }

        Ok(self
            .resolutions
            .get(&key)
            .expect("newly requested resolution is preserved during eviction")
            .peaks
            .as_slice())
    }

    fn next_recency(&mut self) -> u64 {
        if self.recency_clock == u64::MAX {
            let mut by_recency = self
                .resolutions
                .iter()
                .map(|(key, cached)| (*key, cached.last_used))
                .collect::<Vec<_>>();
            by_recency.sort_unstable_by_key(|(_, last_used)| *last_used);
            for (index, (key, _)) in by_recency.into_iter().enumerate() {
                self.resolutions
                    .get_mut(&key)
                    .expect("recency key came from the cache")
                    .last_used = index as u64;
            }
            self.recency_clock = self.resolutions.len() as u64;
        }

        self.recency_clock += 1;
        self.recency_clock
    }

    fn evict_after_inserting(&mut self, preserved: PeakResolution) {
        while self.resolutions.len() > MAX_CACHED_RESOLUTIONS
            || self.cached_peak_bytes > MAX_CACHED_PEAK_BYTES
        {
            let Some(oldest) = self
                .resolutions
                .iter()
                .filter(|(key, _)| **key != preserved)
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(key, _)| *key)
            else {
                // A current resolution may exceed the byte budget by itself.
                break;
            };
            let removed = self
                .resolutions
                .remove(&oldest)
                .expect("oldest resolution came from the cache");
            self.cached_peak_bytes -= removed.peak_bytes;
        }
    }

    pub fn visible_peaks(
        &self,
        start_seconds: f64,
        end_seconds: f64,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        self.source
            .export_range_peaks(start_seconds, end_seconds, channels, max_length, precision)
    }

    pub fn visible_peaks_from_cached_resolution(
        &mut self,
        start_seconds: f64,
        end_seconds: f64,
        channels: usize,
        max_length: usize,
        precision: f64,
        resolution_length: usize,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        if channels == 0 {
            return Err(PeakExportError::NoChannelsRequested);
        }
        if max_length == 0 || resolution_length == 0 {
            return Err(PeakExportError::EmptyExport);
        }
        if !precision.is_finite() || precision <= 0.0 {
            return Err(PeakExportError::InvalidPrecision);
        }
        if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds >= end_seconds {
            return Err(PeakExportError::InvalidRange);
        }

        let duration = self.source.duration();
        let start_seconds = start_seconds.clamp(0.0, duration);
        let end_seconds = end_seconds.clamp(0.0, duration);
        if start_seconds >= end_seconds {
            return Err(PeakExportError::InvalidRange);
        }

        let cached = self.cached_peaks_for_resolution(resolution_length, precision)?;
        let start_index = (start_seconds / duration * resolution_length as f64).floor() as usize;
        let end_index = (end_seconds / duration * resolution_length as f64).ceil() as usize;
        let channel_count = channels.min(cached.len());

        Ok(cached
            .iter()
            .take(channel_count)
            .map(|channel| {
                let start_index = start_index.min(channel.len());
                let end_index = end_index.min(channel.len());
                bucket_max_already_rounded(&channel.as_ref()[start_index..end_index], max_length)
            })
            .collect())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct CachedResolution {
    peaks: Vec<Arc<[f64]>>,
    peak_bytes: usize,
    last_used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PeakResolution {
    max_length: usize,
    precision_bits: u64,
}

impl PeakResolution {
    fn new(max_length: usize, precision: f64) -> Result<Self, PeakExportError> {
        if max_length == 0 {
            return Err(PeakExportError::EmptyExport);
        }
        if !precision.is_finite() || precision <= 0.0 {
            return Err(PeakExportError::InvalidPrecision);
        }

        Ok(Self {
            max_length,
            precision_bits: precision.to_bits(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> WaveformSource {
        WaveformSource::from_precomputed_peaks(vec![vec![0.25, -0.5, 0.75, -1.0]], 1.0)
            .expect("valid precomputed peaks")
    }

    fn key(max_length: usize) -> PeakResolution {
        PeakResolution::new(max_length, 100.0).expect("valid resolution")
    }

    #[test]
    fn hundreds_of_unique_resolutions_remain_within_cache_bounds() {
        let mut cache = PeakCache::new(source());

        for max_length in 1..=300 {
            cache
                .cached_peaks_for_resolution(max_length, 100.0)
                .expect("valid peak request");
        }

        assert!(cache.cached_resolution_count() <= MAX_CACHED_RESOLUTIONS);
        assert!(cache.cached_peak_bytes <= MAX_CACHED_PEAK_BYTES);
        assert!(cache.resolutions.contains_key(&key(300)));
    }

    #[test]
    fn recently_reused_resolution_survives_next_entry_eviction() {
        let mut cache = PeakCache::new(source());
        for max_length in 1..=MAX_CACHED_RESOLUTIONS {
            cache
                .cached_peaks_for_resolution(max_length, 100.0)
                .expect("valid peak request");
        }

        cache
            .cached_peaks_for_resolution(1, 100.0)
            .expect("cached peak request");
        cache
            .cached_peaks_for_resolution(MAX_CACHED_RESOLUTIONS + 1, 100.0)
            .expect("new peak request");

        assert!(cache.resolutions.contains_key(&key(1)));
        assert!(!cache.resolutions.contains_key(&key(2)));
        assert!(
            cache
                .resolutions
                .contains_key(&key(MAX_CACHED_RESOLUTIONS + 1))
        );
    }

    #[test]
    fn cache_hits_preserve_peak_cache_equality_semantics() {
        let mut used = PeakCache::new(source());
        let mut unused = PeakCache::new(source());
        for max_length in 1..=MAX_CACHED_RESOLUTIONS {
            used.cached_peaks_for_resolution(max_length, 100.0)
                .expect("valid peak request");
            unused
                .cached_peaks_for_resolution(max_length, 100.0)
                .expect("valid peak request");
        }

        used.cached_peaks_for_resolution(1, 100.0)
            .expect("cached peak request");

        assert_eq!(used, unused);
    }

    #[test]
    fn byte_budget_evicts_old_entries_but_keeps_oversized_current_entry() {
        let mut cache = PeakCache::new(source());
        let first_length = 1_500_000;
        let second_length = 1_800_000;

        cache
            .cached_peaks_for_resolution(first_length, 100.0)
            .expect("first large peak request");
        cache
            .cached_peaks_for_resolution(second_length, 100.0)
            .expect("second large peak request");

        assert_eq!(cache.cached_resolution_count(), 1);
        assert!(cache.cached_peak_bytes <= MAX_CACHED_PEAK_BYTES);
        assert!(!cache.resolutions.contains_key(&key(first_length)));
        assert!(cache.resolutions.contains_key(&key(second_length)));

        let oversized_length = MAX_CACHED_PEAK_BYTES / size_of::<f64>() + 1;
        let oversized_len = cache
            .cached_peaks_for_resolution(oversized_length, 100.0)
            .expect("oversized peak request")[0]
            .len();

        assert_eq!(oversized_len, oversized_length);
        assert_eq!(cache.cached_resolution_count(), 1);
        assert!(cache.cached_peak_bytes > MAX_CACHED_PEAK_BYTES);
        assert!(cache.resolutions.contains_key(&key(oversized_length)));
    }
}
