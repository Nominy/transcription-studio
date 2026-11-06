//! Dash rules: double dashes (`--`), stammer dashes (`I- I`) and em/en dashes.

use std::sync::LazyLock;

use regex::Regex;

use super::js::{is_blank, is_ws, regex, replace_each, utf16_len};
use super::{GoldLanguage, Reason, data, fixed, reasons};

pub(super) fn triple_dash(_: &GoldLanguage, text: &str) -> Reason {
    if !is_blank(text) && text.contains("---") {
        fixed(&reasons().triple_dash)
    } else {
        None
    }
}

pub(super) fn spaced_double_dash(_: &GoldLanguage, text: &str) -> Reason {
    let spaced = text.char_indices().any(|(index, character)| {
        is_ws(character) && text[index + character.len_utf8()..].starts_with("--")
    });
    if !is_blank(text) && spaced {
        fixed(&reasons().spaced_double_dash)
    } else {
        None
    }
}

static DOUBLE_DASH_PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| regex("--[.,;:!?。、；：！？]"));

pub(super) fn double_dash_punctuation(_: &GoldLanguage, text: &str) -> Reason {
    if !is_blank(text) && DOUBLE_DASH_PUNCTUATION.is_match(text) {
        fixed(&reasons().double_dash_punctuation)
    } else {
        None
    }
}

/// Babel `checkDoubleDashMissingSpace`: `--` followed by something other than a space, a quote or
/// punctuation (or another dash, when the text has a `---` run).
pub(super) fn double_dash_missing_space(_: &GoldLanguage, text: &str) -> Reason {
    if is_blank(text) {
        return None;
    }
    let has_triple = text.contains("---");
    let missing = text.match_indices("--").any(|(index, _)| {
        let Some(after) = text[index + 2..].chars().next() else {
            return false;
        };
        if (has_triple && after == '-') || matches!(after, '.' | ',' | ';' | ':' | '!' | '?') {
            return false;
        }
        !matches!(
            after,
            ' ' | '\u{3000}' | '"' | '\'' | '\u{201D}' | '\u{201C}' | '\u{2019}'
        )
    });
    if missing {
        fixed(&reasons().double_dash_missing_space)
    } else {
        None
    }
}

static SQUARE_TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"\[[^\[\]]*\]"));
static FULL_WIDTH_SQUARE_TAG: LazyLock<Regex> = LazyLock::new(|| regex("［[^［］]*］"));
static LENTICULAR_TAG: LazyLock<Regex> = LazyLock::new(|| regex("【[^【】]*】"));
static CURLY_TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"\{\{[^{}]*\}\}|\{[^{}]*\}"));
static STYLE_TAG: LazyLock<Regex> = LazyLock::new(|| regex("</?[^<>]+>"));
static HYPHENATED: LazyLock<Regex> = LazyLock::new(|| regex(r"[\p{L}\p{N}]+(?:-[\p{L}\p{N}]+)+"));

fn object_replacements(text: &str) -> String {
    "\u{FFFC}".repeat(utf16_len(text))
}

/// Babel `checkSingleDashMissingSpace`: a stammer written `I-I` (the part after a dash starts with
/// the part before it), outside tags and known hyphenated words.
pub(super) fn single_dash_missing_space(_: &GoldLanguage, text: &str) -> Reason {
    if is_blank(text) {
        return None;
    }
    let masked = replace_each(
        text,
        &[
            &SQUARE_TAG,
            &FULL_WIDTH_SQUARE_TAG,
            &LENTICULAR_TAG,
            &CURLY_TAG,
            &STYLE_TAG,
        ],
        object_replacements,
    );
    let words = replace_each(&masked, &[&HYPHENATED], |word| {
        if data()
            .hyphenated_words
            .contains(word.to_lowercase().as_str())
        {
            object_replacements(word)
        } else {
            word.to_owned()
        }
    });
    let stammer = HYPHENATED.find_iter(&words).any(|word| {
        let parts: Vec<String> = word.as_str().split('-').map(str::to_lowercase).collect();
        parts.windows(2).any(|pair| {
            pair[1].starts_with(&pair[0]) && !(all_digits(&pair[0]) && all_digits(&pair[1]))
        })
    });
    if stammer {
        fixed(&reasons().single_dash_missing_space)
    } else {
        None
    }
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

pub(super) fn em_dash(_: &GoldLanguage, text: &str) -> Reason {
    if text.contains(['\u{2013}', '\u{2014}']) {
        fixed(&reasons().em_dash)
    } else {
        None
    }
}
