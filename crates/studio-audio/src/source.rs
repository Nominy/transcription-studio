//! Rodio source that mixes decoded lanes sharing one sample rate and channel
//! count. Each lane may carry enhanced PCM beside its original; the two are
//! blended per lane. Seek, loop, end bound and per-lane gains are read from a
//! shared [`SessionControl`] at control-span boundaries, so they apply to the
//! live source without tearing down players.

use std::{
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use rodio::{ChannelCount, Sample, SampleRate, Source};
use studio_waveform::DecodedAudio;

/// Controls (and speed changes made by rodio's player) take effect at span
/// boundaries; this bounds their latency.
const PLAYBACK_CONTROL_INTERVAL_MILLIS: usize = 20;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ControlState {
    /// `(request id, seconds)`; a source applies each id once.
    pub(crate) seek: Option<(u64, f64)>,
    pub(crate) end_seconds: f64,
    pub(crate) loop_range: Option<(f64, f64)>,
    /// Effective gains per lane slot (mute/solo/volume and the
    /// original/enhanced ratio already applied).
    pub(crate) gains: Vec<LaneGain>,
}

/// Gains applied to a lane's original and enhanced PCM.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LaneGain {
    pub(crate) original: f32,
    pub(crate) enhanced: f32,
}

impl LaneGain {
    /// Gains of an audible lane at `volume` playing `enhanced_ratio` of its enhanced PCM
    /// (`0` when it has none).
    pub(crate) fn blend(volume: f32, enhanced_ratio: f32) -> Self {
        Self {
            original: volume * (1.0 - enhanced_ratio),
            enhanced: volume * enhanced_ratio,
        }
    }

    pub(crate) fn is_audible(self) -> bool {
        self.original > 0.0 || self.enhanced > 0.0
    }

    /// Adds the lane's share of one output sample to `sum`: each present source sample,
    /// clamped to full scale, times its gain. Zero gains skip their source.
    #[inline]
    pub(crate) fn accumulate(self, sum: &mut f32, original: Option<f32>, enhanced: Option<f32>) {
        if self.original != 0.0
            && let Some(sample) = original
        {
            *sum += sample.clamp(-1.0, 1.0) * self.original;
        }
        if self.enhanced != 0.0
            && let Some(sample) = enhanced
        {
            *sum += sample.clamp(-1.0, 1.0) * self.enhanced;
        }
    }
}

/// One lane of a [`MixSource`] group: its control slot, original PCM and
/// optional enhanced PCM (same rate and frame count; mono or the original's
/// channel count).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LaneSources<'a> {
    pub(crate) slot: usize,
    pub(crate) original: &'a DecodedAudio,
    pub(crate) enhanced: Option<&'a DecodedAudio>,
}

/// Shared between the engine (writer) and the audio thread (reader).
#[derive(Debug)]
pub(crate) struct SessionControl {
    generation: AtomicU64,
    state: Mutex<ControlState>,
}

impl SessionControl {
    pub(crate) fn new(state: ControlState) -> Self {
        Self {
            generation: AtomicU64::new(0),
            state: Mutex::new(state),
        }
    }

    pub(crate) fn update(&self, change: impl FnOnce(&mut ControlState)) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        change(&mut state);
        drop(state);
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn seek(&self, seconds: f64) {
        self.update(|state| {
            let id = state.seek.map_or(1, |(id, _)| id + 1);
            state.seek = Some((id, seconds));
        });
    }
}

