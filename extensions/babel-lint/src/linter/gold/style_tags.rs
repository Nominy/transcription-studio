//! Babel `checkStyleTags`: `<tag> content </tag>` syntax, names, pairing, spacing and punctuation.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::{UnicodeNormalization, is_nfc};

use super::js::{WS, is_blank, is_ws, regex, trim_end};
use super::{GoldLanguage, Reason, data, fixed, reasons};

static EMPTY_NAME: LazyLock<Regex> = LazyLock::new(|| regex(&format!("<{WS}*/?{WS}*>")));
static TAG: LazyLock<Regex> = LazyLock::new(|| regex(&format!("<{WS}*(/)?{WS}*([^>]+?){WS}*>")));
static ENDS_WITH_PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| regex(r#"[.!?,;:-](['"])?$"#));
static PUNCTUATION_AFTER: LazyLock<Regex> = LazyLock::new(|| regex(&format!("^{WS}+[.!?,;:]")));
static TAG_SPAN: LazyLock<Regex> = LazyLock::new(|| regex("</?[^>]+>"));

struct OpenTag<'t> {
    name: &'t str,
    start: usize,
    end: usize,
}

/// A matched tag pair: byte offsets of the opening `<`, the content and the closing `>`'s end.
struct TagPair {
    open_start: usize,
    content_start: usize,
    content_end: usize,
    close_end: usize,
}

pub(super) fn style_tag_validation(language: &GoldLanguage, text: &str) -> Reason {
    let reasons = reasons();
    if EMPTY_NAME.is_match(text) {
        return fixed(&reasons.style_tag_empty_name);
    }
    let mut open: Vec<OpenTag<'_>> = Vec::new();
    let mut pairs = Vec::new();
    for captures in TAG.captures_iter(text) {
        let Some(whole) = captures.get(0) else {
            continue;
        };
        let Some(raw_name) = captures.get(2) else {
            continue;
        };
        let closing = captures.get(1).is_some();
        let name = tag_name(language, raw_name.as_str(), closing);
        let tag = whole.as_str();
        if tag[1..].starts_with(is_ws) || tag[..tag.len() - 1].ends_with(is_ws) {
            return fixed(&reasons.style_tag_whitespace_in_syntax);
        }
        let known = if is_nfc(name) {
            language.style_tag_set.contains(name)
        } else {
            language
                .style_tag_set
                .contains(name.nfc().collect::<String>().as_str())
        };
        if !known {
            return fixed(&reasons.style_tag_invalid_name);
        }
        if closing {
            let Some(index) = open.iter().rposition(|tag| tag.name == name) else {
                return fixed(&reasons.style_tag_unmatched);
            };
            let opened = open.remove(index);
            pairs.push(TagPair {
                open_start: opened.start,
                content_start: opened.end,
                content_end: whole.start(),
                close_end: whole.end(),
            });
        } else {
            open.push(OpenTag {
                name,
                start: whole.start(),
                end: whole.end(),
            });
        }
    }
    if !open.is_empty() {
        return fixed(&reasons.style_tag_unmatched);
    }
    if let Some(reason) = pairs.iter().find_map(|pair| pair_issue(text, pair)) {
        return Some(reason);
    }
    let spans: Vec<(usize, usize)> = TAG_SPAN
        .find_iter(text)
        .map(|span| (span.start(), span.end()))
        .collect();
    let stray = text.char_indices().any(|(index, character)| {
        matches!(character, '<' | '>')
            && !spans
                .iter()
                .any(|&(start, end)| (start..end).contains(&index))
    });
    if stray {
        fixed(&reasons.style_tag_stray_bracket)
    } else {
        None
    }
}

/// Babel `normalizeStyleTagName`: an opening `FOREIGN="XX"` with a known language code names the
/// foreign tag.
fn tag_name<'t>(language: &GoldLanguage, name: &'t str, closing: bool) -> &'t str {
    if closing {
        return name;
    }
    let (Some(foreign), Some(coded)) = (
        language.foreign_style_tag,
        &language.patterns().coded_foreign_tag_name,
    ) else {
        return name;
    };
    match coded.captures(name) {
        Some(code) if data().language_codes.contains(&code[1]) => foreign,
        _ => name,
    }
}

/// Babel's per-pair checks: empty content, punctuation just after the closing tag, then missing
/// spaces around the tags or inside them.
fn pair_issue(text: &str, pair: &TagPair) -> Reason {
    let reasons = reasons();
    let content = &text[pair.content_start..pair.content_end];
    if is_blank(content) {
        return fixed(&reasons.style_tag_empty);
    }
    if !ENDS_WITH_PUNCTUATION.is_match(trim_end(content))
        && PUNCTUATION_AFTER.is_match(&text[pair.close_end..])
    {
        return fixed(&reasons.style_tag_punctuation_outside);
    }
    let is_space = |character: char| matches!(character, ' ' | '\u{3000}');
    let glued_before = text[..pair.open_start]
        .chars()
        .next_back()
        .is_some_and(|before| !is_space(before) && before != '>');
    let glued_after = text[pair.close_end..]
        .chars()
        .next()
        .is_some_and(|after| !is_space(after) && after != '<');
    let unspaced_start = content
        .chars()
        .next()
        .is_some_and(|first| !is_space(first) && !matches!(first, '<' | '[' | '\u{FF3B}'));
    let unspaced_end = content
        .chars()
        .next_back()
        .is_some_and(|last| !is_space(last) && !matches!(last, '>' | ']' | '\u{FF3D}'));
    if glued_before || glued_after || unspaced_start || unspaced_end {
        fixed(&reasons.style_tag_spacing)
    } else {
        None
    }
}
