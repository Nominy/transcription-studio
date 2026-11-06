use std::{collections::HashMap, sync::Arc};

use rodio::{
    ChannelCount, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate, Source,
    mixer::{Mixer, MixerSource},
};
use studio_waveform::DecodedAudio;

use crate::{
    AudioError, LoopRange, MIN_PLAYBACK_SPEED, ManualClock, PlaybackEvent, PlaybackState,
    blend::enhanced_mismatch,
    clock::Clock,
    source::{ControlState, LaneGain, LaneSources, MixSource, SessionControl},
    transport::Transport,
};

/// Keeps the full playback path but emits silence (dev runs on shared machines).
const AUDIO_SILENT_ENV: &str = "STUDIO_AUDIO_SILENT";
/// Logs one line per transport start: lanes, start time and speed.
const AUDIO_PLAYBACK_LOG_ENV: &str = "STUDIO_LOG_AUDIO_PLAYBACK";
/// Upper bound for [`AudioEngine::set_lane_volume`].
pub const MAX_LANE_VOLUME: f32 = 2.0;

/// Per-lane mix settings. A lane is audible when it is not muted and either
/// no lane is soloed or it is soloed itself; its gain is then `volume`.
/// `enhanced_ratio` is the share of the lane's enhanced PCM in its sound
/// (`0` = original only, `1` = enhanced only); it has no effect while the
/// lane has no enhanced PCM.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneMix {
    pub muted: bool,
    pub solo: bool,
    pub volume: f32,
    pub enhanced_ratio: f32,
}

impl Default for LaneMix {
    fn default() -> Self {
        Self {
            muted: false,
            solo: false,
            volume: 1.0,
            enhanced_ratio: 0.0,
        }
    }
}

struct Lane {
    id: String,
    audio: Arc<DecodedAudio>,
    /// Same clock as `audio` (validated rate and frame count).
    enhanced: Option<Arc<DecodedAudio>>,
}

/// Players feeding the output for the current play/audition. Dropping it
/// stops them; the mixer releases their sources on its next callback.
struct Session {
    control: Arc<SessionControl>,
    players: Vec<Player>,
}

enum Output {
    /// Default output device, opened lazily and released by `unload`.
    Device(Option<MixerDeviceSink>),
    /// Headless mixer drained by [`OfflineOutput`].
    Offline(Mixer),
}

impl Output {
    fn mixer(&mut self) -> Result<&Mixer, AudioError> {
        match self {
            Self::Device(sink) => {
                if sink.is_none() {
                    let mut opened = DeviceSinkBuilder::open_default_sink()
                        .map_err(|error| AudioError::OutputUnavailable(error.to_string()))?;
                    opened.log_on_drop(false);
                    *sink = Some(opened);
                }
                Ok(sink.as_ref().expect("sink opened above").mixer())
            }
            Self::Offline(mixer) => Ok(mixer),
        }
    }

    fn release(&mut self) {
        if let Self::Device(sink) = self {
            *sink = None;
        }
    }
}

/// Pull side of a headless engine: renders the mixed output on demand.
pub struct OfflineOutput {
    source: MixerSource,
}

impl OfflineOutput {
    pub fn channels(&self) -> u16 {
        self.source.channels().get()
    }

    pub fn sample_rate(&self) -> u32 {
        self.source.sample_rate().get()
    }

    /// Next `samples` interleaved output samples (silence when nothing plays).
    pub fn render(&mut self, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|_| self.source.next().unwrap_or(0.0))
            .collect()
    }
}

/// Multi-lane playback engine: owns decoded lane PCM, the transport (playhead,
/// speed, loop, audition) and the rodio output graph.
pub struct AudioEngine {
    // Declared before `output` so players drop before the device sink.
    session: Option<Session>,
    output: Output,
    clock: Clock,
    lanes: Vec<Lane>,
    mix: HashMap<String, LaneMix>,
    /// Replaced PCM a live source may still read; dropped once the audio
    /// thread has released it so large buffers are not freed in the output
    /// callback.
    retired: Vec<Arc<DecodedAudio>>,
    transport: Transport,
    timeline_end: Option<f64>,
    silent: bool,
    log_playback: bool,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    /// Engine on the default output device (opened on first lane insert or
    /// play) with the system clock. Honours `STUDIO_AUDIO_SILENT` and
    /// `STUDIO_LOG_AUDIO_PLAYBACK`.
    pub fn new() -> Self {
        Self::build(
            Output::Device(None),
            Clock::System,
            env_flag(AUDIO_SILENT_ENV),
            env_flag(AUDIO_PLAYBACK_LOG_ENV),
        )
    }

