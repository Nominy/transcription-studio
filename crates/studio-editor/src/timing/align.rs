//! Exact-word alignment between a row's current text and its lane's timing tokens (Babel
//! Helper `l0-word-timing-alignment.ts`). Words are compared lower-cased with `ё`→`е`; a
//! longest common subsequence gives monotonic anchors; time↔character mapping pins matched
//! word boundaries and interpolates only between neighbouring anchors.

use super::text::{
    TextToken, TokenKind, fold_word, normalize_timing_word, round_half_up, tokenize,
};

/// `[start, end)` seconds of a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeRange {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl TimeRange {
    pub fn new(start_seconds: f64, end_seconds: f64) -> Self {
        Self {
            start_seconds,
            end_seconds,
        }
    }

    fn is_valid(self) -> bool {
        self.start_seconds.is_finite()
            && self.end_seconds.is_finite()
            && self.end_seconds > self.start_seconds
    }
}

/// A timing token with its comparable word (empty when unrecognized).
#[derive(Debug, Clone, PartialEq)]
pub struct TimedWord {
    pub normalized: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl TimedWord {
    fn overlaps(&self, range: TimeRange) -> bool {
        self.start_seconds < range.end_seconds && self.end_seconds > range.start_seconds
    }
}

/// One lane's tokens, normalized and ordered by (start, end); unrecognized tokens kept.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreparedLane {
    tokens: Vec<TimedWord>,
}

