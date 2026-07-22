use iced::widget::text_editor::{self, Motion};

use super::{SegmentDraft, StudioState, drafts::push_bounded_history};

const MAX_SUPPORTED_DIGITS: usize = 12;
// Cross-row moves duplicate snapshots from two segments, so each direction is
// capped at 32 moves and 2 MiB of retained UTF-8 data. As with segment history,
// one newest oversized move remains available for immediate undo or redo.
const CROSS_ROW_HISTORY_MAX_ENTRIES: usize = 32;
const CROSS_ROW_HISTORY_MAX_RETAINED_BYTES: usize = 2 * 1024 * 1024;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextMoveDirection {
    Previous,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CrossRowTextEdit {
    source_segment_id: String,
    target_segment_id: String,
    source_before: String,
    source_after: String,
    target_before: String,
    target_after: String,
    source_before_cursor_chars: usize,
    source_after_cursor_chars: usize,
}

impl CrossRowTextEdit {
    fn retained_utf8_bytes(&self) -> usize {
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

fn push_cross_row_text_history(history: &mut Vec<CrossRowTextEdit>, entry: CrossRowTextEdit) {
    push_bounded_history(
        history,
        entry,
        CROSS_ROW_HISTORY_MAX_ENTRIES,
        CROSS_ROW_HISTORY_MAX_RETAINED_BYTES,
        CrossRowTextEdit::retained_utf8_bytes,
    );
}

impl StudioState {
    pub(super) fn move_text_before_caret_to_previous_segment(&mut self, segment_id: &str) -> bool {
        self.move_text_to_adjacent_same_lane(segment_id, TextMoveDirection::Previous)
    }

    pub(super) fn move_text_after_caret_to_next_segment(&mut self, segment_id: &str) -> bool {
        self.move_text_to_adjacent_same_lane(segment_id, TextMoveDirection::Next)
    }

    pub(super) fn undo_cross_row_text_move(&mut self, segment_id: &str) -> bool {
        self.apply_cross_row_text_history(segment_id, true)
    }

    pub(super) fn redo_cross_row_text_move(&mut self, segment_id: &str) -> bool {
        self.apply_cross_row_text_history(segment_id, false)
    }

    fn apply_cross_row_text_history(&mut self, segment_id: &str, undo: bool) -> bool {
        let history = if undo {
            self.cross_row_text_undo_stack.last()
        } else {
            self.cross_row_text_redo_stack.last()
        }
        .filter(|history| {
            history.source_segment_id == segment_id || history.target_segment_id == segment_id
        })
        .cloned();
        let Some(history) = history else {
            return false;
        };

        let (source_current, source_next, target_current, target_next, source_cursor_chars) =
            if undo {
                (
                    history.source_after.as_str(),
                    history.source_before.as_str(),
                    history.target_after.as_str(),
                    history.target_before.as_str(),
                    history.source_before_cursor_chars,
                )
            } else {
                (
                    history.source_before.as_str(),
                    history.source_after.as_str(),
                    history.target_before.as_str(),
                    history.target_after.as_str(),
                    history.source_after_cursor_chars,
                )
            };
        let source_ready = self
            .segment_drafts
            .get(&history.source_segment_id)
            .is_some_and(|draft| {
                draft.text == source_current
                    && if undo {
                        draft.can_undo_text_edit_to(source_next)
                    } else {
                        draft.can_redo_text_edit_to(source_next)
                    }
            });
        let target_ready = self
            .segment_drafts
            .get(&history.target_segment_id)
            .is_some_and(|draft| {
                draft.text == target_current
                    && if undo {
                        draft.can_undo_text_edit_to(target_next)
                    } else {
                        draft.can_redo_text_edit_to(target_next)
                    }
            });
        if !source_ready || !target_ready {
            return false;
        }

        let source_before_height = self
            .segment_drafts
            .get(&history.source_segment_id)
            .map_or(0.0, SegmentDraft::text_editor_content_height);
        let target_before_height = self
            .segment_drafts
            .get(&history.target_segment_id)
            .map_or(0.0, SegmentDraft::text_editor_content_height);
        if let Some(source) = self.segment_drafts.get_mut(&history.source_segment_id) {
            if undo {
                source.undo_text_edit();
            } else {
                source.redo_text_edit();
            }
            source.set_cursor_to_char_offset(source_cursor_chars);
        }
        if let Some(target) = self.segment_drafts.get_mut(&history.target_segment_id) {
            if undo {
                target.undo_text_edit();
            } else {
                target.redo_text_edit();
            }
        }

        if undo {
            self.cross_row_text_undo_stack.pop();
            push_cross_row_text_history(&mut self.cross_row_text_redo_stack, history.clone());
        } else {
            self.cross_row_text_redo_stack.pop();
            push_cross_row_text_history(&mut self.cross_row_text_undo_stack, history.clone());
        }
        self.mark_dirty();
        self.clear_tag_suggestion_state_for_segment(&history.source_segment_id);
        self.clear_tag_suggestion_state_for_segment(&history.target_segment_id);
        let layout_height_changed = self
            .segment_drafts
            .get(&history.source_segment_id)
            .is_some_and(|draft| {
                draft.text_editor_content_height().to_bits() != source_before_height.to_bits()
            })
            || self
                .segment_drafts
                .get(&history.target_segment_id)
                .is_some_and(|draft| {
                    draft.text_editor_content_height().to_bits() != target_before_height.to_bits()
                });
        if layout_height_changed {
            self.invalidate_transcript_layout_cache();
        }
        self.status = if undo {
            "Undid cross-row text move.".to_string()
        } else {
            "Redid cross-row text move.".to_string()
        };
        true
    }

    pub(super) fn convert_active_selection_to_skaz(&mut self) -> bool {
        let Some(segment_id) = self.selected_segment_id() else {
            self.status = "Select a transcript segment before converting SKAZ.".to_string();
            return false;
        };

        let Some(selection) = self
            .segment_drafts
            .get(&segment_id)
            .and_then(|draft| draft.text_content.selection())
        else {
            self.status = "Select numeric text before converting SKAZ.".to_string();
            return false;
        };

        let Some(replacement) = build_auto_converted_number_text(&selection) else {
            self.status = format!(
                "Selected text `{}` is not a supported SKAZ number.",
                selection
            );
            return false;
        };

        self.replace_segment_selection_with_text(&segment_id, replacement, None)
    }

    pub(super) fn convert_segment_selection_with_digit(
        &mut self,
        segment_id: &str,
        digit: char,
    ) -> bool {
        let Some(selection) = self
            .segment_drafts
            .get(segment_id)
            .and_then(|draft| draft.text_content.selection())
        else {
            self.status = "Select spoken digit text before typing a SKAZ digit.".to_string();
            return false;
        };

        if selection.trim().is_empty() {
            self.status = "Select spoken digit text before typing a SKAZ digit.".to_string();
            return false;
        }

        let replacement = format!("{digit} {{СКАЗ: {selection}}}");
        self.replace_segment_selection_with_text(segment_id, replacement, Some(1))
    }

    fn move_text_to_adjacent_same_lane(
        &mut self,
        segment_id: &str,
        direction: TextMoveDirection,
    ) -> bool {
        let source_segment_id = segment_id.to_string();
        let Some(target_segment_id) =
            self.adjacent_same_lane_segment_id(&source_segment_id, direction)
        else {
            self.status = "No adjacent same-speaker segment found for text move.".to_string();
            return false;
        };

        let Some(source_draft) = self.segment_drafts.get(&source_segment_id) else {
            self.status = format!("Segment `{source_segment_id}` has no editable draft.");
            return false;
        };
        let source_text = source_draft.editor_text();
        let split_index =
            cursor_byte_offset(&source_text, source_draft.text_content.cursor_position())
                .filter(|index| source_text.is_char_boundary(*index))
                .unwrap_or(source_text.len());

        let Some(target_draft) = self.segment_drafts.get(&target_segment_id) else {
            self.status = format!("Segment `{target_segment_id}` has no editable draft.");
            return false;
        };
        let target_text = target_draft.editor_text();

        let (moved_text, next_source_text, next_target_text, source_cursor_chars) = match direction
        {
            TextMoveDirection::Previous => {
                let moved_text = source_text[..split_index].trim().to_string();
                if moved_text.is_empty() {
                    self.status = "No text before the caret to move.".to_string();
                    return false;
                }
                let next_source_text = source_text[split_index..].trim_start().to_string();
                if next_source_text.is_empty() {
                    self.status = "Text move cancelled: the current segment cannot be left empty."
                        .to_string();
                    return false;
                }
                let next_target_text = join_segment_text(&target_text, &moved_text);
                (moved_text, next_source_text, next_target_text, 0)
            }
            TextMoveDirection::Next => {
                let moved_text = source_text[split_index..].trim().to_string();
                if moved_text.is_empty() {
                    self.status = "No text after the caret to move.".to_string();
                    return false;
                }
                let next_source_text = source_text[..split_index].trim_end().to_string();
                if next_source_text.is_empty() {
                    self.status = "Text move cancelled: the current segment cannot be left empty."
                        .to_string();
                    return false;
                }
                let next_target_text = join_segment_text(&moved_text, &target_text);
                let source_cursor_chars = next_source_text.chars().count();
                (
                    moved_text,
                    next_source_text,
                    next_target_text,
                    source_cursor_chars,
                )
            }
        };

        let source_before_cursor_chars = source_text[..split_index].chars().count();
        let history = CrossRowTextEdit {
            source_segment_id: source_segment_id.clone(),
            target_segment_id: target_segment_id.clone(),
            source_before: source_text,
            source_after: next_source_text.clone(),
            target_before: target_text,
            target_after: next_target_text.clone(),
            source_before_cursor_chars,
            source_after_cursor_chars: source_cursor_chars,
        };

        let mut layout_height_changed = false;
        if let Some(target_draft) = self.segment_drafts.get_mut(&target_segment_id) {
            layout_height_changed |=
                replace_draft_text_from_user_edit(target_draft, next_target_text, None);
        }
        if let Some(source_draft) = self.segment_drafts.get_mut(&source_segment_id) {
            layout_height_changed |= replace_draft_text_from_user_edit(
                source_draft,
                next_source_text,
                Some(source_cursor_chars),
            );
        }
        push_cross_row_text_history(&mut self.cross_row_text_undo_stack, history);
        self.cross_row_text_redo_stack.clear();

        self.mark_dirty();
        self.clear_tag_suggestion_state_for_segment(&source_segment_id);
        self.clear_tag_suggestion_state_for_segment(&target_segment_id);
        if layout_height_changed {
            self.invalidate_transcript_layout_cache();
        }

        self.status = match direction {
            TextMoveDirection::Previous => format!(
                "Moved {} characters to the previous same-speaker segment.",
                moved_text.chars().count()
            ),
            TextMoveDirection::Next => format!(
                "Moved {} characters to the next same-speaker segment.",
                moved_text.chars().count()
            ),
        };
        true
    }

    fn replace_segment_selection_with_text(
        &mut self,
        segment_id: &str,
        replacement: String,
        cursor_chars: Option<usize>,
    ) -> bool {
        let Some(draft) = self.segment_drafts.get_mut(segment_id) else {
            self.status = format!("Segment `{segment_id}` has no editable draft.");
            return false;
        };

        let before = draft.text.clone();
        let before_height = draft.text_editor_content_height();
        let text_changed;
        let layout_height_changed;
        {
            draft.perform_text_edit(text_editor::Action::Edit(text_editor::Edit::Paste(
                replacement.clone().into(),
            )));
            if let Some(cursor_chars) = cursor_chars {
                let trailing_chars = replacement.chars().count().saturating_sub(cursor_chars);
                for _ in 0..trailing_chars {
                    draft
                        .text_content
                        .perform(text_editor::Action::Move(Motion::Left));
                }
            }
            text_changed = draft.text != before;
            layout_height_changed =
                draft.text_editor_content_height().to_bits() != before_height.to_bits();
        }

        if !text_changed {
            return false;
        }

        self.mark_dirty();
        self.clear_tag_suggestion_state_for_segment(segment_id);
        if layout_height_changed {
            self.invalidate_transcript_layout_cache();
        }
        self.status = "Converted selected text to SKAZ.".to_string();
        true
    }

    fn selected_segment_id(&self) -> Option<String> {
        self.helper_focused_segment_id.clone().or_else(|| {
            self.editor
                .as_ref()
                .and_then(|editor| editor.selected_segment_id.clone())
        })
    }

    fn adjacent_same_lane_segment_id(
        &self,
        segment_id: &str,
        direction: TextMoveDirection,
    ) -> Option<String> {
        let project = self.project()?;
        let source_index = project
            .segments
            .iter()
            .position(|segment| segment.id == segment_id)?;
        let source = &project.segments[source_index];
        let compare_timeline_position =
            |left_index: usize,
             left: &studio_core::Segment,
             right_index: usize,
             right: &studio_core::Segment| {
                left.start_seconds
                    .total_cmp(&right.start_seconds)
                    .then_with(|| left_index.cmp(&right_index))
            };

        let adjacent = project
            .segments
            .iter()
            .enumerate()
            .filter(|(_, segment)| segment.lane_id == source.lane_id)
            .filter(|(index, segment)| {
                let ordering = compare_timeline_position(*index, segment, source_index, source);
                match direction {
                    TextMoveDirection::Previous => ordering.is_lt(),
                    TextMoveDirection::Next => ordering.is_gt(),
                }
            });

        match direction {
            TextMoveDirection::Previous => adjacent
                .max_by(|(left_index, left), (right_index, right)| {
                    compare_timeline_position(*left_index, left, *right_index, right)
                })
                .map(|(_, segment)| segment.id.clone()),
            TextMoveDirection::Next => adjacent
                .min_by(|(left_index, left), (right_index, right)| {
                    compare_timeline_position(*left_index, left, *right_index, right)
                })
                .map(|(_, segment)| segment.id.clone()),
        }
    }
}

fn replace_draft_text_from_user_edit(
    draft: &mut SegmentDraft,
    text: String,
    cursor_chars: Option<usize>,
) -> bool {
    let before_height = draft.text_editor_content_height();
    draft.replace_text_from_user_edit(text);
    if let Some(cursor_chars) = cursor_chars {
        for _ in 0..cursor_chars {
            draft
                .text_content
                .perform(text_editor::Action::Move(Motion::Right));
        }
    }
    draft.text_editor_content_height().to_bits() != before_height.to_bits()
}

fn join_segment_text(left: &str, right: &str) -> String {
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

fn cursor_byte_offset(text: &str, cursor_position: (usize, usize)) -> Option<usize> {
    let (cursor_line, cursor_column) = cursor_position;
    let mut offset = 0;

    for (line_index, line) in text.split('\n').enumerate() {
        if line_index == cursor_line {
            return byte_offset_at_column(line, cursor_column)
                .map(|column_offset| offset + column_offset);
        }
        offset += line.len() + '\n'.len_utf8();
    }

    if cursor_line == text.lines().count() && cursor_column == 0 && text.ends_with('\n') {
        Some(text.len())
    } else {
        None
    }
}

fn byte_offset_at_column(line: &str, column: usize) -> Option<usize> {
    (column <= line.len() && line.is_char_boundary(column)).then_some(column)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(index: usize, snapshot: String) -> CrossRowTextEdit {
        CrossRowTextEdit {
            source_segment_id: format!("source-{index}"),
            target_segment_id: format!("target-{index}"),
            source_before: snapshot.clone(),
            source_after: snapshot.clone(),
            target_before: snapshot.clone(),
            target_after: snapshot,
            source_before_cursor_chars: index,
            source_after_cursor_chars: index,
        }
    }

    fn retained_bytes(history: &[CrossRowTextEdit]) -> usize {
        history
            .iter()
            .map(CrossRowTextEdit::retained_utf8_bytes)
            .sum()
    }

    #[test]
    fn cross_row_history_keeps_the_newest_fixed_count() {
        let mut entries = Vec::new();
        for index in 0..5_000 {
            push_cross_row_text_history(&mut entries, history(index, String::new()));
        }

        assert_eq!(entries.len(), CROSS_ROW_HISTORY_MAX_ENTRIES);
        assert_eq!(
            entries.first().unwrap().source_segment_id,
            format!("source-{}", 5_000 - CROSS_ROW_HISTORY_MAX_ENTRIES)
        );
        assert_eq!(entries.last().unwrap().source_segment_id, "source-4999");
        assert!(retained_bytes(&entries) <= CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
    }

    #[test]
    fn cross_row_history_is_bounded_by_all_retained_utf8_snapshots() {
        let mut entries = Vec::new();
        for index in 0..20 {
            push_cross_row_text_history(&mut entries, history(index, "я".repeat(32 * 1024)));
        }

        assert!(entries.len() < CROSS_ROW_HISTORY_MAX_ENTRIES);
        assert!(retained_bytes(&entries) <= CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
        assert_eq!(entries.last().unwrap().source_segment_id, "source-19");
    }

    #[test]
    fn cross_row_history_keeps_one_newest_oversized_move() {
        let mut entries = Vec::new();
        push_cross_row_text_history(&mut entries, history(0, String::new()));
        push_cross_row_text_history(
            &mut entries,
            history(1, "x".repeat(CROSS_ROW_HISTORY_MAX_RETAINED_BYTES)),
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source_segment_id, "source-1");
        assert!(retained_bytes(&entries) > CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
    }
}
