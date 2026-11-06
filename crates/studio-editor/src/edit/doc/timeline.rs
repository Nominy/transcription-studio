//! Transport (play/pause, tick, seek, speed, zoom, wheel, overlay), Escape handling, waveform
//! pointer gestures (hover/press/release, drags, autoscroll, audition) and timeline-default
//! persistence in project metadata.

use std::collections::{BTreeMap, BTreeSet};

use studio_waveform::{WaveformHit, WaveformHitTarget};

use super::{EditError, EditResult, EditorDoc, Effect, Outcome, PointerModifiers, WaveformHover};
use crate::edit::format::zoom_after_wheel_pixels;
use crate::edit::structure::{
    AuditionKey, AutoscrollMode, CUT_PREVIEW_DRAG_THRESHOLD_PX, CUT_PREVIEW_HANDLE_HIT_PX,
    CutOverlayHit, MIN_CREATE_DRAG_DISTANCE_FRACTION, MIN_CREATED_REGION_DURATION_SECONDS,
    OverlayDrag, OverlayDragTarget, TIMELINE_AUTOSCROLL_HOLD_MOVE_THRESHOLD_FRACTION,
    TimelineAutoscroll, WaveformCreateDrag, WaveformDrag, audition_range, cut_overlay_hit,
};
use crate::timeline_model::TimelineSegmentPreview;
use crate::track_filter::TrackFilter;

const META_TIMELINE_ZOOM: &str = "studio.native.timeline.zoom";
const META_TIMELINE_SPEED: &str = "studio.native.timeline.speed";
const META_TIMELINE_TRACK_FILTER_LANE_ID: &str = "studio.native.timeline.trackFilterLaneId";
const META_TIMELINE_COLLAPSED_LANE_IDS: &str = "studio.native.timeline.collapsedLaneIds";
const META_TIMELINE_MUTED_LANE_IDS: &str = "studio.native.timeline.mutedLaneIds";
const META_TIMELINE_LANE_VERTICAL_ZOOM: &str = "studio.native.timeline.laneVerticalZoom";
/// Longest transport step accepted per tick (seconds).
pub const MAX_TICK_SECONDS: f64 = 0.25;

impl EditorDoc {
    // ---- transport --------------------------------------------------------------------

    /// Plain play/pause toggle (toolbar button).
    pub fn toggle_playback(&mut self) -> Outcome {
        let paused = self.session.is_playing;
        self.set_paused(paused)
    }

    /// Advances the transport by `elapsed_seconds` (clamped to [`MAX_TICK_SECONDS`]) while
    /// playing or autoscrolling: moves the playhead (looping/stopping at the end), follows the
    /// playing row, moves the ghost cursor, and pans for middle-button autoscroll.
    pub fn tick(&mut self, elapsed_seconds: f64) -> Outcome {
        let elapsed = if elapsed_seconds.is_finite() {
            elapsed_seconds.clamp(0.0, MAX_TICK_SECONDS)
        } else {
            0.0
        };
        if elapsed <= 0.0 {
            return Outcome::none();
        }
        let was_playing = self.session.is_playing;
        let previous_playhead = self.session.playhead_seconds;
        let previous_selection = self.session.selected_segment_id.clone();
        let bounds = self.session.timeline_bounds(self.workflow.project());
        self.session
            .advance_transport_with_bounds(self.workflow.project(), bounds, elapsed);
        self.sync_pending_restore_to_playback();

        if let Some(autoscroll) = self.pointer.autoscroll.as_ref()
            && let Some(delta) =
                autoscroll.pan_delta_seconds(self.session.viewport.duration_seconds(), elapsed)
        {
            self.session
                .pan_viewport_by_time_delta_with_bounds(bounds, delta);
        }

        let mut outcome = Outcome::none();
        if was_playing && !self.session.is_playing {
            outcome.effects.push(Effect::StopPlayback);
        } else if self.session.is_playing && self.session.playhead_seconds < previous_playhead {
            outcome.effects.push(Effect::RestartPlayback);
        }
        if self.session.selected_segment_id != previous_selection {
            self.touch();
        }
        if self.session.is_playing
            && let Some(selected) = self.session.selected_segment_id.clone()
            && self.playback_followed_segment_id.as_deref() != Some(selected.as_str())
        {
            self.playback_followed_segment_id = Some(selected.clone());
            outcome.effects.push(Effect::RevealSegment {
                segment_id: selected,
                center: false,
            });
        }
        outcome
    }

