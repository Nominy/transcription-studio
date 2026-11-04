use std::{sync::Arc, time::Duration};

use studio_audio::{
    AudioEngine, AudioError, DecodedAudio, LoopRange, ManualClock, OfflineOutput, PlaybackEvent,
    PlaybackState,
};

const RATE: u32 = 1_000;
/// Samples per control span at `RATE` (20 ms); live changes apply within one.
const SPAN: usize = 20;

fn setup(channels: u16) -> (AudioEngine, OfflineOutput, ManualClock) {
    let clock = ManualClock::new();
    let (engine, output) = AudioEngine::offline(channels, RATE, clock.clone());
    (engine, output, clock)
}

fn mono(samples: Vec<f64>) -> Arc<DecodedAudio> {
    Arc::new(DecodedAudio::new(RATE, vec![samples]).expect("valid audio"))
}

fn constant(value: f64, seconds: f64) -> Arc<DecodedAudio> {
    mono(vec![value; (seconds * f64::from(RATE)) as usize])
}

fn ramp(frames: usize) -> Arc<DecodedAudio> {
    mono(
        (0..frames)
            .map(|frame| frame as f64 / frames as f64)
            .collect(),
    )
}

fn frames(audio: &DecodedAudio, range: std::ops::Range<usize>) -> Vec<f32> {
    audio.channel_data(0).expect("mono channel")[range].to_vec()
}

/// Lets pending control changes reach the source.
fn settle(output: &mut OfflineOutput) {
    output.render(SPAN);
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn play_mixes_loaded_lanes_and_pause_keeps_the_playhead() {
    let (mut engine, mut output, clock) = setup(1);
    engine.insert_lane("a", constant(0.25, 1.0)).unwrap();
    engine.insert_lane("b", constant(0.5, 1.0)).unwrap();

    assert_eq!(engine.play().unwrap(), 2);
    assert_eq!(engine.state(), PlaybackState::Playing);
    assert!(output.render(100).iter().all(|sample| *sample == 0.75));

    clock.advance(ms(50));
    engine.pause();

    assert_eq!(engine.state(), PlaybackState::Stopped);
    assert_close(engine.position(), 0.05);
    clock.advance(ms(500));
    assert_close(engine.position(), 0.05);
    let after_pause = output.render(100);
    assert!(after_pause[SPAN..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn rendered_audio_starts_at_the_playhead() {
    let (mut engine, mut output, _clock) = setup(1);
    let audio = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&audio)).unwrap();

    engine.play_from(0.3).unwrap();

    assert_eq!(output.render(50), frames(&audio, 300..350));
}

#[test]
fn transport_position_follows_clock_and_speed() {
    let (mut engine, _output, clock) = setup(1);
    engine.insert_lane("a", constant(0.1, 10.0)).unwrap();
    engine.seek(3.5);
    engine.set_speed(2.0);

    engine.play().unwrap();
    clock.advance(ms(16));
    assert_close(engine.position(), 3.532);

    engine.set_speed(0.5);
    clock.advance(ms(100));
    assert_close(engine.position(), 3.582);
}

#[test]
fn invalid_speeds_are_ignored_and_tiny_speeds_clamp() {
    let (mut engine, _output, _clock) = setup(1);
    engine.set_speed(f32::NAN);
    engine.set_speed(0.0);
    engine.set_speed(-1.0);
    assert_eq!(engine.speed(), 1.0);

    engine.set_speed(0.01);
    assert_eq!(engine.speed(), studio_audio::MIN_PLAYBACK_SPEED);
}

#[test]
fn playback_ends_at_the_timeline_end_and_next_play_restarts_from_zero() {
    let (mut engine, _output, clock) = setup(1);
    engine.insert_lane("a", constant(0.1, 1.0)).unwrap();
    engine.play_from(0.9).unwrap();

    clock.advance(ms(50));
    assert_eq!(engine.poll(), None);
    clock.advance(ms(150));
    assert_close(engine.position(), 1.0);
    assert_eq!(engine.poll(), Some(PlaybackEvent::Ended));
    assert_eq!(engine.state(), PlaybackState::Stopped);
    assert_close(engine.position(), 1.0);
    assert_eq!(engine.poll(), None);

    engine.play().unwrap();
    assert_eq!(engine.state(), PlaybackState::Playing);
    assert_close(engine.position(), 0.0);
}

#[test]
fn timeline_end_override_extends_playback_past_the_audio() {
    let (mut engine, mut output, clock) = setup(1);
    engine.insert_lane("a", constant(0.5, 0.1)).unwrap();
    assert_close(engine.duration(), 0.1);

    engine.set_timeline_end(Some(5.0));
    assert_close(engine.duration(), 5.0);
    engine.play_from(0.05).unwrap();
    let rendered = output.render(100);
    assert!(rendered[..50].iter().all(|sample| *sample == 0.5));
    assert!(rendered[50..].iter().all(|sample| *sample == 0.0));
    clock.advance(ms(1_000));
    assert_eq!(engine.poll(), None);
    assert_close(engine.position(), 1.05);

    engine.set_timeline_end(None);
    assert_close(engine.duration(), 0.1);
    assert_eq!(engine.poll(), Some(PlaybackEvent::Ended));
}

#[test]
fn playing_without_audio_still_runs_the_transport() {
    let (mut engine, _output, clock) = setup(1);
    engine.set_timeline_end(Some(10.0));

    assert_eq!(engine.play().unwrap(), 0);
    clock.advance(ms(1_000));

    assert_close(engine.position(), 1.0);
}

#[test]
fn audition_plays_the_range_once_and_returns_to_its_start() {
    let (mut engine, mut output, clock) = setup(1);
    let audio = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&audio)).unwrap();
    engine.seek(0.7);

    assert_eq!(engine.audition(0.2, 0.3).unwrap(), 1);
    assert_eq!(
        engine.state(),
        PlaybackState::Auditioning {
            start_seconds: 0.2,
            end_seconds: 0.3
        }
    );
    assert_eq!(output.render(100), frames(&audio, 200..300));
    assert!(output.render(50).iter().all(|sample| *sample == 0.0));

    clock.advance(ms(60));
    assert_close(engine.position(), 0.26);
    assert_eq!(engine.poll(), None);
    clock.advance(ms(60));
    assert_eq!(engine.poll(), Some(PlaybackEvent::AuditionEnded));
    assert_eq!(engine.state(), PlaybackState::Stopped);
    assert_close(engine.position(), 0.2);
}

