//! Transcript tokenizer (Babel Helper `tokenizeTranscriptText`) and the whitespace-floored text
//! splits shared by smart split and auto-segmentation. Offsets are `char` offsets (Helper's
//! UTF-16 offsets for BMP text).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Word,
    Space,
    /// A whole `<…>`, `{…}` or `[…]` tag.
    Tag,
    Punctuation,
    Symbol,
}

/// `[start, end)` in `char` offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextToken {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

const TAG_DELIMITERS: [(char, char); 3] = [('<', '>'), ('{', '}'), ('[', ']')];

/// `[\p{L}\p{N}\p{M}_]`.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || is_combining_mark(c)
}

fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{0483}'..='\u{0489}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

fn is_punctuation(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | '?' | '!' | ':' | ';' | '"' | '\'' | '(' | ')' | '-'
    )
}

fn is_word_continuation(chars: &[char], index: usize) -> bool {
    let c = chars[index];
    is_word_char(c)
        || (c == '-'
            && index > 0
            && is_word_char(chars[index - 1])
            && chars.get(index + 1).copied().is_some_and(is_word_char))
}

/// Tokenizes `chars` into words (letters/digits/marks/`_`, inner hyphens), whitespace runs,
/// whole tags, single punctuation marks and single symbols.
pub fn tokenize(chars: &[char]) -> Vec<TextToken> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if let Some((_, close)) = TAG_DELIMITERS.iter().find(|(open, _)| *open == c)
            && let Some(close_offset) = chars[index + 1..].iter().position(|x| x == close)
        {
            let end = index + 1 + close_offset + 1;
            tokens.push(TextToken {
                kind: TokenKind::Tag,
                start: index,
                end,
            });
            index = end;
            continue;
        }
        let start = index;
        let kind = if c.is_whitespace() {
            while index < chars.len() && chars[index].is_whitespace() {
                index += 1;
            }
            TokenKind::Space
        } else if is_word_char(c) {
            while index < chars.len() && is_word_continuation(chars, index) {
                index += 1;
            }
            TokenKind::Word
        } else {
            index += 1;
            if is_punctuation(c) {
                TokenKind::Punctuation
            } else {
                TokenKind::Symbol
            }
        };
        tokens.push(TextToken {
            kind,
            start,
            end: index,
        });
    }
    tokens
}

/// Lower-cases and folds `ё` to `е` (Helper `foldWord`).
pub fn fold_word(chars: &[char]) -> String {
    chars
        .iter()
        .flat_map(|c| c.to_lowercase())
        .map(|c| if c == 'ё' { 'е' } else { c })
        .collect()
}

/// A timing token's comparable word: the folded word when the token is exactly one word,
/// otherwise empty (unrecognized).
pub fn normalize_timing_word(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut words = tokenize(&chars)
        .into_iter()
        .filter(|token| token.kind == TokenKind::Word);
    match (words.next(), words.next()) {
        (Some(word), None) => fold_word(&chars[word.start..word.end]),
        _ => String::new(),
    }
}

