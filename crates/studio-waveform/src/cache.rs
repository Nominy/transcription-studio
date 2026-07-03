use std::{collections::BTreeMap, sync::Arc};

use crate::{PeakExportError, WaveformSource, peaks::bucket_max_already_rounded};

#[derive(Clone, Debug, PartialEq)]
pub struct PeakCache {
    source: WaveformSource,
    resolutions: BTreeMap<PeakResolution, Vec<Arc<[f64]>>>,
}

impl PeakCache {
    pub fn new(source: WaveformSource) -> Self {
        Self {
            source,
            resolutions: BTreeMap::new(),
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

    pub fn shared_peaks_for_resolution(
        &mut self,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Arc<[f64]>>, PeakExportError> {
        if channels == 0 {
            return Err(PeakExportError::NoChannelsRequested);
        }

        let channel_count = channels.min(self.source.number_of_channels());
        Ok(self
            .cached_peaks_for_resolution(max_length, precision)?
            .iter()
            .take(channel_count)
            .cloned()
            .collect())
    }

    fn cached_peaks_for_resolution(
        &mut self,
        max_length: usize,
        precision: f64,
    ) -> Result<&[Arc<[f64]>], PeakExportError> {
        let key = PeakResolution::new(max_length, precision)?;
        if !self.resolutions.contains_key(&key) {
            let peaks = self.source.export_peaks(
                self.source.number_of_channels(),
                max_length,
                precision,
            )?;
            self.resolutions.insert(
                key,
                peaks
                    .into_iter()
                    .map(Arc::<[f64]>::from)
                    .collect::<Vec<_>>(),
            );
        }

        Ok(self
            .resolutions
            .get(&key)
            .expect("resolution was inserted or already cached")
            .as_slice())
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

        let full_resolution = self.cached_peaks_for_resolution(resolution_length, precision)?;
        let start_index = (start_seconds / duration * resolution_length as f64).floor() as usize;
        let end_index = (end_seconds / duration * resolution_length as f64).ceil() as usize;
        let channel_count = channels.min(full_resolution.len());

        Ok(full_resolution
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