#[test]
fn audition_ignores_loop_and_invalid_ranges() {
    let (mut engine, mut output, _clock) = setup(1);
    let audio = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&audio)).unwrap();
    engine.set_loop(Some(LoopRange {
        start_seconds: 0.1,
        end_seconds: 0.15,
    }));

    assert_eq!(engine.audition(0.3, 0.3).unwrap(), 0);
    assert_eq!(engine.audition(f64::NAN, 1.0).unwrap(), 0);
    assert_eq!(engine.state(), PlaybackState::Stopped);

    engine.audition(0.1, 0.2).unwrap();
    assert_eq!(output.render(100), frames(&audio, 100..200));
}

#[test]
fn pausing_or_playing_during_an_audition_resumes_from_its_start() {
    let (mut engine, _output, clock) = setup(1);
    engine.insert_lane("a", constant(0.1, 2.0)).unwrap();

    engine.audition(0.5, 1.0).unwrap();
    clock.advance(ms(200));
    engine.pause();
    assert_close(engine.position(), 0.5);

    engine.audition(0.5, 1.0).unwrap();
    clock.advance(ms(200));
    engine.play().unwrap();
    assert_eq!(engine.state(), PlaybackState::Playing);
    assert_close(engine.position(), 0.5);
}

#[test]
fn seek_clamps_to_the_timeline_and_cancels_an_audition() {
    let (mut engine, _output, _clock) = setup(1);
    engine.insert_lane("a", constant(0.1, 2.0)).unwrap();

    engine.seek(-3.0);
    assert_close(engine.position(), 0.0);
    engine.seek(9.0);
    assert_close(engine.position(), 2.0);
    engine.seek(f64::INFINITY);
    assert_close(engine.position(), 2.0);

    engine.audition(0.5, 1.0).unwrap();
    engine.seek(1.5);
    assert_eq!(engine.state(), PlaybackState::Stopped);
    assert_close(engine.position(), 1.5);
}

