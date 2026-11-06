//! Babel's text heuristics: fillers, punctuation runs, repetition, banned characters and words.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use super::js::{
    is_blank, is_letter_mark_or_number, is_ws, quoted, quoted_unique, regex, replace_each,
    truncated, utf16_len,
};
use super::{GoldLanguage, Reason, data, fixed, reasons};

static SQUARE_TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"\[[^\]]*\]"));
static FULL_WIDTH_SQUARE_TAG: LazyLock<Regex> = LazyLock::new(|| regex("［[^］]*］"));
static LENTICULAR_TAG: LazyLock<Regex> = LazyLock::new(|| regex("【[^】]*】"));
static CURLY_SPAN: LazyLock<Regex> = LazyLock::new(|| regex(r"\{\{[^{}]*\}\}|\{[^}]*\}"));
static STYLE_TAG: LazyLock<Regex> = LazyLock::new(|| regex("</?[^>]+>"));

fn tag_patterns() -> [&'static Regex; 5] {
    [
        &SQUARE_TAG,
        &FULL_WIDTH_SQUARE_TAG,
        &LENTICULAR_TAG,
        &CURLY_SPAN,
        &STYLE_TAG,
    ]
}

/// Babel `checkFillerWordSpelling`: the first filler word with a non-standard spelling.
pub(super) fn filler_word_spelling(_: &GoldLanguage, text: &str) -> Reason {
    if is_blank(text) {
        return None;
    }
    let data = data();
    let masked = replace_each(text, &tag_patterns(), |tag| " ".repeat(utf16_len(tag)));
    for token in masked.split(is_ws).filter(|token| !token.is_empty()) {
        let word = token.trim_matches(|character: char| !character.is_ascii_alphanumeric());
        if word.is_empty() {
            continue;
        }
        let lower = word.to_lowercase();
        if let Some(standard) = data.filler_corrections.get(lower.as_str()) {
            return Some(Cow::Owned(format!(
                "Non-standard filler spelling \"{word}\" \u{2014} style guide suggests \"{standard}\"."
            )));
        }
        if let Some(suggestion) = data.filler_suggestions.get(lower.as_str()) {
            return Some(Cow::Owned(format!(
                "Non-standard filler spelling \"{word}\" \u{2014} style guide suggests {suggestion}."
            )));
        }
    }
    None
}

/// Ellipsis tokens (`eJ`).
const ELLIPSES: [&str; 3] = ["...", "\u{2026}", "\u{2026}\u{2026}"];
/// Question and exclamation tokens (`eX`).
const EXCLAMATIONS: [&str; 11] = [
    "?",
    "!",
    "\u{FF1F}",
    "\u{FF01}",
    "\u{061F}",
    "?!",
    "!?",
    "\u{FF1F}\u{FF01}",
    "\u{FF01}\u{FF1F}",
    "\u{061F}!",
    "!\u{061F}",
];

/// Babel `checkConsecutivePunctuationLint`: two or more punctuation tokens in a row (whitespace
/// between them allowed), except the combinations each language permits.
pub(super) fn consecutive_punctuation(language: &GoldLanguage, text: &str) -> Reason {
    let tokens = &language.patterns().punctuation_tokens;
    let spanish = language.language == "ES";
    let mut longest = 0;
    let mut run = 0;
    let mut previous: Option<&str> = None;
    let mut index = 0;
    while let Some(character) = text[index..].chars().next() {
        if is_ws(character) {
            index += character.len_utf8();
            continue;
        }
        let Some(&token) = tokens
            .iter()
            .find(|token| text[index..].starts_with(**token))
        else {
            run = 0;
            previous = None;
            index += character.len_utf8();
            continue;
        };
        let previous_is =
            |options: &[&str]| previous.is_some_and(|previous| options.contains(&previous));
        let opening = ["\u{BF}", "\u{A1}"];
        let allowed = (previous_is(&ELLIPSES) && EXCLAMATIONS.contains(&token))
            || (previous_is(&EXCLAMATIONS) && ELLIPSES.contains(&token))
            || (spanish
                && ((previous.is_some() && !previous_is(&opening) && opening.contains(&token))
                    || (previous_is(&opening) && ELLIPSES.contains(&token))
                    || (previous_is(&["?", "!"]) && token == ",")
                    || (previous == Some("\u{BF}") && token == "\u{A1}")
                    || (previous == Some("\u{A1}") && token == "\u{BF}")));
        if allowed {
            run = 1;
        } else {
            run += 1;
            longest = longest.max(run);
        }
        previous = Some(token);
        index += token.len();
    }
    if longest >= 2 {
        fixed(&reasons().consecutive_punctuation)
    } else {
        None
    }
}

