use std::collections::{BTreeMap, HashMap};

use crate::i18n::localized;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedbackRubricField {
    pub key: &'static str,
    /// The server's form-input label; matches draft responses, never shown.
    pub label: &'static str,
    /// Display title `(en, ru)`.
    pub title: (&'static str, &'static str),
    /// Display description `(en, ru)`.
    pub description: Option<(&'static str, &'static str)>,
    pub required: bool,
}

impl FeedbackRubricField {
    /// Title shown in the feedback form.
    pub fn title(&self) -> &'static str {
        localized(self.title.0, self.title.1)
    }

    /// Description shown under the title in the feedback form.
    pub fn hint(&self) -> Option<&'static str> {
        self.description.map(|(en, ru)| localized(en, ru))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedbackInput {
    pub id: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedbackInputResponse {
    pub form_input_id: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedbackRubricEntry {
    pub rating: u8,
    pub comment: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeedbackDraft {
    pub entries: BTreeMap<String, FeedbackRubricEntry>,
    pub other_feedback: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedbackSaveRequestInput {
    pub enabled: bool,
    pub form_step_response_id: Option<String>,
    pub draft_feedback: FeedbackDraft,
    pub inputs: Option<Vec<FeedbackInput>>,
    pub rubric_fields: Option<Vec<FeedbackRubricField>>,
    pub rubric_flag_names: Option<Vec<String>>,
    pub has_other_feedback: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedbackDraftSaveRequest {
    pub form_step_response_id: String,
    pub input_responses: Vec<FeedbackInputResponse>,
}

pub const FEEDBACK_MAX_COMMENT_LENGTH: usize = 500;

pub const FEEDBACK_RUBRIC_FIELDS: [FeedbackRubricField; 9] = [
    FeedbackRubricField {
        key: "wordAccuracy",
        label: "Word Accuracy",
        title: ("Word accuracy", "Точность слов"),
        description: Some((
            "How accurately words and phrases are transcribed",
            "Насколько точно переданы слова и фразы",
        )),
        required: true,
    },
    FeedbackRubricField {
        key: "timestampAccuracy",
        label: "Timestamp Accuracy",
        title: ("Timestamp accuracy", "Точность таймкодов"),
        description: Some((
            "How precise segment start and end times are",
            "Насколько точны начало и конец сегментов",
        )),
        required: true,
    },
    FeedbackRubricField {
        key: "punctuationFormatting",
        label: "Punctuation & Formatting",
        title: ("Punctuation and formatting", "Пунктуация и форматирование"),
        description: Some((
            "Correct punctuation and text formatting",
            "Правильность пунктуации и форматирования текста",
        )),
        required: true,
    },
    FeedbackRubricField {
        key: "tagsEmphasis",
        label: "Tags & Emphasis",
        title: ("Tags and emphasis", "Теги и акценты"),
        description: Some((
            "Correct speaker tags and emphasis markers",
            "Правильность тегов спикеров и маркеров акцента",
        )),
        required: true,
    },
    FeedbackRubricField {
        key: "segmentation",
        label: "Segmentation",
        title: ("Segmentation", "Сегментация"),
        description: Some((
            "Sensible segment breaks and grouping",
            "Уместное разбиение на сегменты",
        )),
        required: true,
    },
    FeedbackRubricField {
        key: "wordCharacterAccuracy",
        label: "Word & Character Set Accuracy",
        title: (
            "Word and character set accuracy",
            "Точность слов и набора символов",
        ),
        description: Some((
            "How accurately words and character sets are transcribed",
            "Насколько точно переданы слова и наборы символов",
        )),
        required: true,
    },
    FeedbackRubricField {
        key: "punctuationFormattingAccuracy",
        label: "Punctuation & Formatting Accuracy",
        title: (
            "Punctuation and formatting accuracy",
            "Точность пунктуации и форматирования",
        ),
        description: Some((
            "Correct punctuation and formatting",
            "Правильность пунктуации и форматирования",
        )),
        required: true,
    },
    FeedbackRubricField {
        key: "nonSpeechTagAccuracy",
        label: "Non-Speech Tag Accuracy",
        title: ("Non-speech tag accuracy", "Точность неречевых тегов"),
        description: Some((
            "Correct non-speech sound tags, such as [soupir] or [rire]",
            "Правильность тегов неречевых звуков, например [soupir] или [rire]",
        )),
        required: true,
    },
    FeedbackRubricField {
        key: "styleTagAccuracy",
        label: "Style Tag Accuracy",
        title: ("Style tag accuracy", "Точность стилевых тегов"),
        description: Some((
            "Correct style and delivery tags, such as <doux> or <intensite-crescendo>",
            "Правильность тегов стиля и подачи, например <doux> или <intensite-crescendo>",
        )),
        required: true,
    },
];

pub const OTHER_FEEDBACK_FLAGS: [&str; 2] = ["tx-other-feedback", "tts-other-feedback"];

fn feedback_flag_for_label(label: &str) -> Option<&'static str> {
    match label {
        "Word Accuracy" => Some("tx-word-accuracy"),
        "Timestamp Accuracy" => Some("tx-timestamp-accuracy"),
        "Punctuation & Formatting" => Some("tx-punctuation-formatting"),
        "Tags & Emphasis" => Some("tx-tags-emphasis"),
        "Segmentation" => Some("tx-segmentation"),
        "Word & Character Set Accuracy" => Some("tts-word-character-accuracy"),
        "Punctuation & Formatting Accuracy" => Some("tts-punctuation-formatting-accuracy"),
        "Non-Speech Tag Accuracy" => Some("tts-non-speech-tag-accuracy"),
        "Style Tag Accuracy" => Some("tts-style-tag-accuracy"),
        _ => None,
    }
}

fn default_transcription_rubric_fields() -> Vec<FeedbackRubricField> {
    FEEDBACK_RUBRIC_FIELDS
        .iter()
        .copied()
        .filter(|field| {
            feedback_flag_for_label(field.label).is_some_and(|flag| flag.starts_with("tx-"))
        })
        .collect()
}

fn normalize_feedback_rating(value: Option<&str>) -> u8 {
    let value = value.filter(|value| !value.is_empty()).unwrap_or("1");
    let value = value.trim_start();
    let mut digits = String::new();

    for (index, character) in value.chars().enumerate() {
        if index == 0 && (character == '-' || character == '+') {
            digits.push(character);
            continue;
        }
        if character.is_ascii_digit() {
            digits.push(character);
            continue;
        }
        break;
    }

    let Ok(parsed) = digits.parse::<i32>() else {
        return 1;
    };

    if (1..=3).contains(&parsed) {
        parsed as u8
    } else {
        1
    }
}

pub fn get_rubric_fields_for_flags(rubric_flag_names: Option<&[&str]>) -> Vec<FeedbackRubricField> {
    let Some(rubric_flag_names) = rubric_flag_names else {
        return default_transcription_rubric_fields();
    };

    FEEDBACK_RUBRIC_FIELDS
        .iter()
        .copied()
        .filter(|field| {
            feedback_flag_for_label(field.label)
                .is_some_and(|flag| rubric_flag_names.contains(&flag))
        })
        .collect()
}

pub fn has_other_feedback_flag(rubric_flag_names: Option<&[&str]>) -> bool {
    let Some(rubric_flag_names) = rubric_flag_names else {
        return true;
    };

    OTHER_FEEDBACK_FLAGS
        .iter()
        .any(|flag| rubric_flag_names.contains(flag))
}

pub fn parse_feedback_responses(
    input_responses: &[FeedbackInputResponse],
    inputs: Option<&[FeedbackInput]>,
) -> Option<FeedbackDraft> {
    let inputs = inputs?;
    if input_responses.is_empty() {
        return None;
    }

    let response_value_by_input_id: HashMap<&str, &str> = input_responses
        .iter()
        .map(|response| (response.form_input_id.as_str(), response.value.as_str()))
        .collect();
    let mut value_by_label = HashMap::new();

    for input in inputs {
        let Some(label) = input.label.as_deref() else {
            continue;
        };
        if let Some(value) = response_value_by_input_id.get(input.id.as_str()) {
            value_by_label.insert(label, *value);
        }
    }

    let mut draft = FeedbackDraft::default();
    for field in FEEDBACK_RUBRIC_FIELDS {
        draft.entries.insert(
            field.key.to_string(),
            FeedbackRubricEntry {
                rating: normalize_feedback_rating(value_by_label.get(field.label).copied()),
                comment: value_by_label
                    .get(format!("{} Comment", field.label).as_str())
                    .copied()
                    .unwrap_or("")
                    .to_string(),
            },
        );
    }

    draft.other_feedback = value_by_label
        .get("Other Feedback")
        .or_else(|| value_by_label.get("TTS Other Feedback"))
        .copied()
        .unwrap_or("")
        .to_string();

    Some(draft)
}

pub fn serialize_feedback_draft(
    draft_feedback: &FeedbackDraft,
    inputs: Option<&[FeedbackInput]>,
    rubric_fields: Option<&[FeedbackRubricField]>,
    rubric_flag_names: Option<&[&str]>,
    has_other_feedback: Option<bool>,
) -> Vec<FeedbackInputResponse> {
    let Some(inputs) = inputs else {
        return Vec::new();
    };

    let owned_rubric_fields;
    let rubric_fields = if let Some(rubric_fields) = rubric_fields {
        rubric_fields
    } else {
        owned_rubric_fields = get_rubric_fields_for_flags(rubric_flag_names);
        &owned_rubric_fields
    };
    let has_other_feedback =
        has_other_feedback.unwrap_or_else(|| has_other_feedback_flag(rubric_flag_names));
    let mut input_responses = Vec::new();

    for form_input in inputs {
        let Some(label) = form_input.label.as_deref() else {
            continue;
        };

        for field in rubric_fields {
            if label == field.label {
                let rating = draft_feedback
                    .entries
                    .get(field.key)
                    .map(|entry| entry.rating)
                    .unwrap_or(1);
                input_responses.push(FeedbackInputResponse {
                    form_input_id: form_input.id.clone(),
                    value: rating.to_string(),
                });
            } else if label == format!("{} Comment", field.label) {
                let comment = draft_feedback
                    .entries
                    .get(field.key)
                    .map(|entry| entry.comment.clone())
                    .unwrap_or_default();
                input_responses.push(FeedbackInputResponse {
                    form_input_id: form_input.id.clone(),
                    value: comment,
                });
            }
        }

        if has_other_feedback && (label == "Other Feedback" || label == "TTS Other Feedback") {
            input_responses.push(FeedbackInputResponse {
                form_input_id: form_input.id.clone(),
                value: draft_feedback.other_feedback.clone(),
            });
        }
    }

    input_responses
}

pub fn compute_feedback_draft_save_request(
    input: FeedbackSaveRequestInput,
) -> Option<FeedbackDraftSaveRequest> {
    if !input.enabled {
        return None;
    }
    let form_step_response_id = input.form_step_response_id?;
    let flag_names = input
        .rubric_flag_names
        .as_ref()
        .map(|flags| flags.iter().map(String::as_str).collect::<Vec<_>>());

    Some(FeedbackDraftSaveRequest {
        form_step_response_id,
        input_responses: serialize_feedback_draft(
            &input.draft_feedback,
            input.inputs.as_deref(),
            input.rubric_fields.as_deref(),
            flag_names.as_deref(),
            input.has_other_feedback,
        ),
    })
}