#[test]
fn scrub_rate_seeks_move_live_audio_to_the_newest_position() {
    let (mut engine, mut output, _clock) = setup(1);
    let audio = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&audio)).unwrap();
    engine.play().unwrap();
    assert_eq!(output.render(SPAN), frames(&audio, 0..SPAN));

    for step in 0..10 {
        engine.seek(0.25 + f64::from(step) * 0.0625);
    }

    assert_eq!(engine.state(), PlaybackState::Playing);
    assert_close(engine.position(), 0.8125);
    let rendered = output.render(2 * SPAN);
    assert_eq!(rendered[..SPAN], frames(&audio, SPAN..2 * SPAN));
    assert_eq!(rendered[SPAN..], frames(&audio, 812..812 + SPAN));
}

fn level(output: &mut OfflineOutput) -> f32 {
    settle(output);
    let samples = output.render(SPAN);
    assert!(samples.windows(2).all(|pair| pair[0] == pair[1]));
    samples[0]
}

#[test]
fn mute_solo_and_volume_apply_to_live_playback() {
    let (mut engine, mut output, _clock) = setup(1);
    engine.insert_lane("a", constant(0.25, 2.0)).unwrap();
    engine.insert_lane("b", constant(0.5, 2.0)).unwrap();
    engine.play().unwrap();
    assert_eq!(level(&mut output), 0.75);

    engine.set_lane_muted("a", true);
    assert_eq!(engine.audible_lane_count(), 1);
    assert_eq!(level(&mut output), 0.5);

    engine.set_lane_solo("a", true);
    assert_eq!(engine.audible_lane_count(), 0);
    assert_eq!(level(&mut output), 0.0);

    engine.set_lane_muted("a", false);
    assert_eq!(level(&mut output), 0.25);

    engine.set_lane_volume("a", 2.0);
    assert_eq!(level(&mut output), 0.5);

    engine.set_lane_solo("a", false);
    assert_eq!(engine.audible_lane_count(), 2);
    assert_eq!(level(&mut output), 1.0);

    engine.set_lane_volume("b", 10.0);
    assert_eq!(engine.lane_mix("b").volume, studio_audio::MAX_LANE_VOLUME);
}

#[test]
fn mix_settings_made_before_loading_apply_when_audio_arrives() {
    let (mut engine, mut output, _clock) = setup(1);
    engine.set_lane_muted("b", true);
    engine.insert_lane("a", constant(0.25, 1.0)).unwrap();
    engine.insert_lane("b", constant(0.5, 1.0)).unwrap();

    assert_eq!(engine.play().unwrap(), 1);
    assert!(output.render(SPAN).iter().all(|sample| *sample == 0.25));
}

#[test]
fn loop_wraps_audio_and_playhead_and_reports_each_wrap() {
    let (mut engine, mut output, clock) = setup(1);
    let audio = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&audio)).unwrap();
    engine.set_loop(Some(LoopRange {
        start_seconds: 0.1,
        end_seconds: 0.2,
    }));

    engine.play_from(0.1).unwrap();
    let rendered = output.render(150);
    assert_eq!(rendered[..100], frames(&audio, 100..200));
    assert_eq!(rendered[100..], frames(&audio, 100..150));

    clock.advance(ms(150));
    assert!((engine.position() - 0.15).abs() < 1e-9);
    assert_eq!(engine.poll(), Some(PlaybackEvent::Looped));
    assert_eq!(engine.poll(), None);
    clock.advance(ms(100));
    assert_eq!(engine.poll(), Some(PlaybackEvent::Looped));
    assert_eq!(engine.state(), PlaybackState::Playing);
}

#[test]
fn enabling_a_loop_behind_the_playhead_jumps_to_the_loop_start() {
    let (mut engine, mut output, clock) = setup(1);
    let audio = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&audio)).unwrap();
    engine.play_from(0.5).unwrap();
    settle(&mut output);

    engine.set_loop(Some(LoopRange {
        start_seconds: 0.1,
        end_seconds: 0.2,
    }));
    assert_close(engine.position(), 0.1);
    settle(&mut output);
    assert_eq!(output.render(SPAN), frames(&audio, 100..100 + SPAN));

    engine.set_loop(None);
    assert_eq!(engine.loop_range(), None);
    clock.advance(ms(500));
    assert_close(engine.position(), 0.6);
}

#[test]
fn invalid_loop_ranges_disable_the_loop() {
    let (mut engine, _output, _clock) = setup(1);
    engine.set_loop(Some(LoopRange {
        start_seconds: 0.5,
        end_seconds: 0.2,
    }));
    assert_eq!(engine.loop_range(), None);
}

