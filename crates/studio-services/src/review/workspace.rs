//! UI-free Reviewer session workspace: selection, comment drafts, single-flight comment
//! autosave, and finalize-after-comments sequencing.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};
use studio_core::Project;

use super::feedback::reviewer_feedback_for_card;
use super::model::{
    FinalizeOutcome, GeneratedReview, ReviewerCard, ReviewerFeedback, ReviewerFinalize,
    ReviewerSession, ReviewerTemplate,
};
use super::payload::{build_reviewer_analytics_payload, reviewer_session_id_from_project};

/// A comment save to send with [`super::ReviewBackend::update_comments`].
#[derive(Debug, Clone, PartialEq)]
pub struct CommentSave {
    pub session_id: String,
    pub body: Value,
}

/// What the UI must send next after a workspace transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewerNext {
    Idle,
    /// Call [`ReviewerWorkspace::begin_comment_save`] and send it.
    SaveComments,
    /// Send [`super::ReviewBackend::finalize_session`] and settle with
    /// [`ReviewerWorkspace::finish_finalize`].
    Finalize {
        session_id: String,
        mode: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ReviewerWorkspace {
    session_id: String,
    session: Option<ReviewerSession>,
    session_comment: String,
    card_id: String,
    card_comment: String,
    template_id: String,
    proposal_id: String,
    comments_dirty: bool,
    comment_save_failed: bool,
    comment_save_in_flight: Option<CommentSave>,
    pending_finalize: Option<String>,
    finalize_in_flight: Option<(String, String)>,
    pub generated_review: Option<GeneratedReview>,
    pub finalize_result: Option<ReviewerFinalize>,
    pub template_results: Vec<ReviewerTemplate>,
}

impl ReviewerWorkspace {
    /// Fresh workspace for a loaded task, hydrating the persisted session id.
    pub fn from_project(project: &Project) -> Self {
        Self {
            session_id: reviewer_session_id_from_project(project),
            ..Self::default()
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn session(&self) -> Option<&ReviewerSession> {
        self.session.as_ref()
    }

    pub fn session_comment(&self) -> &str {
        &self.session_comment
    }

    pub fn selected_card_id(&self) -> &str {
        &self.card_id
    }

    pub fn card_comment(&self) -> &str {
        &self.card_comment
    }

    pub fn template_id(&self) -> &str {
        &self.template_id
    }

    pub fn proposal_id(&self) -> &str {
        &self.proposal_id
    }

    pub fn comments_dirty(&self) -> bool {
        self.comments_dirty
    }

    /// A failed save disables autosave until the user saves manually.
    pub fn comment_save_failed(&self) -> bool {
        self.comment_save_failed
    }

    pub fn finalize_pending(&self) -> Option<&str> {
        self.pending_finalize.as_deref()
    }

    pub fn comment_save_in_flight(&self) -> bool {
        self.comment_save_in_flight
            .as_ref()
            .is_some_and(|save| save.session_id == self.session_id)
    }

    /// Typed session id. Switching sessions drops unsaved-comment and pending-finalize state;
    /// persist the result with [`super::persist_reviewer_session_id`].
    pub fn set_session_id(&mut self, session_id: &str) {
        let session_id = session_id.trim();
        self.cancel_persistence_if_changed(session_id);
        self.session_id = session_id.to_string();
    }

    pub fn set_session_comment(&mut self, comment: &str) {
        if self.session_comment == comment {
            return;
        }
        self.session_comment = comment.to_string();
        if let Some(session) = &mut self.session {
            session.session_comment = comment.to_string();
        }
        self.comments_dirty = true;
    }

    /// Comment for the selected card.
    pub fn set_card_comment(&mut self, comment: &str) {
        if self.card_comment == comment {
            return;
        }
        self.card_comment = comment.to_string();
        if let Some(card) = self.session.as_mut().and_then(|session| {
            session
                .cards
                .iter_mut()
                .find(|card| card.id == self.card_id)
        }) {
            card.comment = comment.to_string();
        }
        self.comments_dirty = true;
    }

    /// Selects a card; a card from the loaded session syncs its comment and matched template.
    pub fn select_card(&mut self, card_id: &str) {
        self.card_id = card_id.trim().to_string();
        if let Some(card) = self
            .session
            .as_ref()
            .and_then(|session| session.cards.iter().find(|card| card.id == self.card_id))
        {
            self.card_comment = card.comment.clone();
            self.template_id = card.matched_template_id.clone().unwrap_or_default();
        }
    }

    pub fn set_template_id(&mut self, template_id: &str) {
        self.template_id = template_id.trim().to_string();
    }

    pub fn set_proposal_id(&mut self, proposal_id: &str) {
        self.proposal_id = proposal_id.trim().to_string();
    }

    /// `{ sessionComment, cardComments: { <cardId>: comment } }` covering every session card
    /// plus the selected card when it is not in the session.
    pub fn comments_body(&self) -> Value {
        let mut card_comments = Map::new();
        for card in self.session.iter().flat_map(|session| &session.cards) {
            card_comments.insert(card.id.clone(), json!(card.comment));
        }
        let selected = self.card_id.trim();
        if !selected.is_empty() && !card_comments.contains_key(selected) {
            card_comments.insert(selected.to_string(), json!(self.card_comment));
        }
        json!({ "sessionComment": self.session_comment, "cardComments": card_comments })
    }

    /// Autosave tick (old cadence: 500 ms) should save now.
    pub fn should_autosave(&self) -> bool {
        self.comments_dirty
            && !self.comment_save_failed
            && !self.session_id.is_empty()
            && !self.comment_save_in_flight()
    }

    /// Starts a manual or automatic comment save (clears a previous failure).
    pub fn begin_comment_save(&mut self) -> Result<CommentSave> {
        let session_id = required_field("session ID", &self.session_id)?;
        if self.comment_save_in_flight() {
            bail!("Reviewer comments are already being saved.");
        }
        let save = CommentSave {
            session_id,
            body: self.comments_body(),
        };
        self.comment_save_failed = false;
        self.comment_save_in_flight = Some(save.clone());
        Ok(save)
    }

    /// Settles a comment save. Edits made while it was in flight stay dirty and are merged into
    /// the returned session; a failure keeps the drafts dirty and stops autosave. Continues a
    /// pending finalize once every comment is acknowledged.
    pub fn finish_comment_save(
        &mut self,
        save: &CommentSave,
        result: Result<ReviewerSession>,
    ) -> ReviewerNext {
        if self.comment_save_in_flight.as_ref() == Some(save) {
            self.comment_save_in_flight = None;
        }
        if save.session_id != self.session_id {
            return ReviewerNext::Idle;
        }
        let mut session = match result {
            Ok(session) if session.session_id == save.session_id => session,
            Ok(_) => return ReviewerNext::Idle,
            Err(_) => {
                self.comments_dirty = true;
                self.comment_save_failed = true;
                return ReviewerNext::Idle;
            }
        };
        self.comment_save_failed = false;
        if save.body == self.comments_body() {
            self.comments_dirty = false;
        } else {
            self.merge_local_comments_into(&mut session);
            self.comments_dirty = true;
        }
        self.replace_session(session);
        match self.pending_finalize.clone() {
            Some(_) if self.comments_dirty => ReviewerNext::SaveComments,
            Some(mode) => self.begin_finalize(&mode),
            None => ReviewerNext::Idle,
        }
    }

    /// Finalize as `apply` or `skip`; unsaved or in-flight comments are saved first.
    pub fn request_finalize(&mut self, mode: &str) -> Result<ReviewerNext> {
        required_field("session ID", &self.session_id)?;
        if self.comments_dirty || self.comment_save_in_flight() {
            self.pending_finalize = Some(mode.to_string());
            return Ok(if self.comment_save_in_flight() {
                ReviewerNext::Idle
            } else {
                ReviewerNext::SaveComments
            });
        }
        Ok(self.begin_finalize(mode))
    }

    /// Settles a finalize request. `None` when it is stale (session switched or superseded);
    /// otherwise the outcome, with the finalize/generated previews stored on success.
    pub fn finish_finalize(
        &mut self,
        session_id: &str,
        mode: &str,
        result: Result<FinalizeOutcome>,
    ) -> Option<Result<ReviewerFinalize>> {
        let in_flight = self
            .finalize_in_flight
            .as_ref()
            .is_some_and(|(id, in_flight_mode)| id == session_id && in_flight_mode == mode);
        if in_flight {
            self.finalize_in_flight = None;
        }
        if !in_flight
            || self.session_id != session_id
            || self.pending_finalize.as_deref() != Some(mode)
        {
            return None;
        }
        self.pending_finalize = None;
        Some(result.map(|outcome| {
            self.generated_review = Some(outcome.generated);
            self.finalize_result = Some(outcome.finalize.clone());
            outcome.finalize
        }))
    }

    /// Adopts a session response (create/load/template/suggestion calls). Local comment drafts
    /// for the same session survive while dirty or saving; the selected card is kept when present.
    pub fn replace_session(&mut self, mut session: ReviewerSession) {
        self.cancel_persistence_if_changed(&session.session_id);
        if session.session_id == self.session_id
            && (self.comments_dirty || self.comment_save_in_flight())
        {
            self.merge_local_comments_into(&mut session);
        }
        self.session_id = session.session_id.clone();
        self.session_comment = session.session_comment.clone();
        match session
            .cards
            .iter()
            .find(|card| card.id == self.card_id)
            .or_else(|| session.cards.first())
        {
            Some(card) => {
                self.card_id = card.id.clone();
                self.card_comment = card.comment.clone();
                self.template_id = card.matched_template_id.clone().unwrap_or_default();
            }
            None => {
                self.card_id.clear();
                self.card_comment.clear();
                self.template_id.clear();
            }
        }
        self.proposal_id = session
            .suggestions
            .first()
            .map(|suggestion| suggestion.proposal_id.clone())
            .unwrap_or_default();
        self.session = Some(session);
    }

    /// Selected card plus its matched generated feedback (generated cards first, then session
    /// cards) for "Apply card to feedback".
    pub fn selected_card_feedback(&self) -> Option<(&ReviewerCard, Option<&ReviewerFeedback>)> {
        let generated_feedback = self
            .generated_review
            .as_ref()
            .map(|review| review.feedback.as_slice())
            .unwrap_or_default();
        self.generated_review
            .iter()
            .flat_map(|review| &review.cards)
            .chain(self.session.iter().flat_map(|session| &session.cards))
            .find(|card| card.id == self.card_id)
            .map(|card| (card, reviewer_feedback_for_card(card, generated_feedback)))
    }

    /// `(session_id, card_id, template_id)` for "Set template match".
    pub fn template_match_target(&self) -> Result<(String, String, String)> {
        Ok((
            required_field("session ID", &self.session_id)?,
            required_field("card ID", &self.card_id)?,
            required_field("template ID", &self.template_id)?,
        ))
    }

    /// `(session_id, card_id)` for "Clear template match".
    pub fn card_target(&self) -> Result<(String, String)> {
        Ok((
            required_field("session ID", &self.session_id)?,
            required_field("card ID", &self.card_id)?,
        ))
    }

    /// `(session_id, proposal_id)` for accepting/rejecting a template suggestion.
    pub fn suggestion_target(&self) -> Result<(String, String)> {
        Ok((
            required_field("session ID", &self.session_id)?,
            required_field("proposal ID", &self.proposal_id)?,
        ))
    }

    pub fn require_session_id(&self) -> Result<String> {
        required_field("session ID", &self.session_id)
    }

    /// Analytics payload including this workspace's session comments and generated review.
    pub fn analytics_payload(&self, project: &Project) -> Result<Value> {
        build_reviewer_analytics_payload(
            project,
            self.session.as_ref(),
            self.generated_review.as_ref(),
        )
    }

    fn begin_finalize(&mut self, mode: &str) -> ReviewerNext {
        if self.finalize_in_flight.is_some() {
            return ReviewerNext::Idle;
        }
        self.pending_finalize = Some(mode.to_string());
        self.finalize_in_flight = Some((self.session_id.clone(), mode.to_string()));
        ReviewerNext::Finalize {
            session_id: self.session_id.clone(),
            mode: mode.to_string(),
        }
    }

    fn cancel_persistence_if_changed(&mut self, next_session_id: &str) {
        if self.session_id == next_session_id {
            return;
        }
        self.comments_dirty = false;
        self.comment_save_failed = false;
        self.pending_finalize = None;
        self.finalize_in_flight = None;
    }

    fn merge_local_comments_into(&self, session: &mut ReviewerSession) {
        if session.session_id != self.session_id {
            return;
        }
        session.session_comment = self.session_comment.clone();
        let body = self.comments_body();
        let Some(comments) = body.get("cardComments").and_then(Value::as_object) else {
            return;
        };
        for card in &mut session.cards {
            if let Some(comment) = comments.get(&card.id).and_then(Value::as_str) {
                card.comment = comment.to_string();
            }
        }
    }
}

/// `Reviewer request blocked: missing <label>.` for blank values; trimmed value otherwise.
pub fn required_field(label: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Reviewer request blocked: missing {label}."));
    }
    Ok(trimmed.to_string())
}
