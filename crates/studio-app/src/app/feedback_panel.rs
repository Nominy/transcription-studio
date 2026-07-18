use std::collections::BTreeMap;

use iced::widget::text_editor;
use studio_app::{
    FEEDBACK_MAX_COMMENT_LENGTH, FeedbackDraft, FeedbackInput, FeedbackInputResponse,
    FeedbackRubricEntry, FeedbackRubricField, FeedbackSaveRequestInput,
    compute_feedback_draft_save_request, get_rubric_fields_for_flags, has_other_feedback_flag,
    parse_feedback_responses,
};
use studio_core::Project;

use super::StudioState;
use super::native_capabilities::{ReviewerCardPreview, ReviewerFeedbackPreview};
use super::service_callbacks::{
    BabelServiceRequest, FeedbackBootstrapToken, FeedbackDraftBootstrap,
    FeedbackDraftSaveServiceRequest, FeedbackDraftSaveToken, LoadFeedbackDraftRequest,
    LoadFeedbackFormRequest, LoadFeedbackInputsRequest, LoadFeedbackRubricFlagsRequest,
    feedback_project_id, review_action_id,
};
use super::state::FeedbackBootstrapState;

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

pub(super) struct FeedbackPanelState {
    pub(super) open: bool,
    pub(super) read_only: bool,
    pub(super) is_loading: bool,
    pub(super) bootstrap_error: Option<String>,
    pub(super) is_saving: bool,
    pub(super) save_has_error: bool,
    pub(super) last_autosave_error: Option<String>,
    pub(super) is_tts_project: bool,
    pub(super) last_saved: Option<String>,
    pub(super) draft: FeedbackDraft,
    pub(super) rubric_fields: Vec<FeedbackRubricField>,
    pub(super) rubric_flag_names: Option<Vec<String>>,
    pub(super) has_other_feedback: bool,
    comment_contents: BTreeMap<String, text_editor::Content>,
    other_feedback_content: Option<text_editor::Content>,
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

        let comment_contents = feedback_comment_contents(&rubric_fields, &draft);
        let other_feedback_content =
            has_other_feedback.then(|| text_editor::Content::with_text(&draft.other_feedback));

