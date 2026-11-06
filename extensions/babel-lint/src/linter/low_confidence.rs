//! Input of Gold's `unresolved-low-confidence` rule. Babel matches the chunk consensus's
//! highlighted words against each segment (module 72857 `F`); Studio has those words as the
//! imported `babelLowConfidenceWords` segment metadata.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use studio_core::Segment;

const LOW_CONFIDENCE_WORDS_METADATA: &str = "babelLowConfidenceWords";
const LOW_CONFIDENCE_RESOLVED_METADATA: &str = "lowConfidenceResolved";
/// Babel's tolerance around the segment for a word's timing.
const WORD_TIME_TOLERANCE_SECONDS: f64 = 0.1;

static NON_WORD_CHARACTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\p{L}\p{M}\p{N}'-]").expect("valid regex"));

/// Code point ranges in which a low-confidence word may match inside a longer token (scripts
/// written without spaces).
const UNSPACED_SCRIPT_RANGES: &[(u32, u32)] = &[
    (3584, 3839),
    (4096, 4255),
    (6016, 6143),
    (12352, 12543),
    (13312, 40959),
    (63744, 64255),
    (65382, 65439),
    (131072, 262143),
];

pub fn has_unresolved_low_confidence_word(segment: &Segment) -> bool {
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

    let content_words = segment
        .text
        .split_whitespace()
        .map(normalize_word)
        .collect::<Vec<_>>();
    words.iter().any(|word| {
        is_highlighted_low_confidence_word(word)
            && candidate_word_is_inside_segment(segment, word)
            && low_confidence_word_text(word)
                .map(normalize_word)
                .is_some_and(|word| {
                    content_words
                        .iter()
                        .any(|content_word| word_matches(content_word, &word))
                })
    })
}

fn word_matches(content_word: &str, word: &str) -> bool {
    content_word == word || (word.chars().any(is_unspaced_script) && content_word.contains(word))
}

fn is_unspaced_script(character: char) -> bool {
    let code = u32::from(character);
    UNSPACED_SCRIPT_RANGES
        .iter()
        .any(|(start, end)| (*start..=*end).contains(&code))
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
    start >= segment.start_seconds - WORD_TIME_TOLERANCE_SECONDS
        && end <= segment.end_seconds + WORD_TIME_TOLERANCE_SECONDS
}

fn normalize_word(word: &str) -> String {
    NON_WORD_CHARACTER
        .replace_all(&word.to_lowercase(), "")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use studio_core::Segment;

    use super::has_unresolved_low_confidence_word;

    #[test]
    fn highlighted_cyrillic_word_in_the_text_is_unresolved_until_resolved() {
        let mut segment = segment_with_words(
            "Ну, Привет!",
            json!([{ "word": "привет", "start": 1.2, "end": 1.6, "isYellow": true }]),
        );
        assert!(has_unresolved_low_confidence_word(&segment));

        segment
            .metadata
            .insert("lowConfidenceResolved".to_string(), json!(true));
        assert!(!has_unresolved_low_confidence_word(&segment));
    }

    #[test]
    fn edited_out_or_unhighlighted_words_are_not_unresolved() {
        let edited = segment_with_words(
            "Ну, здравствуй.",
            json!([{ "word": "привет", "start": 1.2, "end": 1.6, "isYellow": true }]),
        );
        let plain = segment_with_words(
            "Ну, привет.",
            json!([{ "word": "привет", "start": 1.2, "end": 1.6, "isYellow": false }]),
        );
        let outside = segment_with_words(
            "Ну, привет.",
            json!([{ "word": "привет", "start": 3.5, "end": 3.9, "isYellow": true }]),
        );

        assert!(!has_unresolved_low_confidence_word(&edited));
        assert!(!has_unresolved_low_confidence_word(&plain));
        assert!(!has_unresolved_low_confidence_word(&outside));
    }

    #[test]
    fn unspaced_script_word_matches_inside_a_longer_token() {
        let segment = segment_with_words(
            "我们今天去北京。",
            json!([{ "word": "北京", "start": 1.2, "end": 1.6, "shouldHighlight": true }]),
        );

        assert!(has_unresolved_low_confidence_word(&segment));
    }

    fn segment_with_words(text: &str, words: serde_json::Value) -> Segment {
        Segment {
            id: "segment".to_string(),
            source_annotation_id: None,
            speaker_id: "speaker".to_string(),
            lane_id: "lane".to_string(),
            start_seconds: 1.0,
            end_seconds: 3.0,
            text: text.to_string(),
            metadata: BTreeMap::from([("babelLowConfidenceWords".to_string(), words)]),
        }
    }
}
