//! Selection and transcript navigation, track filter / lane visibility, speaker workflow, and
//! the helper focus workflow (Space/Escape toggles with caret restore and ghost cursor).

use studio_core::Segment;

use super::{EditError, EditResult, EditorDoc, Effect, Outcome};
use crate::edit::draft::TextSelection;
use crate::edit::navigate::{
    self, GhostCursor, HelperFocusRestore, proportional_restore_offset, segment_id_at_playhead,
    speaker_workflow_lane, speaker_workflow_lane_ids, visible_transcription_rows,
};
use crate::track_filter::TrackFilter;

impl EditorDoc {
    // ---- selection --------------------------------------------------------------------

    /// Selects a row and seeks the playhead to it.
    pub fn select_segment(&mut self, segment_id: &str) -> Outcome {
        self.session
            .select_segment(self.workflow.project(), segment_id);
        self.touch();
        Outcome::none()
            .with(Effect::RevealSegment {
                segment_id: segment_id.to_string(),
                center: false,
            })
            .extend(self.restart_if_playing())
    }

    /// Selects a row without moving the playhead.
    pub fn select_segment_without_seek(&mut self, segment_id: &str) -> Outcome {
        self.session
            .select_segment_without_seek(self.workflow.project(), segment_id);
        self.touch();
        Outcome::none().with(Effect::RevealSegment {
            segment_id: segment_id.to_string(),
            center: false,
        })
    }

    /// Alt+Click on a row's text (Helper word seek): moves the playhead to the start of the
    /// clicked word through the lane's word timings, selecting the row without scrolling.
    /// Playback state is unchanged (a playing transport continues from the word). Without a
    /// timestamp for the word nothing moves: there is no ratio estimate.
    pub fn seek_to_word(&mut self, segment_id: &str, char_offset: usize) -> EditResult {
        let segment = self.segment(segment_id)?;
        let Some(lane) = self.word_timings.lane(&segment.lane_id) else {
            return Ok(Outcome::status(
                "Word seek: no word timestamps for this track yet. Run Gold Draft to create them.",
            ));
        };
        let Some(time) = crate::timing::timestamp_at_offset(
            self.drafts.text_or(&segment),
            lane,
            crate::timing::TimeRange::new(segment.start_seconds, segment.end_seconds),
            char_offset,
        ) else {
            return Ok(Outcome::status(
                "Word seek: no word in this row matches its timestamps.",
            ));
        };
        let project = self.workflow.project();
        self.session
            .select_segment_without_seek(project, segment_id);
        self.session.seek_to_time(project, time);
        self.touch();
        Ok(
            Outcome::status(format!("Seeked to the word at {time:.3}s."))
                .extend(self.restart_if_playing()),
        )
    }

    /// Click on a row body: selects without seeking; a double click also seeks to the row.
    pub fn press_row(&mut self, segment_id: &str, click_count: usize) -> Outcome {
        if click_count >= 2 {
            self.select_segment(segment_id)
        } else {
            self.select_segment_without_seek(segment_id)
        }
    }

    /// Left/Right: previous/next visible row with seek (skips collapsed lanes).
    pub fn select_adjacent_segment(&mut self, direction: i32) -> Outcome {
        let previous_playhead = self.session.playhead_seconds;
        self.session
            .navigate_visible_segment(self.workflow.project(), direction);
        self.touch();
        let mut outcome = Outcome::none();
        if previous_playhead.to_bits() != self.session.playhead_seconds.to_bits() {
            outcome = outcome.extend(self.restart_if_playing());
        }
        let Some(selected) = self.session.selected_segment_id.clone() else {
            return outcome;
        };
        self.sync_pending_restore_to_segment(&selected);
        outcome.with(Effect::RevealSegment {
            segment_id: selected,
            center: true,
        })
    }

