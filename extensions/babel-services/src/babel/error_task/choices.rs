//! The error-label questionnaire: Babel's static category vocabulary (native priority
//! order), pairing rules, completeness/validation messages and the
//! `transcriptions.submitTranscriptionErrorLabel` payload, as in Babel's web editor.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use super::task::{ErrorLabel, ErrorSource};

/// Project category of error-labeling projects (`ll.FP`); these claim QA tasks by diff id.
pub const ERROR_CATEGORIZATION: &str = "error-categorization";
/// Maximum length of `notes` / `otherIssues` (UTF-16 units, `ll.BP`).
pub const MAX_NOTE_UNITS: usize = 2000;

/// A category bucket in native order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryGroup {
    pub id: &'static str,
    pub name: &'static str,
    pub categories: &'static [&'static str],
}

/// Babel's buckets and categories in native priority order (`ll.K5`).
pub const CATEGORY_GROUPS: [CategoryGroup; 7] = [
    CategoryGroup {
        id: "cant",
        name: "Can't hear",
        categories: &["hard_to_hear"],
    },
    CategoryGroup {
        id: "disf",
        name: "Speech disfluency",
        categories: &[
            "false_starts",
            "repetition",
            "self_corrections",
            "filler_words",
        ],
    },
    CategoryGroup {
        id: "same",
        name: "Same words, written differently",
        categories: &[
            "script",
            "diacritics",
            "compound_words",
            "contractions",
            "abbrev_acronym",
            "spelled_out",
            "punctuation",
            "spelling",
        ],
    },
    CategoryGroup {
        id: "num",
        name: "Numbers and formats",
        categories: &[
            "dates",
            "times",
            "currency",
            "measurements",
            "phone_numbers",
            "addresses",
            "alphanumerics",
            "numbers",
        ],
    },
    CategoryGroup {
        id: "miss",
        name: "Missing or extra words",
        categories: &["word_added_or_dropped", "phrase_added_or_dropped"],
    },
    CategoryGroup {
        id: "wrong",
        name: "Wrong words",
        categories: &[
            "named_entities",
            "code_switching",
            "homophones",
            "different_tone",
            "affix",
            "near_homophones",
            "possessives",
            "different_phrase",
            "different_word",
        ],
    },
    CategoryGroup {
        id: "nothing",
        name: "Nothing fits",
        categories: &["not_sure", "typo"],
    },
];

/// Categories that need audio; hidden and invalid on synthetic (text-only) diffs (`ll.z9`).
pub const AUDIO_ONLY_CATEGORIES: [&str; 1] = ["hard_to_hear"];

/// Short bucket names of the submitted-labels table (`l$`).
fn short_group_name(id: &str) -> &'static str {
    match id {
        "cant" => "Can't hear",
        "disf" => "Disfluency",
        "same" => "Same words",
        "num" => "Numbers",
        "miss" => "Missing / extra",
        "wrong" => "Wrong words",
        _ => "Nothing fits",
    }
}

/// Every category key in native order (`ll.e4`).
pub fn category_keys() -> impl Iterator<Item = &'static str> {
    CATEGORY_GROUPS
        .iter()
        .flat_map(|group| group.categories.iter().copied())
}

fn category_index(key: &str) -> Option<usize> {
    category_keys().position(|known| known == key)
}

/// Index of the group (bucket) containing `key`.
pub fn group_index(key: &str) -> Option<usize> {
    CATEGORY_GROUPS
        .iter()
        .position(|group| group.categories.contains(&key))
}

pub fn is_known_category(key: &str) -> bool {
    category_index(key).is_some()
}

#[derive(Deserialize)]
struct Localized {
    buckets: BTreeMap<String, String>,
    categories: BTreeMap<String, String>,
}

/// Babel's per-language bucket/category display names (captured web module `71840`).
static LOCALIZED: LazyLock<BTreeMap<String, Localized>> = LazyLock::new(|| {
    serde_json::from_str(exaudio_babel_lint::data::ERROR_CATEGORY_NAMES_JSON)
        .expect("embedded error-category names are valid JSON")
});