    /// Headless engine for tests: no audio device, time driven by `clock`,
    /// output pulled through the returned [`OfflineOutput`].
    pub fn offline(channels: u16, sample_rate: u32, clock: ManualClock) -> (Self, OfflineOutput) {
        let (mixer, source) = rodio::mixer::mixer(
            ChannelCount::new(channels.max(1)).expect("channel count is non-zero"),
            SampleRate::new(sample_rate.max(1)).expect("sample rate is non-zero"),
        );
        (
            Self::build(Output::Offline(mixer), Clock::Manual(clock), false, false),
            OfflineOutput { source },
        )
    }

    fn build(output: Output, clock: Clock, silent: bool, log_playback: bool) -> Self {
        let now = clock.now();
        Self {
            session: None,
            output,
            clock,
            lanes: Vec::new(),
            mix: HashMap::new(),
            retired: Vec::new(),
            transport: Transport::new(now),
            timeline_end: None,
            silent,
            log_playback,
        }
    }

    /// Opens the output device ahead of the first play so starting playback
    /// stays cheap. Called by `insert_lane`; failures resurface from `play`.
    pub fn prepare(&mut self) -> Result<(), AudioError> {
        self.output.mixer().map(|_| ())
    }

    // ---- lanes -------------------------------------------------------------

    /// Adds or replaces a lane's decoded audio. The same `Arc` can back the
    /// lane's waveform, so PCM is decoded and held once. Replacing audio drops
    /// the lane's enhanced PCM (it was derived from the old audio). If
    /// playback is running it restarts in place to include the lane.
    pub fn insert_lane(
        &mut self,
        lane_id: impl Into<String>,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), AudioError> {
        let channels = audio.number_of_channels();
        if channels == 0 || channels > usize::from(u16::MAX) {
            return Err(AudioError::UnsupportedChannelCount(channels));
        }
        let lane_id = lane_id.into();
        match self.lanes.iter_mut().find(|lane| lane.id == lane_id) {
            Some(lane) => {
                let previous = std::mem::replace(&mut lane.audio, audio);
                let enhanced = lane.enhanced.take();
                self.retired.push(previous);
                self.retired.extend(enhanced);
            }
            None => self.lanes.push(Lane {
                id: lane_id,
                audio,
                enhanced: None,
            }),
        }
        self.lanes_changed();
        let _ = self.prepare();
        Ok(())
    }

    /// Removes a lane (and its enhanced PCM) and returns its original audio.
    pub fn remove_lane(&mut self, lane_id: &str) -> Option<Arc<DecodedAudio>> {
        let index = self.lanes.iter().position(|lane| lane.id == lane_id)?;
        let lane = self.lanes.remove(index);
        self.lanes_changed();
        Some(lane.audio)
    }

    pub fn lane_audio(&self, lane_id: &str) -> Option<&Arc<DecodedAudio>> {
        self.lanes
            .iter()
            .find(|lane| lane.id == lane_id)
            .map(|lane| &lane.audio)
    }

    /// Installs enhanced PCM beside the originals of loaded lanes, all or
    /// nothing: every entry must name a loaded lane and match its original's
    /// sample rate and frame count, with one channel (shared by all original
    /// channels) or the original's channel count. Playback restarts in place
    /// once, on the original clock; timeline and positions are unchanged.
    pub fn set_enhanced_audio(
        &mut self,
        lanes: Vec<(String, Arc<DecodedAudio>)>,
    ) -> Result<(), AudioError> {
        let mut targets = Vec::with_capacity(lanes.len());
        for (lane_id, enhanced) in lanes {
            let Some(index) = self.lanes.iter().position(|lane| lane.id == lane_id) else {
                return Err(AudioError::LaneNotLoaded(lane_id));
            };
            if let Some(reason) = enhanced_mismatch(&self.lanes[index].audio, &enhanced) {
                return Err(AudioError::EnhancedAudioMismatch { lane_id, reason });
            }
            targets.push((index, enhanced));
        }
        if targets.is_empty() {
            return Ok(());
        }
        for (index, enhanced) in targets {
            let previous = self.lanes[index].enhanced.replace(enhanced);
            self.retired.extend(previous);
        }
        self.lanes_changed();
        Ok(())
    }

