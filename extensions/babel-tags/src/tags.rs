//! Tag autocompletion for transcript text: trigger detection before the caret, project-aware
//! tag matching, and replacement at the caret (or wrapping a selection in a style tag).

use exaudio_babel_lint::project_lint_tag_config::BabelProjectLintTagConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagSuggestionKind {
    /// `[tag]`
    Bracket,
    /// `<tag></tag>`
    StyleOpen,
    /// `</tag>`
    StyleClose,
    /// `{tag}`
    Curly,
}

/// One completion candidate. Byte offsets refer to the draft text it was computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSuggestion {
    pub label: String,
    pub insert_text: String,
    pub kind: TagSuggestionKind,
    pub replacement_start: usize,
    pub replacement_end: usize,
    /// Chars after the inserted text the caret moves back (e.g. inside `<tag>|</tag>`).
    pub cursor_back_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TagTrigger<'a> {
    kind: TagSuggestionKind,
    partial: &'a str,
    start: usize,
    end: usize,
}

/// Completions for the trigger ending at byte offset `cursor` in `text`.
pub fn suggestions_for_text(
    text: &str,
    cursor: usize,
    config: &BabelProjectLintTagConfig,
) -> Vec<TagSuggestion> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return Vec::new();
    }
    let Some(trigger) = tag_trigger_before_cursor(text, cursor) else {
        return Vec::new();
    };

    let partial = trigger.partial.to_lowercase();
    let tags: Vec<&str> = match trigger.kind {
        TagSuggestionKind::Bracket => config.bracket_tags().to_vec(),
        TagSuggestionKind::StyleOpen | TagSuggestionKind::StyleClose => {
            config.style_tags().to_vec()
        }
        TagSuggestionKind::Curly => config.curly_tags().iter().map(String::as_str).collect(),
    };
    tags.into_iter()
        .filter(|tag| tag_matches_partial(tag, &partial, trigger.kind))
        .map(|tag| suggestion_for_tag(tag, trigger))
        .collect()
}

/// Completion requests use byte ranges so the rule package never owns editor drafts.
pub fn completions_for_request(
    request: &exaudio_extension::document::CompletionRequest<'_>,
) -> Vec<exaudio_extension::document::Completion> {
    use exaudio_extension::document::{Completion, TextRange};
    let config = exaudio_babel_lint::project_lint_tag_config::resolve_babel_project_lint_tag_config(
        request.project,
    );
    let range = request.selection;
    let Some(selected) = request.text.get(range.start..range.end) else {
        return Vec::new();
    };
    if range.start != range.end {
        if selected.trim().is_empty() {
            return Vec::new();
        }
        return config
            .style_tags()
            .iter()
            .map(|tag| {
                let insert_text = format!("<{tag}>{selected}</{tag}>");
                let caret_offset = Some(range.start + insert_text.len());
                Completion {
                    source: "babel.tags".into(),
                    label: format!("<{tag}>"),
                    insert_text,
                    range,
                    documentation: None,
                    caret_offset,
                }
            })
            .collect();
    }
    suggestions_for_text(request.text, range.end, &config)
        .into_iter()
        .map(|suggestion| {
            let suffix_bytes: usize = suggestion
                .insert_text
                .chars()
                .rev()
                .take(suggestion.cursor_back_count)
                .map(char::len_utf8)
                .sum();
            let caret_offset =
                Some(suggestion.replacement_start + suggestion.insert_text.len() - suffix_bytes);
            Completion {
                source: "babel.tags".into(),
                label: suggestion.label,
                insert_text: suggestion.insert_text,
                range: TextRange {
                    start: suggestion.replacement_start,
                    end: suggestion.replacement_end,
                },
                documentation: None,
                caret_offset,
            }
        })
        .collect()
}

fn suggestion_for_tag(tag: &str, trigger: TagTrigger<'_>) -> TagSuggestion {
    let (label, insert_text, cursor_back_count) = match trigger.kind {
        TagSuggestionKind::Bracket => (tag.to_string(), format!("[{tag}]"), 0),
        TagSuggestionKind::StyleOpen => (
            format!("<{tag}>"),
            format!("<{tag}></{tag}>"),
            tag.chars().count() + 3,
        ),
        TagSuggestionKind::StyleClose => {
            let insert_text = format!("</{tag}>");
            (insert_text.clone(), insert_text, 0)
        }
        TagSuggestionKind::Curly => (tag.trim().to_string(), format!("{{{tag}}}"), 1),
    };
    TagSuggestion {
        label,
        insert_text,
        kind: trigger.kind,
        replacement_start: trigger.start,
        replacement_end: trigger.end,
        cursor_back_count,
    }
}

fn tag_trigger_before_cursor(text: &str, cursor: usize) -> Option<TagTrigger<'_>> {
    let prefix = &text[..cursor];
    let trigger = |start: usize, open_len: usize, kind| TagTrigger {
        kind,
        partial: &prefix[start + open_len..],
        start,
        end: cursor,
    };

    if let Some(start) = prefix.rfind("</") {
        let candidate = trigger(start, 2, TagSuggestionKind::StyleClose);
        if is_valid_partial(candidate.partial) {
            return Some(candidate);
        }
    }
    if let Some(start) = prefix.rfind('<') {
        let candidate = trigger(start, 1, TagSuggestionKind::StyleOpen);
        if !candidate.partial.starts_with('/') && is_valid_partial(candidate.partial) {
            return Some(candidate);
        }
    }
    if let Some(start) = prefix.rfind('[') {
        let candidate = trigger(start, 1, TagSuggestionKind::Bracket);
        if is_valid_partial(candidate.partial) {
            return Some(candidate);
        }
    }
    if let Some(start) = prefix.rfind('{') {
        let candidate = trigger(start, 1, TagSuggestionKind::Curly);
        if is_valid_curly_partial(candidate.partial) {
            return Some(candidate);
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
    partial.chars().all(|c| c.is_alphanumeric() || c == '-')
}

fn is_valid_curly_partial(partial: &str) -> bool {
    partial
        .chars()
        .all(|c| c.is_alphanumeric() || c == ':' || c == ' ')
}