/// Display name of `key` in the task language, else the key itself (`lW.lk`).
pub fn category_name(key: &str, language: &str) -> String {
    LOCALIZED
        .get(language)
        .and_then(|names| names.categories.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Localized bucket name, if the language has one (`lW.r2`).
fn localized_group_name(id: &str, language: &str) -> Option<String> {
    LOCALIZED
        .get(language)
        .and_then(|names| names.buckets.get(id))
        .cloned()
}

/// Bucket heading in the picker: localized, else the English bucket name (`lW.AM`).
pub fn group_name(group: &CategoryGroup, language: &str) -> String {
    localized_group_name(group.id, language).unwrap_or_else(|| group.name.to_string())
}

/// Second categories may not come from these buckets, except `affix`.
const NO_SECOND_GROUPS: [&str; 2] = ["miss", "wrong"];
const FIRST_FOUR_GROUPS: [&str; 4] = ["cant", "disf", "same", "num"];

fn group_id(key: &str) -> Option<&'static str> {
    group_index(key).map(|index| CATEGORY_GROUPS[index].id)
}

/// Why this category selection breaks Babel's pairing rules (`lz`), lower-case as Babel
/// phrases it; `None` when allowed. One category is always fine.
pub fn pairing_problem(keys: &[String]) -> Option<&'static str> {
    if keys.len() > 2 {
        return Some("pick at most two categories");
    }
    if keys.len() < 2 {
        return None;
    }
    if keys
        .iter()
        .any(|key| AUDIO_ONLY_CATEGORIES.contains(&key.as_str()))
    {
        return Some("hard_to_hear can't be combined with another category");
    }
    let mut sorted: Vec<&str> = keys.iter().map(String::as_str).collect();
    sorted.sort_by_key(|key| category_index(key).unwrap_or(0));
    let (first, second) = (sorted[0], sorted[1]);
    if group_id(second).is_some_and(|group| NO_SECOND_GROUPS.contains(&group)) && second != "affix"
    {
        return Some(
            "a second category can't be from Missing or extra words or Wrong words, except affix",
        );
    }
    let in_first_four =
        |key: &str| group_id(key).is_some_and(|group| FIRST_FOUR_GROUPS.contains(&group));
    if in_first_four(first) && !in_first_four(second) {
        return Some("a category from the first four groups only pairs with another from them");
    }
    None
}

/// Gold's `validErrorTaskCategories`: 1–2 distinct known keys obeying the pairing rules.
pub fn valid_categories(keys: &[String]) -> bool {
    let distinct = keys
        .iter()
        .enumerate()
        .all(|(index, key)| !keys[..index].contains(key));
    !keys.is_empty()
        && keys.len() <= 2
        && distinct
        && keys.iter().all(|key| is_known_category(key))
        && pairing_problem(keys).is_none()
}

/// The questionnaire state (`lZ` initial: three unanswered questions, nothing else).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorChoices {
    pub captures_whole_error: Option<bool>,
    pub meaning_changed: Option<bool>,
    pub human_transcript_wrong: Option<bool>,
    pub category_keys: Vec<String>,
    pub other_issues: String,
    pub notes: String,
}

impl ErrorChoices {
    /// Toggles `key` in the selection (removal always allowed; adding clears nothing else).
    pub fn toggle_category(&mut self, key: &str) {
        if let Some(index) = self
            .category_keys
            .iter()
            .position(|selected| selected == key)
        {
            self.category_keys.remove(index);
        } else {
            self.category_keys.push(key.to_string());
        }
    }

    /// Whether adding `key` would break the pairing rules (the picker disables it).
    pub fn category_blocked(&self, key: &str) -> bool {
        if self.category_keys.iter().any(|selected| selected == key) {
            return false;
        }
        let mut next = self.category_keys.clone();
        next.push(key.to_string());
        pairing_problem(&next).is_some()
    }
}

