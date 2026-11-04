//! Helper text transformations: SKAZ number conversion and moving text across same-speaker rows
//! (with an atomic two-row undo/redo history).

use studio_core::{Project, Segment};

use super::draft::{Drafts, TextSelection, byte_offset, push_bounded_history};

const MAX_SUPPORTED_DIGITS: usize = 12;
/// Cross-row moves duplicate snapshots from two segments, so each direction keeps at most this
/// many moves (and [`CROSS_ROW_HISTORY_MAX_RETAINED_BYTES`]); one newest oversized move is kept.
pub const CROSS_ROW_HISTORY_MAX_ENTRIES: usize = 32;
pub const CROSS_ROW_HISTORY_MAX_RETAINED_BYTES: usize = 2 * 1024 * 1024;

/// Direction of a cross-row text move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMoveDirection {
    /// Move the text before the caret to the end of the previous same-lane row.
    Previous,
    /// Move the text after the caret to the start of the next same-lane row.
    Next,
}

/// One atomic cross-row move, undoable as a unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossRowTextEdit {
    pub source_segment_id: String,
    pub target_segment_id: String,
    pub source_before: String,
    pub source_after: String,
    pub target_before: String,
    pub target_after: String,
    pub source_before_caret: usize,
    pub source_after_caret: usize,
}

impl CrossRowTextEdit {
    pub fn retained_utf8_bytes(&self) -> usize {
        [
            self.source_segment_id.len(),
            self.target_segment_id.len(),
            self.source_before.len(),
            self.source_after.len(),
            self.target_before.len(),
            self.target_after.len(),
        ]
        .into_iter()
        .fold(0usize, usize::saturating_add)
    }
}

/// Pushes a cross-row move onto a bounded history stack.
pub fn push_cross_row_history(history: &mut Vec<CrossRowTextEdit>, entry: CrossRowTextEdit) {
    push_bounded_history(
        history,
        entry,
        CROSS_ROW_HISTORY_MAX_ENTRIES,
        CROSS_ROW_HISTORY_MAX_RETAINED_BYTES,
        CrossRowTextEdit::retained_utf8_bytes,
    );
}

/// Why a cross-row text move was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextMoveError {
    #[error("No adjacent same-speaker segment found for text move.")]
    NoAdjacentSegment,
    #[error("That segment has no editable draft.")]
    NoDraft(String),
    #[error("No text before the caret to move.")]
    NothingBeforeCaret,
    #[error("No text after the caret to move.")]
    NothingAfterCaret,
    #[error("Text move cancelled: the current segment cannot be left empty.")]
    WouldEmptySource,
}

/// A completed cross-row move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMove {
    pub source_segment_id: String,
    pub target_segment_id: String,
    pub moved_chars: usize,
}

/// SKAZ replacement for a selected number (`1234` → `1 234 {СКАЗ: одна тысяча ...}`), decimal
/// comma, `n/d` fraction, percent, integer range, or a spoken digit sequence.
pub fn skaz_for_selection(selected_text: &str) -> Option<String> {
    build_auto_converted_number_text(selected_text)
}

/// Replacement when a digit is typed over selected spoken digit text: `"{digit} {СКАЗ: sel}"`.
/// The caret belongs one char after the start of the replacement (right after the digit).
pub fn skaz_with_digit(digit: char, selected_text: &str) -> Option<String> {
    (!selected_text.trim().is_empty()).then(|| format!("{digit} {{СКАЗ: {selected_text}}}"))
}