#[test]
fn lanes_with_different_shapes_play_together() {
    let (mut engine, mut output, _clock) = setup(2);
    engine.insert_lane("mono", constant(0.25, 1.0)).unwrap();
    let stereo =
        DecodedAudio::new(RATE, vec![vec![0.5; 1_000], vec![-0.5; 1_000]]).expect("stereo audio");
    engine.insert_lane("stereo", Arc::new(stereo)).unwrap();

    assert_eq!(engine.play().unwrap(), 2);

    let rendered = output.render(40);
    assert!(
        rendered
            .chunks(2)
            .all(|frame| frame[0] == 0.75 && frame[1] == -0.25),
        "{rendered:?}"
    );
}

#[test]
fn a_lane_loaded_during_playback_joins_at_the_playhead() {
    let (mut engine, mut output, clock) = setup(1);
    engine.insert_lane("a", constant(0.25, 0.5)).unwrap();
    engine.play().unwrap();
    output.render(100);
    clock.advance(ms(100));

    engine.insert_lane("b", constant(0.5, 2.0)).unwrap();

    assert_eq!(engine.state(), PlaybackState::Playing);
    assert_close(engine.position(), 0.1);
    assert_close(engine.duration(), 2.0);
    settle(&mut output);
    assert!(output.render(SPAN).iter().all(|sample| *sample == 0.75));
    assert_eq!(engine.lane_ids().collect::<Vec<_>>(), vec!["a", "b"]);
}

#[test]
fn unload_releases_decoded_audio_and_resets_the_transport() {
    let (mut engine, mut output, _clock) = setup(1);
    let audio = constant(0.5, 1.0);
    let samples = Arc::downgrade(&audio.channel_data_shared(0).expect("channel"));
    engine.insert_lane("a", audio).unwrap();
    engine.set_lane_muted("a", true);
    engine.set_speed(1.5);
    engine.set_loop(Some(LoopRange {
        start_seconds: 0.1,
        end_seconds: 0.5,
    }));
    engine.play_from(0.2).unwrap();
    output.render(SPAN);

    engine.unload();
    output.render(SPAN);

    assert!(
        samples.upgrade().is_none(),
        "decoded PCM must be released when leaving the editor"
    );
    assert_eq!(engine.lane_ids().count(), 0);
    assert!(engine.lane_audio("a").is_none());
    assert_eq!(engine.lane_mix("a"), studio_audio::LaneMix::default());
    assert_eq!(engine.state(), PlaybackState::Stopped);
    assert_close(engine.position(), 0.0);
    assert_close(engine.duration(), 0.0);
    assert_eq!(engine.loop_range(), None);
    assert_eq!(engine.speed(), 1.5);
}

#[test]
fn removing_a_lane_returns_its_audio_and_shrinks_the_timeline() {
    let (mut engine, _output, _clock) = setup(1);
    engine.insert_lane("a", constant(0.1, 1.0)).unwrap();
    engine.insert_lane("b", constant(0.1, 3.0)).unwrap();

    let removed = engine.remove_lane("b").expect("lane b loaded");

    assert_close(removed.duration(), 3.0);
    assert_close(engine.duration(), 1.0);
    assert!(engine.remove_lane("b").is_none());
}

#[cfg(windows)]
#[test]
fn engine_can_be_moved_to_another_thread_on_windows() {
    fn assert_send<T: Send>() {}
    assert_send::<AudioEngine>();
}

fn enhanced(lane: &str, audio: &Arc<DecodedAudio>) -> Vec<(String, Arc<DecodedAudio>)> {
    vec![(lane.to_string(), Arc::clone(audio))]
}

/// `original` at frame `f` is `f / 1000`; `enhanced` is the constant 0.5.
fn ramp_with_constant_enhancement(engine: &mut AudioEngine) -> Arc<DecodedAudio> {
    let original = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&original)).unwrap();
    engine
        .set_enhanced_audio(enhanced("a", &constant(0.5, 1.0)))
        .unwrap();
    original
}

fn blend(original: &[f32], enhanced: f32, ratio: f32) -> Vec<f32> {
    original
        .iter()
        .map(|sample| sample * (1.0 - ratio) + enhanced * ratio)
        .collect()
}

fn assert_samples_close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (frame, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() < 1e-6,
            "frame {frame}: expected {expected}, got {actual}"
        );
    }
}

