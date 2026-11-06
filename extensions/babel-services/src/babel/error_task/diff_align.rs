//! Error-diff text/audio alignment, ported from babel-babel-runtime `error-diff.mjs` (the
//! code Gold and the Helper run in the browser). Span offsets are JavaScript UTF-16 code-unit
//! offsets, `[start, end)`, exactly as Babel stores them; all slicing goes through UTF-16.
//!
//! Matching uses only lexical words (letters/numbers + attached marks, NFC, lower case,
//! `ё`→`е`); markup (`<…>`, `{…}`, `[…]`) and punctuation are boundaries. Only matches shared
//! by every maximum monotonic word alignment count as evidence; no fuzzy matching and no
//! interpolated timestamps.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

/// One 8 kHz sample: clip-relative clock tolerance (absolute clip bounds lose precision on
/// subtraction and captures round to samples).
const SAMPLE_TOLERANCE: f64 = 1.0 / 8000.0 + 1e-9;
/// Longest ASR / comparison text accepted anywhere in the error-task protocol (UTF-16 units).
pub const MAX_TEXT_UNITS: usize = 16_000;

/// JavaScript `\s` (ECMAScript WhiteSpace + LineTerminator).
pub(crate) fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// JavaScript `String.prototype.trim`.
pub(crate) fn js_trim(text: &str) -> &str {
    text.trim_matches(js_space)
}

/// JavaScript `String.length`.
pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// JavaScript `text.slice(start, end)` on UTF-16 units (lone surrogates become U+FFFD).
pub(crate) fn slice16(units: &[u16], start: usize, end: usize) -> String {
    String::from_utf16_lossy(&units[start..end])
}

/// UTF-16 offset of every byte boundary of `text` (index = byte offset).
fn utf16_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0; text.len() + 1];
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        offsets[byte] = units;
        units += c.len_utf16();
    }
    offsets[text.len()] = units;
    offsets
}

static NON_SPACE_RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[^\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]+",
    )
    .expect("valid whitespace-word pattern")
});
static LEXICAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<[^>]*>|\{[^}]*\}|\[[^\]]*\]|[\p{L}\p{N}][\p{L}\p{N}\p{M}]*")
        .expect("valid lexical-word pattern")
});

/// Whitespace-delimited words (`/\S+/gu`) as UTF-16 `[start, end)` ranges.
fn space_words(text: &str) -> Vec<(usize, usize)> {
    let offsets = utf16_offsets(text);
    NON_SPACE_RUN
        .find_iter(text)
        .map(|found| (offsets[found.start()], offsets[found.end()]))
        .collect()
}

/// One side of a normalized comparison. `start`/`end` are UTF-16 offsets into `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSide {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

impl DiffSide {
    pub fn new(text: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            text: text.into(),
            start,
            end,
        }
    }

    /// `(before, highlighted, after)` split at the span (UTF-16 slicing).
    pub fn parts(&self) -> (String, String, String) {
        let units: Vec<u16> = self.text.encode_utf16().collect();
        let start = self.start.min(units.len());
        let end = self.end.clamp(start, units.len());
        (
            slice16(&units, 0, start),
            slice16(&units, start, end),
            slice16(&units, end, units.len()),
        )
    }
}

/// The normalized human/machine pair of a transcription error (`*NormalizedText`,
/// `*NormalizedSpanStart`, `*NormalizedSpanEnd`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDiff {
    pub human: DiffSide,
    pub machine: DiffSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SideName {
    Human,
    Machine,
}

impl SideName {
    fn label(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Machine => "machine",
        }
    }
}

impl NormalizedDiff {
    fn side(&self, name: SideName) -> &DiffSide {
        match name {
            SideName::Human => &self.human,
            SideName::Machine => &self.machine,
        }
    }

    fn side_mut(&mut self, name: SideName) -> &mut DiffSide {
        match name {
            SideName::Human => &mut self.human,
            SideName::Machine => &mut self.machine,
        }
    }
}

const SIDES: [SideName; 2] = [SideName::Human, SideName::Machine];

