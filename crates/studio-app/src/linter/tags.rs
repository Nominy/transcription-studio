const INVALID_BRACKET_TAG_REASON: &str = "Invalid bracket tag found.";
const INVALID_STYLE_TAG_REASON: &str = "Invalid style tag found.";
const UNCLOSED_BRACKET_REASON: &str = "Brackets must be properly closed.";
const UNCLOSED_STYLE_TAG_REASON: &str = "Style tags must be properly closed.";
const EMPTY_STYLE_TAG_REASON: &str = "Empty style tags <> are not allowed.";
const STYLE_TAG_SPACING_REASON: &str = "Spaces are not allowed within style tags.";
const STYLE_TAG_BOUNDARY_SPACING_REASON: &str =
    "Style tags must have spaces around them: 'word <tag> content </tag> word'";
const MISMATCHED_STYLE_TAG_REASON: &str =
    "Style tags must be closed with the same tag they opened.";
const EMPTY_BRACKET_REASON: &str = "Empty brackets [] are not allowed.";
const EMPTY_CURLY_BRACKET_REASON: &str = "Empty curly brackets {} are not allowed.";
const UNCLOSED_CURLY_BRACKET_REASON: &str = "Curly brackets must be properly closed.";
const BRACKET_SPACING_REASON: &str = "Spaces are not allowed within bracket tags.";

pub(super) const TAG_ONLY_PUNCTUATION_REASON: &str =
    "A segment that contains only a tag must not include any punctuation.";
pub(super) const UNBRACKETED_TAG_REASON: &str = "Annotation tag outside brackets. Use [неразборчиво], {ИСКАЖ: word} for mispronunciations, or {СКАЗ: word} for pronunciations.";
pub const RUSSIAN_SQUARE_BRACKET_TAGS: &[&str] =
    crate::project_lint_tag_config::RUSSIAN_TRANSCRIPTION_BRACKET_TAGS;
pub const RUSSIAN_STYLE_TAGS: &[&str] =
    crate::project_lint_tag_config::RUSSIAN_TRANSCRIPTION_STYLE_TAGS;

pub(super) fn validate_square_brackets_with_tags(
    text: &str,
    valid_tags: &[&str],
) -> Option<&'static str> {
    validate_brackets(
        text,
        '[',
        ']',
        Some(|tag: &str| valid_tags.contains(&tag)),
        EMPTY_BRACKET_REASON,
        UNCLOSED_BRACKET_REASON,
        Some(INVALID_BRACKET_TAG_REASON),
    )
}

pub(super) fn validate_curly_brackets(text: &str) -> Option<&'static str> {
    validate_brackets(
        text,
        '{',
        '}',
        None::<fn(&str) -> bool>,
        EMPTY_CURLY_BRACKET_REASON,
        UNCLOSED_CURLY_BRACKET_REASON,
        None,
    )
}

pub(super) fn validate_style_tags_with_tags(
    text: &str,
    valid_tags: &[&str],
) -> Option<&'static str> {
    let mut stack = Vec::new();
    let mut search_start = 0;

    while let Some(open_offset) = text[search_start..].find('<') {
        let open = search_start + open_offset;
        let content_start = open + '<'.len_utf8();
        let Some(close_offset) = text[content_start..].find('>') else {
            return Some(UNCLOSED_STYLE_TAG_REASON);
        };
        let close = content_start + close_offset;
        let content = &text[content_start..close];
        let trimmed = content.trim();

        if content != trimmed {
            return Some(STYLE_TAG_SPACING_REASON);
        }
        if trimmed.is_empty() {
            return Some(EMPTY_STYLE_TAG_REASON);
        }

        if let Some(tag) = trimmed.strip_prefix('/') {
            if tag.is_empty() {
                return Some(EMPTY_STYLE_TAG_REASON);
            }
            if !valid_tags.contains(&tag) {
                return Some(INVALID_STYLE_TAG_REASON);
            }
            if stack.pop() != Some(tag) {
                return Some(MISMATCHED_STYLE_TAG_REASON);
            }
        } else {
            if !valid_tags.contains(&trimmed) {
                return Some(INVALID_STYLE_TAG_REASON);
            }
            stack.push(trimmed);
        }

        let tag_end = close + '>'.len_utf8();
        let previous = text[..open].chars().next_back();
        let next = text[tag_end..].chars().next();
        if previous.is_some_and(|character| !character.is_whitespace())
            || next.is_some_and(|character| !character.is_whitespace())
        {
            return Some(STYLE_TAG_BOUNDARY_SPACING_REASON);
        }

        search_start = close + '>'.len_utf8();
    }

    (!stack.is_empty()).then_some(UNCLOSED_STYLE_TAG_REASON)
}

