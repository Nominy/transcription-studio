//! Lint autofix: deterministic Babel text-hygiene rewrites for one transcript row.
//!
//! [`autofix_transcript_text`] is pure; the host applies provider edits through its undo history.

/// Applies every autofix rule to `text`; `previous_same_speaker_text` is the preceding
/// same-lane row (decides whether the row must start capitalized).
pub fn autofix_transcript_text(text: &str, previous_same_speaker_text: &str) -> String {
    let mut fixed = text.to_string();
    fixed = fix_leading_trailing_spaces(&fixed);
    fixed = fix_double_spaces(&fixed);
    fixed = fix_comma_spacing(&fixed);
    fixed = fix_period_spacing(&fixed);
    fixed = fix_unicode_quotes(&fixed);
    fixed = fix_curly_spacing(&fixed);
    fixed = fix_delimited_tag_spacing(&fixed, '<', '>');
    fixed = fix_delimited_tag_spacing(&fixed, '[', ']');
    fixed = fix_unicode_dashes(&fixed);
    fixed = fix_curly_tag_trailing_punctuation(&fixed);
    fixed = fix_angle_tag_trailing_punctuation(&fixed);
    fixed = fix_square_bracket_tag_trailing_punctuation(&fixed);
    fixed = fix_comma_before_dash(&fixed);
    fixed = fix_free_mid_sentence_double_dash(&fixed);
    fixed = fix_double_dash_punctuation(&fixed);
    fixed = fix_single_dash_punctuation(&fixed);
    fixed = normalize_incorrect_interjection_forms(&fixed);
    fixed = fix_sentence_boundary_capitalization(&fixed);
    fixed = fix_terminal_punctuation(&fixed);
    fixed = fix_polite_pronoun_case(&fixed);
    fix_segment_start_capitalization(&fixed, previous_same_speaker_text)
}

fn fix_leading_trailing_spaces(text: &str) -> String {
    text.trim_matches(|character| character == ' ' || character == '\t')
        .to_string()
}

fn fix_double_spaces(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == ' ' {
            let run_start = index;
            while index < chars.len() && chars[index] == ' ' {
                index += 1;
            }
            let run_len = index - run_start;
            let prev = result.chars().next_back();
            let next = chars.get(index).copied();
            if run_len >= 2
                && prev.is_some_and(|character| !character.is_whitespace())
                && next.is_some_and(|character| !character.is_whitespace())
            {
                result.push(' ');
            } else {
                for _ in 0..run_len {
                    result.push(' ');
                }
            }
            continue;
        }
        result.push(chars[index]);
        index += 1;
    }
    result
}

fn fix_comma_spacing(text: &str) -> String {
    if !text.contains(',') {
        return text.to_string();
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != ',' {
            result.push(chars[index]);
            index += 1;
            continue;
        }

        trim_inline_space_suffix(&mut result);
        result.push(',');
        index += 1;
        let space_start = index;
        while matches!(chars.get(index), Some(' ' | '\t')) {
            index += 1;
        }
        let consumed_spaces = index > space_start;
        let next = chars.get(index).copied();
        if let Some(next) = next
            && (!next.is_ascii_digit() || consumed_spaces)
        {
            result.push(' ');
        }
    }
    result
}

fn fix_period_spacing(text: &str) -> String {
    if !text.contains('.') {
        return text.to_string();
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character != '.' || !is_standalone_period(&chars, index) {
            result.push(character);
            index += 1;
            continue;
        }

        trim_inline_space_suffix(&mut result);
        result.push('.');
        index += 1;
        while matches!(chars.get(index), Some(' ' | '\t')) {
            index += 1;
        }
        if chars
            .get(index)
            .is_some_and(|next| next.is_alphanumeric() || matches!(next, '<' | '{' | '['))
        {
            result.push(' ');
        }
    }
    result
}

fn fix_unicode_quotes(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\u{00AB}' | '\u{00BB}' | '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            _ => character,
        })
        .collect()
}

