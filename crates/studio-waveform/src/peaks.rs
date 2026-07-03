use rayon::prelude::*;

use crate::DecodedAudio;

const PARALLEL_BUCKET_MAX_MIN: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeakExportError {
    NoChannelsRequested,
    EmptyExport,
    InvalidRange,
    InvalidPrecision,
}

pub fn export_peaks(
    decoded: &DecodedAudio,
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

    let channel_count = channels.min(decoded.number_of_channels());
    let ranges = vec![(0, decoded.frame_count()); channel_count];

    export_peaks_for_ranges(decoded, channel_count, &ranges, max_length, precision)
}

pub fn export_range_peaks(
    decoded: &DecodedAudio,
    start_seconds: f64,
    end_seconds: f64,
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
    if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds >= end_seconds {
        return Err(PeakExportError::InvalidRange);
    }

    let start_seconds = start_seconds.clamp(0.0, decoded.duration());
    let end_seconds = end_seconds.clamp(0.0, decoded.duration());
    if start_seconds >= end_seconds {
        return Err(PeakExportError::InvalidRange);
    }

    let start_frame = (start_seconds * decoded.sample_rate() as f64).floor() as usize;
    let end_frame = (end_seconds * decoded.sample_rate() as f64).ceil() as usize;
    let start_frame = start_frame.min(decoded.frame_count());
    let end_frame = end_frame.min(decoded.frame_count());
    if start_frame >= end_frame {
        return Err(PeakExportError::InvalidRange);
    }

    let channel_count = channels.min(decoded.number_of_channels());
    let ranges = vec![(start_frame, end_frame); channel_count];

    export_peaks_for_ranges(decoded, channel_count, &ranges, max_length, precision)
}

fn export_peaks_for_ranges(
    decoded: &DecodedAudio,
    channel_count: usize,
    ranges: &[(usize, usize)],
    max_length: usize,
    precision: f64,
) -> Result<Vec<Vec<f64>>, PeakExportError> {
    let mut exported = Vec::with_capacity(channel_count);

    for (channel_index, &(start_frame, end_frame)) in ranges.iter().enumerate().take(channel_count)
    {
        let samples = decoded
            .channel_data(channel_index)
            .expect("channel index is bounded by number_of_channels");
        let samples = &samples[start_frame..end_frame];
        exported.push(bucket_max_f32(samples, max_length, precision));
    }

    Ok(exported)
}

pub(crate) fn bucket_max_f32(samples: &[f32], max_length: usize, precision: f64) -> Vec<f64> {
    bucket_max_with(samples, max_length, |peak| {
        (peak * precision).round() / precision
    })
}

pub(crate) fn bucket_max(samples: &[f64], max_length: usize, precision: f64) -> Vec<f64> {
    bucket_max_with(samples, max_length, |peak| {
        (peak * precision).round() / precision
    })
}

pub(crate) fn bucket_max_already_rounded(samples: &[f64], max_length: usize) -> Vec<f64> {
    bucket_max_with(samples, max_length, |peak| peak)
}

fn bucket_max_with(
    samples: &[impl Copy + Into<f64> + Sync],
    max_length: usize,
    finalize_peak: impl Fn(f64) -> f64 + Copy + Sync,
) -> Vec<f64> {
    if max_length >= PARALLEL_BUCKET_MAX_MIN && samples.len() >= PARALLEL_BUCKET_MAX_MIN {
        return (0..max_length)
            .into_par_iter()
            .map(|bucket_index| bucket_max_at(samples, max_length, bucket_index, finalize_peak))
            .collect();
    }

    (0..max_length)
        .map(|bucket_index| bucket_max_at(samples, max_length, bucket_index, finalize_peak))
        .collect()
}

fn bucket_max_at(
    samples: &[impl Copy + Into<f64>],
    max_length: usize,
    bucket_index: usize,
    finalize_peak: impl Fn(f64) -> f64,
) -> f64 {
    let bucket_width = samples.len() as f64 / max_length as f64;
    let start = (bucket_index as f64 * bucket_width).floor() as usize;
    let end = (((bucket_index + 1) as f64) * bucket_width)
        .ceil()
        .min(samples.len() as f64) as usize;
    let mut peak = 0.0_f64;
    let mut peak_abs = 0.0_f64;

    for sample in &samples[start..end] {
        let sample = (*sample).into();
        let sample_abs = sample.abs();
        if sample_abs > peak_abs {
            peak = sample;
            peak_abs = sample_abs;
        }
    }

    finalize_peak(peak)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_rounded_bucket_max_matches_regular_bucket_max_for_cached_peaks() {
        let precision = 100.0;
        let samples: [f64; 11] = [
            0.0, -0.12, 0.35, -0.72, 0.41, 0.63, -0.98, 0.22, -0.31, 0.44, 0.55,
        ];
        let rounded_samples = samples
            .iter()
            .copied()
            .map(|sample| (sample * precision).round() / precision)
            .collect::<Vec<_>>();

        for max_length in 1..=16 {
            assert_eq!(
                bucket_max(&rounded_samples, max_length, precision),
                bucket_max_already_rounded(&rounded_samples, max_length)
            );
        }
    }

    #[test]
    fn bucket_max_tracks_current_peak_abs_without_recomputing_it_per_sample() {
        let source = include_str!("peaks.rs");
        let bucket_max_with = source
            .split("fn bucket_max_with(")
            .nth(1)
            .expect("bucket max helper should exist")
            .split("#[cfg(test)]")
            .next()
            .expect("tests should follow bucket max helper");

        assert!(bucket_max_with.contains("let mut peak_abs = 0.0_f64;"));
        assert!(
            !bucket_max_with.contains("peak.abs()"),
            "peak export is a startup hot loop and should not recompute the current peak abs for every sample"
        );
    }

    #[test]
    fn large_bucket_exports_use_parallel_iterators() {
        let source = include_str!("peaks.rs");
        let imports_and_constants = source
            .split("#[derive(Clone, Debug, PartialEq, Eq)]")
            .next()
            .expect("peak export constants should precede the error type");
        let bucket_max_with = source
            .split("fn bucket_max_with(")
            .nth(1)
            .expect("bucket max helper should exist")
            .split("#[cfg(test)]")
            .next()
            .expect("tests should follow bucket max helper");

        assert!(imports_and_constants.contains("use rayon::prelude::*;"));
        assert!(imports_and_constants.contains("const PARALLEL_BUCKET_MAX_MIN: usize"));
        assert!(bucket_max_with.contains("max_length >= PARALLEL_BUCKET_MAX_MIN"));
        assert!(
            bucket_max_with.contains("into_par_iter()"),
            "large peak exports are load-time hot loops and should use available CPU cores"
        );
    }
}
