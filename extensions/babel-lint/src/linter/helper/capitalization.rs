//! Letter-case rules: sentence starts, Russian polite pronouns and segment starts.

use super::text::{
    Text, enclosing_generic_tag, first_letter_index, is_inside_open_quote, is_js_space,
    is_letter_number_mark, is_lowercase_letter, skip_leading_capitalization_tokens,
    strip_trailing_tag_tokens, trim_end,
};
use super::{HelperEntry, Row};

/// page.ts `skipSentenceBoundaryTokens`: skips whitespace, quotes, parentheses and tags.
fn skip_sentence_boundary_tokens(text: &Text<'_>, start: usize) -> usize {
    let mut index = start;
    while let Some(character) = text.char_at(index) {
        if is_js_space(character) || "\"'“”«»„‟‘’()".contains(character) {
            index += 1;
            continue;
        }
        let next = skip_leading_capitalization_tokens(text, index);
        if next == index {
            break;
        }
        index = next;
    }
    index
}

/// page.ts `getRussianDirectSpeechAuthorContinuationLetterIndex`: in `"…?" - сказал он` the
/// author's words after `?`/`!`, a closing quote and a dash continue the sentence.
fn direct_speech_author_letter_index(text: &Text<'_>, boundary: usize) -> Option<usize> {
    if !matches!(text.char_at(boundary), Some('?' | '!')) {
        return None;
    }
    let mut index = boundary + 1;
    while matches!(text.char_at(index), Some('?' | '!')) {
        index += 1;
    }
    let inside_open_quote = is_inside_open_quote(text, boundary);
    while let Some(character) = text.char_at(index) {
        if is_js_space(character) {
            index += 1;
            continue;
        }
        if matches!(character, '"' | '\u{00BB}' | '\u{201D}') {
            index += 1;
            break;
        }
        if inside_open_quote && character == '-' {
            break;
        }
        return None;
    }
    while text.char_at(index).is_some_and(is_js_space) {
        index += 1;
    }
    if !text.is(index, '-') {
        return None;
    }
    first_letter_index(text, skip_leading_capitalization_tokens(text, index + 1))
}

/// `sentence-boundary-capitalization` (`findSentenceBoundaryLowercaseIndices`).
pub(super) fn sentence_boundary_capitalization(row: &Row<'_>) -> bool {
    let text = &row.text;
    (0..text.len()).any(|index| {
        let character = text.char_at(index);
        if !matches!(character, Some('.' | '?' | '!')) {
            return false;
        }
        if character == Some('.')
            && ((index > 0 && text.is(index - 1, '.')) || text.is(index + 1, '.'))
        {
            return false;
        }
        let author = direct_speech_author_letter_index(text, index);
        if author.is_some_and(|author| is_lowercase_letter(text.char_at(author))) {
            return false;
        }
        first_letter_index(text, skip_sentence_boundary_tokens(text, index + 1))
            .is_some_and(|letter| is_lowercase_letter(text.char_at(letter)))
    })
}

/// page.ts `POLITE_PRONOUN_PATTERN` forms.
const POLITE_PRONOUNS: [&str; 17] = [
    "вы",
    "вас",
    "вам",
    "вами",
    "ваш",
    "ваша",
    "ваше",
    "ваши",
    "вашего",
    "вашему",
    "вашим",
    "вашем",
    "вашу",
    "вашей",
    "вашею",
    "ваших",
    "вашими",
];

/// `[A-Za-zА-Яа-яЁё0-9_-]`.
fn is_inline_tag_name_character(character: Option<char>) -> bool {
    matches!(
        character,
        Some('A'..='Z' | 'a'..='z' | 'А'..='я' | 'Ё' | 'ё' | '0'..='9' | '_' | '-')
    )
}

/// page.ts `isInsidePairedInlineTagContent`: `index` sits between `<name>` and `</name>`
/// with no other angle bracket in between.
fn is_inside_paired_inline_tag_content(text: &Text<'_>, index: usize) -> bool {
    let Some(open_end) = (0..index)
        .rev()
        .find(|&pointer| matches!(text.char_at(pointer), Some('<' | '>')))
    else {
        return false;
    };
    if !text.is(open_end, '>') {
        return false;
    }
    let mut name_start = open_end;
    while name_start > 0 && is_inline_tag_name_character(text.char_at(name_start - 1)) {
        name_start -= 1;
    }
    if name_start == open_end || name_start == 0 || !text.is(name_start - 1, '<') {
        return false;
    }
    let Some(close) =
        (index..text.len()).find(|&pointer| matches!(text.char_at(pointer), Some('<' | '>')))
    else {
        return false;
    };
    let name = text.slice(name_start, open_end);
    text.starts_with_at(close, "</")
        && text.starts_with_at(close + 2, &name)
        && text.is(close + 2 + (open_end - name_start), '>')
}