/// `transcriptions.submitTranscriptionErrorLabel` input (`lq`). Absent fields are omitted,
/// as JavaScript `undefined` is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorLabelSubmission {
    pub review_action_id: String,
    pub captures_whole_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meaning_changed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_transcript_wrong: Option<bool>,
    pub category_keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_issues: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn trimmed(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Builds the submission for `choices` (`lq`). "Not whole" discards the diff: no categories
/// and no answers, only the optional notes.
pub fn submission(
    review_action_id: &str,
    choices: &ErrorChoices,
    source: ErrorSource,
) -> ErrorLabelSubmission {
    let other_issues = trimmed(&choices.other_issues);
    let notes = trimmed(&choices.notes);
    if choices.captures_whole_error == Some(true) {
        ErrorLabelSubmission {
            review_action_id: review_action_id.to_string(),
            captures_whole_error: true,
            meaning_changed: choices.meaning_changed,
            human_transcript_wrong: match source {
                ErrorSource::Real => choices.human_transcript_wrong,
                ErrorSource::Synthetic => None,
            },
            category_keys: choices.category_keys.clone(),
            other_issues,
            notes,
        }
    } else {
        ErrorLabelSubmission {
            review_action_id: review_action_id.to_string(),
            captures_whole_error: false,
            meaning_changed: None,
            human_transcript_wrong: None,
            category_keys: Vec::new(),
            other_issues,
            notes,
        }
    }
}

/// Unanswered required items (`S`): `["first question"]`, `["category", "2 questions"]` …
pub fn missing_items(choices: &ErrorChoices, source: ErrorSource) -> Vec<String> {
    match choices.captures_whole_error {
        None => vec!["first question".to_string()],
        Some(false) => Vec::new(),
        Some(true) => {
            let mut unanswered = usize::from(choices.meaning_changed.is_none());
            if source == ErrorSource::Real && choices.human_transcript_wrong.is_none() {
                unanswered += 1;
            }
            let mut missing = Vec::new();
            if choices.category_keys.is_empty() {
                missing.push("category".to_string());
            }
            if unanswered > 0 {
                let plural = if unanswered == 1 { "" } else { "s" };
                missing.push(format!("{unanswered} question{plural}"));
            }
            missing
        }
    }
}

/// `lQ`: capitalize and end with a period.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => ".".to_string(),
    }
}

/// Why a complete-looking answer still can't be submitted (`_`), or `None` when it can.
pub fn validation_problem(choices: &ErrorChoices, source: ErrorSource) -> Option<String> {
    let Some(whole) = choices.captures_whole_error else {
        return Some(sentence(
            "Answer whether the highlight captures the whole error",
        ));
    };
    if !whole {
        return None;
    }
    let mut keys: Vec<String> = Vec::new();
    for key in &choices.category_keys {
        if !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    let problem = if keys.is_empty() {
        Some("pick at least one category")
    } else if let Some(problem) = pairing_problem(&keys) {
        Some(problem)
    } else if choices.meaning_changed.is_none() {
        Some("answer whether the error changed the meaning")
    } else if source == ErrorSource::Real && choices.human_transcript_wrong.is_none() {
        Some("answer whether the human transcript is wrong")
    } else if source == ErrorSource::Synthetic
        && keys
            .iter()
            .any(|key| AUDIO_ONLY_CATEGORIES.contains(&key.as_str()))
    {
        Some("a synthetic diff has no audio, so hard_to_hear doesn't apply")
    } else {
        None
    };
    problem.map(sentence)
}

/// Readiness of the questionnaire: `Ok(())` when submittable, else the status text Babel
/// shows next to the buttons (`Missing: …` or the validation message).
pub fn readiness(choices: &ErrorChoices, source: ErrorSource) -> Result<(), String> {
    let missing = missing_items(choices, source);
    if !missing.is_empty() {
        return Err(format!("Missing: {}", missing.join(", ")));
    }
    validation_problem(choices, source).map_or(Ok(()), Err)
}

/// One row of a submitted-labels table (`l2`): `L<level>` or `QA` plus the label.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelRow {
    pub name: String,
    pub label: ErrorLabel,
}

/// Prior labels, then the submitted one (named `QA` on QA reviews).
pub fn label_rows(
    prior: &[ErrorLabel],
    submitted: Option<&ErrorLabel>,
    is_qa_review: bool,
) -> Vec<LabelRow> {
    let level_name = |label: &ErrorLabel| match label.level {
        Some(level) => format!("L{level}"),
        None => "L".to_string(),
    };
    let mut rows: Vec<LabelRow> = prior
        .iter()
        .map(|label| LabelRow {
            name: level_name(label),
            label: label.clone(),
        })
        .collect();
    if let Some(label) = submitted {
        rows.push(LabelRow {
            name: if is_qa_review {
                "QA".to_string()
            } else {
                level_name(label)
            },
            label: label.clone(),
        });
    }
    rows
}

/// A label's categories grouped by bucket for the labels table: `(bucket, names)`, unknown
/// keys under "Other".
pub fn grouped_label_categories(keys: &[String], language: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = CATEGORY_GROUPS
        .iter()
        .map(|group| {
            (
                localized_group_name(group.id, language)
                    .unwrap_or_else(|| short_group_name(group.id).to_string()),
                group
                    .categories
                    .iter()
                    .filter(|key| keys.iter().any(|selected| selected == *key))
                    .map(|key| category_name(key, language))
                    .collect(),
            )
        })
        .collect();
    groups.push((
        "Other".to_string(),
        keys.iter()
            .filter(|key| !is_known_category(key))
            .cloned()
            .collect(),
    ));
    groups.retain(|(_, names)| !names.is_empty());
    groups
}

