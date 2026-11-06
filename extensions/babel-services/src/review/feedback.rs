//! Required L2 feedback form ("feedback drawer"): bootstrap from Babel, local draft edits,
//! single-flight autosave, submit gating, and applying reviewer feedback to the rubric.

use anyhow::{Result, anyhow, bail};
use exaudio_babel_domain::editor_model::feedback_assessment::{
    FEEDBACK_MAX_COMMENT_LENGTH, FeedbackDraft, FeedbackDraftSaveRequest, FeedbackInput,
    FeedbackInputResponse, FeedbackRubricEntry, FeedbackRubricField, FeedbackSaveRequestInput,
    compute_feedback_draft_save_request, get_rubric_fields_for_flags, has_other_feedback_flag,
    parse_feedback_responses,
};
use exaudio_babel_domain::i18n::{localized, with_detail};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use studio_core::Project;
use studio_editor::workflow::StudioWorkflow;

use super::model::{CategoryGrade, ReviewerFeedback};
use crate::babel::{BabelClient, FeedbackDraftBootstrap};

const META_FEEDBACK_SHOW_BUTTON: &str = "studio.babel.feedback.showButton";
const META_FEEDBACK_INITIAL_OPEN: &str = "studio.babel.feedback.initialOpen";
const META_FEEDBACK_READ_ONLY: &str = "studio.babel.feedback.readOnly";
const META_FEEDBACK_RUBRIC_FLAGS: &str = "studio.babel.feedback.rubricFlags";
const META_FEEDBACK_LAST_SAVED: &str = "studio.babel.feedback.lastSaved";
const META_FEEDBACK_IS_LOADING: &str = "studio.babel.feedback.isLoading";
const META_FEEDBACK_IS_TTS_PROJECT: &str = "studio.babel.feedback.isTtsProject";
const META_FEEDBACK_INITIAL_FEEDBACK: &str = "studio.babel.feedback.initialFeedback";
const META_FEEDBACK_INPUTS: &str = "studio.babel.feedback.inputs";
const META_FEEDBACK_INPUT_RESPONSES: &str = "studio.babel.feedback.inputResponses";
const META_FEEDBACK_FORM_STEP_RESPONSE_ID: &str = "studio.babel.feedback.formStepResponseId";

/// Ids needed to load the live feedback form for an editable L2 task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackBootstrapIds {
    pub project_id: String,
    pub review_action_id: String,
    pub worker_id: String,
}

impl FeedbackBootstrapIds {
    /// Errors when the project, task or worker id is blank.
    pub fn for_project(project: &Project, worker_id: &str) -> Result<Self> {
        let ids = Self {
            project_id: crate::babel::feedback_project_id(project),
            review_action_id: crate::babel::review_action_id(project),
            worker_id: worker_id.trim().to_string(),
        };
        if ids.project_id.is_empty() || ids.review_action_id.is_empty() || ids.worker_id.is_empty()
        {
            bail!(
                "{}",
                localized(
                    "The feedback form isn't available for this task.",
                    "Форма отзыва недоступна для этой задачи.",
                )
            );
        }
        Ok(ids)
    }
}

/// Everything Babel returns for the live form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackBootstrap {
    pub rubric_flag_names: Vec<String>,
    pub inputs: Vec<FeedbackInput>,
    pub draft: FeedbackDraftBootstrap,
}

/// Loads rubric flags in parallel with form → inputs → saved draft (the draft request waits
/// for the inputs, matching the old sequencing). Needs the services tokio runtime.
pub async fn load_feedback_bootstrap(
    client: &BabelClient,
    ids: &FeedbackBootstrapIds,
) -> Result<FeedbackBootstrap> {
    let form = async {
        let step_id = client.feedback_form_step_id(&ids.project_id).await?;
        let inputs = client.feedback_inputs(&step_id).await?;
        let draft = client
            .feedback_draft(&step_id, &ids.review_action_id, &ids.worker_id)
            .await?;
        anyhow::Ok((inputs, draft))
    };
    let (flags, form) = tokio::join!(client.feedback_rubric_flags(&ids.project_id), form);
    let (inputs, draft) = form?;
    Ok(FeedbackBootstrap {
        rubric_flag_names: flags?,
        inputs,
        draft,
    })
}

