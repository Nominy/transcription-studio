use studio_app::{
    FEEDBACK_MAX_COMMENT_LENGTH, FeedbackDraft, FeedbackInput, FeedbackInputResponse,
    FeedbackRubricEntry, FeedbackRubricField, FeedbackSaveRequestInput,
    compute_feedback_draft_save_request, get_rubric_fields_for_flags, has_other_feedback_flag,
    parse_feedback_responses,
};
use studio_core::Project;

use super::StudioState;
use super::native_capabilities::{ReviewerCardPreview, ReviewerFeedbackPreview};
use super::service_callbacks::BabelServiceRequest;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FeedbackPanelState {
    pub(super) open: bool,
    pub(super) read_only: bool,
    pub(super) is_loading: bool,
    pub(super) is_saving: bool,
    pub(super) is_tts_project: bool,
    pub(super) last_saved: Option<String>,
    pub(super) draft: FeedbackDraft,
    pub(super) rubric_fields: Vec<FeedbackRubricField>,
    pub(super) rubric_flag_names: Option<Vec<String>>,
    pub(super) has_other_feedback: bool,
    form_step_response_id: Option<String>,
    inputs: Option<Vec<FeedbackInput>>,
}

impl FeedbackPanelState {
    pub(super) fn from_project(project: &Project) -> Option<Self> {
        let show_button = metadata_bool(project, META_FEEDBACK_SHOW_BUTTON)
            || metadata_bool(project, META_FEEDBACK_INITIAL_OPEN);
        if !show_button {
            return None;
        }

        let rubric_flag_names =
            parse_rubric_flags(project.metadata.get(META_FEEDBACK_RUBRIC_FLAGS));
        let rubric_flag_refs = rubric_flag_names
            .as_ref()
            .map(|flags| flags.iter().map(String::as_str).collect::<Vec<_>>());
        let rubric_fields = get_rubric_fields_for_flags(rubric_flag_refs.as_deref());
        let has_other_feedback = has_other_feedback_flag(rubric_flag_refs.as_deref());
        let read_only = metadata_bool(project, META_FEEDBACK_READ_ONLY);
        let inputs = project
            .metadata
            .get(META_FEEDBACK_INPUTS)
            .and_then(|value| parse_feedback_inputs(value));
        let draft = initial_feedback_draft(project, &rubric_fields, inputs.as_deref())
            .unwrap_or_else(|| {
                if read_only {
                    FeedbackDraft::default()
                } else {
                    default_feedback_draft(&rubric_fields)
                }
            });

        Some(Self {
            open: metadata_bool(project, META_FEEDBACK_INITIAL_OPEN),
            read_only,
            is_loading: metadata_bool(project, META_FEEDBACK_IS_LOADING),
            is_saving: false,
            is_tts_project: metadata_bool(project, META_FEEDBACK_IS_TTS_PROJECT),
            last_saved: project.metadata.get(META_FEEDBACK_LAST_SAVED).cloned(),
            draft,
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
        })
    }

    pub(super) fn title(&self) -> &'static str {
        if self.read_only {
            "Feedback Received"
        } else {
            "L1 Feedback Form"
        }
    }

    pub(super) fn description(&self) -> &'static str {
        if self.read_only {
            "Review the feedback provided."
        } else {
            "Provide feedback for the L1 transcriber."
        }
    }

    pub(super) fn footer_label(&self) -> &'static str {
        if self.is_ready() {
            "Ready"
        } else {
            "Complete all fields"
        }
    }

    pub(super) fn is_ready(&self) -> bool {
        self.rubric_fields.iter().all(|field| {
            self.draft
                .entries
                .get(field.key)
                .is_some_and(|entry| !entry.comment.trim().is_empty())
        })
    }

    fn set_rating(&mut self, field_key: &str, rating: u8) -> bool {
        let Some(entry) = self.draft.entries.get_mut(field_key) else {
            return false;
        };
        entry.rating = rating.clamp(1, 3);
        self.is_saving = true;
        true
    }

    fn set_comment(&mut self, field_key: &str, comment: String) -> bool {
        if comment.len() > FEEDBACK_MAX_COMMENT_LENGTH {
            return false;
        }
        let Some(entry) = self.draft.entries.get_mut(field_key) else {
            return false;
        };
        entry.comment = comment;
        self.is_saving = true;
        true
    }

    fn set_other_feedback(&mut self, feedback: String) -> bool {
        if feedback.len() > FEEDBACK_MAX_COMMENT_LENGTH {
            return false;
        }
        self.draft.other_feedback = feedback;
        self.is_saving = true;
        true
    }

    fn mark_saved(&mut self, last_saved: String) {
        self.is_saving = false;
        self.last_saved = Some(last_saved);
    }

    fn mark_save_failed(&mut self) {
        self.is_saving = false;
    }
}

