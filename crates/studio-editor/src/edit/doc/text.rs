//! Draft text editing, local/project undo, time and track cells, helper text tools, autofix,
//! tag suggestions, find, low-confidence review and warning assertions.

use super::{EditError, EditResult, EditorDoc, Effect, Outcome, TagSuggestionUi};
use crate::edit::autofix::{autofix_transcript_text, previous_same_lane_text};
use crate::edit::draft::TextSelection;
use crate::edit::find::{self, FindSummary};
use crate::edit::format::format_time;
use crate::edit::tags::{self, TagSuggestion, TagSuggestionKey};
use crate::edit::text::{TextMoveDirection, skaz_for_selection, skaz_with_digit};
use crate::edit::time::{
    CommitTimeCellEdit, CommitTimeCellResult, TimeEditState, TimeField, commit_time_cell_edit,
};
use crate::project_lint_tag_config::resolve_babel_project_lint_tag_config;

/// Open tag-suggestion list of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSuggestionList {
    pub items: Vec<TagSuggestion>,
    pub highlighted: usize,
}

impl EditorDoc {
    // ---- draft text ---------------------------------------------------------------------

    /// Text input change reported by the UI (typing, paste, cut). One local undo step.
    pub fn edit_text(
        &mut self,
        segment_id: &str,
        text: String,
        selection: TextSelection,
    ) -> EditResult {
        self.guard("Text edit")?;
        let draft = self
            .drafts
            .get_mut(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        let changed = draft.edit(text, selection);
        self.clear_tag_ui_for(segment_id);
        if changed {
            self.mark_dirty();
        }
        Ok(Outcome::none())
    }

    /// Caret/selection change without a text change.
    pub fn set_selection(&mut self, segment_id: &str, selection: TextSelection) {
        if let Some(draft) = self.drafts.get_mut(segment_id) {
            draft.set_selection(selection);
        }
    }

    /// Text-editor undo inside a row: undoes a cross-row move touching the row first,
    /// otherwise the row's own last text edit.
    pub fn undo_text(&mut self, segment_id: &str) -> EditResult {
        self.step_text_history(segment_id, true)
    }

    /// Text-editor redo inside a row (cross-row move first).
    pub fn redo_text(&mut self, segment_id: &str) -> EditResult {
        self.step_text_history(segment_id, false)
    }

    fn step_text_history(&mut self, segment_id: &str, undo: bool) -> EditResult {
        self.guard("Text edit")?;
        let cross_row = if undo {
            self.drafts.undo_cross_row_move(segment_id)
        } else {
            self.drafts.redo_cross_row_move(segment_id)
        };
        if let Some((source, target)) = cross_row {
            self.clear_tag_ui_for(&source);
            self.clear_tag_ui_for(&target);
            self.mark_dirty();
            let status = if undo {
                "Undid cross-row text move."
            } else {
                "Redid cross-row text move."
            };
            return Ok(Outcome::status(status)
                .with(Effect::DraftChanged { segment_id: source })
                .with(Effect::DraftChanged { segment_id: target }));
        }

        let draft = self
            .drafts
            .get_mut(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        let changed = if undo { draft.undo() } else { draft.redo() };
        self.clear_tag_ui_for(segment_id);
        if !changed {
            return Ok(Outcome::none());
        }
        self.mark_dirty();
        Ok(Outcome::none().with(Effect::DraftChanged {
            segment_id: segment_id.to_string(),
        }))
    }

    /// Commits one row's draft (text + typed times) to the project as one undo step.
    pub fn commit_draft(&mut self, segment_id: &str) -> EditResult {
        self.guard("Apply")?;
        let segment = self.segment(segment_id)?;
        let draft = self
            .drafts
            .get(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        let (start, end) =
            crate::edit::draft::parse_draft_times(&segment, draft).map_err(|source| {
                EditError::Draft {
                    action: "Apply",
                    source,
                }
            })?;
        if start >= end {
            return Err(EditError::Draft {
                action: "Apply",
                source: crate::edit::draft::DraftError::StartNotBeforeEnd,
            });
        }
        let text = draft.text().to_string();
        if segment.start_seconds == start && segment.end_seconds == end && segment.text == text {
            return Ok(Outcome::status(format!(
                "No changes to apply for {}.",
                self.segment_label(segment_id)
            )));
        }
        self.apply("Apply", |workflow| {
            workflow.apply_segment_edit(segment_id, start, end, text)
        })?;
        self.drafts.refresh(self.workflow.project(), segment_id);
        self.mark_dirty();
        Ok(Outcome::status(format!(
            "Applied edits to {}.",
            self.segment_label(segment_id)
        ))
        .with(Effect::DraftChanged {
            segment_id: segment_id.to_string(),
        }))
    }

    /// Segment ids whose drafts differ from the project.
    pub fn pending_draft_segment_ids(&self) -> Result<Vec<String>, crate::edit::draft::DraftError> {
        self.drafts.pending_segment_ids(self.workflow.project())
    }

    /// Whether any draft differs from the project (or is invalid).
    pub fn has_pending_drafts(&self) -> bool {
        self.drafts.has_pending(self.workflow.project())
    }

    /// Commits every pending draft as a single undo step (before save/submit/export).
    /// `action` names the caller in messages ("Save", "Submit").
    pub fn commit_pending_drafts(&mut self, action: &'static str) -> EditResult {
        let updates = self
            .drafts
            .pending_updates(self.workflow.project())
            .map_err(|source| EditError::Draft { action, source })?;
        if updates.is_empty() {
            return Ok(Outcome::none());
        }
        let count = updates.len();
        self.apply(action, |workflow| workflow.apply_segment_edits(updates))?;
        self.mark_dirty();
        Ok(Outcome::status(format!(
            "{action}: applied {count} pending draft rows."
        )))
    }

    // ---- project history --------------------------------------------------------------

    /// Project undo. Refused while drafts are pending (their local undo comes first).
    pub fn undo(&mut self) -> EditResult {
        self.step_history(true)
    }

    /// Project redo. Refused while drafts are pending.
    pub fn redo(&mut self) -> EditResult {
        self.step_history(false)
    }

    fn step_history(&mut self, undo: bool) -> EditResult {
        let action = if undo { "Undo" } else { "Redo" };
        self.guard(action)?;
        if self.has_pending_drafts() {
            return Err(EditError::PendingDrafts { action });
        }
        self.apply(action, |workflow| {
            if undo {
                workflow.undo()
            } else {
                workflow.redo()
            }
        })?;
        self.drafts.reset(self.workflow.project());
        self.tag_ui = None;
        self.mark_dirty();
        self.reconcile(true);
        Ok(Outcome::status(if undo {
            "Undid last edit."
        } else {
            "Redid last edit."
        }))
    }

    // ---- time cells -------------------------------------------------------------------

    /// Typing in a time cell (starts editing that field when needed).
    pub fn set_time_draft(
        &mut self,
        segment_id: &str,
        field: TimeField,
        value: String,
    ) -> EditResult {
        self.guard("Time edit")?;
        let current = self
            .workflow
            .project()
            .segment(segment_id)
            .map(|segment| field.value_from(segment.start_seconds, segment.end_seconds));
        let draft = self
            .drafts
            .get_mut(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        if draft.time_edit().active_field != Some(field)
            && let Some(current) = current
        {
            draft.start_time_edit(field, current);
        }
        draft.set_time_value(field, value.clone());
        *draft.time_edit_mut() = TimeEditState {
            active_field: Some(field),
            draft_value: value,
            error: None,
        };
        Ok(Outcome::none())
    }

    /// Commits a time cell (Enter, or blur with `from_blur`). A valid value moves the boundary
    /// as one undo step without flushing the row's pending text draft.
    pub fn commit_time_edit(
        &mut self,
        segment_id: &str,
        field: TimeField,
        from_blur: bool,
    ) -> EditResult {
        self.guard("Time edit")?;
        let segment = self.segment(segment_id)?;
        let draft = self
            .drafts
            .get(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        let result = commit_time_cell_edit(CommitTimeCellEdit {
            active_field: draft.time_edit().active_field,
            field,
            draft_value: draft.time_value(field).to_string(),
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
            from_blur,
        });
        let CommitTimeCellResult::Handled {
            active_field,
            draft_value,
            error,
            next_times,
        } = result
        else {
            return Ok(Outcome::none());
        };
        if let Some(draft) = self.drafts.get_mut(segment_id) {
            *draft.time_edit_mut() = TimeEditState {
                active_field,
                draft_value: draft_value.clone(),
                error: error.clone(),
            };
            draft.set_time_value(field, draft_value);
        }
        if let Some(error) = error {
            return Err(EditError::Time(error));
        }
        let Some(next) = next_times else {
            return Ok(Outcome::none());
        };
        if next.start_seconds == segment.start_seconds && next.end_seconds == segment.end_seconds {
            if let Some(draft) = self.drafts.get_mut(segment_id) {
                draft.replace_times(segment.start_seconds, segment.end_seconds);
            }
            return Ok(Outcome::status(format!(
                "No timestamp changes to apply for {}.",
                self.segment_label(segment_id)
            )));
        }

        self.apply("Time edit", |workflow| {
            workflow.move_segment_boundary(segment_id, next.start_seconds, next.end_seconds)?;
            workflow.finish_boundary_edit();
            Ok(())
        })?;
        if let Some(draft) = self.drafts.get_mut(segment_id) {
            draft.replace_times(next.start_seconds, next.end_seconds);
            *draft.time_edit_mut() = TimeEditState::default();
        }
        self.mark_dirty();
        self.reconcile(true);
        Ok(Outcome::status(format!(
            "Updated {} {} time to {}.",
            self.segment_label(segment_id),
            field.label(),
            format_time(field.value_from(next.start_seconds, next.end_seconds))
        )))
    }

    /// Escape in a time cell: leaves it without applying, showing the stored time again
    /// (Babel's time cell cancel).
    pub fn cancel_time_edit(&mut self, segment_id: &str, field: TimeField) {
        let Some(stored) = self
            .workflow
            .project()
            .segment(segment_id)
            .map(|segment| field.value_from(segment.start_seconds, segment.end_seconds))
        else {
            return;
        };
        if let Some(draft) = self.drafts.get_mut(segment_id) {
            draft.set_time_value(field, format_time(stored));
            *draft.time_edit_mut() = TimeEditState::default();
        }
    }

    // ---- track cell -------------------------------------------------------------------

    /// Click on a row's track cell: selects the row; a double click opens the lane picker.
    pub fn press_track_cell(&mut self, segment_id: &str, click_count: usize) -> EditResult {
        self.guard("Track edit")?;
        let outcome = self.select_segment(segment_id);
        if click_count >= 2 {
            self.begin_track_edit(segment_id);
        }
        Ok(outcome)
    }

    /// Opens the lane picker of a row.
    pub fn begin_track_edit(&mut self, segment_id: &str) {
        let lane_id = self
            .workflow
            .project()
            .segment(segment_id)
            .map(|segment| segment.lane_id.clone());
        if let (Some(lane_id), Some(draft)) = (lane_id, self.drafts.get_mut(segment_id)) {
            draft.track_edit_mut().begin(&lane_id);
        }
    }

    /// Lane picker opened/closed without choosing (closing resets the choice).
    pub fn set_track_select_open(&mut self, segment_id: &str, open: bool) {
        let lane_id = self
            .workflow
            .project()
            .segment(segment_id)
            .map(|segment| segment.lane_id.clone());
        if let (Some(lane_id), Some(draft)) = (lane_id, self.drafts.get_mut(segment_id)) {
            draft.track_edit_mut().set_open(open, &lane_id);
        }
    }

    /// Moves a row to another lane (speaker track), as one undo step.
    pub fn change_segment_track(&mut self, segment_id: &str, lane_id: &str) -> EditResult {
        self.guard("Track change")?;
        if let Some(draft) = self.drafts.get_mut(segment_id) {
            draft.track_edit_mut().select(lane_id);
        }
        self.apply("Track change", |workflow| {
            workflow.change_segment_track(segment_id, lane_id)
        })?;
        if let Some(draft) = self.drafts.get_mut(segment_id) {
            draft.track_edit_mut().select(lane_id);
        }
        self.mark_dirty();
        self.reconcile_selection_to_visible();
        Ok(Outcome::status(format!(
            "Moved {} to {}.",
            self.segment_label(segment_id),
            self.lane_label(lane_id)
        )))
    }

    // ---- helper text tools ------------------------------------------------------------

    /// Moves the text before the caret to the previous same-lane row (Alt+[).
    pub fn move_text_to_previous_row(&mut self, segment_id: &str) -> EditResult {
        self.move_text(segment_id, TextMoveDirection::Previous)
    }

    /// Moves the text after the caret to the next same-lane row (Alt+]).
    pub fn move_text_to_next_row(&mut self, segment_id: &str) -> EditResult {
        self.move_text(segment_id, TextMoveDirection::Next)
    }

    fn move_text(&mut self, segment_id: &str, direction: TextMoveDirection) -> EditResult {
        self.guard("Text move")?;
        let moved =
            self.drafts
                .move_text_across_rows(self.workflow.project(), segment_id, direction)?;
        self.clear_tag_ui_for(&moved.source_segment_id);
        self.clear_tag_ui_for(&moved.target_segment_id);
        self.mark_dirty();
        let where_to = match direction {
            TextMoveDirection::Previous => "previous",
            TextMoveDirection::Next => "next",
        };
        Ok(Outcome::status(format!(
            "Moved {} characters to the {where_to} same-speaker segment.",
            moved.moved_chars
        ))
        .with(Effect::DraftChanged {
            segment_id: moved.source_segment_id,
        })
        .with(Effect::DraftChanged {
            segment_id: moved.target_segment_id,
        }))
    }

    /// Converts the selected number in the focused/selected row to SKAZ form (Alt+A).
    pub fn convert_selection_to_skaz(&mut self) -> EditResult {
        self.guard("SKAZ conversion")?;
        let segment_id = self
            .helper
            .focused_segment_id
            .clone()
            .or_else(|| self.session.selected_segment_id.clone())
            .ok_or(EditError::NoSelection {
                action: "SKAZ conversion",
            })?;
        let selected = self
            .drafts
            .get(&segment_id)
            .and_then(|draft| draft.selected_text())
            .ok_or_else(|| {
                EditError::Unavailable("Select numeric text before converting SKAZ.".into())
            })?;
        let replacement = skaz_for_selection(selected).ok_or_else(|| {
            EditError::Unavailable(format!(
                "Selected text `{selected}` is not a supported SKAZ number."
            ))
        })?;
        self.replace_selection(&segment_id, &replacement, 0)
    }

    /// A digit typed over selected spoken-digit text: `"{digit} {СКАЗ: selection}"`, caret
    /// right after the digit.
    pub fn convert_selection_with_digit(&mut self, segment_id: &str, digit: char) -> EditResult {
        self.guard("SKAZ conversion")?;
        let unavailable = || {
            EditError::Unavailable("Select spoken digit text before typing a SKAZ digit.".into())
        };
        let selected = self
            .drafts
            .get(segment_id)
            .and_then(|draft| draft.selected_text())
            .ok_or_else(unavailable)?;
        let replacement = skaz_with_digit(digit, selected).ok_or_else(unavailable)?;
        let back = replacement.chars().count().saturating_sub(1);
        self.replace_selection(segment_id, &replacement, back)
    }

    fn replace_selection(
        &mut self,
        segment_id: &str,
        replacement: &str,
        caret_back: usize,
    ) -> EditResult {
        let draft = self
            .drafts
            .get_mut(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        if !draft.replace_selection(replacement, caret_back) {
            return Ok(Outcome::none());
        }
        self.mark_dirty();
        self.clear_tag_ui_for(segment_id);
        Ok(
            Outcome::status("Converted selected text to SKAZ.").with(Effect::DraftChanged {
                segment_id: segment_id.to_string(),
            }),
        )
    }

    /// Applies lint autofix to the selected row's draft (Alt+F).
    pub fn autofix_current_row(&mut self) -> EditResult {
        self.guard("Lint autofix")?;
        let segment_id = self.selected_segment_id("Lint autofix")?;
        if self.autofix_row(&segment_id) {
            Ok(Outcome::status("Applied lint autofix to current row.")
                .with(Effect::DraftChanged { segment_id }))
        } else {
            Ok(Outcome::status("No lint autofix changes for current row."))
        }
    }

    /// Applies lint autofix to every row's draft in project order (Alt+Shift+F).
    pub fn autofix_all_rows(&mut self) -> EditResult {
        self.guard("Lint autofix")?;
        let ids: Vec<String> = self
            .workflow
            .project()
            .segments
            .iter()
            .map(|segment| segment.id.clone())
            .collect();
        let total = ids.len();
        let mut outcome = Outcome::none();
        for segment_id in ids {
            if self.autofix_row(&segment_id) {
                outcome.effects.push(Effect::DraftChanged { segment_id });
            }
        }
        let fixed = outcome.effects.len();
        Ok(outcome.with_status(if fixed == 0 {
            format!("No lint autofix changes across {total} rows.")
        } else {
            format!("Applied lint autofix to {fixed} of {total} rows.")
        }))
    }

    fn autofix_row(&mut self, segment_id: &str) -> bool {
        let previous = previous_same_lane_text(self.workflow.project(), &self.drafts, segment_id);
        let Some(draft) = self.drafts.get_mut(segment_id) else {
            return false;
        };
        let fixed = autofix_transcript_text(draft.text(), &previous);
        if fixed == draft.text() {
            return false;
        }
        let caret = draft.selection().head.min(fixed.chars().count());
        draft.replace_text_from_user_edit(fixed);
        draft.set_selection(TextSelection::caret(caret));
        self.clear_tag_ui_for(segment_id);
        self.mark_dirty();
        true
    }

    // ---- tag suggestions --------------------------------------------------------------

    /// Suggestions for the row's caret/selection, or `None` when there are none or the list
    /// was dismissed for the current text and caret.
    pub fn tag_suggestions(&self, segment_id: &str) -> Option<TagSuggestionList> {
        let draft = self.drafts.get(segment_id)?;
        let ui = self
            .tag_ui
            .as_ref()
            .filter(|ui| ui.matches(segment_id, draft));
        if ui.is_some_and(|ui| ui.dismissed) {
            return None;
        }
        let config = resolve_babel_project_lint_tag_config(self.workflow.project());
        let items = tags::suggestions_for_draft(draft, &config);
        if items.is_empty() {
            return None;
        }
        let highlighted = ui.map_or(0, |ui| ui.highlighted.min(items.len() - 1));
        Some(TagSuggestionList { items, highlighted })
    }

    /// Up/Down/Enter/Escape inside an open suggestion list.
    pub fn tag_suggestion_key(&mut self, segment_id: &str, key: TagSuggestionKey) -> EditResult {
        self.guard("Text edit")?;
        let Some(list) = self.tag_suggestions(segment_id) else {
            self.clear_tag_ui_for(segment_id);
            return Ok(Outcome::none());
        };
        let last = list.items.len() - 1;
        let next = match key {
            TagSuggestionKey::Previous => list.highlighted.saturating_sub(1),
            TagSuggestionKey::Next => (list.highlighted + 1).min(last),
            TagSuggestionKey::Accept | TagSuggestionKey::Dismiss => list.highlighted,
        };
        if key == TagSuggestionKey::Accept {
            return self.apply_tag_suggestion(segment_id, &list.items[next]);
        }
        let Some(draft) = self.drafts.get(segment_id) else {
            return Ok(Outcome::none());
        };
        self.tag_ui = Some(TagSuggestionUi {
            segment_id: segment_id.to_string(),
            text: draft.text().to_string(),
            selection: draft.selection(),
            highlighted: next,
            dismissed: key == TagSuggestionKey::Dismiss,
        });
        Ok(Outcome::none())
    }

    /// Inserts a suggestion (click or Enter) as one local undo step.
    pub fn apply_tag_suggestion(
        &mut self,
        segment_id: &str,
        suggestion: &TagSuggestion,
    ) -> EditResult {
        self.guard("Text edit")?;
        let draft = self
            .drafts
            .get_mut(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        let changed = tags::apply_suggestion(draft, suggestion);
        self.clear_tag_ui_for(segment_id);
        if !changed {
            return Ok(Outcome::none());
        }
        self.mark_dirty();
        Ok(Outcome::none().with(Effect::DraftChanged {
            segment_id: segment_id.to_string(),
        }))
    }

    // ---- find -------------------------------------------------------------------------

    pub fn open_find(&mut self) -> Outcome {
        self.find.open = true;
        self.reveal_selected_find_match()
    }

    pub fn close_find(&mut self) {
        self.find.open = false;
    }

    /// Updates the query and selects the first match (without seeking).
    pub fn set_find_query(&mut self, query: String) -> Outcome {
        self.find.query = query;
        let Some(first) = self.find_matches().into_iter().next() else {
            return Outcome::none();
        };
        self.session
            .select_segment_without_seek(self.workflow.project(), &first);
        self.touch();
        Outcome::none().with(Effect::RevealSegment {
            segment_id: first,
            center: true,
        })
    }

    /// Selects the next match (wraps).
    pub fn find_next(&mut self) -> Outcome {
        self.step_find(1)
    }

    /// Selects the previous match (wraps).
    pub fn find_previous(&mut self) -> Outcome {
        self.step_find(-1)
    }

    fn step_find(&mut self, direction: i32) -> Outcome {
        let matches = self.find_matches();
        let Some(index) = find::step_match_index(
            &matches,
            self.session.selected_segment_id.as_deref(),
            direction,
        ) else {
            return Outcome::none();
        };
        let segment_id = matches[index].clone();
        self.session
            .select_segment_without_seek(self.workflow.project(), &segment_id);
        self.touch();
        Outcome::none().with(Effect::RevealSegment {
            segment_id,
            center: true,
        })
    }

    fn reveal_selected_find_match(&self) -> Outcome {
        let matches = self.find_matches();
        match self.session.selected_segment_id.as_ref() {
            Some(selected) if matches.contains(selected) => {
                Outcome::none().with(Effect::RevealSegment {
                    segment_id: selected.clone(),
                    center: true,
                })
            }
            _ => Outcome::none(),
        }
    }

    /// Matching visible rows (draft text included), in row order.
    pub fn find_matches(&self) -> Vec<String> {
        find::find_matches(&self.transcript_rows(), &self.drafts, &self.find.query)
    }

    /// "`n` of `m`" for the find bar; `None` for a blank query.
    pub fn find_summary(&self) -> Option<FindSummary> {
        find::find_summary(
            &self.find_matches(),
            &self.find.query,
            self.session.selected_segment_id.as_deref(),
        )
    }

    // ---- review -----------------------------------------------------------------------

    /// Marks one low-confidence word reviewed; when all words of the row are reviewed the
    /// row's low-confidence flag is resolved in the project (one undo step).
    pub fn acknowledge_low_confidence_word(
        &mut self,
        segment_id: &str,
        word_index: usize,
    ) -> EditResult {
        self.guard("Low-confidence review")?;
        let row = self
            .session
            .all_transcript_rows(self.workflow.project())
            .into_iter()
            .find(|row| row.segment_id == segment_id)
            .ok_or_else(|| EditError::SegmentNotFound(segment_id.to_string()))?;
        let reviewable: std::collections::BTreeSet<usize> = row
            .word_views
            .iter()
            .filter(|word| word.is_low_confidence)
            .map(|word| word.index)
            .collect();
        if !reviewable.contains(&word_index) {
            return Err(EditError::Unavailable(
                "That low-confidence word was not found.".into(),
            ));
        }
        let acknowledged = self
            .low_confidence_acks
            .entry(segment_id.to_string())
            .or_default();
        acknowledged.insert(word_index);
        let (reviewed, total) = (acknowledged.len(), reviewable.len());
        let complete = reviewable.iter().all(|index| acknowledged.contains(index));
        self.touch();
        if !complete {
            return Ok(Outcome::status(format!(
                "Reviewed {reviewed} of {total} low-confidence words in {}.",
                self.segment_label(segment_id)
            )));
        }
        self.apply("Low-confidence review", |workflow| {
            workflow.resolve_low_confidence(segment_id)
        })?;
        self.low_confidence_acks.remove(segment_id);
        self.mark_dirty();
        self.reconcile(false);
        Ok(Outcome::status(format!(
            "Marked low-confidence words reviewed for {}.",
            self.segment_label(segment_id)
        )))
    }

    /// Toggles the warning assertion of `segment_id` (selecting it) or of the selected row.
    pub fn toggle_warning_assertion(&mut self, segment_id: Option<&str>) -> EditResult {
        self.guard("Warning assertion")?;
        let project = self.workflow.project();
        if let Some(segment_id) = segment_id {
            self.session.select_segment(project, segment_id);
        }
        let toggled = self.session.toggle_warning_assertion(project);
        self.touch();
        Ok(if toggled {
            Outcome::status("Warning assertion updated.")
        } else {
            Outcome::none()
        })
    }
}