    /// Drops every lane's enhanced PCM; playback continues on the originals.
    pub fn clear_enhanced_audio(&mut self) {
        let mut cleared = false;
        for lane in &mut self.lanes {
            if let Some(enhanced) = lane.enhanced.take() {
                self.retired.push(enhanced);
                cleared = true;
            }
        }
        if cleared {
            self.lanes_changed();
        }
    }

    pub fn lane_enhanced_audio(&self, lane_id: &str) -> Option<&Arc<DecodedAudio>> {
        self.lanes
            .iter()
            .find(|lane| lane.id == lane_id)
            .and_then(|lane| lane.enhanced.as_ref())
    }

    /// Loaded lane ids in insertion order.
    pub fn lane_ids(&self) -> impl Iterator<Item = &str> {
        self.lanes.iter().map(|lane| lane.id.as_str())
    }

    /// Stops playback and releases every decoded buffer held by the engine,
    /// the mix settings and the output device. Speed is kept (it is a user
    /// preference); position, loop and timeline end reset.
    pub fn unload(&mut self) {
        self.session = None;
        self.lanes = Vec::new();
        self.mix = HashMap::new();
        self.retired = Vec::new();
        self.output.release();
        self.timeline_end = None;
        let speed = self.transport.speed();
        let now = self.now();
        self.transport = Transport::new(now);
        self.transport.set_speed(now, speed);
    }

    // ---- mix ---------------------------------------------------------------

    pub fn lane_mix(&self, lane_id: &str) -> LaneMix {
        self.mix.get(lane_id).copied().unwrap_or_default()
    }

    /// Mix settings may be set before the lane's audio is loaded. Changes
    /// apply to live playback within one control span (~20 ms).
    pub fn set_lane_muted(&mut self, lane_id: &str, muted: bool) {
        self.mix.entry(lane_id.to_string()).or_default().muted = muted;
        self.push_controls();
    }

    pub fn set_lane_solo(&mut self, lane_id: &str, solo: bool) {
        self.mix.entry(lane_id.to_string()).or_default().solo = solo;
        self.push_controls();
    }

    /// Non-finite values are ignored; others clamp to `0..=MAX_LANE_VOLUME`.
    pub fn set_lane_volume(&mut self, lane_id: &str, volume: f32) {
        if !volume.is_finite() {
            return;
        }
        self.mix.entry(lane_id.to_string()).or_default().volume =
            volume.clamp(0.0, MAX_LANE_VOLUME);
        self.push_controls();
    }

    /// Share of the lane's enhanced PCM in its sound. Non-finite values are
    /// ignored; others clamp to `0..=1`. Applies to live playback within one
    /// control span without restarting it.
    pub fn set_lane_enhanced_ratio(&mut self, lane_id: &str, ratio: f32) {
        if !ratio.is_finite() {
            return;
        }
        self.mix
            .entry(lane_id.to_string())
            .or_default()
            .enhanced_ratio = ratio.clamp(0.0, 1.0);
        self.push_controls();
    }

    /// Loaded lanes that currently contribute sound.
    pub fn audible_lane_count(&self) -> usize {
        self.gains()
            .into_iter()
            .filter(|gain| gain.is_audible())
            .count()
    }

    // ---- transport ---------------------------------------------------------

    pub fn state(&self) -> PlaybackState {
        self.transport.state()
    }

    /// Current playhead in seconds. Cheap (clock read + arithmetic); call it
    /// every frame. Does not change state; see [`poll`](Self::poll).
    pub fn position(&self) -> f64 {
        self.transport.position(self.now())
    }

    /// Applies time-driven transitions (end of playback, end of audition)
    /// and reports them. Call once per frame while not stopped.
    pub fn poll(&mut self) -> Option<PlaybackEvent> {
        self.release_retired();
        let state = self.transport.state();
        if state == PlaybackState::Stopped {
            return None;
        }
        let now = self.now();
        let sample = self.transport.sample(now);
        if sample.reached_end {
            self.session = None;
            return Some(match state {
                PlaybackState::Auditioning { start_seconds, .. } => {
                    self.transport
                        .set_state(now, PlaybackState::Stopped, start_seconds);
                    PlaybackEvent::AuditionEnded
                }
                _ => {
                    self.transport
                        .set_state(now, PlaybackState::Stopped, sample.seconds);
                    PlaybackEvent::Ended
                }
            });
        }
        self.transport
            .report_wraps(sample.wraps)
            .then_some(PlaybackEvent::Looped)
    }