/// Nearest row on the same lane in timeline order (ties broken by project order).
pub fn adjacent_same_lane_segment_id(
    project: &Project,
    segment_id: &str,
    direction: TextMoveDirection,
) -> Option<String> {
    let source_index = project.segments.iter().position(|s| s.id == segment_id)?;
    let source = &project.segments[source_index];
    let order = |left_index: usize, left: &Segment, right_index: usize, right: &Segment| {
        left.start_seconds
            .total_cmp(&right.start_seconds)
            .then_with(|| left_index.cmp(&right_index))
    };
    let candidates = project
        .segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| segment.lane_id == source.lane_id)
        .filter(|(index, segment)| {
            let ordering = order(*index, segment, source_index, source);
            match direction {
                TextMoveDirection::Previous => ordering.is_lt(),
                TextMoveDirection::Next => ordering.is_gt(),
            }
        });
    let pick = match direction {
        TextMoveDirection::Previous => candidates.max_by(|(li, l), (ri, r)| order(*li, l, *ri, r)),
        TextMoveDirection::Next => candidates.min_by(|(li, l), (ri, r)| order(*li, l, *ri, r)),
    };
    pick.map(|(_, segment)| segment.id.clone())
}

/// Joins two text fragments with a single space unless one side already has whitespace.
pub fn join_segment_text(left: &str, right: &str) -> String {
    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() {
        return left.to_string();
    }
    if left.chars().next_back().is_some_and(char::is_whitespace)
        || right.chars().next().is_some_and(char::is_whitespace)
    {
        return format!("{left}{right}");
    }
    format!("{left} {right}")
}

impl Drafts {
    /// Moves the text before (or after) the caret of `segment_id`'s draft into the adjacent
    /// same-lane row. Both drafts record one local undo step; the pair is undoable atomically.
    pub fn move_text_across_rows(
        &mut self,
        project: &Project,
        segment_id: &str,
        direction: TextMoveDirection,
    ) -> Result<TextMove, TextMoveError> {
        let target_segment_id = adjacent_same_lane_segment_id(project, segment_id, direction)
            .ok_or(TextMoveError::NoAdjacentSegment)?;
        let source = self
            .get(segment_id)
            .ok_or_else(|| TextMoveError::NoDraft(segment_id.to_string()))?;
        let source_text = source.text().to_string();
        let split = source.caret_byte_offset();
        let target_text = self
            .get(&target_segment_id)
            .ok_or_else(|| TextMoveError::NoDraft(target_segment_id.clone()))?
            .text()
            .to_string();

        let (moved, next_source, next_target, source_after_caret) = match direction {
            TextMoveDirection::Previous => {
                let moved = source_text[..split].trim().to_string();
                if moved.is_empty() {
                    return Err(TextMoveError::NothingBeforeCaret);
                }
                let next_source = source_text[split..].trim_start().to_string();
                if next_source.is_empty() {
                    return Err(TextMoveError::WouldEmptySource);
                }
                let next_target = join_segment_text(&target_text, &moved);
                (moved, next_source, next_target, 0)
            }
            TextMoveDirection::Next => {
                let moved = source_text[split..].trim().to_string();
                if moved.is_empty() {
                    return Err(TextMoveError::NothingAfterCaret);
                }
                let next_source = source_text[..split].trim_end().to_string();
                if next_source.is_empty() {
                    return Err(TextMoveError::WouldEmptySource);
                }
                let next_target = join_segment_text(&moved, &target_text);
                let caret = next_source.chars().count();
                (moved, next_source, next_target, caret)
            }
        };

        let history = CrossRowTextEdit {
            source_segment_id: segment_id.to_string(),
            target_segment_id: target_segment_id.clone(),
            source_before_caret: source_text[..split].chars().count(),
            source_before: source_text,
            source_after: next_source.clone(),
            target_before: target_text,
            target_after: next_target.clone(),
            source_after_caret,
        };
        if let Some(target) = self.get_mut(&target_segment_id) {
            target.replace_text_from_user_edit(next_target);
        }
        if let Some(source) = self.get_mut(segment_id) {
            source.replace_text_from_user_edit(next_source);
            source.set_selection(TextSelection::caret(source_after_caret));
        }
        push_cross_row_history(&mut self.cross_row_undo, history);
        self.cross_row_redo.clear();

        Ok(TextMove {
            source_segment_id: segment_id.to_string(),
            target_segment_id,
            moved_chars: moved.chars().count(),
        })
    }

    /// Undoes the newest cross-row move touching `segment_id`, if both rows still hold the
    /// moved texts. Returns the `(source, target)` ids that changed.
    pub fn undo_cross_row_move(&mut self, segment_id: &str) -> Option<(String, String)> {
        self.apply_cross_row_history(segment_id, true)
    }

