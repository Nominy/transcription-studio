use std::{sync::Arc, time::Duration};

#[cfg(not(test))]
use std::time::Instant;

use rodio::{ChannelCount, Sample, SampleRate, Source};
use studio_waveform::DecodedAudio;

#[cfg(not(test))]
const AUDIO_STARTUP_TRACE_ENV: &str = "STUDIO_TRACE_AUDIO_STARTUP";
const AUDIO_PREWARMED_PLAYER_POOL_SIZE: usize = 4;
const AUDIO_PLAYBACK_CONTROL_INTERVAL_MILLIS: usize = 20;

fn playback_control_span_samples(
    samples_remaining: usize,
    sample_rate: u32,
    channel_count: usize,
) -> usize {
    if samples_remaining == 0 {
        return 0;
    }

    let frames_per_span = (sample_rate as usize)
        .saturating_mul(AUDIO_PLAYBACK_CONTROL_INTERVAL_MILLIS)
        .div_ceil(1_000)
        .max(1);
    let samples_per_span = frames_per_span
        .saturating_mul(channel_count)
        .max(channel_count);
    samples_remaining.min(samples_per_span)
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlaybackTrack {
    pub(super) lane_id: String,
    audio: Arc<DecodedAudio>,
}

impl PlaybackTrack {
    pub(super) fn new(lane_id: impl Into<String>, audio: Arc<DecodedAudio>) -> Self {
        Self {
            lane_id: lane_id.into(),
            audio,
        }
    }

    fn source_until(
        &self,
        start_seconds: f64,
        end_seconds: Option<f64>,
    ) -> Result<DecodedAudioPlaybackSource, String> {
        DecodedAudioPlaybackSource::new(Arc::clone(&self.audio), start_seconds, end_seconds)
    }
}

#[derive(Clone, Debug)]
struct DecodedAudioPlaybackSource {
    channels: Vec<Arc<[f32]>>,
    next_frame: usize,
    end_frame: usize,
    next_channel: usize,
    channel_count: usize,
    sample_rate: u32,
    duration: Duration,
}

impl DecodedAudioPlaybackSource {
    fn new(
        audio: Arc<DecodedAudio>,
        start_seconds: f64,
        end_seconds: Option<f64>,
    ) -> Result<Self, String> {
        let channel_count = audio.number_of_channels();
        if channel_count == 0 || channel_count > u16::MAX as usize {
            return Err(format!("unsupported channel count: {channel_count}"));
        }

        let sample_rate = audio.sample_rate();
        let start_frame = frame_index_for_seconds(&audio, start_seconds, FrameBound::Start);
        let end_frame = end_seconds
            .map(|seconds| frame_index_for_seconds(&audio, seconds, FrameBound::End))
            .unwrap_or(audio.frame_count())
            .max(start_frame);
        let frame_count = end_frame.saturating_sub(start_frame);
        let duration = Duration::from_secs_f64(frame_count as f64 / sample_rate as f64);
        let channels = (0..channel_count)
            .map(|channel_index| {
                audio
                    .channel_data_shared(channel_index)
                    .ok_or_else(|| format!("missing decoded audio channel: {channel_index}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            channels,
            next_frame: start_frame,
            end_frame,
            next_channel: 0,
            channel_count,
            sample_rate,
            duration,
        })
    }

    fn is_empty(&self) -> bool {
        self.next_frame >= self.end_frame
    }
}

impl Iterator for DecodedAudioPlaybackSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_frame >= self.end_frame {
            return None;
        }

        let sample = self
            .channels
            .get(self.next_channel)
            .and_then(|channel| channel.get(self.next_frame))
            .copied()
            .unwrap_or_default()
            .clamp(-1.0, 1.0) as Sample;

        self.next_channel += 1;
        if self.next_channel >= self.channel_count {
            self.next_channel = 0;
            self.next_frame += 1;
        }

        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let frames_remaining = self.end_frame.saturating_sub(self.next_frame);
        let samples_remaining = frames_remaining
            .saturating_mul(self.channel_count)
            .saturating_sub(self.next_channel);
        (samples_remaining, Some(samples_remaining))
    }
}

impl Source for DecodedAudioPlaybackSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(playback_control_span_samples(
            self.size_hint().0,
            self.sample_rate,
            self.channel_count,
        ))
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channel_count as u16).expect("channel count is validated")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.sample_rate).expect("decoded audio sample rate is validated")
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