fn fix_unicode_dashes(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{FE58}' | '\u{FE63}' | '\u{FF0D}' => '-',
            _ => character,
        })
        .collect()
}

fn fix_curly_spacing(text: &str) -> String {
    if !text.contains('{') || !text.contains('}') {
        return text.to_string();
    }
    let chars = text.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '{' {
            result.push(chars[index]);
            index += 1;
            continue;
        }

        if result
            .chars()
            .next_back()
            .is_some_and(|character| character.is_alphanumeric())
        {
            result.push(' ');
        }
        result.push('{');
        index += 1;
        while matches!(chars.get(index), Some(character) if character.is_whitespace()) {
            index += 1;
        }
        while index < chars.len() && chars[index] != '}' {
            result.push(chars[index]);
            index += 1;
        }
        trim_inline_space_suffix(&mut result);
        if index < chars.len() && chars[index] == '}' {
            result.push('}');
            index += 1;
            while matches!(chars.get(index), Some(' ' | '\t')) {
                index += 1;
            }
            if chars
                .get(index)
                .is_some_and(|character| character.is_alphanumeric())
            {
                result.push(' ');
            }
        }
    }
    result
}

fn fix_delimited_tag_spacing(text: &str, open: char, close: char) -> String {
    if !text.contains(open) {
        return text.to_string();
    }
    let chars = text.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;

    while index < chars.len() {
        if chars[index] != open {
            result.push(chars[index]);
            index += 1;
            continue;
        }

        let mut end = index + 1;
        while end < chars.len() && chars[end] != close && chars[end] != '\n' && chars[end] != '\r' {
            if chars[end] == open {
                break;
            }
            end += 1;
        }
        if end >= chars.len() || chars[end] != close {
            result.push(chars[index]);
            index += 1;
            continue;
        }

        let has_content_before = !result.trim_end().is_empty();
        trim_inline_space_suffix(&mut result);
        if has_content_before {
            result.push(' ');
        }

        let inner = chars[index + 1..end]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        let inner = inner.strip_prefix("/ ").unwrap_or(&inner);
        result.push(open);
        result.push_str(inner);
        result.push(close);
        index = end + 1;
        while matches!(chars.get(index), Some(' ' | '\t')) {
            index += 1;
        }
        if index < chars.len() {
            result.push(' ');
        }
    }

    result
}

fn fix_curly_tag_trailing_punctuation(text: &str) -> String {
    fix_trailing_punctuation_after_simple_tag(text, '{', '}', ".,?!:;\"", false)
}

fn fix_square_bracket_tag_trailing_punctuation(text: &str) -> String {
    fix_trailing_punctuation_after_simple_tag(text, '[', ']', ".,?!:;-", true)
}

fn fix_trailing_punctuation_after_simple_tag(
    text: &str,
    open: char,
    close: char,
    punctuation_chars: &str,
    normalize_following_space: bool,
) -> String {
    if !text.contains(close) {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for (open_index, tag_end) in simple_tag_ranges(text, open, close) {
        if open_index < cursor || !visible_text_has_alnum(&text[..open_index]) {
            continue;
        }

        let mut punctuation_start = tag_end;
        while text
            .as_bytes()
            .get(punctuation_start)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            punctuation_start += 1;
        }
        let mut punctuation_end = punctuation_start;
        while text
            .as_bytes()
            .get(punctuation_end)
            .is_some_and(|byte| punctuation_chars.as_bytes().contains(byte))
        {
            punctuation_end += 1;
        }
        if punctuation_end <= punctuation_start {
            continue;
        }

        result.push_str(&text[cursor..open_index]);
        trim_inline_space_suffix(&mut result);
        result.push_str(&text[punctuation_start..punctuation_end]);
        result.push(' ');
        result.push_str(&text[open_index..tag_end]);
        cursor = punctuation_end;
        if normalize_following_space {
            while text
                .as_bytes()
                .get(cursor)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                cursor += 1;
            }
            if cursor < text.len() {
                result.push(' ');
            }
        }
    }

    result.push_str(&text[cursor..]);
    result
}

