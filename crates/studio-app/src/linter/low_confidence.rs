use serde_json::Value;
use studio_core::Segment;

const LOW_CONFIDENCE_WORDS_METADATA: &str = "babelLowConfidenceWords";
const LOW_CONFIDENCE_RESOLVED_METADATA: &str = "lowConfidenceResolved";

pub(super) const LOW_CONFIDENCE_REASON: &str =
    "This segment has unresolved low-confidence words. Please review and acknowledge or edit.";

pub(super) fn has_unresolved_low_confidence_word(segment: &Segment) -> bool {
    if segment
        .metadata
        .get(LOW_CONFIDENCE_RESOLVED_METADATA)
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }

    let Some(words) = segment
        .metadata
        .get(LOW_CONFIDENCE_WORDS_METADATA)
        .and_then(Value::as_array)
    else {
        return false;
    };

    let content_words = normalized_content_words(&segment.text);
    words.iter().any(|word| {
        is_highlighted_low_confidence_word(word)
            && candidate_word_is_inside_segment(segment, word)
            && low_confidence_word_text(word)
                .map(normalize_word)
                .filter(|word| !word.is_empty())
                .is_some_and(|word| {
                    content_words
                        .iter()
                        .any(|content_word| content_word == &word)
                })
    })
}

fn normalized_content_words(content: &str) -> Vec<String> {
    content
        .split_whitespace()
        .map(normalize_word)
        .filter(|word| !word.is_empty())
        .collect()
}

fn is_highlighted_low_confidence_word(word: &Value) -> bool {
    word.get("isYellow")
        .or_else(|| word.get("shouldHighlight"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn low_confidence_word_text(word: &Value) -> Option<&str> {
    word.get("finalWord")
        .or_else(|| word.get("word"))
        .or_else(|| word.get("sourceWord"))
        .and_then(Value::as_str)
}

fn candidate_word_is_inside_segment(segment: &Segment, word: &Value) -> bool {
    let Some(start) = word.get("start").and_then(Value::as_f64) else {
        return false;
    };
    let Some(end) = word.get("end").and_then(Value::as_f64) else {
        return false;
    };
    start >= segment.start_seconds - 0.1 && end <= segment.end_seconds + 0.1
}

fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '\'' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}
