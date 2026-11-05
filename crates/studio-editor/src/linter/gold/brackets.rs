//! Non-speech bracket tag (`[laugh]`) and curly bracket rules.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::{UnicodeNormalization, is_nfc};

use super::curly_tags::mask_comments;
use super::js::{is_blank, is_letter_mark_or_number, is_ws, regex, replace_each, trim, utf16_len};
use super::{GoldLanguage, Reason, fixed, reasons};

/// Characters that may follow a non-speech tag directly (Babel `tw`).
const TAG_FOLLOWERS: [char; 41] = [
    '.', ',', '!', '?', ';', ':', '\u{2026}', '\u{3001}', '\u{3002}', '\u{FF01}', '\u{FF0C}',
    '\u{FF0E}', '\u{FF1A}', '\u{FF1B}', '\u{FF1F}', '\u{060C}', '\u{061B}', '\u{061F}', '\u{06D4}',
    '\u{0964}', '\u{0965}', '\u{104A}', '\u{104B}', '\u{17D4}', '\u{17D5}', '\u{0589}', '\u{1362}',
    '\u{037E}', '\u{0E5A}', '\u{0E5B}', '\u{2019}', '\u{201D}', '\u{BB}', ')', '\u{FF09}',
    '\u{3009}', '\u{300B}', '\u{300D}', '\u{300F}', '\u{3011}', '\u{3015}',
];

struct BracketReasons {
    empty: &'static str,
    unclosed: &'static str,
}

/// Valid tag names and the reason for a tag outside them.
struct KnownTags<'s> {
    tags: &'s HashSet<&'static str>,
    invalid: &'static str,
}

/// Babel `validateBrackets`: the first unopened close, spaced, empty or unknown tag, else the first
/// unclosed open.
fn validate(
    text: &str,
    open: &str,
    close: &str,
    reasons: BracketReasons,
    known: Option<KnownTags<'_>>,
) -> Reason {
    let mut opened = Vec::new();
    for (index, character) in text.char_indices() {
        let unit = &text[index..index + character.len_utf8()];
        if unit == open {
            opened.push(index + unit.len());
        } else if unit == close {
            let Some(start) = opened.pop() else {
                return fixed(reasons.unclosed);
            };
            let inner = &text[start..index];
            let trimmed = trim(inner);
            if inner != trimmed {
                return fixed(&super::reasons().bracket_spacing);
            }
            if trimmed.is_empty() {
                return fixed(reasons.empty);
            }
            if let Some(KnownTags { tags, invalid }) = &known {
                let valid = if is_nfc(trimmed) {
                    tags.contains(trimmed)
                } else {
                    tags.contains(trimmed.nfc().collect::<String>().as_str())
                };
                if !valid {
                    return fixed(invalid);
                }
            }
        }
    }
    if opened.is_empty() {
        None
    } else {
        fixed(reasons.unclosed)
    }
}

pub(super) fn bracket_validation(language: &GoldLanguage, text: &str) -> Reason {
    let reasons = reasons();
    language.bracket_types.iter().find_map(|bracket| {
        validate(
            text,
            bracket.open,
            bracket.close,
            BracketReasons {
                empty: &reasons.empty_bracket,
                unclosed: &reasons.unclosed_bracket,
            },
            Some(KnownTags {
                tags: &language.bracket_tag_set,
                invalid: &reasons.invalid_bracket_tag,
            }),
        )
    })
}

pub(super) fn curly_bracket_validation(_: &GoldLanguage, text: &str) -> Reason {
    let reasons = reasons();
    validate(
        &mask_comments(text),
        "{",
        "}",
        BracketReasons {
            empty: &reasons.empty_curly_bracket,
            unclosed: &reasons.unclosed_curly_bracket,
        },
        None,
    )
}

/// Babel `checkNssTagSpacing`: a bracket tag glued to the text before or after it.
pub(super) fn nss_tag_spacing(language: &GoldLanguage, text: &str) -> Reason {
    for bracket in &language.bracket_types {
        let mut next = text.find(bracket.open);
        while let Some(start) = next {
            let after_open = start + bracket.open.len();
            let Some(close) = text[after_open..].find(bracket.close) else {
                break;
            };
            let end = after_open + close + bracket.close.len();
            if missing_space_before(text, start) || missing_space_after(text, end) {
                return fixed(&reasons().nss_tag_spacing);
            }
            next = text[end..].find(bracket.open).map(|offset| end + offset);
        }
    }
    None
}

fn missing_space_before(text: &str, start: usize) -> bool {
    text[..start]
        .chars()
        .next_back()
        .is_some_and(|before| !is_ws(before))
}

fn missing_space_after(text: &str, end: usize) -> bool {
    let rest = &text[end..];
    let mut characters = rest.chars();
    match characters.next() {
        None => false,
        Some('"' | '\'') => characters.next().is_some_and(is_letter_mark_or_number),
        Some(after) => !rest.starts_with("--") && !is_ws(after) && !TAG_FOLLOWERS.contains(&after),
    }
}

static STYLE_TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"</?[^>]+>"));
static CURLY_SPAN: LazyLock<Regex> = LazyLock::new(|| regex(r"\{\{[^{}]*\}\}|\{[^}]*\}"));
static ALL_PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| regex(r"^\p{P}+$"));

/// Babel `checkTagOnlyPunctuation`: nothing but tags and punctuation.
pub(super) fn tag_only_punctuation(language: &GoldLanguage, text: &str) -> Reason {
    if is_blank(text) {
        return None;
    }
    let mut patterns: Vec<&Regex> = vec![&STYLE_TAG, &CURLY_SPAN];
    patterns.extend(&language.patterns().bracket_spans);
    let masked = replace_each(text, &patterns, |tag| " ".repeat(utf16_len(tag)));
    if masked == text {
        return None;
    }
    let rest: String = masked
        .chars()
        .filter(|character| !is_ws(*character))
        .collect();
    if !rest.is_empty() && ALL_PUNCTUATION.is_match(&rest) {
        fixed(&reasons().tag_only_punctuation)
    } else {
        None
    }
}