fn fix_angle_tag_trailing_punctuation(text: &str) -> String {
    if !text.contains('<') {
        return text.to_string();
    }

    let mut parts = Vec::new();
    for (tag_start, tag_end) in angle_tag_ranges(text, true) {
        if !visible_text_has_alnum(&text[..tag_start]) {
            continue;
        }
        let mut punctuation_start = tag_end;
        while text
            .as_bytes()
            .get(punctuation_start)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            punctuation_start += 1;
        }
        let mut punctuation_end = punctuation_start;
        while text
            .as_bytes()
            .get(punctuation_end)
            .is_some_and(|byte| b".,?!:;\"".contains(byte))
        {
            punctuation_end += 1;
        }
        if punctuation_end > punctuation_start {
            parts.push(AngleTrailingPunctuationPart {
                kind: AngleTrailingPunctuationKind::Closing,
                tag_start,
                tag_end,
                punctuation_start,
                punctuation_end,
            });
        }
    }

    for (tag_start, tag_end) in angle_tag_ranges(text, false) {
        if !visible_text_has_alnum(&text[tag_end..]) {
            continue;
        }
        let mut punctuation_end = tag_start;
        while punctuation_end > 0
            && text
                .as_bytes()
                .get(punctuation_end - 1)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            punctuation_end -= 1;
        }
        if text.as_bytes().get(punctuation_end.saturating_sub(1)) != Some(&b'"') {
            continue;
        }
        let mut punctuation_start = punctuation_end - 1;
        while punctuation_start > 0 && text.as_bytes().get(punctuation_start - 1) == Some(&b'"') {
            punctuation_start -= 1;
        }
        if punctuation_start > 0
            && !text
                .as_bytes()
                .get(punctuation_start - 1)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            continue;
        }
        parts.push(AngleTrailingPunctuationPart {
            kind: AngleTrailingPunctuationKind::Opening,
            tag_start,
            tag_end,
            punctuation_start,
            punctuation_end,
        });
    }

    if parts.is_empty() {
        return text.to_string();
    }
    parts.sort_by_key(|part| part.punctuation_start);

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for part in parts {
        let part_start = match part.kind {
            AngleTrailingPunctuationKind::Opening => part.punctuation_start,
            AngleTrailingPunctuationKind::Closing => part.tag_start,
        };
        if part_start < cursor {
            continue;
        }

        match part.kind {
            AngleTrailingPunctuationKind::Opening => {
                result.push_str(&text[cursor..part.punctuation_start]);
                trim_inline_space_suffix(&mut result);
                if !result.trim_end().is_empty() {
                    result.push(' ');
                }
                result.push_str(&text[part.tag_start..part.tag_end]);
                result.push(' ');
                result.push_str(&text[part.punctuation_start..part.punctuation_end]);
                cursor = part.tag_end;
                while text
                    .as_bytes()
                    .get(cursor)
                    .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
                {
                    cursor += 1;
                }
            }
            AngleTrailingPunctuationKind::Closing => {
                result.push_str(&text[cursor..part.tag_start]);
                trim_inline_space_suffix(&mut result);
                result.push_str(&text[part.punctuation_start..part.punctuation_end]);
                result.push(' ');
                result.push_str(&text[part.tag_start..part.tag_end]);
                cursor = part.punctuation_end;
            }
        }
    }
    result.push_str(&text[cursor..]);
    result
}

#[derive(Clone, Copy)]
struct AngleTrailingPunctuationPart {
    kind: AngleTrailingPunctuationKind,
    tag_start: usize,
    tag_end: usize,
    punctuation_start: usize,
    punctuation_end: usize,
}

#[derive(Clone, Copy)]
enum AngleTrailingPunctuationKind {
    Opening,
    Closing,
}