/// Seconds interval (`{startSeconds, endSeconds}`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioInterval {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl AudioInterval {
    pub fn new(start_seconds: f64, end_seconds: f64) -> Self {
        Self {
            start_seconds,
            end_seconds,
        }
    }

    pub fn duration(&self) -> f64 {
        self.end_seconds - self.start_seconds
    }
}

/// A clip-relative timed ASR item (may contain several lexical words).
#[derive(Debug, Clone, PartialEq)]
pub struct TimedWord {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// Jointly aligned text window (UTF-16 offsets into the uncropped normalized texts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextWindow {
    pub human_start: usize,
    pub human_end: usize,
    pub machine_start: usize,
    pub machine_end: usize,
}

impl TextWindow {
    fn bounds(&self, name: SideName) -> (usize, usize) {
        match name {
            SideName::Human => (self.human_start, self.human_end),
            SideName::Machine => (self.machine_start, self.machine_end),
        }
    }
}

/// Why an alignment is an enclosing fallback rather than a precisely located diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryNote {
    /// Overlapping provider word timings bound an empty gap; the enclosing words are used.
    OverlappingAsrWords,
    /// The whole (≤ 10 s) native segment; no word-level location.
    SegmentOnly,
}

/// Absolute recording timestamps of a located diff and its padded context.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffAlignment {
    pub diff_start_seconds: f64,
    pub diff_end_seconds: f64,
    pub context_start_seconds: f64,
    pub context_end_seconds: f64,
    pub text_window: TextWindow,
    pub boundary_note: Option<BoundaryNote>,
    /// ASR text of the context window (whole provider items).
    pub asr_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipDirection {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AlignError {
    /// Invalid input (spans, intervals, timestamps).
    #[error("{0}")]
    Invalid(String),
    /// No trustworthy alignment in this evidence.
    #[error("{0}")]
    Miss(String),
    /// The needed context lies outside the clip; continue from observed words.
    #[error("Unable to align diff: ASR evidence needs {} context", match direction {
        ClipDirection::After => "later",
        ClipDirection::Before => "earlier",
    })]
    ClipMiss {
        direction: ClipDirection,
        anchor_start_seconds: f64,
        anchor_end_seconds: f64,
    },
}

impl AlignError {
    /// `instanceof ErrorDiffAlignmentMiss` (a clip miss is also a miss).
    pub fn is_miss(&self) -> bool {
        matches!(self, Self::Miss(_) | Self::ClipMiss { .. })
    }

    fn invalid(message: &str) -> Self {
        Self::Invalid(message.to_string())
    }

    fn miss(message: &str) -> Self {
        Self::Miss(message.to_string())
    }
}

struct ReadSide {
    name: SideName,
    units: Vec<u16>,
    start: usize,
    end: usize,
}

fn read_side(diff: &NormalizedDiff, name: SideName) -> Result<ReadSide, AlignError> {
    let side = diff.side(name);
    let units: Vec<u16> = side.text.encode_utf16().collect();
    if side.start > side.end || side.end > units.len() {
        return Err(AlignError::Invalid(format!(
            "Invalid {} normalized diff span",
            name.label()
        )));
    }
    Ok(ReadSide {
        name,
        units,
        start: side.start,
        end: side.end,
    })
}

fn validate_clip(clip: AudioInterval) -> Result<(), AlignError> {
    if !clip.start_seconds.is_finite()
        || !clip.end_seconds.is_finite()
        || clip.start_seconds < 0.0
        || clip.end_seconds <= clip.start_seconds
    {
        return Err(AlignError::invalid(
            "Audio interval must have finite increasing nonnegative times",
        ));
    }
    Ok(())
}

/// Token index range `[first, last)` covered by `[start, end)`; a zero-width edit inside a
/// word belongs to that word, at a boundary to neither neighbor.
fn span_words(words: &[(usize, usize)], start: usize, end: usize) -> (usize, usize) {
    let first = words
        .iter()
        .position(|word| word.1 > start)
        .unwrap_or(words.len());
    let mut last = first;
    while last < words.len() && words[last].0 < end {
        last += 1;
    }
    if start == end && first < words.len() && words[first].0 < start {
        last = first + 1;
    }
    (first, last)
}