/// Collapses whitespace runs to one space and trims (`normalizeAutoSegmentText`).
pub fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text without any whitespace, for conservation checks.
pub fn compact_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Where smart-split text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitSource {
    /// The row text was empty.
    Empty,
    /// The cut fell between two unambiguously matched words.
    CompletedWord,
    /// Interpolated between matched words.
    Interpolated,
    /// No usable timing: whitespace units by time ratio.
    WordRatio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitParts {
    pub first: String,
    pub second: String,
    /// Words on the first side.
    pub split_count: usize,
    pub word_count: usize,
    pub source: SplitSource,
}

/// Floors `index` to the start of the whitespace run at or before it (`floorWordBoundary`);
/// `min` when none.
pub fn floor_word_boundary(chars: &[char], index: f64, min: usize, max: usize) -> usize {
    let clamped = (index.floor().max(min as f64) as usize).min(max);
    if clamped >= max {
        return max;
    }
    let mut boundary = min;
    for token in tokenize(chars) {
        if token.start > clamped {
            break;
        }
        if token.kind != TokenKind::Space {
            continue;
        }
        let candidate = clamped.min(token.end - 1);
        if candidate >= min {
            boundary = candidate;
        }
    }
    boundary
}

/// The whitespace position nearest `index` within 80 characters (backward first on ties),
/// else `index` itself (`nearestWordBoundary`).
pub fn nearest_word_boundary(chars: &[char], index: f64, min: usize, max: usize) -> usize {
    let is_space = |at: isize| at >= 0 && chars.get(at as usize).is_some_and(|c| c.is_whitespace());
    let clamped = round_half_up(index).clamp(min as f64, max as f64) as isize;
    if is_space(clamped) {
        return clamped as usize;
    }
    for offset in 1..=80isize {
        if clamped - offset >= min as isize && is_space(clamped - offset) {
            return (clamped - offset) as usize;
        }
        if clamped + offset <= max as isize && is_space(clamped + offset) {
            return (clamped + offset) as usize;
        }
    }
    clamped as usize
}

/// JavaScript `Math.round`.
pub(crate) fn round_half_up(value: f64) -> f64 {
    (value + 0.5).floor()
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Splits raw `text` at `offset` floored to whitespace, trimming both halves
/// (`splitAutoSegmentTextAtFloorOffset`).
pub fn split_at_floor_offset(text: &str, offset: f64, source: SplitSource) -> SplitParts {
    let chars: Vec<char> = text.chars().collect();
    let boundary = floor_word_boundary(&chars, offset, 0, chars.len());
    let first: String = chars[..boundary].iter().collect();
    let second: String = chars[boundary..].iter().collect();
    let (first, second) = (first.trim().to_string(), second.trim().to_string());
    let split_count = word_count(&first);
    SplitParts {
        word_count: split_count + word_count(&second),
        first,
        second,
        split_count,
        source,
    }
}

/// Ratio fallback (`splitAutoSegmentTextByWordRatio`): whitespace-separated units (tags kept
/// whole) rounded by `ratio`; an interior ratio keeps at least one unit on each side.
pub fn split_by_word_ratio(text: &str, ratio: f64) -> SplitParts {
    let chars: Vec<char> = text.chars().collect();
    let mut starts = Vec::new();
    let mut in_unit = false;
    for token in tokenize(&chars) {
        if token.kind == TokenKind::Space {
            in_unit = false;
            continue;
        }
        if !in_unit {
            starts.push(token.start);
        }
        in_unit = true;
    }
    let fraction = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut count = round_half_up(starts.len() as f64 * fraction) as usize;
    if fraction > 0.0 && fraction < 1.0 && starts.len() > 1 {
        count = count.clamp(1, starts.len() - 1);
    }
    let offset = if count == 0 {
        0
    } else if count >= starts.len() {
        chars.len()
    } else {
        starts[count]
    };
    split_at_floor_offset(text, offset as f64, SplitSource::WordRatio)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(TokenKind, String)> {
        let chars: Vec<char> = text.chars().collect();
        tokenize(&chars)
            .into_iter()
            .map(|t| (t.kind, chars[t.start..t.end].iter().collect()))
            .collect()
    }

    #[test]
    fn tokenizer_keeps_tags_whole_and_hyphenated_words_together() {
        assert_eq!(
            kinds("Кто-то {laugh} пришёл, да."),
            vec![
                (TokenKind::Word, "Кто-то".to_string()),
                (TokenKind::Space, " ".to_string()),
                (TokenKind::Tag, "{laugh}".to_string()),
                (TokenKind::Space, " ".to_string()),
                (TokenKind::Word, "пришёл".to_string()),
                (TokenKind::Punctuation, ",".to_string()),
                (TokenKind::Space, " ".to_string()),
                (TokenKind::Word, "да".to_string()),
                (TokenKind::Punctuation, ".".to_string()),
            ]
        );
    }

    #[test]
    fn timing_word_normalization_folds_yo_and_rejects_multi_word_tokens() {
        assert_eq!(normalize_timing_word("Ёлка,"), "елка");
        assert_eq!(normalize_timing_word("две части"), "");
        assert_eq!(normalize_timing_word("…"), "");
    }

    #[test]
    fn ratio_split_keeps_tag_payloads_intact() {
        let parts = split_by_word_ratio("one {two three} four", 0.5);
        assert_eq!(parts.first, "one {two three}");
        assert_eq!(parts.second, "four");
        let parts = split_by_word_ratio("one two", 0.01);
        assert_eq!(
            (parts.first.as_str(), parts.second.as_str()),
            ("one", "two")
        );
    }
}
