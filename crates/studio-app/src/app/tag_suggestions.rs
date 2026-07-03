use iced::widget::text_editor::{self, Motion};

use studio_app::{RUSSIAN_SQUARE_BRACKET_TAGS, RUSSIAN_STYLE_TAGS};

use super::SegmentDraft;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TagSuggestionKind {
    Bracket,
    StyleOpen,
    StyleClose,
    Curly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TagSuggestionKeyboardAction {
    Previous,
    Next,
    Accept,
    Dismiss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TagSuggestion {
    pub(super) label: String,
    pub(super) insert_text: String,
    pub(super) kind: TagSuggestionKind,
    pub(super) replacement_start: usize,
    pub(super) replacement_end: usize,
    pub(super) backspace_count: usize,
    pub(super) cursor_back_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TagTrigger<'a> {
    kind: TagSuggestionKind,
    partial: &'a str,
    start: usize,
    end: usize,
}

const BABEL_CURLY_TAGS: &[&str] = &["MIS: ", "PRO: "];

pub(super) fn tag_suggestions_for_draft(draft: &SegmentDraft) -> Vec<TagSuggestion> {
    if let Some(selection) = draft.text_content.selection() {
        if selection.trim().is_empty() {
            return Vec::new();
        }
        return RUSSIAN_STYLE_TAGS
            .iter()
            .copied()
            .map(selection_wrap_suggestion_for_style_tag)
            .collect();
    }

    let text = draft.editor_text();
    let cursor = cursor_byte_offset(&text, draft.text_content.cursor_position())
        .filter(|cursor| text.is_char_boundary(*cursor))
        .unwrap_or(text.len());
    tag_suggestions_for_text(&text, cursor)
}

pub(super) fn tag_suggestions_for_text(text: &str, cursor: usize) -> Vec<TagSuggestion> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return Vec::new();
    }

    let Some(trigger) = tag_trigger_before_cursor(text, cursor) else {
        return Vec::new();
    };

    let partial = trigger.partial.to_lowercase();
    let tags = match trigger.kind {
        TagSuggestionKind::Bracket => RUSSIAN_SQUARE_BRACKET_TAGS,
        TagSuggestionKind::StyleOpen | TagSuggestionKind::StyleClose => RUSSIAN_STYLE_TAGS,
        TagSuggestionKind::Curly => BABEL_CURLY_TAGS,
    };

    tags.iter()
        .copied()
        .filter(|tag| tag_matches_partial(tag, &partial, trigger.kind))
        .map(|tag| suggestion_for_tag(tag, trigger))
        .collect()
}

pub(super) fn apply_tag_suggestion(draft: &mut SegmentDraft, suggestion: &TagSuggestion) {
    if let Some(selection) = draft.text_content.selection()
        && let Some(replacement) = selected_text_replacement(&selection, suggestion)
    {
        draft.perform_text_edit(text_editor::Action::Edit(text_editor::Edit::Paste(
            replacement.into(),
        )));
        return;
    }

    let text = draft.editor_text();
    if suggestion.replacement_start > suggestion.replacement_end
        || suggestion.replacement_end > text.len()
        || !text.is_char_boundary(suggestion.replacement_start)
        || !text.is_char_boundary(suggestion.replacement_end)
    {
        return;
    }

    let mut updated = String::with_capacity(
        text.len() - (suggestion.replacement_end - suggestion.replacement_start)
            + suggestion.insert_text.len(),
    );
    updated.push_str(&text[..suggestion.replacement_start]);
    updated.push_str(&suggestion.insert_text);
    updated.push_str(&text[suggestion.replacement_end..]);

    let cursor_offset = text[..suggestion.replacement_start].chars().count()
        + suggestion
            .insert_text
            .chars()
            .count()
            .saturating_sub(suggestion.cursor_back_count);
    draft.replace_text_from_user_edit(updated);
    for _ in 0..cursor_offset {
        draft
            .text_content
            .perform(text_editor::Action::Move(Motion::Right));
    }
}

fn selection_wrap_suggestion_for_style_tag(tag: &str) -> TagSuggestion {
    TagSuggestion {
        label: format!("<{tag}>"),
        insert_text: format!("<{tag}></{tag}>"),
        kind: TagSuggestionKind::StyleOpen,
        replacement_start: 0,
        replacement_end: 0,
        backspace_count: 0,
        cursor_back_count: 0,
    }
}

fn selected_text_replacement(selection: &str, suggestion: &TagSuggestion) -> Option<String> {
    if selection.is_empty() {
        return None;
    }
    match suggestion.kind {
        TagSuggestionKind::StyleOpen => {
            let tag = style_tag_name_from_label(&suggestion.label)?;
            Some(format!("<{tag}>{selection}</{tag}>"))
        }
        TagSuggestionKind::Bracket | TagSuggestionKind::StyleClose | TagSuggestionKind::Curly => {
            None
        }
    }
}

