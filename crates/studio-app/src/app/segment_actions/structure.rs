use super::super::create_region::{
    CreateRegionMode, create_region_runtime_trace_line, create_region_trace_enabled,
};
#[cfg(test)]
use super::super::formatting::parse_seconds;
use super::super::{AuditionKey, CreateRegionDialog, StudioState};
use crate::app::state::{
    BABEL_TRACK_CELL_DOUBLE_CLICK_WINDOW, OverlayDrag, OverlayDragTarget, TimelineAutoscroll,
    TimelineAutoscrollMode, TrackCellPress, WaveformCreateDrag, WaveformDrag, WaveformHitHover,
};
use std::time::Instant;
use studio_app::waveform_region_label;
use studio_waveform::{WaveformHit, WaveformHitTarget};

const MIN_CREATED_REGION_DURATION_SECONDS: f64 = 0.001;
const MIN_CREATE_DRAG_DISTANCE_FRACTION: f32 = 0.006;
const BABEL_AUDITION_BUFFER_SECONDS: f64 = 0.5;
const BABEL_SPLIT_TAIL_GUARD_SECONDS: f64 = 0.1;
const CUT_PREVIEW_HANDLE_HIT_FRACTION: f32 = 0.008;
const CUT_PREVIEW_MIN_DURATION_SECONDS: f64 = 1.0;
const TIMELINE_AUTOSCROLL_HOLD_MOVE_THRESHOLD_FRACTION: f32 = 0.004;
const ROW_ACTION_TRACE_ENV: &str = "STUDIO_TRACE_ACTION_MENU";

fn split_text_by_babel_word_ratio(text: &str, ratio: f64) -> (String, String) {
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return (String::new(), String::new());
    }

    let clamped_ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut split_count = (words.len() as f64 * clamped_ratio).round() as usize;
    if clamped_ratio > 0.0 && clamped_ratio < 1.0 && words.len() > 1 {
        split_count = split_count.clamp(1, words.len() - 1);
    } else {
        split_count = split_count.min(words.len());
    }

    (
        words[..split_count].join(" "),
        words[split_count..].join(" "),
    )
}

fn anchored_cut_preview_bounds(
    bounds: studio_app::TimeRange,
    anchor_seconds: f64,
    current_seconds: f64,
) -> (f64, f64) {
    let min_duration = CUT_PREVIEW_MIN_DURATION_SECONDS.min(bounds.duration_seconds().max(0.0));
    let anchor = anchor_seconds.clamp(bounds.start_seconds, bounds.end_seconds);
    let current = if current_seconds.is_finite() {
        current_seconds.clamp(bounds.start_seconds, bounds.end_seconds)
    } else {
        anchor
    };

    if current <= anchor {
        let end_seconds = anchor;
        let mut start_seconds = current.min(anchor - min_duration).max(bounds.start_seconds);
        if end_seconds - start_seconds < min_duration {
            start_seconds = (end_seconds - min_duration).max(bounds.start_seconds);
        }
        return (start_seconds, end_seconds);
    }

    let start_seconds = anchor;
    let mut end_seconds = current.max(anchor + min_duration).min(bounds.end_seconds);
    if end_seconds - start_seconds < min_duration {
        end_seconds = (start_seconds + min_duration).min(bounds.end_seconds);
    }
    (start_seconds, end_seconds)
}

pub(in crate::app) fn row_structure_action_runtime_trace_line(
    action: &str,
    segment_id: &str,
    result_segment_id: Option<&str>,
    selected_segment_id: Option<&str>,
    segment_count: usize,
    is_dirty: bool,
    open_menu_segment_id: Option<&str>,
) -> String {
    format!(
        "Transcription Studio row action: action={action} segment={segment_id} result={} selected={} segments={segment_count} dirty={is_dirty} menu={}",
        result_segment_id.unwrap_or("-"),
        selected_segment_id.unwrap_or("-"),
        open_menu_segment_id.unwrap_or("none")
    )
}

fn row_structure_action_trace_enabled() -> bool {
    std::env::var(ROW_ACTION_TRACE_ENV).is_ok_and(|value| value != "0")
}

impl StudioState {
    fn close_transcript_action_menu_for_structure_command(&mut self) {
        self.with_editor_mut(|editor| {
            editor.open_actions_segment_id = None;
        });
    }

