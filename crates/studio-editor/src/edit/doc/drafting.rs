//! Applying transcription-provider results and restoring captured text with stale-result guards.

use studio_core::SegmentUpdate;

use super::{EditError, EditResult, EditorDoc, Effect, Outcome};

/// Row bounds compared within this tolerance when a transcription returns.
pub const SEGMENT_BOUNDARY_TOLERANCE_SECONDS: f64 = 0.001;

/// A drafted row to write on a lane.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftedRow {
    pub lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

/// Snapshot of one row (identity, track, bounds, text) for Restore original.
#[derive(Debug, Clone, PartialEq)]
pub struct RowSnapshot {
    pub segment_id: String,
    pub lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

/// The empty row a segment transcription was requested for.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionTarget {
    pub segment_id: String,
    pub lane_id: String,
    pub speaker_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// Position among the task's transcription rows sorted by start.
    pub index: usize,
}

/// Why a segment transcription cannot start or cannot be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranscriptionRefusal {
    #[error("Transcribe segment: select a transcript segment first.")]
    NoSegment,
    #[error("Transcribe segment unavailable: project is read-only.")]
    ReadOnly,
    #[error("Transcribe segment works on speech segments only.")]
    NotTranscription,
    #[error("Transcribe segment only fills an empty segment; this one already has text.")]
    NotEmpty,
    #[error("Transcribe segment: the segment has an invalid time range.")]
    InvalidRange,
    #[error("Transcription discarded: the segment is no longer current.")]
    NoLongerCurrent,
    #[error("Transcription discarded: the segment's boundaries changed.")]
    BoundariesChanged,
    #[error("Transcription discarded: the segment is no longer empty.")]
    NoLongerEmpty,
    #[error("Transcription returned no text.")]
    EmptyResult,
    /// The write itself failed.
    #[error("{0}")]
    Edit(String),
}

impl EditorDoc {
    /// Snapshot of every transcription row (sorted by start), for Restore original.
    pub fn transcription_row_snapshot(&self) -> Vec<RowSnapshot> {
        let mut rows: Vec<RowSnapshot> = self
            .workflow
            .project()
            .segments
            .iter()
            .filter(|segment| self.environment().is_text_segment(segment))
            .map(|segment| RowSnapshot {
                segment_id: segment.id.clone(),
                lane_id: segment.lane_id.clone(),
                start_seconds: segment.start_seconds,
                end_seconds: segment.end_seconds,
                text: self.drafts.text_or(segment).to_string(),
            })
            .collect();
        rows.sort_by(|left, right| {
            left.start_seconds
                .total_cmp(&right.start_seconds)
                .then_with(|| left.segment_id.cmp(&right.segment_id))
        });
        rows
    }

    /// Gold Draft "Writing N segments": replaces every transcription row on `lane_ids` with
    /// `rows` (one undo step; other lanes and non-speech annotations are untouched). Returns
    /// the created row ids in `rows` order.
    pub fn replace_lane_rows(
        &mut self,
        lane_ids: &[String],
        rows: Vec<DraftedRow>,
    ) -> Result<Vec<String>, EditError> {
        self.guard("Write draft")?;
        if let Some(row) = rows.iter().find(|row| !lane_ids.contains(&row.lane_id)) {
            return Err(EditError::Unavailable(format!(
                "Write draft refused: a drafted row targets {}, which was not drafted.",
                self.lane_label(&row.lane_id)
            )));
        }
        let removed: Vec<String> = self
            .workflow
            .project()
            .segments
            .iter()
            .filter(|segment| {
                lane_ids.contains(&segment.lane_id) && self.environment().is_text_segment(segment)
            })
            .map(|segment| segment.id.clone())
            .collect();
        let rows = rows
            .into_iter()
            .map(|row| (row.lane_id, row.start_seconds, row.end_seconds, row.text))
            .collect();
        let created = self.apply("Write draft", |workflow| {
            workflow.replace_segments_with_rows(removed, rows)
        })?;
        self.mark_dirty();
        self.reconcile(true);
        Ok(created)
    }

    /// Gold Draft "Restore original": writes the captured texts back when the transcript still
    /// has exactly the captured rows (same ids, tracks and bounds); any structural change is
    /// refused. One undo step.
    pub fn restore_row_texts(&mut self, snapshot: &[RowSnapshot]) -> EditResult {
        self.guard("Restore original")?;
        let current = self.transcription_row_snapshot();
        let same_structure = current.len() == snapshot.len()
            && current.iter().zip(snapshot).all(|(now, then)| {
                now.segment_id == then.segment_id
                    && now.lane_id == then.lane_id
                    && now.start_seconds == then.start_seconds
                    && now.end_seconds == then.end_seconds
            });
        if !same_structure {
            return Err(EditError::Unavailable(
                "Restore original refused: segments were added, removed, moved or resized since the draft was written.".into(),
            ));
        }
        let updates: Vec<SegmentUpdate> = current
            .iter()
            .zip(snapshot)
            .filter(|(now, then)| now.text != then.text)
            .map(|(_, then)| SegmentUpdate {
                segment_id: then.segment_id.clone(),
                start_seconds: then.start_seconds,
                end_seconds: then.end_seconds,
                text: then.text.clone(),
            })
            .collect();
        if updates.is_empty() {
            return Ok(Outcome::status("Restore original: nothing to restore."));
        }
        let ids: Vec<String> = updates.iter().map(|u| u.segment_id.clone()).collect();
        self.apply("Restore original", |workflow| {
            workflow.apply_segment_edits(updates)
        })?;
        let mut outcome = Outcome::status(format!("Restored {} segments.", ids.len()));
        for id in ids {
            self.drafts.refresh(self.workflow.project(), &id);
            outcome = outcome.with(Effect::DraftChanged { segment_id: id });
        }
        self.mark_dirty();
        self.reconcile(true);
        Ok(outcome)
    }