pub(crate) fn playback_control_span_samples(
    samples_remaining: usize,
    sample_rate: u32,
    channel_count: usize,
) -> usize {
    if samples_remaining == 0 {
        return 0;
    }
    let frames_per_span = (sample_rate as usize)
        .saturating_mul(PLAYBACK_CONTROL_INTERVAL_MILLIS)
        .div_ceil(1_000)
        .max(1);
    let samples_per_span = frames_per_span
        .saturating_mul(channel_count)
        .max(channel_count);
    samples_remaining.min(samples_per_span)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameBound {
    Start,
    End,
}

fn frame_for_seconds(seconds: f64, sample_rate: u32, bound: FrameBound) -> usize {
    if !seconds.is_finite() {
        return if seconds > 0.0 { usize::MAX } else { 0 };
    }
    let frame = seconds.max(0.0) * f64::from(sample_rate);
    let frame = match bound {
        FrameBound::Start => frame.floor(),
        FrameBound::End => frame.ceil(),
    };
    frame as usize
}

#[derive(Debug)]
struct MixLane {
    original: Vec<Arc<[f32]>>,
    /// One buffer per output channel; mono enhanced PCM is shared by all.
    enhanced: Option<Vec<Arc<[f32]>>>,
    slot: usize,
    gain: LaneGain,
}

fn shared_channels(audio: &DecodedAudio, channel_count: usize) -> Option<Vec<Arc<[f32]>>> {
    let source_channels = audio.number_of_channels();
    if source_channels != channel_count && source_channels != 1 {
        return None;
    }
    (0..channel_count)
        .map(|index| audio.channel_data_shared(if source_channels == 1 { 0 } else { index }))
        .collect()
}

#[derive(Debug)]
pub(crate) struct MixSource {
    lanes: Vec<MixLane>,
    channel_count: usize,
    sample_rate: u32,
    next_frame: usize,
    next_channel: usize,
    end_frame: usize,
    loop_frames: Option<(usize, usize)>,
    span_remaining: usize,
    control: Arc<SessionControl>,
    seen_generation: Option<u64>,
    seen_seek: u64,
}

impl MixSource {
    /// Lane originals must share sample rate and channel count
    /// (1..=u16::MAX channels); enhanced PCM must match its original's rate
    /// and frame count and be mono or have the original's channel count.
    /// Returns `None` for an empty or mismatched group.
    pub(crate) fn new(
        lanes: &[LaneSources<'_>],
        start_seconds: f64,
        control: Arc<SessionControl>,
    ) -> Option<Self> {
        let first = lanes.first()?.original;
        let channel_count = first.number_of_channels();
        let sample_rate = first.sample_rate();
        if channel_count == 0
            || channel_count > usize::from(u16::MAX)
            || lanes.iter().any(|lane| {
                lane.original.number_of_channels() != channel_count
                    || lane.original.sample_rate() != sample_rate
                    || lane.enhanced.is_some_and(|enhanced| {
                        enhanced.sample_rate() != sample_rate
                            || enhanced.frame_count() != lane.original.frame_count()
                    })
            })
        {
            return None;
        }
        let lanes = lanes
            .iter()
            .map(|lane| {
                Some(MixLane {
                    original: shared_channels(lane.original, channel_count)?,
                    enhanced: match lane.enhanced {
                        Some(enhanced) => Some(shared_channels(enhanced, channel_count)?),
                        None => None,
                    },
                    slot: lane.slot,
                    gain: LaneGain::default(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let mut source = Self {
            lanes,
            channel_count,
            sample_rate,
            next_frame: frame_for_seconds(start_seconds, sample_rate, FrameBound::Start),
            next_channel: 0,
            end_frame: 0,
            loop_frames: None,
            span_remaining: 0,
            control,
            seen_generation: None,
            seen_seek: 0,
        };
        source.begin_span();
        Some(source)
    }

    fn refresh_controls(&mut self) {
        let generation = self.control.generation.load(Ordering::Acquire);
        if self.seen_generation == Some(generation) {
            return;
        }
        let state = match self.control.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            // The engine is mid-update; pick the change up next span.
            Err(TryLockError::WouldBlock) => return,
        };
        let first_refresh = self.seen_generation.is_none();
        self.seen_generation = Some(generation);
        if let Some((id, seconds)) = state.seek
            && id != self.seen_seek
        {
            self.seen_seek = id;
            // A seek recorded before this source existed is already reflected
            // in its start position.
            if !first_refresh {
                self.next_frame = frame_for_seconds(seconds, self.sample_rate, FrameBound::Start);
            }
        }
        self.end_frame = frame_for_seconds(state.end_seconds, self.sample_rate, FrameBound::End);
        self.loop_frames = state.loop_range.map(|(start, end)| {
            (
                frame_for_seconds(start, self.sample_rate, FrameBound::Start),
                frame_for_seconds(end, self.sample_rate, FrameBound::End),
            )
        });
        for lane in &mut self.lanes {
            lane.gain = state.gains.get(lane.slot).copied().unwrap_or_default();
        }
    }

    /// Called at frame-aligned span boundaries.
    fn begin_span(&mut self) {
        self.refresh_controls();
        if let Some((loop_start, loop_end)) = self.loop_frames
            && loop_start < loop_end
        {
            if self.next_frame >= loop_end {
                self.next_frame = loop_start;
            }
            // Looping never reaches the end bound.
            self.span_remaining =
                playback_control_span_samples(usize::MAX, self.sample_rate, self.channel_count);
            return;
        }
        let frames_remaining = self.end_frame.saturating_sub(self.next_frame);
        self.span_remaining = playback_control_span_samples(
            frames_remaining.saturating_mul(self.channel_count),
            self.sample_rate,
            self.channel_count,
        );
    }
}

impl Iterator for MixSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.span_remaining == 0 {
            return None;
        }
        if self.next_channel == 0
            && let Some((loop_start, loop_end)) = self.loop_frames
            && loop_start < loop_end
            && self.next_frame >= loop_end
        {
            self.next_frame = loop_start;
        }

        let frame = self.next_frame;
        let channel = self.next_channel;
        let mut sum = 0.0_f32;
        for lane in &self.lanes {
            lane.gain.accumulate(
                &mut sum,
                lane.original[channel].get(frame).copied(),
                lane.enhanced
                    .as_ref()
                    .and_then(|enhanced| enhanced[channel].get(frame).copied()),
            );
        }

        self.next_channel += 1;
        if self.next_channel == self.channel_count {
            self.next_channel = 0;
            self.next_frame += 1;
        }
        self.span_remaining -= 1;
        if self.span_remaining == 0 {
            self.begin_span();
        }
        Some(sum as Sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.span_remaining, None)
    }
}

impl Source for MixSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.span_remaining)
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channel_count as u16).expect("channel count is validated")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.sample_rate).expect("decoded audio sample rate is non-zero")
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use rodio::Player;

    use super::*;

    fn audio(sample_rate: u32, channels: Vec<Vec<f64>>) -> DecodedAudio {
        DecodedAudio::new(sample_rate, channels).expect("valid decoded audio")
    }

    fn gain(original: f32, enhanced: f32) -> LaneGain {
        LaneGain { original, enhanced }
    }

    fn control(end_seconds: f64, lanes: usize) -> Arc<SessionControl> {
        Arc::new(SessionControl::new(ControlState {
            seek: None,
            end_seconds,
            loop_range: None,
            gains: vec![gain(1.0, 0.0); lanes],
        }))
    }

    fn originals<'a>(lanes: &[&'a DecodedAudio]) -> Vec<LaneSources<'a>> {
        lanes
            .iter()
            .enumerate()
            .map(|(slot, audio)| LaneSources {
                slot,
                original: audio,
                enhanced: None,
            })
            .collect()
    }

    fn source(lanes: &[&DecodedAudio], start: f64, control: Arc<SessionControl>) -> MixSource {
        MixSource::new(&originals(lanes), start, control).expect("compatible lanes")
    }

    #[test]
    fn interleaves_decoded_channels_from_start_time() {
        let audio = audio(
            2,
            vec![vec![0.1, 0.2, 0.3, 0.4], vec![-0.1, -0.2, -0.3, -0.4]],
        );

        let samples = source(&[&audio], 0.5, control(2.0, 1)).collect::<Vec<_>>();

        assert_eq!(samples, vec![0.2, -0.2, 0.3, -0.3, 0.4, -0.4]);
    }

    #[test]
    fn clamps_samples_and_is_empty_past_the_end() {
        let audio = audio(1, vec![vec![1.5, -2.0]]);

        assert_eq!(
            source(&[&audio], 0.0, control(2.0, 1)).collect::<Vec<_>>(),
            vec![1.0, -1.0]
        );
        assert_eq!(source(&[&audio], 10.0, control(2.0, 1)).next(), None);
    }

    #[test]
    fn stops_at_the_end_bound() {
        let audio = audio(4, vec![vec![0.1, 0.2, 0.3, 0.4]]);

        assert_eq!(
            source(&[&audio], 0.25, control(0.75, 1)).collect::<Vec<_>>(),
            vec![0.2, 0.3]
        );
    }

    #[test]
    fn sums_lanes_and_pads_shorter_lanes_with_silence() {
        let first = audio(2, vec![vec![0.1, 0.2, 0.3]]);
        let second = audio(2, vec![vec![0.4, -0.7]]);

        assert_eq!(
            source(&[&first, &second], 0.0, control(2.0, 2)).collect::<Vec<_>>(),
            vec![0.5, -0.5, 0.3, 0.0]
        );
    }

    #[test]
    fn rejects_mismatched_shapes() {
        let mono = audio(2, vec![vec![0.1, 0.2]]);
        let stereo = audio(2, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);
        let other_rate = audio(3, vec![vec![0.1, 0.2]]);

        assert!(MixSource::new(&originals(&[&mono, &stereo]), 0.0, control(1.0, 2)).is_none());
        assert!(MixSource::new(&originals(&[&mono, &other_rate]), 0.0, control(1.0, 2)).is_none());
        assert!(MixSource::new(&[], 0.0, control(1.0, 0)).is_none());

        let with_enhanced = |original, enhanced| {
            MixSource::new(
                &[LaneSources {
                    slot: 0,
                    original,
                    enhanced: Some(enhanced),
                }],
                0.0,
                control(1.0, 1),
            )
        };
        let short = audio(2, vec![vec![0.1]]);
        let three_channels = audio(2, vec![vec![0.1, 0.2], vec![0.1, 0.2], vec![0.1, 0.2]]);
        assert!(with_enhanced(&mono, &short).is_none());
        assert!(with_enhanced(&mono, &other_rate).is_none());
        assert!(with_enhanced(&stereo, &three_channels).is_none());
        assert!(with_enhanced(&stereo, &mono).is_some());
    }

    #[test]
    fn blends_original_and_enhanced_with_their_gains() {
        let original = audio(2, vec![vec![0.5, -0.5], vec![0.25, 0.25]]);
        let enhanced = audio(2, vec![vec![0.1, 1.5]]);
        let control = Arc::new(SessionControl::new(ControlState {
            seek: None,
            end_seconds: 1.0,
            loop_range: None,
            gains: vec![gain(0.5, 0.5)],
        }));
        let lanes = [LaneSources {
            slot: 0,
            original: &original,
            enhanced: Some(&enhanced),
        }];

        let samples = MixSource::new(&lanes, 0.0, control)
            .expect("compatible lanes")
            .collect::<Vec<_>>();

        // Mono enhanced PCM feeds every channel; enhanced samples clamp too.
        assert_eq!(samples, vec![0.3, 0.175, 0.25, 0.625]);
    }

    #[test]
    fn gains_apply_per_lane_at_the_next_span() {
        let first = audio(1_000, vec![vec![0.5; 100]]);
        let second = audio(1_000, vec![vec![0.25; 100]]);
        let control = control(0.1, 2);
        let mut source = source(&[&first, &second], 0.0, Arc::clone(&control));
        let span = source.current_span_len().expect("finite span");
        assert_eq!(span, 20);

        assert_eq!(source.next(), Some(0.75));
        control.update(|state| state.gains = vec![gain(0.0, 0.0), gain(2.0, 0.0)]);
        let rest_of_span = source.by_ref().take(span - 1).collect::<Vec<_>>();
        assert!(rest_of_span.iter().all(|sample| *sample == 0.75));
        assert_eq!(source.next(), Some(0.5));
    }

    #[test]
    fn seek_jumps_at_the_next_span_boundary() {
        let samples = (0..100).map(|frame| f64::from(frame) / 100.0).collect();
        let audio = audio(1_000, vec![samples]);
        let control = control(0.1, 1);
        let mut source = source(&[&audio], 0.0, Arc::clone(&control));

        control.seek(0.05);
        let first_span = source.by_ref().take(20).collect::<Vec<_>>();
        assert_eq!(first_span.last().copied(), Some(0.19));
        assert_eq!(source.next(), Some(0.5));
        assert_eq!(source.by_ref().count(), 49);
    }

    #[test]
    fn loop_wraps_without_ending_and_unlooping_resumes_toward_the_end() {
        // Sample value = frame / 10, so `frame_of` recovers the frame index.
        let samples = (0..10).map(|frame| f64::from(frame) / 10.0).collect();
        let frame_of = |sample: f32| (f64::from(sample) * 10.0).round();
        let audio = audio(10, vec![samples]);
        let control = control(1.0, 1);
        control.update(|state| state.loop_range = Some((0.2, 0.5)));
        let mut source = source(&[&audio], 0.0, Arc::clone(&control));

        // Ten frames per second, so one-frame spans: changes apply per frame.
        let looped = source.by_ref().take(8).map(frame_of).collect::<Vec<_>>();
        assert_eq!(looped, vec![0.0, 1.0, 2.0, 3.0, 4.0, 2.0, 3.0, 4.0]);

        control.update(|state| state.loop_range = None);
        let rest = source.map(frame_of).collect::<Vec<_>>();
        assert_eq!(rest, vec![2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    }

    #[test]
    fn control_spans_are_finite_and_frame_aligned() {
        for sample_rate in [1, 8_000, 48_000, 192_000] {
            for channel_count in [1, 2, 6] {
                let samples_remaining = 1_001 * channel_count;
                let span =
                    playback_control_span_samples(samples_remaining, sample_rate, channel_count);
                assert!(span > 0);
                assert!(span <= samples_remaining);
                assert_eq!(span % channel_count, 0);
            }
        }
        assert_eq!(playback_control_span_samples(0, 48_000, 2), 0);
    }

    const RATE_TEST_SAMPLE_RATE: u32 = 1_000;
    const RATE_TEST_FRAMES: usize = 4_000;
    const RATE_CHANGE_AFTER_SAMPLES: usize = 1_000;
    const MAX_RENDERED_SAMPLES: usize = 10_000;

    fn rate_test_source(lane_count: usize) -> MixSource {
        let lanes = (0..lane_count)
            .map(|_| audio(RATE_TEST_SAMPLE_RATE, vec![vec![0.25; RATE_TEST_FRAMES]]))
            .collect::<Vec<_>>();
        let refs = lanes.iter().collect::<Vec<_>>();
        source(
            &refs,
            0.0,
            control(
                RATE_TEST_FRAMES as f64 / f64::from(RATE_TEST_SAMPLE_RATE),
                lane_count,
            ),
        )
    }

    fn rendered_samples(source: MixSource, initial_speed: f32, next_speed: Option<f32>) -> usize {
        let (mixer, mut output) = rodio::mixer::mixer(
            ChannelCount::new(1).expect("one channel"),
            SampleRate::new(RATE_TEST_SAMPLE_RATE).expect("positive sample rate"),
        );
        let player = Player::connect_new(&mixer);
        player.set_speed(initial_speed);
        player.append(source);

        let mut rendered = 0;
        while !player.empty() && rendered < MAX_RENDERED_SAMPLES {
            if rendered == RATE_CHANGE_AFTER_SAMPLES
                && let Some(speed) = next_speed
            {
                player.set_speed(speed);
            }
            assert!(output.next().is_some());
            rendered += 1;
        }
        assert!(player.empty(), "playback source should finish");
        rendered
    }

    fn assert_speed_resamples(lane_count: usize, change_while_playing: bool) {
        let run = |speed: f32| {
            if change_while_playing {
                rendered_samples(rate_test_source(lane_count), 1.0, Some(speed))
            } else {
                rendered_samples(rate_test_source(lane_count), speed, None)
            }
        };
        let normal = run(1.0);
        let fast = run(2.0);
        let slow = run(0.5);

        assert!(
            fast < normal * 3 / 4,
            "2x playback should render materially fewer samples: normal={normal}, fast={fast}"
        );
        assert!(
            slow > normal * 3 / 2,
            "0.5x playback should render materially more samples: normal={normal}, slow={slow}"
        );
    }

    #[test]
    fn initial_player_speed_resamples_single_and_mixed_lanes() {
        assert_speed_resamples(1, false);
        assert_speed_resamples(2, false);
    }

    #[test]
    fn active_player_speed_changes_resample_single_and_mixed_lanes() {
        assert_speed_resamples(1, true);
        assert_speed_resamples(2, true);
    }
}