    fn trace_row_structure_action(
        &self,
        action: &str,
        segment_id: &str,
        result_segment_id: Option<&str>,
    ) {
        if !row_structure_action_trace_enabled() {
            return;
        }

        let segment_count = self
            .project()
            .map(|project| project.segments.len())
            .unwrap_or_default();
        let selected_segment_id = self
            .editor
            .as_ref()
            .and_then(|editor| editor.selected_segment_id.as_deref());
        let is_dirty = self.editor.as_ref().is_some_and(|editor| editor.is_dirty);
        let open_menu_segment_id = self
            .editor
            .as_ref()
            .and_then(|editor| editor.open_actions_segment_id.as_deref());

        eprintln!(
            "{}",
            row_structure_action_runtime_trace_line(
                action,
                segment_id,
                result_segment_id,
                selected_segment_id,
                segment_count,
                is_dirty,
                open_menu_segment_id,
            )
        );
    }

    #[cfg(test)]
    pub(in crate::app) fn split_segment(&mut self, segment_id: &str) {
        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .cloned()
        else {
            self.status = format!("Segment `{segment_id}` was not found.");
            return;
        };
        let Some(draft) = self.segment_drafts.get(segment_id) else {
            self.status = format!("Segment `{segment_id}` has no editable draft.");
            return;
        };
        let split_at = match parse_seconds("Split", &draft.split_seconds) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        self.split_segment_at(&segment, split_at);
    }

    pub(in crate::app) fn hover_waveform_hit(&mut self, lane_id: String, hit: WaveformHit) {
        if self.waveform_hover_is_unchanged(lane_id.as_str(), &hit)
            && !self.waveform_drag_is_active_for_lane(lane_id.as_str())
        {
            return;
        }

        let (region_label, region_tooltip) = self.waveform_region_hover_text(&hit);
        let fraction = self
            .editor
            .as_ref()
            .map(|editor| {
                let duration = editor.viewport.duration_seconds().max(0.001);
                ((hit.time_seconds - editor.viewport.start_seconds) / duration).clamp(0.0, 1.0)
                    as f32
            })
            .unwrap_or(0.0);
        self.waveform_hit_hover = Some(WaveformHitHover {
            lane_id: lane_id.clone(),
            hit: hit.clone(),
            fraction,
            region_label,
            region_tooltip,
        });

        if self
            .active_overlay_drag
            .as_ref()
            .is_some_and(|drag| drag.lane_id == lane_id)
        {
            self.apply_overlay_drag(&hit);
            return;
        }
        if self
            .active_timeline_autoscroll
            .as_ref()
            .is_some_and(|drag| drag.lane_id == lane_id)
        {
            self.update_timeline_autoscroll_target(&hit);
        }
        if self
            .active_waveform_drag
            .as_ref()
            .is_some_and(|drag| drag.lane_id == lane_id)
        {
            self.apply_waveform_drag(&hit);
        }
        if let Some(create_drag) = &mut self.active_waveform_create_drag
            && create_drag.lane_id == lane_id
        {
            if (fraction - create_drag.start_fraction).abs() >= MIN_CREATE_DRAG_DISTANCE_FRACTION {
                create_drag.has_moved = true;
            }
            create_drag.current_seconds = hit.time_seconds;
            create_drag.current_fraction = fraction;
        }
    }

    fn waveform_hover_is_unchanged(&self, lane_id: &str, hit: &WaveformHit) -> bool {
        self.waveform_hit_hover
            .as_ref()
            .is_some_and(|hover| hover.lane_id == lane_id && hover.hit == *hit)
    }

    fn waveform_drag_is_active_for_lane(&self, lane_id: &str) -> bool {
        self.active_overlay_drag
            .as_ref()
            .is_some_and(|drag| drag.lane_id == lane_id)
            || self
                .active_waveform_drag
                .as_ref()
                .is_some_and(|drag| drag.lane_id == lane_id)
            || self
                .active_waveform_create_drag
                .as_ref()
                .is_some_and(|drag| drag.lane_id == lane_id)
            || self
                .active_timeline_autoscroll
                .as_ref()
                .is_some_and(|drag| drag.lane_id == lane_id)
    }

    fn waveform_region_hover_text(&self, hit: &WaveformHit) -> (Option<String>, Option<String>) {
        let Some(region_id) = &hit.region_id else {
            return (None, None);
        };
        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(region_id))
        else {
            return (None, None);
        };