#[test]
fn enhanced_ratio_blends_original_and_enhanced_audio() {
    let (mut engine, mut output, _clock) = setup(1);
    engine.insert_lane("a", constant(0.2, 1.0)).unwrap();
    engine
        .set_enhanced_audio(enhanced("a", &constant(0.6, 1.0)))
        .unwrap();
    engine.play().unwrap();

    // Ratio 0 (the default) is the untouched original.
    assert_eq!(level(&mut output), 0.2);
    for (ratio, expected) in [(0.1, 0.24), (0.5, 0.4), (0.9, 0.56), (1.0, 0.6)] {
        engine.set_lane_enhanced_ratio("a", ratio);
        assert!(
            (level(&mut output) - expected).abs() < 1e-6,
            "ratio {ratio}"
        );
    }

    engine.set_lane_enhanced_ratio("a", f32::NAN);
    assert_eq!(engine.lane_mix("a").enhanced_ratio, 1.0);
    engine.set_lane_enhanced_ratio("a", 7.0);
    assert_eq!(engine.lane_mix("a").enhanced_ratio, 1.0);
    engine.set_lane_enhanced_ratio("a", -1.0);
    assert_eq!(level(&mut output), 0.2);
}

#[test]
fn ratio_changes_apply_live_without_restarting_playback() {
    let (mut engine, mut output, _clock) = setup(1);
    let original = ramp_with_constant_enhancement(&mut engine);
    engine.play().unwrap();
    let pcm = frames(&original, 0..1_000);
    assert_eq!(output.render(SPAN), pcm[..SPAN]);

    // The clock has not moved: a restarted session would begin at frame 0 again.
    engine.set_lane_enhanced_ratio("a", 0.3);
    let rendered = output.render(2 * SPAN);

    assert_eq!(rendered[..SPAN], pcm[SPAN..2 * SPAN]);
    assert_samples_close(
        &rendered[SPAN..],
        &blend(&pcm[2 * SPAN..3 * SPAN], 0.5, 0.3),
    );
}

#[test]
fn installing_enhanced_audio_keeps_the_original_clock() {
    let (mut engine, mut output, clock) = setup(1);
    let original = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&original)).unwrap();
    engine.set_lane_enhanced_ratio("a", 0.5);
    engine.play().unwrap();
    output.render(300);
    clock.advance(ms(300));

    engine
        .set_enhanced_audio(enhanced("a", &constant(0.5, 1.0)))
        .unwrap();

    assert_eq!(engine.state(), PlaybackState::Playing);
    assert_close(engine.position(), 0.3);
    assert_close(engine.duration(), 1.0);
    let pcm = frames(&original, 300..300 + SPAN);
    assert_samples_close(&output.render(SPAN), &blend(&pcm, 0.5, 0.5));

    engine.clear_enhanced_audio();
    assert!(engine.lane_enhanced_audio("a").is_none());
    assert_close(engine.position(), 0.3);
    assert_eq!(output.render(SPAN), frames(&original, 300..300 + SPAN));
}

#[test]
fn seeks_and_loops_keep_original_and_enhanced_aligned() {
    let (mut engine, mut output, _clock) = setup(1);
    let original = ramp(1_000);
    engine.insert_lane("a", Arc::clone(&original)).unwrap();
    let enhanced_ramp = mono((0..1_000).map(|frame| -(frame as f64) / 1_000.0).collect());
    engine
        .set_enhanced_audio(enhanced("a", &enhanced_ramp))
        .unwrap();
    engine.set_lane_enhanced_ratio("a", 0.25);
    let expected = |range: std::ops::Range<usize>| {
        // 0.75 * f/1000 + 0.25 * -f/1000 = 0.5 * f/1000
        frames(&original, range)
            .iter()
            .map(|sample| sample * 0.5)
            .collect::<Vec<_>>()
    };

    engine.play_from(0.6).unwrap();
    assert_samples_close(&output.render(SPAN), &expected(600..600 + SPAN));
    engine.seek(0.2);
    settle(&mut output);
    assert_samples_close(&output.render(SPAN), &expected(200..200 + SPAN));

    engine.pause();
    engine.set_loop(Some(LoopRange {
        start_seconds: 0.1,
        end_seconds: 0.15,
    }));
    engine.play_from(0.1).unwrap();
    let rendered = output.render(100);
    assert_samples_close(&rendered[..50], &expected(100..150));
    assert_samples_close(&rendered[50..], &expected(100..150));
}

