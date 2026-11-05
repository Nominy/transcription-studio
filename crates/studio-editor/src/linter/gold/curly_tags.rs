//! `{MIS: word}` / `{PRO: word}` tag rules and the Arabic foreign-block marker.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

use super::js::{blank_utf16, has_decimal_digit, is_blank, regex};
use super::{GoldLanguage, Reason, data, fixed, reasons};

static REVIEWER_COMMENT: LazyLock<Regex> = LazyLock::new(|| regex(r"\{\{[^{}]+\}\}"));

/// Babel `L2`: blanks `{{reviewer comments}}`, keeping UTF-16 length.
pub(super) fn mask_comments(text: &str) -> Cow<'_, str> {
    blank_utf16(&REVIEWER_COMMENT, text)
}

/// Babel `checkUnbracketedAnnotationTags`: a MIS/PRO keyword with a colon outside a curly tag, or the
/// unintelligible tag outside its brackets.
pub(super) fn unbracketed_annotation_tags(language: &GoldLanguage, text: &str) -> Reason {
    let patterns = language.patterns();
    let reason = || Some(Cow::Owned(patterns.unbracketed_reason.clone()));
    if patterns.unbracketed_keyword.is_match(&mask_comments(text)) {
        return reason();
    }
    let mut rest = Cow::Borrowed(text);
    for bracketed in &patterns.bracketed_unintelligible {
        if let Cow::Owned(removed) = bracketed.replace_all(&rest, "") {
            rest = Cow::Owned(removed);
        }
    }
    if patterns.unintelligible.is_match(&rest) {
        reason()
    } else {
        None
    }
}

static EMPTY_COMMENT: LazyLock<Regex> = LazyLock::new(|| regex(r"\{\{\}\}"));
static CURLY: LazyLock<Regex> = LazyLock::new(|| regex(r"\{[^}]*\}"));
static LANGUAGE_MARKER: LazyLock<Regex> = LazyLock::new(|| regex(r"^\{([A-Z]{2})\}$"));
static FOLLOWING_TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"^[^{}\p{L}\p{N}]*(\{[^}]*\})"));

/// Babel `fI`: whether `tag` is a well-formed MIS/PRO tag for the language.
fn is_well_formed(language: &GoldLanguage, tag: &str) -> bool {
    let patterns = language.patterns();
    patterns.curly_tag.is_match(tag)
        || (language.is_arabic() && patterns.arabic_curly_tag.is_match(tag))
}

fn is_language_code(code: &str) -> bool {
    data().language_codes.contains(code)
}

/// Babel `checkMalformedCurlyBracketTags`: a malformed MIS/PRO tag, or a bare `{written}` brace not
/// followed by its tag.
pub(super) fn malformed_curly_bracket_tags(language: &GoldLanguage, text: &str) -> Reason {
    if !text.contains('{') {
        return None;
    }
    let masked = mask_comments(text);
    let masked = EMPTY_COMMENT.replace_all(&masked, "    ");
    for curly in CURLY.find_iter(&masked) {
        let tag = curly.as_str();
        if is_blank(&tag[1..tag.len() - 1]) {
            continue;
        }
        if tag.contains(':') {
            if !is_well_formed(language, tag) {
                return Some(Cow::Owned(format!(
                    "Malformed or disallowed curly-bracket tag \"{tag}\". Use {{{}: word}} or {{{}: word}} \u{2014} exact tag, single colon, no surrounding spaces, no nested tags.",
                    language.mis_keyword, language.pro_keyword
                )));
            }
            continue;
        }
        if language.is_arabic()
            && LANGUAGE_MARKER
                .captures(tag)
                .is_some_and(|marker| is_language_code(&marker[1]))
        {
            continue;
        }
        let followed_by_tag = FOLLOWING_TAG
            .captures(&masked[curly.end()..])
            .is_some_and(|following| is_well_formed(language, &following[1]));
        if !followed_by_tag {
            return fixed(&reasons().bare_curly_comment);
        }
    }
    None
}

/// Invisible characters Babel skips when looking for the space around a curly tag (`eM`).
const INVISIBLE_SPACES: [char; 5] = ['\u{A0}', '\u{202F}', '\u{200B}', '\u{2060}', '\u{FEFF}'];

/// Babel `checkCurlyTagSpacing`: a `{KEYWORD:` opening glued to the word before it or missing the
/// space after its colon.
pub(super) fn curly_tag_spacing(language: &GoldLanguage, text: &str) -> Reason {
    if !text.contains('{') {
        return None;
    }
    let glued = language
        .patterns()
        .curly_tag_opening
        .find_iter(text)
        .any(|opening| {
            // Fine: nothing but invisible spaces before the brace, or a plain space right before it.
            let before = text[..opening.start()].trim_end_matches(INVISIBLE_SPACES);
            let spaced_before =
                before.is_empty() || (before.ends_with(' ') && before.len() == opening.start());
            // Fine: the tag ends (or the text does) after invisible spaces, or a plain space follows.
            let after = text[opening.end()..].trim_start_matches(INVISIBLE_SPACES);
            let spaced_after = match after.chars().next() {
                None | Some('}') => true,
                Some(next) => next == ' ' && after.len() == text.len() - opening.end(),
            };
            !spaced_before || !spaced_after
        });
    if glued {
        fixed(&reasons().curly_tag_spacing)
    } else {
        None
    }
}

/// Babel `checkProTagNumerals`: digits inside a PRO tag.
pub(super) fn pro_tag_numerals(language: &GoldLanguage, text: &str) -> Reason {
    if !text.contains('{') {
        return None;
    }
    if language
        .patterns()
        .pro_tag
        .find_iter(text)
        .any(|tag| has_decimal_digit(tag.as_str()))
    {
        fixed(&reasons().pro_tag_numerals)
    } else {
        None
    }
}

static BLOCK_MARKER: LazyLock<Regex> = LazyLock::new(|| regex(r"^ \{([A-Z]{2})\} "));

/// Babel `checkArabicForeignTagMarker`: a foreign block that does not open with ` {XX} `.
pub(super) fn arabic_foreign_tag_marker(language: &GoldLanguage, text: &str) -> Reason {
    let block = language.patterns().foreign_block.as_ref()?;
    let unmarked = block.captures_iter(text).any(|captures| {
        let content = captures.get(1).map_or("", |content| content.as_str());
        !BLOCK_MARKER
            .captures(content)
            .is_some_and(|marker| is_language_code(&marker[1]))
    });
    if unmarked {
        fixed(&reasons().arabic_foreign_tag_missing_lang_marker)
    } else {
        None
    }
}