fn context_window(side: &ReadSide, text: &str, word_count: usize) -> (usize, usize) {
    let words = space_words(text);
    let (first, last) = span_words(&words, side.start, side.end);
    let from = first.saturating_sub(word_count);
    let to = (last + word_count).min(words.len());
    let left = if from == 0 {
        0
    } else {
        side.start
            .min(words.get(from).map_or(side.start, |word| word.0))
    };
    let right = if to == words.len() {
        side.units.len()
    } else {
        side.end.max(
            to.checked_sub(1)
                .and_then(|index| words.get(index))
                .map_or(side.end, |word| word.1),
        )
    };
    (left, right)
}

fn crop_side(result: &mut NormalizedDiff, side: &ReadSide, left: usize, right: usize) {
    let prefix = if left > 0 { "… " } else { "" };
    let suffix = if right < side.units.len() { " …" } else { "" };
    let prefix_units = utf16_len(prefix);
    let target = result.side_mut(side.name);
    target.text = format!("{prefix}{}{suffix}", slice16(&side.units, left, right));
    target.start = prefix_units + side.start - left;
    target.end = prefix_units + side.end - left;
}

/// Crops both normalized texts to `word_count` whitespace words around the highlight
/// (partially selected words included, `… ` / ` …` marking omissions) and rebases the spans.
pub fn crop_error_diff_context(
    diff: &NormalizedDiff,
    word_count: usize,
) -> Result<NormalizedDiff, AlignError> {
    let mut result = diff.clone();
    for name in SIDES {
        let side = read_side(diff, name)?;
        let (left, right) = context_window(&side, &diff.side(name).text, word_count);
        crop_side(&mut result, &side, left, right);
    }
    Ok(result)
}

/// Crops to the jointly aligned window, keeping the native highlight.
pub fn crop_aligned_error_diff_context(
    diff: &NormalizedDiff,
    alignment: &DiffAlignment,
) -> Result<NormalizedDiff, AlignError> {
    let mut result = diff.clone();
    for name in SIDES {
        let side = read_side(diff, name)?;
        let (left, right) = alignment.text_window.bounds(name);
        if left > side.start || right < side.end || right > side.units.len() {
            return Err(AlignError::invalid(
                "Invalid aligned text window: it must contain the original highlight",
            ));
        }
        crop_side(&mut result, &side, left, right);
    }
    Ok(result)
}

/// Rough proportional provider crop (≈ 4 s per flank, at most `min(max_seconds, 10)` s,
/// clamped to the segment). Never a final alignment.
pub fn estimate_error_diff_clip(
    diff: &NormalizedDiff,
    segment: AudioInterval,
    max_seconds: f64,
) -> Result<AudioInterval, AlignError> {
    validate_clip(segment)?;
    if !max_seconds.is_finite() || max_seconds <= 0.0 {
        return Err(AlignError::invalid(
            "maxSeconds must be positive and finite",
        ));
    }
    let mut sides = Vec::new();
    for name in SIDES {
        let side = read_side(diff, name)?;
        if !side.units.is_empty() {
            sides.push(side);
        }
    }
    if sides.is_empty() {
        return Err(AlignError::invalid("Cannot estimate an empty diff"));
    }
    let count = sides.len() as f64;
    let ratio = |pick: fn(&ReadSide) -> usize| {
        sides
            .iter()
            .map(|side| pick(side) as f64 / side.units.len() as f64)
            .sum::<f64>()
            / count
    };
    let duration = segment.duration();
    let estimated_start = segment.start_seconds + duration * ratio(|side| side.start);
    let estimated_end = segment.start_seconds + duration * ratio(|side| side.end);
    let width = 10f64
        .min(max_seconds)
        .min(estimated_end - estimated_start + 8.0);
    let center = (estimated_start + estimated_end) / 2.0;
    Ok(AudioInterval {
        start_seconds: segment.start_seconds.max(center - width / 2.0),
        end_seconds: segment.end_seconds.min(center + width / 2.0),
    })
}

#[derive(Debug, Clone)]
struct Lexical {
    value: String,
    start: usize,
    end: usize,
}

