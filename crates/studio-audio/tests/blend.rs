use std::sync::Arc;

use studio_audio::{AudioEngine, DecodedAudio, ManualClock, blend_lane_audio};

const RATE: u32 = 1_000;
const FRAMES: usize = 1_000;

/// Deterministic pseudo-random samples in `-amplitude..amplitude`.
fn noise(seed: u32, amplitude: f32) -> Vec<f32> {
    let mut state = seed;
    (0..FRAMES)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (f64::from(state) / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * amplitude
        })
        .collect()
}

/// Interleaved output of `original` (with `enhanced` installed) playing alone at `ratio`.
fn played(original: &Arc<DecodedAudio>, enhanced: &Arc<DecodedAudio>, ratio: f32) -> Vec<f32> {
    let channels = original.number_of_channels();
    let (mut engine, mut output) = AudioEngine::offline(channels as u16, RATE, ManualClock::new());
    engine.insert_lane("a", Arc::clone(original)).unwrap();
    engine
        .set_enhanced_audio(vec![("a".to_string(), Arc::clone(enhanced))])
        .unwrap();
    engine.set_lane_enhanced_ratio("a", ratio);
    engine.play().unwrap();
    output.render(FRAMES * channels)
}

fn interleaved(audio: &DecodedAudio) -> Vec<f32> {
    let channels = audio.number_of_channels();
    (0..audio.frame_count() * channels)
        .map(|index| audio.channel_data(index % channels).unwrap()[index / channels])
        .collect()
}

#[test]
fn blended_lane_audio_is_exactly_what_the_lane_plays_at_every_slider_step() {
    // Stereo original beyond full scale (the mixer clamps) beside mono enhanced PCM.
    let original = Arc::new(
        DecodedAudio::from_f32_channels(RATE, vec![noise(1, 1.4), noise(2, 0.6)]).unwrap(),
    );
    let enhanced = Arc::new(DecodedAudio::from_f32_channels(RATE, vec![noise(3, 1.2)]).unwrap());

    for percent in (0..=100_u8).step_by(10) {
        let ratio = f32::from(percent) / 100.0;
        let blended = blend_lane_audio(&original, &enhanced, ratio).unwrap();

        assert_eq!(blended.sample_rate(), RATE);
        assert_eq!(blended.frame_count(), FRAMES);
        let expected = played(&original, &enhanced, ratio);
        let actual = interleaved(&blended);
        assert!(
            actual
                .iter()
                .zip(&expected)
                .all(|(actual, expected)| actual.to_bits() == expected.to_bits()
                    || (*actual == 0.0 && *expected == 0.0)),
            "{percent}% differs from playback"
        );
    }
}

#[test]
fn blending_rejects_enhanced_audio_that_cannot_play_beside_the_original() {
    let original = DecodedAudio::from_f32_channels(RATE, vec![noise(1, 0.5)]).unwrap();
    let short = DecodedAudio::from_f32_channels(RATE, vec![vec![0.1; FRAMES - 1]]).unwrap();
    let other_rate = DecodedAudio::from_f32_channels(RATE * 2, vec![noise(2, 0.5)]).unwrap();
    let stereo = DecodedAudio::from_f32_channels(RATE, vec![noise(2, 0.5), noise(3, 0.5)]).unwrap();

    assert!(blend_lane_audio(&original, &short, 0.5).is_none());
    assert!(blend_lane_audio(&original, &other_rate, 0.5).is_none());
    assert!(blend_lane_audio(&original, &stereo, 0.5).is_none());
    assert!(blend_lane_audio(&original, &original, f32::NAN).is_none());
}
