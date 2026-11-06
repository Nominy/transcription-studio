//! Initial fill decisions: the OpenRouter `POST /api/alpha/decisions` request for
//! `cloudflare/clef-omni` and its strict answer parser, exactly as Gold's error-task broker
//! builds them (six choice questions, `provider.allow_fallbacks: false`).

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::Value;

use super::choices::{CATEGORY_GROUPS, ErrorLabelDraft, category_keys};
use super::diff_align::{DiffSide, js_space, js_trim, utf16_len};
use super::{ErrorTaskError, StyleGuide};

pub const CLEF_MODEL: &str = "cloudflare/clef-omni";
const DECISIONS_URL: &str = "https://openrouter.ai/api/alpha/decisions";
/// Points Initial fill at a decisions server on this machine (smoke runs). Honored only for
/// `http://localhost` / `http://127.0.0.1` URLs, so the Gold key never goes anywhere else.
pub const DECISIONS_URL_ENV: &str = "STUDIO_OPENROUTER_DECISIONS_URL";

/// The decisions endpoint: `STUDIO_OPENROUTER_DECISIONS_URL` when it is a loopback URL, else
/// OpenRouter's.
pub fn decisions_url() -> String {
    decisions_url_with(std::env::var(DECISIONS_URL_ENV).ok().as_deref())
}

/// [`decisions_url`] for an explicit override value.
pub fn decisions_url_with(override_url: Option<&str>) -> String {
    override_url
        .map(str::trim)
        .filter(|url| {
            url::Url::parse(url).is_ok_and(|url| {
                url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            })
        })
        .unwrap_or(DECISIONS_URL)
        .to_string()
}

/// Cache schema of paid decision results.
pub(crate) const SCHEMA_VERSION: &str = "error-task-choices-v1";

