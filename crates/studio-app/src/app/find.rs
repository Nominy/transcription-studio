use iced::widget::{scrollable, text_input};
use studio_app::{EditorSession, TranscriptRowView};
use studio_core::Project;

use super::StudioState;

const FIND_INPUT_ID: &str = "babel-browser-find-input";
const TRANSCRIPT_SCROLL_ID: &str = "babel-transcript-scroll";

pub(super) fn find_input_id() -> text_input::Id {
    text_input::Id::new(FIND_INPUT_ID)
}

pub(super) fn transcript_scroll_id() -> scrollable::Id {
    scrollable::Id::new(TRANSCRIPT_SCROLL_ID)
}

impl StudioState {
    pub(super) fn open_find_bar(&mut self) {
        self.find_bar_open = true;
    }

    pub(super) fn close_find_bar(&mut self) {
        self.find_bar_open = false;
    }

    pub(super) fn update_find_query(&mut self, query: String) {
        self.find_query = query;
        self.select_first_find_match();
    }

    pub(super) fn select_find_match(&mut self, direction: i32) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let matches = self.find_match_segment_ids(&project, editor);
        if matches.is_empty() {
            return;
        }

        let current_segment_id = editor.selected_segment_id.clone();
        let current_index = matches
            .iter()
            .position(|segment_id| Some(segment_id.as_str()) == current_segment_id.as_deref());
        let next_index = match (current_index, direction.is_negative()) {
            (Some(index), false) => (index + 1) % matches.len(),
            (Some(0), true) => matches.len() - 1,
            (Some(index), true) => index - 1,
            (None, true) => matches.len() - 1,
            (None, false) => 0,
        };
        let next_segment_id = matches[next_index].clone();
        self.select_segment_without_seek(&next_segment_id);
    }

    pub(super) fn find_match_summary(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Option<String> {
        if self.find_query.trim().is_empty() {
            return None;
        }

        let matches = self.find_match_segment_ids(project, editor);
        if matches.is_empty() {
            return Some("No matches".to_string());
        }

        let current_index = matches
            .iter()
            .position(|segment_id| {
                Some(segment_id.as_str()) == editor.selected_segment_id.as_deref()
            })
            .unwrap_or(0);
        Some(format!("{} of {}", current_index + 1, matches.len()))
    }

    pub(super) fn find_match_segment_ids(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Vec<String> {
        let query = self.find_query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }

        editor
            .transcript_rows(project)
            .into_iter()
            .filter(|row| row_matches_query(row, &query))
            .map(|row| row.segment_id)
            .collect()
    }

    #[cfg(test)]
    pub(super) fn find_scroll_fraction_for_selected_match(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Option<f32> {
        let rows = editor.transcript_rows(project);
        let selected_segment_id = editor.selected_segment_id.as_deref()?;
        let selected_index = rows
            .iter()
            .position(|row| row.segment_id == selected_segment_id)?;
        let selected_row = rows.get(selected_index)?;
        let query = self.find_query.trim().to_lowercase();
        if query.is_empty() || !row_matches_query(selected_row, &query) {
            return None;
        }

        let denominator = rows.len().saturating_sub(1);
        Some(if denominator == 0 {
            0.0
        } else {
            selected_index as f32 / denominator as f32
        })
    }

    pub(super) fn find_scroll_offset_for_selected_match(
        &self,
        project: &Project,
        editor: &EditorSession,
    ) -> Option<f32> {
        let selected_segment_id = editor.selected_segment_id.as_deref()?;
        let selected_row = editor
            .transcript_rows(project)
            .into_iter()
            .find(|row| row.segment_id == selected_segment_id)?;
        let query = self.find_query.trim().to_lowercase();
        if query.is_empty() || !row_matches_query(&selected_row, &query) {
            return None;
        }

        self.transcript_scroll_offset_for_segment(project, editor, selected_segment_id)
    }

    fn select_first_find_match(&mut self) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let Some(segment_id) = self
            .find_match_segment_ids(&project, editor)
            .first()
            .cloned()
        else {
            return;
        };

        self.select_segment_without_seek(&segment_id);
    }
}

fn row_matches_query(row: &TranscriptRowView, query: &str) -> bool {
    row.display_id.to_lowercase().contains(query)
        || row.speaker_label.to_lowercase().contains(query)
        || row.start_time.to_lowercase().contains(query)
        || row.end_time.to_lowercase().contains(query)
        || row.text.to_lowercase().contains(query)
}
