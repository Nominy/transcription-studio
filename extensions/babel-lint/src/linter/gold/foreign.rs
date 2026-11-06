//! Babel `checkForeignTagConsistency`: a word left untagged where the same word is tagged foreign
//! elsewhere in the transcript.
//!
//! Babel splits words with `Intl.Segmenter`; this port uses Unicode word boundaries (UAX #29), so
//! scripts ICU segments by dictionary (Chinese, Japanese, Thai) can split differently.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use super::js::{WS, is_letter_mark_or_number, regex};
use super::{GoldAnnotation, GoldLanguage, Issues};

/// One word of an annotation (Babel `ed`).
struct Word<'a> {
    annotation_id: &'a str,
    /// The text as written.
    text: &'a str,
    /// Babel's comparison key: case-, accent- and digit-folded letters.
    key: String,
    tagged: bool,
    /// The word is the only word of its foreign tag.
    evidence: bool,
    language_code: Option<&'a str>,
}

/// The content of a foreign tag (Babel `el`), as byte offsets into the annotation text.
struct TaggedSpan<'a> {
    start: usize,
    end: usize,
    language_code: Option<&'a str>,
}

static LANGUAGE_MARKER: LazyLock<Regex> =
    LazyLock::new(|| regex(&format!(r"^{WS}*\{{([A-Z]{{2}})\}}")));
static NON_WORD_MARKUP: LazyLock<Regex> =
    LazyLock::new(|| regex(r"\[[^\[\]]*\]|［[^［］]*］|【[^【】]*】|\{\{[^{}]*\}\}|\{[^{}]*\}|\*"));
static STYLE_TAG: LazyLock<Regex> = LazyLock::new(|| regex("<[^>]*>"));

pub(super) fn foreign_tag_consistency(
    language: &GoldLanguage,
    annotations: &[GoldAnnotation<'_>],
    issues: &mut Issues<'_>,
) {
    let Some(foreign_tag) = &language.patterns().foreign_tag else {
        return;
    };
    let turkish = language.language == "TR";
    let mut tagged_keys: HashSet<String> = HashSet::new();
    let mut codes: HashMap<String, Vec<&str>> = HashMap::new();
    let mut untagged_index: HashMap<String, usize> = HashMap::new();
    let mut untagged: Vec<Vec<Word<'_>>> = Vec::new();
    for annotation in annotations
        .iter()
        .filter(|annotation| !annotation.text.is_empty())
    {
        for word in words(annotation, foreign_tag, turkish) {
            if word.evidence {
                tagged_keys.insert(word.key.clone());
                if let Some(code) = word.language_code {
                    let known = codes.entry(word.key.clone()).or_default();
                    if !known.contains(&code) {
                        known.push(code);
                    }
                }
            } else if !word.tagged {
                let index = *untagged_index.entry(word.key.clone()).or_insert_with(|| {
                    untagged.push(Vec::new());
                    untagged.len() - 1
                });
                untagged[index].push(word);
            }
        }
    }
    for occurrences in &untagged {
        let key = &occurrences[0].key;
        if !tagged_keys.contains(key) || codes.get(key).is_some_and(|codes| codes.len() > 1) {
            continue;
        }
        let mut reported: Vec<&str> = Vec::new();
        for occurrence in occurrences {
            if !reported.contains(&occurrence.annotation_id) {
                reported.push(occurrence.annotation_id);
                issues.push(
                    occurrence.annotation_id,
                    format!(
                        "\"{}\" is tagged as foreign elsewhere. Apply the tag here, fix the other occurrence, or acknowledge this warning.",
                        occurrence.text
                    ),
                );
            }
        }
    }
}

/// Babel `ed`: the annotation's word-like segments outside markup, with their foreign-tag state.
fn words<'a>(annotation: &GoldAnnotation<'a>, foreign_tag: &Regex, turkish: bool) -> Vec<Word<'a>> {
    let text = annotation.text;
    let spans = tagged_spans(text, foreign_tag);
    let same_length_spaces = |found: &regex::Captures<'_>| " ".repeat(found[0].len());
    let unmarked = NON_WORD_MARKUP.replace_all(text, same_length_spaces);
    let unmarked = STYLE_TAG.replace_all(&unmarked, same_length_spaces);
    let segments: Vec<(usize, &str)> = unmarked
        .split_word_bound_indices()
        .filter(|(_, segment)| segment.chars().any(char::is_alphanumeric))
        .collect();
    let span_of = |start: usize, end: usize| {
        spans
            .iter()
            .position(|span| start >= span.start && end <= span.end)
    };
    let mut words_per_span = vec![0usize; spans.len()];
    for &(start, segment) in &segments {
        if let Some(span) = span_of(start, start + segment.len()) {
            words_per_span[span] += 1;
        }
    }
    segments
        .iter()
        .filter_map(|&(start, segment)| {
            let key = word_key(segment, turkish);
            if key.is_empty() {
                return None;
            }
            let end = start + segment.len();
            let span = span_of(start, end);
            Some(Word {
                annotation_id: annotation.id,
                text: &text[start..end],
                key,
                tagged: span.is_some(),
                evidence: span.is_some_and(|span| words_per_span[span] == 1),
                language_code: span.and_then(|span| spans[span].language_code),
            })
        })
        .collect()
}

/// Babel `el`: each opening foreign tag's content up to the next closing tag, with the language code
/// from the tag's `="XX"` attribute or a leading `{XX}` marker.
fn tagged_spans<'a>(text: &'a str, foreign_tag: &Regex) -> Vec<TaggedSpan<'a>> {
    let mut spans = Vec::new();
    let mut open: Option<(usize, Option<&'a str>)> = None;
    for captures in foreign_tag.captures_iter(text) {
        let Some(tag) = captures.get(0) else { continue };
        if captures.get(1).is_some_and(|slash| !slash.is_empty()) {
            let Some((start, attribute)) = open.take() else {
                continue;
            };
            let marker = || {
                LANGUAGE_MARKER
                    .captures(&text[start..tag.start()])
                    .and_then(|marker| marker.get(1))
            };
            spans.push(TaggedSpan {
                start,
                end: tag.start(),
                language_code: attribute.or_else(|| marker().map(|code| code.as_str())),
            });
        } else if open.is_none() {
            let attribute = captures
                .get(2)
                .map(|code| code.as_str())
                .filter(|code| !code.is_empty());
            open = Some((tag.end(), attribute));
        }
    }
    spans
}

/// Babel's word key: NFKC, locale lower case, accents (U+0300–U+036F) and decimal digits removed,
/// then everything but letters, marks and numbers.
fn word_key(segment: &str, turkish: bool) -> String {
    let compatible: String = segment.nfkc().collect();
    let lower = if turkish {
        compatible
            .replace('I', "\u{131}")
            .replace('\u{130}', "i")
            .to_lowercase()
    } else {
        compatible.to_lowercase()
    };
    let unaccented: String = lower
        .nfd()
        .filter(|character| !('\u{300}'..='\u{36F}').contains(character))
        .nfc()
        .collect();
    unaccented
        .chars()
        .filter(|&character| is_letter_mark_or_number(character) && !is_decimal_digit(character))
        .collect()
}

fn is_decimal_digit(character: char) -> bool {
    super::js::has_decimal_digit(character.encode_utf8(&mut [0; 4]))
}
