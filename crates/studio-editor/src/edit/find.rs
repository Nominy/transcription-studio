//! Find-in-transcript: case-insensitive matching over visible rows (live draft text included)
//! and next/previous match selection with wrap-around.

use super::draft::Drafts;
use crate::transcript_model::TranscriptRowView;

/// Find bar state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindState {
    pub open: bool,
    pub query: String,
}

/// "`n` of `m`" position of the selected match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindSummary {
    /// 1-based index of the selected match (1 when the selection is not a match).
    pub current: usize,
    pub total: usize,
}

/// Normalized query (trimmed, lowercased); `None` when blank.
pub fn normalized_query(query: &str) -> Option<String> {
    let query = query.trim().to_lowercase();
    (!query.is_empty()).then_some(query)
}

/// Whether a row matches `query` (already normalized) by id, speaker, times or text.
pub fn row_matches_query(row: &TranscriptRowView, text: &str, query: &str) -> bool {
    row.display_id.to_lowercase().contains(query)
        || row.speaker_label.to_lowercase().contains(query)
        || row.start_time.to_lowercase().contains(query)
        || row.end_time.to_lowercase().contains(query)
        || text.to_lowercase().contains(query)
}

/// Segment ids of matching rows in row order. Draft text wins over the stored row text.
pub fn find_matches(rows: &[TranscriptRowView], drafts: &Drafts, query: &str) -> Vec<String> {
    let Some(query) = normalized_query(query) else {
        return Vec::new();
    };
    rows.iter()
        .filter(|row| {
            let text = drafts
                .get(&row.segment_id)
                .map_or(row.text.as_str(), |draft| draft.text());
            row_matches_query(row, text, &query)
        })
        .map(|row| row.segment_id.clone())
        .collect()
}

/// Index of the match to select when stepping `direction` (`>= 0` next, `< 0` previous) from
/// `current` (wraps; starts at the first/last match when the selection is not a match).
pub fn step_match_index(
    matches: &[String],
    current: Option<&str>,
    direction: i32,
) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let current_index = current.and_then(|id| matches.iter().position(|m| m == id));
    Some(match (current_index, direction < 0) {
        (Some(index), false) => (index + 1) % matches.len(),
        (Some(0), true) => matches.len() - 1,
        (Some(index), true) => index - 1,
        (None, true) => matches.len() - 1,
        (None, false) => 0,
    })
}

/// Summary for the find bar; `None` when the query is blank.
pub fn find_summary(
    matches: &[String],
    query: &str,
    selected: Option<&str>,
) -> Option<FindSummary> {
    normalized_query(query)?;
    let current = selected
        .and_then(|id| matches.iter().position(|m| m == id))
        .map_or(1, |index| index + 1);
    Some(FindSummary {
        current: if matches.is_empty() { 0 } else { current },
        total: matches.len(),
    })
}