    /// Whether [`EditorDoc::tick`] should keep being called (playing or autoscrolling).
    pub fn needs_ticks(&self) -> bool {
        self.session.is_playing || self.pointer.autoscroll.is_some()
    }

    /// Moves the playhead (restarts audio when playing).
    pub fn seek(&mut self, time_seconds: f64) -> Outcome {
        self.session
            .seek_to_time(self.workflow.project(), time_seconds);
        self.touch();
        Outcome::none().extend(self.restart_if_playing())
    }

    /// Alt+X: playhead back 1s (not before the timeline start); the timeline recenters on it
    /// when it lands outside the visible window.
    pub fn rewind_one_second(&mut self) -> Outcome {
        let start = self
            .session
            .timeline_bounds(self.workflow.project())
            .start_seconds;
        let target = (self.session.playhead_seconds - 1.0).max(start);
        let outcome = self.seek(target);
        self.session.reveal_playhead(self.workflow.project());
        outcome.with_status(format!("Rewound playback to {target:.2}s."))
    }

    pub fn set_speed(&mut self, speed: f32) -> Outcome {
        self.session.set_speed(speed);
        self.persist_timeline_defaults();
        Outcome::none().with(Effect::SpeedChanged(self.session.speed))
    }

    /// Shift+1 / Shift+2: next faster / slower step of [`crate::edit::format::SPEED_OPTIONS`].
    pub fn step_speed(&mut self, direction: i32) -> Outcome {
        self.session.step_speed(direction);
        self.persist_timeline_defaults();
        let speed = self.session.speed;
        Outcome::status(format!("Playback speed set to {speed:.2}x."))
            .with(Effect::SpeedChanged(speed))
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.session.set_zoom(self.workflow.project(), zoom);
        self.persist_timeline_defaults();
    }

    /// The timeline's lane surface is `width_px` CSS px wide (see
    /// [`crate::editor_model::EditorSession::set_timeline_width`]). Returns whether that
    /// changed the viewport.
    pub fn set_timeline_width(&mut self, width_px: f64) -> bool {
        let viewport = self.session.viewport;
        self.session
            .set_timeline_width(self.workflow.project(), width_px);
        self.session.viewport != viewport
    }

    /// Wheel over the timeline: secondary = zoom around the hovered time, shift or horizontal
    /// delta = pan, plain vertical = nothing (the UI scrolls the page).
    pub fn timeline_wheel(&mut self, delta_x: f32, delta_y: f32) -> bool {
        let project = self.workflow.project();
        if self.pointer.modifiers.secondary {
            let zoom = zoom_after_wheel_pixels(self.session.zoom, delta_y);
            match self
                .pointer
                .waveform_hover
                .as_ref()
                .map(|h| h.hit.time_seconds)
            {
                Some(anchor) => self.session.set_zoom_around_time(project, zoom, anchor),
                None => self.session.set_zoom(project, zoom),
            }
            self.persist_timeline_defaults();
            return true;
        }
        let delta = if self.pointer.modifiers.shift {
            delta_y
        } else {
            delta_x
        };
        if delta == 0.0 {
            return false;
        }
        self.session.pan_viewport_by_wheel_pixels(project, delta);
        true
    }

    /// T: toggles the 1-second cut-preview overlay at the playhead (on the selected row's lane).
    pub fn toggle_overlay(&mut self) {
        self.session.toggle_overlay(self.workflow.project());
    }

