//! Draft text editing, local/project undo, time and track cells, helper text tools, autofix,
//! tag suggestions, find, low-confidence review and warning assertions.

use super::{EditError, EditResult, EditorDoc, Effect, Outcome, TagSuggestionUi};
use crate::edit::draft::TextSelection;
use crate::edit::find::{self, FindSummary};
use crate::edit::format::format_time;
use crate::edit::text::TextMoveDirection;
use crate::edit::time::{
    CommitTimeCellEdit, CommitTimeCellResult, TimeEditState, TimeField, commit_time_cell_edit,
};
use crate::transcript_model::speaker_label;
use exaudio_extension::document::{Completion, CompletionRequest, TextRange};
use studio_core::{EditorCommand, Speaker};

/// Open provider completion list for a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionList {
    pub items: Vec<Completion>,
    pub highlighted: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKey {
    Previous,
    Next,
    Accept,
    Dismiss,
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

    /// Enter, or focus leaving a row: applies the row's typed text to the project as one undo
    /// step when it differs from the segment text (time cells commit on their own), then
    /// reloads the draft from the project, dropping its local text history. A row that no
    /// longer exists has nothing to commit.
    pub fn commit_text_draft(&mut self, segment_id: &str) -> EditResult {
        let Some(segment) = self.workflow.project().segment(segment_id) else {
            return Ok(Outcome::none());
        };
        let Some(draft) = self.drafts.get(segment_id) else {
            return Ok(Outcome::none());
        };
        if draft.text() == segment.text {
            return Ok(Outcome::none());
        }
        let text = draft.text().to_string();
        self.apply("Text edit", |workflow| {
            workflow.apply_text_edit(segment_id, text)
        })?;
        self.drafts.refresh(self.workflow.project(), segment_id);
        self.clear_tag_ui_for(segment_id);
        self.mark_dirty();
        Ok(Outcome::none().with(Effect::DraftChanged {
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

    /// Captures valid pending text/time drafts without changing history or saved revisions.
    pub fn recovery_project(&self) -> Result<studio_core::Project, EditError> {
        let updates = self
            .drafts
            .pending_updates(self.workflow.project())
            .map_err(|source| EditError::Draft {
                action: "Recovery snapshot",
                source,
            })?;
        let mut project = self.workflow.project().clone();
        for update in updates {
            let segment = project
                .segment_mut(&update.segment_id)
                .ok_or_else(|| EditError::SegmentNotFound(update.segment_id.clone()))?;
            segment.start_seconds = update.start_seconds;
            segment.end_seconds = update.end_seconds;
            segment.text = update.text;
        }
        Ok(project)
    }

    /// Commits every pending draft as a single undo step (before save/submit/export): text
    /// still being typed in a focused row and rows a cross-row move touched. Nothing pending
    /// means no undo step. `action` names the caller in messages ("Save", "Submit").
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

    // ---- track / speakers ---------------------------------------------------------------

    /// Moves a row to another lane (speaker track), as one undo step.
    pub fn change_segment_track(&mut self, segment_id: &str, lane_id: &str) -> EditResult {
        self.guard("Track change")?;
        self.apply("Track change", |workflow| {
            workflow.change_segment_track(segment_id, lane_id)
        })?;
        self.mark_dirty();
        self.reconcile_selection_to_visible();
        Ok(Outcome::status(format!(
            "Moved {} to {}.",
            self.segment_label(segment_id),
            self.lane_label(lane_id)
        )))
    }

    /// Assigns a row to a speaker, as one undo step. Where speakers are bound to lanes
    /// (policy), this is a move to the speaker's lane.
    pub fn set_segment_speaker(&mut self, segment_id: &str, speaker_id: &str) -> EditResult {
        self.guard("Speaker change")?;
        if self.environment().policy.speakers_bound_to_lanes {
            let lane_id = self
                .workflow
                .project()
                .lanes
                .iter()
                .find(|lane| lane.speaker_id == speaker_id)
                .map(|lane| lane.id.clone())
                .ok_or_else(|| {
                    EditError::Unavailable(
                        "Speaker change unavailable: that speaker has no audio track.".into(),
                    )
                })?;
            return self.change_segment_track(segment_id, &lane_id);
        }
        let lane_id = self.segment(segment_id)?.lane_id;
        self.apply("Speaker change", |workflow| {
            workflow.apply(EditorCommand::ChangeSegmentSpeaker {
                segment_id: segment_id.to_string(),
                speaker_id: speaker_id.to_string(),
                lane_id,
            })
        })?;
        self.mark_dirty();
        Ok(Outcome::status(format!(
            "Set {} to {}.",
            self.segment_label(segment_id),
            speaker_label(self.workflow.project(), speaker_id)
        )))
    }

    /// Creates a speaker (`label_template` contains `{n}`) and assigns the row to it, as one
    /// undo step; returns the new speaker id. Refused where speakers are the audio tracks.
    pub fn add_speaker_for_segment(
        &mut self,
        segment_id: &str,
        label_template: &str,
    ) -> Result<String, EditError> {
        self.guard("Add speaker")?;
        if self.environment().policy.speakers_bound_to_lanes {
            return Err(EditError::Unavailable(
                "This project's speakers are its audio tracks.".into(),
            ));
        }
        let lane_id = self.segment(segment_id)?.lane_id;
        let project = self.workflow.project();
        let speaker = Speaker {
            id: project.next_speaker_id(),
            label: project.next_speaker_label(label_template),
        };
        let speaker_id = speaker.id.clone();
        self.apply("Add speaker", |workflow| {
            workflow.apply_batch(vec![
                EditorCommand::AddSpeaker { speaker },
                EditorCommand::ChangeSegmentSpeaker {
                    segment_id: segment_id.to_string(),
                    speaker_id: speaker_id.clone(),
                    lane_id,
                },
            ])
        })?;
        self.mark_dirty();
        Ok(speaker_id)
    }

    /// Renames a speaker (trimmed; empty names are refused), as one undo step.
    pub fn rename_speaker(&mut self, speaker_id: &str, label: &str) -> EditResult {
        self.guard("Rename speaker")?;
        let label = label.trim();
        if label.is_empty() {
            return Err(EditError::Unavailable(
                "Rename speaker: the name cannot be empty.".into(),
            ));
        }
        if self
            .workflow
            .project()
            .speaker(speaker_id)
            .is_some_and(|speaker| speaker.label == label)
        {
            return Ok(Outcome::none());
        }
        self.apply("Rename speaker", |workflow| {
            workflow.apply(EditorCommand::RenameSpeaker {
                speaker_id: speaker_id.to_string(),
                label: label.to_string(),
            })
        })?;
        self.mark_dirty();
        Ok(Outcome::status(format!("Renamed speaker to {label}.")))
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

    // ---- tag suggestions --------------------------------------------------------------

    /// Suggestions for the row's caret/selection, or `None` when there are none or the list
    /// was dismissed for the current text and caret.
    pub fn completions(&self, segment_id: &str) -> Option<CompletionList> {
        let draft = self.drafts.get(segment_id)?;
        let ui = self
            .tag_ui
            .as_ref()
            .filter(|ui| ui.matches(segment_id, draft));
        if ui.is_some_and(|ui| ui.dismissed) {
            return None;
        }
        let selection = draft.selection().range();
        let request = CompletionRequest {
            project: self.workflow.project(),
            revision: self.session.current_edit_revision(),
            segment_id,
            text: draft.text(),
            selection: TextRange {
                start: crate::edit::draft::byte_offset(draft.text(), selection.start),
                end: crate::edit::draft::byte_offset(draft.text(), selection.end),
            },
        };
        let items: Vec<Completion> = self
            .environment()
            .providers
            .completions
            .iter()
            .flat_map(|provider| provider.complete(&request))
            .collect();
        if items.is_empty() {
            return None;
        }
        let highlighted = ui.map_or(0, |ui| ui.highlighted.min(items.len() - 1));
        Some(CompletionList { items, highlighted })
    }

    /// Up/Down/Enter/Escape inside an open suggestion list.
    pub fn completion_key(&mut self, segment_id: &str, key: CompletionKey) -> EditResult {
        self.guard("Text edit")?;
        let Some(list) = self.completions(segment_id) else {
            self.clear_tag_ui_for(segment_id);
            return Ok(Outcome::none());
        };
        let last = list.items.len() - 1;
        let next = match key {
            CompletionKey::Previous => list.highlighted.saturating_sub(1),
            CompletionKey::Next => (list.highlighted + 1).min(last),
            CompletionKey::Accept | CompletionKey::Dismiss => list.highlighted,
        };
        if key == CompletionKey::Accept {
            return self.apply_completion(segment_id, &list.items[next]);
        }
        let Some(draft) = self.drafts.get(segment_id) else {
            return Ok(Outcome::none());
        };
        self.tag_ui = Some(TagSuggestionUi {
            segment_id: segment_id.to_string(),
            text: draft.text().to_string(),
            selection: draft.selection(),
            highlighted: next,
            dismissed: key == CompletionKey::Dismiss,
        });
        Ok(Outcome::none())
    }

    /// Inserts a suggestion (click or Enter) as one local undo step.
    pub fn apply_completion(&mut self, segment_id: &str, suggestion: &Completion) -> EditResult {
        self.guard("Text edit")?;
        let draft = self
            .drafts
            .get_mut(segment_id)
            .ok_or_else(|| EditError::NoDraft(segment_id.to_string()))?;
        let range = suggestion.range;
        if range.start > range.end
            || range.end > draft.text().len()
            || !draft.text().is_char_boundary(range.start)
            || !draft.text().is_char_boundary(range.end)
        {
            return Err(EditError::Unavailable(
                "Completion returned an invalid text range.".into(),
            ));
        }
        let mut text = draft.text().to_string();
        text.replace_range(range.start..range.end, &suggestion.insert_text);
        let caret = suggestion
            .caret_offset
            .unwrap_or(range.start + suggestion.insert_text.len());
        if caret > text.len() || !text.is_char_boundary(caret) {
            return Err(EditError::Unavailable(
                "Completion returned an invalid caret.".into(),
            ));
        }
        let selection = TextSelection::caret(text[..caret].chars().count());
        let changed = draft.edit(text, selection);
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
}