    /// Alt+Shift+G preconditions: the current row exists, is a writable speech row with a
    /// valid range and its (draft) text is empty.
    pub fn transcription_target(
        &self,
        segment_id: Option<&str>,
    ) -> Result<TranscriptionTarget, TranscriptionRefusal> {
        let segment_id = segment_id
            .or(self.session.selected_segment_id.as_deref())
            .ok_or(TranscriptionRefusal::NoSegment)?;
        if self.is_read_only() {
            return Err(TranscriptionRefusal::ReadOnly);
        }
        let project = self.workflow.project();
        let segment = project
            .segment(segment_id)
            .ok_or(TranscriptionRefusal::NoSegment)?;
        if !self.environment().is_text_segment(segment) {
            return Err(TranscriptionRefusal::NotTranscription);
        }
        if !self.drafts.text_or(segment).trim().is_empty() || !segment.text.trim().is_empty() {
            return Err(TranscriptionRefusal::NotEmpty);
        }
        if !segment.start_seconds.is_finite()
            || !segment.end_seconds.is_finite()
            || segment.end_seconds <= segment.start_seconds
        {
            return Err(TranscriptionRefusal::InvalidRange);
        }
        let index = self
            .transcription_row_snapshot()
            .iter()
            .position(|row| row.segment_id == segment.id)
            .unwrap_or(0);
        Ok(TranscriptionTarget {
            segment_id: segment.id.clone(),
            lane_id: segment.lane_id.clone(),
            speaker_id: segment.speaker_id.clone(),
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
            index,
        })
    }

    /// Writes a returned transcription once, after revalidating that `target` is still the
    /// current row (`current_segment_id`), its bounds are unchanged within 1 ms and both the
    /// row and its draft are still empty. The first letter is upper-cased.
    pub fn apply_segment_transcription(
        &mut self,
        target: &TranscriptionTarget,
        current_segment_id: Option<&str>,
        text: &str,
    ) -> Result<Outcome, TranscriptionRefusal> {
        let current = current_segment_id.or(self.session.selected_segment_id.as_deref());
        if current != Some(target.segment_id.as_str()) {
            return Err(TranscriptionRefusal::NoLongerCurrent);
        }
        let segment = self
            .workflow
            .project()
            .segment(&target.segment_id)
            .ok_or(TranscriptionRefusal::NoLongerCurrent)?;
        if (segment.start_seconds - target.start_seconds).abs() > SEGMENT_BOUNDARY_TOLERANCE_SECONDS
            || (segment.end_seconds - target.end_seconds).abs() > SEGMENT_BOUNDARY_TOLERANCE_SECONDS
        {
            return Err(TranscriptionRefusal::BoundariesChanged);
        }
        if !segment.text.trim().is_empty() || !self.drafts.text_or(segment).trim().is_empty() {
            return Err(TranscriptionRefusal::NoLongerEmpty);
        }
        let text = capitalize_first_letter(text.trim());
        if text.is_empty() {
            return Err(TranscriptionRefusal::EmptyResult);
        }
        let (start, end) = (segment.start_seconds, segment.end_seconds);
        let id = target.segment_id.clone();
        let length = text.chars().count();
        self.apply("Transcribe segment", |workflow| {
            workflow.apply_segment_edit(&id, start, end, text)
        })
        .map_err(|error| TranscriptionRefusal::Edit(error.to_string()))?;
        self.drafts.refresh(self.workflow.project(), &id);
        self.mark_dirty();
        self.reconcile(true);
        Ok(Outcome::status(format!(
            "Transcribed {} ({length} characters).",
            self.segment_label(&id)
        ))
        .with(Effect::DraftChanged { segment_id: id }))
    }
}

/// Upper-cases the first Unicode letter (`/\p{L}/u`), leaving everything else as is.
pub fn capitalize_first_letter(text: &str) -> String {
    match text.char_indices().find(|(_, c)| c.is_alphabetic()) {
        Some((index, letter)) => {
            let mut out = String::with_capacity(text.len());
            out.push_str(&text[..index]);
            out.extend(letter.to_uppercase());
            out.push_str(&text[index + letter.len_utf8()..]);
            out
        }
        None => text.to_string(),
    }
}

/// Gold `removeWordFinalEllipsis` (`/(?<=[\p{L}\p{N}])(?:\.\.\.|…)(?![\p{L}\p{N}])/gu` → ""):
/// a model may never attach "..." or "…" to the end of a word; it only prefixes a following
/// word. The lookbehind reads the original text, as the global regex does.
pub fn remove_word_final_ellipsis(text: &str) -> String {
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphabetic() || c.is_numeric());
    let mut out = String::with_capacity(text.len());
    let mut previous = None;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let ellipsis = if rest.starts_with("...") {
            3
        } else if c == '…' {
            c.len_utf8()
        } else {
            0
        };
        if ellipsis > 0 && is_word(previous) && !is_word(rest[ellipsis..].chars().next()) {
            previous = rest[..ellipsis].chars().last();
            rest = &rest[ellipsis..];
            continue;
        }
        out.push(c);
        previous = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}