#[derive(Clone, Debug)]
struct MixedDecodedAudioPlaybackSource {
    sources: Vec<Option<DecodedAudioPlaybackSource>>,
    active_source_count: usize,
    channel_count: usize,
    sample_rate: u32,
    duration: Duration,
}

impl MixedDecodedAudioPlaybackSource {
    fn new(sources: Vec<DecodedAudioPlaybackSource>) -> Option<Self> {
        let first = sources.first()?;
        let channel_count = first.channel_count;
        let sample_rate = first.sample_rate;
        if sources.iter().any(|source| {
            source.channel_count != channel_count || source.sample_rate != sample_rate
        }) {
            return None;
        }

        let duration = sources
            .iter()
            .map(|source| source.duration)
            .max()
            .unwrap_or(Duration::ZERO);
        let active_source_count = sources.len();
        Some(Self {
            sources: sources.into_iter().map(Some).collect(),
            active_source_count,
            channel_count,
            sample_rate,
            duration,
        })
    }
}

impl Iterator for MixedDecodedAudioPlaybackSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.active_source_count == 0 {
            return None;
        }

        let mut sum = 0.0;
        let mut emitted = false;
        for source in &mut self.sources {
            let Some(active_source) = source.as_mut() else {
                continue;
            };
            match active_source.next() {
                Some(sample) => {
                    sum += sample;
                    emitted = true;
                }
                None => {
                    *source = None;
                    self.active_source_count = self.active_source_count.saturating_sub(1);
                }
            }
        }

        emitted.then_some(sum)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let samples_remaining = self
            .sources
            .iter()
            .filter_map(|source| source.as_ref())
            .map(|source| source.size_hint().0)
            .max()
            .unwrap_or(0);
        (samples_remaining, Some(samples_remaining))
    }
}

impl Source for MixedDecodedAudioPlaybackSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(playback_control_span_samples(
            self.size_hint().0,
            self.sample_rate,
            self.channel_count,
        ))
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channel_count as u16).expect("channel count is validated")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.sample_rate).expect("decoded audio sample rate is validated")
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

#[cfg(not(test))]
fn playable_sources_for_tracks(
    tracks: Vec<PlaybackTrack>,
    start_seconds: f64,
    end_seconds: Option<f64>,
) -> Result<Vec<DecodedAudioPlaybackSource>, String> {
    let mut sources = Vec::with_capacity(tracks.len());
    for track in tracks {
        let source = track.source_until(start_seconds, end_seconds)?;
        if !source.is_empty() {
            sources.push(source);
        }
    }
    Ok(sources)
}