/// JSON with insertion-ordered objects (the prompt reads in this order).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum J {
    Str(String),
    Num(usize),
    Bool(bool),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl Serialize for J {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Str(text) => serializer.serialize_str(text),
            Self::Num(number) => serializer.serialize_u64(*number as u64),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Arr(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Obj(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

fn s(text: impl Into<String>) -> J {
    J::Str(text.into())
}

fn obj<const N: usize>(entries: [(&str, J); N]) -> J {
    J::Obj(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

/// `{what, …extra}` criteria objects.
fn criterion(what: &str, extra: Vec<(&str, J)>) -> J {
    let mut entries = vec![("what".to_string(), s(what))];
    entries.extend(
        extra
            .into_iter()
            .map(|(key, value)| (key.to_string(), value)),
    );
    J::Obj(entries)
}

fn examples(items: &[&str]) -> J {
    J::Arr(items.iter().map(|item| s(*item)).collect())
}

/// Gold's `CATEGORY_CRITERIA`, in declaration order.
const CATEGORY_CRITERIA: [(&str, &str); 34] = [
    (
        "hard_to_hear",
        "Audio is too unclear to determine what was said.",
    ),
    (
        "false_starts",
        "An abandoned partial word or utterance is omitted or completed differently. Shared false starts are not the difference.",
    ),
    (
        "repetition",
        "The number of repeated whole words differs between transcripts. Merely having repetitions on both sides does not qualify.",
    ),
    (
        "self_corrections",
        "A spoken correction is transcribed differently.",
    ),
    (
        "filler_words",
        "A filler, hesitation or short listener reaction is omitted or changed.",
    ),
    (
        "script",
        "The same word is represented in different writing systems.",
    ),
    (
        "diacritics",
        "Only diacritics, stress marks or tone marks differ.",
    ),
    (
        "compound_words",
        "Only joining, spaces or hyphens differ; no letters or speech sounds are added or removed.",
    ),
    (
        "contractions",
        "A contracted, expanded or colloquial form of the same word differs.",
    ),
    (
        "abbrev_acronym",
        "An abbreviation or acronym differs from its expanded form.",
    ),
    (
        "spelled_out",
        "A word spoken letter by letter is represented differently.",
    ),
    ("punctuation", "Only punctuation or capitalization differs."),
    (
        "spelling",
        "The same pronounced word is spelled differently, without adding or removing a speech sound.",
    ),
    ("dates", "A calendar date is represented differently."),
    ("times", "A clock time is represented differently."),
    ("currency", "A currency symbol or monetary unit differs."),
    (
        "measurements",
        "A measurement unit or its representation differs.",
    ),
    (
        "phone_numbers",
        "A phone number is grouped or represented differently.",
    ),
    (
        "addresses",
        "A postal, email or web address is represented differently.",
    ),
    ("alphanumerics", "An alphanumeric identifier differs."),
    ("numbers", "A number or numerical value differs."),
    (
        "word_added_or_dropped",
        "A whole word is present only on one side, not a changed part of a word.",
    ),
    (
        "phrase_added_or_dropped",
        "A multiword phrase is present only on one side.",
    ),
    (
        "named_entities",
        "A proper name or named entity is recognized as a different name or word.",
    ),
    (
        "code_switching",
        "Speech switches languages and the words or language are transcribed incorrectly.",
    ),
    (
        "homophones",
        "A different lexical word with exactly the same pronunciation.",
    ),
    ("different_tone", "A lexical tone differs."),
    (
        "affix",
        "A word prefix, suffix, infix or ending is added, removed or changed; the rest of the word remains. Includes grammatical endings.",
    ),
    (
        "near_homophones",
        "A different lexical word with similar pronunciation, rather than just a changed affix.",
    ),
    (
        "possessives",
        "The possessive construction itself differs, not ordinary gender or number endings.",
    ),
    (
        "different_phrase",
        "Two or more words are replaced by different words; no earlier category describes the change.",
    ),
    (
        "different_word",
        "A different, phonetically dissimilar lexical word; no earlier category describes the change.",
    ),
    (
        "not_sure",
        "Audio is clear but none of the other categories describes the difference.",
    ),
    (
        "typo",
        "An apparent typographical error not covered by an earlier category.",
    ),
];

/// Categories a secondary answer may use (not Missing/extra or Wrong words, except affix).
const NOT_SECONDARY: [&str; 10] = [
    "word_added_or_dropped",
    "phrase_added_or_dropped",
    "named_entities",
    "code_switching",
    "homophones",
    "different_tone",
    "near_homophones",
    "possessives",
    "different_phrase",
    "different_word",
];

/// The question keys, in order.
pub(crate) const QUESTION_KEYS: [&str; 6] = [
    "errorOutsideHighlight",
    "onlyNonSemanticDifference",
    "humanTranscriptWrong",
    "primaryCategory",
    "hasSecondaryCategory",
    "secondaryCategory",
];

/// Allowed choices per question (the keys of each question's `criteria`).
pub(crate) fn question_choices(key: &str) -> Vec<&'static str> {
    match key {
        "errorOutsideHighlight"
        | "onlyNonSemanticDifference"
        | "humanTranscriptWrong"
        | "hasSecondaryCategory" => vec!["yes", "no"],
        "primaryCategory" => CATEGORY_CRITERIA.iter().map(|(key, _)| *key).collect(),
        "secondaryCategory" => std::iter::once("none")
            .chain(category_keys().filter(|key| !NOT_SECONDARY.contains(key)))
            .collect(),
        _ => Vec::new(),
    }
}

fn questions() -> J {
    let primary = J::Obj(
        CATEGORY_CRITERIA
            .iter()
            .map(|&(key, text)| {
                let value = match key {
                    "affix" => criterion(
                        "A prefix, suffix, infix or word ending is added, removed or changed while the rest of the word remains.",
                        vec![
                            ("examples", examples(&["несправедливый versus справедливый", "животные versus животное"])),
                            ("not_for", s("Only spaces/hyphens change with no letters added: compound_words. An abandoned word is completed: false_starts.")),
                        ],
                    ),
                    "compound_words" => criterion(
                        "Only spaces or hyphens differ; both sides contain exactly the same letters.",
                        vec![("not_for", s("A letter or sound is added or removed from a word ending."))],
                    ),
                    "spelling" => criterion(
                        "Two spellings of the same word with exactly the same pronunciation.",
                        vec![("not_for", s("Adding or removing a speech sound or a grammatical ending."))],
                    ),
                    _ => s(text),
                };
                (key.to_string(), value)
            })
            .collect(),
    );
    let criteria = CATEGORY_CRITERIA;
    let secondary = J::Obj(
        std::iter::once((
            "none".to_string(),
            s("One category already describes the highlighted difference; no separate second type is present."),
        ))
        .chain(
            category_keys()
                .filter(|key| !NOT_SECONDARY.contains(key))
                .map(|key| {
                    let text = criteria
                        .iter()
                        .find(|(known, _)| *known == key)
                        .map_or("", |(_, text)| *text);
                    (key.to_string(), s(text))
                }),
        )
        .collect(),
    );
    obj([
        (
            "errorOutsideHighlight",
            obj([
                ("type", s("choice")),
                (
                    "instructions",
                    s(
                        "Is any part of the selected error outside the highlighted strings? Inspect only `unhighlightedSelectionEdges` and `selectedComparison`, not independent errors elsewhere.",
                    ),
                ),
                (
                    "criteria",
                    obj([
                        (
                            "yes",
                            obj([
                                (
                                    "what",
                                    s(
                                        "This same changed word or directly connected omission/insertion continues into unhighlighted neighboring text.",
                                    ),
                                ),
                                (
                                    "example",
                                    s(
                                        "The replacement is highlighted but a missing adjacent word belonging to that same phrase is outside the selection.",
                                    ),
                                ),
                            ]),
                        ),
                        (
                            "no",
                            obj([
                                (
                                    "what",
                                    s(
                                        "All of this particular discrepancy is inside the selection. Adjacent unchanged text needs no highlighting.",
                                    ),
                                ),
                                (
                                    "example",
                                    s(
                                        "An entire differing ending is highlighted; the identical rest of the word and its unchanged neighboring words need not be highlighted.",
                                    ),
                                ),
                            ]),
                        ),
                    ]),
                ),
            ]),
        ),
        (
            "onlyNonSemanticDifference",
            obj([
                ("type", s("choice")),
                (
                    "instructions",
                    s(
                        "Is the selected difference ONLY a non-semantic representation change, filler or speech disfluency, preserving the same lexical and grammatical meaning?",
                    ),
                ),
                (
                    "criteria",
                    obj([
                        (
                            "yes",
                            obj([
                                (
                                    "what",
                                    s(
                                        "Same lexical/grammatical meaning; only writing format, punctuation, case, an equivalent number representation, a filler or a pronunciation variant differs.",
                                    ),
                                ),
                                (
                                    "examples",
                                    examples(&[
                                        "24 versus twenty-four",
                                        "окей versus окэй",
                                        "э versus an omitted hesitation",
                                    ]),
                                ),
                            ]),
                        ),
                        (
                            "no",
                            obj([
                                (
                                    "what",
                                    s(
                                        "A lexical word or its grammatical meaning changes; this is not merely another spelling of the same pronounced word.",
                                    ),
                                ),
                                (
                                    "examples",
                                    examples(&[
                                        "сестра versus брат",
                                        "несправедливо versus справедливо",
                                    ]),
                                ),
                            ]),
                        ),
                    ]),
                ),
            ]),
        ),
        (
            "humanTranscriptWrong",
            obj([
                ("type", s("choice")),
                (
                    "instructions",
                    s(
                        "Does `selectedComparison.human` mistranscribe the target-speaker audio? Ignore independent issues outside this selection.",
                    ),
                ),
                (
                    "criteria",
                    obj([
                        ("yes", s("The Human transcript does not match the speech.")),
                        (
                            "no",
                            s(
                                "The Human transcript faithfully represents the speech, including disfluencies and word fragments; punctuation is normalized.",
                            ),
                        ),
                    ]),
                ),
            ]),
        ),
        (
            "primaryCategory",
            obj([
                ("type", s("choice")),
                (
                    "instructions",
                    s(
                        "Which category describes the change from `selectedComparison.human` to `selectedComparison.machine`? Select the first applicable category in native priority order. Classify what differs, not features shared by both strings.",
                    ),
                ),
                ("criteria", primary),
            ]),
        ),
        (
            "hasSecondaryCategory",
            obj([
                ("type", s("choice")),
                (
                    "instructions",
                    s(
                        "Does the highlighted comparison contain TWO DISTINCT TYPES of error, rather than repeated instances of one type?",
                    ),
                ),
                (
                    "criteria",
                    obj([
                        (
                            "yes",
                            s(
                                "Two independent, different error types are needed to describe the highlighted difference.",
                            ),
                        ),
                        (
                            "no",
                            s(
                                "One error type explains it, even if that same change occurs more than once in the selection.",
                            ),
                        ),
                    ]),
                ),
            ]),
        ),
        (
            "secondaryCategory",
            obj([
                ("type", s("choice")),
                (
                    "instructions",
                    s(
                        "Usually none. Select a second category only for a separate additional TYPE of error inside the highlight. Never repeat the primary category. hard_to_hear stands alone. If the primary is in the first four groups, the secondary must also be in those groups. A secondary cannot be Missing/extra or Wrong words except affix.",
                    ),
                ),
                ("criteria", secondary),
            ]),
        ),
    ])
}

/// `trim().split(/\s+/u)` without empty parts.
fn split_words(text: &str) -> Vec<&str> {
    js_trim(text)
        .split(js_space)
        .filter(|part| !part.is_empty())
        .collect()
}

fn last_word(text: &str) -> String {
    split_words(text).last().copied().unwrap_or("").to_string()
}

fn first_word(text: &str) -> String {
    split_words(text).first().copied().unwrap_or("").to_string()
}

/// `/^\S*/u`
fn leading_non_space(text: &str) -> String {
    text.chars().take_while(|c| !js_space(*c)).collect()
}

/// `/\S*$/u`
fn trailing_non_space(text: &str) -> String {
    let tail: Vec<char> = text.chars().rev().take_while(|c| !js_space(*c)).collect();
    tail.into_iter().rev().collect()
}

/// `observedTextFeatures`: token counts, and per-position common prefix/suffix when the
/// highlighted strings have the same number of words.
fn text_edit_features(human: &str, machine: &str) -> J {
    let words = |text: &str| -> Vec<String> {
        if js_trim(text).is_empty() {
            Vec::new()
        } else {
            js_trim(text)
                .split(js_space)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect()
        }
    };
    let (a, b) = (words(human), words(machine));
    let mut entries = vec![
        ("humanTokenCount".to_string(), J::Num(a.len())),
        ("machineTokenCount".to_string(), J::Num(b.len())),
    ];
    if a.len() == b.len() {
        let comparisons = a
            .iter()
            .zip(&b)
            .map(|(word, other)| {
                let left: Vec<char> = word.chars().collect();
                let right: Vec<char> = other.chars().collect();
                let mut start = 0;
                while start < left.len() && start < right.len() && left[start] == right[start] {
                    start += 1;
                }
                let mut end = 0;
                while end < left.len() - start
                    && end < right.len() - start
                    && left[left.len() - 1 - end] == right[right.len() - 1 - end]
                {
                    end += 1;
                }
                let text = |chars: &[char]| chars.iter().collect::<String>();
                obj([
                    ("human", s(word.clone())),
                    ("machine", s(other.clone())),
                    ("commonBeginning", s(text(&left[..start]))),
                    (
                        "humanChangedCharacters",
                        s(text(&left[start..left.len() - end])),
                    ),
                    (
                        "machineChangedCharacters",
                        s(text(&right[start..right.len() - end])),
                    ),
                    (
                        "commonEnding",
                        s(if end > 0 {
                            text(&left[left.len() - end..])
                        } else {
                            String::new()
                        }),
                    ),
                ])
            })
            .collect();
        entries.push((
            "positionalTokenComparisons".to_string(),
            J::Arr(comparisons),
        ));
    }
    J::Obj(entries)
}

/// What the decision model sees: the aligned, cropped comparison (UTF-16 spans), the ASR of
/// the aligned window and the project's complete style guide.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionComparison {
    pub language: String,
    pub human: DiffSide,
    pub machine: DiffSide,
    pub asr_text: String,
    pub style_guide: StyleGuide,
}

/// The decisions request body (`buildErrorTaskDraftBody`) with the clip as base64 WAV.
pub(crate) fn decision_body(
    comparison: &DecisionComparison,
    wav_base64: &str,
) -> Result<J, ErrorTaskError> {
    if js_trim(&comparison.asr_text).is_empty() || utf16_len(&comparison.asr_text) > 16_000 {
        return Err(ErrorTaskError::new(
            "alignment-required",
            "Initial fill requires the ASR transcript of the aligned audio window.",
        ));
    }
    let (human_before, human_highlighted, human_after) = comparison.human.parts();
    let (machine_before, machine_highlighted, machine_after) = comparison.machine.parts();
    let groups = J::Arr(
        CATEGORY_GROUPS
            .iter()
            .map(|group| J::Arr(group.categories.iter().map(|key| s(*key)).collect()))
            .collect(),
    );
    let context = obj([
        (
            "task",
            s(
                "Classify the difference in selectedComparison. surroundingContext is not another comparison. asrTranscript is auxiliary evidence from the attached audio, not ground truth.",
            ),
        ),
        ("language", s(comparison.language.clone())),
        (
            "selectedComparison",
            obj([
                ("human", s(human_highlighted.clone())),
                ("machine", s(machine_highlighted.clone())),
            ]),
        ),
        (
            "surroundingContext",
            obj([
                ("before", s(human_before.clone())),
                ("after", s(human_after.clone())),
            ]),
        ),
        ("asrTranscript", s(comparison.asr_text.clone())),
        (
            "unhighlightedSelectionEdges",
            obj([
                (
                    "before",
                    obj([
                        ("human", s(last_word(&human_before))),
                        ("machine", s(last_word(&machine_before))),
                    ]),
                ),
                (
                    "after",
                    obj([
                        ("human", s(first_word(&human_after))),
                        ("machine", s(first_word(&machine_after))),
                    ]),
                ),
                (
                    "remainingWordTails",
                    obj([
                        ("human", s(leading_non_space(&human_after))),
                        ("machine", s(leading_non_space(&machine_after))),
                    ]),
                ),
                (
                    "precedingWordFragments",
                    obj([
                        ("human", s(trailing_non_space(&human_before))),
                        ("machine", s(trailing_non_space(&machine_before))),
                    ]),
                ),
            ]),
        ),
        (
            "observedTextFeatures",
            text_edit_features(&human_highlighted, &machine_highlighted),
        ),
        ("categoryGroupsInNativeOrder", groups),
    ]);
    Ok(obj([
        ("model", s(CLEF_MODEL)),
        (
            "state",
            J::Arr(vec![
                context,
                obj([
                    ("type", s("input_audio")),
                    (
                        "input_audio",
                        obj([("data", s(wav_base64)), ("format", s("wav"))]),
                    ),
                ]),
                s(format!(
                    "Current project style guide:\n{}",
                    comparison.style_guide.content
                )),
            ]),
        ),
        ("questions", questions()),
        ("provider", obj([("allow_fallbacks", J::Bool(false))])),
    ]))
}

/// Validates the provider reply and converts the six answers into a label draft
/// (`parseErrorTaskDraftResponse` plus the broker's envelope checks).
pub fn parse_decision_response(payload: &Value) -> Result<ErrorLabelDraft, ErrorTaskError> {
    let invalid = |message: &str| ErrorTaskError::new("invalid-provider-response", message);
    let Some(object) = payload.as_object() else {
        return Err(invalid("The AI service sent an unexpected reply."));
    };
    if object.contains_key("error")
        || object.get("model").is_some_and(|model| {
            model.as_str().is_none_or(|model| {
                model != CLEF_MODEL && !model.starts_with(&format!("{CLEF_MODEL}-"))
            })
        })
    {
        return Err(invalid("The AI service sent an unexpected reply."));
    }
    let answers = object
        .get("answers")
        .and_then(Value::as_object)
        .filter(|answers| {
            answers.len() == QUESTION_KEYS.len()
                && answers
                    .keys()
                    .all(|key| QUESTION_KEYS.contains(&key.as_str()))
        })
        .ok_or_else(|| invalid("Clef returned an incomplete answer. Nothing was applied."))?;
    let mut choices = Vec::with_capacity(QUESTION_KEYS.len());
    for key in QUESTION_KEYS {
        let answer = answers.get(key).and_then(Value::as_object);
        let choice = answer
            .filter(|answer| answer.get("type").and_then(Value::as_str) == Some("choice"))
            .and_then(|answer| answer.get("choice"))
            .and_then(Value::as_str)
            .filter(|choice| question_choices(key).contains(choice))
            .ok_or_else(|| invalid("Clef returned an unexpected choice. Nothing was applied."))?;
        choices.push(choice.to_string());
    }
    let [
        outside,
        only_non_semantic,
        human_wrong,
        primary,
        has_secondary,
        secondary,
    ] = <[String; 6]>::try_from(choices).expect("six questions");
    let whole = outside == "no";
    let with_secondary = whole && has_secondary == "yes";
    if with_secondary && secondary == "none" {
        return Err(invalid("The AI's answer was incomplete. Review by hand."));
    }
    let draft = ErrorLabelDraft {
        captures_whole_error: whole,
        meaning_changed: only_non_semantic == "no",
        human_transcript_wrong: human_wrong == "yes",
        category_keys: if whole {
            let mut keys = vec![primary];
            if with_secondary {
                keys.push(secondary);
            }
            keys
        } else {
            Vec::new()
        },
    };
    if !draft.is_valid() {
        return Err(invalid(
            "Clef’s questionnaire violates native category rules. Paid output is retained; no choices were applied.",
        ));
    }
    Ok(draft)
}
