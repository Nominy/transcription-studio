//! Moving text across same-speaker rows with atomic two-row undo/redo history.

use studio_core::{Project, Segment};

use super::draft::{Drafts, TextSelection, byte_offset, push_bounded_history};

/// Cross-row moves duplicate snapshots from two segments, so each direction keeps at most this
/// many moves (and [`CROSS_ROW_HISTORY_MAX_RETAINED_BYTES`]); one newest oversized move is kept.
pub const CROSS_ROW_HISTORY_MAX_ENTRIES: usize = 32;
pub const CROSS_ROW_HISTORY_MAX_RETAINED_BYTES: usize = 2 * 1024 * 1024;

/// Direction of a cross-row text move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMoveDirection {
    /// Move the text before the caret to the end of the previous same-lane row.
    Previous,
    /// Move the text after the caret to the start of the next same-lane row.
    Next,
}

/// One atomic cross-row move, undoable as a unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossRowTextEdit {
    pub source_segment_id: String,
    pub target_segment_id: String,
    pub source_before: String,
    pub source_after: String,
    pub target_before: String,
    pub target_after: String,
    pub source_before_caret: usize,
    pub source_after_caret: usize,
}

impl CrossRowTextEdit {
    pub fn retained_utf8_bytes(&self) -> usize {
        [
            self.source_segment_id.len(),
            self.target_segment_id.len(),
            self.source_before.len(),
            self.source_after.len(),
            self.target_before.len(),
            self.target_after.len(),
        ]
        .into_iter()
        .fold(0usize, usize::saturating_add)
    }
}

/// Pushes a cross-row move onto a bounded history stack.
pub fn push_cross_row_history(history: &mut Vec<CrossRowTextEdit>, entry: CrossRowTextEdit) {
    push_bounded_history(
        history,
        entry,
        CROSS_ROW_HISTORY_MAX_ENTRIES,
        CROSS_ROW_HISTORY_MAX_RETAINED_BYTES,
        CrossRowTextEdit::retained_utf8_bytes,
    );
}

/// Why a cross-row text move was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextMoveError {
    #[error("No adjacent same-speaker segment found for text move.")]
    NoAdjacentSegment,
    #[error("That segment has no editable draft.")]
    NoDraft(String),
    #[error("No text before the caret to move.")]
    NothingBeforeCaret,
    #[error("No text after the caret to move.")]
    NothingAfterCaret,
    #[error("Text move cancelled: the current segment cannot be left empty.")]
    WouldEmptySource,
}

/// A completed cross-row move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMove {
    pub source_segment_id: String,
    pub target_segment_id: String,
    pub moved_chars: usize,
}

/// Nearest row on the same lane in timeline order (ties broken by project order).
pub fn adjacent_same_lane_segment_id(
    project: &Project,
    segment_id: &str,
    direction: TextMoveDirection,
) -> Option<String> {
    let source_index = project.segments.iter().position(|s| s.id == segment_id)?;
    let source = &project.segments[source_index];
    let order = |left_index: usize, left: &Segment, right_index: usize, right: &Segment| {
        left.start_seconds
            .total_cmp(&right.start_seconds)
            .then_with(|| left_index.cmp(&right_index))
    };
    let candidates = project
        .segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| segment.lane_id == source.lane_id)
        .filter(|(index, segment)| {
            let ordering = order(*index, segment, source_index, source);
            match direction {
                TextMoveDirection::Previous => ordering.is_lt(),
                TextMoveDirection::Next => ordering.is_gt(),
            }
        });
    let pick = match direction {
        TextMoveDirection::Previous => candidates.max_by(|(li, l), (ri, r)| order(*li, l, *ri, r)),
        TextMoveDirection::Next => candidates.min_by(|(li, l), (ri, r)| order(*li, l, *ri, r)),
    };
    pick.map(|(_, segment)| segment.id.clone())
}

/// Joins two text fragments with a single space unless one side already has whitespace.
pub fn join_segment_text(left: &str, right: &str) -> String {
    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() {
        return left.to_string();
    }
    if left.chars().next_back().is_some_and(char::is_whitespace)
        || right.chars().next().is_some_and(char::is_whitespace)
    {
        return format!("{left}{right}");
    }
    format!("{left} {right}")
}