/// Saves one draft snapshot; returns Babel's `lastSaved` timestamp.
pub async fn save_feedback_draft(
    client: &BabelClient,
    request: &FeedbackDraftSaveRequest,
) -> Result<String> {
    Ok(client.save_feedback_draft(request).await?)
}

/// Whether the drawer should start open (`studio.babel.feedback.initialOpen`).
pub fn feedback_initially_open(project: &Project) -> bool {
    metadata_bool(project, META_FEEDBACK_INITIAL_OPEN)
}

pub fn feedback_is_tts_project(project: &Project) -> bool {
    metadata_bool(project, META_FEEDBACK_IS_TTS_PROJECT)
}

/// UI-free feedback form state. Edits return whether anything changed; call
/// [`FeedbackForm::request_save`] after a change and send the returned request (if any).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackForm {
    pub read_only: bool,
    pub is_tts_project: bool,
    pub rubric_fields: Vec<FeedbackRubricField>,
    pub rubric_flag_names: Option<Vec<String>>,
    pub has_other_feedback: bool,
    pub form_step_response_id: Option<String>,
    pub inputs: Option<Vec<FeedbackInput>>,
    is_loading: bool,
    bootstrap_error: Option<String>,
    draft: FeedbackDraft,
    last_saved: Option<String>,
    is_saving: bool,
    save_has_error: bool,
    last_autosave_error: Option<String>,
    save_in_flight: Option<FeedbackDraftSaveRequest>,
}

impl FeedbackForm {
    /// Form described by imported project metadata (`studio.babel.feedback.*`), shown when
    /// `showButton` or `initialOpen` is set. Read-only forms without saved feedback stay empty.
    pub fn from_project(project: &Project) -> Option<Self> {
        if !metadata_bool(project, META_FEEDBACK_SHOW_BUTTON)
            && !metadata_bool(project, META_FEEDBACK_INITIAL_OPEN)
        {
            return None;
        }
        let rubric_flag_names = project
            .metadata
            .get(META_FEEDBACK_RUBRIC_FLAGS)
            .and_then(|value| parse_rubric_flags(value));
        let flag_refs = rubric_flag_names
            .as_ref()
            .map(|flags| flags.iter().map(String::as_str).collect::<Vec<_>>());
        let rubric_fields = get_rubric_fields_for_flags(flag_refs.as_deref());
        let has_other_feedback = has_other_feedback_flag(flag_refs.as_deref());
        let read_only = metadata_bool(project, META_FEEDBACK_READ_ONLY);
        let inputs = project
            .metadata
            .get(META_FEEDBACK_INPUTS)
            .and_then(|value| parse_feedback_inputs(value));
        let draft = project
            .metadata
            .get(META_FEEDBACK_INITIAL_FEEDBACK)
            .and_then(|value| parse_initial_feedback(value, &rubric_fields))
            .or_else(|| {
                let responses = parse_feedback_input_responses(
                    project.metadata.get(META_FEEDBACK_INPUT_RESPONSES)?,
                )?;
                parse_feedback_responses(&responses, inputs.as_deref())
            })
            .unwrap_or_else(|| {
                if read_only {
                    FeedbackDraft::default()
                } else {
                    default_feedback_draft(&rubric_fields)
                }
            });
        Some(Self {
            read_only,
            is_tts_project: feedback_is_tts_project(project),
            rubric_fields,
            rubric_flag_names,
            has_other_feedback,
            form_step_response_id: project
                .metadata
                .get(META_FEEDBACK_FORM_STEP_RESPONSE_ID)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            inputs,
            is_loading: metadata_bool(project, META_FEEDBACK_IS_LOADING),
            bootstrap_error: None,
            draft,
            last_saved: project.metadata.get(META_FEEDBACK_LAST_SAVED).cloned(),
            is_saving: false,
            save_has_error: false,
            last_autosave_error: None,
            save_in_flight: None,
        })
    }

