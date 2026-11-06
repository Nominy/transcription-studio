//! Per-segment editable drafts: text with caret/selection and bounded local undo history,
//! time-cell text, track-cell edit state, and pending-draft validation.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use studio_core::{Project, Segment, SegmentUpdate};

use super::format::{format_time, parse_time};
use super::text::CrossRowTextEdit;
use super::time::{TimeEditState, TimeField, start_time_cell_edit};
use exaudio_extension::document::Severity;

/// Each segment retains at most this many text snapshots in either direction.
pub const TEXT_HISTORY_MAX_ENTRIES: usize = 100;
/// Each segment retains at most this many UTF-8 bytes of snapshots in either direction.
/// A newest snapshot larger than the budget is kept alone so the last edit stays reversible.
pub const TEXT_HISTORY_MAX_RETAINED_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn push_bounded_history<T>(
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
    let mut drop_count = 0;
    while history.len() - drop_count > 1
        && (history.len() - drop_count > max_entries || retained_bytes > max_retained_bytes)
    {
        retained_bytes = retained_bytes.saturating_sub(retained_utf8_bytes(&history[drop_count]));
        drop_count += 1;
    }
    history.drain(..drop_count);
}

/// Caret/selection inside a draft, in `char` offsets. `anchor == head` is a plain caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: usize,
    pub head: usize,
}

impl TextSelection {
    pub fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// Ordered `char` range covered by the selection.
    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }

    fn clamped(self, char_len: usize) -> Self {
        Self {
            anchor: self.anchor.min(char_len),
            head: self.head.min(char_len),
        }
    }
}

/// Byte offset of `char_offset` in `text` (clamped to the end).
pub fn byte_offset(text: &str, char_offset: usize) -> usize {
    text.char_indices()
        .nth(char_offset)
        .map_or(text.len(), |(index, _)| index)
}

/// Editable draft of one transcript row.
#[derive(Clone, Debug)]
pub struct SegmentDraft {
    start_text: String,
    end_text: String,
    text: String,
    selection: TextSelection,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    time_edit: TimeEditState,
}