    /// What a left press at `time_seconds` on `lane_id` grabs of the cut preview with the
    /// current modifiers: an edge within [`CUT_PREVIEW_HANDLE_HIT_PX`] resizes, secondary inside
    /// the range moves it, a plain body press is locked. `None` off the preview (or on another
    /// lane). Alt draws a new preview instead (see [`Self::press_waveform`]).
    pub fn cut_overlay_target(
        &self,
        lane_id: &str,
        time_seconds: f64,
    ) -> Option<OverlayDragTarget> {
        let range = self.session.overlay_on(lane_id)?;
        let handle_seconds = CUT_PREVIEW_HANDLE_HIT_PX * self.session.seconds_per_px();
        let hit = cut_overlay_hit(range, time_seconds, handle_seconds)?;
        let inside = range.start_seconds <= time_seconds && time_seconds <= range.end_seconds;
        Some(match hit {
            _ if self.pointer.modifiers.secondary && inside => OverlayDragTarget::Body,
            CutOverlayHit::StartHandle => OverlayDragTarget::StartHandle,
            CutOverlayHit::EndHandle => OverlayDragTarget::EndHandle,
            CutOverlayHit::Body => OverlayDragTarget::Locked,
        })
    }

    /// Escape outside text inputs: cancels autoscroll, else closes the find bar / edit
    /// annotation / add region dialog, else cancels the cut selection (ending its loop), else
    /// performs the helper play/pause toggle (pausing focuses the restored row). App-level
    /// dialogs are the UI's.
    pub fn escape(&mut self) -> Outcome {
        if self.cancel_autoscroll() {
            return Outcome::none();
        }
        if self.find.open {
            self.close_find();
            return Outcome::none();
        }
        if self.create_region.take().is_some() {
            return Outcome::none();
        }
        if let Some(overlay) = self.session.overlay.take() {
            self.pointer.overlay_drag = None;
            return if overlay.looping {
                Outcome::status("Selection cleared; stopped looping.")
            } else {
                Outcome::none()
            };
        }
        self.try_helper_unfocused_toggle().unwrap_or_else(|| {
            Outcome::status("Helper focus toggle unavailable: no selected segment.")
        })
    }

    // ---- pointer ----------------------------------------------------------------------

    pub fn set_modifiers(&mut self, modifiers: PointerModifiers) {
        self.pointer.modifiers = modifiers;
    }

    /// Q/W/E/R hold state (see [`crate::edit::keymap::audition_key_for`]).
    pub fn set_audition_key(&mut self, key: AuditionKey, pressed: bool) {
        if pressed {
            self.pointer.held_audition_keys.insert(key);
        } else {
            self.pointer.held_audition_keys.remove(&key);
        }
    }

    /// Window lost focus: forget held keys/modifiers and release captured drags.
    pub fn window_unfocused(&mut self) -> EditResult {
        self.pointer.modifiers = PointerModifiers::default();
        self.pointer.held_audition_keys.clear();
        self.release_pointer()
    }