const DUPLICATE_WINDOW: usize = 50;

/// Babel `checkIntraSegmentDuplicateLint`: a 50-character window repeated within a segment of 100+
/// characters (tags collapsed to one space), measured in UTF-16 code units.
pub(super) fn intra_segment_duplicate(_: &GoldLanguage, text: &str) -> Reason {
    let collapsed = replace_each(text, &tag_patterns(), |_| " ".to_owned());
    let units: Vec<u16> = collapsed.encode_utf16().collect();
    if units.len() < 100 {
        return None;
    }
    let mut seen: HashSet<&[u16]> = HashSet::new();
    let mut repeated: Vec<&[u16]> = Vec::new();
    for window in units.windows(DUPLICATE_WINDOW) {
        if window
            .iter()
            .all(|&unit| char::from_u32(u32::from(unit)).is_some_and(is_ws))
        {
            continue;
        }
        if !seen.insert(window) && !repeated.contains(&window) {
            repeated.push(window);
        }
    }
    let first = repeated.first()?;
    let more = if repeated.len() > 1 {
        format!(" (+{} more)", repeated.len() - 1)
    } else {
        String::new()
    };
    Some(Cow::Owned(format!(
        "Repeated {DUPLICATE_WINDOW}-char substring: \"{}...\"{more}",
        String::from_utf16_lossy(&first[..40])
    )))
}

static ESCAPED_QUOTE: LazyLock<Regex> = LazyLock::new(|| regex(r#"\\["']"#));
static BANNED_CHARACTERS: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"!==|<<=|>>=|\+=|-=|\*=|/=|<=|>=|&&|\|\||::|->|=>|<<|>>|=|\||\\|\^|±|×|÷|√|∞|≈|≠|≤|≥|∑|∏|∫|Δ|∂",
    )
});

/// Babel `checkBannedCharactersLint`: programming operators and math symbols outside style tags.
pub(super) fn banned_characters(_: &GoldLanguage, text: &str) -> Reason {
    let unescaped = ESCAPED_QUOTE.replace_all(text, "");
    let cleaned = STYLE_TAG.replace_all(&unescaped, " ");
    let mut found = BANNED_CHARACTERS
        .find_iter(&cleaned)
        .map(|found| found.as_str())
        .peekable();
    found.peek()?;
    Some(Cow::Owned(format!(
        "{} Found: {}",
        reasons().banned_characters,
        quoted_unique(found)
    )))
}

static PICTOGRAPH: LazyLock<Regex> = LazyLock::new(|| regex(r"\p{Extended_Pictographic}"));

pub(super) fn emoji(_: &GoldLanguage, text: &str) -> Reason {
    let mut found: Vec<&str> = Vec::new();
    for pictograph in PICTOGRAPH.find_iter(text) {
        if !found.contains(&pictograph.as_str()) {
            found.push(pictograph.as_str());
        }
    }
    if found.is_empty() {
        return None;
    }
    Some(Cow::Owned(format!(
        "{} Found: {}",
        reasons().emoji,
        found.join(" ")
    )))
}

static CAPITALS: LazyLock<Regex> = LazyLock::new(|| regex(r"\p{Lu}{5,}"));

/// Babel `checkConsecutiveCapitalsLint`, skipped for languages without letter case.
pub(super) fn consecutive_capitals(language: &GoldLanguage, text: &str) -> Reason {
    if data()
        .capitals_exempt_languages
        .iter()
        .any(|exempt| **exempt == *language.language)
    {
        return None;
    }
    let mut runs: Vec<&str> = Vec::new();
    for run in CAPITALS.find_iter(text) {
        if !runs.contains(&run.as_str()) {
            runs.push(run.as_str());
        }
    }
    if runs.is_empty() {
        return None;
    }
    let listed = quoted(runs.iter().map(|run| truncated(run, 20)));
    Some(Cow::Owned(format!(
        "{} Found 5+ consecutive uppercase letters: {listed}",
        reasons().consecutive_capitals
    )))
}