fn style_tag_name_from_label(label: &str) -> Option<&str> {
    label
        .strip_prefix('<')?
        .strip_suffix('>')
        .filter(|tag| !tag.is_empty() && !tag.starts_with('/'))
}

fn suggestion_for_tag(tag: &str, trigger: TagTrigger<'_>) -> TagSuggestion {
    let (label, insert_text, cursor_back_count) = match trigger.kind {
        TagSuggestionKind::Bracket => (tag.to_string(), format!("[{tag}]"), 0),
        TagSuggestionKind::StyleOpen => {
            let insert_text = format!("<{tag}></{tag}>");
            let cursor_back_count = tag.chars().count() + 3;
            (format!("<{tag}>"), insert_text, cursor_back_count)
        }
        TagSuggestionKind::StyleClose => {
            let insert_text = format!("</{tag}>");
            (insert_text.clone(), insert_text, 0)
        }
        TagSuggestionKind::Curly => {
            let label = tag.trim().to_string();
            (label, format!("{{{tag}}}"), 1)
        }
    };

    TagSuggestion {
        label,
        insert_text,
        kind: trigger.kind,
        replacement_start: trigger.start,
        replacement_end: trigger.end,
        backspace_count: trigger.backspace_count(),
        cursor_back_count,
    }
}

fn tag_trigger_before_cursor(text: &str, cursor: usize) -> Option<TagTrigger<'_>> {
    let prefix = &text[..cursor];
    if let Some(start) = prefix.rfind("</") {
        let partial_start = start + "</".len();
        let partial = &prefix[partial_start..];
        if is_valid_partial(partial) {
            return Some(TagTrigger {
                kind: TagSuggestionKind::StyleClose,
                partial,
                start,
                end: cursor,
            });
        }
    }

    if let Some(start) = prefix.rfind('<') {
        let partial_start = start + '<'.len_utf8();
        let partial = &prefix[partial_start..];
        if !partial.starts_with('/') && is_valid_partial(partial) {
            return Some(TagTrigger {
                kind: TagSuggestionKind::StyleOpen,
                partial,
                start,
                end: cursor,
            });
        }
    }

    if let Some(start) = prefix.rfind('[') {
        let partial_start = start + '['.len_utf8();
        let partial = &prefix[partial_start..];
        if is_valid_partial(partial) {
            return Some(TagTrigger {
                kind: TagSuggestionKind::Bracket,
                partial,
                start,
                end: cursor,
            });
        }
    }

    if let Some(start) = prefix.rfind('{') {
        let partial_start = start + '{'.len_utf8();
        let partial = &prefix[partial_start..];
        if is_valid_curly_partial(partial) {
            return Some(TagTrigger {
                kind: TagSuggestionKind::Curly,
                partial,
                start,
                end: cursor,
            });
        }
    }

    None
}

fn tag_matches_partial(tag: &str, partial: &str, kind: TagSuggestionKind) -> bool {
    if partial.is_empty() {
        return true;
    }
    match kind {
        TagSuggestionKind::Curly => tag.to_lowercase().starts_with(partial),
        TagSuggestionKind::Bracket
        | TagSuggestionKind::StyleOpen
        | TagSuggestionKind::StyleClose => tag.to_lowercase().contains(partial),
    }
}

fn is_valid_partial(partial: &str) -> bool {
    partial
        .chars()
        .all(|character| character.is_alphanumeric() || character == '-')
}

fn is_valid_curly_partial(partial: &str) -> bool {
    partial
        .chars()
        .all(|character| character.is_alphanumeric() || character == ':' || character == ' ')
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

impl TagTrigger<'_> {
    fn backspace_count(self) -> usize {
        self.partial.chars().count()
            + match self.kind {
                TagSuggestionKind::Bracket | TagSuggestionKind::StyleOpen => 1,
                TagSuggestionKind::StyleClose => 2,
                TagSuggestionKind::Curly => 1,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::{TagSuggestionKind, tag_suggestions_for_text};

    #[test]
    fn ignores_invalid_non_suffix_tag_partial() {
        assert!(tag_suggestions_for_text("hello [не word", "hello [не word".len()).is_empty());
    }

    #[test]
    fn uses_contains_matching_like_babel() {
        let suggestions = tag_suggestions_for_text("[раз", "[раз".len());

        assert!(suggestions.iter().any(|suggestion| {
            suggestion.kind == TagSuggestionKind::Bracket && suggestion.label == "неразборчиво"
        }));
    }
}
