use iced::widget::text_editor;
use studio_core::Segment;

use super::formatting::format_seconds_for_edit;
use super::time_edit::{TimeEditState, TimeField, start_time_cell_edit};

#[derive(Debug)]
pub(super) struct SegmentDraft {
    pub(super) start_seconds: String,
    pub(super) end_seconds: String,
    #[cfg(test)]
    pub(super) split_seconds: String,
    pub(super) text: String,
    pub(super) text_content: text_editor::Content,
    text_undo_stack: Vec<String>,
    text_redo_stack: Vec<String>,
    pub(super) time_edit: TimeEditState,
    pub(super) track_edit: TrackCellEditState,
}

impl SegmentDraft {
    pub(super) fn from_segment(segment: &Segment) -> Self {
        let mut draft = Self {
            start_seconds: format_seconds_for_edit(segment.start_seconds),
            end_seconds: format_seconds_for_edit(segment.end_seconds),
            #[cfg(test)]
            split_seconds: format_seconds_for_edit(
                (segment.start_seconds + segment.end_seconds) / 2.0,
            ),
            text_content: text_editor::Content::new(),
            text: String::new(),
            text_undo_stack: Vec::new(),
            text_redo_stack: Vec::new(),
            time_edit: TimeEditState::default(),
            track_edit: TrackCellEditState::from_lane(&segment.lane_id),
        };
        draft.replace_text(segment.text.clone());
        draft
    }

    pub(super) fn time_value(&self, field: TimeField) -> &str {
        match field {
            TimeField::Start => &self.start_seconds,
            TimeField::End => &self.end_seconds,
        }
    }

    pub(super) fn set_time_value(&mut self, field: TimeField, value: String) {
        match field {
            TimeField::Start => self.start_seconds = value,
            TimeField::End => self.end_seconds = value,
        }
    }

    pub(super) fn replace_times(&mut self, start_seconds: f64, end_seconds: f64) {
        self.start_seconds = format_seconds_for_edit(start_seconds);
        self.end_seconds = format_seconds_for_edit(end_seconds);
    }

    pub(super) fn start_time_edit(&mut self, field: TimeField, value: f64) {
        self.time_edit = start_time_cell_edit(field, value);
    }

    pub(super) fn time_error(&self, field: TimeField) -> Option<&str> {
        if self.time_edit.active_field == Some(field) {
            self.time_edit.error.as_deref()
        } else {
            None
        }
    }

    pub(super) fn replace_text(&mut self, text: String) {
        self.set_text_without_history(text);
        self.text_undo_stack.clear();
        self.text_redo_stack.clear();
    }

    pub(super) fn replace_text_from_user_edit(&mut self, text: String) {
        let before = self.text.clone();
        self.set_text_without_history(text);
        if self.text != before {
            self.text_undo_stack.push(before);
            self.text_redo_stack.clear();
        }
    }

    pub(super) fn perform_text_edit(&mut self, action: text_editor::Action) {
        let before = self.text.clone();
        let is_edit = action.is_edit();
        self.text_content.perform(action);
        self.text = self.editor_text();
        if is_edit && self.text != before {
            self.text_undo_stack.push(before);
            self.text_redo_stack.clear();
        }
    }

    pub(super) fn set_cursor_to_char_offset(&mut self, char_offset: usize) {
        self.text_content = text_editor::Content::with_text(&self.text);
        let cursor_offset = char_offset.min(self.text.chars().count());
        for _ in 0..cursor_offset {
            self.text_content
                .perform(text_editor::Action::Move(text_editor::Motion::Right));
        }
    }

    pub(super) fn undo_text_edit(&mut self) -> bool {
        let Some(previous) = self.text_undo_stack.pop() else {
            return false;
        };
        let current = self.text.clone();
        self.text_redo_stack.push(current);
        self.set_text_without_history(previous);
        true
    }

    pub(super) fn redo_text_edit(&mut self) -> bool {
        let Some(next) = self.text_redo_stack.pop() else {
            return false;
        };
        let current = self.text.clone();
        self.text_undo_stack.push(current);
        self.set_text_without_history(next);
        true
    }

    pub(super) fn editor_text(&self) -> String {
        normalized_editor_text(&self.text_content)
    }

    pub(super) fn text_editor_content_height(&self) -> f32 {
        let explicit_lines = self.text.lines().count().max(1);
        let approximate_wrapped_lines = self.text.chars().count().div_ceil(120).max(1);
        let rows = explicit_lines.max(approximate_wrapped_lines).max(1);
        24.0 + rows.saturating_sub(1) as f32 * 18.0
    }

    fn set_text_without_history(&mut self, text: String) {
        self.text = text;
        self.text_content = text_editor::Content::with_text(&self.text);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrackCellEditState {
    pub(super) is_editing: bool,
    pub(super) select_open: bool,
    pub(super) draft_lane_id: String,
    pub(super) should_focus: bool,
}

impl TrackCellEditState {
    fn from_lane(lane_id: &str) -> Self {
        Self {
            is_editing: false,
            select_open: false,
            draft_lane_id: lane_id.to_string(),
            should_focus: false,
        }
    }

    pub(super) fn begin(&mut self, current_lane_id: &str) {
        self.is_editing = true;
        self.select_open = true;
        self.draft_lane_id = current_lane_id.to_string();
        self.should_focus = true;
    }

    pub(super) fn set_open(&mut self, is_open: bool, current_lane_id: &str) {
        self.is_editing = is_open;
        self.select_open = is_open;
        self.should_focus = false;
        if !is_open {
            self.draft_lane_id = current_lane_id.to_string();
        }
    }

    pub(super) fn select(&mut self, lane_id: &str) {
        self.is_editing = false;
        self.select_open = false;
        self.draft_lane_id = lane_id.to_string();
        self.should_focus = false;
    }
}

fn normalized_editor_text(content: &text_editor::Content) -> String {
    let mut text = content.text();
    if text.ends_with('\n') {
        text.pop();
    }
    text
}
