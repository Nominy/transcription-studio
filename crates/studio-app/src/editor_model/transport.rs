use super::{EditorSession, MIN_LOOP_DURATION_SECONDS, visible_duration_for_zoom};
use crate::timeline_model::{
    LoopStripHit, LoopStripView, MAX_VERTICAL_ZOOM, MIN_VERTICAL_ZOOM, TimeRange, build_loop_strip,
    hit_loop_strip,
};
use crate::transcript_model::format_mm_ss;
use studio_core::Project;

const BABEL_OVERLAY_DURATION_SECONDS: f64 = 1.0;

impl EditorSession {
    pub fn toggle_playback(&mut self) {
        self.is_playing = !self.is_playing;
    }

    pub fn advance_transport(&mut self, project: &Project, elapsed_seconds: f64) {
        self.advance_transport_with_bounds(
            project,
            TimeRange::for_project(project),
            elapsed_seconds,
        );
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

        if self.loop_enabled {
            let loop_duration = self.loop_range.duration_seconds();
            if loop_duration > MIN_LOOP_DURATION_SECONDS
                && next_playhead >= self.loop_range.end_seconds
            {
                let overflow = next_playhead - self.loop_range.end_seconds;
                next_playhead = self.loop_range.start_seconds + overflow % loop_duration;
            }
        } else if next_playhead >= bounds.end_seconds {
            next_playhead = bounds.end_seconds;
            self.is_playing = false;
        }

        self.playhead_seconds = next_playhead.clamp(bounds.start_seconds, bounds.end_seconds);
        self.select_segment_at_playhead(project);
        self.recenter_viewport(bounds);
    }

    pub fn set_speed(&mut self, speed: f32) {
        if speed.is_finite() && speed > 0.0 {
            self.speed = speed;
        }
    }