impl SegmentDraft {
    pub fn from_segment(segment: &Segment) -> Self {
        Self {
            start_text: format_time(segment.start_seconds),
            end_text: format_time(segment.end_seconds),
            text: segment.text.clone(),
            selection: TextSelection::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            time_edit: TimeEditState::default(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> TextSelection {
        self.selection
    }

    /// Selected text, or `None` for a plain caret.
    pub fn selected_text(&self) -> Option<&str> {
        if self.selection.is_empty() {
            return None;
        }
        let range = self.selection.range();
        Some(&self.text[byte_offset(&self.text, range.start)..byte_offset(&self.text, range.end)])
    }

    /// Byte offset of the caret (selection head).
    pub fn caret_byte_offset(&self) -> usize {
        byte_offset(&self.text, self.selection.head)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Moves the caret/selection (clamped to the text).
    pub fn set_selection(&mut self, selection: TextSelection) {
        self.selection = selection.clamped(self.text.chars().count());
    }

    /// Replaces the text without recording history (and clears history).
    pub fn replace_text(&mut self, text: String) {
        self.text = text;
        self.selection = self.selection.clamped(self.text.chars().count());
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Applies an edit reported by the text input. Records one undo step when the text changed.
    pub fn edit(&mut self, text: String, selection: TextSelection) -> bool {
        let changed = self.replace_text_from_user_edit(text);
        self.set_selection(selection);
        changed
    }

    /// Replaces the text as a user edit (one undo step) and clamps the selection.
    pub fn replace_text_from_user_edit(&mut self, text: String) -> bool {
        if text == self.text {
            return false;
        }
        let before = std::mem::replace(&mut self.text, text);
        self.push_history(before, true);
        self.redo_stack.clear();
        self.selection = self.selection.clamped(self.text.chars().count());
        true
    }

    /// Replaces the selection (or inserts at the caret) with `replacement` as one user edit.
    /// The caret lands after the inserted text, moved back by `caret_back_chars`.
    pub fn replace_selection(&mut self, replacement: &str, caret_back_chars: usize) -> bool {
        let range = self.selection.range();
        let start = byte_offset(&self.text, range.start);
        let end = byte_offset(&self.text, range.end);
        let mut updated =
            String::with_capacity(self.text.len() - (end - start) + replacement.len());
        updated.push_str(&self.text[..start]);
        updated.push_str(replacement);
        updated.push_str(&self.text[end..]);
        let caret = range.start + replacement.chars().count().saturating_sub(caret_back_chars);
        let changed = self.replace_text_from_user_edit(updated);
        self.set_selection(TextSelection::caret(caret));
        changed
    }

    /// Restores the previous text snapshot. Caret moves to the end of the restored text.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo_stack.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.text, previous);
        self.push_history(current, false);
        self.selection = TextSelection::caret(self.text.chars().count());
        true
    }

    /// Re-applies the next text snapshot. Caret moves to the end of the restored text.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.text, next);
        self.push_history(current, true);
        self.selection = TextSelection::caret(self.text.chars().count());
        true
    }

    pub(crate) fn can_undo_to(&self, previous: &str) -> bool {
        self.undo_stack
            .last()
            .is_some_and(|candidate| candidate == previous)
    }

    pub(crate) fn can_redo_to(&self, next: &str) -> bool {
        self.redo_stack
            .last()
            .is_some_and(|candidate| candidate == next)
    }

    /// Text shown in the start (`TimeField::Start`) or end time cell.
    pub fn time_value(&self, field: TimeField) -> &str {
        match field {
            TimeField::Start => &self.start_text,
            TimeField::End => &self.end_text,
        }
    }

    pub fn set_time_value(&mut self, field: TimeField, value: String) {
        match field {
            TimeField::Start => self.start_text = value,
            TimeField::End => self.end_text = value,
        }
    }

    pub fn replace_times(&mut self, start_seconds: f64, end_seconds: f64) {
        self.start_text = format_time(start_seconds);
        self.end_text = format_time(end_seconds);
    }

    pub fn time_edit(&self) -> &TimeEditState {
        &self.time_edit
    }

    pub(crate) fn time_edit_mut(&mut self) -> &mut TimeEditState {
        &mut self.time_edit
    }

    pub fn start_time_edit(&mut self, field: TimeField, value: f64) {
        self.time_edit = start_time_cell_edit(field, value);
    }

    /// Error to show next to `field`, if it is the active time cell.
    pub fn time_error(&self, field: TimeField) -> Option<String> {
        if self.time_edit.active_field == Some(field) {
            self.time_edit.error.as_ref().map(ToString::to_string)
        } else {
            None
        }
    }

    fn push_history(&mut self, snapshot: String, undo: bool) {
        let history = if undo {
            &mut self.undo_stack
        } else {
            &mut self.redo_stack
        };
        push_bounded_history(
            history,
            snapshot,
            TEXT_HISTORY_MAX_ENTRIES,
            TEXT_HISTORY_MAX_RETAINED_BYTES,
            String::len,
        );
    }
}

/// Why pending drafts cannot be committed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    #[error("start time is invalid")]
    InvalidStart,
    #[error("end time is invalid")]
    InvalidEnd,
    #[error("Start must be before end.")]
    StartNotBeforeEnd,
    #[error("that segment no longer exists")]
    SegmentNotFound(String),
    #[error("that segment has no editable draft")]
    NoDraft(String),
}

/// Parses the draft time cells; untouched cells keep the exact stored seconds.
pub fn parse_draft_times(
    segment: &Segment,
    draft: &SegmentDraft,
) -> Result<(f64, f64), DraftError> {
    let start_seconds = if draft.start_text == format_time(segment.start_seconds) {
        segment.start_seconds
    } else {
        parse_time(&draft.start_text).ok_or(DraftError::InvalidStart)?
    };
    let end_seconds = if draft.end_text == format_time(segment.end_seconds) {
        segment.end_seconds
    } else {
        parse_time(&draft.end_text).ok_or(DraftError::InvalidEnd)?
    };
    Ok((start_seconds, end_seconds))
}

/// Lint shown for a row whose draft time cells are invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftLint {
    pub severity: Severity,
    pub message: String,
}

/// All drafts of an open task plus the cross-row text-move history.
#[derive(Clone, Debug, Default)]
pub struct Drafts {
    map: BTreeMap<String, SegmentDraft>,
    pub(crate) cross_row_undo: Vec<CrossRowTextEdit>,
    pub(crate) cross_row_redo: Vec<CrossRowTextEdit>,
}

impl Drafts {
    pub fn for_project(project: &Project) -> Self {
        let mut drafts = Self::default();
        drafts.reset(project);
        drafts
    }

    pub fn get(&self, segment_id: &str) -> Option<&SegmentDraft> {
        self.map.get(segment_id)
    }