/// page.ts `skipBackwardIgnorableTokens`: the last index at or before `pointer` that is not
/// whitespace, inside a tag, a closing quote/bracket or a closing style tag.
fn skip_backward_ignorable_tokens(text: &Text<'_>, pointer: Option<usize>) -> Option<usize> {
    let mut current = pointer;
    while let Some(pointer) = current {
        let mut index = trim_end(text, pointer + 1).checked_sub(1)?;
        if let Some((tag_start, _)) = enclosing_generic_tag(text, index) {
            current = tag_start.checked_sub(1);
            continue;
        }
        while matches!(
            text.char_at(index),
            Some('"' | '\'' | ')' | ']' | '}' | '\u{00BB}' | '\u{201D}' | '\u{2019}')
        ) {
            index = trim_end(text, index).checked_sub(1)?;
        }
        if let Some(tag_start) = closing_style_tag_start(text, index) {
            current = tag_start.checked_sub(1);
            continue;
        }
        return Some(index);
    }
    None
}

/// The start of a `/<\/[^>]+>$/u` match in `text.slice(0, end + 1)`.
fn closing_style_tag_start(text: &Text<'_>, end: usize) -> Option<usize> {
    if !text.is(end, '>') {
        return None;
    }
    let after_previous_close = (0..end)
        .rev()
        .find(|&pointer| text.is(pointer, '>'))
        .map_or(0, |close| close + 1);
    (after_previous_close..end.saturating_sub(2)).find(|&start| text.starts_with_at(start, "</"))
}

/// page.ts `getPolitePronounTargetToken`: whether the pronoun at `index` must be lowercase.
fn polite_pronoun_must_be_lowercase(text: &Text<'_>, index: usize) -> bool {
    if enclosing_generic_tag(text, index).is_some()
        || is_inside_paired_inline_tag_content(text, index)
        || first_letter_index(text, skip_leading_capitalization_tokens(text, 0)) == Some(index)
    {
        return false;
    }
    skip_backward_ignorable_tokens(text, index.checked_sub(1))
        .is_some_and(|pointer| !matches!(text.char_at(pointer), Some('-' | '.' | '?' | '!' | ':')))
}

/// `polite-pronoun-case` (`getPolitePronounCaseMatches`): a capitalized вы/ваш form
/// mid-sentence, outside tags.
pub(super) fn polite_pronoun_case(row: &Row<'_>) -> bool {
    let text = &row.text;
    let mut index = 0;
    while let Some(character) = text.code_point_at(index) {
        if !is_letter_number_mark(character) {
            index += character.len_utf16();
            continue;
        }
        let start = index;
        while let Some(character) = text
            .code_point_at(index)
            .filter(|&character| is_letter_number_mark(character))
        {
            index += character.len_utf16();
        }
        let token = text.slice(start, index);
        let lower = token.to_lowercase();
        if token != lower
            && POLITE_PRONOUNS.contains(&lower.as_str())
            && polite_pronoun_must_be_lowercase(text, start)
        {
            return true;
        }
    }
    false
}

/// page.ts `stripTrailingContinuationClosers` + `endsWithLowercaseContinuationMarker`: the
/// text ends with `...` or `--` before trailing tags, quotes and closing brackets.
fn ends_with_lowercase_continuation_marker(source: &str) -> bool {
    let text = Text::new(source);
    let mut end = strip_trailing_tag_tokens(&text);
    while end > 0
        && text.char_at(end - 1).is_some_and(|character| {
            is_js_space(character) || "\"')]}\u{00BB}\u{201D}\u{2019}".contains(character)
        })
    {
        end = trim_end(&text, end - 1);
    }
    let prefix = text.slice(0, end);
    prefix.ends_with("...") || prefix.ends_with("--")
}

/// page.ts `previousSameSpeakerAllowsLowercase`: the speaker's previous segment ends with a
/// continuation marker.
fn previous_same_speaker_allows_lowercase(entries: &[HelperEntry<'_>], index: usize) -> bool {
    let current = &entries[index];
    !current.speaker_key.is_empty()
        && entries[..index]
            .iter()
            .rev()
            .find(|candidate| candidate.speaker_key == current.speaker_key)
            .is_some_and(|candidate| ends_with_lowercase_continuation_marker(candidate.text))
}

/// `segment-start-capitalization`: a lowercase first letter, unless the segment starts with
/// an ellipsis or continues the same speaker's `...`/`--`.
pub(super) fn segment_start_capitalization(row: &Row<'_>) -> bool {
    let text = &row.text;
    let content_start = skip_leading_capitalization_tokens(text, 0);
    !text.starts_with_at(content_start, "...")
        && first_letter_index(text, content_start)
            .is_some_and(|letter| is_lowercase_letter(text.char_at(letter)))
        && !previous_same_speaker_allows_lowercase(row.entries, row.index)
}
