use super::EditorSession;
use crate::timeline_model::TimeRange;
use studio_core::{Project, Segment};

impl EditorSession {
    pub fn select_segment(&mut self, project: &Project, segment_id: &str) {
        if let Some(segment) = project.segment(segment_id) {
            self.selected_segment_id = Some(segment_id.to_string());
            self.playhead_seconds = segment.start_seconds;
            self.recenter_viewport(self.timeline_bounds(project));
        }
    }

    pub fn select_segment_without_seek(&mut self, project: &Project, segment_id: &str) {
        if project.segment(segment_id).is_some() {
            self.selected_segment_id = Some(segment_id.to_string());
        }
    }

    pub fn reconcile_selection_to_visible(&mut self, project: &Project) {
        if self
            .selected_segment_id
            .as_deref()
            .and_then(|segment_id| project.segment(segment_id))
            .is_some_and(|segment| !self.is_transcription_segment_visible(segment))
        {
            self.selected_segment_id = None;
        }
    }

    pub(super) fn select_segment_at_playhead(&mut self, project: &Project) {
        if self
            .selected_segment_id
            .as_deref()
            .and_then(|segment_id| project.segment(segment_id))
            .is_some_and(|segment| segment_contains_playhead(segment, self.playhead_seconds))
        {
            return;
        }

        let Some(segment) = project
            .segments
            .iter()
            .filter(|segment| segment_contains_playhead(segment, self.playhead_seconds))
            .min_by(|left, right| {
                left.start_seconds
                    .total_cmp(&right.start_seconds)
                    .then_with(|| left.id.cmp(&right.id))
            })
        else {
            return;
        };

        self.selected_segment_id = Some(segment.id.clone());
    }

    pub fn navigate_problem_rows(&mut self, project: &Project, direction: i32) {
        let problem_rows = self.problem_rows(project);
        if problem_rows.is_empty() {
            self.problem_row_index = 0;
            return;
        }

        let current_index = problem_rows
            .iter()
            .position(|row| Some(row.segment_id.as_str()) == self.selected_segment_id.as_deref())
            .unwrap_or(self.problem_row_index.min(problem_rows.len() - 1));
        let next_index = if direction < 0 {
            if current_index == 0 {
                problem_rows.len() - 1
            } else {
                current_index - 1
            }
        } else {
            (current_index + 1) % problem_rows.len()
        };
        let next_segment_id = problem_rows[next_index].segment_id.clone();
        self.problem_row_index = next_index;
        self.select_segment(project, &next_segment_id);
    }

    pub fn navigate_visible_segment(&mut self, project: &Project, direction: i32) {
        let mut visible_segments = project
            .segments
            .iter()
            .filter(|segment| self.is_transcription_segment_visible(segment))
            .collect::<Vec<_>>();
        visible_segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));

        let bounds = self.timeline_bounds(project);
        if visible_segments.is_empty() {
            let target = if direction < 0 {
                bounds.start_seconds
            } else {
                bounds.end_seconds
            };
            self.seek_without_active_segment(bounds, target);
            return;
        }

        let selected_index = self.selected_segment_id.as_deref().and_then(|segment_id| {
            visible_segments
                .iter()
                .position(|segment| segment.id == segment_id)
        });
        if let Some(current_index) = selected_index.filter(|index| {
            (visible_segments[*index].start_seconds - self.playhead_seconds).abs() <= 0.05
        }) {
            let next_index = if direction < 0 {
                current_index.checked_sub(1)
            } else {
                (current_index + 1 < visible_segments.len()).then_some(current_index + 1)
            };
            if let Some(next_index) = next_index {
                self.select_segment(project, &visible_segments[next_index].id);
            } else {
                let target = if direction < 0 {
                    bounds.start_seconds
                } else {
                    bounds.end_seconds
                };
                self.seek_without_active_segment(bounds, target);
            }
            return;
        }

        if direction < 0 {
            let threshold = self.playhead_seconds - 0.05;
            if let Some(segment) = visible_segments
                .iter()
                .rev()
                .find(|segment| segment.start_seconds < threshold)
            {
                self.select_segment(project, &segment.id);
            } else {
                self.seek_without_active_segment(bounds, bounds.start_seconds);
            }
            return;
        }

        let threshold = self.playhead_seconds + 0.05;
        if let Some(segment) = visible_segments
            .iter()
            .find(|segment| segment.start_seconds > threshold)
        {
            self.select_segment(project, &segment.id);
        } else {
            self.seek_without_active_segment(bounds, bounds.end_seconds);
        }
    }

    pub fn reconcile_project(&mut self, project: &Project) {
        let previous_viewport = self.viewport;
        if self
            .selected_segment_id
            .as_deref()
            .is_some_and(|segment_id| project.segment(segment_id).is_none())
        {
            self.selected_segment_id = project
                .segments
                .iter()
                .find(|segment| self.is_transcription_segment_visible(segment))
                .map(|segment| segment.id.clone());
        }
        if self
            .open_actions_segment_id
            .as_deref()
            .is_some_and(|segment_id| project.segment(segment_id).is_none())
        {
            self.open_actions_segment_id = None;
        }
        self.asserted_warning_keys.retain(|key| {
            project
                .segments
                .iter()
                .any(|segment| key.starts_with(&segment.id))
        });
        self.viewport =
            viewport_preserving_position(previous_viewport, self.timeline_bounds(project));
    }

    pub fn reconcile_project_preserving_viewport(&mut self, project: &Project) {
        let previous_viewport = self.viewport;
        self.reconcile_project(project);
        self.viewport =
            viewport_preserving_position(previous_viewport, self.timeline_bounds(project));
    }

    fn seek_without_active_segment(&mut self, bounds: TimeRange, time_seconds: f64) {
        self.selected_segment_id = None;
        self.playhead_seconds = time_seconds.clamp(bounds.start_seconds, bounds.end_seconds);
        self.recenter_viewport(bounds);
    }

    fn is_transcription_segment_visible(&self, segment: &Segment) -> bool {
        self.track_filter.matches_lane(&segment.lane_id)
            && !self.collapsed_lane_ids.contains(&segment.lane_id)
            && self.environment.is_text_segment(segment)
    }
}

fn viewport_preserving_position(viewport: TimeRange, bounds: TimeRange) -> TimeRange {
    let bounds_duration = (bounds.end_seconds - bounds.start_seconds).max(0.0);
    if bounds_duration <= 0.0 {
        return bounds;
    }

    let duration = viewport.duration_seconds().min(bounds_duration);
    if duration >= bounds_duration {
        return bounds;
    }

    let max_start = bounds.end_seconds - duration;
    let start = if viewport.start_seconds.is_finite() {
        viewport
            .start_seconds
            .clamp(bounds.start_seconds, max_start)
    } else {
        bounds.start_seconds
    };
    TimeRange {
        start_seconds: start,
        end_seconds: start + duration,
    }
}

fn segment_contains_playhead(segment: &Segment, playhead_seconds: f64) -> bool {
    segment.start_seconds <= playhead_seconds && playhead_seconds < segment.end_seconds
}