    /// Placeholder while the live form loads (blocks submit).
    pub fn loading(is_tts_project: bool) -> Self {
        Self {
            read_only: false,
            is_tts_project,
            rubric_fields: Vec::new(),
            rubric_flag_names: None,
            has_other_feedback: false,
            form_step_response_id: None,
            inputs: None,
            is_loading: true,
            bootstrap_error: None,
            draft: FeedbackDraft::default(),
            last_saved: None,
            is_saving: false,
            save_has_error: false,
            last_autosave_error: None,
            save_in_flight: None,
        }
    }

    /// Live form from a completed bootstrap; saved responses hydrate the draft.
    pub fn from_bootstrap(bootstrap: FeedbackBootstrap, is_tts_project: bool) -> Result<Self> {
        if bootstrap.inputs.is_empty() {
            bail!("The server's feedback form has no inputs.");
        }
        let flag_refs = bootstrap
            .rubric_flag_names
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let rubric_fields = get_rubric_fields_for_flags(Some(&flag_refs));
        let has_other_feedback = has_other_feedback_flag(Some(&flag_refs));
        let has_responses = !bootstrap.draft.input_responses.is_empty();
        let draft = if has_responses {
            parse_feedback_responses(&bootstrap.draft.input_responses, Some(&bootstrap.inputs))
                .ok_or_else(|| {
                    anyhow!("The saved feedback draft doesn't match the form's inputs.")
                })?
        } else {
            default_feedback_draft(&rubric_fields)
        };
        Ok(Self {
            read_only: false,
            is_tts_project,
            rubric_fields,
            rubric_flag_names: Some(bootstrap.rubric_flag_names),
            has_other_feedback,
            form_step_response_id: Some(bootstrap.draft.form_step_response_id),
            inputs: Some(bootstrap.inputs),
            is_loading: false,
            bootstrap_error: None,
            draft,
            last_saved: has_responses.then_some(bootstrap.draft.updated_at),
            is_saving: false,
            save_has_error: false,
            last_autosave_error: None,
            save_in_flight: None,
        })
    }

    /// Bootstrap failed: the form stays unavailable (and blocks submit) until a retry succeeds.
    pub fn mark_bootstrap_failed(&mut self, error: impl Into<String>) {
        self.is_loading = false;
        self.bootstrap_error = Some(error.into());
        self.is_saving = false;
        self.save_has_error = false;
        self.last_autosave_error = None;
    }

