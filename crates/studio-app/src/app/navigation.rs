use std::time::Instant;

use studio_app::{TrackFilter, TranscriptAnnotationKind};

use super::StudioState;
use super::state::{BABEL_TRACK_CELL_DOUBLE_CLICK_WINDOW, HelperFocusRestore, TranscriptRowPress};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TranscriptRowPressOutcome {
    pub(super) scroll_target: Option<f32>,
    pub(super) sought: bool,
}

impl StudioState {
    pub(super) fn select_segment(&mut self, segment_id: &str) -> Option<f32> {
        let Some(project) = self.project().cloned() else {
            return None;
        };
        self.with_editor_mut(|editor| editor.select_segment(&project, segment_id));
        self.scroll_transcript_to_segment_if_needed(&project, segment_id)
    }

    pub(super) fn select_segment_without_seek(&mut self, segment_id: &str) -> Option<f32> {
        let Some(project) = self.project().cloned() else {
            return None;
        };
        self.with_editor_mut(|editor| editor.select_segment_without_seek(&project, segment_id));
        self.scroll_transcript_to_segment_if_needed(&project, segment_id)
    }

    pub(super) fn press_transcript_row_body(
        &mut self,
        segment_id: &str,
    ) -> TranscriptRowPressOutcome {
        self.press_transcript_row_body_at(segment_id, Instant::now())
    }

    pub(in crate::app) fn press_transcript_row_body_at(
        &mut self,
        segment_id: &str,
        at: Instant,
    ) -> TranscriptRowPressOutcome {
        let is_double_click = self.last_transcript_row_press.as_ref().is_some_and(|last| {
            last.segment_id == segment_id
                && at
                    .checked_duration_since(last.at)
                    .is_some_and(|elapsed| elapsed <= BABEL_TRACK_CELL_DOUBLE_CLICK_WINDOW)
        });
        self.last_transcript_row_press = Some(TranscriptRowPress {
            segment_id: segment_id.to_string(),
            at,
        });
        let scroll_target = if is_double_click {
            self.last_transcript_row_press = None;
            self.select_segment(segment_id)
        } else {
            self.select_segment_without_seek(segment_id)
        };
        TranscriptRowPressOutcome {
            scroll_target,
            sought: is_double_click,
        }
    }

    pub(super) fn select_adjacent_segment(&mut self, direction: i32) -> Option<f32> {
        let Some(project) = self.project().cloned() else {
            return None;
        };
        self.with_editor_mut(|editor| editor.navigate_visible_segment(&project, direction));
        let selected_segment_id = self
            .editor
            .as_ref()
            .and_then(|editor| editor.selected_segment_id.clone())?;
        self.sync_pending_helper_focus_restore_to_segment(&selected_segment_id);
        self.scroll_transcript_to_segment_centered(&project, &selected_segment_id)
    }

    pub(super) fn select_adjacent_focused_segment(&mut self, direction: i32) -> Option<f32> {
        let Some(project) = self.project().cloned() else {
            self.status = "Helper focus navigation unavailable: no project loaded.".to_string();
            return None;
        };
        let Some(editor) = self.editor.as_ref() else {
            self.status = "Helper focus navigation unavailable: no editor session.".to_string();
            return None;
        };

        let mut visible_segments = project
            .segments
            .iter()
            .filter(|segment| match &editor.track_filter {
                TrackFilter::All => true,
                TrackFilter::Lane { id, .. } => id == &segment.lane_id,
            })
            .filter(|segment| !editor.collapsed_lane_ids.contains(&segment.lane_id))
            .filter(|segment| TranscriptAnnotationKind::from_segment(segment).is_transcription())
            .collect::<Vec<_>>();
        visible_segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));

        if visible_segments.is_empty() {
            self.status = "Helper focus navigation unavailable: no visible segments.".to_string();
            return None;
        }

        let current_index = editor
            .selected_segment_id
            .as_deref()
            .and_then(|segment_id| {
                visible_segments
                    .iter()
                    .position(|segment| segment.id == segment_id)
            });
        let base_index = current_index.unwrap_or(0);
        let step = if direction < 0 { -1 } else { 1 };
        let next_index =
            (base_index as i32 + step).clamp(0, visible_segments.len() as i32 - 1) as usize;
        if current_index.is_some() && next_index == base_index {
            self.status = "Helper focus navigation reached transcript edge.".to_string();
            return None;
        }

        let segment_id = visible_segments[next_index].id.clone();
        let playhead_seconds = editor.playhead_seconds;
        self.with_editor_mut(|editor| editor.select_segment_without_seek(&project, &segment_id));

        let restore = HelperFocusRestore {
            segment_id: segment_id.clone(),
            blur_playhead_seconds: playhead_seconds,
            restore_playhead_seconds: playhead_seconds,
            restore_offset: 0,
            synthetic: false,
        };
        self.helper_focused_segment_id = Some(segment_id.clone());
        self.helper_focus_restore_pending = Some(restore.clone());
        self.apply_helper_focus_restore_cursor(&restore);
        self.helper_last_focus_restore = Some(restore);
        self.status = if direction < 0 {
            "Helper focus moved to previous segment from start.".to_string()
        } else {
            "Helper focus moved to next segment from start.".to_string()
        };
        self.scroll_transcript_to_segment_centered(&project, &segment_id)
    }
}