impl StudioState {
    pub(super) fn open_feedback_panel(&mut self) {
        if let Some(panel) = self.feedback_panel.as_mut() {
            panel.open = true;
            self.status = "Feedback panel opened.".to_string();
        }
    }

    pub(super) fn close_feedback_panel(&mut self) {
        if let Some(panel) = self.feedback_panel.as_mut() {
            panel.open = false;
            self.status = "Feedback panel closed.".to_string();
        }
    }

    pub(super) fn change_feedback_rating(&mut self, field_key: &str, rating: u8) {
        let Some(panel) = self.feedback_panel.as_mut() else {
            return;
        };
        if panel.read_only {
            return;
        }
        if !panel.set_rating(field_key, rating) {
            return;
        }
        self.status = "Saving feedback draft...".to_string();
        self.update_feedback_submit_metadata();
        self.queue_feedback_draft_save_request();
    }

    pub(super) fn change_feedback_comment(&mut self, field_key: &str, comment: String) {
        let Some(panel) = self.feedback_panel.as_mut() else {
            return;
        };
        if panel.read_only {
            return;
        }
        if !panel.set_comment(field_key, comment) {
            return;
        }
        self.status = "Saving feedback draft...".to_string();
        self.update_feedback_submit_metadata();
        self.queue_feedback_draft_save_request();
    }

    pub(super) fn change_feedback_other_feedback(&mut self, feedback: String) {
        let Some(panel) = self.feedback_panel.as_mut() else {
            return;
        };
        if panel.read_only {
            return;
        }
        if !panel.set_other_feedback(feedback) {
            return;
        }
        self.status = "Saving feedback draft...".to_string();
        self.update_feedback_submit_metadata();
        self.queue_feedback_draft_save_request();
    }

    pub(super) fn apply_selected_reviewer_card_feedback(&mut self) {
        let card_id = self.reviewer_card_id.trim().to_string();
        if card_id.is_empty() {
            self.status = "Select a reviewer card before applying feedback.".to_string();
            return;
        }
        let Some((card, feedback)) = self.reviewer_card_feedback_for_id(&card_id) else {
            self.status = format!("Reviewer card {card_id} was not found.");
            return;
        };
        let Some(category) = reviewer_card_feedback_category(&card, feedback.as_ref()) else {
            self.status = format!("Reviewer card {card_id} has no feedback category.");
            return;
        };
        let Some(panel) = self.feedback_panel.as_ref() else {
            self.status = "Feedback panel is unavailable for this Babel task.".to_string();
            return;
        };
        if panel.read_only {
            self.status = "Feedback panel is read-only.".to_string();
            return;
        }
        let Some(field) = feedback_panel_field_for_category(panel, &category) else {
            self.status = format!("No feedback rubric field matches {category}.");
            return;
        };
        let existing_rating = panel
            .draft
            .entries
            .get(field.key)
            .map(|entry| entry.rating)
            .unwrap_or(1);
        let rating =
            reviewer_feedback_rating(feedback.as_ref().map(|feedback| feedback.score.as_str()))
                .unwrap_or(existing_rating);
        let comment = reviewer_card_feedback_comment(&card, feedback.as_ref());
        let field_key = field.key.to_string();
        let field_label = field.label;

        let Some(panel) = self.feedback_panel.as_mut() else {
            return;
        };
        panel
            .draft
            .entries
            .insert(field_key, FeedbackRubricEntry { rating, comment });
        panel.is_saving = true;
        panel.open = true;
        self.status = format!("Applied reviewer card {card_id} to {field_label} feedback.");
        self.update_feedback_submit_metadata();
        self.queue_feedback_draft_save_request();
    }