    pub(crate) fn get_mut(&mut self, segment_id: &str) -> Option<&mut SegmentDraft> {
        self.map.get_mut(segment_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &SegmentDraft)> {
        self.map.iter()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Draft text when present, otherwise the stored segment text.
    pub fn text_or<'a>(&'a self, segment: &'a Segment) -> &'a str {
        self.map
            .get(&segment.id)
            .map_or(segment.text.as_str(), SegmentDraft::text)
    }

    /// Recreates every draft from the project and drops all local histories.
    pub fn reset(&mut self, project: &Project) {
        self.cross_row_undo.clear();
        self.cross_row_redo.clear();
        self.map = project
            .segments
            .iter()
            .map(|segment| (segment.id.clone(), SegmentDraft::from_segment(segment)))
            .collect();
    }

    /// Adds drafts for new segments, drops drafts of removed segments, keeps unapplied edits.
    pub fn sync_preserving_existing(&mut self, project: &Project) {
        self.cross_row_undo.clear();
        self.cross_row_redo.clear();
        let current: BTreeSet<&str> = project.segments.iter().map(|s| s.id.as_str()).collect();
        self.map
            .retain(|segment_id, _| current.contains(segment_id.as_str()));
        for segment in &project.segments {
            self.map
                .entry(segment.id.clone())
                .or_insert_with(|| SegmentDraft::from_segment(segment));
        }
    }

    /// Recreates one draft from the project (or removes it when the segment is gone).
    pub fn refresh(&mut self, project: &Project, segment_id: &str) {
        match project.segment(segment_id) {
            Some(segment) => {
                self.map
                    .insert(segment_id.to_string(), SegmentDraft::from_segment(segment));
            }
            None => {
                self.map.remove(segment_id);
            }
        }
    }

    pub fn remove(&mut self, segment_id: &str) {
        self.map.remove(segment_id);
    }

    /// Whether the draft of `segment` differs from it (text or parsed times).
    pub fn differs(&self, segment: &Segment) -> Result<bool, DraftError> {
        let Some(draft) = self.map.get(&segment.id) else {
            return Ok(false);
        };
        let (start_seconds, end_seconds) = parse_draft_times(segment, draft)?;
        if start_seconds >= end_seconds {
            return Err(DraftError::StartNotBeforeEnd);
        }
        Ok(segment.start_seconds != start_seconds
            || segment.end_seconds != end_seconds
            || segment.text != draft.text)
    }

    /// Segment ids whose drafts differ from the project, in project order.
    pub fn pending_segment_ids(&self, project: &Project) -> Result<Vec<String>, DraftError> {
        project
            .segments
            .iter()
            .filter_map(|segment| match self.differs(segment) {
                Ok(true) => Some(Ok(segment.id.clone())),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    pub fn has_pending(&self, project: &Project) -> bool {
        !matches!(self.pending_segment_ids(project), Ok(ids) if ids.is_empty())
    }

    /// Segment updates committing every pending draft (validated).
    pub fn pending_updates(&self, project: &Project) -> Result<Vec<SegmentUpdate>, DraftError> {
        let mut updates = Vec::new();
        for segment_id in self.pending_segment_ids(project)? {
            let segment = project
                .segment(&segment_id)
                .ok_or_else(|| DraftError::SegmentNotFound(segment_id.clone()))?;
            let draft = self
                .map
                .get(&segment_id)
                .ok_or_else(|| DraftError::NoDraft(segment_id.clone()))?;
            let (start_seconds, end_seconds) = parse_draft_times(segment, draft)?;
            if start_seconds >= end_seconds {
                return Err(DraftError::StartNotBeforeEnd);
            }
            updates.push(SegmentUpdate {
                segment_id,
                start_seconds,
                end_seconds,
                text: draft.text.clone(),
            });
        }
        Ok(updates)
    }

    /// Lint override for a row whose draft time cells are invalid; `None` means "use the row
    /// lint". Typed text is not pending work: it commits when the row's editor loses focus.
    pub fn draft_lint(&self, segment: &Segment) -> Option<DraftLint> {
        let draft = self.map.get(&segment.id)?;
        let message = match parse_draft_times(segment, draft) {
            Ok((start_seconds, end_seconds)) if start_seconds >= end_seconds => {
                DraftError::StartNotBeforeEnd.to_string()
            }
            Ok(_) => return None,
            Err(reason) => reason.to_string(),
        };
        Some(DraftLint {
            severity: Severity::Error,
            message,
        })
    }
}