/// Babel `checkConsecutiveCharsLint`: a non-numeric character repeated five or more times
/// (`([^\p{N}])\1{4,}`).
pub(super) fn consecutive_chars(_: &GoldLanguage, text: &str) -> Reason {
    let mut runs: Vec<&str> = Vec::new();
    let mut characters = text.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        let mut end = start + character.len_utf8();
        let mut count = 1;
        while let Some(&(index, _)) = characters.peek().filter(|&&(_, next)| next == character) {
            end = index + character.len_utf8();
            count += 1;
            characters.next();
        }
        if count >= 5 && !character.is_numeric() {
            runs.push(&text[start..end]);
        }
    }
    if runs.is_empty() {
        return None;
    }
    let listed = quoted(runs.iter().map(|run| truncated(run, 10)));
    Some(Cow::Owned(format!(
        "{} Found 5+ consecutive identical chars: {listed}",
        reasons().consecutive_chars
    )))
}

static LONG_NUMBER: LazyLock<Regex> = LazyLock::new(|| regex(r"\p{N}{13,}"));

pub(super) fn excessive_digits(_: &GoldLanguage, text: &str) -> Reason {
    let numbers: Vec<String> = LONG_NUMBER
        .find_iter(text)
        .map(|number| truncated(number.as_str(), 20))
        .collect();
    if numbers.is_empty() {
        return None;
    }
    Some(Cow::Owned(format!(
        "{} Found 13+ digit sequence: {}",
        reasons().excessive_digits,
        quoted(numbers)
    )))
}

/// Babel `checkBannedWordsLint`: a banned word or its romanization, case-insensitively, not inside
/// a longer word (`(?<!\p{L}|\p{N}|\p{M})word(?!\p{L}|\p{N}|\p{M})`).
pub(super) fn banned_words(language: &GoldLanguage, text: &str) -> Reason {
    let found = language
        .patterns()
        .banned_words
        .iter()
        .filter(|(_, pattern)| contains_whole_word(pattern, text))
        .map(|(word, _)| *word);
    let mut found = found.peekable();
    found.peek()?;
    Some(Cow::Owned(format!(
        "{} Found: {}",
        reasons().banned_words,
        quoted_unique(found)
    )))
}

fn contains_whole_word(pattern: &Regex, text: &str) -> bool {
    let mut from = 0;
    while let Some(found) = pattern.find_at(text, from) {
        let free_before = !text[..found.start()]
            .chars()
            .next_back()
            .is_some_and(is_letter_mark_or_number);
        let free_after = !text[found.end()..]
            .chars()
            .next()
            .is_some_and(is_letter_mark_or_number);
        if free_before && free_after {
            return true;
        }
        let Some(next) = text[found.start()..].chars().next() else {
            return false;
        };
        from = found.start() + next.len_utf8();
    }
    false
}

pub(super) fn parentheses_usage(_: &GoldLanguage, text: &str) -> Reason {
    if text.contains(['(', ')']) {
        fixed(&reasons().parentheses)
    } else {
        None
    }
}

static MEASUREMENT: LazyLock<Regex> = LazyLock::new(|| {
    regex(&format!(
        "[0-9]+['\u{2018}\u{2019}]{}?[0-9]+[\"\u{201D}]",
        super::WS
    ))
});

/// Babel `checkUnpairedQuotes`: an odd number of double quotes, ignoring curly tags and
/// feet-and-inches measurements (`5'10"`).
pub(super) fn unpaired_quotes(_: &GoldLanguage, text: &str) -> Reason {
    let without_tags = CURLY_SPAN.replace_all(text, "");
    let without_measurements = MEASUREMENT.replace_all(&without_tags, "");
    let quotes = without_measurements
        .chars()
        .filter(|character| {
            matches!(
                character,
                '"' | '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{AB}' | '\u{BB}'
            )
        })
        .count();
    if quotes % 2 == 1 {
        fixed(&reasons().unpaired_quotes)
    } else {
        None
    }
}