    pub fn draft(&self) -> &FeedbackDraft {
        &self.draft
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn bootstrap_error(&self) -> Option<&str> {
        self.bootstrap_error.as_deref()
    }

    pub fn is_saving(&self) -> bool {
        self.is_saving
    }

    pub fn save_has_error(&self) -> bool {
        self.save_has_error
    }

    pub fn last_autosave_error(&self) -> Option<&str> {
        self.last_autosave_error.as_deref()
    }

    pub fn last_saved(&self) -> Option<&str> {
        self.last_saved.as_deref()
    }

    /// Loaded, no bootstrap error, and every rubric field has a non-blank comment.
    pub fn is_ready(&self) -> bool {
        !self.is_loading
            && self.bootstrap_error.is_none()
            && self.rubric_fields.iter().all(|field| {
                self.draft
                    .entries
                    .get(field.key)
                    .is_some_and(|entry| !entry.comment.trim().is_empty())
            })
    }

    /// Why submit must be blocked before the confirm dialog (form loading or failed to load).
    pub fn submit_block_reason(&self) -> Option<String> {
        if self.is_loading {
            return Some(
                localized(
                    "The feedback form is still loading.",
                    "Форма отзыва ещё загружается.",
                )
                .to_owned(),
            );
        }
        self.bootstrap_error.as_ref().map(|error| {
            with_detail(
                localized(
                    "Couldn't load the feedback form. Use Retry in the Feedback panel.",
                    "Не удалось загрузить форму отзыва. Нажмите «Повторить» на панели отзыва.",
                ),
                error,
            )
        })
    }

    /// Mirrors save/readiness into the workflow's submit metadata (drives submit gating).
    pub fn sync_submit_state(&self, workflow: &mut StudioWorkflow) {
        workflow.set_feedback_submit_state(
            self.is_loading
                || self.bootstrap_error.is_some()
                || self.is_saving
                || self.save_has_error,
            !self.is_ready(),
        );
    }

    /// Rating clamped to 1..=3; false when read-only or the field is unknown.
    pub fn set_rating(&mut self, field_key: &str, rating: u8) -> bool {
        if self.read_only {
            return false;
        }
        let Some(entry) = self.draft.entries.get_mut(field_key) else {
            return false;
        };
        entry.rating = rating.clamp(1, 3);
        true
    }

    /// Rejects edits longer than `FEEDBACK_MAX_COMMENT_LENGTH` characters.
    pub fn set_comment(&mut self, field_key: &str, comment: &str) -> bool {
        if self.read_only || comment.chars().count() > FEEDBACK_MAX_COMMENT_LENGTH {
            return false;
        }
        let Some(entry) = self.draft.entries.get_mut(field_key) else {
            return false;
        };
        if entry.comment == comment {
            return false;
        }
        entry.comment = comment.to_string();
        true
    }

    pub fn set_other_feedback(&mut self, feedback: &str) -> bool {
        if self.read_only
            || !self.has_other_feedback
            || feedback.chars().count() > FEEDBACK_MAX_COMMENT_LENGTH
            || self.draft.other_feedback == feedback
        {
            return false;
        }
        self.draft.other_feedback = feedback.to_string();
        true
    }

    /// Applies every generated/finalized feedback item with a non-empty note to its rubric field
    /// (exact label/key match, else a unique alias match). Returns the number applied.
    pub fn apply_reviewer_feedback(&mut self, feedback: &[ReviewerFeedback]) -> usize {
        if self.read_only {
            return 0;
        }
        let mut applied = 0;
        for item in feedback {
            let Some(comment) = non_empty_feedback_value(&item.note) else {
                continue;
            };
            let Some(field) = field_for_category(&self.rubric_fields, &item.category) else {
                continue;
            };
            let rating = reviewer_feedback_rating(&item.score)
                .unwrap_or_else(|| self.existing_rating(field.key));
            self.draft.entries.insert(
                field.key.to_string(),
                FeedbackRubricEntry {
                    rating,
                    comment: truncate_comment(comment),
                },
            );
            applied += 1;
        }
        applied
    }

    /// Grader apply: sets only the rating of each grade's rubric field; comments are untouched.
    /// Returns the number of fields updated.
    pub fn apply_grade_scores(&mut self, grades: &[CategoryGrade]) -> usize {
        if self.read_only {
            return 0;
        }
        let mut applied = 0;
        for grade in grades {
            let Some(field) = field_for_category(&self.rubric_fields, grade.category) else {
                continue;
            };
            let entry = self
                .draft
                .entries
                .entry(field.key.to_string())
                .or_insert_with(|| FeedbackRubricEntry {
                    rating: 1,
                    comment: String::new(),
                });
            entry.rating = grade.score.clamp(1, 3);
            applied += 1;
        }
        applied
    }

    /// Save payload for the current draft (`None` when read-only or no form step response).
    pub fn current_save_request(&self) -> Option<FeedbackDraftSaveRequest> {
        compute_feedback_draft_save_request(FeedbackSaveRequestInput {
            enabled: !self.read_only,
            form_step_response_id: self.form_step_response_id.clone(),
            draft_feedback: self.draft.clone(),
            inputs: self.inputs.clone(),
            rubric_fields: Some(self.rubric_fields.clone()),
            rubric_flag_names: self.rubric_flag_names.clone(),
            has_other_feedback: Some(self.has_other_feedback),
        })
    }

    /// Marks the draft as saving and returns the request to send now, or `None` when a save is
    /// already in flight (the newest snapshot is sent when it settles) or nothing is savable.
    pub fn request_save(&mut self) -> Option<FeedbackDraftSaveRequest> {
        let request = self.current_save_request()?;
        self.is_saving = true;
        self.save_has_error = false;
        self.last_autosave_error = None;
        if self.save_in_flight.is_some() {
            return None;
        }
        self.save_in_flight = Some(request.clone());
        Some(request)
    }

    /// Settles the in-flight save `sent` (`Ok(last_saved)` or the error). Late results for other
    /// snapshots are ignored. Returns the next request to send when the draft changed meanwhile.
    pub fn finish_save(
        &mut self,
        sent: &FeedbackDraftSaveRequest,
        result: Result<String>,
    ) -> Option<FeedbackDraftSaveRequest> {
        if self.save_in_flight.as_ref() != Some(sent) {
            return None;
        }
        self.save_in_flight = None;
        if self.current_save_request().as_ref() != Some(sent) {
            return self.request_save();
        }
        self.is_saving = false;
        match result {
            Ok(last_saved) => {
                self.save_has_error = false;
                self.last_autosave_error = None;
                self.last_saved = Some(last_saved);
            }
            Err(error) => {
                let error = format!("{error:#}");
                let error = error.trim();
                self.save_has_error = true;
                self.last_autosave_error = Some(if error.is_empty() {
                    localized("Couldn't save the feedback.", "Не удалось сохранить отзыв.")
                        .to_owned()
                } else {
                    error.to_owned()
                });
            }
        }
        None
    }

    fn existing_rating(&self, field_key: &str) -> u8 {
        self.draft
            .entries
            .get(field_key)
            .map(|entry| entry.rating)
            .unwrap_or(1)
    }
}

/// Rubric field for a reviewer category: exact (normalized) label/key match, else a unique
/// alias match (ambiguous aliases match nothing).
pub fn field_for_category(
    fields: &[FeedbackRubricField],
    category: &str,
) -> Option<FeedbackRubricField> {
    let normalized = normalized_feedback_label(category);
    if let Some(field) = fields.iter().copied().find(|field| {
        normalized_feedback_label(field.label) == normalized
            || normalized_feedback_label(field.key) == normalized
    }) {
        return Some(field);
    }
    let mut matches = fields
        .iter()
        .copied()
        .filter(|field| category_aliases(field.key).contains(&normalized.as_str()));
    let field = matches.next()?;
    matches.next().is_none().then_some(field)
}

/// Leading integer of a score (`"2 - acceptable"` → 2) when it is 1..=3.
pub fn reviewer_feedback_rating(score: &str) -> Option<u8> {
    let rating = leading_integer(score.trim())?;
    (1..=3).contains(&rating).then_some(rating as u8)
}

fn category_aliases(field_key: &str) -> &'static [&'static str] {
    match field_key {
        "wordAccuracy" => &[
            "word",
            "words",
            "transcript",
            "transcription",
            "transcriptaccuracy",
            "transcriptionaccuracy",
        ],
        "timestampAccuracy" => &[
            "time",
            "timing",
            "timestamp",
            "timestamps",
            "timingaccuracy",
            "timestampstiming",
        ],
        "punctuationFormatting" => &["formatting", "punctuation", "punctuationandformatting"],
        "tagsEmphasis" => &["emphasis", "tags", "tagsandemphasis"],
        "segmentation" => &["segment", "segmentbreaks", "segments"],
        "wordCharacterAccuracy" => &[
            "characteraccuracy",
            "characters",
            "characterset",
            "transcript",
            "transcription",
            "transcriptaccuracy",
            "transcriptionaccuracy",
            "word",
            "wordaccuracy",
            "words",
        ],
        "punctuationFormattingAccuracy" => &[
            "formatting",
            "punctuation",
            "punctuationaccuracy",
            "punctuationandformatting",
        ],
        "nonSpeechTagAccuracy" => &[
            "nonspeech",
            "nonspeechtag",
            "nonspeechtags",
            "soundtag",
            "soundtags",
        ],
        "styleTagAccuracy" => &[
            "deliverytag",
            "deliverytags",
            "emphasis",
            "style",
            "styletag",
            "styletags",
        ],
        _ => &[],
    }
}