impl PreparedLane {
    pub fn new(tokens: &[studio_core::timing::TimingToken]) -> Self {
        let mut tokens: Vec<TimedWord> = tokens
            .iter()
            .map(|token| TimedWord {
                normalized: normalize_timing_word(&token.text),
                start_seconds: token.start_seconds,
                end_seconds: token.end_seconds,
            })
            .collect();
        // Stable, like Array.prototype.sort.
        tokens.sort_by(|left, right| {
            left.start_seconds
                .total_cmp(&right.start_seconds)
                .then(left.end_seconds.total_cmp(&right.end_seconds))
        });
        Self { tokens }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Every token overlapping `range`, recognized or not, in time order.
    fn overlapping(&self, range: TimeRange) -> impl Iterator<Item = &TimedWord> {
        self.tokens.iter().filter(move |word| word.overlaps(range))
    }

    /// Recognized words overlapping `range`.
    fn words_in(&self, range: TimeRange) -> Vec<&TimedWord> {
        self.overlapping(range)
            .filter(|word| !word.normalized.is_empty())
            .collect()
    }
}

#[derive(Debug, Clone)]
struct TextWord {
    normalized: String,
    start: usize,
    end: usize,
}

fn text_words(chars: &[char], tokens: &[TextToken]) -> Vec<TextWord> {
    tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Word)
        .map(|token| TextWord {
            normalized: fold_word(&chars[token.start..token.end]),
            start: token.start,
            end: token.end,
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct ControlPoint {
    time: f64,
    offset: usize,
}

fn append_point(points: &mut Vec<ControlPoint>, time: f64, offset: usize) {
    if let Some(previous) = points.last_mut() {
        if time < previous.time {
            return;
        }
        if time == previous.time {
            previous.offset = previous.offset.max(offset);
            return;
        }
    }
    points.push(ControlPoint { time, offset });
}

/// First index in `start..len` where `before(index)` is false.
fn lower_bound(len: usize, start: usize, before: impl Fn(usize) -> bool) -> usize {
    let (mut low, mut high) = (start, len);
    while low < high {
        let middle = (low + high) / 2;
        if before(middle) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

/// `table[i][j]`: longest exact-word match between the first `i` text words and `j` clocks.
fn prefix_match_lengths(text: &[TextWord], timed: &[&TimedWord]) -> Vec<Vec<u32>> {
    let mut lengths = vec![vec![0u32; timed.len() + 1]; text.len() + 1];
    for i in 0..text.len() {
        for j in 0..timed.len() {
            lengths[i + 1][j + 1] = if text[i].normalized == timed[j].normalized {
                lengths[i][j] + 1
            } else {
                lengths[i][j + 1].max(lengths[i + 1][j])
            };
        }
    }
    lengths
}

/// A maximum monotonic sequence of exact-word matches as `(text index, timed index)`.
fn find_anchors(text: &[TextWord], timed: &[&TimedWord]) -> Vec<(usize, usize)> {
    if text.is_empty() || timed.is_empty() {
        return Vec::new();
    }
    let lengths = prefix_match_lengths(text, timed);
    let mut anchors = Vec::new();
    let (mut i, mut j) = (text.len(), timed.len());
    while i > 0 && j > 0 {
        if text[i - 1].normalized == timed[j - 1].normalized {
            anchors.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if lengths[i - 1][j] >= lengths[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    anchors.reverse();
    anchors
}

/// Alignment of one row's text against its lane's clocks.
#[derive(Debug, Clone)]
pub struct Alignment {
    text_len: usize,
    range: TimeRange,
    words: Vec<TextWord>,
    /// Clamped start time of each text word that is anchored.
    word_starts: Vec<Option<f64>>,
    playback_points: Vec<ControlPoint>,
    /// Separate inverse mapping only when some anchor clock is reversed.
    seek_points: Option<Vec<ControlPoint>>,
}

impl Alignment {
    /// `None` when the text is empty, the range invalid, or no word matches.
    pub fn new(text: &str, lane: &PreparedLane, range: TimeRange) -> Option<Self> {
        if text.is_empty() || !range.is_valid() {
            return None;
        }
        let chars: Vec<char> = text.chars().collect();
        let words = text_words(&chars, &tokenize(&chars));
        let timed = lane.words_in(range);
        let anchors = find_anchors(&words, &timed);
        if anchors.is_empty() {
            return None;
        }
        let first = ControlPoint {
            time: range.start_seconds,
            offset: 0,
        };
        let mut playback_points = vec![first];
        let mut seek_points = anchors
            .iter()
            .any(|&(_, j)| timed[j].end_seconds < timed[j].start_seconds)
            .then(|| vec![first]);
        let clamp = |t: f64| t.min(range.end_seconds).max(range.start_seconds);
        let mut word_starts = vec![None; words.len()];
        for &(i, j) in &anchors {
            let start = clamp(timed[j].start_seconds);
            let end = clamp(timed[j].end_seconds);
            word_starts[i] = Some(start);
            append_point(&mut playback_points, start, words[i].start);
            append_point(&mut playback_points, start.max(end), words[i].end);
            if let Some(seek) = seek_points.as_mut() {
                append_point(seek, start, words[i].start);
                append_point(seek, end, words[i].end);
            }
        }
        append_point(&mut playback_points, range.end_seconds, chars.len());
        if let Some(seek) = seek_points.as_mut() {
            append_point(seek, range.end_seconds, chars.len());
        }
        Some(Self {
            text_len: chars.len(),
            range,
            words,
            word_starts,
            playback_points,
            seek_points,
        })
    }

    /// Time of a `char` offset: the start of the anchored word under it, else interpolated
    /// between anchors (`computePreparedL0TimestampAtCharacterOffset`).
    pub fn time_at_offset(&self, char_offset: usize) -> f64 {
        let offset = char_offset.min(self.text_len);
        let word_index = lower_bound(self.words.len(), 0, |i| self.words[i].end < offset);
        if let Some(word) = self.words.get(word_index)
            && offset >= word.start
            && let Some(start) = self.word_starts[word_index]
        {
            return start;
        }
        let points = self.seek_points.as_ref().unwrap_or(&self.playback_points);
        let index = lower_bound(points.len(), 1, |i| points[i].offset < offset);
        if index == points.len() {
            return self.range.end_seconds;
        }
        let (left, right) = (points[index - 1], points[index]);
        if right.offset <= left.offset {
            return right.time;
        }
        let ratio = (offset - left.offset) as f64 / (right.offset - left.offset) as f64;
        left.time + ratio * (right.time - left.time)
    }

    /// `char` offset reached at `time` (`computePreparedL0TimedCharacterOffset`).
    pub fn offset_at_time(&self, time: f64) -> usize {
        let points = &self.playback_points;
        let time = time
            .min(self.range.end_seconds)
            .max(self.range.start_seconds);
        let index = lower_bound(points.len(), 1, |i| points[i].time < time);
        if index == points.len() {
            return self.text_len;
        }
        let (left, right) = (points[index - 1], points[index]);
        if right.time <= left.time {
            return right.offset.min(self.text_len);
        }
        let ratio = (time - left.time) / (right.time - left.time);
        let offset =
            round_half_up(left.offset as f64 + ratio * (right.offset as f64 - left.offset as f64));
        (offset.max(0.0) as usize).min(self.text_len)
    }
}

/// Time of the word at `char_offset` (Alt+Click seek); `None` without a matched word.
pub fn timestamp_at_offset(
    text: &str,
    lane: &PreparedLane,
    range: TimeRange,
    char_offset: usize,
) -> Option<f64> {
    Some(Alignment::new(text, lane, range)?.time_at_offset(char_offset))
}

/// `char` offset spoken at `time` (ghost cursor, interpolated text placement).
pub fn offset_at_time(
    text: &str,
    lane: &PreparedLane,
    range: TimeRange,
    time: f64,
) -> Option<usize> {
    if !time.is_finite() {
        return None;
    }
    Some(Alignment::new(text, lane, range)?.offset_at_time(time))
}

/// The exact text boundary for a cut at `cut_seconds` when every maximum exact-word
/// alignment agrees locally (`computeL0CutBoundaryCharacterOffset`): completed words go left,
/// the word being spoken goes right, adjacent punctuation/whole tags travel with the
/// completed word. `None` when ambiguous (repeated words, overlapping clocks) or unmatched.
pub fn cut_boundary_offset(
    text: &str,
    lane: &PreparedLane,
    range: TimeRange,
    cut_seconds: f64,
) -> Option<usize> {
    if text.is_empty()
        || !cut_seconds.is_finite()
        || !range.is_valid()
        || cut_seconds < range.start_seconds
        || cut_seconds > range.end_seconds
    {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let text_tokens = tokenize(&chars);
    let words = text_words(&chars, &text_tokens);
    // Keep unrecognized timed tokens: silently dropping one would invent adjacency.
    let timed: Vec<&TimedWord> = lane.overlapping(range).collect();
    let (n, m) = (words.len(), timed.len());
    if n == 0
        || m == 0
        || timed.iter().any(|word| {
            !word.start_seconds.is_finite()
                || !word.end_seconds.is_finite()
                || word.end_seconds <= word.start_seconds
        })
    {
        return None;
    }
    let before = prefix_match_lengths(&words, &timed);
    let mut after = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            after[i][j] = if words[i].normalized == timed[j].normalized {
                after[i + 1][j + 1] + 1
            } else {
                after[i + 1][j].max(after[i][j + 1])
            };
        }
    }
    let best = before[n][m];
    if best == 0 {
        return None;
    }
    let required_text_anchor = |j: usize| -> Option<usize> {
        let mut found = None;
        for i in 0..=n {
            // If an equally good alignment can omit this clock, it is not evidence.
            if before[i][j] + after[i][j + 1] == best {
                return None;
            }
            if i < n
                && words[i].normalized == timed[j].normalized
                && before[i][j] + 1 + after[i + 1][j + 1] == best
            {
                if found.is_some() {
                    return None;
                }
                found = Some(i);
            }
        }
        found
    };

    let right = timed
        .iter()
        .position(|word| word.end_seconds > cut_seconds)
        .unwrap_or(m);
    if right > 0 && right < m && timed[right - 1].end_seconds > timed[right].start_seconds {
        return None;
    }
    if right + 1 < m && timed[right + 1].start_seconds <= cut_seconds {
        return None;
    }
    let left_text = if right > 0 {
        required_text_anchor(right - 1)
    } else {
        None
    };
    let right_text = if right < m {
        required_text_anchor(right)
    } else {
        None
    };
    if right == 0 {
        return (right_text == Some(0)).then_some(0);
    }
    if right == m {
        return (left_text == Some(n - 1)).then_some(chars.len());
    }
    let left_text = left_text?;
    if right_text != Some(left_text + 1) {
        return None;
    }
    let mut offset = words[left_text].end;
    for token in &text_tokens {
        if token.end <= offset {
            continue;
        }
        match token.kind {
            TokenKind::Space => return Some(offset),
            TokenKind::Word => return None,
            _ => offset = token.end,
        }
    }
    Some(offset)
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

    const RANGE: TimeRange = TimeRange {
        start_seconds: 0.0,
        end_seconds: 4.0,
    };

    #[test]
    fn cut_boundary_puts_completed_words_left_and_carries_punctuation() {
        let lane = lane(&[("привет", 0.2, 0.8), ("мир", 1.0, 1.5), ("ёж", 2.0, 2.5)]);
        // An unmatched later word does not disturb a boundary between matched words.
        assert_eq!(
            cut_boundary_offset("Привет, мир ежик.", &lane, RANGE, 1.2),
            Some(7)
        );
        // Inside "мир": the active word goes right.
        let text = "Привет, мир еж.";
        assert_eq!(cut_boundary_offset(text, &lane, RANGE, 1.2), Some(7));
        assert_eq!(cut_boundary_offset(text, &lane, RANGE, 1.7), Some(11));
        assert_eq!(cut_boundary_offset(text, &lane, RANGE, 3.0), Some(15));
    }

    #[test]
    fn cut_boundary_rejects_ambiguous_repeated_words_and_overlap() {
        let repeated = lane(&[("да", 0.2, 0.5), ("да", 1.0, 1.4)]);
        assert_eq!(cut_boundary_offset("да да да", &repeated, RANGE, 0.8), None);
        let overlapping = lane(&[("один", 0.2, 1.2), ("два", 1.0, 1.6)]);
        assert_eq!(
            cut_boundary_offset("один два", &overlapping, RANGE, 1.3),
            None
        );
    }

    #[test]
    fn time_and_offset_mapping_pin_matched_words_and_interpolate_edits() {
        let lane = lane(&[("раз", 1.0, 1.4), ("три", 3.0, 3.5)]);
        let text = "раз два три";
        assert_eq!(timestamp_at_offset(text, &lane, RANGE, 1), Some(1.0));
        assert_eq!(timestamp_at_offset(text, &lane, RANGE, 9), Some(3.0));
        // "два" is unmatched: interpolated between the end of "раз" and the start of "три".
        let mid = timestamp_at_offset(text, &lane, RANGE, 5).unwrap();
        assert!(mid > 1.4 && mid < 3.0, "{mid}");
        assert_eq!(offset_at_time(text, &lane, RANGE, 3.0), Some(8));
        assert_eq!(offset_at_time(text, &lane, RANGE, 4.0), Some(11));
        assert_eq!(timestamp_at_offset("ничего общего", &lane, RANGE, 3), None);
    }
}
