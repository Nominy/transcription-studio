//! JavaScript string semantics Babel's rules rely on.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{Captures, Regex};

/// JavaScript `\s`.
pub(super) const WS: &str = concat!("[", ws_chars!(), "]");

pub(super) fn regex(pattern: &str) -> Regex {
    Regex::new(pattern)
        .unwrap_or_else(|error| panic!("invalid Gold lint pattern `{pattern}`: {error}"))
}

/// JavaScript `\s`: Unicode spaces, line terminators and U+FEFF (not U+0085).
pub(super) fn is_ws(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

pub(super) fn trim(text: &str) -> &str {
    text.trim_matches(is_ws)
}

pub(super) fn trim_end(text: &str) -> &str {
    text.trim_end_matches(is_ws)
}

/// `text.trim() === ""`.
pub(super) fn is_blank(text: &str) -> bool {
    text.chars().all(is_ws)
}

/// JavaScript `length`.
pub(super) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// `text.slice(0, units)` followed by `"..."` when `text` is longer (UTF-16 code units).
pub(super) fn truncated(text: &str, units: usize) -> String {
    let mut used = 0;
    for (index, character) in text.char_indices() {
        used += character.len_utf16();
        if used > units {
            return format!("{}...", &text[..index]);
        }
    }
    text.to_owned()
}

/// `text.replace(pattern, (match) => " ".repeat(match.length))`.
pub(super) fn blank_utf16<'t>(pattern: &Regex, text: &'t str) -> Cow<'t, str> {
    pattern.replace_all(text, |captures: &Captures<'_>| {
        " ".repeat(utf16_len(&captures[0]))
    })
}

/// Applies `replace` to `text` once per pattern, in order.
pub(super) fn replace_each<'t>(
    text: &'t str,
    patterns: &[&Regex],
    replace: impl Fn(&str) -> String,
) -> Cow<'t, str> {
    let mut result = Cow::Borrowed(text);
    for pattern in patterns {
        if let Cow::Owned(replaced) =
            pattern.replace_all(&result, |captures: &Captures<'_>| replace(&captures[0]))
        {
            result = Cow::Owned(replaced);
        }
    }
    result
}

static LETTER_OR_NUMBER: LazyLock<Regex> = LazyLock::new(|| regex(r"[\p{L}\p{N}]"));
static LETTER_MARK_OR_NUMBER: LazyLock<Regex> = LazyLock::new(|| regex(r"^[\p{L}\p{M}\p{N}]$"));
static DECIMAL_DIGIT: LazyLock<Regex> = LazyLock::new(|| regex(r"\p{Nd}"));

/// `/[\p{L}\p{N}]/u.test(text)`.
pub(super) fn has_letter_or_number(text: &str) -> bool {
    LETTER_OR_NUMBER.is_match(text)
}

/// `/[\p{L}\p{M}\p{N}]/u.test(character)`.
pub(super) fn is_letter_mark_or_number(character: char) -> bool {
    LETTER_MARK_OR_NUMBER.is_match(character.encode_utf8(&mut [0; 4]))
}

/// `/\p{Nd}/u.test(text)`.
pub(super) fn has_decimal_digit(text: &str) -> bool {
    DECIMAL_DIGIT.is_match(text)
}

/// `[...new Set(values)].map((value) => `"${value}"`).join(", ")`.
pub(super) fn quoted_unique<'v>(values: impl IntoIterator<Item = &'v str>) -> String {
    let mut unique: Vec<&str> = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    quoted(unique)
}

/// `values.map((value) => `"${value}"`).join(", ")`.
pub(super) fn quoted<S: AsRef<str>>(values: impl IntoIterator<Item = S>) -> String {
    let quoted: Vec<String> = values
        .into_iter()
        .map(|value| format!("\"{}\"", value.as_ref()))
        .collect();
    quoted.join(", ")
}