    /// Pointer moved over a lane waveform; drives active drags on that lane.
    pub fn hover_waveform(&mut self, lane_id: &str, hit: WaveformHit) {
        let unchanged = self
            .pointer
            .waveform_hover
            .as_ref()
            .is_some_and(|hover| hover.lane_id == lane_id && hover.hit == hit);
        if unchanged && !self.drag_active_on(lane_id) {
            return;
        }
        let fraction = self.viewport_fraction(hit.time_seconds);
        self.pointer.waveform_hover = Some(WaveformHover {
            lane_id: lane_id.to_string(),
            hit: hit.clone(),
            fraction,
        });

        if let Some(drag) = self
            .pointer
            .overlay_drag
            .as_mut()
            .filter(|d| d.lane_id == lane_id)
        {
            if drag.target == OverlayDragTarget::Locked {
                return;
            }
            drag.has_moved |= (hit.time_seconds - drag.grab_time_seconds).abs()
                >= CUT_PREVIEW_DRAG_THRESHOLD_PX * self.session.seconds_per_px();
            let drag = drag.clone();
            let project = self.workflow.project();
            let bounds = self.session.timeline_bounds(project);
            if let Some((start, end)) = drag.range_at(bounds, hit.time_seconds) {
                self.session.set_overlay_range(project, lane_id, start, end);
            }
            return;
        }
        if let Some(autoscroll) = self
            .pointer
            .autoscroll
            .as_mut()
            .filter(|a| a.lane_id == lane_id)
        {
            autoscroll.target_fraction = fraction;
            if autoscroll.mode == AutoscrollMode::Held
                && (fraction - autoscroll.anchor_fraction).abs()
                    >= TIMELINE_AUTOSCROLL_HOLD_MOVE_THRESHOLD_FRACTION
            {
                autoscroll.moved_while_held = true;
            }
        }
        if let Some(drag) = self
            .pointer
            .waveform_drag
            .as_mut()
            .filter(|d| d.lane_id == lane_id)
        {
            drag.update(hit.time_seconds);
        }
        if let Some(drag) = self
            .pointer
            .create_drag
            .as_mut()
            .filter(|d| d.lane_id == lane_id)
        {
            if (fraction - drag.start_fraction).abs() >= MIN_CREATE_DRAG_DISTANCE_FRACTION {
                drag.has_moved = true;
            }
            drag.current_seconds = hit.time_seconds;
            drag.current_fraction = fraction;
        }
    }

    pub fn leave_waveform(&mut self, lane_id: &str) {
        if self
            .pointer
            .waveform_hover
            .as_ref()
            .is_some_and(|hover| hover.lane_id == lane_id)
        {
            self.pointer.waveform_hover = None;
        }
    }