fn fix_comma_before_dash(text: &str) -> String {
    if !text.contains(',') || !text.contains('-') {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for (comma_start, dash_start) in comma_before_dash_parts(text) {
        if comma_start < cursor {
            continue;
        }
        result.push_str(&text[cursor..comma_start]);
        trim_inline_space_suffix(&mut result);
        if !result.trim_end().is_empty() {
            result.push(' ');
        }
        cursor = dash_start;
    }
    result.push_str(&text[cursor..]);
    result
}

fn fix_free_mid_sentence_double_dash(text: &str) -> String {
    if !text.contains("--") {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in free_mid_sentence_double_dash_parts(text) {
        if start < cursor {
            continue;
        }
        result.push_str(&text[cursor..start]);
        result.push_str(" - ");
        cursor = end;
    }
    result.push_str(&text[cursor..]);
    result
}

fn fix_double_dash_punctuation(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars.get(index) == Some(&'-') && chars.get(index + 1) == Some(&'-') {
            result.push_str("--");
            index += 2;
            while chars
                .get(index)
                .is_some_and(|character| is_dash_punctuation(*character))
            {
                index += 1;
            }
            continue;
        }
        result.push(chars[index]);
        index += 1;
    }
    result
}

fn fix_single_dash_punctuation(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '-'
            && chars.get(index.wrapping_sub(1)) != Some(&'-')
            && chars.get(index + 1) != Some(&'-')
        {
            result.push('-');
            index += 1;
            while chars
                .get(index)
                .is_some_and(|character| is_dash_punctuation(*character))
            {
                index += 1;
            }
            continue;
        }
        result.push(chars[index]);
        index += 1;
    }
    result
}

fn normalize_incorrect_interjection_forms(text: &str) -> String {
    let mut fixed = text.to_string();
    for (canonical, variant) in interjection_corrections() {
        fixed = replace_boundary_variant_preserving_case(&fixed, variant, canonical);
    }
    fixed
}

fn fix_polite_pronoun_case(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, token) in word_spans(text) {
        let lower = token.to_lowercase();
        if is_polite_pronoun_token(&lower) && polite_pronoun_expects_lowercase(text, start) {
            result.push_str(&text[cursor..start]);
            result.push_str(&lower);
            cursor = end;
        }
    }
    result.push_str(&text[cursor..]);
    result
}

fn simple_tag_ranges(text: &str, open: char, close: char) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut search_start = 0;
    while let Some(relative_open) = text[search_start..].find(open) {
        let open_index = search_start + relative_open;
        let inner_start = open_index + open.len_utf8();
        let Some(relative_close) = text[inner_start..].find(close) else {
            break;
        };
        let close_index = inner_start + relative_close;
        let tag_end = close_index + close.len_utf8();
        let inner = &text[inner_start..close_index];
        if !inner.contains(open) && !inner.contains('\n') && !inner.contains('\r') {
            ranges.push((open_index, tag_end));
        }
        search_start = tag_end;
    }
    ranges
}

fn angle_tag_ranges(text: &str, closing: bool) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut search_start = 0;
    while let Some(relative_open) = text[search_start..].find('<') {
        let tag_start = search_start + relative_open;
        let inner_start = tag_start + 1;
        let Some(relative_close) = text[inner_start..].find('>') else {
            break;
        };
        let tag_end = inner_start + relative_close + 1;
        let inner = &text[inner_start..tag_end - 1];
        let is_closing = inner.starts_with('/');
        let body = if is_closing { &inner[1..] } else { inner };
        if is_closing == closing
            && !body.contains('<')
            && !inner.contains('\n')
            && !inner.contains('\r')
        {
            ranges.push((tag_start, tag_end));
        }
        search_start = tag_end;
    }
    ranges
}

