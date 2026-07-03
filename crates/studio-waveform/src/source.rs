use std::sync::Arc;

use crate::{DecodedAudio, PeakExportError, export_peaks, export_range_peaks, peaks::bucket_max};

#[derive(Clone, Debug, PartialEq)]
pub enum WaveformSource {
    Decoded(Arc<DecodedAudio>),
    Peaks(Arc<PrecomputedPeaks>),
}

impl WaveformSource {
    pub fn from_decoded(decoded: DecodedAudio) -> Self {
        Self::Decoded(Arc::new(decoded))
    }

    pub fn from_precomputed_peaks(
        peaks: Vec<Vec<f64>>,
        duration_seconds: f64,
    ) -> Result<Self, String> {
        Ok(Self::Peaks(Arc::new(PrecomputedPeaks::new(
            peaks,
            duration_seconds,
        )?)))
    }

    pub fn duration(&self) -> f64 {
        match self {
            Self::Decoded(decoded) => decoded.duration(),
            Self::Peaks(peaks) => peaks.duration_seconds,
        }
    }

    pub fn number_of_channels(&self) -> usize {
        match self {
            Self::Decoded(decoded) => decoded.number_of_channels(),
            Self::Peaks(peaks) => peaks.channels.len(),
        }
    }

    pub fn export_peaks(
        &self,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        match self {
            Self::Decoded(decoded) => export_peaks(decoded, channels, max_length, precision),
            Self::Peaks(peaks) => peaks.export_peaks(channels, max_length, precision),
        }
    }

    pub fn export_range_peaks(
        &self,
        start_seconds: f64,
        end_seconds: f64,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        match self {
            Self::Decoded(decoded) => export_range_peaks(
                decoded,
                start_seconds,
                end_seconds,
                channels,
                max_length,
                precision,
            ),
            Self::Peaks(peaks) => peaks.export_range_peaks(
                start_seconds,
                end_seconds,
                channels,
                max_length,
                precision,
            ),
        }
    }

    pub fn render_window_samples(
        &self,
        offset_px: f64,
        clamped_width_px: f64,
        total_width_px: f64,
        channels: usize,
    ) -> Vec<Vec<f64>> {
        match self {
            Self::Decoded(decoded) => slice_channels(
                (0..channels.min(decoded.number_of_channels()))
                    .filter_map(|channel_index| decoded.channel_data(channel_index))
                    .collect(),
                offset_px,
                clamped_width_px,
                total_width_px,
            ),
            Self::Peaks(peaks) => slice_channels(
                peaks
                    .channels
                    .iter()
                    .take(channels.min(peaks.channels.len()))
                    .map(Vec::as_slice)
                    .collect(),
                offset_px,
                clamped_width_px,
                total_width_px,
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrecomputedPeaks {
    channels: Vec<Vec<f64>>,
    duration_seconds: f64,
}

impl PrecomputedPeaks {
    pub fn new(channels: Vec<Vec<f64>>, duration_seconds: f64) -> Result<Self, String> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err("precomputed peak duration must be greater than zero".to_string());
        }
        if channels.is_empty() {
            return Err("precomputed peaks must contain at least one channel".to_string());
        }

        let peak_count = channels[0].len();
        if peak_count == 0 {
            return Err("precomputed peak channels cannot be empty".to_string());
        }
        if channels.iter().any(|channel| channel.len() != peak_count) {
            return Err("precomputed peak channels must have the same length".to_string());
        }

        Ok(Self {
            channels,
            duration_seconds,
        })
    }

    fn export_peaks(
        &self,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        if channels == 0 {
            return Err(PeakExportError::NoChannelsRequested);
        }
        if max_length == 0 {
            return Err(PeakExportError::EmptyExport);
        }
        if !precision.is_finite() || precision <= 0.0 {
            return Err(PeakExportError::InvalidPrecision);
        }

        Ok(self
            .channels
            .iter()
            .take(channels.min(self.channels.len()))
            .map(|channel| bucket_max(channel, max_length, precision))
            .collect())
    }

    fn export_range_peaks(
        &self,
        start_seconds: f64,
        end_seconds: f64,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds >= end_seconds {
            return Err(PeakExportError::InvalidRange);
        }
        let start_seconds = start_seconds.clamp(0.0, self.duration_seconds);
        let end_seconds = end_seconds.clamp(0.0, self.duration_seconds);
        if start_seconds >= end_seconds {
            return Err(PeakExportError::InvalidRange);
        }

        let start_index = (start_seconds / self.duration_seconds * self.channels[0].len() as f64)
            .floor() as usize;
        let end_index =
            (end_seconds / self.duration_seconds * self.channels[0].len() as f64).ceil() as usize;
        Ok(self
            .channels
            .iter()
            .take(channels.min(self.channels.len()))
            .map(|channel| {
                let start_index = start_index.min(channel.len());
                let end_index = end_index.min(channel.len());
                bucket_max(&channel[start_index..end_index], max_length, precision)
            })
            .collect())
    }
}

fn slice_channels(
    channels: Vec<&[impl Copy + Into<f64>]>,
    offset_px: f64,
    clamped_width_px: f64,
    total_width_px: f64,
) -> Vec<Vec<f64>> {
    if total_width_px <= 0.0 || !total_width_px.is_finite() {
        return Vec::new();
    }

    channels
        .into_iter()
        .map(|channel| {
            let start = (finite_non_negative(offset_px) / total_width_px * channel.len() as f64)
                .floor() as usize;
            let end = ((finite_non_negative(offset_px) + finite_non_negative(clamped_width_px))
                / total_width_px
                * channel.len() as f64)
                .floor() as usize;
            let start = start.min(channel.len());
            let end = end.min(channel.len());
            channel[start..end]
                .iter()
                .copied()
                .map(sample_to_render_f64)
                .collect()
        })
        .collect()
}

fn sample_to_render_f64(sample: impl Into<f64>) -> f64 {
    const RENDER_SAMPLE_PRECISION: f64 = 1_000_000.0;
    let sample = sample.into();
    (sample * RENDER_SAMPLE_PRECISION).round() / RENDER_SAMPLE_PRECISION
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