        (
            Some(waveform_region_label(&segment.text)),
            Some(segment.text.clone()),
        )
    }

    fn cut_preview_overlay_drag_target(
        &self,
        lane_id: &str,
        hit: &WaveformHit,
    ) -> Option<(OverlayDragTarget, studio_app::TimeRange)> {
        let editor = self.editor.as_ref()?;
        let overlay_range = editor.overlay_range?;
        if !editor.overlay_contains_time(hit.time_seconds) {
            return None;
        }
        if self.keyboard_modifiers.command() {
            return Some((OverlayDragTarget::Body, overlay_range));
        }

        let hover_fraction = self.waveform_hover_fraction(lane_id, hit, editor);
        let viewport_duration = editor.viewport.duration_seconds().max(0.001);
        let start_fraction = ((overlay_range.start_seconds - editor.viewport.start_seconds)
            / viewport_duration)
            .clamp(0.0, 1.0) as f32;
        let end_fraction = ((overlay_range.end_seconds - editor.viewport.start_seconds)
            / viewport_duration)
            .clamp(0.0, 1.0) as f32;
        let near_left = (hover_fraction - start_fraction).abs() <= CUT_PREVIEW_HANDLE_HIT_FRACTION;
        let near_right = (hover_fraction - end_fraction).abs() <= CUT_PREVIEW_HANDLE_HIT_FRACTION;

        let target = if near_left && !near_right {
            OverlayDragTarget::StartHandle
        } else if near_right {
            OverlayDragTarget::EndHandle
        } else {
            OverlayDragTarget::Locked
        };
        Some((target, overlay_range))
    }

    fn cut_preview_create_start_fraction(&self, lane_id: &str, hit: &WaveformHit) -> Option<f32> {
        if !self.keyboard_modifiers.alt()
            || self.keyboard_modifiers.shift()
            || self.keyboard_modifiers.command()
            || matches!(
                hit.target,
                WaveformHitTarget::RegionStartHandle | WaveformHitTarget::RegionEndHandle
            )
        {
            return None;
        }
        let editor = self.editor.as_ref()?;
        Some(self.waveform_hover_fraction(lane_id, hit, editor))
    }

    fn waveform_hover_fraction(
        &self,
        lane_id: &str,
        hit: &WaveformHit,
        editor: &studio_app::EditorSession,
    ) -> f32 {
        self.waveform_hit_hover
            .as_ref()
            .filter(|hover| hover.lane_id == lane_id)
            .map(|hover| hover.fraction)
            .unwrap_or_else(|| {
                let duration = editor.viewport.duration_seconds().max(0.001);
                ((hit.time_seconds - editor.viewport.start_seconds) / duration).clamp(0.0, 1.0)
                    as f32
            })
    }

    pub(in crate::app) fn press_waveform_hit(&mut self, lane_id: &str) {
        let Some(hit) = self
            .waveform_hit_hover
            .as_ref()
            .filter(|hover| hover.lane_id == lane_id)
            .map(|hover| hover.hit.clone())
        else {
            return;
        };

        if let Some(start_fraction) = self.cut_preview_create_start_fraction(lane_id, &hit) {
            self.with_editor_mut(|editor| {
                editor.overlay_range = None;
            });
            self.active_overlay_drag = Some(OverlayDrag {
                lane_id: lane_id.to_string(),
                target: OverlayDragTarget::Create,
                grab_time_seconds: hit.time_seconds,
                start_fraction,
                original_start_seconds: hit.time_seconds,
                original_end_seconds: hit.time_seconds,
                has_moved: false,
            });
            return;
        }

        if let Some((target, overlay_range)) = self.cut_preview_overlay_drag_target(lane_id, &hit) {
            let start_fraction = self
                .editor
                .as_ref()
                .map(|editor| self.waveform_hover_fraction(lane_id, &hit, editor))
                .unwrap_or_default();
            self.active_overlay_drag = Some(OverlayDrag {
                lane_id: lane_id.to_string(),
                target,
                grab_time_seconds: hit.time_seconds,
                start_fraction,
                original_start_seconds: overlay_range.start_seconds,
                original_end_seconds: overlay_range.end_seconds,
                has_moved: false,
            });
            return;
        }

        let Some(segment_id) = hit.region_id.clone() else {
            if matches!(hit.target, WaveformHitTarget::Waveform)
                && !self.keyboard_modifiers.command()
            {
                self.active_waveform_create_drag = Some(WaveformCreateDrag {
                    lane_id: lane_id.to_string(),
                    start_seconds: hit.time_seconds,
                    current_seconds: hit.time_seconds,
                    start_fraction: self
                        .waveform_hit_hover
                        .as_ref()
                        .filter(|hover| hover.lane_id == lane_id)
                        .map(|hover| hover.fraction)
                        .unwrap_or(0.0),
                    current_fraction: self
                        .waveform_hit_hover
                        .as_ref()
                        .filter(|hover| hover.lane_id == lane_id)
                        .map(|hover| hover.fraction)
                        .unwrap_or(0.0),
                    has_moved: false,
                });
            }
            return;
        };

        let Some(project) = self.project().cloned() else {
            self.status = format!("Segment `{segment_id}` was not found.");
            return;
        };
        let Some(segment) = project.segment(&segment_id).cloned() else {
            self.status = format!("Segment `{segment_id}` was not found.");
            return;
        };

        if self.keyboard_modifiers.command() {
            self.split_segment_at(&segment, hit.time_seconds);
            return;
        }

        if let Some((start_seconds, end_seconds)) = self.audition_range_for_segment(&segment) {
            self.play_audio_range(start_seconds, end_seconds);
            return;
        }

        self.with_editor_mut(|editor| {
            editor.select_segment_without_seek(&project, &segment_id);
            editor.seek_to_time(&project, hit.time_seconds);
        });
        self.restart_audio_playback_if_playing();

        if matches!(
            hit.target,
            WaveformHitTarget::RegionStartHandle | WaveformHitTarget::RegionEndHandle
        ) {
            self.active_waveform_drag = Some(WaveformDrag {
                lane_id: lane_id.to_string(),
                segment_id,
                target: hit.target,
                grab_time_seconds: hit.time_seconds,
                original_start_seconds: segment.start_seconds,
                original_end_seconds: segment.end_seconds,
                preview_start_seconds: segment.start_seconds,
                preview_end_seconds: segment.end_seconds,
                has_moved: false,
            });
        }
    }

    pub(in crate::app) fn press_timeline_autoscroll(&mut self, lane_id: &str) {
        if self
            .active_timeline_autoscroll
            .as_ref()
            .is_some_and(|autoscroll| autoscroll.lane_id == lane_id)
        {
            self.active_timeline_autoscroll = None;
            self.last_transport_tick = None;
            return;
        }

        let Some(hover) = self
            .waveform_hit_hover
            .as_ref()
            .filter(|hover| hover.lane_id == lane_id)
            .cloned()
        else {
            return;
        };

        self.active_overlay_drag = None;
        self.active_waveform_drag = None;
        self.active_waveform_create_drag = None;
        self.last_transport_tick = None;
        self.active_timeline_autoscroll = Some(TimelineAutoscroll {
            lane_id: lane_id.to_string(),
            anchor_fraction: hover.fraction,
            target_fraction: hover.fraction,
            mode: TimelineAutoscrollMode::Held,
            moved_while_held: false,
        });
    }

    pub(in crate::app) fn release_timeline_autoscroll(&mut self) -> bool {
        let Some(autoscroll) = self.active_timeline_autoscroll.as_mut() else {
            return false;
        };
        if autoscroll.mode != TimelineAutoscrollMode::Held {
            return true;
        }
        if autoscroll.moved_while_held {
            return self.cancel_timeline_autoscroll();
        }
        autoscroll.mode = TimelineAutoscrollMode::Click;
        autoscroll.moved_while_held = false;
        true
    }

    pub(in crate::app) fn cancel_timeline_autoscroll(&mut self) -> bool {
        let was_active = self.active_timeline_autoscroll.take().is_some();
        if was_active {
            self.last_transport_tick = None;
        }
        was_active
    }

    fn update_timeline_autoscroll_target(&mut self, _hit: &WaveformHit) {
        let Some(autoscroll) = self.active_timeline_autoscroll.as_mut() else {
            return;
        };
        let Some(hover) = self.waveform_hit_hover.as_ref() else {
            return;
        };
        autoscroll.target_fraction = hover.fraction;
        if autoscroll.mode == TimelineAutoscrollMode::Held
            && (autoscroll.target_fraction - autoscroll.anchor_fraction).abs()
                >= TIMELINE_AUTOSCROLL_HOLD_MOVE_THRESHOLD_FRACTION
        {
            autoscroll.moved_while_held = true;
        }
    }

    fn audition_range_for_segment(&self, segment: &studio_core::Segment) -> Option<(f64, f64)> {
        if self.keyboard_modifiers.shift() {
            return Some((segment.start_seconds, segment.end_seconds));
        }

        let audition_key = [
            AuditionKey::BeforeStart,
            AuditionKey::AfterStart,
            AuditionKey::BeforeEnd,
            AuditionKey::AfterEnd,
        ]
        .into_iter()
        .find(|key| self.held_audition_keys.contains(key))?;

        let range = match audition_key {
            AuditionKey::BeforeStart => (
                (segment.start_seconds - BABEL_AUDITION_BUFFER_SECONDS).max(0.0),
                segment.start_seconds,
            ),
            AuditionKey::AfterStart => (
                segment.start_seconds,
                (segment.start_seconds + BABEL_AUDITION_BUFFER_SECONDS).min(segment.end_seconds),
            ),
            AuditionKey::BeforeEnd => (
                (segment.end_seconds - BABEL_AUDITION_BUFFER_SECONDS).max(segment.start_seconds),
                segment.end_seconds,
            ),
            AuditionKey::AfterEnd => (
                segment.end_seconds,
                segment.end_seconds + BABEL_AUDITION_BUFFER_SECONDS,
            ),
        };

        (range.1 > range.0).then_some(range)
    }

    pub(in crate::app) fn release_waveform_hit(&mut self, lane_id: &str) {
        if self
            .active_overlay_drag
            .as_ref()
            .is_some_and(|drag| drag.lane_id == lane_id)
        {
            self.active_overlay_drag = None;
            return;
        }

        if self
            .active_waveform_create_drag
            .as_ref()
            .is_some_and(|drag| drag.lane_id == lane_id)
        {
            let create_drag = self.active_waveform_create_drag.take().unwrap();
            if !create_drag.has_moved {
                if let Some(project) = self.project().cloned() {
                    self.with_editor_mut(|editor| {
                        editor.seek_to_time(&project, create_drag.start_seconds)
                    });
                    self.restart_audio_playback_if_playing();
                }
                return;
            }
            self.create_segment_from_waveform_drag(create_drag);
            return;
        }

        if self
            .active_waveform_drag
            .as_ref()
            .is_some_and(|drag| drag.lane_id == lane_id)
        {
            let drag = self.active_waveform_drag.take().unwrap();
            if drag.preview_differs_from_original() {
                self.commit_waveform_drag(drag);
            } else {
                self.select_segment_without_seek(&drag.segment_id);
            }
        }
    }

    fn apply_overlay_drag(&mut self, hit: &WaveformHit) {
        let Some(mut drag) = self.active_overlay_drag.clone() else {
            return;
        };
        if drag.target == OverlayDragTarget::Locked {
            return;
        }
        let delta_seconds = hit.time_seconds - drag.grab_time_seconds;
        if !delta_seconds.is_finite() {
            return;
        }
        let Some(project) = self.project().cloned() else {
            return;
        };
        let bounds = studio_app::TimeRange::for_project(&project);
        let current_fraction = self
            .editor
            .as_ref()
            .map(|editor| self.waveform_hover_fraction(&drag.lane_id, hit, editor))
            .unwrap_or(drag.start_fraction);
        if let Some(active_drag) = &mut self.active_overlay_drag {
            let preview_moved = (current_fraction - active_drag.start_fraction).abs()
                >= MIN_CREATE_DRAG_DISTANCE_FRACTION;
            active_drag.has_moved |= preview_moved;
            drag.has_moved = active_drag.has_moved;
        }
        if drag.target == OverlayDragTarget::Create && !drag.has_moved {
            return;
        }

        let (start_seconds, end_seconds) = match drag.target {
            OverlayDragTarget::Create => {
                anchored_cut_preview_bounds(bounds, drag.grab_time_seconds, hit.time_seconds)
            }
            OverlayDragTarget::Body => {
                let duration = (drag.original_end_seconds - drag.original_start_seconds)
                    .max(CUT_PREVIEW_MIN_DURATION_SECONDS)
                    .min(
                        bounds
                            .duration_seconds()
                            .max(CUT_PREVIEW_MIN_DURATION_SECONDS),
                    );
                let mut start = drag.original_start_seconds + delta_seconds;
                let mut end = drag.original_end_seconds + delta_seconds;
                if start < bounds.start_seconds {
                    end += bounds.start_seconds - start;
                    start = bounds.start_seconds;
                }
                if end > bounds.end_seconds {
                    start -= end - bounds.end_seconds;
                    end = bounds.end_seconds;
                }
                if end - start < duration {
                    end = (start + duration).min(bounds.end_seconds);
                    start = (end - duration).max(bounds.start_seconds);
                }
                (start, end)
            }
            OverlayDragTarget::StartHandle => {
                anchored_cut_preview_bounds(bounds, drag.original_end_seconds, hit.time_seconds)
            }
            OverlayDragTarget::EndHandle => {
                anchored_cut_preview_bounds(bounds, drag.original_start_seconds, hit.time_seconds)
            }
            OverlayDragTarget::Locked => return,
        };

        self.with_editor_mut(|editor| {
            editor.set_overlay_range(&project, start_seconds, end_seconds);
        });
    }

    fn apply_waveform_drag(&mut self, hit: &WaveformHit) {
        let Some(drag) = self.active_waveform_drag.clone() else {
            return;
        };

        let (start_seconds, end_seconds) = match drag.target {
            WaveformHitTarget::RegionStartHandle => (hit.time_seconds, drag.original_end_seconds),
            WaveformHitTarget::RegionEndHandle => (drag.original_start_seconds, hit.time_seconds),
            WaveformHitTarget::RegionBody | WaveformHitTarget::Waveform => return,
        };

        if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds >= end_seconds {
            return;
        }

        if let Some(active_drag) = &mut self.active_waveform_drag {
            let preview_moved = active_drag.preview_start_seconds.to_bits()
                != start_seconds.to_bits()
                || active_drag.preview_end_seconds.to_bits() != end_seconds.to_bits();
            active_drag.preview_start_seconds = start_seconds;
            active_drag.preview_end_seconds = end_seconds;
            active_drag.has_moved |= preview_moved;
        }
    }

    fn commit_waveform_drag(&mut self, drag: WaveformDrag) {
        let segment_id = drag.segment_id.clone();
        let start_seconds = drag.preview_start_seconds;
        let end_seconds = drag.preview_end_seconds;
        if self.with_editable_workflow_mut("Waveform edit", |workflow| {
            workflow
                .move_segment_boundary(&segment_id, start_seconds, end_seconds)
                .map_err(|error| error.to_string())?;
            workflow.finish_boundary_edit();
            Ok(format!(
                "Moved `{segment_id}` to {start_seconds:.3}s-{end_seconds:.3}s."
            ))
        }) {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
        } else if let Ok(workflow) = &mut self.workflow {
            workflow.finish_boundary_edit();
        }
    }

    fn create_segment_from_waveform_drag(&mut self, drag: WaveformCreateDrag) {
        if self.block_read_only_edit("Create") {
            return;
        }

        let start_seconds = drag.start_seconds.min(drag.current_seconds).max(0.0);
        let end_seconds = drag.start_seconds.max(drag.current_seconds).max(0.0);
        if end_seconds - start_seconds < MIN_CREATED_REGION_DURATION_SECONDS {
            self.status = "Created region is too short.".to_string();
            return;
        }

        let Some(project) = self.project() else {
            self.status = "Create unavailable: no project loaded.".to_string();
            return;
        };
        let mut dialog = CreateRegionDialog::new_for_project(
            project,
            drag.lane_id.clone(),
            start_seconds,
            end_seconds,
        );
        if dialog.mode == CreateRegionMode::Transcription {
            let mut created_segment_id = None;
            if self.with_editable_workflow_mut("Create", |workflow| {
                let new_segment_id = workflow
                    .create_segment_in_lane(&drag.lane_id, start_seconds, end_seconds)
                    .map_err(|error| error.to_string())?;
                created_segment_id = Some(new_segment_id.clone());
                Ok(format!(
                    "Created `{new_segment_id}` at {start_seconds:.3}s-{end_seconds:.3}s."
                ))
            }) {
                let Some(created_segment_id) = created_segment_id else {
                    return;
                };
                dialog = dialog.with_created_segment_id(created_segment_id.clone());
                if create_region_trace_enabled() {
                    eprintln!(
                        "{}",
                        create_region_runtime_trace_line("create-open", &dialog)
                    );
                }
                self.create_region_dialog = Some(dialog);
                self.mark_dirty();
                self.reconcile_editor_with_project_preserving_viewport();
                self.refresh_segment_draft(&created_segment_id);
                self.select_segment_without_seek(&created_segment_id);
            }
            return;
        }

        let status = if dialog.mode == CreateRegionMode::Annotation {
            "Add New Annotation opened."
        } else {
            "Add New Region opened."
        };
        if create_region_trace_enabled() {
            eprintln!("{}", create_region_runtime_trace_line("open", &dialog));
        }
        self.create_region_dialog = Some(dialog);
        self.status = status.to_string();
    }

    fn split_segment_at(&mut self, segment: &studio_core::Segment, split_at: f64) {
        if self.block_read_only_edit("Split") {
            return;
        }

        if split_at <= segment.start_seconds || split_at >= segment.end_seconds {
            self.status = "Split must be inside the segment.".to_string();
            return;
        }
        if split_at >= segment.end_seconds - BABEL_SPLIT_TAIL_GUARD_SECONDS {
            self.status = "Split must be at least 100ms before the segment end.".to_string();
            return;
        }
        let segment_id = segment.id.clone();
        let split_segment_id = segment_id.clone();
        let source_text = self
            .segment_drafts
            .get(&segment_id)
            .map(|draft| draft.text.clone())
            .unwrap_or_else(|| segment.text.clone());
        let duration = segment.end_seconds - segment.start_seconds;
        let split_ratio = if duration.is_finite() && duration > 0.0 {
            (split_at - segment.start_seconds) / duration
        } else {
            0.0
        };
        let (left_text, right_text) = split_text_by_babel_word_ratio(&source_text, split_ratio);
        let mut new_selected_segment_id = None;
        if self.with_editable_workflow_mut("Split", |workflow| {
            let new_segment_id = workflow
                .split_segment_with_text(&segment_id, split_at, left_text, right_text)
                .map_err(|error| error.to_string())?;
            new_selected_segment_id = Some(new_segment_id.clone());
            Ok(format!(
                "Split `{segment_id}` at {split_at:.3}s into `{new_segment_id}`."
            ))
        }) {
            self.refresh_segment_draft(&split_segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
            if let Some(new_segment_id) = new_selected_segment_id {
                self.select_segment_without_seek(&new_segment_id);
            }
        }
    }

    pub(in crate::app) fn merge_segment_above(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        self.close_transcript_action_menu_for_structure_command();
        if self.block_read_only_edit("Merge") {
            return;
        }
        self.clear_low_confidence_review_state(&segment_id);
        if self.with_workflow_mut("Merge", |workflow| {
            workflow
                .merge_with_previous(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Merged `{segment_id}` into the previous segment on its track."
            ))
        }) {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project();
            self.trace_row_structure_action("merge-above", &segment_id, None);
        }
    }

    pub(in crate::app) fn merge_segment_below(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        self.close_transcript_action_menu_for_structure_command();
        if self.block_read_only_edit("Merge") {
            return;
        }
        self.clear_low_confidence_review_state(&segment_id);
        if self.with_workflow_mut("Merge", |workflow| {
            workflow
                .merge_with_next(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Merged `{segment_id}` with the next segment on its track."
            ))
        }) {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project();
            self.trace_row_structure_action("merge-below", &segment_id, None);
        }
    }

    pub(in crate::app) fn delete_segment(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        self.close_transcript_action_menu_for_structure_command();
        if self.block_read_only_edit("Delete") {
            return;
        }
        self.clear_low_confidence_review_state(&segment_id);
        if self.with_workflow_mut("Delete", |workflow| {
            workflow
                .delete_segment(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!("Deleted `{segment_id}`."))
        }) {
            self.segment_drafts.remove(&segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project();
            self.trace_row_structure_action("delete", &segment_id, None);
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn resolve_low_confidence(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        if self.with_editable_workflow_mut("Low-confidence review", |workflow| {
            workflow
                .resolve_low_confidence(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Marked low-confidence words reviewed for `{segment_id}`."
            ))
        }) {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project();
        }
    }

    pub(in crate::app) fn create_segment_after(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        self.close_transcript_action_menu_for_structure_command();
        let mut created_segment_id = None;
        if self.with_editable_workflow_mut("Create", |workflow| {
            let new_segment_id = workflow
                .create_segment_after(&segment_id)
                .map_err(|error| error.to_string())?;
            created_segment_id = Some(new_segment_id.clone());
            Ok(format!("Created `{new_segment_id}` after `{segment_id}`."))
        }) {
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
            if let Some(created_segment_id) = created_segment_id {
                self.select_segment_without_seek(&created_segment_id);
                self.trace_row_structure_action(
                    "create-after",
                    &segment_id,
                    Some(&created_segment_id),
                );
            }
        }
    }

    pub(in crate::app) fn create_segment_before(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        self.close_transcript_action_menu_for_structure_command();
        let mut created_segment_id = None;
        if self.with_editable_workflow_mut("Create", |workflow| {
            let new_segment_id = workflow
                .create_segment_before(&segment_id)
                .map_err(|error| error.to_string())?;
            created_segment_id = Some(new_segment_id.clone());
            Ok(format!("Created `{new_segment_id}` before `{segment_id}`."))
        }) {
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
            if let Some(created_segment_id) = created_segment_id {
                self.select_segment_without_seek(&created_segment_id);
                self.trace_row_structure_action(
                    "create-before",
                    &segment_id,
                    Some(&created_segment_id),
                );
            }
        }
    }

    pub(in crate::app) fn begin_segment_track_edit(&mut self, segment_id: &str) {
        let current_lane_id = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .map(|segment| segment.lane_id.clone());
        if let (Some(current_lane_id), Some(draft)) =
            (current_lane_id, self.segment_drafts.get_mut(segment_id))
        {
            draft.track_edit.begin(&current_lane_id);
        }
    }

    pub(in crate::app) fn press_segment_track_cell(&mut self, segment_id: &str) {
        self.press_segment_track_cell_at(segment_id, Instant::now());
    }

    pub(in crate::app) fn press_segment_track_cell_at(&mut self, segment_id: &str, at: Instant) {
        self.select_segment(segment_id);
        let is_double_click = self.last_track_cell_press.as_ref().is_some_and(|last| {
            last.segment_id == segment_id
                && at
                    .checked_duration_since(last.at)
                    .is_some_and(|elapsed| elapsed <= BABEL_TRACK_CELL_DOUBLE_CLICK_WINDOW)
        });
        self.last_track_cell_press = Some(TrackCellPress {
            segment_id: segment_id.to_string(),
            at,
        });
        if is_double_click {
            self.begin_segment_track_edit(segment_id);
            self.last_track_cell_press = None;
        }
    }

    pub(in crate::app) fn set_segment_track_select_open(
        &mut self,
        segment_id: &str,
        is_open: bool,
    ) {
        let current_lane_id = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .map(|segment| segment.lane_id.clone());
        if let (Some(current_lane_id), Some(draft)) =
            (current_lane_id, self.segment_drafts.get_mut(segment_id))
        {
            draft.track_edit.set_open(is_open, &current_lane_id);
        }
    }

    pub(in crate::app) fn select_segment_track(&mut self, segment_id: &str, lane_id: &str) {
        if let Some(draft) = self.segment_drafts.get_mut(segment_id) {
            draft.track_edit.select(lane_id);
        }
        self.change_segment_track(segment_id, lane_id);
    }

    pub(in crate::app) fn change_segment_track(&mut self, segment_id: &str, lane_id: &str) {
        let segment_id = segment_id.to_string();
        let lane_id = lane_id.to_string();
        if self.with_editable_workflow_mut("Track change", |workflow| {
            workflow
                .change_segment_track(&segment_id, &lane_id)
                .map_err(|error| error.to_string())?;
            Ok(format!("Moved `{segment_id}` to `{lane_id}`."))
        }) {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
            self.reconcile_transcript_ownership_to_visible();
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn set_segment_annotation_type(
        &mut self,
        segment_id: &str,
        annotation_type: &str,
    ) {
        let segment_id = segment_id.to_string();
        let annotation_type = annotation_type.to_string();
        if self.with_editable_workflow_mut("Annotation type change", |workflow| {
            workflow
                .set_segment_annotation_type(&segment_id, &annotation_type)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Set `{segment_id}` annotation type to `{annotation_type}`."
            ))
        }) {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project();
        }
    }
}