fn comma_before_dash_parts(text: &str) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let mut search_start = 0;
    while let Some(relative_comma) = text[search_start..].find(',') {
        let comma_start = search_start + relative_comma;
        if !visible_text_has_alnum(&text[..comma_start])
            || is_byte_inside_generic_tag(text, comma_start)
        {
            search_start = comma_start + 1;
            continue;
        }

        let mut dash_start = comma_start + 1;
        while text
            .as_bytes()
            .get(dash_start)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            dash_start += 1;
        }
        if text.as_bytes().get(dash_start) != Some(&b'-') {
            search_start = comma_start + 1;
            continue;
        }
        let dash_end = if text.as_bytes().get(dash_start + 1) == Some(&b'-') {
            dash_start + 2
        } else {
            dash_start + 1
        };
        if text.as_bytes().get(dash_end) == Some(&b'-')
            || !text
                .as_bytes()
                .get(dash_end)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            || !text[dash_end..]
                .chars()
                .any(|character| !character.is_whitespace())
        {
            search_start = comma_start + 1;
            continue;
        }

        parts.push((comma_start, dash_start));
        search_start = comma_start + 1;
    }
    parts
}

fn free_mid_sentence_double_dash_parts(text: &str) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let mut search_start = 0;
    while let Some(relative_dash) = text[search_start..].find("--") {
        let dash_start = search_start + relative_dash;
        let dash_end = dash_start + 2;
        if text.as_bytes().get(dash_start.wrapping_sub(1)) == Some(&b'-')
            || text.as_bytes().get(dash_end) == Some(&b'-')
            || is_byte_inside_generic_tag(text, dash_start)
        {
            search_start = dash_end;
            continue;
        }
        if !text
            .as_bytes()
            .get(dash_start.wrapping_sub(1))
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            || !text
                .as_bytes()
                .get(dash_end)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            search_start = dash_end;
            continue;
        }

        let mut start = dash_start - 1;
        while start > 0
            && text
                .as_bytes()
                .get(start - 1)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            start -= 1;
        }
        let mut end = dash_end + 1;
        while end < text.len()
            && text
                .as_bytes()
                .get(end)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            end += 1;
        }
        if start > 0 && end < text.len() {
            parts.push((start, end));
        }
        search_start = dash_end;
    }
    parts
}

fn visible_text_has_alnum(text: &str) -> bool {
    let mut skip_until = None;
    for (index, character) in text.char_indices() {
        if let Some(close) = skip_until {
            if character == close {
                skip_until = None;
            }
            continue;
        }
        match character {
            '{' => skip_until = Some('}'),
            '[' => skip_until = Some(']'),
            '<' => skip_until = Some('>'),
            _ if character.is_alphanumeric() => return true,
            _ => {}
        }
        if index >= text.len() {
            break;
        }
    }
    false
}

fn is_byte_inside_generic_tag(text: &str, byte_index: usize) -> bool {
    let mut skip_until = None;
    let mut tag_start = 0;
    for (index, character) in text.char_indices() {
        if index > byte_index {
            break;
        }
        if let Some(close) = skip_until {
            if character == close {
                if tag_start <= byte_index && byte_index < index + character.len_utf8() {
                    return true;
                }
                skip_until = None;
            } else if tag_start <= byte_index && byte_index < index + character.len_utf8() {
                return true;
            }
            continue;
        }
        match character {
            '{' => {
                skip_until = Some('}');
                tag_start = index;
            }
            '[' => {
                skip_until = Some(']');
                tag_start = index;
            }
            '<' => {
                skip_until = Some('>');
                tag_start = index;
            }
            _ => {}
        }
    }
    skip_until.is_some() && tag_start <= byte_index
}

