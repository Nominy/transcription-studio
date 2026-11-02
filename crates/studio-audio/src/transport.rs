//! Wall-clock transport model. The playhead is derived from an anchor
//! `(instant, seconds)` plus elapsed time × speed, so position queries are
//! cheap, smooth and deterministic under a manual clock. The audio sources
//! apply the same loop/end rules, so audio and playhead stay aligned.

use std::time::Instant;

/// Loop ranges shorter than this are ignored.
pub const MIN_LOOP_DURATION_SECONDS: f64 = 0.01;
/// Slowest supported playback rate; slower requests are clamped.
pub const MIN_PLAYBACK_SPEED: f32 = 0.1;
/// Speed steps offered by the toolbar and the speed hotkeys.
pub const PLAYBACK_SPEED_STEPS: [f32; 6] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0];

/// Next speed step above (`direction >= 0`) or below `current_speed`,
/// saturating at the first/last step.
pub fn adjacent_playback_speed(current_speed: f32, direction: i32) -> f32 {
    if direction >= 0 {
        PLAYBACK_SPEED_STEPS
            .iter()
            .copied()
            .find(|speed| *speed > current_speed + 0.001)
            .unwrap_or(PLAYBACK_SPEED_STEPS[PLAYBACK_SPEED_STEPS.len() - 1])
    } else {
        PLAYBACK_SPEED_STEPS
            .iter()
            .rev()
            .copied()
            .find(|speed| *speed < current_speed - 0.001)
            .unwrap_or(PLAYBACK_SPEED_STEPS[0])
    }
}

/// Loop region in timeline seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopRange {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackState {
    Stopped,
    /// Normal transport playback; honours the loop range and stops at the timeline end.
    Playing,
    /// Range playback (shift-click / held audition keys / buffered playback).
    /// Ignores the loop; ends at `end_seconds`.
    Auditioning {
        start_seconds: f64,
        end_seconds: f64,
    },
}

/// Transitions reported by [`crate::AudioEngine::poll`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackEvent {
    /// Playback reached the timeline end; the engine is now stopped at the end.
    Ended,
    /// An audition reached its end; the engine is stopped at the audition start.
    AuditionEnded,
    /// The playhead wrapped from the loop end back to the loop start.
    Looped,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    pub(crate) seconds: f64,
    pub(crate) wraps: u64,
    pub(crate) reached_end: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Transport {
    state: PlaybackState,
    anchor_time: Instant,
    anchor_seconds: f64,
    speed: f32,
    loop_range: Option<LoopRange>,
    timeline_end: f64,
    reported_wraps: u64,
}