fn validate_brackets(
    text: &str,
    open: char,
    close: char,
    is_valid_tag: Option<impl Fn(&str) -> bool>,
    empty_reason: &'static str,
    unclosed_reason: &'static str,
    invalid_reason: Option<&'static str>,
) -> Option<&'static str> {
    let mut stack = Vec::new();
    for (index, character) in text.char_indices() {
        if character == open {
            stack.push(index);
        } else if character == close {
            let Some(open_index) = stack.pop() else {
                return Some(unclosed_reason);
            };
            let content_start = open_index + open.len_utf8();
            let content = &text[content_start..index];
            let trimmed = content.trim();
            if content != trimmed {
                return Some(BRACKET_SPACING_REASON);
            }
            if trimmed.is_empty() {
                return Some(empty_reason);
            }
            if let (Some(is_valid_tag), Some(invalid_reason)) =
                (is_valid_tag.as_ref(), invalid_reason)
                && !is_valid_tag(trimmed)
            {
                return Some(invalid_reason);
            }
        }
    }

    (!stack.is_empty()).then_some(unclosed_reason)
}

pub(super) fn has_tag_only_punctuation(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }

    let stripped = strip_tag_ranges(text);
    if stripped == text {
        return false;
    }
    let compact: String = stripped
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();

    !compact.is_empty() && compact.chars().all(is_punctuation)
}

fn strip_tag_ranges(text: &str) -> String {
    let mut without_angle_and_curly = strip_ranges(text, '<', '>');
    without_angle_and_curly = strip_ranges(&without_angle_and_curly, '{', '}');
    strip_ranges(&without_angle_and_curly, '[', ']')
}

fn strip_ranges(text: &str, open: char, close: char) -> String {
    let mut output = String::with_capacity(text.len());
    let mut skipping = false;
    for character in text.chars() {
        if !skipping && character == open {
            skipping = true;
            continue;
        }
        if skipping && character == close {
            skipping = false;
            continue;
        }
        if !skipping {
            output.push(character);
        }
    }
    output
}

fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '。' | '、' | '；' | '：' | '！' | '？' | '…' | '—' | '–' | '«' | '»' | '“' | '”'
        )
}

pub(super) fn malformed_curly_annotation_tag_with_tags(
    text: &str,
    curly_tags: &[&str],
) -> Option<String> {
    let mut search_start = 0;
    while let Some(open_offset) = text[search_start..].find('{') {
        let open = search_start + open_offset;
        let content_start = open + '{'.len_utf8();
        let Some(close_offset) = text[content_start..].find('}') else {
            break;
        };
        let close = content_start + close_offset;
        let content = &text[content_start..close];
        if content.contains(':')
            && !is_valid_curly_annotation_tag(content, curly_tags)
            && looks_like_annotation_tag(content, curly_tags)
        {
            return Some(text[open..=close].to_string());
        }
        search_start = close + '}'.len_utf8();
    }

    None
}

fn looks_like_annotation_tag(content: &str, curly_tags: &[&str]) -> bool {
    curly_tags
        .iter()
        .map(|tag| curly_tag_name(tag))
        .any(|tag| !tag.is_empty() && content.contains(tag))
}

fn is_valid_curly_annotation_tag(content: &str, curly_tags: &[&str]) -> bool {
    curly_tags.iter().any(|tag| {
        let tag = curly_tag_name(tag);
        content
            .strip_prefix(tag)
            .and_then(|payload| payload.strip_prefix(": "))
            .is_some_and(is_valid_curly_annotation_payload)
    })
}

fn curly_tag_name(tag: &str) -> &str {
    tag.trim().strip_suffix(':').unwrap_or(tag.trim())
}

fn is_valid_curly_annotation_payload(payload: &str) -> bool {
    !payload.trim().is_empty()
        && payload == payload.trim()
        && !payload.contains('{')
        && !payload.contains('}')
}

pub(super) fn has_unbracketed_annotation_tag_with_tags(text: &str, curly_tags: &[&str]) -> bool {
    curly_tags.iter().any(|tag| {
        let tag = curly_tag_name(tag);
        let needle = format!("{tag}:");
        let mut search_start = 0;
        while let Some(offset) = text[search_start..].find(&needle) {
            let tag_start = search_start + offset;
            let previous = text[..tag_start]
                .chars()
                .rev()
                .find(|character| !character.is_whitespace());
            if previous != Some('{') {
                return true;
            }
            search_start = tag_start + needle.len();
        }

        false
    })
}