#[cfg(not(test))]
fn mixed_source_for_tracks(
    sources: &[DecodedAudioPlaybackSource],
) -> Option<MixedDecodedAudioPlaybackSource> {
    MixedDecodedAudioPlaybackSource::new(sources.to_vec())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameBound {
    Start,
    End,
}

fn frame_index_for_seconds(audio: &DecodedAudio, seconds: f64, bound: FrameBound) -> usize {
    if !seconds.is_finite() {
        return 0;
    }

    let frame = seconds.max(0.0) * audio.sample_rate() as f64;
    let frame = match bound {
        FrameBound::Start => frame.floor(),
        FrameBound::End => frame.ceil(),
    };
    (frame as usize).min(audio.frame_count())
}

#[cfg(not(test))]
#[derive(Default)]
pub(super) struct AudioPlaybackBackend {
    sink: Option<rodio::MixerDeviceSink>,
    players: Vec<rodio::Player>,
    idle_players: Vec<rodio::Player>,
    draining_players: Vec<rodio::Player>,
}

#[cfg(not(test))]
impl AudioPlaybackBackend {
    pub(super) fn prepare(&mut self) -> Result<(), String> {
        let sink_start = Instant::now();
        let mut result = self.ensure_sink();
        let sink_elapsed = sink_start.elapsed();
        let mut player_pool_elapsed = Duration::ZERO;
        if result.is_ok() {
            let player_pool_start = Instant::now();
            result = self.ensure_player_pool(AUDIO_PREWARMED_PLAYER_POOL_SIZE);
            player_pool_elapsed = player_pool_start.elapsed();
        }
        if audio_startup_trace_enabled() {
            eprintln!(
                "{}",
                audio_backend_prepare_runtime_trace_line(
                    result.is_ok(),
                    sink_elapsed,
                    player_pool_elapsed,
                )
            );
        }
        result
    }

    pub(super) fn play_tracks(
        &mut self,
        tracks: Vec<PlaybackTrack>,
        start_seconds: f64,
        speed: f32,
    ) -> Result<usize, String> {
        self.play_tracks_until(tracks, start_seconds, None, speed)
    }

    pub(super) fn play_tracks_until(
        &mut self,
        tracks: Vec<PlaybackTrack>,
        start_seconds: f64,
        end_seconds: Option<f64>,
        speed: f32,
    ) -> Result<usize, String> {
        let trace_enabled = audio_startup_trace_enabled();
        let requested_track_count = tracks.len();
        let stop_start = Instant::now();
        self.stop();
        let stop_elapsed = stop_start.elapsed();
        if tracks.is_empty() {
            if trace_enabled {
                eprintln!(
                    "{}",
                    audio_backend_startup_runtime_trace_line(
                        requested_track_count,
                        stop_elapsed,
                        Duration::ZERO,
                        Duration::ZERO,
                        Duration::ZERO,
                        0,
                    )
                );
            }
            return Ok(0);
        }
        let sources = playable_sources_for_tracks(tracks, start_seconds, end_seconds)?;
        let playable_track_count = sources.len();
        if playable_track_count == 0 {
            if trace_enabled {
                eprintln!(
                    "{}",
                    audio_backend_startup_runtime_trace_line(
                        requested_track_count,
                        stop_elapsed,
                        Duration::ZERO,
                        Duration::ZERO,
                        Duration::ZERO,
                        0,
                    )
                );
            }
            return Ok(0);
        }
        let sink_start = Instant::now();
        let sink_result = self.ensure_sink();
        let sink_elapsed = sink_start.elapsed();
        if let Err(error) = sink_result {
            if trace_enabled {
                eprintln!(
                    "{}",
                    audio_backend_startup_runtime_trace_line(
                        requested_track_count,
                        stop_elapsed,
                        sink_elapsed,
                        Duration::ZERO,
                        Duration::ZERO,
                        0,
                    )
                );
            }
            return Err(error);
        }
        let player_pool_start = Instant::now();
        let player_pool_result = if mixed_source_for_tracks(&sources).is_some() {
            self.ensure_player_pool(1)
        } else {
            self.ensure_player_pool(playable_track_count)
        };
        let player_pool_elapsed = player_pool_start.elapsed();
        if let Err(error) = player_pool_result {
            if trace_enabled {
                eprintln!(
                    "{}",
                    audio_backend_startup_runtime_trace_line(
                        requested_track_count,
                        stop_elapsed,
                        sink_elapsed,
                        player_pool_elapsed,
                        Duration::ZERO,
                        0,
                    )
                );
            }
            return Err(error);
        }

        let append_start = Instant::now();
        let appended_track_count = if let Some(source) = mixed_source_for_tracks(&sources) {
            self.append_source(source, speed);
            playable_track_count
        } else {
            self.append_individual_sources(sources, speed)
        };
        let append_elapsed = append_start.elapsed();
        if trace_enabled {
            eprintln!(
                "{}",
                audio_backend_startup_runtime_trace_line(
                    requested_track_count,
                    stop_elapsed,
                    sink_elapsed,
                    player_pool_elapsed,
                    append_elapsed,
                    appended_track_count,
                )
            );
        }

        Ok(appended_track_count)
    }

    fn append_individual_sources(
        &mut self,
        sources: Vec<DecodedAudioPlaybackSource>,
        speed: f32,
    ) -> usize {
        let source_count = sources.len();
        for source in sources {
            self.append_source(source, speed);
        }
        source_count
    }

    fn append_source(&mut self, source: impl Source + Send + 'static, speed: f32) {
        let player = self
            .idle_players
            .pop()
            .expect("player pool is prepared for requested tracks");
        player.set_speed(speed.max(0.1));
        player.play();
        player.append(source);
        self.players.push(player);
    }

    pub(super) fn stop(&mut self) {
        for player in self.players.drain(..) {
            player.stop();
            self.draining_players.push(player);
        }
        self.reclaim_drained_players();
        self.trim_idle_player_pool();
    }

    pub(super) fn set_speed(&mut self, speed: f32) {
        for player in &self.players {
            player.set_speed(speed.max(0.1));
        }
    }

    fn ensure_sink(&mut self) -> Result<(), String> {
        if self.sink.is_none() {
            self.sink = Some(
                rodio::DeviceSinkBuilder::open_default_sink()
                    .map_err(|error| format!("audio output unavailable: {error}"))?,
            );
        }

        Ok(())
    }

    fn ensure_player_pool(&mut self, min_idle_count: usize) -> Result<(), String> {
        self.ensure_sink()?;
        self.reclaim_drained_players();

        while self.idle_players.len() < min_idle_count {
            self.idle_players.push(self.connect_player());
        }

        Ok(())
    }

    fn connect_player(&self) -> rodio::Player {
        let sink = self
            .sink
            .as_ref()
            .expect("sink is initialized before connecting players");
        rodio::Player::connect_new(sink.mixer())
    }

    fn reclaim_drained_players(&mut self) {
        let mut index = 0;
        while index < self.draining_players.len() {
            if self.draining_players[index].len() == 0 {
                let player = self.draining_players.swap_remove(index);
                self.idle_players.push(player);
            } else {
                index += 1;
            }
        }
    }

    fn trim_idle_player_pool(&mut self) {
        while self.idle_players.len() > AUDIO_PREWARMED_PLAYER_POOL_SIZE {
            let _ = self.idle_players.pop();
        }
    }
}

#[cfg(not(test))]
fn audio_startup_trace_enabled() -> bool {
    std::env::var_os(AUDIO_STARTUP_TRACE_ENV).is_some_and(|value| value != "0")
}

fn audio_backend_startup_runtime_trace_line(
    requested_track_count: usize,
    stop_elapsed: Duration,
    sink_elapsed: Duration,
    player_pool_elapsed: Duration,
    append_elapsed: Duration,
    appended_track_count: usize,
) -> String {
    format!(
        "Transcription Studio audio backend startup: tracks={} appended={} stop_ms={:.3} sink_ms={:.3} player_pool_ms={:.3} append_ms={:.3}",
        requested_track_count,
        appended_track_count,
        stop_elapsed.as_secs_f64() * 1_000.0,
        sink_elapsed.as_secs_f64() * 1_000.0,
        player_pool_elapsed.as_secs_f64() * 1_000.0,
        append_elapsed.as_secs_f64() * 1_000.0
    )
}

fn audio_backend_prepare_runtime_trace_line(
    succeeded: bool,
    sink_elapsed: Duration,
    player_pool_elapsed: Duration,
) -> String {
    let outcome = if succeeded { "ready" } else { "error" };
    format!(
        "Transcription Studio audio backend prepare: outcome={} sink_ms={:.3} player_pool_ms={:.3}",
        outcome,
        sink_elapsed.as_secs_f64() * 1_000.0,
        player_pool_elapsed.as_secs_f64() * 1_000.0
    )
}

#[cfg(test)]
#[derive(Default, Debug)]
pub(super) struct AudioPlaybackBackend {
    active: bool,
    track_count: usize,
    prepare_count: usize,
    prepared_player_count: usize,
    pub(super) last_start_seconds: f64,
    pub(super) last_end_seconds: Option<f64>,
    pub(super) last_speed: f32,
}

#[cfg(test)]
impl AudioPlaybackBackend {
    pub(super) fn prepare(&mut self) -> Result<(), String> {
        self.prepare_count += 1;
        self.prepared_player_count = self
            .prepared_player_count
            .max(AUDIO_PREWARMED_PLAYER_POOL_SIZE);
        Ok(())
    }

    pub(super) fn play_tracks(
        &mut self,
        tracks: Vec<PlaybackTrack>,
        start_seconds: f64,
        speed: f32,
    ) -> Result<usize, String> {
        self.play_tracks_until(tracks, start_seconds, None, speed)
    }

    pub(super) fn play_tracks_until(
        &mut self,
        tracks: Vec<PlaybackTrack>,
        start_seconds: f64,
        end_seconds: Option<f64>,
        speed: f32,
    ) -> Result<usize, String> {
        self.track_count = tracks
            .iter()
            .filter(|track| {
                track
                    .source_until(start_seconds, end_seconds)
                    .is_ok_and(|source| !source.is_empty())
            })
            .count();
        self.active = self.track_count > 0;
        self.last_start_seconds = start_seconds;
        self.last_end_seconds = end_seconds;
        self.last_speed = speed;
        Ok(self.track_count)
    }

    pub(super) fn stop(&mut self) {
        self.active = false;
        self.track_count = 0;
    }

    pub(super) fn set_speed(&mut self, speed: f32) {
        self.last_speed = speed;
    }

    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn track_count(&self) -> usize {
        self.track_count
    }

    pub(super) fn prepare_count(&self) -> usize {
        self.prepare_count
    }

    pub(super) fn prepared_player_count(&self) -> usize {
        self.prepared_player_count
    }
}

#[cfg(test)]
mod tests {
    use rodio::{ChannelCount, Player, SampleRate, Source};
    use std::{sync::Arc, time::Duration};

    use studio_waveform::DecodedAudio;

    use super::{
        DecodedAudioPlaybackSource, MixedDecodedAudioPlaybackSource,
        audio_backend_prepare_runtime_trace_line, audio_backend_startup_runtime_trace_line,
        playback_control_span_samples,
    };

    #[test]
    fn audio_backend_startup_trace_line_reports_sink_and_append_boundaries() {
        assert_eq!(
            audio_backend_startup_runtime_trace_line(
                2,
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(3),
                Duration::from_millis(4),
                2,
            ),
            "Transcription Studio audio backend startup: tracks=2 appended=2 stop_ms=1.000 sink_ms=2.000 player_pool_ms=3.000 append_ms=4.000"
        );
    }

    #[test]
    fn audio_backend_prepare_trace_line_reports_sink_prewarm_boundary() {
        assert_eq!(
            audio_backend_prepare_runtime_trace_line(
                true,
                Duration::from_millis(4),
                Duration::from_millis(5),
            ),
            "Transcription Studio audio backend prepare: outcome=ready sink_ms=4.000 player_pool_ms=5.000"
        );
        assert_eq!(
            audio_backend_prepare_runtime_trace_line(
                false,
                Duration::from_millis(6),
                Duration::from_millis(0),
            ),
            "Transcription Studio audio backend prepare: outcome=error sink_ms=6.000 player_pool_ms=0.000"
        );
    }

    #[test]
    fn decoded_audio_playback_source_interleaves_decoded_channels_from_seek_time() {
        let audio = DecodedAudio::new(
            2,
            vec![vec![0.1, 0.2, 0.3, 0.4], vec![-0.1, -0.2, -0.3, -0.4]],
        )
        .expect("valid decoded audio");

        let source = DecodedAudioPlaybackSource::new(Arc::new(audio), 0.5, None)
            .expect("valid source should be created");
        assert_eq!(source.size_hint(), (6, Some(6)));
        let samples = source.collect::<Vec<_>>();

        assert_eq!(samples, vec![0.2, -0.2, 0.3, -0.3, 0.4, -0.4]);
    }

    #[test]
    fn decoded_audio_playback_source_clamps_to_audio_range_and_end() {
        let audio = DecodedAudio::new(1, vec![vec![1.5, -2.0]]).expect("valid decoded audio");

        assert_eq!(
            DecodedAudioPlaybackSource::new(Arc::new(audio.clone()), 0.0, None)
                .expect("valid source should be created")
                .collect::<Vec<_>>(),
            vec![1.0, -1.0]
        );
        assert!(
            DecodedAudioPlaybackSource::new(Arc::new(audio), 10.0, None)
                .expect("valid empty source should be created")
                .is_empty()
        );
    }

    #[test]
    fn decoded_audio_playback_source_stops_at_requested_end_time() {
        let audio = DecodedAudio::new(4, vec![vec![0.1, 0.2, 0.3, 0.4]]).expect("valid audio");

        assert_eq!(
            DecodedAudioPlaybackSource::new(Arc::new(audio), 0.25, Some(0.75))
                .expect("valid source should be created")
                .collect::<Vec<_>>(),
            vec![0.2, 0.3]
        );
    }

    #[test]
    fn mixed_decoded_audio_playback_source_sums_compatible_tracks() {
        let first = DecodedAudio::new(2, vec![vec![0.1, 0.2, 0.3]]).expect("valid first audio");
        let second = DecodedAudio::new(2, vec![vec![0.4, -0.7]]).expect("valid second audio");
        let first = DecodedAudioPlaybackSource::new(Arc::new(first), 0.0, None)
            .expect("valid first source");
        let second = DecodedAudioPlaybackSource::new(Arc::new(second), 0.0, None)
            .expect("valid second source");

        let mixed = MixedDecodedAudioPlaybackSource::new(vec![first, second])
            .expect("same-shape decoded sources should be mixed");

        assert_eq!(mixed.collect::<Vec<_>>(), vec![0.5, -0.5, 0.3]);
    }

    #[test]
    fn mixed_decoded_audio_playback_source_rejects_shape_mismatches_for_fallback() {
        let mono = DecodedAudio::new(2, vec![vec![0.1, 0.2]]).expect("valid mono audio");
        let stereo =
            DecodedAudio::new(2, vec![vec![0.1, 0.2], vec![0.3, 0.4]]).expect("valid stereo audio");
        let mono =
            DecodedAudioPlaybackSource::new(Arc::new(mono), 0.0, None).expect("valid mono source");
        let stereo = DecodedAudioPlaybackSource::new(Arc::new(stereo), 0.0, None)
            .expect("valid stereo source");

        assert!(MixedDecodedAudioPlaybackSource::new(vec![mono, stereo]).is_none());
    }
    const RATE_TEST_SAMPLE_RATE: u32 = 1_000;
    const RATE_TEST_SOURCE_SAMPLES: usize = 4_000;
    const RATE_CHANGE_AFTER_SAMPLES: usize = 1_000;
    const MAX_RATE_TEST_RENDERED_SAMPLES: usize = 10_000;

    fn decoded_rate_test_source() -> Box<dyn Source + Send> {
        let audio = DecodedAudio::new(
            RATE_TEST_SAMPLE_RATE,
            vec![vec![0.5; RATE_TEST_SOURCE_SAMPLES]],
        )
        .expect("valid decoded audio");
        Box::new(
            DecodedAudioPlaybackSource::new(Arc::new(audio), 0.0, None)
                .expect("valid playback source"),
        )
    }

    fn mixed_rate_test_source() -> Box<dyn Source + Send> {
        let first = DecodedAudio::new(
            RATE_TEST_SAMPLE_RATE,
            vec![vec![0.25; RATE_TEST_SOURCE_SAMPLES]],
        )
        .expect("valid first audio");
        let second = DecodedAudio::new(
            RATE_TEST_SAMPLE_RATE,
            vec![vec![0.25; RATE_TEST_SOURCE_SAMPLES]],
        )
        .expect("valid second audio");
        let first = DecodedAudioPlaybackSource::new(Arc::new(first), 0.0, None)
            .expect("valid first source");
        let second = DecodedAudioPlaybackSource::new(Arc::new(second), 0.0, None)
            .expect("valid second source");
        Box::new(
            MixedDecodedAudioPlaybackSource::new(vec![first, second])
                .expect("compatible tracks should mix"),
        )
    }

    fn rendered_samples_after_active_speed_change(
        source: Box<dyn Source + Send>,
        next_speed: f32,
    ) -> usize {
        let (mixer, mut output) = rodio::mixer::mixer(
            ChannelCount::new(1).expect("one channel"),
            SampleRate::new(RATE_TEST_SAMPLE_RATE).expect("positive sample rate"),
        );
        let player = Player::connect_new(&mixer);
        player.append(source);

        for _ in 0..RATE_CHANGE_AFTER_SAMPLES {
            assert!(output.next().is_some());
        }
        player.set_speed(next_speed);

        let mut rendered_samples = RATE_CHANGE_AFTER_SAMPLES;
        while !player.empty() && rendered_samples < MAX_RATE_TEST_RENDERED_SAMPLES {
            assert!(output.next().is_some());
            rendered_samples += 1;
        }
        assert!(player.empty(), "playback source should finish");
        rendered_samples
    }

    fn rendered_samples_at_initial_speed(source: Box<dyn Source + Send>, speed: f32) -> usize {
        let (mixer, mut output) = rodio::mixer::mixer(
            ChannelCount::new(1).expect("one channel"),
            SampleRate::new(RATE_TEST_SAMPLE_RATE).expect("positive sample rate"),
        );
        let player = Player::connect_new(&mixer);
        player.set_speed(speed);
        player.append(source);

        let mut rendered_samples = 0;
        while !player.empty() && rendered_samples < MAX_RATE_TEST_RENDERED_SAMPLES {
            assert!(output.next().is_some());
            rendered_samples += 1;
        }
        assert!(player.empty(), "playback source should finish");
        rendered_samples
    }

    fn assert_initial_player_speed_resamples(source: impl Fn() -> Box<dyn Source + Send>) {
        let normal_samples = rendered_samples_at_initial_speed(source(), 1.0);
        let sped_up_samples = rendered_samples_at_initial_speed(source(), 2.0);
        let slowed_samples = rendered_samples_at_initial_speed(source(), 0.5);

        assert!(
            sped_up_samples < normal_samples * 3 / 4,
            "initial 2x playback should render materially fewer output samples: normal={normal_samples}, fast={sped_up_samples}"
        );
        assert!(
            slowed_samples > normal_samples * 3 / 2,
            "initial 0.5x playback should render materially more output samples: normal={normal_samples}, slow={slowed_samples}"
        );
    }

    fn assert_active_player_speed_change_resamples(source: impl Fn() -> Box<dyn Source + Send>) {
        let normal_samples = rendered_samples_after_active_speed_change(source(), 1.0);
        let sped_up_samples = rendered_samples_after_active_speed_change(source(), 2.0);
        let slowed_samples = rendered_samples_after_active_speed_change(source(), 0.5);

        assert!(
            sped_up_samples < normal_samples * 3 / 4,
            "2x playback should render materially fewer output samples: normal={normal_samples}, fast={sped_up_samples}"
        );
        assert!(
            slowed_samples > normal_samples * 3 / 2,
            "0.5x playback should render materially more output samples: normal={normal_samples}, slow={slowed_samples}"
        );
    }

    #[test]
    fn active_player_speed_changes_resample_decoded_audio() {
        assert_active_player_speed_change_resamples(decoded_rate_test_source);
    }

    #[test]
    fn active_player_speed_changes_resample_mixed_audio() {
        assert_active_player_speed_change_resamples(mixed_rate_test_source);
    }

    #[test]
    fn initial_player_speed_resamples_decoded_and_mixed_audio() {
        assert_initial_player_speed_resamples(decoded_rate_test_source);
        assert_initial_player_speed_resamples(mixed_rate_test_source);
    }

    #[test]
    fn playback_control_spans_are_finite_and_frame_aligned() {
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
}