/// Lexical words of `text` with UTF-16 positions; tags are skipped.
fn lexical_words(text: &str) -> Vec<Lexical> {
    let offsets = utf16_offsets(text);
    LEXICAL
        .find_iter(text)
        .filter(|found| !found.as_str().starts_with(['<', '{', '[']))
        .map(|found| Lexical {
            value: found
                .as_str()
                .nfc()
                .collect::<String>()
                .to_lowercase()
                .replace('ё', "е"),
            start: offsets[found.start()],
            end: offsets[found.end()],
        })
        .collect()
}

#[derive(Debug, Clone)]
struct Timed {
    value: String,
    source_index: usize,
    start_seconds: f64,
    end_seconds: f64,
}

fn prepare_words(words: &[TimedWord], duration: f64) -> Result<Vec<Timed>, AlignError> {
    if words.is_empty() {
        return Err(AlignError::invalid("Alignment requires actual ASR words"));
    }
    let mut result = Vec::new();
    let mut previous_start = f64::NEG_INFINITY;
    for (source_index, word) in words.iter().enumerate() {
        let end_seconds = word.end_seconds.min(duration);
        if !word.start_seconds.is_finite()
            || !word.end_seconds.is_finite()
            || word.start_seconds < 0.0
            || word.end_seconds <= word.start_seconds
            || word.end_seconds > duration + SAMPLE_TOLERANCE
            || end_seconds <= word.start_seconds
            || word.start_seconds < previous_start
        {
            return Err(AlignError::invalid(
                "Invalid ASR word timestamps: expected ordered, positive intervals inside the clip",
            ));
        }
        previous_start = word.start_seconds;
        // A provider item with several lexical words keeps its WHOLE real interval for each.
        for token in lexical_words(&word.text) {
            result.push(Timed {
                value: token.value,
                source_index,
                start_seconds: word.start_seconds,
                end_seconds,
            });
        }
    }
    if result.is_empty() {
        return Err(AlignError::invalid("Alignment requires lexical ASR words"));
    }
    Ok(result)
}

/// Coarse fallback for a capture of the entire (≤ 10 s) native segment: no word timing,
/// text cropped to ten words around the highlight, `segment-only` disclosed.
pub fn segment_error_diff_context(
    diff: &NormalizedDiff,
    words: &[TimedWord],
    clip: AudioInterval,
    segment: AudioInterval,
) -> Result<DiffAlignment, AlignError> {
    validate_clip(clip)?;
    validate_clip(segment)?;
    if segment.duration() > 10.0
        || clip.duration() > 10.0
        || clip.start_seconds > segment.start_seconds + SAMPLE_TOLERANCE
        || clip.end_seconds < segment.end_seconds - SAMPLE_TOLERANCE
        || clip.start_seconds < segment.start_seconds - SAMPLE_TOLERANCE
        || clip.end_seconds > segment.end_seconds + SAMPLE_TOLERANCE
    {
        return Err(AlignError::invalid(
            "Segment fallback requires a full bounded native segment capture",
        ));
    }
    prepare_words(words, clip.duration())?;
    let asr_text = words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if js_trim(&asr_text).is_empty() || utf16_len(&asr_text) > MAX_TEXT_UNITS {
        return Err(AlignError::invalid("Invalid segment ASR text"));
    }
    let mut window = [0usize; 4];
    for (index, name) in SIDES.into_iter().enumerate() {
        let side = read_side(diff, name)?;
        let (left, right) = context_window(&side, &diff.side(name).text, 10);
        window[index * 2] = left;
        window[index * 2 + 1] = right;
    }
    Ok(DiffAlignment {
        diff_start_seconds: clip.start_seconds,
        diff_end_seconds: clip.end_seconds,
        context_start_seconds: clip.start_seconds,
        context_end_seconds: clip.end_seconds,
        text_window: TextWindow {
            human_start: window[0],
            human_end: window[1],
            machine_start: window[2],
            machine_end: window[3],
        },
        boundary_note: Some(BoundaryNote::SegmentOnly),
        asr_text,
    })
}

#[derive(Debug, Clone)]
struct Matches {
    /// Timed index → token index, or -1.
    map: Vec<isize>,
    count: usize,
    best: u32,
}