    /// Starts transport playback from the playhead (from the audition start
    /// if auditioning; from 0 if parked at the end). The transport runs even
    /// when audio fails: `Err` only reports that the output device is
    /// unavailable. `Ok(n)` is the number of audible lanes.
    pub fn play(&mut self) -> Result<usize, AudioError> {
        let now = self.now();
        let start = match self.transport.state() {
            PlaybackState::Playing => return Ok(self.audible_lane_count()),
            PlaybackState::Auditioning { start_seconds, .. } => start_seconds,
            PlaybackState::Stopped => self.transport.position(now),
        };
        let end = self.transport.timeline_end();
        let start = if end > 0.0 && start >= end {
            0.0
        } else {
            start
        };
        let start = self.transport.set_state(now, PlaybackState::Playing, start);
        let result = self.start_session();
        if self.log_playback
            && let Ok(lanes) = result
            && lanes > 0
        {
            eprintln!(
                "Exaudio audio playback: tracks={lanes} start={start:.3}s speed={:.2}x",
                self.transport.speed()
            );
        }
        result
    }

    /// `seek(seconds)` then `play()`.
    pub fn play_from(&mut self, seconds: f64) -> Result<usize, AudioError> {
        self.seek(seconds);
        self.play()
    }

    /// Plays `start..end` once at the current speed, ignoring the loop. When
    /// it ends (or is paused) the playhead returns to `start`. Non-finite or
    /// empty ranges are ignored (`Ok(0)`, state unchanged).
    pub fn audition(&mut self, start_seconds: f64, end_seconds: f64) -> Result<usize, AudioError> {
        if !start_seconds.is_finite() || !end_seconds.is_finite() {
            return Ok(0);
        }
        let start_seconds = start_seconds.max(0.0);
        if end_seconds <= start_seconds {
            return Ok(0);
        }
        let now = self.now();
        self.transport.set_state(
            now,
            PlaybackState::Auditioning {
                start_seconds,
                end_seconds,
            },
            start_seconds,
        );
        self.start_session()
    }

    /// Stops playback or audition and keeps the playhead (an audition
    /// returns to its start).
    pub fn pause(&mut self) {
        let now = self.now();
        let seconds = match self.transport.state() {
            PlaybackState::Stopped => return,
            PlaybackState::Auditioning { start_seconds, .. } => start_seconds,
            PlaybackState::Playing => self.transport.position(now),
        };
        self.session = None;
        self.transport
            .set_state(now, PlaybackState::Stopped, seconds);
    }

    /// Moves the playhead, clamped to `0..=duration()`. While playing the
    /// live sources jump without restarting players (scrub-safe); an active
    /// audition is cancelled.
    pub fn seek(&mut self, seconds: f64) {
        if !seconds.is_finite() {
            return;
        }
        let now = self.now();
        let seconds = seconds.clamp(0.0, self.transport.timeline_end());
        match self.transport.state() {
            PlaybackState::Auditioning { .. } => {
                self.session = None;
                self.transport
                    .set_state(now, PlaybackState::Stopped, seconds);
            }
            PlaybackState::Playing => {
                let seconds = self.transport.set_position(now, seconds);
                if let Some(session) = &self.session {
                    session.control.seek(seconds);
                }
            }
            PlaybackState::Stopped => {
                self.transport.set_position(now, seconds);
            }
        }
    }

    pub fn speed(&self) -> f32 {
        self.transport.speed()
    }

    /// Ignores non-finite or non-positive speeds; clamps to
    /// `MIN_PLAYBACK_SPEED`. Applies to live playback (pitch follows speed).
    pub fn set_speed(&mut self, speed: f32) {
        if !speed.is_finite() || speed <= 0.0 {
            return;
        }
        let speed = speed.max(MIN_PLAYBACK_SPEED);
        let now = self.now();
        self.transport.set_speed(now, speed);
        if let Some(session) = &self.session {
            for player in &session.players {
                player.set_speed(speed);
            }
        }
    }

    pub fn loop_range(&self) -> Option<LoopRange> {
        self.transport.loop_range()
    }

    /// Enables (`Some`) or disables (`None`) looping during `Playing`. Ranges
    /// that are non-finite or not increasing disable the loop. A playhead at
    /// or past the loop end jumps to the loop start.
    pub fn set_loop(&mut self, range: Option<LoopRange>) {
        let range = range.filter(|range| {
            range.start_seconds.is_finite()
                && range.end_seconds.is_finite()
                && range.end_seconds > range.start_seconds
        });
        let now = self.now();
        self.transport.set_loop(now, range);
        self.push_controls();
    }