    /// Shift+Left/Right: select the previous/next visible row (no seek) and focus its text
    /// from the start.
    pub fn focus_adjacent_segment(&mut self, direction: i32) -> EditResult {
        let rows = visible_transcription_rows(self.workflow.project(), &self.session);
        if rows.is_empty() {
            return Err(EditError::Unavailable(
                "Helper focus navigation unavailable: no visible segments.".into(),
            ));
        }
        let playhead = self.session.playhead_seconds;
        let current = self
            .session
            .selected_segment_id
            .as_deref()
            .and_then(|id| rows.iter().position(|(row_id, _)| row_id == id));
        let next = match current {
            Some(index) => {
                let next = step_index(index, direction, rows.len());
                if next == index {
                    return Err(EditError::Unavailable(
                        "Helper focus navigation reached transcript edge.".into(),
                    ));
                }
                next
            }
            None if direction < 0 => rows
                .iter()
                .rposition(|(_, start)| *start <= playhead)
                .unwrap_or(0),
            None => rows
                .iter()
                .position(|(_, start)| *start >= playhead)
                .unwrap_or(rows.len() - 1),
        };
        let segment_id = rows[next].0.clone();
        self.session
            .select_segment_without_seek(self.workflow.project(), &segment_id);
        let restore = HelperFocusRestore {
            segment_id: segment_id.clone(),
            blur_playhead_seconds: playhead,
            restore_playhead_seconds: playhead,
            restore_offset: 0,
            synthetic: false,
            lane_locked: false,
        };
        let left = self.set_text_focus(Some(&segment_id))?;
        self.helper.pending = Some(restore.clone());
        let focus = self.apply_restore(restore);
        self.touch();
        Ok(Outcome::status(if direction < 0 {
            "Helper focus moved to previous segment from start."
        } else {
            "Helper focus moved to next segment from start."
        })
        .extend(left.effects)
        .with(Effect::RevealSegment {
            segment_id,
            center: true,
        })
        .with(focus))
    }

