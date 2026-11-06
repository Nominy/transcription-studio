//! Target-speaker clip encoding (Gold `encodeErrorTaskWav`): mono PCM16 WAV of at most ten
//! seconds of one already isolated recording, sample-rounded bounds.

use studio_waveform::DecodedAudio;

use super::ErrorTaskError;
use super::diff_align::AudioInterval;

/// Largest clip WAV accepted by the error-task protocol.
pub const MAX_CLIP_BYTES: usize = 4 * 1024 * 1024;

/// Encodes `range` of `audio` (channels averaged) as mono PCM16 WAV. Returns the bytes and
/// the sample-rounded interval actually encoded.
pub fn encode_clip_wav(
    audio: &DecodedAudio,
    range: AudioInterval,
) -> Result<(Vec<u8>, AudioInterval), ErrorTaskError> {
    let invalid = || {
        ErrorTaskError::new(
            "invalid-audio",
            "The audio clip is invalid or longer than 10 seconds.",
        )
    };
    let rate = audio.sample_rate();
    let channels = audio.number_of_channels();
    if !(8000..=96_000).contains(&rate)
        || !(1..=8).contains(&channels)
        || !range.start_seconds.is_finite()
        || !range.end_seconds.is_finite()
        || range.start_seconds < 0.0
        || range.end_seconds <= range.start_seconds
        || range.duration() > 10.001
        || range.end_seconds > audio.duration() + 0.001
    {
        return Err(invalid());
    }
    let rate_f = f64::from(rate);
    let first = (range.start_seconds * rate_f).floor() as usize;
    let last = audio
        .frame_count()
        .min((range.end_seconds * rate_f).floor() as usize)
        .min(first + rate as usize * 10);
    if last <= first {
        return Err(ErrorTaskError::new(
            "invalid-audio",
            "The audio clip is empty.",
        ));
    }
    let frames = last - first;
    let data_bytes = frames * 2;
    let mut bytes = Vec::with_capacity(44 + data_bytes);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    let data: Vec<&[f32]> = (0..channels)
        .filter_map(|index| audio.channel_data(index))
        .collect();
    for sample in first..last {
        let sum: f64 = data.iter().map(|channel| f64::from(channel[sample])).sum();
        let value = (sum / data.len() as f64).clamp(-1.0, 1.0);
        let scaled = value * if value < 0.0 { 32768.0 } else { 32767.0 };
        // JavaScript Math.round: halves round toward +∞.
        let pcm = (scaled + 0.5).floor() as i16;
        bytes.extend_from_slice(&pcm.to_le_bytes());
    }
    if bytes.len() > MAX_CLIP_BYTES {
        return Err(invalid());
    }
    Ok((
        bytes,
        AudioInterval::new(first as f64 / rate_f, last as f64 / rate_f),
    ))
}