#[test]
fn muted_lanes_silence_enhanced_audio_and_lanes_without_it_stay_original() {
    let (mut engine, mut output, _clock) = setup(1);
    engine.insert_lane("a", constant(0.2, 1.0)).unwrap();
    engine.insert_lane("b", constant(0.1, 1.0)).unwrap();
    engine
        .set_enhanced_audio(enhanced("a", &constant(0.4, 1.0)))
        .unwrap();
    engine.set_lane_enhanced_ratio("a", 1.0);
    // Lane b has no enhanced audio: its ratio must not attenuate the original.
    engine.set_lane_enhanced_ratio("b", 1.0);
    engine.play().unwrap();
    assert!((level(&mut output) - 0.5).abs() < 1e-6);

    engine.set_lane_muted("a", true);
    assert_eq!(engine.audible_lane_count(), 1);
    assert!((level(&mut output) - 0.1).abs() < 1e-6);

    engine.set_lane_muted("a", false);
    engine.set_lane_solo("a", true);
    engine.set_lane_volume("a", 2.0);
    assert!((level(&mut output) - 0.8).abs() < 1e-6);
}

#[test]
fn enhanced_audio_must_match_its_original_and_installs_all_or_nothing() {
    let (mut engine, mut output, _clock) = setup(1);
    engine.insert_lane("a", constant(0.2, 1.0)).unwrap();
    engine.insert_lane("b", constant(0.1, 1.0)).unwrap();
    let valid = constant(0.4, 1.0);
    let short = constant(0.4, 0.5);
    let other_rate = Arc::new(DecodedAudio::new(2 * RATE, vec![vec![0.4; 2_000]]).unwrap());

    let attempts = [
        (short, "b"),
        (other_rate, "b"),
        (Arc::clone(&valid), "missing"),
    ];
    for (bad, lane) in attempts {
        let result = engine.set_enhanced_audio(vec![
            ("a".to_string(), Arc::clone(&valid)),
            (lane.to_string(), bad),
        ]);
        assert!(
            matches!(
                result,
                Err(AudioError::EnhancedAudioMismatch { .. } | AudioError::LaneNotLoaded(_))
            ),
            "{result:?}"
        );
        assert!(engine.lane_enhanced_audio("a").is_none());
    }

    engine.set_lane_enhanced_ratio("a", 1.0);
    engine.play().unwrap();
    assert!((level(&mut output) - 0.3).abs() < 1e-6);
}

#[test]
fn mono_enhanced_audio_feeds_every_channel_of_a_stereo_original() {
    let (mut engine, mut output, _clock) = setup(2);
    let stereo =
        DecodedAudio::new(RATE, vec![vec![0.5; 1_000], vec![-0.5; 1_000]]).expect("stereo audio");
    engine.insert_lane("a", Arc::new(stereo)).unwrap();
    engine
        .set_enhanced_audio(enhanced("a", &constant(0.25, 1.0)))
        .unwrap();
    engine.set_lane_enhanced_ratio("a", 0.5);
    engine.play().unwrap();

    let rendered = output.render(40);
    assert!(
        rendered
            .chunks(2)
            .all(|frame| frame[0] == 0.375 && frame[1] == -0.125),
        "{rendered:?}"
    );
}

#[test]
fn replacing_a_lane_original_drops_its_enhanced_audio() {
    let (mut engine, _output, _clock) = setup(1);
    engine.insert_lane("a", constant(0.2, 1.0)).unwrap();
    engine
        .set_enhanced_audio(enhanced("a", &constant(0.4, 1.0)))
        .unwrap();

    engine.insert_lane("a", constant(0.3, 1.0)).unwrap();

    assert!(engine.lane_enhanced_audio("a").is_none());
}

#[test]
fn replaced_enhanced_audio_is_released_after_the_audio_thread_drops_it() {
    let (mut engine, mut output, _clock) = setup(1);
    engine.insert_lane("a", constant(0.2, 1.0)).unwrap();
    let first = constant(0.4, 1.0);
    let samples = Arc::downgrade(&first.channel_data_shared(0).expect("channel"));
    engine.set_enhanced_audio(enhanced("a", &first)).unwrap();
    drop(first);
    engine.set_lane_enhanced_ratio("a", 1.0);
    engine.play().unwrap();
    output.render(SPAN);

    engine.clear_enhanced_audio();
    // The output callback drops the old source but must not free its PCM.
    output.render(SPAN);
    assert!(samples.upgrade().is_some());

    engine.poll();
    assert!(samples.upgrade().is_none());
}
