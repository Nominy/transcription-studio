use iced::widget::text_editor;
use studio_core::Segment;

use super::formatting::format_seconds_for_edit;
use super::time_edit::{TimeEditState, TimeField, start_time_cell_edit};
// Each segment retains at most 100 snapshots in either direction and at most
// 2 MiB of their UTF-8 contents. A newest snapshot larger than the byte budget
// is kept by itself so the immediately reversible edit remains available.
const TEXT_HISTORY_MAX_ENTRIES: usize = 100;
const TEXT_HISTORY_MAX_RETAINED_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn push_bounded_history<T>(
    history: &mut Vec<T>,
    entry: T,
    max_entries: usize,
    max_retained_bytes: usize,
    retained_utf8_bytes: impl Fn(&T) -> usize,
) {
    debug_assert!(max_entries > 0);
    history.push(entry);

    let mut retained_bytes = history
        .iter()
        .map(&retained_utf8_bytes)
        .fold(0usize, usize::saturating_add);
    while history.len() > 1 && (history.len() > max_entries || retained_bytes > max_retained_bytes)
    {
        let removed = history.remove(0);
        retained_bytes = retained_bytes.saturating_sub(retained_utf8_bytes(&removed));
    }
}

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
            self.push_text_history(before, true);
            self.text_redo_stack.clear();
        }
    }

    pub(super) fn perform_text_edit(&mut self, action: text_editor::Action) {
        let before = self.text.clone();
        let is_edit = action.is_edit();
        self.text_content.perform(action);
        self.text = self.editor_text();
        if is_edit && self.text != before {
            self.push_text_history(before, true);
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

    pub(super) fn can_undo_text_edit_to(&self, previous: &str) -> bool {
        self.text_undo_stack
            .last()
            .is_some_and(|candidate| candidate == previous)
    }

    pub(super) fn can_redo_text_edit_to(&self, next: &str) -> bool {
        self.text_redo_stack
            .last()
            .is_some_and(|candidate| candidate == next)
    }

    pub(super) fn undo_text_edit(&mut self) -> bool {
        let Some(previous) = self.text_undo_stack.pop() else {
            return false;
        };
        let current = self.text.clone();
        self.push_text_history(current, false);
        self.set_text_without_history(previous);
        true
    }

    pub(super) fn redo_text_edit(&mut self) -> bool {
        let Some(next) = self.text_redo_stack.pop() else {
            return false;
        };
        let current = self.text.clone();
        self.push_text_history(current, true);
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

    fn push_text_history(&mut self, snapshot: String, undo: bool) {
        let history = if undo {
            &mut self.text_undo_stack
        } else {
            &mut self.text_redo_stack
        };
        push_bounded_history(
            history,
            snapshot,
            TEXT_HISTORY_MAX_ENTRIES,
            TEXT_HISTORY_MAX_RETAINED_BYTES,
            String::len,
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn draft_with_text(text: &str) -> SegmentDraft {
        let mut draft = SegmentDraft {
            start_seconds: String::new(),
            end_seconds: String::new(),
            split_seconds: String::new(),
            text: String::new(),
            text_content: text_editor::Content::new(),
            text_undo_stack: Vec::new(),
            text_redo_stack: Vec::new(),
            time_edit: TimeEditState::default(),
            track_edit: TrackCellEditState::from_lane("lane"),
        };
        draft.replace_text(text.to_string());
        draft
    }

    fn retained_bytes(history: &[String]) -> usize {
        history.iter().map(String::len).sum()
    }

    #[test]
    fn thousands_of_edits_keep_only_the_latest_undo_chain_and_redo_it() {
        let edit_count = TEXT_HISTORY_MAX_ENTRIES + 2_000;
        let mut draft = draft_with_text("edit-0");
        for index in 1..=edit_count {
            draft.replace_text_from_user_edit(format!("edit-{index}"));
        }

        assert_eq!(draft.text_undo_stack.len(), TEXT_HISTORY_MAX_ENTRIES);
        assert!(retained_bytes(&draft.text_undo_stack) <= TEXT_HISTORY_MAX_RETAINED_BYTES);

        for expected in (edit_count - TEXT_HISTORY_MAX_ENTRIES..edit_count).rev() {
            assert!(draft.undo_text_edit());
            assert_eq!(draft.text, format!("edit-{expected}"));
        }
        assert!(!draft.undo_text_edit());

        for expected in edit_count - TEXT_HISTORY_MAX_ENTRIES + 1..=edit_count {
            assert!(draft.redo_text_edit());
            assert_eq!(draft.text, format!("edit-{expected}"));
        }
        assert!(!draft.redo_text_edit());
    }

    #[test]
    fn text_history_is_bounded_by_utf8_bytes() {
        let mut draft = draft_with_text("initial");
        for index in 0..80 {
            draft.replace_text_from_user_edit(format!("{index}:{}", "я".repeat(32 * 1024)));
        }

        assert!(draft.text_undo_stack.len() < TEXT_HISTORY_MAX_ENTRIES);
        assert!(retained_bytes(&draft.text_undo_stack) <= TEXT_HISTORY_MAX_RETAINED_BYTES);
        let expected = draft.text_undo_stack.last().cloned().unwrap();
        assert!(draft.undo_text_edit());
        assert_eq!(draft.text, expected);
    }

    #[test]
    fn newest_oversized_snapshot_remains_immediately_undoable() {
        let mut draft = draft_with_text("initial");
        draft.replace_text_from_user_edit("small edit".to_string());
        let oversized = "x".repeat(TEXT_HISTORY_MAX_RETAINED_BYTES + 1);
        draft.replace_text_from_user_edit(oversized.clone());
        draft.replace_text_from_user_edit("current".to_string());

        assert_eq!(draft.text_undo_stack.len(), 1);
        assert!(retained_bytes(&draft.text_undo_stack) > TEXT_HISTORY_MAX_RETAINED_BYTES);
        assert!(draft.undo_text_edit());
        assert_eq!(draft.text, oversized);
    }
}