    /// Redoes the newest undone cross-row move touching `segment_id`.
    pub fn redo_cross_row_move(&mut self, segment_id: &str) -> Option<(String, String)> {
        self.apply_cross_row_history(segment_id, false)
    }

    fn apply_cross_row_history(
        &mut self,
        segment_id: &str,
        undo: bool,
    ) -> Option<(String, String)> {
        let stack = if undo {
            &self.cross_row_undo
        } else {
            &self.cross_row_redo
        };
        let history = stack
            .last()
            .filter(|h| h.source_segment_id == segment_id || h.target_segment_id == segment_id)?
            .clone();

        let (source_current, source_next, target_current, target_next, caret) = if undo {
            (
                &history.source_after,
                &history.source_before,
                &history.target_after,
                &history.target_before,
                history.source_before_caret,
            )
        } else {
            (
                &history.source_before,
                &history.source_after,
                &history.target_before,
                &history.target_after,
                history.source_after_caret,
            )
        };
        let ready = |id: &str, current: &str, next: &str| {
            self.get(id).is_some_and(|draft| {
                draft.text() == current
                    && if undo {
                        draft.can_undo_to(next)
                    } else {
                        draft.can_redo_to(next)
                    }
            })
        };
        if !ready(&history.source_segment_id, source_current, source_next)
            || !ready(&history.target_segment_id, target_current, target_next)
        {
            return None;
        }

        for (id, is_source) in [
            (&history.source_segment_id, true),
            (&history.target_segment_id, false),
        ] {
            if let Some(draft) = self.get_mut(id) {
                if undo {
                    draft.undo();
                } else {
                    draft.redo();
                }
                if is_source {
                    draft.set_selection(TextSelection::caret(caret));
                }
            }
        }

        if undo {
            self.cross_row_undo.pop();
            push_cross_row_history(&mut self.cross_row_redo, history.clone());
        } else {
            self.cross_row_redo.pop();
            push_cross_row_history(&mut self.cross_row_undo, history.clone());
        }
        Some((history.source_segment_id, history.target_segment_id))
    }
}

/// Splits `text` at a `char` offset.
pub fn split_text_at_char_offset(text: &str, char_offset: usize) -> (&str, &str) {
    text.split_at(byte_offset(text, char_offset))
}

const HUNDREDS: [&str; 9] = [
    "сто",
    "двести",
    "триста",
    "четыреста",
    "пятьсот",
    "шестьсот",
    "семьсот",
    "восемьсот",
    "девятьсот",
];

const TENS: [&str; 8] = [
    "двадцать",
    "тридцать",
    "сорок",
    "пятьдесят",
    "шестьдесят",
    "семьдесят",
    "восемьдесят",
    "девяносто",
];

const TEENS: [&str; 10] = [
    "десять",
    "одиннадцать",
    "двенадцать",
    "тринадцать",
    "четырнадцать",
    "пятнадцать",
    "шестнадцать",
    "семнадцать",
    "восемнадцать",
    "девятнадцать",
];

const UNITS_MALE: [&str; 9] = [
    "один",
    "два",
    "три",
    "четыре",
    "пять",
    "шесть",
    "семь",
    "восемь",
    "девять",
];

const UNITS_FEMALE: [&str; 9] = [
    "одна",
    "две",
    "три",
    "четыре",
    "пять",
    "шесть",
    "семь",
    "восемь",
    "девять",
];

const SCALE_FORMS: [(&str, &str, &str); 4] = [
    ("", "", ""),
    ("тысяча", "тысячи", "тысяч"),
    ("миллион", "миллиона", "миллионов"),
    ("миллиард", "миллиарда", "миллиардов"),
];

const DIGIT_WORDS: [&str; 10] = [
    "ноль",
    "один",
    "два",
    "три",
    "четыре",
    "пять",
    "шесть",
    "семь",
    "восемь",
    "девять",
];