/// `L1`/`L2` picks per category key (badges in the picker).
pub fn picks_by_key(prior: &[ErrorLabel]) -> BTreeMap<String, Vec<String>> {
    let mut picks: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for label in prior {
        let name = label
            .level
            .map_or_else(|| "L".to_string(), |level| format!("L{level}"));
        for key in &label.category_keys {
            picks.entry(key.clone()).or_default().push(name.clone());
        }
    }
    picks
}

/// L1's category selection, when it is a valid known-key list ("Agree with L1's selection").
pub fn level_one_categories(prior: &[ErrorLabel]) -> Option<Vec<String>> {
    let label = prior.iter().find(|label| label.level == Some(1))?;
    (!label.category_keys.is_empty()
        && label.category_keys.iter().all(|key| is_known_category(key)))
    .then(|| label.category_keys.clone())
}

/// Picker groups after the filter query and audio availability (`lV`): each group with the
/// categories to show; empty groups dropped.
pub fn filtered_groups(
    query: &str,
    has_audio: bool,
    language: &str,
) -> Vec<(&'static CategoryGroup, Vec<&'static str>)> {
    let needle = query.trim().to_lowercase();
    CATEGORY_GROUPS
        .iter()
        .filter_map(|group| {
            let categories: Vec<&'static str> = group
                .categories
                .iter()
                .copied()
                .filter(|key| has_audio || !AUDIO_ONLY_CATEGORIES.contains(key))
                .filter(|key| {
                    needle.is_empty()
                        || [
                            key.to_string(),
                            category_name(key, language),
                            group.name.to_string(),
                            group_name(group, language),
                        ]
                        .iter()
                        .any(|candidate| candidate.to_lowercase().contains(&needle))
                })
                .collect();
            (!categories.is_empty()).then_some((group, categories))
        })
        .collect()
}

/// The decision Initial fill proposes (Gold `ErrorLabelDraft`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorLabelDraft {
    pub captures_whole_error: bool,
    pub meaning_changed: bool,
    pub human_transcript_wrong: bool,
    pub category_keys: Vec<String>,
}

impl ErrorLabelDraft {
    /// Gold `isErrorLabelDraft`: whole → valid 1–2 categories; not whole → none.
    pub fn is_valid(&self) -> bool {
        if self.captures_whole_error {
            valid_categories(&self.category_keys)
        } else {
            self.category_keys.is_empty()
        }
    }
}

/// Why Initial fill could not be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApplyDraftError {
    #[error("You edited the choices during Initial fill. Nothing was overwritten.")]
    AnswersChanged,
    #[error("The suggestion conflicts with your first answer. Your choices weren’t changed.")]
    IncompatibleDraft,
}

/// Fills only unanswered choices of `baseline` from `draft` (Gold native apply); notes and
/// other issues are never touched. `current` must still equal `baseline` (the user may have
/// edited during the provider call). `Ok(None)`: everything was already answered.
pub fn apply_draft(
    baseline: &ErrorChoices,
    current: &ErrorChoices,
    draft: &ErrorLabelDraft,
) -> Result<Option<ErrorChoices>, ApplyDraftError> {
    if current != baseline {
        return Err(ApplyDraftError::AnswersChanged);
    }
    let whole = baseline
        .captures_whole_error
        .unwrap_or(draft.captures_whole_error);
    let categories = if !baseline.category_keys.is_empty() || !whole {
        baseline.category_keys.clone()
    } else {
        draft.category_keys.clone()
    };
    if whole && !valid_categories(&categories) {
        return Err(ApplyDraftError::IncompatibleDraft);
    }
    let next = ErrorChoices {
        captures_whole_error: Some(whole),
        meaning_changed: if whole {
            baseline.meaning_changed.or(Some(draft.meaning_changed))
        } else {
            baseline.meaning_changed
        },
        human_transcript_wrong: if whole {
            baseline
                .human_transcript_wrong
                .or(Some(draft.human_transcript_wrong))
        } else {
            baseline.human_transcript_wrong
        },
        category_keys: categories,
        other_issues: baseline.other_issues.clone(),
        notes: baseline.notes.clone(),
    };
    Ok((next != *baseline).then_some(next))
}