fn normalized_feedback_label(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn leading_integer(value: &str) -> Option<i32> {
    let sign_len = usize::from(value.starts_with(['-', '+']));
    let digits_len = value[sign_len..]
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len() - sign_len);
    value[..sign_len + digits_len].parse().ok()
}

fn non_empty_feedback_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn truncate_comment(value: String) -> String {
    if value.chars().count() <= FEEDBACK_MAX_COMMENT_LENGTH {
        value
    } else {
        value.chars().take(FEEDBACK_MAX_COMMENT_LENGTH).collect()
    }
}

fn default_feedback_draft(rubric_fields: &[FeedbackRubricField]) -> FeedbackDraft {
    FeedbackDraft {
        entries: rubric_fields
            .iter()
            .map(|field| {
                (
                    field.key.to_string(),
                    FeedbackRubricEntry {
                        rating: 1,
                        comment: String::new(),
                    },
                )
            })
            .collect(),
        other_feedback: String::new(),
    }
}

fn parse_initial_feedback(
    value: &str,
    rubric_fields: &[FeedbackRubricField],
) -> Option<FeedbackDraft> {
    let value = serde_json::from_str::<serde_json::Value>(value).ok()?;
    let object = value.as_object()?;
    let mut draft = default_feedback_draft(rubric_fields);
    for field in rubric_fields {
        let Some(entry) = object.get(field.key).and_then(serde_json::Value::as_object) else {
            continue;
        };
        draft.entries.insert(
            field.key.to_string(),
            FeedbackRubricEntry {
                rating: normalize_rating_value(entry.get("rating")),
                comment: entry
                    .get("comment")
                    .and_then(json_scalar_to_string)
                    .unwrap_or_default(),
            },
        );
    }
    if let Some(other_feedback) = object
        .get("otherFeedback")
        .or_else(|| object.get("other_feedback"))
        .and_then(json_scalar_to_string)
    {
        draft.other_feedback = other_feedback;
    }
    Some(draft)
}