fn build_auto_converted_number_text(selected_text: &str) -> Option<String> {
    build_expanded_skaz_text(selected_text)
        .or_else(|| build_skaz_from_slash_fraction(selected_text))
        .or_else(|| build_expanded_skaz_for_numeric_pattern(selected_text))
        .or_else(|| build_skaz_from_spoken_digit_sequence(selected_text))
}

fn build_expanded_skaz_text(selected_text: &str) -> Option<String> {
    let trimmed = selected_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if is_decimal_comma_text(trimmed) {
        let words = decimal_comma_text_to_russian_words(trimmed)?;
        return Some(format!(
            "{} {{СКАЗ: {words}}}",
            format_grouped_decimal_text(trimmed)
        ));
    }

    if !is_integer_text(trimmed) {
        return None;
    }

    let words = number_text_to_russian_words(trimmed)?;
    Some(format!(
        "{} {{СКАЗ: {words}}}",
        format_grouped_integer_text(trimmed)
    ))
}

fn build_skaz_from_slash_fraction(selected_text: &str) -> Option<String> {
    let trimmed = selected_text.trim();
    let (numerator_text, denominator_text) = split_slash_fraction(trimmed)?;
    let denominator = denominator_text.parse::<i64>().ok()?;
    if !(2..=20).contains(&denominator) {
        return None;
    }
    let numerator = numerator_text.parse::<i64>().ok()?;
    let numerator_words = integer_text_to_russian_words(numerator_text, true)?;
    let denominator_word = choose_fraction_denominator_word(denominator, numerator)?;
    Some(format!(
        "{trimmed} {{СКАЗ: {numerator_words} {denominator_word}}}"
    ))
}

fn build_expanded_skaz_for_numeric_pattern(selected_text: &str) -> Option<String> {
    let trimmed = selected_text.trim();
    if let Some(digits) = trimmed.strip_suffix('%').map(str::trim_end)
        && is_integer_text(digits)
    {
        let words = number_text_to_russian_words(digits)?;
        let percent_word = pick_plural_form(
            digits.parse::<i64>().ok()?,
            "процент",
            "процента",
            "процентов",
        );
        return Some(format!(
            "{} % {{СКАЗ: {words} {percent_word}}}",
            format_grouped_integer_text(digits)
        ));
    }

    let parts = parse_integer_range_parts(trimmed)?;
    if parts.len() < 2 {
        return None;
    }
    let words = parts
        .iter()
        .map(|part| number_text_to_russian_words(part))
        .collect::<Option<Vec<_>>>()?;
    let grouped = parts
        .iter()
        .map(|part| format_grouped_integer_text(part))
        .collect::<Vec<_>>()
        .join("-");
    Some(format!("{grouped} {{СКАЗ: {}}}", words.join(" ")))
}

fn build_skaz_from_spoken_digit_sequence(selected_text: &str) -> Option<String> {
    let trimmed = selected_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tokens = trimmed
        .to_lowercase()
        .split_whitespace()
        .map(spoken_digit_word)
        .collect::<Option<Vec<_>>>()?;
    if tokens.len() < 2 {
        return None;
    }
    Some(format!("{} {{СКАЗ: {trimmed}}}", tokens.join("-")))
}

fn number_text_to_russian_words(number_text: &str) -> Option<String> {
    integer_text_to_russian_words(number_text, false)
}

fn decimal_comma_text_to_russian_words(number_text: &str) -> Option<String> {
    if !is_decimal_comma_text(number_text) {
        return None;
    }
    let (sign_prefix, unsigned) = unsigned_with_sign_prefix(number_text);
    let (integer_part, fraction_part) = unsigned.split_once(',')?;
    let integer_words = number_text_to_russian_words(integer_part)?;
    let fraction_words = fraction_part
        .chars()
        .map(|character| {
            character
                .to_digit(10)
                .and_then(|digit| DIGIT_WORDS.get(digit as usize).copied())
        })
        .collect::<Option<Vec<_>>>()?;
    if fraction_words.len() != fraction_part.len() || fraction_words.is_empty() {
        return None;
    }
    Some(format!(
        "{sign_prefix}{integer_words} и {}",
        fraction_words.join(" ")
    ))
}