    /// Left press on the hovered waveform point of `lane_id`:
    /// - Alt (not on a region handle): start drawing a cut preview on this lane;
    /// - on this lane's cut preview: an edge resizes it, secondary-drag inside moves it, else
    ///   locked (see [`Self::cut_overlay_target`]);
    /// - empty waveform: start a create-region drag;
    /// - region + secondary: split at the pointer (with Shift: smart split by word timings);
    /// - region + Shift or held Q/W/E/R: audition the region / boundary window;
    /// - region: select (no seek), move the playhead to the pointer; handles start a resize.
    pub fn press_waveform(&mut self, lane_id: &str) -> EditResult {
        let Some(hover) = self
            .pointer
            .waveform_hover
            .clone()
            .filter(|hover| hover.lane_id == lane_id)
        else {
            return Ok(Outcome::none());
        };
        let hit = hover.hit;
        let modifiers = self.pointer.modifiers;
        let on_handle = matches!(
            hit.target,
            WaveformHitTarget::RegionStartHandle | WaveformHitTarget::RegionEndHandle
        );

        if modifiers.alt && !modifiers.shift && !modifiers.secondary && !on_handle {
            self.session.overlay = None;
            self.pointer.overlay_drag = Some(OverlayDrag {
                lane_id: lane_id.to_string(),
                target: OverlayDragTarget::Create,
                grab_time_seconds: hit.time_seconds,
                original_start_seconds: hit.time_seconds,
                original_end_seconds: hit.time_seconds,
                has_moved: false,
            });
            return Ok(Outcome::none());
        }

        if let Some(target) = self.cut_overlay_target(lane_id, hit.time_seconds)
            && let Some(range) = self.session.overlay_on(lane_id)
        {
            self.pointer.overlay_drag = Some(OverlayDrag {
                lane_id: lane_id.to_string(),
                target,
                grab_time_seconds: hit.time_seconds,
                original_start_seconds: range.start_seconds,
                original_end_seconds: range.end_seconds,
                has_moved: false,
            });
            return Ok(Outcome::none());
        }

        let Some(segment_id) = hit.region_id.clone() else {
            if hit.target == WaveformHitTarget::Waveform && !modifiers.secondary {
                self.pointer.create_drag = Some(WaveformCreateDrag {
                    lane_id: lane_id.to_string(),
                    start_seconds: hit.time_seconds,
                    current_seconds: hit.time_seconds,
                    start_fraction: hover.fraction,
                    current_fraction: hover.fraction,
                    has_moved: false,
                });
            }
            return Ok(Outcome::none());
        };
        let segment = self.segment(&segment_id)?;

        if modifiers.secondary {
            return if modifiers.shift && !modifiers.alt {
                self.smart_split_segment_at(&segment_id, hit.time_seconds)
            } else {
                self.split_segment_at(&segment_id, hit.time_seconds)
            };
        }
        let audition = if modifiers.shift {
            Some((segment.start_seconds, segment.end_seconds))
        } else {
            AuditionKey::ALL
                .into_iter()
                .find(|key| self.pointer.held_audition_keys.contains(key))
                .and_then(|key| audition_range(&segment, key))
        };
        if let Some((start_seconds, end_seconds)) = audition {
            self.session.is_playing = false;
            self.session.playhead_seconds = start_seconds;
            self.touch();
            return Ok(Outcome::status(format!(
                "Auditioning {start_seconds:.3}s-{end_seconds:.3}s."
            ))
            .with(Effect::PlayRange {
                start_seconds,
                end_seconds,
            }));
        }

        let project = self.workflow.project();
        self.session
            .select_segment_without_seek(project, &segment_id);
        self.session.seek_to_time(project, hit.time_seconds);
        self.touch();
        if on_handle {
            self.pointer.waveform_drag = Some(WaveformDrag {
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
        Ok(Outcome::none().extend(self.restart_if_playing()))
    }

    /// Left release on `lane_id`: finishes the overlay drag, creates a region from a create
    /// drag (or seeks on a plain click), or commits a boundary resize as one undo step.
    pub fn release_waveform(&mut self, lane_id: &str) -> EditResult {
        if self
            .pointer
            .overlay_drag
            .as_ref()
            .is_some_and(|d| d.lane_id == lane_id)
        {
            self.pointer.overlay_drag = None;
            return Ok(Outcome::none());
        }
        if let Some(drag) = self.pointer.create_drag.take_if(|d| d.lane_id == lane_id) {
            if !drag.has_moved {
                return Ok(self.seek(drag.start_seconds));
            }
            return self.create_segment_from_drag(drag);
        }
        if let Some(drag) = self.pointer.waveform_drag.take_if(|d| d.lane_id == lane_id) {
            if !drag.preview_differs_from_original() {
                return Ok(self.select_segment_without_seek(&drag.segment_id));
            }
            let (segment_id, start, end) = (
                drag.segment_id.clone(),
                drag.preview_start_seconds,
                drag.preview_end_seconds,
            );
            let moved = self.apply("Waveform edit", |workflow| {
                let result = workflow.move_segment_boundary(&segment_id, start, end);
                workflow.finish_boundary_edit();
                result
            });
            moved?;
            if let Some(draft) = self.drafts.get_mut(&segment_id) {
                draft.replace_times(start, end);
            }
            self.mark_dirty();
            self.reconcile(true);
            return Ok(Outcome::status(format!(
                "Moved {} to {start:.3}s-{end:.3}s.",
                self.segment_label(&segment_id)
            )));
        }
        Ok(Outcome::none())
    }

    /// Global pointer release / capture loss: finishes any waveform drag and ends autoscroll.
    pub fn release_pointer(&mut self) -> EditResult {
        let lane_id = self
            .pointer
            .overlay_drag
            .as_ref()
            .map(|d| d.lane_id.clone())
            .or_else(|| self.pointer.create_drag.as_ref().map(|d| d.lane_id.clone()))
            .or_else(|| {
                self.pointer
                    .waveform_drag
                    .as_ref()
                    .map(|d| d.lane_id.clone())
            });
        self.cancel_autoscroll();
        match lane_id {
            Some(lane_id) => self.release_waveform(&lane_id),
            None => Ok(Outcome::none()),
        }
    }

    /// Preview region of an in-progress resize or create drag.
    pub fn timeline_preview(&self) -> Option<TimelineSegmentPreview> {
        self.pointer
            .waveform_drag
            .as_ref()
            .map(WaveformDrag::timeline_preview)
            .or_else(|| {
                self.pointer
                    .create_drag
                    .as_ref()
                    .and_then(WaveformCreateDrag::timeline_preview)
            })
    }

    /// Middle press on a lane: starts autoscroll (held mode); a second press stops it.
    pub fn press_autoscroll(&mut self, lane_id: &str) {
        if self
            .pointer
            .autoscroll
            .as_ref()
            .is_some_and(|a| a.lane_id == lane_id)
        {
            self.pointer.autoscroll = None;
            return;
        }
        let Some(hover) = self
            .pointer
            .waveform_hover
            .as_ref()
            .filter(|hover| hover.lane_id == lane_id)
        else {
            return;
        };
        let fraction = hover.fraction;
        self.pointer.overlay_drag = None;
        self.pointer.waveform_drag = None;
        self.pointer.create_drag = None;
        self.pointer.autoscroll = Some(TimelineAutoscroll {
            lane_id: lane_id.to_string(),
            anchor_fraction: fraction,
            target_fraction: fraction,
            mode: AutoscrollMode::Held,
            moved_while_held: false,
        });
    }

    /// Middle release: a held drag stops; a click without movement keeps scrolling ("click"
    /// mode) until the next middle press or Escape.
    pub fn release_autoscroll(&mut self) {
        let Some(autoscroll) = self.pointer.autoscroll.as_mut() else {
            return;
        };
        if autoscroll.mode != AutoscrollMode::Held {
            return;
        }
        if autoscroll.moved_while_held {
            self.pointer.autoscroll = None;
            return;
        }
        autoscroll.mode = AutoscrollMode::Click;
    }

    pub fn cancel_autoscroll(&mut self) -> bool {
        self.pointer.autoscroll.take().is_some()
    }

    fn drag_active_on(&self, lane_id: &str) -> bool {
        let pointer = &self.pointer;
        pointer
            .overlay_drag
            .as_ref()
            .is_some_and(|d| d.lane_id == lane_id)
            || pointer
                .waveform_drag
                .as_ref()
                .is_some_and(|d| d.lane_id == lane_id)
            || pointer
                .create_drag
                .as_ref()
                .is_some_and(|d| d.lane_id == lane_id)
            || pointer
                .autoscroll
                .as_ref()
                .is_some_and(|a| a.lane_id == lane_id)
    }

    fn viewport_fraction(&self, time_seconds: f64) -> f32 {
        let viewport = self.session.viewport;
        ((time_seconds - viewport.start_seconds) / viewport.duration_seconds().max(0.001))
            .clamp(0.0, 1.0) as f32
    }

    fn create_segment_from_drag(&mut self, drag: WaveformCreateDrag) -> EditResult {
        self.guard("Create")?;
        let (start, end) = drag.range();
        if end - start < MIN_CREATED_REGION_DURATION_SECONDS {
            return Err(EditError::Unavailable(
                "Created region is too short.".into(),
            ));
        }
        if let Some(command) = &self.environment().policy.region_creation_command {
            return Ok(Outcome::none().with(Effect::ExecuteCommand {
                command: command.clone(),
                arguments: serde_json::json!({
                    "lane_id": drag.lane_id, "start_seconds": start, "end_seconds": end
                }),
            }));
        }
        let lane_id = drag.lane_id.clone();
        let created = self.apply("Create", |workflow| {
            workflow.create_segment_in_lane(&lane_id, start, end)
        })?;
        self.mark_dirty();
        self.reconcile(true);
        self.drafts.refresh(self.workflow.project(), &created);
        let outcome = self.edit_segment_text(&created)?;
        let status = format!(
            "Created {} at {start:.3}s-{end:.3}s.",
            self.segment_label(&created)
        );
        Ok(outcome.with_status(status))
    }

    // ---- timeline defaults ------------------------------------------------------------

    /// Writes zoom, speed, track filter, collapsed/muted lanes and lane heights to project
    /// metadata (not an undoable edit; saved with the project).
    pub(super) fn persist_timeline_defaults(&mut self) {
        let session = &self.session;
        let filter_lane_id = match &session.track_filter {
            TrackFilter::All => String::new(),
            TrackFilter::Lane { id, .. } => id.clone(),
        };
        let vertical_zoom: BTreeMap<&String, f32> = session
            .lane_vertical_zoom
            .iter()
            .filter(|(_, zoom)| zoom.is_finite())
            .map(|(lane, zoom)| (lane, *zoom))
            .collect();
        let json = |value: serde_json::Result<String>, empty: &str| {
            value.unwrap_or_else(|_| empty.to_string())
        };
        let entries = [
            (
                META_TIMELINE_ZOOM.to_string(),
                format!("{:.6}", session.zoom),
            ),
            (
                META_TIMELINE_SPEED.to_string(),
                format!("{:.6}", session.speed),
            ),
            (
                META_TIMELINE_TRACK_FILTER_LANE_ID.to_string(),
                filter_lane_id,
            ),
            (
                META_TIMELINE_COLLAPSED_LANE_IDS.to_string(),
                json(serde_json::to_string(&session.collapsed_lane_ids), "[]"),
            ),
            (
                META_TIMELINE_MUTED_LANE_IDS.to_string(),
                json(serde_json::to_string(&session.muted_lane_ids), "[]"),
            ),
            (
                META_TIMELINE_LANE_VERTICAL_ZOOM.to_string(),
                json(serde_json::to_string(&vertical_zoom), "{}"),
            ),
        ];
        self.workflow.merge_project_metadata(entries);
        self.touch();
    }

    pub(super) fn hydrate_timeline_defaults(&mut self) {
        let project = self.workflow.project();
        let metadata = &project.metadata;
        let float = |key: &str| {
            metadata
                .get(key)
                .and_then(|value| value.trim().parse::<f32>().ok())
                .filter(|value| value.is_finite())
        };
        if let Some(zoom) = float(META_TIMELINE_ZOOM) {
            self.session.set_zoom(project, zoom);
        }
        if let Some(speed) = float(META_TIMELINE_SPEED) {
            self.session.set_speed(speed);
        }
        self.session.track_filter = metadata
            .get(META_TIMELINE_TRACK_FILTER_LANE_ID)
            .map(|value| value.trim())
            .filter(|lane_id| !lane_id.is_empty())
            .and_then(|lane_id| project.lanes.iter().find(|lane| lane.id == lane_id))
            .map(|lane| TrackFilter::lane(lane.id.clone(), lane.label.clone()))
            .unwrap_or(TrackFilter::All);
        let lane_ids: BTreeSet<&str> = project.lanes.iter().map(|lane| lane.id.as_str()).collect();
        let lane_set = |key: &str| -> BTreeSet<String> {
            metadata
                .get(key)
                .and_then(|value| serde_json::from_str::<BTreeSet<String>>(value).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|lane_id| lane_ids.contains(lane_id.as_str()))
                .collect()
        };
        self.session.collapsed_lane_ids = lane_set(META_TIMELINE_COLLAPSED_LANE_IDS);
        self.session.muted_lane_ids = lane_set(META_TIMELINE_MUTED_LANE_IDS);
        self.session.lane_vertical_zoom.clear();
        let heights = metadata
            .get(META_TIMELINE_LANE_VERTICAL_ZOOM)
            .and_then(|value| serde_json::from_str::<BTreeMap<String, f32>>(value).ok())
            .unwrap_or_default();
        for (lane_id, zoom) in heights {
            if zoom.is_finite() && lane_ids.contains(lane_id.as_str()) {
                self.session.set_lane_vertical_zoom(&lane_id, zoom);
            }
        }
        self.touch();
    }
}