/// Matches shared by EVERY maximum monotonic exact-word alignment (prefix/suffix LCS).
fn stable_word_matches(
    tokens: &[u32],
    timed: &[u32],
    blocked: Option<&[bool]>,
) -> Result<Matches, AlignError> {
    let (n, m) = (tokens.len(), timed.len());
    let stride = m + 1;
    let cells = (n + 1) * stride;
    if cells > 4_000_000 {
        return Err(AlignError::invalid(
            "Alignment exceeds the bounded word-matching capacity",
        ));
    }
    let is_blocked = |j: usize| blocked.is_some_and(|blocked| blocked[j]);
    let matches = |i: usize, j: usize| !is_blocked(j) && tokens[i] == timed[j];
    let mut before = vec![0u32; cells];
    let mut after = vec![0u32; cells];
    for i in 0..n {
        for j in 0..m {
            before[(i + 1) * stride + j + 1] = if matches(i, j) {
                before[i * stride + j] + 1
            } else {
                before[i * stride + j + 1].max(before[(i + 1) * stride + j])
            };
        }
    }
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            after[i * stride + j] = if matches(i, j) {
                after[(i + 1) * stride + j + 1] + 1
            } else {
                after[(i + 1) * stride + j].max(after[i * stride + j + 1])
            };
        }
    }
    let best = before[n * stride + m];
    let mut map = vec![-1isize; m];
    let mut count = 0;
    for j in 0..m {
        if is_blocked(j) {
            continue;
        }
        let mut found: Option<usize> = None;
        let mut uncertain = false;
        for i in 0..=n {
            if before[i * stride + j] + after[i * stride + j + 1] == best {
                uncertain = true;
                break;
            }
            if i < n
                && tokens[i] == timed[j]
                && before[i * stride + j] + 1 + after[(i + 1) * stride + j + 1] == best
            {
                if found.is_some() {
                    uncertain = true;
                    break;
                }
                found = Some(i);
            }
        }
        if let (false, Some(index)) = (uncertain, found) {
            map[j] = index as isize;
            count += 1;
        }
    }
    Ok(Matches { map, count, best })
}

struct AlignSide {
    start: usize,
    end: usize,
    tokens: Vec<Lexical>,
    token_ids: Vec<u32>,
    first: usize,
    last: usize,
    matches: Matches,
}

impl AlignSide {
    fn empty(&self) -> bool {
        self.first == self.last
    }

    fn mapped(&self, j: usize) -> isize {
        self.matches.map[j]
    }
}

struct Anchors {
    common: Vec<usize>,
    left: Option<usize>,
    right: Option<usize>,
}

fn shared_anchors(sides: &[AlignSide; 2], count: usize) -> Anchors {
    let mut anchors = Anchors {
        common: Vec::new(),
        left: None,
        right: None,
    };
    for j in 0..count {
        if sides.iter().any(|side| side.mapped(j) < 0) {
            continue;
        }
        anchors.common.push(j);
        if sides
            .iter()
            .all(|side| side.mapped(j) <= side.first as isize - isize::from(side.empty()))
        {
            anchors.left = Some(j);
        }
        if anchors.right.is_none()
            && sides
                .iter()
                .all(|side| side.mapped(j) >= side.last as isize - isize::from(!side.empty()))
        {
            anchors.right = Some(j);
        }
    }
    anchors
}

/// `[from, to)` of timed words matching the whole (nonempty) highlight of `side`.
fn full_target_match(side: &AlignSide) -> Option<(usize, usize)> {
    if side.empty() {
        return None;
    }
    let mut reverse = vec![-1isize; side.tokens.len()];
    for (j, &index) in side.matches.map.iter().enumerate() {
        if index >= 0 {
            reverse[index as usize] = j as isize;
        }
    }
    if (side.first..side.last).any(|i| reverse[i] < 0) {
        return None;
    }
    Some((
        reverse[side.first] as usize,
        reverse[side.last - 1] as usize + 1,
    ))
}

/// An insertion's matched context on the gap side must not place it elsewhere.
fn target_agrees_with_gap(target: (usize, usize), side: &AlignSide) -> bool {
    side.matches.map.iter().enumerate().all(|(j, &index)| {
        index < 0
            || !((index < side.first as isize && j >= target.0)
                || (index >= side.last as isize && j < target.1))
    })
}