fn interjection_corrections() -> Vec<(&'static str, &'static str)> {
    let specs: &[(&str, &[&str])] = &[
        ("а", &["аа", "а-а", "а-а-а"]),
        ("ага", &["ага-а", "агаа"]),
        ("Ам", &["А-м", "а-ам"]),
        ("ах", &["ахх", "а-а-ах"]),
        ("блин", &["бли-ин"]),
        ("Вау", &["уау"]),
        ("вот", &["вооот"]),
        ("ей-богу", &["ейбогу", "ей богу"]),
        ("м-да", &["мда", "мдя"]),
        ("мгм", &["мм-гм", "мхм"]),
        ("м", &["мм", "Ммм", "м-м-м"]),
        ("Н-да", &["Нда"]),
        ("ну", &["нууу", "ну-у"]),
        ("Ну да", &["Ну, да"]),
        ("о да", &["о, да"]),
        ("о нет", &["о, нет"]),
        ("ой", &["оой", "ойй"]),
        ("окей", &["о'кей"]),
        ("ох", &["охх"]),
        ("у", &["у-у"]),
        ("угу", &["у-г-у", "угуу"]),
        ("ух", &["ухх"]),
        ("фу", &["фу-у"]),
        ("ха-ха", &["хахаха"]),
        ("ха", &["ха-а", "хаха"]),
        ("хм", &["хмм", "гм"]),
        ("чёрт", &["чорт"]),
        ("э", &["э-э", "эээ", "ээ", "э…э"]),
        ("эх", &["э-эх", "эхх"]),
        ("о", &["оо", "о-о"]),
    ];
    let mut corrections = specs
        .iter()
        .flat_map(|(canonical, variants)| {
            variants.iter().map(move |variant| (*canonical, *variant))
        })
        .collect::<Vec<_>>();
    corrections.sort_by_key(|(_, variant)| std::cmp::Reverse(variant.chars().count()));
    corrections
}

fn replace_boundary_variant_preserving_case(text: &str, variant: &str, canonical: &str) -> String {
    let variant_lower = variant.to_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let Some((relative_start, matched_end)) =
            find_boundary_variant(&text[cursor..], &variant_lower)
        else {
            break;
        };
        let start = cursor + relative_start;
        let end = cursor + matched_end;
        result.push_str(&text[cursor..start]);
        let matched = &text[start..end];
        result.push_str(&apply_letter_case_shape(
            canonical,
            letter_case_shape(matched),
        ));
        cursor = end;
    }
    result.push_str(&text[cursor..]);
    result
}