    pub(super) fn mark_feedback_draft_saved(&mut self, last_saved: String) {
        let Some(panel) = self.feedback_panel.as_mut() else {
            return;
        };
        panel.mark_saved(last_saved);
        self.status = "Feedback draft saved.".to_string();
        self.update_feedback_submit_metadata();
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::SaveFeedbackDraft(_))
        });
    }

    pub(super) fn fail_feedback_draft_save(&mut self, error: String) {
        let Some(panel) = self.feedback_panel.as_mut() else {
            return;
        };
        panel.mark_save_failed();
        self.status = format!("Feedback draft save failed: {error}");
        self.update_feedback_submit_metadata();
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::SaveFeedbackDraft(_))
        });
    }

    pub(super) fn update_feedback_submit_metadata(&mut self) {
        let Some(panel) = self.feedback_panel.as_ref() else {
            return;
        };
        let updated = if let Ok(workflow) = &mut self.workflow {
            workflow.set_feedback_submit_state(panel.is_saving, !panel.is_ready());
            true
        } else {
            false
        };
        if updated {
            self.invalidate_render_caches_for_project_metadata();
        }
    }

    fn queue_feedback_draft_save_request(&mut self) {
        let Some(panel) = self.feedback_panel.as_ref() else {
            return;
        };
        let request = compute_feedback_draft_save_request(FeedbackSaveRequestInput {
            enabled: !panel.read_only,
            form_step_response_id: panel.form_step_response_id.clone(),
            draft_feedback: panel.draft.clone(),
            inputs: panel.inputs.clone(),
            rubric_fields: Some(panel.rubric_fields.clone()),
            rubric_flag_names: panel.rubric_flag_names.clone(),
            has_other_feedback: Some(panel.has_other_feedback),
        });
        if let Some(request) = request {
            self.babel_service_requests
                .push(BabelServiceRequest::SaveFeedbackDraft(request));
        }
    }

    fn reviewer_card_feedback_for_id(
        &self,
        card_id: &str,
    ) -> Option<(ReviewerCardPreview, Option<ReviewerFeedbackPreview>)> {
        if let Some(generated) = self.reviewer_generated_review_preview.as_ref()
            && let Some(card) = generated.cards.iter().find(|card| card.id == card_id)
        {
            return Some((
                card.clone(),
                reviewer_feedback_for_card(card, &generated.feedback).cloned(),
            ));
        }
        if let Some(session) = self.reviewer_session_preview.as_ref()
            && let Some(card) = session.cards.iter().find(|card| card.id == card_id)
        {
            let feedback = self
                .reviewer_generated_review_preview
                .as_ref()
                .and_then(|generated| reviewer_feedback_for_card(card, &generated.feedback))
                .cloned();
            return Some((card.clone(), feedback));
        }
        None
    }
}

fn reviewer_feedback_for_card<'a>(
    card: &ReviewerCardPreview,
    feedback: &'a [ReviewerFeedbackPreview],
) -> Option<&'a ReviewerFeedbackPreview> {
    for category in &card.categories {
        if let Some(feedback) = feedback
            .iter()
            .find(|feedback| feedback_category_matches(&feedback.category, category))
        {
            return Some(feedback);
        }
    }
    if feedback.len() == 1 {
        feedback.first()
    } else {
        None
    }
}

fn reviewer_card_feedback_category(
    card: &ReviewerCardPreview,
    feedback: Option<&ReviewerFeedbackPreview>,
) -> Option<String> {
    card.categories
        .iter()
        .find(|category| !category.trim().is_empty())
        .cloned()
        .or_else(|| {
            feedback
                .map(|feedback| feedback.category.trim().to_string())
                .filter(|category| !category.is_empty())
        })
}

fn feedback_panel_field_for_category(
    panel: &FeedbackPanelState,
    category: &str,
) -> Option<FeedbackRubricField> {
    let normalized_category = normalized_feedback_label(category);
    panel.rubric_fields.iter().copied().find(|field| {
        normalized_feedback_label(field.label) == normalized_category
            || normalized_feedback_label(field.key) == normalized_category
    })
}

fn feedback_category_matches(left: &str, right: &str) -> bool {
    normalized_feedback_label(left) == normalized_feedback_label(right)
}