    pub fn set_zoom(&mut self, project: &Project, zoom: f32) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(0.05, 10.0);
            self.recenter_viewport(TimeRange::for_project(project));
        }
    }

    pub fn set_zoom_around_time(&mut self, project: &Project, zoom: f32, anchor_time_seconds: f64) {
        if !zoom.is_finite() {
            return;
        }

        let bounds = TimeRange::for_project(project);
        let previous_viewport = self.viewport;
        self.zoom = zoom.clamp(0.05, 10.0);
        let visible_duration = visible_duration_for_zoom(bounds, self.zoom);
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

    pub fn pan_viewport_by_scroll_delta(&mut self, project: &Project, delta_y: f32) {
        if !delta_y.is_finite() || delta_y == 0.0 {
            return;
        }
        let bounds = TimeRange::for_project(project);
        let duration = self
            .viewport
            .duration_seconds()
            .min(bounds.duration_seconds());
        if duration >= bounds.duration_seconds() {
            self.viewport = bounds;
            return;
        }

        let delta_seconds = duration * delta_y as f64 * 0.05;
        let max_start = bounds.end_seconds - duration;
        let start_seconds =
            (self.viewport.start_seconds + delta_seconds).clamp(bounds.start_seconds, max_start);
        self.viewport = TimeRange {
            start_seconds,
            end_seconds: start_seconds + duration,
        };
    }

    pub fn pan_viewport_by_time_delta(&mut self, project: &Project, delta_seconds: f64) {
        let bounds = TimeRange::for_project(project);
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
        let bounds = TimeRange::for_project(project);
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
        let bounds = TimeRange::for_project(project);
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
        let bounds = TimeRange::for_project(project);
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
        let bounds = TimeRange::for_project(project);
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

    pub fn toggle_loop(&mut self, project: &Project) {
        if self.stored_loop_range.is_none() {
            return;
        }

        self.loop_enabled = !self.loop_enabled;
        if let Some(loop_range) = self.stored_loop_range {
            self.loop_range = loop_range;
        }
        self.recenter_viewport(TimeRange::for_project(project));
    }

    pub fn toggle_overlay(&mut self, project: &Project) {
        if self.overlay_range.is_some() {
            self.overlay_range = None;
            return;
        }

        self.overlay_range = Some(one_second_overlay_range(project, self.playhead_seconds));
    }

    pub fn move_overlay_to_time(&mut self, project: &Project, start_seconds: f64) {
        if self.overlay_range.is_none() {
            return;
        }

        self.overlay_range = Some(one_second_overlay_range(project, start_seconds));
    }

    pub fn set_overlay_range(&mut self, project: &Project, start_seconds: f64, end_seconds: f64) {
        let bounds = TimeRange::for_project(project);
        let start_seconds =
            finite_seconds(start_seconds).clamp(bounds.start_seconds, bounds.end_seconds);
        let end_seconds =
            finite_seconds(end_seconds).clamp(bounds.start_seconds, bounds.end_seconds);
        if end_seconds <= start_seconds {
            return;
        }

        self.overlay_range = Some(TimeRange {
            start_seconds,
            end_seconds,
        });
    }

    pub fn overlay_contains_time(&self, time_seconds: f64) -> bool {
        let Some(overlay_range) = self.overlay_range else {
            return false;
        };
        let time_seconds = finite_seconds(time_seconds);
        overlay_range.start_seconds <= time_seconds && time_seconds <= overlay_range.end_seconds
    }

    pub fn loop_strip(&self, _project: &Project) -> LoopStripView {
        build_loop_strip(
            self.loop_enabled,
            self.stored_loop_range,
            self.viewport,
            self.playhead_seconds,
        )
    }

    pub fn loop_strip_hit(&self, project: &Project, fraction: f32) -> LoopStripHit {
        hit_loop_strip(self.loop_strip(project), self.viewport, fraction)
    }

    pub fn set_loop_start(&mut self, _project: &Project, start_seconds: f64) {
        self.loop_range.start_seconds = finite_seconds(start_seconds)
            .min(self.loop_range.end_seconds - MIN_LOOP_DURATION_SECONDS);
        self.stored_loop_range = Some(self.loop_range);
        self.loop_enabled = true;
    }

    pub fn set_loop_end(&mut self, _project: &Project, end_seconds: f64) {
        self.loop_range.end_seconds = finite_seconds(end_seconds)
            .max(self.loop_range.start_seconds + MIN_LOOP_DURATION_SECONDS);
        self.stored_loop_range = Some(self.loop_range);
        self.loop_enabled = true;
    }

    pub fn move_loop_range(
        &mut self,
        project: &Project,
        original_start_seconds: f64,
        original_end_seconds: f64,
        grab_time_seconds: f64,
        current_time_seconds: f64,
    ) {
        let bounds = TimeRange::for_project(project);
        let duration = (original_end_seconds - original_start_seconds)
            .max(MIN_LOOP_DURATION_SECONDS)
            .min(bounds.duration_seconds());
        let delta = finite_seconds(current_time_seconds) - finite_seconds(grab_time_seconds);
        let max_start = bounds.end_seconds - duration;
        let start = (original_start_seconds + delta).clamp(bounds.start_seconds, max_start);
        self.loop_range = TimeRange {
            start_seconds: start,
            end_seconds: start + duration,
        };
        self.stored_loop_range = Some(self.loop_range);
        self.loop_enabled = true;
    }

    pub fn set_loop_range_from_drag(
        &mut self,
        project: &Project,
        anchor_seconds: f64,
        current_seconds: f64,
    ) {
        let bounds = TimeRange::for_project(project);
        let anchor = finite_seconds(anchor_seconds).clamp(bounds.start_seconds, bounds.end_seconds);
        let current =
            finite_seconds(current_seconds).clamp(bounds.start_seconds, bounds.end_seconds);
        let start_seconds = anchor.min(current);
        let end_seconds = anchor.max(current);
        self.loop_range = TimeRange {
            start_seconds,
            end_seconds,
        };
        self.stored_loop_range = Some(self.loop_range);
        self.loop_enabled = true;
    }

    pub fn clear_loop(&mut self, project: &Project) {
        self.loop_enabled = true;
        self.stored_loop_range = None;
        self.loop_range = TimeRange::for_project(project);
    }

    pub fn loop_label(&self) -> String {
        let Some(loop_range) = self.stored_loop_range else {
            return "Loop Off".to_string();
        };

        let suffix = if self.loop_enabled { "" } else { " Off" };
        format!(
            "Loop {} - {}{}",
            format_mm_ss(loop_range.start_seconds),
            format_mm_ss(loop_range.end_seconds),
            suffix
        )
    }
}

impl EditorSession {
    pub(super) fn reconcile_loop_range_with_project(&mut self, project: &Project) {
        let bounds = TimeRange::for_project(project);
        let Some(loop_range) = self.stored_loop_range else {
            self.loop_range = bounds;
            return;
        };

        let start = loop_range
            .start_seconds
            .clamp(bounds.start_seconds, bounds.end_seconds);
        let end = loop_range
            .end_seconds
            .clamp(bounds.start_seconds, bounds.end_seconds);
        if end - start < MIN_LOOP_DURATION_SECONDS {
            self.stored_loop_range = None;
            self.loop_range = bounds;
            return;
        }

        self.loop_range = TimeRange {
            start_seconds: start,
            end_seconds: end,
        };
        self.stored_loop_range = Some(self.loop_range);
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

fn one_second_overlay_range(project: &Project, start_seconds: f64) -> TimeRange {
    let bounds = TimeRange::for_project(project);
    let project_end = if bounds.end_seconds > bounds.start_seconds {
        bounds.end_seconds
    } else {
        bounds.start_seconds + BABEL_OVERLAY_DURATION_SECONDS
    };
    let max_start = (project_end - BABEL_OVERLAY_DURATION_SECONDS).max(bounds.start_seconds);
    let start = finite_seconds(start_seconds).clamp(bounds.start_seconds, max_start);
    TimeRange {
        start_seconds: start,
        end_seconds: start + BABEL_OVERLAY_DURATION_SECONDS,
    }
}