fn can_share_context(human: &AlignSide, machine: &AlignSide, direction: ClipDirection) -> bool {
    let outside = |side: &AlignSide| -> Vec<u32> {
        match direction {
            ClipDirection::After => side.token_ids[side.last..].to_vec(),
            ClipDirection::Before => side.token_ids[..side.first].to_vec(),
        }
    };
    let values: HashSet<u32> = outside(machine).into_iter().collect();
    outside(human).iter().any(|value| values.contains(value))
}

/// DP-aligns normalized lexical words against timestamped clip ASR and returns the smallest
/// jointly anchored window around the paired spans, padded by up to three ASR words per side
/// (whole provider items kept). Times are absolute (clip start added).
pub fn align_error_diff_context(
    diff: &NormalizedDiff,
    words: &[TimedWord],
    clip: AudioInterval,
) -> Result<DiffAlignment, AlignError> {
    validate_clip(clip)?;
    let timed = prepare_words(words, clip.duration())?;
    let mut interned: HashMap<String, u32> = HashMap::new();
    let mut intern = |value: &str| -> u32 {
        let next = interned.len() as u32;
        *interned.entry(value.to_string()).or_insert(next)
    };
    let timed_ids: Vec<u32> = timed.iter().map(|word| intern(&word.value)).collect();
    let mut build = |name: SideName| -> Result<AlignSide, AlignError> {
        let read = read_side(diff, name)?;
        let tokens = lexical_words(&diff.side(name).text);
        let token_ids: Vec<u32> = tokens.iter().map(|token| intern(&token.value)).collect();
        let ranges: Vec<(usize, usize)> = tokens
            .iter()
            .map(|token| (token.start, token.end))
            .collect();
        let (first, last) = span_words(&ranges, read.start, read.end);
        let matches = stable_word_matches(&token_ids, &timed_ids, None)?;
        Ok(AlignSide {
            start: read.start,
            end: read.end,
            tokens,
            token_ids,
            first,
            last,
            matches,
        })
    };
    let mut sides = [build(SideName::Human)?, build(SideName::Machine)?];
    let count = timed.len();
    let mut anchors = shared_anchors(&sides, count);
    if !matches!((anchors.left, anchors.right), (Some(left), Some(right)) if right >= left) {
        // A recognized selected word has no counterpart in an explicit empty span: it must not
        // steal an equally good match from a later repeated context word.
        for side_index in 0..2 {
            let (side, other) = (&sides[side_index], &sides[1 - side_index]);
            if !side.empty() || other.empty() {
                continue;
            }
            let blocked: Vec<bool> = (0..count)
                .map(|j| {
                    let index = other.mapped(j);
                    side.mapped(j) < 0
                        && index >= other.first as isize
                        && index < other.last as isize
                })
                .collect();
            if blocked.iter().any(|&value| value) {
                let rematched =
                    stable_word_matches(&sides[side_index].token_ids, &timed_ids, Some(&blocked))?;
                sides[side_index].matches = rematched;
            }
        }
        anchors = shared_anchors(&sides, count);
    }
    let [human, machine] = &sides;

    let (from, to, mut context_from, mut context_to);
    match (anchors.left, anchors.right) {
        (Some(left), Some(right)) if right >= left => {
            let in_highlight = |j: usize| {
                sides.iter().any(|side| {
                    side.mapped(j) >= side.first as isize && side.mapped(j) < side.last as isize
                })
            };
            from = left + usize::from(!in_highlight(left));
            to = right + usize::from(in_highlight(right));
            context_from = left;
            context_to = right + 1;
        }
        _ => {
            // A complete, uniquely mapped spoken target can be timed at a source or crop
            // boundary, including an insertion opposite an explicit empty span.
            let targets: Vec<(usize, usize)> = sides.iter().filter_map(full_target_match).collect();
            if let Some(&target) = targets.first() {
                if sides
                    .iter()
                    .any(|side| side.empty() && !target_agrees_with_gap(target, side))
                {
                    return Err(AlignError::miss(
                        "Ambiguous diff alignment: spoken target conflicts with the other transcript",
                    ));
                }
                if targets.iter().any(|other| *other != target) {
                    return Err(AlignError::miss(
                        "Ambiguous diff alignment: transcript variants identify different spoken intervals",
                    ));
                }
                (from, to) = target;
                context_from = anchors.left.map_or(from, |left| left.min(from));
                context_to = anchors.right.map_or(to, |right| (right + 1).max(to));
            } else {
                let common = &anchors.common;
                let supported =
                    common.len() >= 2 && sides.iter().any(|side| side.matches.count * 2 >= count);
                if supported {
                    let before: Vec<usize> = common
                        .iter()
                        .copied()
                        .filter(|&j| {
                            sides
                                .iter()
                                .all(|side| side.mapped(j) < side.first as isize)
                        })
                        .collect();
                    let after: Vec<usize> = common
                        .iter()
                        .copied()
                        .filter(|&j| {
                            sides
                                .iter()
                                .all(|side| side.mapped(j) >= side.last as isize)
                        })
                        .collect();
                    if before.len() == common.len()
                        && can_share_context(human, machine, ClipDirection::After)
                    {
                        return Err(AlignError::ClipMiss {
                            direction: ClipDirection::After,
                            anchor_start_seconds: clip.start_seconds
                                + timed[before[before.len() - 2]].start_seconds,
                            anchor_end_seconds: clip.start_seconds
                                + timed[before[before.len() - 1]].end_seconds,
                        });
                    }
                    if after.len() == common.len()
                        && can_share_context(human, machine, ClipDirection::Before)
                    {
                        return Err(AlignError::ClipMiss {
                            direction: ClipDirection::Before,
                            anchor_start_seconds: clip.start_seconds
                                + timed[after[0]].start_seconds,
                            anchor_end_seconds: clip.start_seconds + timed[after[1]].end_seconds,
                        });
                    }
                }
                if sides
                    .iter()
                    .any(|side| side.matches.best as usize > side.matches.count)
                {
                    return Err(AlignError::miss(
                        "Ambiguous diff alignment: maximum word alignments disagree",
                    ));
                }
                return Err(AlignError::miss(
                    "Unable to align diff: no trustworthy enclosing word window",
                ));
            }
        }
    }

    let shared_item = from == to
        && from > 0
        && to < count
        && timed[from - 1].source_index == timed[to].source_index;
    let (start, boundary_note);
    let mut end;
    if from < to {
        start = timed[from].start_seconds;
        end = timed[from].end_seconds;
        for word in &timed[from + 1..to] {
            end = end.max(word.end_seconds);
        }
        boundary_note = None;
    } else if shared_item {
        start = timed[to].start_seconds;
        end = timed[to].end_seconds;
        boundary_note = None;
    } else {
        if from == 0 || to == count {
            return Err(AlignError::miss(
                "Unable to align a gap without two real word boundaries",
            ));
        }
        let (gap_start, gap_end) = (timed[from - 1].end_seconds, timed[to].start_seconds);
        if gap_end < gap_start {
            // Overlapping provider timings cannot define a gap: return the enclosing real
            // word intervals instead of inventing a boundary.
            start = timed[from - 1].start_seconds;
            end = timed[from - 1].end_seconds.max(timed[to].end_seconds);
            boundary_note = Some(BoundaryNote::OverlappingAsrWords);
        } else {
            start = gap_start;
            end = gap_end;
            boundary_note = None;
        }
    }
    // Keep whole provider items in audio and text; never invent subword timestamps.
    let widen_to_items = |from: &mut usize, to: &mut usize| {
        while *from > 0
            && *from < count
            && timed[*from - 1].source_index == timed[*from].source_index
        {
            *from -= 1;
        }
        while *to < count && *to > 0 && timed[*to].source_index == timed[*to - 1].source_index {
            *to += 1;
        }
    };
    widen_to_items(&mut context_from, &mut context_to);
    // Comfort context around the resolved container, clamped to existing ASR evidence.
    context_from = context_from.saturating_sub(3);
    context_to = (context_to + 3).min(count);
    widen_to_items(&mut context_from, &mut context_to);

    let mut window = [0usize; 4];
    for (index, side) in sides.iter().enumerate() {
        let (mut first, mut last) = (side.first, side.last);
        for j in context_from..context_to {
            let mapped = side.mapped(j);
            if mapped >= 0 {
                first = first.min(mapped as usize);
                last = last.max(mapped as usize + 1);
            }
        }
        window[index * 2] = side.start.min(
            side.tokens
                .get(first)
                .map_or(side.start, |token| token.start),
        );
        window[index * 2 + 1] = side.end.max(
            last.checked_sub(1)
                .and_then(|index| side.tokens.get(index))
                .map_or(side.end, |token| token.end),
        );
    }
    let mut context_start = start;
    let mut context_end = end;
    for word in &timed[context_from..context_to] {
        context_start = context_start.min(word.start_seconds);
        context_end = context_end.max(word.end_seconds);
    }
    let asr_text = (timed[context_from].source_index..=timed[context_to - 1].source_index)
        .map(|index| words[index].text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(DiffAlignment {
        diff_start_seconds: clip.start_seconds + start,
        diff_end_seconds: clip.start_seconds + end,
        context_start_seconds: clip.start_seconds + context_start,
        context_end_seconds: clip.start_seconds + context_end,
        text_window: TextWindow {
            human_start: window[0],
            human_end: window[1],
            machine_start: window[2],
            machine_end: window[3],
        },
        boundary_note,
        asr_text,
    })
}

/// A bounded search continuation from observed words (inside the segment, ≤ 10 s).
pub fn next_error_diff_clip(
    miss: &AlignError,
    clip: AudioInterval,
    segment: AudioInterval,
) -> Result<AudioInterval, AlignError> {
    validate_clip(clip)?;
    validate_clip(segment)?;
    let AlignError::ClipMiss {
        direction,
        anchor_start_seconds: anchor_start,
        anchor_end_seconds: anchor_end,
    } = *miss
    else {
        return Err(AlignError::invalid("Invalid observed continuation anchor"));
    };
    if !anchor_start.is_finite()
        || !anchor_end.is_finite()
        || anchor_start < clip.start_seconds
        || anchor_end > clip.end_seconds
        || anchor_end <= anchor_start
    {
        return Err(AlignError::invalid("Invalid observed continuation anchor"));
    }
    let next = match direction {
        ClipDirection::After => AudioInterval {
            start_seconds: segment.start_seconds.max(anchor_start),
            end_seconds: segment.end_seconds.min(anchor_start + 10.0),
        },
        ClipDirection::Before => AudioInterval {
            start_seconds: segment.start_seconds.max(anchor_end - 10.0),
            end_seconds: segment.end_seconds.min(anchor_end),
        },
    };
    let stuck = match direction {
        ClipDirection::After => next.end_seconds <= clip.end_seconds + 0.001,
        ClipDirection::Before => next.start_seconds >= clip.start_seconds - 0.001,
    };
    if next.end_seconds <= next.start_seconds || stuck {
        return Err(AlignError::invalid(
            "Unable to extend the ASR window within the native segment",
        ));
    }
    Ok(next)
}

/// `validErrorDiffAlignment`: a well-formed alignment of at most ~10 s.
pub fn valid_alignment(alignment: &DiffAlignment) -> bool {
    let window = alignment.text_window;
    let times = [
        alignment.diff_start_seconds,
        alignment.diff_end_seconds,
        alignment.context_start_seconds,
        alignment.context_end_seconds,
    ];
    window.human_start <= window.human_end
        && window.machine_start <= window.machine_end
        && !js_trim(&alignment.asr_text).is_empty()
        && utf16_len(&alignment.asr_text) <= MAX_TEXT_UNITS
        && times.iter().all(|time| time.is_finite())
        && alignment.context_start_seconds >= 0.0
        && alignment.context_start_seconds <= alignment.diff_start_seconds
        && alignment.diff_start_seconds <= alignment.diff_end_seconds
        && alignment.diff_end_seconds <= alignment.context_end_seconds
        && alignment.context_end_seconds > alignment.context_start_seconds
        && alignment.context_end_seconds - alignment.context_start_seconds <= 10.001
}