fn integer_text_to_russian_words(number_text: &str, last_triad_feminine: bool) -> Option<String> {
    if !is_integer_text(number_text) {
        return None;
    }

    let (sign_prefix, mut digits) = unsigned_with_sign_prefix(number_text);
    digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some(format!("{sign_prefix}ноль"));
    }
    if digits.len() > MAX_SUPPORTED_DIGITS {
        return None;
    }

    let mut triads = Vec::new();
    let mut end = digits.len();
    while end > 0 {
        let start = end.saturating_sub(3);
        triads.insert(0, &digits[start..end]);
        end = start;
    }

    let mut parts = Vec::new();
    for (index, triad) in triads.iter().enumerate() {
        let triad_value = triad.parse::<i64>().ok()?;
        if triad_value == 0 {
            continue;
        }
        let scale_index = triads.len().saturating_sub(index + 1);
        let is_feminine = scale_index == 1 || (scale_index == 0 && last_triad_feminine);
        let triad_words = three_digit_to_words(triad_value, is_feminine)?;
        if scale_index > 0 {
            let (one, two_to_four, many) = SCALE_FORMS.get(scale_index).copied()?;
            parts.push(format!(
                "{triad_words} {}",
                pick_plural_form(triad_value, one, two_to_four, many)
            ));
        } else {
            parts.push(triad_words);
        }
    }

    if parts.is_empty() {
        Some(format!("{sign_prefix}ноль"))
    } else {
        Some(format!("{sign_prefix}{}", parts.join(" ")))
    }
}

fn three_digit_to_words(value: i64, is_feminine: bool) -> Option<String> {
    let mut parts = Vec::new();
    let hundreds = value / 100;
    let remainder = value % 100;
    let tens = remainder / 10;
    let units = remainder % 10;

    if hundreds > 0 {
        parts.push(*HUNDREDS.get(hundreds as usize - 1)?);
    }
    if (10..=19).contains(&remainder) {
        parts.push(*TEENS.get(remainder as usize - 10)?);
        return Some(parts.join(" "));
    }
    if tens >= 2 {
        parts.push(*TENS.get(tens as usize - 2)?);
    }
    if units > 0 {
        let units_table = if is_feminine {
            &UNITS_FEMALE
        } else {
            &UNITS_MALE
        };
        parts.push(*units_table.get(units as usize - 1)?);
    }
    Some(parts.join(" "))
}

fn choose_fraction_denominator_word(denominator: i64, numerator: i64) -> Option<&'static str> {
    let (singular, plural, plural_few) = match denominator {
        2 => ("вторая", "вторых", None),
        3 => ("третья", "третьих", None),
        4 => ("четвертая", "четвертых", Some("четверти")),
        5 => ("пятая", "пятых", None),
        6 => ("шестая", "шестых", None),
        7 => ("седьмая", "седьмых", None),
        8 => ("восьмая", "восьмых", None),
        9 => ("девятая", "девятых", None),
        10 => ("десятая", "десятых", None),
        11 => ("одиннадцатая", "одиннадцатых", None),
        12 => ("двенадцатая", "двенадцатых", None),
        13 => ("тринадцатая", "тринадцатых", None),
        14 => ("четырнадцатая", "четырнадцатых", None),
        15 => ("пятнадцатая", "пятнадцатых", None),
        16 => ("шестнадцатая", "шестнадцатых", None),
        17 => ("семнадцатая", "семнадцатых", None),
        18 => ("восемнадцатая", "восемнадцатых", None),
        19 => ("девятнадцатая", "девятнадцатых", None),
        20 => ("двадцатая", "двадцатых", None),
        _ => return None,
    };

    let absolute = numerator.abs();
    let mod10 = absolute % 10;
    let mod100 = absolute % 100;
    if mod10 == 1 && mod100 != 11 {
        return Some(singular);
    }
    if let Some(plural_few) = plural_few
        && (2..=4).contains(&mod10)
        && !(12..=14).contains(&mod100)
    {
        return Some(plural_few);
    }
    Some(plural)
}

