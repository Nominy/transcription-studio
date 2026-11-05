use super::{CutOverlay, EditorSession, MIN_LOOP_DURATION_SECONDS};
use crate::timeline_model::{MAX_VERTICAL_ZOOM, MIN_VERTICAL_ZOOM, TimeRange};
use studio_core::Project;

/// Playback speeds offered by the speed control, ascending.
pub const PLAYBACK_SPEED_STEPS: [f32; 6] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0];
const OVERLAY_DURATION_SECONDS: f64 = 1.0;
const TIMELINE_WHEEL_PAN_VIEWPORT_PIXELS: f64 = 1_000.0;

impl EditorSession {
    /// Moves to the next faster (`direction >= 0`) or slower step of [`PLAYBACK_SPEED_STEPS`],
    /// saturating at the ends.
    pub fn step_speed(&mut self, direction: i32) {
        let current = self.speed;
        self.speed = if direction >= 0 {
            PLAYBACK_SPEED_STEPS
                .iter()
                .copied()
                .find(|speed| *speed > current + 0.001)
                .unwrap_or(PLAYBACK_SPEED_STEPS[PLAYBACK_SPEED_STEPS.len() - 1])
        } else {
            PLAYBACK_SPEED_STEPS
                .iter()
                .rev()
                .copied()
                .find(|speed| *speed < current - 0.001)
                .unwrap_or(PLAYBACK_SPEED_STEPS[0])
        };
    }
}

impl EditorSession {
    pub fn toggle_playback(&mut self) {
        self.is_playing = !self.is_playing;
    }

    pub fn advance_transport(&mut self, project: &Project, elapsed_seconds: f64) {
        self.advance_transport_with_bounds(project, self.timeline_bounds(project), elapsed_seconds);
    }

    pub fn advance_transport_with_bounds(
        &mut self,
        project: &Project,
        bounds: TimeRange,
        elapsed_seconds: f64,
    ) {
        if !self.is_playing || !elapsed_seconds.is_finite() || elapsed_seconds <= 0.0 {
            return;
        }

        let mut next_playhead = self.playhead_seconds + elapsed_seconds * self.speed as f64;

        if let Some(range) = self.loop_range()
            && range.duration_seconds() > MIN_LOOP_DURATION_SECONDS
        {
            if next_playhead >= range.end_seconds {
                // Crossing the end keeps the overshoot; a playhead already past the end (the
                // selection moved or shrank behind it) starts over at the loop start.
                next_playhead = if self.playhead_seconds < range.end_seconds {
                    range.start_seconds
                        + (next_playhead - range.end_seconds) % range.duration_seconds()
                } else {
                    range.start_seconds
                };
            }
        } else if next_playhead >= bounds.end_seconds {
            next_playhead = bounds.end_seconds;
            self.is_playing = false;
        }

        self.playhead_seconds = next_playhead.clamp(bounds.start_seconds, bounds.end_seconds);
        self.select_segment_at_playhead(project);
        self.follow_playback_viewport(bounds);
    }

    /// Align the timeline before playback using the Wavesurfer
    /// follow rule. A playhead in the left half of the visible window keeps
    /// the user's current context; an off-screen playhead or one past the
    /// midpoint is centered so playback has room to advance.
    pub fn prepare_viewport_for_playback(&mut self, project: &Project) {
        self.prepare_viewport_for_playback_with_bounds(self.timeline_bounds(project));
    }

    pub fn prepare_viewport_for_playback_with_bounds(&mut self, bounds: TimeRange) {
        self.follow_playback_viewport(bounds);
    }

    fn follow_playback_viewport(&mut self, bounds: TimeRange) {
        let viewport_start = self.viewport.start_seconds;
        let viewport_end = self.viewport.end_seconds;
        let viewport_midpoint = viewport_start + (viewport_end - viewport_start) / 2.0;
        let viewport_is_invalid = !viewport_start.is_finite()
            || !viewport_end.is_finite()
            || viewport_end <= viewport_start;
        let playhead_is_outside =
            self.playhead_seconds < viewport_start || self.playhead_seconds > viewport_end;

        if viewport_is_invalid
            || !self.playhead_seconds.is_finite()
            || playhead_is_outside
            || self.playhead_seconds > viewport_midpoint
        {
            self.recenter_viewport(bounds);
        }
    }

    /// Recenters the timeline on the playhead when it lies outside the visible window (a
    /// visible playhead keeps the user's view).
    pub fn reveal_playhead(&mut self, project: &Project) {
        let viewport = self.viewport;
        if !(viewport.start_seconds..=viewport.end_seconds).contains(&self.playhead_seconds) {
            self.recenter_viewport(self.timeline_bounds(project));
        }
    }

    pub fn set_speed(&mut self, speed: f32) {
        if speed.is_finite() && speed > 0.0 {
            self.speed = speed;
        }
    }