fn find_boundary_variant(text: &str, variant_lower: &str) -> Option<(usize, usize)> {
    let variant_char_count = variant_lower.chars().count();
    for (start, _) in text.char_indices() {
        if previous_char(text, start).is_some_and(is_word_character) {
            continue;
        }
        let mut end = start;
        let mut lowered = String::new();
        while end < text.len() && lowered.chars().count() < variant_char_count {
            let character = text.get(end..)?.chars().next()?;
            lowered.extend(character.to_lowercase());
            end += character.len_utf8();
        }
        if lowered == variant_lower
            && !text
                .get(end..)
                .and_then(|rest| rest.chars().next())
                .is_some_and(is_word_character)
        {
            return Some((start, end));
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LetterCaseShape {
    Upper,
    Lower,
    Title,
    Mixed,
}

fn letter_case_shape(text: &str) -> LetterCaseShape {
    let letters = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect::<Vec<_>>();
    if letters.is_empty() {
        return LetterCaseShape::Mixed;
    }
    if letters
        .iter()
        .all(|character| is_uppercase_letter(*character))
    {
        return LetterCaseShape::Upper;
    }
    if letters
        .iter()
        .all(|character| is_lowercase_letter(*character))
    {
        return LetterCaseShape::Lower;
    }
    if is_uppercase_letter(letters[0])
        && letters[1..]
            .iter()
            .all(|character| is_lowercase_letter(*character))
    {
        return LetterCaseShape::Title;
    }
    LetterCaseShape::Mixed
}

fn apply_letter_case_shape(text: &str, shape: LetterCaseShape) -> String {
    match shape {
        LetterCaseShape::Upper => text.to_uppercase(),
        LetterCaseShape::Lower => text.to_lowercase(),
        LetterCaseShape::Title => {
            let mut applied = false;
            let mut result = String::with_capacity(text.len());
            for character in text.chars() {
                if !character.is_alphabetic() {
                    result.push(character);
                } else if !applied {
                    result.extend(character.to_uppercase());
                    applied = true;
                } else {
                    result.extend(character.to_lowercase());
                }
            }
            result
        }
        LetterCaseShape::Mixed => text.to_string(),
    }
}

fn word_spans(text: &str) -> Vec<(usize, usize, &str)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if is_word_character(character) {
            start.get_or_insert(index);
            continue;
        }
        if let Some(start_index) = start.take() {
            spans.push((start_index, index, &text[start_index..index]));
        }
    }
    if let Some(start_index) = start {
        spans.push((start_index, text.len(), &text[start_index..]));
    }
    spans
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric()
        || matches!(
            character,
            '\u{0300}'..='\u{036F}'
                | '\u{0483}'..='\u{0489}'
                | '\u{0591}'..='\u{05BD}'
                | '\u{05BF}'
                | '\u{05C1}'..='\u{05C2}'
                | '\u{05C4}'..='\u{05C5}'
                | '\u{05C7}'
                | '\u{0610}'..='\u{061A}'
                | '\u{064B}'..='\u{065F}'
                | '\u{0670}'
                | '\u{06D6}'..='\u{06DC}'
                | '\u{06DF}'..='\u{06E4}'
                | '\u{06E7}'..='\u{06E8}'
                | '\u{06EA}'..='\u{06ED}'
                | '\u{1AB0}'..='\u{1AFF}'
                | '\u{1DC0}'..='\u{1DFF}'
                | '\u{20D0}'..='\u{20FF}'
                | '\u{FE20}'..='\u{FE2F}'
        )
}

fn is_polite_pronoun_token(lower: &str) -> bool {
    matches!(
        lower,
        "вы" | "вас"
            | "вам"
            | "вами"
            | "ваш"
            | "ваша"
            | "ваше"
            | "ваши"
            | "вашего"
            | "вашему"
            | "вашим"
            | "вашем"
            | "вашу"
            | "вашей"
            | "вашею"
            | "ваших"
            | "вашими"
    )
}

fn polite_pronoun_expects_lowercase(text: &str, token_index: usize) -> bool {
    if is_byte_inside_generic_tag(text, token_index) {
        return false;
    }
    let first_letter = text
        .char_indices()
        .find(|(_, character)| character.is_alphabetic())
        .map(|(index, _)| index);
    if first_letter == Some(token_index) {
        return false;
    }
    let Some(pointer) = previous_visible_non_space_byte(text, token_index) else {
        return false;
    };
    let current = text[pointer..].chars().next().unwrap_or_default();
    let pointer_end = pointer + current.len_utf8();
    if text[..pointer_end].ends_with("...") {
        return false;
    }
    if matches!(current, '.' | '?' | '!' | ':' | '-') {
        return false;
    }
    true
}

fn previous_visible_non_space_byte(text: &str, before: usize) -> Option<usize> {
    let mut previous = None;
    for (index, character) in text[..before].char_indices() {
        if !character.is_whitespace() {
            previous = Some(index);
        }
    }
    previous
}

fn previous_char(text: &str, index: usize) -> Option<char> {
    text.get(..index)?.chars().next_back()
}

fn fix_sentence_boundary_capitalization(text: &str) -> String {
    let mut chars = text.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        let boundary = chars[index];
        if !matches!(boundary, '.' | '?' | '!') || !is_sentence_boundary(&chars, index) {
            index += 1;
            continue;
        }

        let mut letter_index = index + 1;
        while chars
            .get(letter_index)
            .is_some_and(|character| is_boundary_skip_character(*character))
        {
            letter_index += 1;
        }
        if chars
            .get(letter_index)
            .is_some_and(|character| is_lowercase_letter(*character))
        {
            chars[letter_index] = chars[letter_index]
                .to_uppercase()
                .next()
                .unwrap_or(chars[letter_index]);
        }
        index += 1;
    }
    chars.into_iter().collect()
}