fn pick_plural_form<'a>(value: i64, one: &'a str, two_to_four: &'a str, many: &'a str) -> &'a str {
    let absolute = value.abs();
    let mod10 = absolute % 10;
    let mod100 = absolute % 100;
    if mod10 == 1 && mod100 != 11 {
        one
    } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
        two_to_four
    } else {
        many
    }
}

fn format_grouped_integer_text(number_text: &str) -> String {
    if !is_integer_text(number_text) {
        return number_text.to_string();
    }
    let sign = number_text.strip_prefix('-').map(|_| "-").unwrap_or("");
    let digits = number_text.strip_prefix('-').unwrap_or(number_text);
    if digits.len() <= 3 || (digits.len() > 1 && digits.starts_with('0')) {
        return number_text.to_string();
    }

    let mut grouped = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(character);
    }
    format!("{sign}{grouped}")
}

fn format_grouped_decimal_text(number_text: &str) -> String {
    if !is_decimal_comma_text(number_text) {
        return number_text.to_string();
    }
    let sign = number_text.strip_prefix('-').map(|_| "-").unwrap_or("");
    let unsigned = number_text.strip_prefix('-').unwrap_or(number_text);
    let Some((integer_part, fraction_part)) = unsigned.split_once(',') else {
        return number_text.to_string();
    };
    let grouped_integer_part = format_grouped_integer_text(&format!("{sign}{integer_part}"));
    let normalized_integer_part = grouped_integer_part
        .strip_prefix('-')
        .unwrap_or(&grouped_integer_part);
    format!("{sign}{normalized_integer_part},{fraction_part}")
}

fn split_slash_fraction(trimmed: &str) -> Option<(&str, &str)> {
    let (numerator, denominator) = trimmed.split_once('/')?;
    let numerator = numerator.trim();
    let denominator = denominator.trim();
    if !is_integer_text(numerator)
        || denominator.starts_with('-')
        || !is_unsigned_integer(denominator)
    {
        return None;
    }
    Some((numerator, denominator))
}

fn parse_integer_range_parts(trimmed: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut index = 0;
    let bytes = trimmed.as_bytes();

    loop {
        skip_ascii_whitespace(trimmed, &mut index);
        let start = index;
        if bytes.get(index) == Some(&b'-') {
            index += 1;
        }
        let digit_start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if digit_start == index {
            return None;
        }
        parts.push(&trimmed[start..index]);
        skip_ascii_whitespace(trimmed, &mut index);
        if index == trimmed.len() {
            break;
        }
        if bytes.get(index) != Some(&b'-') {
            return None;
        }
        index += 1;
    }

    (parts.len() >= 2).then_some(parts)
}

fn skip_ascii_whitespace(text: &str, index: &mut usize) {
    while text
        .as_bytes()
        .get(*index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        *index += 1;
    }
}

fn is_integer_text(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    !unsigned.is_empty() && unsigned.chars().all(|character| character.is_ascii_digit())
}

fn is_unsigned_integer(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|character| character.is_ascii_digit())
}

fn is_decimal_comma_text(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let Some((integer_part, fraction_part)) = unsigned.split_once(',') else {
        return false;
    };
    !integer_part.is_empty()
        && !fraction_part.is_empty()
        && is_unsigned_integer(integer_part)
        && is_unsigned_integer(fraction_part)
}

fn unsigned_with_sign_prefix(text: &str) -> (&'static str, &str) {
    if let Some(unsigned) = text.strip_prefix('-') {
        ("минус ", unsigned)
    } else {
        ("", text)
    }
}

fn spoken_digit_word(token: &str) -> Option<&'static str> {
    match token {
        "ноль" => Some("0"),
        "один" | "одна" => Some("1"),
        "два" | "две" => Some("2"),
        "три" => Some("3"),
        "четыре" => Some("4"),
        "пять" => Some("5"),
        "шесть" => Some("6"),
        "семь" => Some("7"),
        "восемь" => Some("8"),
        "девять" => Some("9"),
        _ => None,
    }
}