        Some(Self {
            open: metadata_bool(project, META_FEEDBACK_INITIAL_OPEN),
            read_only,
            is_loading: metadata_bool(project, META_FEEDBACK_IS_LOADING),
            bootstrap_error: None,
            is_saving: false,
            save_has_error: false,
            last_autosave_error: None,
            is_tts_project: metadata_bool(project, META_FEEDBACK_IS_TTS_PROJECT),
            last_saved: project.metadata.get(META_FEEDBACK_LAST_SAVED).cloned(),
            draft,
            rubric_fields,
            rubric_flag_names,
            has_other_feedback,
            comment_contents,
            other_feedback_content,
            form_step_response_id: project
                .metadata
                .get(META_FEEDBACK_FORM_STEP_RESPONSE_ID)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            inputs,
        })
    }

    pub(super) fn loading(open: bool, is_tts_project: bool) -> Self {
        Self {
            open,
            read_only: false,
            is_loading: true,
            bootstrap_error: None,
            is_saving: false,
            save_has_error: false,
            last_autosave_error: None,
            is_tts_project,
            last_saved: None,
            draft: FeedbackDraft::default(),
            rubric_fields: Vec::new(),
            rubric_flag_names: None,
            has_other_feedback: false,
            comment_contents: BTreeMap::new(),
            other_feedback_content: None,
            form_step_response_id: None,
            inputs: None,
        }
    }

    pub(super) fn from_runtime(
        open: bool,
        is_tts_project: bool,
        rubric_flag_names: Vec<String>,
        inputs: Vec<FeedbackInput>,
        response: FeedbackDraftBootstrap,
    ) -> Result<Self, String> {
        if inputs.is_empty() {
            return Err("Babel feedback form returned no inputs".to_string());
        }
        let rubric_flag_refs = rubric_flag_names
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let rubric_fields = get_rubric_fields_for_flags(Some(&rubric_flag_refs));
        let has_other_feedback = has_other_feedback_flag(Some(&rubric_flag_refs));
        let has_responses = !response.input_responses.is_empty();
        let draft = if has_responses {
            parse_feedback_responses(&response.input_responses, Some(&inputs)).ok_or_else(|| {
                "Babel feedback draft responses could not be matched to form inputs".to_string()
            })?
        } else {
            default_feedback_draft(&rubric_fields)
        };

        let comment_contents = feedback_comment_contents(&rubric_fields, &draft);
        let other_feedback_content =
            has_other_feedback.then(|| text_editor::Content::with_text(&draft.other_feedback));

        Ok(Self {
            open,
            read_only: false,
            is_loading: false,
            bootstrap_error: None,
            is_saving: false,
            save_has_error: false,
            last_autosave_error: None,
            is_tts_project,
            last_saved: has_responses.then_some(response.updated_at),
            draft,
            rubric_fields,
            rubric_flag_names: Some(rubric_flag_names),
            has_other_feedback,
            comment_contents,
            other_feedback_content,
            form_step_response_id: Some(response.form_step_response_id),
            inputs: Some(inputs),
        })
    }

    fn mark_bootstrap_failed(&mut self, error: String) {
        self.is_loading = false;
        self.bootstrap_error = Some(error);
        self.is_saving = false;
        self.save_has_error = false;
        self.last_autosave_error = None;
    }

    pub(super) fn comment_content(&self, field_key: &str) -> Option<&text_editor::Content> {
        self.comment_contents.get(field_key)
    }

    pub(super) fn other_feedback_content(&self) -> Option<&text_editor::Content> {
        self.other_feedback_content.as_ref()
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
        if self.bootstrap_error.is_some() {
            "Feedback unavailable"
        } else if self.save_has_error {
            "Save failed"
        } else if self.is_saving {
            "Saving..."
        } else if self.is_ready() {
            "Ready"
        } else {
            "Complete all fields"
        }
    }

    pub(super) fn is_ready(&self) -> bool {
        !self.is_loading
            && self.bootstrap_error.is_none()
            && self.rubric_fields.iter().all(|field| {
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
        self.mark_save_pending();
        true
    }

    fn perform_comment_action(&mut self, field_key: &str, action: text_editor::Action) -> bool {
        let Some(before) = self
            .draft
            .entries
            .get(field_key)
            .map(|entry| entry.comment.clone())
        else {
            return false;
        };
        let Some(content) = self.comment_contents.get_mut(field_key) else {
            return false;
        };
        content.perform(action);
        let comment = normalized_editor_text(content);
        if comment.chars().count() > FEEDBACK_MAX_COMMENT_LENGTH {
            *content = text_editor::Content::with_text(&before);
            return false;
        }
        if comment == before {
            return false;
        }
        let Some(entry) = self.draft.entries.get_mut(field_key) else {
            return false;
        };
        entry.comment = comment;
        self.mark_save_pending();
        true
    }

    fn perform_other_feedback_action(&mut self, action: text_editor::Action) -> bool {
        let before = self.draft.other_feedback.clone();
        let Some(content) = self.other_feedback_content.as_mut() else {
            return false;
        };
        content.perform(action);
        let feedback = normalized_editor_text(content);
        if feedback.chars().count() > FEEDBACK_MAX_COMMENT_LENGTH {
            *content = text_editor::Content::with_text(&before);
            return false;
        }
        if feedback == before {
            return false;
        }
        self.draft.other_feedback = feedback;
        self.mark_save_pending();
        true
    }

    fn replace_entry(&mut self, field_key: &str, entry: FeedbackRubricEntry) {
        if self.comment_contents.contains_key(field_key) {
            let current_editor_text = self
                .comment_contents
                .get(field_key)
                .map(normalized_editor_text);
            if current_editor_text.as_deref() != Some(entry.comment.as_str()) {
                self.comment_contents.insert(
                    field_key.to_string(),
                    text_editor::Content::with_text(&entry.comment),
                );
            }
        }
        self.draft.entries.insert(field_key.to_string(), entry);
    }

    fn mark_save_pending(&mut self) {
        self.is_saving = true;
        self.save_has_error = false;
        self.last_autosave_error = None;
    }

    fn mark_saved(&mut self, last_saved: String) {
        self.is_saving = false;
        self.save_has_error = false;
        self.last_autosave_error = None;
        self.last_saved = Some(last_saved);
    }

    fn mark_save_failed(&mut self, error: String) {
        self.is_saving = false;
        self.save_has_error = true;
        let error = error.trim();
        self.last_autosave_error = Some(if error.is_empty() {
            "Unknown autosave error".to_string()
        } else {
            error.to_string()
        });
    }
}

impl StudioState {
    pub(super) fn initialize_feedback_panel_for_workflow(&mut self) {
        let should_bootstrap = self.reviewer_capability_available()
            && self
                .project()
                .is_some_and(|project| !studio_app::EditorSession::project_is_read_only(project))
            && !self
                .feedback_panel
                .as_ref()
                .is_some_and(|panel| panel.read_only);
        if should_bootstrap {
            self.begin_feedback_bootstrap();
        } else {
            self.feedback_bootstrap = None;
        }
    }

    pub(super) fn retry_feedback_bootstrap(&mut self) {
        if !self.reviewer_capability_available() {
            self.status =
                "Feedback retry unavailable: this is not an editable L2 task.".to_string();
            return;
        }
        if let Some(panel) = self.feedback_panel.as_mut() {
            panel.open = true;
        }
        self.begin_feedback_bootstrap();
    }

    fn begin_feedback_bootstrap(&mut self) {
        self.feedback_bootstrap_attempt = self.feedback_bootstrap_attempt.saturating_add(1);
        let token = FeedbackBootstrapToken {
            workspace_generation: self.workspace_generation,
            attempt: self.feedback_bootstrap_attempt,
        };
        self.drain_feedback_bootstrap_requests();

        let (project_id, review_action_id, initial_open, is_tts_project) = self
            .project()
            .map(|project| {
                (
                    feedback_project_id(project),
                    review_action_id(project),
                    metadata_bool(project, META_FEEDBACK_INITIAL_OPEN),
                    metadata_bool(project, META_FEEDBACK_IS_TTS_PROJECT),
                )
            })
            .unwrap_or_default();
        let worker_id = self.dashboard.worker_id.trim().to_string();
        let open = self.feedback_panel.as_ref().is_some_and(|panel| panel.open) || initial_open;
        let is_tts_project = self
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| panel.is_tts_project)
            || is_tts_project;
        self.feedback_panel = Some(FeedbackPanelState::loading(open, is_tts_project));
        self.feedback_bootstrap = Some(FeedbackBootstrapState {
            token,
            project_id: project_id.clone(),
            review_action_id: review_action_id.clone(),
            worker_id: worker_id.clone(),
            rubric_flag_names: None,
            form_step_id: None,
            inputs: None,
            draft: None,
            inputs_requested: false,
            draft_requested: false,
        });
        self.update_feedback_submit_metadata();

        let mut missing = Vec::new();
        if project_id.is_empty() {
            missing.push("Babel project id");
        }
        if review_action_id.is_empty() {
            missing.push("review action id");
        }
        if worker_id.is_empty() {
            missing.push("worker id");
        }
        if !missing.is_empty() {
            self.fail_feedback_bootstrap(
                token,
                format!("Feedback form unavailable: missing {}.", missing.join(", ")),
            );
            return;
        }

        self.babel_service_requests
            .push(BabelServiceRequest::LoadFeedbackRubricFlags(
                LoadFeedbackRubricFlagsRequest {
                    project_id: project_id.clone(),
                    token,
                },
            ));
        self.babel_service_requests
            .push(BabelServiceRequest::LoadFeedbackForm(
                LoadFeedbackFormRequest { project_id, token },
            ));
        self.status = "Loading required L2 feedback form...".to_string();
    }

    pub(super) fn complete_feedback_rubric_flags(
        &mut self,
        token: FeedbackBootstrapToken,
        rubric_flag_names: Vec<String>,
    ) {
        if !self.feedback_bootstrap_token_is_current(token) {
            return;
        }
        self.drain_babel_service_requests_matching(|request| {
            matches!(
                request,
                BabelServiceRequest::LoadFeedbackRubricFlags(request)
                    if request.token == token
            )
        });
        if let Some(bootstrap) = self.feedback_bootstrap.as_mut() {
            bootstrap.rubric_flag_names = Some(rubric_flag_names);
        }
        self.try_finish_feedback_bootstrap();
    }

    pub(super) fn complete_feedback_form(
        &mut self,
        token: FeedbackBootstrapToken,
        form_step_id: String,
    ) {
        if !self.feedback_bootstrap_token_is_current(token) {
            return;
        }
        self.drain_babel_service_requests_matching(|request| {
            matches!(
                request,
                BabelServiceRequest::LoadFeedbackForm(request) if request.token == token
            )
        });
        let should_request_inputs = self.feedback_bootstrap.as_mut().is_some_and(|bootstrap| {
            bootstrap.form_step_id = Some(form_step_id.clone());
            if bootstrap.inputs_requested {
                false
            } else {
                bootstrap.inputs_requested = true;
                true
            }
        });
        if should_request_inputs {
            self.babel_service_requests
                .push(BabelServiceRequest::LoadFeedbackInputs(
                    LoadFeedbackInputsRequest {
                        step_id: form_step_id,
                        token,
                    },
                ));
        }
    }

    pub(super) fn complete_feedback_inputs(
        &mut self,
        token: FeedbackBootstrapToken,
        inputs: Vec<FeedbackInput>,
    ) {
        if !self.feedback_bootstrap_token_is_current(token) {
            return;
        }
        self.drain_babel_service_requests_matching(|request| {
            matches!(
                request,
                BabelServiceRequest::LoadFeedbackInputs(request) if request.token == token
            )
        });
        let draft_request = self.feedback_bootstrap.as_mut().and_then(|bootstrap| {
            bootstrap.inputs = Some(inputs);
            if bootstrap.draft_requested {
                return None;
            }
            bootstrap.draft_requested = true;
            Some(LoadFeedbackDraftRequest {
                form_step_id: bootstrap.form_step_id.clone().unwrap_or_default(),
                review_action_id: bootstrap.review_action_id.clone(),
                worker_id: bootstrap.worker_id.clone(),
                token,
            })
        });
        if let Some(request) = draft_request {
            self.babel_service_requests
                .push(BabelServiceRequest::LoadFeedbackDraft(request));
        }
    }

    pub(super) fn complete_feedback_draft(
        &mut self,
        token: FeedbackBootstrapToken,
        draft: FeedbackDraftBootstrap,
    ) {
        if !self.feedback_bootstrap_token_is_current(token) {
            return;
        }
        self.drain_babel_service_requests_matching(|request| {
            matches!(
                request,
                BabelServiceRequest::LoadFeedbackDraft(request) if request.token == token
            )
        });
        if let Some(bootstrap) = self.feedback_bootstrap.as_mut() {
            bootstrap.draft = Some(draft);
        }
        self.try_finish_feedback_bootstrap();
    }

    pub(super) fn fail_feedback_bootstrap(&mut self, token: FeedbackBootstrapToken, error: String) {
        if !self.feedback_bootstrap_token_is_current(token) {
            return;
        }
        self.drain_feedback_bootstrap_requests();
        self.feedback_bootstrap = None;
        if let Some(panel) = self.feedback_panel.as_mut() {
            panel.mark_bootstrap_failed(error.clone());
            panel.open = true;
        }
        self.status = error;
        self.update_feedback_submit_metadata();
    }

    fn try_finish_feedback_bootstrap(&mut self) {
        let Some((token, rubric_flag_names, inputs, draft)) =
            self.feedback_bootstrap.as_ref().and_then(|bootstrap| {
                Some((
                    bootstrap.token,
                    bootstrap.rubric_flag_names.clone()?,
                    bootstrap.inputs.clone()?,
                    bootstrap.draft.clone()?,
                ))
            })
        else {
            return;
        };
        let open = self.feedback_panel.as_ref().is_some_and(|panel| panel.open);
        let is_tts_project = self
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| panel.is_tts_project);
        match FeedbackPanelState::from_runtime(
            open,
            is_tts_project,
            rubric_flag_names,
            inputs,
            draft,
        ) {
            Ok(panel) => {
                self.feedback_bootstrap = None;
                self.feedback_panel = Some(panel);
                self.status = "Required L2 feedback form loaded.".to_string();
                self.update_feedback_submit_metadata();
            }
            Err(error) => self.fail_feedback_bootstrap(token, error),
        }
    }

    fn feedback_bootstrap_token_is_current(&self, token: FeedbackBootstrapToken) -> bool {
        token.workspace_generation == self.workspace_generation
            && self
                .feedback_bootstrap
                .as_ref()
                .is_some_and(|bootstrap| bootstrap.token == token)
    }

    fn drain_feedback_bootstrap_requests(&mut self) {
        self.drain_babel_service_requests_matching(|request| {
            matches!(
                request,
                BabelServiceRequest::LoadFeedbackRubricFlags(_)
                    | BabelServiceRequest::LoadFeedbackForm(_)
                    | BabelServiceRequest::LoadFeedbackInputs(_)
                    | BabelServiceRequest::LoadFeedbackDraft(_)
            )
        });
    }

    pub(super) fn feedback_bootstrap_blocks_submit(&mut self) -> bool {
        let Some(panel) = self.feedback_panel.as_mut() else {
            return false;
        };
        let message = if panel.is_loading {
            Some("Submit blocked: the required feedback form is still loading.".to_string())
        } else {
            panel.bootstrap_error.as_ref().map(|error| {
                format!(
                    "Submit blocked: the required feedback form could not be loaded: {error} Use Retry in the feedback panel."
                )
            })
        };
        let Some(message) = message else {
            return false;
        };
        panel.open = true;
        self.show_submit_dialog = false;
        self.status = message;
        true
    }

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
        self.feedback_draft_revision = self.feedback_draft_revision.saturating_add(1);
        self.status = "Saving feedback draft...".to_string();
        self.update_feedback_submit_metadata();
        self.queue_feedback_draft_save_request();
    }

    pub(super) fn edit_feedback_comment(&mut self, field_key: &str, action: text_editor::Action) {
        let Some(panel) = self.feedback_panel.as_mut() else {
            return;
        };
        if panel.read_only {
            return;
        }
        if !panel.perform_comment_action(field_key, action) {
            return;
        }
        self.feedback_draft_revision = self.feedback_draft_revision.saturating_add(1);
        self.status = "Saving feedback draft...".to_string();
        self.update_feedback_submit_metadata();
        self.queue_feedback_draft_save_request();
    }

    pub(super) fn edit_feedback_other_feedback(&mut self, action: text_editor::Action) {
        let Some(panel) = self.feedback_panel.as_mut() else {
            return;
        };
        if panel.read_only {
            return;
        }
        if !panel.perform_other_feedback_action(action) {
            return;
        }
        self.feedback_draft_revision = self.feedback_draft_revision.saturating_add(1);
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
        panel.replace_entry(&field_key, FeedbackRubricEntry { rating, comment });
        panel.mark_save_pending();
        panel.open = true;
        self.feedback_draft_revision = self.feedback_draft_revision.saturating_add(1);
        self.status = format!("Applied reviewer card {card_id} to {field_label} feedback.");
        self.update_feedback_submit_metadata();
        self.queue_feedback_draft_save_request();
    }

    pub(super) fn apply_reviewer_feedback_items(
        &mut self,
        feedback: &[ReviewerFeedbackPreview],
    ) -> usize {
        let Some(panel) = self.feedback_panel.as_mut() else {
            self.status = "Feedback panel is unavailable for this Babel task.".to_string();
            return 0;
        };
        if panel.read_only {
            self.status =
                "Feedback panel is read-only; reviewer feedback was not applied.".to_string();
            return 0;
        }

        let mut applied = 0;
        for item in feedback {
            let Some(comment) = non_empty_feedback_value(&item.note) else {
                continue;
            };
            let Some(field) = feedback_panel_field_for_category(panel, &item.category) else {
                continue;
            };
            let existing_rating = panel
                .draft
                .entries
                .get(field.key)
                .map(|entry| entry.rating)
                .unwrap_or(1);
            let rating = reviewer_feedback_rating(Some(&item.score)).unwrap_or(existing_rating);
            let field_key = field.key.to_string();
            panel.replace_entry(
                &field_key,
                FeedbackRubricEntry {
                    rating,
                    comment: truncate_feedback_comment(comment),
                },
            );
            applied += 1;
        }

        if applied == 0 {
            self.status =
                "No matching non-empty reviewer feedback was available to apply.".to_string();
            return 0;
        }

        panel.mark_save_pending();
        panel.open = true;
        self.feedback_draft_revision = self.feedback_draft_revision.saturating_add(1);
        self.status = format!(
            "Applied reviewer feedback to {applied} Babel feedback field(s); saving draft..."
        );
        self.update_feedback_submit_metadata();
        self.queue_feedback_draft_save_request();
        applied
    }

    pub(super) fn mark_feedback_draft_saved(
        &mut self,
        token: FeedbackDraftSaveToken,
        last_saved: String,
    ) {
        if token.workspace_generation != self.workspace_generation || self.feedback_panel.is_none()
        {
            return;
        }
        let Some(acknowledged) = self.in_flight_feedback_draft_save_request(token) else {
            return;
        };
        self.remove_feedback_draft_save_request(&acknowledged);
        let current = self.current_feedback_draft_save_request();

        if current.as_ref() == Some(&acknowledged) {
            let Some(panel) = self.feedback_panel.as_mut() else {
                return;
            };
            panel.mark_saved(last_saved);
            self.status = "Feedback draft saved.".to_string();
        } else {
            let Some(panel) = self.feedback_panel.as_mut() else {
                return;
            };
            panel.mark_save_pending();
            self.status = "Feedback changed while saving; saving the latest draft...".to_string();
            self.queue_feedback_draft_save_request();
        }
        self.update_feedback_submit_metadata();
    }

    pub(super) fn fail_feedback_draft_save(
        &mut self,
        token: FeedbackDraftSaveToken,
        error: String,
    ) {
        if token.workspace_generation != self.workspace_generation || self.feedback_panel.is_none()
        {
            return;
        }
        let Some(acknowledged) = self.in_flight_feedback_draft_save_request(token) else {
            return;
        };
        self.remove_feedback_draft_save_request(&acknowledged);
        let current = self.current_feedback_draft_save_request();

        if current.as_ref() == Some(&acknowledged) {
            let Some(panel) = self.feedback_panel.as_mut() else {
                return;
            };
            panel.mark_save_failed(error.clone());
            self.status = format!("Feedback draft save failed: {error}");
        } else {
            let Some(panel) = self.feedback_panel.as_mut() else {
                return;
            };
            panel.mark_save_pending();
            self.status =
                format!("Feedback draft save failed ({error}); saving the latest draft...");
            self.queue_feedback_draft_save_request();
        }
        self.update_feedback_submit_metadata();
    }

    pub(super) fn update_feedback_submit_metadata(&mut self) {
        let Some(panel) = self.feedback_panel.as_ref() else {
            return;
        };
        let updated = if let Ok(workflow) = &mut self.workflow {
            workflow.set_feedback_submit_state(
                panel.is_loading
                    || panel.bootstrap_error.is_some()
                    || panel.is_saving
                    || panel.save_has_error,
                !panel.is_ready(),
            );
            true
        } else {
            false
        };
        if updated {
            self.invalidate_render_caches_for_project_metadata();
        }
    }

    fn queue_feedback_draft_save_request(&mut self) {
        let Some(request) = self.current_feedback_draft_save_request() else {
            return;
        };
        if self
            .dispatched_babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::SaveFeedbackDraft(_)))
        {
            return;
        }

        self.babel_service_requests
            .retain(|request| !matches!(request, BabelServiceRequest::SaveFeedbackDraft(_)));
        self.babel_service_requests.push(request);
    }

    fn current_feedback_draft_save_request(&self) -> Option<BabelServiceRequest> {
        let panel = self.feedback_panel.as_ref()?;
        compute_feedback_draft_save_request(FeedbackSaveRequestInput {
            enabled: !panel.read_only,
            form_step_response_id: panel.form_step_response_id.clone(),
            draft_feedback: panel.draft.clone(),
            inputs: panel.inputs.clone(),
            rubric_fields: Some(panel.rubric_fields.clone()),
            rubric_flag_names: panel.rubric_flag_names.clone(),
            has_other_feedback: Some(panel.has_other_feedback),
        })
        .map(|payload| {
            BabelServiceRequest::SaveFeedbackDraft(FeedbackDraftSaveServiceRequest {
                payload,
                token: FeedbackDraftSaveToken {
                    workspace_generation: self.workspace_generation,
                    revision: self.feedback_draft_revision,
                },
            })
        })
    }

    fn in_flight_feedback_draft_save_request(
        &self,
        token: FeedbackDraftSaveToken,
    ) -> Option<BabelServiceRequest> {
        self.dispatched_babel_service_requests
            .iter()
            .find(|request| {
                matches!(
                    request,
                    BabelServiceRequest::SaveFeedbackDraft(request) if request.token == token
                )
            })
            .cloned()
    }

    fn remove_feedback_draft_save_request(&mut self, acknowledged: &BabelServiceRequest) {
        self.babel_service_requests
            .retain(|request| request != acknowledged);
        self.dispatched_babel_service_requests
            .retain(|request| request != acknowledged);
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
    if let Some(field) = panel.rubric_fields.iter().copied().find(|field| {
        normalized_feedback_label(field.label) == normalized_category
            || normalized_feedback_label(field.key) == normalized_category
    }) {
        return Some(field);
    }

    let mut alias_matches = panel.rubric_fields.iter().copied().filter(|field| {
        feedback_category_aliases(field.key).contains(&normalized_category.as_str())
    });
    let field = alias_matches.next()?;
    if alias_matches.next().is_some() {
        return None;
    }
    Some(field)
}

fn feedback_category_aliases(field_key: &str) -> &'static [&'static str] {
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

fn normalized_editor_text(content: &text_editor::Content) -> String {
    let mut text = content.text();
    if text.ends_with('\n') {
        text.pop();
    }
    text
}

fn feedback_comment_contents(
    rubric_fields: &[FeedbackRubricField],
    draft: &FeedbackDraft,
) -> BTreeMap<String, text_editor::Content> {
    rubric_fields
        .iter()
        .filter_map(|field| {
            draft.entries.get(field.key).map(|entry| {
                (
                    field.key.to_string(),
                    text_editor::Content::with_text(&entry.comment),
                )
            })
        })
        .collect()
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
