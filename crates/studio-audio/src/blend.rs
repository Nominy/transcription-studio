//! The PCM a lane plays while enhanced audio is installed: its original and enhanced audio
//! blended sample by sample exactly as the mixer blends them.

use std::sync::Arc;

use studio_waveform::DecodedAudio;

use crate::source::LaneGain;

/// Why `enhanced` cannot play beside `original`, if it cannot: it must share the original's
/// sample rate and frame count and be mono (shared by every original channel) or have the
/// original's channel count.
pub(crate) fn enhanced_mismatch(
    original: &DecodedAudio,
    enhanced: &DecodedAudio,
) -> Option<String> {
    if enhanced.sample_rate() != original.sample_rate() {
        Some(format!(
            "sample rate {} Hz, original {} Hz",
            enhanced.sample_rate(),
            original.sample_rate()
        ))
    } else if enhanced.frame_count() != original.frame_count() {
        Some(format!(
            "{} frames, original {}",
            enhanced.frame_count(),
            original.frame_count()
        ))
    } else if enhanced.number_of_channels() != 1
        && enhanced.number_of_channels() != original.number_of_channels()
    {
        Some(format!(
            "{} channels, original {}",
            enhanced.number_of_channels(),
            original.number_of_channels()
        ))
    } else {
        None
    }
}

/// What a lane alone plays at unity volume with `enhanced_ratio` of its enhanced PCM
/// (clamped to `0..=1`, as [`AudioEngine::set_lane_enhanced_ratio`] does): every sample is
/// the mixer's contribution of that lane, on the original's clock and channels (mono
/// enhanced PCM is shared by all of them). `None` when `enhanced` could not play beside
/// `original` or the ratio is not finite.
///
/// O(frames × channels); run off the UI thread.
///
/// [`AudioEngine::set_lane_enhanced_ratio`]: crate::AudioEngine::set_lane_enhanced_ratio
pub fn blend_lane_audio(
    original: &DecodedAudio,
    enhanced: &DecodedAudio,
    enhanced_ratio: f32,
) -> Option<DecodedAudio> {
    if !enhanced_ratio.is_finite() || enhanced_mismatch(original, enhanced).is_some() {
        return None;
    }
    let gain = LaneGain::blend(1.0, enhanced_ratio.clamp(0.0, 1.0));
    let shared_enhanced = enhanced.number_of_channels() == 1;
    let channels = (0..original.number_of_channels())
        .map(|index| {
            let original = original.channel_data(index)?;
            let enhanced = enhanced.channel_data(if shared_enhanced { 0 } else { index })?;
            Some(
                original
                    .iter()
                    .zip(enhanced)
                    .map(|(&original, &enhanced)| {
                        let mut sample = 0.0_f32;
                        gain.accumulate(&mut sample, Some(original), Some(enhanced));
                        sample
                    })
                    .collect::<Arc<[f32]>>(),
            )
        })
        .collect::<Option<Vec<_>>>()?;
    DecodedAudio::from_shared_channels(original.sample_rate(), channels).ok()
}
