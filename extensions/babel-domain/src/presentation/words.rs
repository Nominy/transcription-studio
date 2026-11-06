use std::collections::BTreeSet;

use serde_json::Value;
use studio_core::Segment;

use super::TranscriptWordView;

const LOW_CONFIDENCE_WORDS_METADATA: &str = "babelLowConfidenceWords";
const LOW_CONFIDENCE_RESOLVED_METADATA: &str = "lowConfidenceResolved";

pub(crate) fn build_transcript_word_views(segment: &Segment) -> Vec<TranscriptWordView> {
    if segment
        .metadata
        .get(LOW_CONFIDENCE_RESOLVED_METADATA)
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Vec::new();
    }

    let Some(words) = segment
        .metadata
        .get(LOW_CONFIDENCE_WORDS_METADATA)
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let content_words = normalized_content_words(&segment.text);
    let display_words = display_words_by_global_index(segment, words, &content_words);
    let mut views = words
        .iter()
        .filter_map(|word| {
            if !is_highlighted_low_confidence_word(word)
                || !candidate_word_is_inside_segment(segment, word)
            {
                return None;
            }
            let text = candidate_low_confidence_word_text(word)?.to_string();
            let normalized = normalize_word(&text);
            if normalized.is_empty() || !content_words.contains(&normalized) {
                return None;
            }
            let global_index = word.get("index").and_then(Value::as_u64)? as usize;
            let display_word = display_words.get(&global_index)?;

            Some(TranscriptWordView {
                index: display_word.local_index,
                text: display_word.text.clone(),
                start_seconds: display_word.start_seconds,
                end_seconds: display_word.end_seconds,
                is_low_confidence: true,
                is_acknowledged: false,
                command: Some("babel.acknowledge-low-confidence-word".into()),
                command_arguments: serde_json::json!({ "word_index": display_word.local_index }),
            })
        })
        .collect::<Vec<_>>();

    views.sort_by(|left, right| {
        left.start_seconds
            .unwrap_or(f64::INFINITY)
            .total_cmp(&right.start_seconds.unwrap_or(f64::INFINITY))
            .then_with(|| left.index.cmp(&right.index))
    });
    views
}

fn normalized_content_words(content: &str) -> BTreeSet<String> {
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

fn candidate_low_confidence_word_text(word: &Value) -> Option<&str> {
    word.get("finalWord")
        .or_else(|| word.get("word"))
        .or_else(|| word.get("sourceWord"))
        .and_then(Value::as_str)
}

fn display_low_confidence_word_text(word: &Value) -> Option<&str> {
    word.get("finalWord").and_then(Value::as_str)
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

fn display_word_is_inside_segment(segment: &Segment, word: &Value) -> bool {
    let Some(start) = word.get("start").and_then(Value::as_f64) else {
        return false;
    };
    let Some(end) = word.get("end").and_then(Value::as_f64) else {
        return false;
    };
    start >= segment.start_seconds - 0.05 && end <= segment.end_seconds + 0.05
}

fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '\'' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Clone, Debug)]
struct DisplayLowConfidenceWord {
    local_index: usize,
    text: String,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
}

fn display_words_by_global_index(
    segment: &Segment,
    words: &[Value],
    content_words: &BTreeSet<String>,
) -> std::collections::BTreeMap<usize, DisplayLowConfidenceWord> {
    let mut display_words = words
        .iter()
        .filter_map(|word| {
            let text = display_low_confidence_word_text(word)?;
            if !display_word_is_inside_segment(segment, word) {
                return None;
            }
            let normalized = normalize_word(text);
            if normalized.is_empty() || !content_words.contains(&normalized) {
                return None;
            }
            Some((
                word.get("index").and_then(Value::as_u64)? as usize,
                DisplayLowConfidenceWord {
                    local_index: 0,
                    text: text.to_string(),
                    start_seconds: word.get("start").and_then(Value::as_f64),
                    end_seconds: word.get("end").and_then(Value::as_f64),
                },
            ))
        })
        .collect::<Vec<_>>();

    display_words.sort_by(|(_, left), (_, right)| {
        left.start_seconds
            .unwrap_or(f64::INFINITY)
            .total_cmp(&right.start_seconds.unwrap_or(f64::INFINITY))
    });

    display_words
        .into_iter()
        .enumerate()
        .map(|(local_index, (global_index, mut word))| {
            word.local_index = local_index;
            (global_index, word)
        })
        .collect()
}