impl Drafts {
    /// Moves the text before (or after) the caret of `segment_id`'s draft into the adjacent
    /// same-lane row. Both drafts record one local undo step; the pair is undoable atomically.
    pub fn move_text_across_rows(
        &mut self,
        project: &Project,
        segment_id: &str,
        direction: TextMoveDirection,
    ) -> Result<TextMove, TextMoveError> {
        let target_segment_id = adjacent_same_lane_segment_id(project, segment_id, direction)
            .ok_or(TextMoveError::NoAdjacentSegment)?;
        let source = self
            .get(segment_id)
            .ok_or_else(|| TextMoveError::NoDraft(segment_id.to_string()))?;
        let source_text = source.text().to_string();
        let split = source.caret_byte_offset();
        let target_text = self
            .get(&target_segment_id)
            .ok_or_else(|| TextMoveError::NoDraft(target_segment_id.clone()))?
            .text()
            .to_string();

        let (moved, next_source, next_target, source_after_caret) = match direction {
            TextMoveDirection::Previous => {
                let moved = source_text[..split].trim().to_string();
                if moved.is_empty() {
                    return Err(TextMoveError::NothingBeforeCaret);
                }
                let next_source = source_text[split..].trim_start().to_string();
                if next_source.is_empty() {
                    return Err(TextMoveError::WouldEmptySource);
                }
                let next_target = join_segment_text(&target_text, &moved);
                (moved, next_source, next_target, 0)
            }
            TextMoveDirection::Next => {
                let moved = source_text[split..].trim().to_string();
                if moved.is_empty() {
                    return Err(TextMoveError::NothingAfterCaret);
                }
                let next_source = source_text[..split].trim_end().to_string();
                if next_source.is_empty() {
                    return Err(TextMoveError::WouldEmptySource);
                }
                let next_target = join_segment_text(&moved, &target_text);
                let caret = next_source.chars().count();
                (moved, next_source, next_target, caret)
            }
        };

        let history = CrossRowTextEdit {
            source_segment_id: segment_id.to_string(),
            target_segment_id: target_segment_id.clone(),
            source_before_caret: source_text[..split].chars().count(),
            source_before: source_text,
            source_after: next_source.clone(),
            target_before: target_text,
            target_after: next_target.clone(),
            source_after_caret,
        };
        if let Some(target) = self.get_mut(&target_segment_id) {
            target.replace_text_from_user_edit(next_target);
        }
        if let Some(source) = self.get_mut(segment_id) {
            source.replace_text_from_user_edit(next_source);
            source.set_selection(TextSelection::caret(source_after_caret));
        }
        push_cross_row_history(&mut self.cross_row_undo, history);
        self.cross_row_redo.clear();

        Ok(TextMove {
            source_segment_id: segment_id.to_string(),
            target_segment_id,
            moved_chars: moved.chars().count(),
        })
    }

    /// Undoes the newest cross-row move touching `segment_id`, if both rows still hold the
    /// moved texts. Returns the `(source, target)` ids that changed.
    pub fn undo_cross_row_move(&mut self, segment_id: &str) -> Option<(String, String)> {
        self.apply_cross_row_history(segment_id, true)
    }

    /// Redoes the newest undone cross-row move touching `segment_id`.
    pub fn redo_cross_row_move(&mut self, segment_id: &str) -> Option<(String, String)> {
        self.apply_cross_row_history(segment_id, false)
    }

    fn apply_cross_row_history(
        &mut self,
        segment_id: &str,
        undo: bool,
    ) -> Option<(String, String)> {
        let stack = if undo {
            &self.cross_row_undo
        } else {
            &self.cross_row_redo
        };
        let history = stack
            .last()
            .filter(|h| h.source_segment_id == segment_id || h.target_segment_id == segment_id)?
            .clone();

        let (source_current, source_next, target_current, target_next, caret) = if undo {
            (
                &history.source_after,
                &history.source_before,
                &history.target_after,
                &history.target_before,
                history.source_before_caret,
            )
        } else {
            (
                &history.source_before,
                &history.source_after,
                &history.target_before,
                &history.target_after,
                history.source_after_caret,
            )
        };
        let ready = |id: &str, current: &str, next: &str| {
            self.get(id).is_some_and(|draft| {
                draft.text() == current
                    && if undo {
                        draft.can_undo_to(next)
                    } else {
                        draft.can_redo_to(next)
                    }
            })
        };
        if !ready(&history.source_segment_id, source_current, source_next)
            || !ready(&history.target_segment_id, target_current, target_next)
        {
            return None;
        }

        for (id, is_source) in [
            (&history.source_segment_id, true),
            (&history.target_segment_id, false),
        ] {
            if let Some(draft) = self.get_mut(id) {
                if undo {
                    draft.undo();
                } else {
                    draft.redo();
                }
                if is_source {
                    draft.set_selection(TextSelection::caret(caret));
                }
            }
        }

        if undo {
            self.cross_row_undo.pop();
            push_cross_row_history(&mut self.cross_row_redo, history.clone());
        } else {
            self.cross_row_redo.pop();
            push_cross_row_history(&mut self.cross_row_undo, history.clone());
        }
        Some((history.source_segment_id, history.target_segment_id))
    }
}

/// Splits `text` at a `char` offset.
pub fn split_text_at_char_offset(text: &str, char_offset: usize) -> (&str, &str) {
    text.split_at(byte_offset(text, char_offset))
}
