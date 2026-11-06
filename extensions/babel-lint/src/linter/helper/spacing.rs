//! Whitespace, comma, period, tag-spacing, quote and dash-glyph rules.

use super::Row;
use super::text::{
    Text, delimited_tags, is_blank, is_double_quote, is_js_space, is_letter, is_number,
    is_range_inside_generic_tag, is_unicode_double_quote, skip_blanks,
};

/// `leading-trailing-spaces`: `^[ \t]+` or `[ \t]+$`.
pub(super) fn leading_trailing_spaces(row: &Row<'_>) -> bool {
    let text = &row.text;
    !text.is_empty() && (is_blank(text.char_at(0)) || is_blank(text.char_at(text.len() - 1)))
}

/// `double-spaces`: `/(\S)( {2,})(?=\S)/gu`.
pub(super) fn double_spaces(row: &Row<'_>) -> bool {
    let text = &row.text;
    let mut index = 1;
    while index < text.len() {
        if !text.is(index, ' ') {
            index += 1;
            continue;
        }
        let start = index;
        while text.is(index, ' ') {
            index += 1;
        }
        let before = text
            .char_at(start - 1)
            .is_some_and(|character| !is_js_space(character));
        let after = text
            .char_at(index)
            .is_some_and(|character| !is_js_space(character));
        if index - start >= 2 && before && after {
            return true;
        }
    }
    false
}

/// `comma-spacing`: `/\s+,|,(?![\d ]|$)|, {2,}/gu`.
pub(super) fn comma_spacing(row: &Row<'_>) -> bool {
    let text = &row.text;
    (0..text.len())
        .filter(|&index| text.is(index, ','))
        .any(|index| {
            let space_before = index > 0 && text.char_at(index - 1).is_some_and(is_js_space);
            let bad_follower = text
                .char_at(index + 1)
                .is_some_and(|next| !next.is_ascii_digit() && next != ' ');
            space_before || bad_follower || (text.is(index + 1, ' ') && text.is(index + 2, ' '))
        })
}

/// `isStandalonePeriodAt`.
fn is_standalone_period(text: &Text<'_>, index: usize) -> bool {
    if !text.is(index, '.') {
        return false;
    }
    let previous = index.checked_sub(1).and_then(|before| text.char_at(before));
    let next = text.char_at(index + 1);
    if previous == Some('.') || next == Some('.') {
        return false;
    }
    if previous.is_some_and(|character| character.is_ascii_digit())
        && next.is_some_and(|character| character.is_ascii_digit())
    {
        return false;
    }
    !is_range_inside_generic_tag(text, index, index + 1)
}

/// `period-spacing` (`getPeriodSpacingParts`): no space before a standalone period, exactly
/// one before a following word, number or tag.
pub(super) fn period_spacing(row: &Row<'_>) -> bool {
    let text = &row.text;
    (0..text.len()).any(|index| {
        if !is_standalone_period(text, index) {
            return false;
        }
        let space_before = index > 0 && is_blank(text.char_at(index - 1));
        let next_index = skip_blanks(text, index + 1);
        let should_have_space_after = text.char_at(next_index).is_some_and(|character| {
            is_letter(character) || is_number(character) || matches!(character, '<' | '{' | '[')
        });
        let exactly_one_space_after = next_index == index + 2 && text.is(index + 1, ' ');
        space_before || (should_have_space_after && !exactly_one_space_after)
    })
}

/// `getDelimitedTagSpacingMatches`: a tag padded inside (or after its closing slash), or
/// touching a non-space neighbour.
fn delimited_tag_spacing(text: &Text<'_>, open: char, close: char) -> bool {
    delimited_tags(text, open, close)
        .into_iter()
        .any(|(start, end)| {
            let inner_first = text.char_at(start + 1).filter(|_| start + 1 < end - 1);
            let inner_last = text.char_at(end - 2).filter(|_| start + 1 < end - 1);
            let padded =
                inner_first.is_some_and(is_js_space) || inner_last.is_some_and(is_js_space);
            let slash_padded =
                text.is(start + 1, '/') && text.char_at(start + 2).is_some_and(is_js_space);
            let touches_before =
                start > 0 && text.char_at(start - 1).is_some_and(|c| !is_js_space(c));
            let touches_after = text
                .char_at(end)
                .is_some_and(|character| !is_js_space(character));
            padded || slash_padded || touches_before || touches_after
        })
}

/// `angle-tag-spacing`.
pub(super) fn angle_tag_spacing(row: &Row<'_>) -> bool {
    delimited_tag_spacing(&row.text, '<', '>')
}

/// `square-bracket-tag-spacing`.
pub(super) fn square_bracket_tag_spacing(row: &Row<'_>) -> bool {
    delimited_tag_spacing(&row.text, '[', ']')
}

/// `curly-tag-spacing`: `/(?<=\S)\{[\p{L}\p{N}_-]+:[^{}\r\n]*\}/gu`.
pub(super) fn curly_tag_spacing(row: &Row<'_>) -> bool {
    let text = &row.text;
    (1..text.len()).any(|start| {
        if !text.is(start, '{') || text.char_at(start - 1).is_none_or(is_js_space) {
            return false;
        }
        let mut index = start + 1;
        while let Some(character) = text.code_point_at(index) {
            if !(is_letter(character) || is_number(character) || matches!(character, '_' | '-')) {
                break;
            }
            index += character.len_utf16();
        }
        if index == start + 1 || !text.is(index, ':') {
            return false;
        }
        (index + 1..text.len())
            .find(|&stop| matches!(text.char_at(stop), Some('{' | '}' | '\r' | '\n')))
            .is_some_and(|stop| text.is(stop, '}'))
    })
}

/// `quote-balance`: an odd number of double quotes, Unicode variants included.
pub(super) fn quote_balance(row: &Row<'_>) -> bool {
    let text = &row.text;
    (0..text.len())
        .filter(|&index| is_double_quote(text.char_at(index)))
        .count()
        % 2
        == 1
}

/// `unicode-quotes`.
pub(super) fn unicode_quotes(row: &Row<'_>) -> bool {
    row.text.source.chars().any(is_unicode_double_quote)
}

/// `unicode-dashes`: `/[\u2010-\u2015\u2212\u2E3A\u2E3B\uFE58\uFE63\uFF0D]/gu`.
pub(super) fn unicode_dashes(row: &Row<'_>) -> bool {
    row.text.source.chars().any(|character| {
        matches!(
            character,
            '\u{2010}'
                ..='\u{2015}'
                    | '\u{2212}'
                    | '\u{2E3A}'
                    | '\u{2E3B}'
                    | '\u{FE58}'
                    | '\u{FE63}'
                    | '\u{FF0D}'
        )
    })
}
