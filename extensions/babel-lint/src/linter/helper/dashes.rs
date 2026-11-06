//! Dash and segment-end punctuation rules.

use super::text::{
    Text, is_blank, is_double_quote, is_inside_open_quote, is_range_inside_generic_tag,
    skip_blanks, strip_trailing_tag_tokens, trim_end,
};
use super::{COMPACTED_MATCH_LIMIT, Row};

fn is_dash_punctuation(character: Option<char>) -> bool {
    matches!(character, Some('.' | ',' | '?' | '!' | ':' | ';'))
}

/// `free-mid-sentence-double-dash` (`getFreeMidSentenceDoubleDashParts`): a `--` with blanks
/// on both sides, outside tags and open quotes, with text before and after it.
pub(super) fn free_mid_sentence_double_dash(row: &Row<'_>) -> bool {
    let text = &row.text;
    let mut found = text.index_of("--", 0);
    while let Some(index) = found {
        found = text.index_of("--", index + 2);
        if index == 0
            || !is_blank(text.char_at(index - 1))
            || !is_blank(text.char_at(index + 2))
            || is_range_inside_generic_tag(text, index, index + 2)
            || is_inside_open_quote(text, index)
        {
            continue;
        }
        let mut start = index - 1;
        while start > 0 && is_blank(text.char_at(start - 1)) {
            start -= 1;
        }
        if start != 0 && skip_blanks(text, index + 3) < text.len() {
            return true;
        }
    }
    false
}

/// Non-overlapping matches of `/(?<!-)-[.,?!:;]+/gu` (`after_double_dash == false`) or
/// `/--[.,?!:;]+/gu`, as `[start, end)`; at most [`COMPACTED_MATCH_LIMIT`].
fn dash_punctuation_matches(text: &Text<'_>, after_double_dash: bool) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let mut index = 0;
    while index < text.len() && matches.len() < COMPACTED_MATCH_LIMIT {
        let punctuation_start = if after_double_dash {
            text.starts_with_at(index, "--").then_some(index + 2)
        } else {
            (text.is(index, '-') && (index == 0 || !text.is(index - 1, '-'))).then_some(index + 1)
        };
        match punctuation_start {
            Some(mut end) if is_dash_punctuation(text.char_at(end)) => {
                while is_dash_punctuation(text.char_at(end)) {
                    end += 1;
                }
                matches.push((index, end));
                index = end;
            }
            _ => index += 1,
        }
    }
    matches
}

/// `double-dash-punctuation`: punctuation glued after `--`, except a `--...` fragment.
pub(super) fn double_dash_punctuation(row: &Row<'_>) -> bool {
    let text = &row.text;
    dash_punctuation_matches(text, true)
        .into_iter()
        .any(|(start, end)| {
            !is_range_inside_generic_tag(text, start, end) && text.slice(start, end) != "--..."
        })
}

/// `single-dash-punctuation`.
pub(super) fn single_dash_punctuation(row: &Row<'_>) -> bool {
    let text = &row.text;
    dash_punctuation_matches(text, false)
        .into_iter()
        .any(|(start, end)| !is_range_inside_generic_tag(text, start, end))
}

const INDECIPHERABLE_TAG: &str = "[неразборчиво]";

/// `terminal-punctuation` (`getMissingTerminalPunctuationIndex`): the text before trailing
/// tags must end with `.`, `,`, `?`, `!`, `:`, `;`, `"` or `-`, unless the segment ends with
/// `[неразборчиво]`.
pub(super) fn terminal_punctuation(row: &Row<'_>) -> bool {
    let text = &row.text;
    let trimmed = trim_end(text, text.len());
    let tag_len = INDECIPHERABLE_TAG.encode_utf16().count();
    if trimmed >= tag_len && text.starts_with_at(trimmed - tag_len, INDECIPHERABLE_TAG) {
        return false;
    }
    let end = strip_trailing_tag_tokens(text);
    end > 0 && {
        let last = text.char_at(end - 1);
        !(is_double_quote(last) || matches!(last, Some('.' | ',' | '?' | '!' | ':' | ';' | '-')))
    }
}
