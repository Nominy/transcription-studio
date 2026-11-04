//! Text allocation for splits and auto-segmentation (Babel Helper `smart-split.ts`
//! `getSmartSplitTextParts` and `auto-segmentation/text-allocation.ts`). Allocations never
//! rewrite, add or drop words: every result is checked for exact text conservation.

use super::align::{PreparedLane, TimeRange, cut_boundary_offset, offset_at_time};
use super::text::{
    SplitParts, SplitSource, compact_text, nearest_word_boundary, normalize_text,
    split_at_floor_offset, split_by_word_ratio,
};

/// Splits `text` for a cut at `split_seconds` inside `range`: exact completed-word boundary →
/// interpolated timing offset → word-ratio estimate. `None` for an invalid split point.
pub fn smart_split_text(
    text: &str,
    lane: Option<&PreparedLane>,
    range: TimeRange,
    split_seconds: f64,
) -> Option<SplitParts> {
    if !range.start_seconds.is_finite()
        || !range.end_seconds.is_finite()
        || !split_seconds.is_finite()
        || split_seconds <= range.start_seconds
        || split_seconds >= range.end_seconds
    {
        return None;
    }
    if text.trim().is_empty() {
        return Some(SplitParts {
            first: String::new(),
            second: String::new(),
            split_count: 0,
            word_count: 0,
            source: SplitSource::Empty,
        });
    }
    if let Some(lane) = lane {
        if let Some(offset) = cut_boundary_offset(text, lane, range, split_seconds) {
            return Some(split_at_floor_offset(
                text,
                offset as f64,
                SplitSource::CompletedWord,
            ));
        }
        if let Some(offset) = offset_at_time(text, lane, range, split_seconds) {
            return Some(split_at_floor_offset(
                text,
                offset as f64,
                SplitSource::Interpolated,
            ));
        }
    }
    let ratio = (split_seconds - range.start_seconds) / (range.end_seconds - range.start_seconds);
    Some(split_by_word_ratio(text, ratio))
}