fn fix_terminal_punctuation(text: &str) -> String {
    let trimmed = text.trim_end();
    if trimmed.is_empty()
        || trimmed.ends_with("...")
        || trimmed.ends_with("--")
        || trimmed.chars().next_back().is_some_and(|character| {
            matches!(character, '.' | ',' | '?' | '!' | ':' | ';' | '"' | '-')
        })
    {
        return text.to_string();
    }

    let trailing = &text[trimmed.len()..];
    format!("{trimmed}.{trailing}")
}

fn fix_segment_start_capitalization(text: &str, previous_same_speaker_text: &str) -> String {
    let mut chars = text.chars().collect::<Vec<_>>();
    let Some(initial_letter_index) = first_letter_index(&chars, 0) else {
        return text.to_string();
    };

    let content_start = first_content_index(&chars);
    if starts_with_chars(&chars, content_start, "...") {
        let Some(letter_index) = first_letter_index(&chars, content_start + 3) else {
            return text.to_string();
        };
        if is_uppercase_letter(chars[letter_index]) {
            chars[letter_index] = chars[letter_index]
                .to_lowercase()
                .next()
                .unwrap_or(chars[letter_index]);
        }
        return chars.into_iter().collect();
    }

    if is_lowercase_letter(chars[initial_letter_index])
        && !ends_with_lowercase_continuation_marker(previous_same_speaker_text)
    {
        chars[initial_letter_index] = chars[initial_letter_index]
            .to_uppercase()
            .next()
            .unwrap_or(chars[initial_letter_index]);
    }
    chars.into_iter().collect()
}

fn is_standalone_period(chars: &[char], index: usize) -> bool {
    if chars.get(index) != Some(&'.') {
        return false;
    }
    let prev = index
        .checked_sub(1)
        .and_then(|prev| chars.get(prev))
        .copied();
    let next = chars.get(index + 1).copied();
    if prev == Some('.') || next == Some('.') {
        return false;
    }
    !(prev.is_some_and(|character| character.is_ascii_digit())
        && next.is_some_and(|character| character.is_ascii_digit()))
}

fn is_sentence_boundary(chars: &[char], index: usize) -> bool {
    if chars[index] == '.'
        && (chars.get(index.wrapping_sub(1)) == Some(&'.') || chars.get(index + 1) == Some(&'.'))
    {
        return false;
    }
    true
}

fn is_dash_punctuation(character: char) -> bool {
    matches!(character, '.' | ',' | '?' | '!' | ':' | ';')
}

fn is_boundary_skip_character(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '"' | '\'' | ')' | ']' | '}' | '\u{00BB}' | '\u{201D}' | '\u{2019}'
        )
}

fn is_lowercase_letter(character: char) -> bool {
    character.is_alphabetic() && character.to_lowercase().to_string() == character.to_string()
}

fn is_uppercase_letter(character: char) -> bool {
    character.is_alphabetic() && character.to_uppercase().to_string() == character.to_string()
}

fn first_content_index(chars: &[char]) -> usize {
    chars
        .iter()
        .position(|character| !is_boundary_skip_character(*character))
        .unwrap_or(0)
}

fn first_letter_index(chars: &[char], start: usize) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, character)| character.is_alphabetic())
        .map(|(index, _)| index)
}

fn starts_with_chars(chars: &[char], start: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(offset, character)| chars.get(start + offset) == Some(&character))
}

fn ends_with_lowercase_continuation_marker(text: &str) -> bool {
    let trimmed = text
        .trim_end_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '"' | '\'' | ')' | ']' | '}' | '\u{00BB}' | '\u{201D}' | '\u{2019}'
                )
        })
        .trim_end();
    trimmed.ends_with("...") || trimmed.ends_with("--")
}

fn trim_inline_space_suffix(text: &mut String) {
    while text.ends_with(' ') || text.ends_with('\t') {
        text.pop();
    }
}