    /// Click on a timeline region's tooltip: selects the row without seeking and focuses its
    /// text editor with the caret at the end of its text.
    pub fn edit_segment_text(&mut self, segment_id: &str) -> EditResult {
        self.guard("Text editing")?;
        let draft = self
            .drafts
            .get_mut(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        let caret = draft.text().chars().count();
        draft.set_selection(TextSelection::caret(caret));
        self.session
            .select_segment_without_seek(self.workflow.project(), segment_id);
        let left = self.set_text_focus(Some(segment_id))?;
        self.touch();
        Ok(left
            .with(Effect::RevealSegment {
                segment_id: segment_id.to_string(),
                center: false,
            })
            .with(Effect::FocusText {
                segment_id: segment_id.to_string(),
                caret,
            }))
    }

    /// Tab (Helper `toggleGhostCursorLane`): moves the cursor to the next speaker lane with
    /// visible rows, onto that lane's row for the cursor's time ([`navigate::lane_row_at`]).
    /// While a row's text editor has focus (`focused_segment`), focus moves there with the
    /// caret at the text spoken at that time (the playhead, clamped into the focused row).
    /// Otherwise the pending ghost cursor moves (at the playhead) and playback keeps it on
    /// that lane.
    pub fn switch_ghost_cursor_lane(&mut self, focused_segment: Option<&str>) -> EditResult {
        let ghost = self
            .helper
            .pending
            .as_ref()
            .filter(|_| self.helper.focused_segment_id.is_none())
            .map(|pending| pending.segment_id.as_str());
        let source_id = focused_segment
            .or(ghost)
            .ok_or_else(|| EditError::Unavailable("No cursor to move to another lane.".into()))?;
        let project = self.workflow.project();
        let session = &self.session;
        let source = project
            .segment(source_id)
            .ok_or_else(|| EditError::SegmentNotFound(source_id.to_string()))?;
        let playhead = session.playhead_seconds;
        let seconds = match focused_segment {
            Some(_) => playhead.clamp(source.start_seconds, source.end_seconds),
            None => playhead,
        };
        let lanes: Vec<&str> = project
            .lanes
            .iter()
            .map(|lane| lane.id.as_str())
            .filter(|lane_id| {
                project.segments.iter().any(|segment| {
                    segment.lane_id == *lane_id && navigate::segment_is_visible(session, segment)
                })
            })
            .collect();
        if lanes.len() < 2 {
            return Err(EditError::Unavailable(
                "No other speaker lane to move the cursor to.".into(),
            ));
        }
        let current = lanes.iter().position(|id| *id == source.lane_id);
        let lane_id = lanes[current.map_or(0, |index| (index + 1) % lanes.len())];
        let target = navigate::lane_row_at(project, session, lane_id, seconds)
            .ok_or_else(|| EditError::Unavailable("No row to move the cursor to.".into()))?;
        let caret = focused_segment.map(|_| self.spoken_offset_at(target, seconds, 0));
        let segment_id = target.id.clone();

        self.session
            .select_segment_without_seek(self.workflow.project(), &segment_id);
        let mut outcome = Outcome::none().with(Effect::RevealSegment {
            segment_id: segment_id.clone(),
            center: true,
        });
        if let Some(caret) = caret {
            outcome = outcome.extend(self.set_text_focus(Some(&segment_id))?.effects);
            self.helper.pending = None;
            outcome = outcome.with(self.apply_restore(HelperFocusRestore {
                segment_id,
                blur_playhead_seconds: playhead,
                restore_playhead_seconds: playhead,
                restore_offset: caret,
                synthetic: false,
                lane_locked: false,
            }));
        } else {
            self.sync_pending_restore_to_segment(&segment_id);
            if let Some(pending) = self.helper.pending.as_mut() {
                pending.lane_locked = true;
            }
        }
        self.touch();
        Ok(outcome)
    }

    /// Previous/next row with a lint problem (seeks; restarts playback when playing).
    pub fn navigate_problem_rows(&mut self, direction: i32) -> Outcome {
        self.session
            .navigate_problem_rows(self.workflow.project(), direction);
        self.touch();
        let mut outcome = Outcome::none().extend(self.restart_if_playing());
        if let Some(segment_id) = self.session.selected_segment_id.clone() {
            outcome.effects.insert(
                0,
                Effect::RevealSegment {
                    segment_id,
                    center: false,
                },
            );
        }
        outcome
    }

    // ---- filter / lanes ---------------------------------------------------------------

    pub fn set_track_filter(&mut self, filter: TrackFilter) {
        self.session.track_filter = filter;
        self.reconcile_selection_to_visible();
        self.persist_timeline_defaults();
    }

    pub fn toggle_lane_collapse(&mut self, lane_id: &str) {
        self.session.toggle_lane_collapse(lane_id);
        self.reconcile_selection_to_visible();
        self.persist_timeline_defaults();
    }

    pub fn toggle_lane_mute(&mut self, lane_id: &str) -> Outcome {
        self.session.toggle_lane_mute(lane_id);
        self.touch();
        self.persist_timeline_defaults();
        Outcome::none().extend(self.restart_if_playing())
    }

    pub fn set_lane_vertical_zoom(&mut self, lane_id: &str, vertical_zoom: f32) {
        self.session.set_lane_vertical_zoom(lane_id, vertical_zoom);
        self.touch();
        self.persist_timeline_defaults();
    }

    /// Alt+1/Alt+2: show only the "Speaker N" lane; collapse and mute the other speaker lane.
    pub fn switch_speaker_workflow_lane(&mut self, label: &str) -> EditResult {
        let lane = speaker_workflow_lane(self.workflow.project(), label).ok_or_else(|| {
            EditError::Unavailable(format!(
                "Speaker workflow unavailable: lane {label} not found."
            ))
        })?;
        let (lane_id, lane_label) = (lane.id.clone(), lane.label.clone());
        let speaker_lanes = speaker_workflow_lane_ids(self.workflow.project());
        self.session.track_filter = TrackFilter::lane(lane_id.clone(), lane_label);
        self.session.collapsed_lane_ids.remove(&lane_id);
        self.session.muted_lane_ids.remove(&lane_id);
        for other in speaker_lanes.into_iter().filter(|id| *id != lane_id) {
            self.session.collapsed_lane_ids.insert(other.clone());
            self.session.muted_lane_ids.insert(other);
        }
        self.reconcile_selection_to_visible();
        self.persist_timeline_defaults();
        Ok(Outcome::status(format!(
            "Speaker workflow switched to {label}."
        )))
    }

    /// Alt+`: show all tracks and unmute/expand both speaker lanes.
    pub fn reset_speaker_workflow(&mut self) -> Outcome {
        for lane_id in speaker_workflow_lane_ids(self.workflow.project()) {
            self.session.collapsed_lane_ids.remove(&lane_id);
            self.session.muted_lane_ids.remove(&lane_id);
        }
        self.session.track_filter = TrackFilter::All;
        self.touch();
        self.persist_timeline_defaults();
        Outcome::status("Speaker workflow reset: showing all tracks and unmuting speakers.")
    }

    // ---- helper focus workflow --------------------------------------------------------

    /// Space outside text inputs: pausing restores the caret into the row under the playhead
    /// (focus effect); starting plays and leaves a ghost cursor in the row at the playhead or
    /// the selected row. A selected row the playhead is outside of plays from its start.
    /// Falls back to a plain toggle when there is no row.
    pub fn helper_toggle_playback(&mut self) -> Outcome {
        self.try_helper_unfocused_toggle()
            .unwrap_or_else(|| self.toggle_playback())
    }

    /// Escape inside a row's text editor: pauses when playing (caret stays); otherwise blurs
    /// the editor (committing its typed text), leaves a ghost cursor at the spoken offset
    /// (word timings, else playhead-proportional) and plays.
    pub fn transcript_escape(&mut self, segment_id: &str) -> EditResult {
        if self.workflow.project().segment(segment_id).is_none() {
            return Err(EditError::SegmentNotFound(segment_id.to_string()));
        }
        self.session
            .select_segment_without_seek(self.workflow.project(), segment_id);
        let focused = self.set_text_focus(Some(segment_id))?;
        self.touch();
        if self.session.is_playing {
            return Ok(self
                .set_paused(true)
                .extend(focused.effects)
                .with_status("Helper focus toggle paused playback from focused transcript row."));
        }
        let committed = self.set_text_focus(None)?;
        self.helper.pending = self.restore_for_segment(segment_id, false, None);
        Ok(Outcome::none()
            .with(Effect::BlurText)
            .extend(focused.effects)
            .extend(committed.effects)
            .extend(self.set_paused(false).effects))
    }

    /// Ghost caret for a row while a restore is pending and the row is not focused. While
    /// playing it tracks the playhead through the lane's word timings when available, else
    /// proportionally (snapped to word starts); never before the restore baseline.
    pub fn ghost_cursor(&self, segment_id: &str) -> Option<GhostCursor> {
        let restore = self.helper.pending.as_ref()?;
        if restore.segment_id != segment_id
            || self.helper.focused_segment_id.as_deref() == Some(segment_id)
        {
            return None;
        }
        let segment = self.workflow.project().segment(segment_id)?;
        let char_offset = if self.session.is_playing {
            self.spoken_offset(segment, restore.restore_offset)
        } else {
            restore
                .restore_offset
                .min(self.drafts.text_or(segment).chars().count())
        };
        Some(GhostCursor {
            segment_id: segment_id.to_string(),
            char_offset,
        })
    }

    /// `char` offset of the row's (draft) text spoken at the playhead (see
    /// [`Self::spoken_offset_at`]). Restore baselines and the playing ghost both use it, so a
    /// baseline never runs ahead of the timed ghost.
    fn spoken_offset(&self, segment: &Segment, baseline_offset: usize) -> usize {
        self.spoken_offset_at(segment, self.session.playhead_seconds, baseline_offset)
    }

    /// `char` offset of the row's (draft) text spoken at `seconds`: through the lane's word
    /// timings when they match the text, else proportional (snapped to word starts); never
    /// before `baseline_offset`.
    fn spoken_offset_at(&self, segment: &Segment, seconds: f64, baseline_offset: usize) -> usize {
        let text = self.drafts.text_or(segment);
        self.word_timings
            .lane(&segment.lane_id)
            .and_then(|lane| {
                crate::timing::offset_at_time(
                    text,
                    lane,
                    crate::timing::TimeRange::new(segment.start_seconds, segment.end_seconds),
                    seconds,
                )
            })
            .map(|offset| offset.max(baseline_offset).min(text.chars().count()))
            .unwrap_or_else(|| proportional_restore_offset(text, segment, seconds, baseline_offset))
    }

    /// Marks a row's text editor as focused/unfocused by the user (click into/out of a row).
    /// The row that loses focus commits its typed text ([`Self::commit_text_draft`]).
    pub fn set_text_focus(&mut self, segment_id: Option<&str>) -> EditResult {
        let previous = self.helper.focused_segment_id.take();
        self.helper.focused_segment_id = segment_id.map(str::to_string);
        match previous {
            Some(previous) if segment_id != Some(previous.as_str()) => {
                self.commit_text_draft(&previous)
            }
            _ => Ok(Outcome::none()),
        }
    }

    /// A row's text editor lost keyboard focus (Enter, a click elsewhere): commits its typed
    /// text and releases the helper focus when the row still held it (another grid's editor
    /// may already hold it).
    pub fn blur_text(&mut self, segment_id: &str) -> EditResult {
        if self.helper.focused_segment_id.as_deref() == Some(segment_id) {
            self.helper.focused_segment_id = None;
        }
        self.commit_text_draft(segment_id)
    }

    /// Releases the helper focus where no `Result` flows: a refused commit becomes the status.
    fn release_text_focus(&mut self) -> Outcome {
        self.set_text_focus(None)
            .unwrap_or_else(|error| Outcome::status(error.to_string()))
    }

    pub(super) fn try_helper_unfocused_toggle(&mut self) -> Option<Outcome> {
        if self.session.is_playing {
            let focus = self.restore_helper_focus_for_current_playhead();
            let outcome = self.set_paused(true);
            return Some(match focus {
                Ok(focus) => outcome.extend(focus.effects),
                Err(status) => outcome.with_status(status),
            });
        }
        let segment_id = match segment_id_at_playhead(self.workflow.project(), &self.session) {
            Some(segment_id) => segment_id,
            // The playhead is in no row: the selected row plays from its start.
            None => {
                let segment_id = self.session.selected_segment_id.clone()?;
                let start_seconds = self.workflow.project().segment(&segment_id)?.start_seconds;
                self.session
                    .seek_to_time(self.workflow.project(), start_seconds);
                self.touch();
                segment_id
            }
        };
        self.helper.pending = self.restore_for_segment(&segment_id, true, None);
        let released = self.release_text_focus();
        Some(released.extend(self.set_paused(false).effects))
    }

    pub(super) fn set_paused(&mut self, paused: bool) -> Outcome {
        if paused {
            if !self.session.is_playing {
                return Outcome::none();
            }
            self.session.is_playing = false;
            return Outcome::none().with(Effect::StopPlayback);
        }
        if self.session.is_playing {
            return Outcome::none();
        }
        let bounds = self.session.timeline_bounds(self.workflow.project());
        self.session
            .prepare_viewport_for_playback_with_bounds(bounds);
        self.session.is_playing = true;
        Outcome::none().with(Effect::StartPlayback)
    }

    /// Pausing focuses where the ghost cursor is shown — its row (synced to the playhead first,
    /// as the next playback tick would) and offset, timed through word timings when the lane
    /// has them (Helper `getGhostCursorTarget`). Without a shown ghost, the row under the
    /// playhead gets the spoken offset at the playhead.
    fn restore_helper_focus_for_current_playhead(&mut self) -> Result<Outcome, String> {
        self.sync_pending_restore_to_playback();
        let project = self.workflow.project();
        let session = &self.session;
        let visible = |id: &String| {
            project
                .segment(id)
                .is_some_and(|segment| navigate::segment_is_visible(session, segment))
        };
        let pending = self.helper.pending.clone();
        let ghost = pending
            .as_ref()
            .filter(|restore| visible(&restore.segment_id))
            .and_then(|restore| self.ghost_cursor(&restore.segment_id));
        let segment_id = match &ghost {
            Some(ghost) => ghost.segment_id.clone(),
            None => segment_id_at_playhead(project, session)
                .or_else(|| {
                    pending
                        .as_ref()
                        .map(|restore| restore.segment_id.clone())
                        .filter(visible)
                })
                .or_else(|| session.selected_segment_id.clone().filter(visible))
                .ok_or_else(|| {
                    "Helper focus restore failed: no segment at playback position.".to_string()
                })?,
        };
        let blur = pending
            .as_ref()
            .map(|restore| restore.blur_playhead_seconds)
            .unwrap_or(session.playhead_seconds);
        let synthetic = pending.as_ref().is_some_and(|restore| restore.synthetic);
        let mut restore = self
            .restore_for_segment(&segment_id, synthetic, Some(blur))
            .ok_or_else(|| {
                "Helper focus restore failed: that segment no longer exists.".to_string()
            })?;
        if let Some(ghost) = ghost {
            restore.restore_offset = ghost.char_offset;
        }
        self.session
            .select_segment_without_seek(self.workflow.project(), &segment_id);
        let left = self
            .set_text_focus(Some(&segment_id))
            .map_err(|error| error.to_string())?;
        self.helper.pending = None;
        self.touch();
        Ok(left.with(self.apply_restore(restore)))
    }

    fn apply_restore(&mut self, restore: HelperFocusRestore) -> Effect {
        if let Some(draft) = self.drafts.get_mut(&restore.segment_id) {
            draft.set_selection(TextSelection::caret(restore.restore_offset));
        }
        let effect = Effect::FocusText {
            segment_id: restore.segment_id.clone(),
            caret: restore.restore_offset,
        };
        self.helper.last = Some(restore);
        effect
    }

    pub(super) fn sync_pending_restore_to_segment(&mut self, segment_id: &str) {
        let Some(previous) = self.helper.pending.clone() else {
            return;
        };
        if previous.segment_id == segment_id {
            return;
        }
        if let Some(restore) = self.restore_for_segment(
            segment_id,
            previous.synthetic,
            Some(previous.blur_playhead_seconds),
        ) {
            self.helper.pending = Some(HelperFocusRestore {
                lane_locked: previous.lane_locked,
                ..restore
            });
            self.helper.focused_segment_id = None;
        }
    }

    /// During playback, moves the pending ghost cursor to the row that should own it.
    pub(super) fn sync_pending_restore_to_playback(&mut self) {
        if !self.session.is_playing || self.helper.focused_segment_id.is_some() {
            return;
        }
        let Some(pending) = self.helper.pending.as_ref() else {
            return;
        };
        let Some(target) =
            navigate::playback_follow_target(self.workflow.project(), &self.session, pending)
        else {
            return;
        };
        self.session
            .select_segment_without_seek(self.workflow.project(), &target);
        self.sync_pending_restore_to_segment(&target);
    }

    /// Restore target in `segment_id` at the spoken offset (see [`Self::spoken_offset`]).
    fn restore_for_segment(
        &self,
        segment_id: &str,
        synthetic: bool,
        blur_playhead_seconds: Option<f64>,
    ) -> Option<HelperFocusRestore> {
        let segment = self.workflow.project().segment(segment_id)?;
        let playhead = self.session.playhead_seconds;
        Some(HelperFocusRestore {
            segment_id: segment_id.to_string(),
            blur_playhead_seconds: blur_playhead_seconds.unwrap_or(playhead),
            restore_playhead_seconds: playhead,
            restore_offset: self.spoken_offset(segment, 0),
            synthetic,
            lane_locked: false,
        })
    }
}

fn step_index(index: usize, direction: i32, len: usize) -> usize {
    if direction < 0 {
        index.saturating_sub(1)
    } else {
        (index + 1).min(len - 1)
    }
}