/// One row receiving text.
#[derive(Debug, Clone, PartialEq)]
pub struct AllocationSegment {
    pub id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// Consecutive rows of one lane sharing a captured text (`AutoSegmentTextGroup`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextGroup {
    pub lane_id: String,
    pub full_text: String,
    /// Sorted by start.
    pub segments: Vec<AllocationSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AllocationError {
    #[error("the group has no rows")]
    MissingSegments,
    #[error("a row has an invalid time range")]
    InvalidSegment,
    #[error("no word timestamps anchor this text")]
    MissingTimingAnchors,
    #[error("the allocation would lose text")]
    LostText,
}

fn validate_group(group: &TextGroup) -> Result<(), AllocationError> {
    if group.segments.is_empty() {
        return Err(AllocationError::MissingSegments);
    }
    if group.segments.iter().any(|segment| {
        segment.id.is_empty()
            || !segment.start_seconds.is_finite()
            || !segment.end_seconds.is_finite()
            || segment.end_seconds <= segment.start_seconds
    }) {
        return Err(AllocationError::InvalidSegment);
    }
    Ok(())
}

/// Cuts `source` into `count` normalized chunks; each cut may depend on where the previous one
/// ended.
fn slice_chunks(
    source: &[char],
    count: usize,
    mut next_boundary: impl FnMut(usize, usize) -> usize,
) -> Vec<String> {
    let mut chunks = Vec::with_capacity(count);
    let mut cursor = 0;
    for index in 0..count.saturating_sub(1) {
        let boundary = next_boundary(index, cursor).clamp(cursor, source.len());
        chunks.push(normalize_text(
            &source[cursor..boundary].iter().collect::<String>(),
        ));
        cursor = boundary;
    }
    chunks.push(normalize_text(&source[cursor..].iter().collect::<String>()));
    chunks
}

/// Whether `allocations` (one per group row, in order) are exactly the group text.
pub fn allocations_preserve_text(group: &TextGroup, allocations: &[String]) -> bool {
    allocations.len() == group.segments.len()
        && compact_text(&group.full_text) == compact_text(&allocations.join(" "))
}

/// Timed allocation (`createL0TimedAutoSegmentTextAllocations`, nearest-word mode): each
/// boundary between rows is the midpoint of their gap mapped through the lane's word clocks.
pub fn timed_allocations(
    group: &TextGroup,
    lane: &PreparedLane,
) -> Result<Vec<String>, AllocationError> {
    validate_group(group)?;
    let source = normalize_text(&group.full_text);
    let segments = &group.segments;
    let range = TimeRange::new(
        segments[0].start_seconds,
        segments[segments.len() - 1].end_seconds,
    );
    let mut offsets = Vec::with_capacity(segments.len() - 1);
    for pair in segments.windows(2) {
        let boundary = (pair[0].end_seconds + pair[1].start_seconds) / 2.0;
        offsets.push(
            offset_at_time(&source, lane, range, boundary)
                .ok_or(AllocationError::MissingTimingAnchors)?,
        );
    }
    let chars: Vec<char> = source.chars().collect();
    let allocations = slice_chunks(&chars, segments.len(), |index, cursor| {
        nearest_word_boundary(&chars, offsets[index] as f64, cursor, chars.len())
    });
    if !allocations_preserve_text(group, &allocations) {
        return Err(AllocationError::LostText);
    }
    Ok(allocations)
}

/// Sentence units (`splitSentenceUnits`): `(?:^[.!?]+)?[^.!?]+[.!?]+(?:\s*\{[^}]+\}[.!?]*)?`
/// matched globally, plus the unterminated tail.
fn sentence_units(text: &str) -> Vec<String> {
    let source: Vec<char> = normalize_text(text).chars().collect();
    let terminator = |c: char| matches!(c, '.' | '!' | '?');
    let mut units = Vec::new();
    let mut start = 0;
    let mut consumed = 0;
    while start < source.len() {
        let mut at = start;
        if start == 0 {
            while at < source.len() && terminator(source[at]) {
                at += 1;
            }
        }
        let body = at;
        while at < source.len() && !terminator(source[at]) {
            at += 1;
        }
        if at == body {
            start += 1;
            continue;
        }
        if at == source.len() {
            // No terminator follows: no later start can match either.
            break;
        }
        while at < source.len() && terminator(source[at]) {
            at += 1;
        }
        let mut tag = at;
        while tag < source.len() && source[tag].is_whitespace() {
            tag += 1;
        }
        if source.get(tag) == Some(&'{')
            && let Some(close) = source[tag + 1..].iter().position(|c| *c == '}')
            && close > 0
        {
            at = tag + 1 + close + 1;
            while at < source.len() && terminator(source[at]) {
                at += 1;
            }
        }
        units.push(normalize_text(
            &source[start..at].iter().collect::<String>(),
        ));
        consumed = at;
        start = at;
    }
    let tail = normalize_text(&source[consumed..].iter().collect::<String>());
    if !tail.is_empty() {
        units.push(tail);
    }
    units
}

fn duration(segment: &AllocationSegment) -> f64 {
    let duration = segment.end_seconds - segment.start_seconds;
    if duration.is_finite() && duration > 0.0 {
        duration
    } else {
        0.001
    }
}

fn duration_cost(char_count: f64, duration: f64, chars_per_second: f64) -> f64 {
    let expected = duration * chars_per_second;
    let normalized = (char_count - expected) / expected.max(12.0);
    normalized * normalized
}

/// Untimed allocation (`createAutoSegmentTextRedistributionDraft`): whole sentences by
/// duration when there are enough sentences, else whitespace boundaries by duration.
pub fn sentence_allocations(group: &TextGroup) -> Result<Vec<String>, AllocationError> {
    validate_group(group)?;
    let source = normalize_text(&group.full_text);
    let chars: Vec<char> = source.chars().collect();
    let segments = &group.segments;
    let count = segments.len();
    let durations: Vec<f64> = segments.iter().map(duration).collect();
    let total: f64 = durations.iter().sum();
    let units = sentence_units(&source);
    let by_duration = || {
        let mut elapsed = 0.0;
        slice_chunks(&chars, count, |index, cursor| {
            elapsed += durations[index];
            let ideal = elapsed / total * chars.len() as f64;
            nearest_word_boundary(&chars, ideal, cursor, chars.len() - (count - index - 1))
        })
    };
    let allocations = if units.len() < count {
        by_duration()
    } else {
        let unit_count = units.len();
        // lengths[k]: characters of the first k units, each followed by one joining space.
        let mut lengths = vec![0usize];
        for unit in &units {
            lengths.push(lengths.last().unwrap() + unit.chars().count() + 1);
        }
        let chars_per_second = (lengths[unit_count] - 1) as f64 / total;
        let mut table = vec![vec![1e12f64; unit_count + 1]; count + 1];
        let mut previous = vec![vec![usize::MAX; unit_count + 1]; count + 1];
        table[0][0] = 0.0;
        for segment_index in 1..=count {
            let max_units = unit_count - (count - segment_index);
            for unit_end in segment_index..=max_units {
                for unit_start in segment_index - 1..unit_end {
                    let char_count = (lengths[unit_end] - lengths[unit_start] - 1) as f64;
                    let cost = table[segment_index - 1][unit_start]
                        + duration_cost(char_count, durations[segment_index - 1], chars_per_second);
                    if cost < table[segment_index][unit_end] {
                        table[segment_index][unit_end] = cost;
                        previous[segment_index][unit_end] = unit_start;
                    }
                }
            }
        }
        if previous[count][unit_count] == usize::MAX {
            by_duration()
        } else {
            let mut chunks = Vec::with_capacity(count);
            let mut cursor = unit_count;
            for segment_index in (1..=count).rev() {
                let next = previous[segment_index][cursor];
                chunks.push(units[next..cursor].join(" "));
                cursor = next;
            }
            chunks.reverse();
            chunks
        }
    };
    if !allocations_preserve_text(group, &allocations) {
        return Err(AllocationError::LostText);
    }
    Ok(allocations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::timing::TimingToken;

    fn lane(words: &[(&str, f64, f64)]) -> PreparedLane {
        PreparedLane::new(
            &words
                .iter()
                .map(|(text, start, end)| TimingToken {
                    text: text.to_string(),
                    start_seconds: *start,
                    end_seconds: *end,
                })
                .collect::<Vec<_>>(),
        )
    }

    fn group(text: &str, rows: &[(f64, f64)]) -> TextGroup {
        TextGroup {
            lane_id: "lane".into(),
            full_text: text.into(),
            segments: rows
                .iter()
                .enumerate()
                .map(|(index, (start, end))| AllocationSegment {
                    id: format!("row-{index}"),
                    start_seconds: *start,
                    end_seconds: *end,
                })
                .collect(),
        }
    }

    #[test]
    fn smart_split_prefers_completed_words_then_falls_back_to_ratio() {
        let range = TimeRange::new(0.0, 4.0);
        let clocks = lane(&[("один", 0.1, 0.5), ("два", 0.6, 0.9), ("три", 3.0, 3.5)]);
        let parts = smart_split_text("Один, два. Три", Some(&clocks), range, 2.0).unwrap();
        assert_eq!(
            (parts.first.as_str(), parts.second.as_str()),
            ("Один, два.", "Три")
        );
        assert_eq!(parts.source, SplitSource::CompletedWord);

        let parts = smart_split_text("Один, два. Три", None, range, 2.0).unwrap();
        assert_eq!(parts.source, SplitSource::WordRatio);
        assert_eq!(
            (parts.first.as_str(), parts.second.as_str()),
            ("Один, два.", "Три")
        );

        assert!(smart_split_text("text", None, range, 4.0).is_none());
    }

    #[test]
    fn timed_allocation_follows_clocks_and_conserves_text() {
        let clocks = lane(&[
            ("а", 0.0, 0.3),
            ("бы", 0.4, 0.8),
            ("в", 2.2, 2.4),
            ("где", 2.5, 2.9),
        ]);
        let group = group("а бы  в где", &[(0.0, 1.0), (2.0, 3.0)]);
        assert_eq!(
            timed_allocations(&group, &clocks).unwrap(),
            vec!["а бы".to_string(), "в где".to_string()]
        );
        let unrelated = lane(&[("нет", 0.0, 0.3)]);
        assert_eq!(
            timed_allocations(&group, &unrelated),
            Err(AllocationError::MissingTimingAnchors)
        );
    }

    #[test]
    fn sentence_allocation_keeps_sentences_whole_and_text_intact() {
        let group = group(
            "Первое предложение. Второе! {laugh} Третье тут?",
            &[(0.0, 2.0), (2.5, 3.0), (3.5, 5.0)],
        );
        let allocations = sentence_allocations(&group).unwrap();
        assert_eq!(
            allocations,
            vec![
                "Первое предложение.".to_string(),
                "Второе! {laugh}".to_string(),
                "Третье тут?".to_string()
            ]
        );
        assert!(allocations_preserve_text(&group, &allocations));
    }
}