fn parse_feedback_inputs(value: &str) -> Option<Vec<FeedbackInput>> {
    let value = serde_json::from_str::<serde_json::Value>(value).ok()?;
    let inputs = value
        .as_array()?
        .iter()
        .filter_map(|input| {
            let object = input.as_object()?;
            Some(FeedbackInput {
                id: object.get("id").and_then(json_scalar_to_string)?,
                label: object.get("label").and_then(json_scalar_to_string),
            })
        })
        .collect::<Vec<_>>();
    (!inputs.is_empty()).then_some(inputs)
}

fn parse_feedback_input_responses(value: &str) -> Option<Vec<FeedbackInputResponse>> {
    let value = serde_json::from_str::<serde_json::Value>(value).ok()?;
    let responses = value
        .as_array()?
        .iter()
        .filter_map(|response| {
            let object = response.as_object()?;
            Some(FeedbackInputResponse {
                form_input_id: object
                    .get("formInputId")
                    .or_else(|| object.get("form_input_id"))
                    .and_then(json_scalar_to_string)?,
                value: object.get("value").and_then(json_scalar_to_string)?,
            })
        })
        .collect::<Vec<_>>();
    (!responses.is_empty()).then_some(responses)
}

fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

fn normalize_rating_value(value: Option<&serde_json::Value>) -> u8 {
    let rating = match value {
        Some(serde_json::Value::Number(number)) => {
            number.as_i64().and_then(|n| i32::try_from(n).ok())
        }
        Some(serde_json::Value::String(text)) => leading_integer(text.trim_start()),
        _ => None,
    };
    rating
        .filter(|rating| (1..=3).contains(rating))
        .map_or(1, |rating| rating as u8)
}

fn parse_rubric_flags(value: &str) -> Option<Vec<String>> {
    let flags = match serde_json::from_str::<serde_json::Value>(value) {
        Ok(serde_json::Value::Array(flags)) => flags
            .into_iter()
            .filter_map(|flag| flag.as_str().map(str::to_string))
            .collect::<Vec<_>>(),
        _ => value
            .split(',')
            .map(str::trim)
            .filter(|flag| !flag.is_empty())
            .map(str::to_string)
            .collect(),
    };
    (!flags.is_empty()).then_some(flags)
}

fn metadata_bool(project: &Project, key: &str) -> bool {
    project.metadata.get(key).is_some_and(|value| {
        matches!(
            value.as_str(),
            "true" | "1" | "yes" | "on" | "True" | "TRUE" | "Yes" | "YES" | "On" | "ON"
        )
    })
}