    /// Timeline length: the override from [`set_timeline_end`](Self::set_timeline_end),
    /// else the longest loaded lane (0 with no lanes).
    pub fn duration(&self) -> f64 {
        self.transport.timeline_end()
    }

    /// Overrides the timeline end (e.g. project bounds that extend past the
    /// audio). `None` falls back to the longest lane.
    pub fn set_timeline_end(&mut self, end_seconds: Option<f64>) {
        self.timeline_end = end_seconds.filter(|end| end.is_finite() && *end >= 0.0);
        let now = self.now();
        self.transport
            .set_timeline_end(now, self.effective_timeline_end());
        self.push_controls();
    }

    // ---- internals ---------------------------------------------------------

    fn now(&self) -> std::time::Instant {
        self.clock.now()
    }

    fn effective_timeline_end(&self) -> f64 {
        self.timeline_end.unwrap_or_else(|| {
            self.lanes
                .iter()
                .map(|lane| lane.audio.duration())
                .fold(0.0, f64::max)
        })
    }

    fn gains(&self) -> Vec<LaneGain> {
        let any_solo = self.mix.values().any(|mix| mix.solo);
        self.lanes
            .iter()
            .map(|lane| {
                let mix = self.lane_mix(&lane.id);
                if mix.muted || (any_solo && !mix.solo) {
                    return LaneGain::default();
                }
                let ratio = if lane.enhanced.is_some() {
                    mix.enhanced_ratio
                } else {
                    0.0
                };
                LaneGain::blend(mix.volume, ratio)
            })
            .collect()
    }

    /// Drops retired PCM no live source references any more (the engine's
    /// clone plus the retired entry are the only owners).
    fn release_retired(&mut self) {
        self.retired.retain(|audio| {
            audio
                .channel_data_shared(0)
                .is_some_and(|channel| Arc::strong_count(&channel) > 2)
        });
    }

    fn control_state(&self) -> ControlState {
        ControlState {
            seek: None,
            end_seconds: self.transport.end_seconds(),
            loop_range: self.transport.effective_loop(),
            gains: self.gains(),
        }
    }

    fn push_controls(&self) {
        let Some(session) = &self.session else {
            return;
        };
        let next = self.control_state();
        session.control.update(|state| {
            state.end_seconds = next.end_seconds;
            state.loop_range = next.loop_range;
            state.gains = next.gains;
        });
    }

    fn lanes_changed(&mut self) {
        self.release_retired();
        let now = self.now();
        self.transport
            .set_timeline_end(now, self.effective_timeline_end());
        if self.transport.state() != PlaybackState::Stopped {
            let _ = self.start_session();
        }
    }

    /// Replaces the output session with fresh players starting at the
    /// playhead. Lanes are grouped by (sample rate, channels); each group is
    /// mixed into one source on one player.
    fn start_session(&mut self) -> Result<usize, AudioError> {
        self.session = None;
        if self.lanes.is_empty() {
            return Ok(0);
        }
        let start_seconds = self.position();
        let control_state = self.control_state();
        let audible = control_state
            .gains
            .iter()
            .filter(|gain| gain.is_audible())
            .count();
        let control = Arc::new(SessionControl::new(control_state));

        let mut groups: Vec<Vec<LaneSources<'_>>> = Vec::new();
        for (slot, lane) in self.lanes.iter().enumerate() {
            let sources = LaneSources {
                slot,
                original: lane.audio.as_ref(),
                enhanced: lane.enhanced.as_deref(),
            };
            let audio = sources.original;
            match groups.iter_mut().find(|group| {
                group.first().is_some_and(|first| {
                    first.original.sample_rate() == audio.sample_rate()
                        && first.original.number_of_channels() == audio.number_of_channels()
                })
            }) {
                Some(group) => group.push(sources),
                None => groups.push(vec![sources]),
            }
        }

        let mixer = self.output.mixer()?;
        let speed = self.transport.speed();
        let players = groups
            .iter()
            .filter_map(|group| MixSource::new(group, start_seconds, Arc::clone(&control)))
            .map(|source| {
                // Queue the source before attaching to the mixer: the mixer
                // sizes its first resampling span from the queue's metadata,
                // and an empty queue would report a default rate instead.
                let (player, queue) = Player::new();
                player.set_speed(speed);
                if self.silent {
                    player.set_volume(0.0);
                }
                player.append(source);
                mixer.add(queue);
                player
            })
            .collect();
        self.session = Some(Session { control, players });
        Ok(audible)
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| value != "0")
}