    pub fn set_zoom(&mut self, project: &Project, zoom: f32) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(0.05, 10.0);
            self.recenter_viewport(self.timeline_bounds(project));
        }
    }

    pub fn set_zoom_around_time(&mut self, project: &Project, zoom: f32, anchor_time_seconds: f64) {
        if !zoom.is_finite() {
            return;
        }

        let bounds = self.timeline_bounds(project);
        let previous_viewport = self.viewport;
        self.zoom = zoom.clamp(0.05, 10.0);
        let visible_duration = self.visible_duration(bounds);
        if visible_duration >= bounds.duration_seconds() {
            self.viewport = bounds;
            return;
        }

        let anchor =
            finite_seconds(anchor_time_seconds).clamp(bounds.start_seconds, bounds.end_seconds);
        let previous_duration = previous_viewport.duration_seconds();
        let anchor_fraction = if previous_duration > 0.0 {
            ((anchor - previous_viewport.start_seconds) / previous_duration).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let max_start = bounds.end_seconds - visible_duration;
        let start =
            (anchor - visible_duration * anchor_fraction).clamp(bounds.start_seconds, max_start);
        self.viewport = TimeRange {
            start_seconds: start,
            end_seconds: start + visible_duration,
        };
    }

    pub fn pan_viewport_by_wheel_pixels(&mut self, project: &Project, delta_pixels: f32) {
        if !delta_pixels.is_finite() || delta_pixels == 0.0 {
            return;
        }
        let bounds = self.timeline_bounds(project);
        let duration = self
            .viewport
            .duration_seconds()
            .min(bounds.duration_seconds());
        if duration >= bounds.duration_seconds() {
            self.viewport = bounds;
            return;
        }

        let delta_seconds = duration * delta_pixels as f64 / TIMELINE_WHEEL_PAN_VIEWPORT_PIXELS;
        let max_start = bounds.end_seconds - duration;
        let start_seconds =
            (self.viewport.start_seconds + delta_seconds).clamp(bounds.start_seconds, max_start);
        self.viewport = TimeRange {
            start_seconds,
            end_seconds: start_seconds + duration,
        };
    }

    pub fn pan_viewport_by_time_delta(&mut self, project: &Project, delta_seconds: f64) {
        let bounds = self.timeline_bounds(project);
        self.pan_viewport_by_time_delta_with_bounds(bounds, delta_seconds);
    }

    pub fn pan_viewport_by_time_delta_with_bounds(
        &mut self,
        bounds: TimeRange,
        delta_seconds: f64,
    ) {
        if !delta_seconds.is_finite() || delta_seconds == 0.0 {
            return;
        }
        let duration = self
            .viewport
            .duration_seconds()
            .min(bounds.duration_seconds());
        if duration >= bounds.duration_seconds() {
            self.viewport = bounds;
            return;
        }

        let max_start = bounds.end_seconds - duration;
        let start_seconds =
            (self.viewport.start_seconds + delta_seconds).clamp(bounds.start_seconds, max_start);
        self.viewport = TimeRange {
            start_seconds,
            end_seconds: start_seconds + duration,
        };
    }

    pub fn seek_overview_fraction(&mut self, project: &Project, fraction: f32) {
        let bounds = self.timeline_bounds(project);
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.playhead_seconds = bounds.start_seconds + bounds.duration_seconds() * fraction as f64;
        self.select_segment_at_playhead(project);
        self.recenter_viewport(bounds);
    }

    pub fn set_overview_viewport_start_fraction(&mut self, project: &Project, fraction: f32) {
        let bounds = self.timeline_bounds(project);
        let visible_duration = self
            .viewport
            .duration_seconds()
            .min(bounds.duration_seconds());
        if visible_duration >= bounds.duration_seconds() {
            self.viewport = bounds;
            return;
        }

        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let max_start = bounds.end_seconds - visible_duration;
        let start = (bounds.start_seconds + bounds.duration_seconds() * fraction as f64)
            .clamp(bounds.start_seconds, max_start);
        self.viewport = TimeRange {
            start_seconds: start,
            end_seconds: start + visible_duration,
        };
    }

    pub fn drag_timeline_scrollbar_thumb(
        &mut self,
        project: &Project,
        pointer_fraction: f32,
        grab_offset_fraction: f32,
    ) {
        let bounds = self.timeline_bounds(project);
        let visible_duration = self
            .viewport
            .duration_seconds()
            .min(bounds.duration_seconds());
        if visible_duration >= bounds.duration_seconds() {
            self.viewport = bounds;
            return;
        }

        let pointer_fraction = finite_fraction(pointer_fraction);
        let grab_offset_fraction = finite_fraction(grab_offset_fraction);
        let thumb_width_fraction =
            (visible_duration / bounds.duration_seconds()).clamp(0.0, 1.0) as f32;
        let max_left_fraction = (1.0 - thumb_width_fraction).max(0.0);
        let left_fraction = (pointer_fraction - grab_offset_fraction).clamp(0.0, max_left_fraction);
        let start = bounds.start_seconds + bounds.duration_seconds() * left_fraction as f64;
        self.viewport = TimeRange {
            start_seconds: start,
            end_seconds: start + visible_duration,
        };
    }

    pub fn seek_to_time(&mut self, project: &Project, time_seconds: f64) {
        let bounds = self.timeline_bounds(project);
        self.playhead_seconds =
            finite_seconds(time_seconds).clamp(bounds.start_seconds, bounds.end_seconds);
        self.select_segment_at_playhead(project);
    }

    pub fn toggle_lane_mute(&mut self, lane_id: &str) {
        if !self.muted_lane_ids.insert(lane_id.to_string()) {
            self.muted_lane_ids.remove(lane_id);
        }
    }

    pub fn toggle_lane_collapse(&mut self, lane_id: &str) {
        if !self.collapsed_lane_ids.insert(lane_id.to_string()) {
            self.collapsed_lane_ids.remove(lane_id);
        }
    }

    pub fn set_lane_vertical_zoom(&mut self, lane_id: &str, vertical_zoom: f32) {
        if vertical_zoom.is_finite() {
            self.lane_vertical_zoom.insert(
                lane_id.to_string(),
                vertical_zoom.clamp(MIN_VERTICAL_ZOOM, MAX_VERTICAL_ZOOM),
            );
        }
    }

    pub fn toggle_diff_view(&mut self) {
        self.diff_view = !self.diff_view;
    }

    /// T: toggles a one-second cut preview at the playhead on the selected row's lane (else the
    /// first visible lane).
    pub fn toggle_overlay(&mut self, project: &Project) {
        if self.overlay.take().is_some() {
            return;
        }
        let Some(lane_id) = self.default_overlay_lane(project) else {
            return;
        };
        self.overlay = Some(CutOverlay {
            lane_id,
            range: one_second_overlay_range(self.timeline_bounds(project), self.playhead_seconds),
            looping: false,
        });
    }

    fn default_overlay_lane(&self, project: &Project) -> Option<String> {
        let visible = |lane_id: &str| {
            self.track_filter.matches_lane(lane_id) && !self.collapsed_lane_ids.contains(lane_id)
        };
        self.selected_segment_id
            .as_deref()
            .and_then(|segment_id| project.segment(segment_id))
            .map(|segment| segment.lane_id.as_str())
            .filter(|lane_id| visible(lane_id))
            .or_else(|| {
                project
                    .lanes
                    .iter()
                    .map(|lane| lane.id.as_str())
                    .find(|lane_id| visible(lane_id))
            })
            .map(str::to_string)
    }

    /// Moves an open cut preview (keeping its lane) to a one-second range at `start_seconds`.
    pub fn move_overlay_to_time(&mut self, project: &Project, start_seconds: f64) {
        let bounds = self.timeline_bounds(project);
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.range = one_second_overlay_range(bounds, start_seconds);
        }
    }

    /// Places the cut preview on `lane_id` over `start_seconds..end_seconds` (clamped to the
    /// timeline; an empty range leaves the overlay unchanged). Editing the selection on its
    /// lane keeps its loop; a selection on another lane starts unlooped.
    pub fn set_overlay_range(
        &mut self,
        project: &Project,
        lane_id: &str,
        start_seconds: f64,
        end_seconds: f64,
    ) {
        let bounds = self.timeline_bounds(project);
        let start_seconds =
            finite_seconds(start_seconds).clamp(bounds.start_seconds, bounds.end_seconds);
        let end_seconds =
            finite_seconds(end_seconds).clamp(bounds.start_seconds, bounds.end_seconds);
        if end_seconds <= start_seconds {
            return;
        }
        let looping = self
            .overlay
            .as_ref()
            .is_some_and(|overlay| overlay.lane_id == lane_id && overlay.looping);
        self.overlay = Some(CutOverlay {
            lane_id: lane_id.to_string(),
            range: TimeRange {
                start_seconds,
                end_seconds,
            },
            looping,
        });
    }

    /// Range of the cut preview when it sits on `lane_id`.
    pub fn overlay_on(&self, lane_id: &str) -> Option<TimeRange> {
        self.overlay
            .as_ref()
            .filter(|overlay| overlay.lane_id == lane_id)
            .map(|overlay| overlay.range)
    }

    /// The range playback loops over: the cut selection while it loops (L).
    pub fn loop_range(&self) -> Option<TimeRange> {
        self.overlay
            .as_ref()
            .filter(|overlay| overlay.looping)
            .map(|overlay| overlay.range)
    }

    /// Timeline seconds spanned by one CSS px of the lane surface.
    pub fn seconds_per_px(&self) -> f64 {
        self.viewport.duration_seconds() / self.timeline_width_px
    }
}

fn finite_seconds(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_fraction(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn one_second_overlay_range(bounds: TimeRange, start_seconds: f64) -> TimeRange {
    let timeline_end = if bounds.end_seconds > bounds.start_seconds {
        bounds.end_seconds
    } else {
        bounds.start_seconds + OVERLAY_DURATION_SECONDS
    };
    let max_start = (timeline_end - OVERLAY_DURATION_SECONDS).max(bounds.start_seconds);
    let start = finite_seconds(start_seconds).clamp(bounds.start_seconds, max_start);
    TimeRange {
        start_seconds: start,
        end_seconds: start + OVERLAY_DURATION_SECONDS,
    }
}
