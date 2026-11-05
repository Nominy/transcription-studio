//! Chinese and Japanese rules.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use super::js::regex;
use super::{GoldLanguage, Reason, data, fixed, reasons};

/// Code point ranges Babel treats as CJK (`t_`).
const CJK_RANGES: [(u32, u32); 14] = [
    (12289, 12303),
    (12306, 12351),
    (12352, 12447),
    (12448, 12543),
    (13312, 19903),
    (19968, 40959),
    (63744, 64255),
    (65281, 65295),
    (65306, 65312),
    (65340, 65340),
    (65342, 65344),
    (65371, 65439),
    (131072, 196607),
    (196608, 262143),
];

fn is_cjk(character: char) -> bool {
    let code = u32::from(character);
    CJK_RANGES
        .iter()
        .any(|&(start, end)| (start..=end).contains(&code))
}

/// Babel `checkCjkHalfWidthSpace`: an ASCII space between two CJK characters.
pub(super) fn cjk_half_width_space(_: &GoldLanguage, text: &str) -> Reason {
    let found = text.match_indices(' ').any(|(index, _)| {
        text[..index].chars().next_back().is_some_and(is_cjk)
            && text[index + 1..].chars().next().is_some_and(is_cjk)
    });
    if found {
        fixed(&reasons().cjk_half_width_space)
    } else {
        None
    }
}

static CURLY_SPAN: LazyLock<Regex> = LazyLock::new(|| regex(r"\{\{[^{}]*\}\}|\{[^}]*\}"));

/// Babel `checkFullWidthPunctuationLint`: ASCII punctuation in a ZH or JA segment, except an
/// ellipsis, a separator between digits, or punctuation touching Latin letters.
pub(super) fn full_width_punctuation(language: &GoldLanguage, text: &str) -> Reason {
    if !matches!(language.language, "JA" | "ZH") {
        return None;
    }
    let characters: Vec<char> = CURLY_SPAN.replace_all(text, "").chars().collect();
    let found = characters.iter().enumerate().any(|(index, &character)| {
        if !matches!(character, '.' | '!' | '?' | ',' | ';' | ':') {
            return false;
        }
        if character == '.' && in_ellipsis(&characters, index) {
            return false;
        }
        let before = index.checked_sub(1).map(|before| characters[before]);
        let after = characters.get(index + 1).copied();
        let between_digits =
            before.is_some_and(|c| c.is_ascii_digit()) && after.is_some_and(|c| c.is_ascii_digit());
        if matches!(character, '.' | ',' | ':') && between_digits {
            return false;
        }
        let touches_latin = before.is_some_and(|c| c.is_ascii_alphabetic())
            || after.is_some_and(|c| c.is_ascii_alphabetic());
        !(matches!(character, '.' | ',' | '!' | '?') && touches_latin)
    });
    if found {
        fixed(&reasons().full_width_punctuation)
    } else {
        None
    }
}

/// The dot at `index` belongs to a run of three or more dots.
fn in_ellipsis(characters: &[char], index: usize) -> bool {
    let first = characters[..index]
        .iter()
        .rev()
        .take_while(|&&c| c == '.')
        .count();
    let last = characters[index + 1..]
        .iter()
        .take_while(|&&c| c == '.')
        .count();
    first + 1 + last >= 3
}

static TRADITIONAL: LazyLock<HashSet<char>> =
    LazyLock::new(|| data().traditional_chinese.chars().collect());

/// Babel `checkTraditionalChineseLint`: Traditional characters in a ZH segment.
pub(super) fn traditional_chinese(language: &GoldLanguage, text: &str) -> Reason {
    if language.language != "ZH" {
        return None;
    }
    let mut found: Vec<char> = Vec::new();
    for character in text.chars() {
        if TRADITIONAL.contains(&character) && !found.contains(&character) {
            found.push(character);
        }
    }
    if found.is_empty() {
        return None;
    }
    let listed: Vec<String> = found.iter().map(char::to_string).collect();
    Some(Cow::Owned(format!(
        "Found Traditional character(s): {}. Transcripts must use Simplified Chinese.",
        listed.join(" ")
    )))
}