fn normalized_feedback_label(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn reviewer_feedback_rating(score: Option<&str>) -> Option<u8> {
    let score = score?.trim();
    if score.is_empty() {
        return None;
    }
    let mut digits = String::new();
    for (index, character) in score.chars().enumerate() {
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
        return None;
    };
    if (1..=3).contains(&parsed) {
        Some(parsed as u8)
    } else {
        None
    }
}

fn reviewer_card_feedback_comment(
    card: &ReviewerCardPreview,
    feedback: Option<&ReviewerFeedbackPreview>,
) -> String {
    let comment = feedback
        .and_then(|feedback| non_empty_feedback_value(&feedback.note))
        .or_else(|| {
            card.opinion_text
                .as_deref()
                .and_then(non_empty_feedback_value)
        })
        .or_else(|| card.rationale.as_deref().and_then(non_empty_feedback_value))
        .or_else(|| non_empty_feedback_value(&card.evidence))
        .or_else(|| non_empty_feedback_value(&card.summary))
        .unwrap_or_else(|| format!("Reviewer card {}", card.id));
    truncate_feedback_comment(comment)
}

fn non_empty_feedback_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

fn truncate_feedback_comment(value: String) -> String {
    if value.chars().count() <= FEEDBACK_MAX_COMMENT_LENGTH {
        return value;
    }
    value.chars().take(FEEDBACK_MAX_COMMENT_LENGTH).collect()
}

fn default_feedback_draft(rubric_fields: &[FeedbackRubricField]) -> FeedbackDraft {
    let mut draft = FeedbackDraft::default();
    for field in rubric_fields {
        draft.entries.insert(
            field.key.to_string(),
            FeedbackRubricEntry {
                rating: 1,
                comment: String::new(),
            },
        );
    }
    draft
}

fn initial_feedback_draft(
    project: &Project,
    rubric_fields: &[FeedbackRubricField],
    inputs: Option<&[FeedbackInput]>,
) -> Option<FeedbackDraft> {
    project
        .metadata
        .get(META_FEEDBACK_INITIAL_FEEDBACK)
        .and_then(|value| parse_initial_feedback(value, rubric_fields))
        .or_else(|| parse_form_data_feedback(project, inputs))
}

fn parse_initial_feedback(
    value: &str,
    rubric_fields: &[FeedbackRubricField],
) -> Option<FeedbackDraft> {
    let value = serde_json::from_str::<serde_json::Value>(value).ok()?;
    let object = value.as_object()?;
    let mut draft = default_feedback_draft(rubric_fields);

    for field in rubric_fields {
        let Some(entry_value) = object.get(field.key) else {
            continue;
        };
        if let Some(entry) = parse_initial_feedback_entry(entry_value) {
            draft.entries.insert(field.key.to_string(), entry);
        }
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

fn parse_initial_feedback_entry(value: &serde_json::Value) -> Option<FeedbackRubricEntry> {
    let object = value.as_object()?;
    Some(FeedbackRubricEntry {
        rating: normalize_feedback_rating_value(object.get("rating")),
        comment: object
            .get("comment")
            .and_then(json_scalar_to_string)
            .unwrap_or_default(),
    })
}

fn parse_form_data_feedback(
    project: &Project,
    inputs: Option<&[FeedbackInput]>,
) -> Option<FeedbackDraft> {
    let input_responses =
        parse_feedback_input_responses(project.metadata.get(META_FEEDBACK_INPUT_RESPONSES)?)?;
    parse_feedback_responses(&input_responses, inputs)
}

fn parse_feedback_inputs(value: &str) -> Option<Vec<FeedbackInput>> {
    let value = serde_json::from_str::<serde_json::Value>(value).ok()?;
    let inputs = value
        .as_array()?
        .iter()
        .filter_map(|input| {
            let object = input.as_object()?;
            let id = object.get("id").and_then(json_scalar_to_string)?;
            let label = object.get("label").and_then(json_scalar_to_string);
            Some(FeedbackInput { id, label })
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
            let form_input_id = object
                .get("formInputId")
                .or_else(|| object.get("form_input_id"))
                .and_then(json_scalar_to_string)?;
            let value = object.get("value").and_then(json_scalar_to_string)?;
            Some(FeedbackInputResponse {
                form_input_id,
                value,
            })
        })
        .collect::<Vec<_>>();

    (!responses.is_empty()).then_some(responses)
}

fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_string());
    }
    if value.is_number() || value.is_boolean() {
        return Some(value.to_string());
    }
    None
}

fn normalize_feedback_rating_value(value: Option<&serde_json::Value>) -> u8 {
    let Some(value) = value else {
        return 1;
    };
    if let Some(value) = value.as_i64() {
        return if (1..=3).contains(&value) {
            value as u8
        } else {
            1
        };
    }

    let Some(value) = value.as_str() else {
        return 1;
    };
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

fn parse_rubric_flags(value: Option<&String>) -> Option<Vec<String>> {
    let value = value?;
    if let Ok(serde_json::Value::Array(flags)) = serde_json::from_str::<serde_json::Value>(value) {
        let flags = flags
            .into_iter()
            .filter_map(|flag| flag.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        return (!flags.is_empty()).then_some(flags);
    }

    let flags = value
        .split(',')
        .map(str::trim)
        .filter(|flag| !flag.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
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