impl Transport {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            state: PlaybackState::Stopped,
            anchor_time: now,
            anchor_seconds: 0.0,
            speed: 1.0,
            loop_range: None,
            timeline_end: 0.0,
            reported_wraps: 0,
        }
    }

    pub(crate) fn state(&self) -> PlaybackState {
        self.state
    }

    pub(crate) fn speed(&self) -> f32 {
        self.speed
    }

    pub(crate) fn loop_range(&self) -> Option<LoopRange> {
        self.loop_range
    }

    pub(crate) fn timeline_end(&self) -> f64 {
        self.timeline_end
    }

    /// End bound for the current mode.
    pub(crate) fn end_seconds(&self) -> f64 {
        match self.state {
            PlaybackState::Auditioning { end_seconds, .. } => end_seconds,
            _ => self.timeline_end,
        }
    }

    /// The loop as applied to playback: only while `Playing`, clipped to the
    /// timeline end, and only when long enough.
    pub(crate) fn effective_loop(&self) -> Option<(f64, f64)> {
        if self.state != PlaybackState::Playing {
            return None;
        }
        let range = self.loop_range?;
        let end = range.end_seconds.min(self.timeline_end);
        (end - range.start_seconds > MIN_LOOP_DURATION_SECONDS)
            .then_some((range.start_seconds, end))
    }

    pub(crate) fn sample(&self, now: Instant) -> Sample {
        if self.state == PlaybackState::Stopped {
            return Sample {
                seconds: self.anchor_seconds,
                wraps: 0,
                reached_end: false,
            };
        }
        let elapsed = now
            .saturating_duration_since(self.anchor_time)
            .as_secs_f64();
        let seconds = self.anchor_seconds + elapsed * f64::from(self.speed);
        if let Some((loop_start, loop_end)) = self.effective_loop() {
            if seconds < loop_end {
                return Sample {
                    seconds,
                    wraps: 0,
                    reached_end: false,
                };
            }
            let duration = loop_end - loop_start;
            let overflow = seconds - loop_end;
            return Sample {
                seconds: loop_start + overflow % duration,
                wraps: (overflow / duration).floor() as u64 + 1,
                reached_end: false,
            };
        }
        let end = self.end_seconds();
        if seconds >= end {
            Sample {
                seconds: end,
                wraps: 0,
                reached_end: true,
            }
        } else {
            Sample {
                seconds,
                wraps: 0,
                reached_end: false,
            }
        }
    }

    pub(crate) fn position(&self, now: Instant) -> f64 {
        self.sample(now).seconds
    }

    /// Re-anchors at the current position so parameters can change without
    /// moving the playhead.
    pub(crate) fn rebase(&mut self, now: Instant) {
        self.anchor_seconds = self.position(now);
        self.anchor_time = now;
        self.reported_wraps = 0;
        self.normalize_anchor();
    }

    /// Sets the playhead (mode unchanged). Returns the normalized position.
    pub(crate) fn set_position(&mut self, now: Instant, seconds: f64) -> f64 {
        self.anchor_seconds = seconds;
        self.anchor_time = now;
        self.reported_wraps = 0;
        self.normalize_anchor();
        self.anchor_seconds
    }

    pub(crate) fn set_state(&mut self, now: Instant, state: PlaybackState, seconds: f64) -> f64 {
        self.state = state;
        self.set_position(now, seconds)
    }

    pub(crate) fn set_speed(&mut self, now: Instant, speed: f32) {
        self.rebase(now);
        self.speed = speed;
    }

    pub(crate) fn set_loop(&mut self, now: Instant, loop_range: Option<LoopRange>) {
        self.rebase(now);
        self.loop_range = loop_range;
        self.normalize_anchor();
    }

    pub(crate) fn set_timeline_end(&mut self, now: Instant, timeline_end: f64) {
        self.rebase(now);
        self.timeline_end = timeline_end;
        self.normalize_anchor();
    }

    /// Records a loop wrap count; true when it is new since the last report.
    pub(crate) fn report_wraps(&mut self, wraps: u64) -> bool {
        if wraps > self.reported_wraps {
            self.reported_wraps = wraps;
            return true;
        }
        false
    }

    /// Positions at/after the loop end jump to the loop start, matching the
    /// audio source's wrap rule.
    fn normalize_anchor(&mut self) {
        if let Some((loop_start, loop_end)) = self.effective_loop()
            && self.anchor_seconds >= loop_end
        {
            self.anchor_seconds = loop_start;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn playing(now: Instant, start: f64, end: f64) -> Transport {
        let mut transport = Transport::new(now);
        transport.set_timeline_end(now, end);
        transport.set_state(now, PlaybackState::Playing, start);
        transport
    }

    #[test]
    fn speed_steps_move_to_adjacent_values_and_saturate() {
        assert_eq!(adjacent_playback_speed(1.0, 1), 1.5);
        assert_eq!(adjacent_playback_speed(1.0, -1), 0.75);
        assert_eq!(adjacent_playback_speed(2.0, 1), 2.0);
        assert_eq!(adjacent_playback_speed(0.25, -1), 0.25);
        assert_eq!(adjacent_playback_speed(1.2, 1), 1.5);
        assert_eq!(adjacent_playback_speed(1.2, -1), 1.0);
    }

    #[test]
    fn playhead_advances_by_elapsed_time_times_speed() {
        let start = Instant::now();
        let mut transport = playing(start, 3.5, 60.0);
        transport.set_speed(start, 2.0);

        let seconds = transport.position(start + Duration::from_millis(16));

        assert!((seconds - 3.532).abs() < 1e-9);
    }

    #[test]
    fn stopped_transport_holds_position() {
        let start = Instant::now();
        let mut transport = Transport::new(start);
        transport.set_timeline_end(start, 10.0);
        transport.set_position(start, 4.0);

        assert_eq!(transport.position(start + Duration::from_secs(3)), 4.0);
    }

    #[test]
    fn playback_clamps_to_timeline_end_and_reports_end() {
        let start = Instant::now();
        let transport = playing(start, 9.5, 10.0);

        let sample = transport.sample(start + Duration::from_secs(1));

        assert_eq!(sample.seconds, 10.0);
        assert!(sample.reached_end);
    }

    #[test]
    fn loop_wraps_overflow_into_loop_range_and_counts_wraps() {
        let start = Instant::now();
        let mut transport = playing(start, 1.0, 10.0);
        transport.set_loop(
            start,
            Some(LoopRange {
                start_seconds: 1.0,
                end_seconds: 2.0,
            }),
        );

        let sample = transport.sample(start + Duration::from_millis(2_250));

        assert!((sample.seconds - 1.25).abs() < 1e-9);
        assert_eq!(sample.wraps, 2);
        assert!(!sample.reached_end);
    }

    #[test]
    fn enabling_loop_past_its_end_jumps_to_loop_start() {
        let start = Instant::now();
        let mut transport = playing(start, 5.0, 10.0);

        transport.set_loop(
            start,
            Some(LoopRange {
                start_seconds: 1.0,
                end_seconds: 2.0,
            }),
        );

        assert_eq!(transport.position(start), 1.0);
    }

    #[test]
    fn short_loops_and_auditions_ignore_the_loop() {
        let start = Instant::now();
        let mut transport = playing(start, 1.0, 10.0);
        transport.set_loop(
            start,
            Some(LoopRange {
                start_seconds: 1.0,
                end_seconds: 1.005,
            }),
        );
        assert_eq!(transport.effective_loop(), None);

        transport.set_loop(
            start,
            Some(LoopRange {
                start_seconds: 1.0,
                end_seconds: 2.0,
            }),
        );
        transport.set_state(
            start,
            PlaybackState::Auditioning {
                start_seconds: 1.5,
                end_seconds: 3.0,
            },
            1.5,
        );
        assert_eq!(transport.effective_loop(), None);
        let sample = transport.sample(start + Duration::from_secs(2));
        assert_eq!(sample.seconds, 3.0);
        assert!(sample.reached_end);
    }

    #[test]
    fn speed_change_rebases_without_moving_the_playhead() {
        let start = Instant::now();
        let mut transport = playing(start, 0.0, 60.0);
        let mid = start + Duration::from_secs(1);

        transport.set_speed(mid, 0.5);

        assert!((transport.position(mid) - 1.0).abs() < 1e-9);
        assert!((transport.position(mid + Duration::from_secs(2)) - 2.0).abs() < 1e-9);
    }
}
