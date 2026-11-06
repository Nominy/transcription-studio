//! UI-free interactive Magic Review workspace: the open session, independent per-card state
//! (expanded, comment draft, template search), single-flight comment autosave, and
//! finalize-after-comments sequencing.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow, bail};
use exaudio_babel_domain::i18n::localized;
use serde_json::{Map, Value, json};
use studio_core::Project;

use super::model::{
    FinalizeOutcome, GeneratedReview, ReviewerFinalize, ReviewerSession, ReviewerTemplate,
};
use super::payload::build_reviewer_analytics_payload;

/// Results per template search (the Review Helper's limit).
pub const TEMPLATE_SEARCH_LIMIT: usize = 8;

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

/// One card's inline template search.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TemplateSearch {
    pub query: String,
    pub results: Vec<ReviewerTemplate>,
    pub loading: bool,
    pub error: Option<String>,
    generation: u64,
}

/// A template search to send; settle with [`ReviewerWorkspace::finish_template_search`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSearchRequest {
    pub session_id: String,
    pub card_id: String,
    pub query: String,
    pub generation: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ReviewerWorkspace {
    session: Option<ReviewerSession>,
    session_comment: String,
    card_comments: BTreeMap<String, String>,
    expanded: BTreeSet<String>,
    template_search: BTreeMap<String, TemplateSearch>,
    search_generation: u64,
    comments_dirty: bool,
    comment_save_failed: bool,
    comment_save_in_flight: Option<CommentSave>,
    pending_finalize: Option<String>,
    finalize_in_flight: Option<(String, String)>,
    /// Fast-mode result (echoed into analytics).
    pub generated_review: Option<GeneratedReview>,
    pub finalize_result: Option<ReviewerFinalize>,
}

impl ReviewerWorkspace {
    pub fn session_id(&self) -> &str {
        self.session
            .as_ref()
            .map_or("", |session| session.session_id.as_str())
    }

    pub fn session(&self) -> Option<&ReviewerSession> {
        self.session.as_ref()
    }

    /// Drops the session and every per-card state (close, refresh, task change).
    pub fn clear(&mut self) {
        let generated_review = self.generated_review.take();
        *self = Self {
            generated_review,
            search_generation: self.search_generation,
            ..Self::default()
        };
    }

    pub fn session_comment(&self) -> &str {
        &self.session_comment
    }

    pub fn card_comment(&self, card_id: &str) -> &str {
        self.card_comments.get(card_id).map_or("", String::as_str)
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
            .is_some_and(|save| save.session_id == self.session_id())
    }

    pub fn set_session_comment(&mut self, comment: &str) {
        if self.session.is_none() || self.session_comment == comment {
            return;
        }
        self.session_comment = comment.to_string();
        self.comments_dirty = true;
    }

    pub fn set_card_comment(&mut self, card_id: &str, comment: &str) {
        let known = self
            .session
            .as_ref()
            .is_some_and(|session| session.cards.iter().any(|card| card.id == card_id));
        if !known || self.card_comment(card_id) == comment {
            return;
        }
        self.card_comments
            .insert(card_id.to_string(), comment.to_string());
        self.comments_dirty = true;
    }

    pub fn is_expanded(&self, card_id: &str) -> bool {
        self.expanded.contains(card_id)
    }

    pub fn toggle_expanded(&mut self, card_id: &str) {
        if !self.expanded.remove(card_id) {
            self.expanded.insert(card_id.to_string());
        }
    }

    /// Expand All / Collapse All.
    pub fn set_all_expanded(&mut self, expanded: bool) {
        self.expanded = if expanded {
            self.session
                .iter()
                .flat_map(|session| &session.cards)
                .map(|card| card.id.clone())
                .collect()
        } else {
            BTreeSet::new()
        };
    }

    /// The card's open template search (`None` when the inline editor is closed).
    pub fn template_search(&self, card_id: &str) -> Option<&TemplateSearch> {
        self.template_search.get(card_id)
    }

    /// Opens ("Match template"/"Change template") or closes ("Cancel") the card's search.
    pub fn set_template_search_open(&mut self, card_id: &str, open: bool) {
        if open {
            self.template_search.entry(card_id.to_string()).or_default();
        } else {
            self.template_search.remove(card_id);
        }
    }

    /// Records the query typed into the card's search. Returns the request to send, or `None`
    /// for a blank query (results cleared). Older in-flight results for the card are ignored.
    pub fn begin_template_search(
        &mut self,
        card_id: &str,
        query: &str,
    ) -> Option<TemplateSearchRequest> {
        let session_id = self.session_id().to_string();
        let search = self.template_search.get_mut(card_id)?;
        self.search_generation += 1;
        search.generation = self.search_generation;
        search.query = query.to_string();
        search.error = None;
        if query.trim().is_empty() {
            search.results.clear();
            search.loading = false;
            return None;
        }
        search.loading = true;
        Some(TemplateSearchRequest {
            session_id,
            card_id: card_id.to_string(),
            query: query.trim().to_string(),
            generation: search.generation,
        })
    }

    /// Settles a search; stale results (newer query, closed search, other session) are dropped.
    pub fn finish_template_search(
        &mut self,
        request: &TemplateSearchRequest,
        result: Result<Vec<ReviewerTemplate>>,
    ) -> bool {
        if request.session_id != self.session_id() {
            return false;
        }
        let Some(search) = self.template_search.get_mut(&request.card_id) else {
            return false;
        };
        if search.generation != request.generation {
            return false;
        }
        search.loading = false;
        match result {
            Ok(results) => {
                search.results = results;
                search.error = None;
            }
            Err(error) => {
                search.results.clear();
                search.error = Some(format!("{error:#}"));
            }
        }
        true
    }

    /// `{ sessionComment, cardComments: { <cardId>: comment } }`. The backend merges card
    /// comments (an empty one deletes) and logs every listed card as commented, so the body
    /// lists the cards with a comment plus the cards whose saved comment was cleared.
    pub fn comments_body(&self) -> Value {
        let card_comments: Map<String, Value> = self
            .session
            .iter()
            .flat_map(|session| &session.cards)
            .filter_map(|card| {
                let comment = self.card_comment(&card.id);
                (!comment.trim().is_empty() || !card.comment.trim().is_empty())
                    .then(|| (card.id.clone(), json!(comment)))
            })
            .collect();
        json!({ "sessionComment": self.session_comment, "cardComments": card_comments })
    }

    /// Autosave tick (500 ms) should save now.
    pub fn should_autosave(&self) -> bool {
        self.comments_dirty
            && !self.comment_save_failed
            && self.session.is_some()
            && !self.comment_save_in_flight()
    }

    /// Starts a manual or automatic comment save (clears a previous failure).
    pub fn begin_comment_save(&mut self) -> Result<CommentSave> {
        let session_id = required_session_id(self.session_id())?;
        if self.comment_save_in_flight() {
            bail!(
                "{}",
                localized(
                    "Still saving the previous comments.",
                    "Предыдущие комментарии ещё сохраняются.",
                )
            );
        }
        let save = CommentSave {
            session_id,
            body: self.comments_body(),
        };
        self.comment_save_failed = false;
        self.comment_save_in_flight = Some(save.clone());
        Ok(save)
    }

    /// Settles a comment save. Edits made while it was in flight stay dirty; a failure keeps
    /// the drafts dirty and stops autosave. Continues a pending finalize once every comment is
    /// acknowledged.
    pub fn finish_comment_save(
        &mut self,
        save: &CommentSave,
        result: Result<ReviewerSession>,
    ) -> ReviewerNext {
        if self.comment_save_in_flight.as_ref() == Some(save) {
            self.comment_save_in_flight = None;
        }
        if save.session_id != self.session_id() {
            return ReviewerNext::Idle;
        }
        let session = match result {
            Ok(session) if session.session_id == save.session_id => session,
            Ok(_) => return ReviewerNext::Idle,
            Err(_) => {
                self.comments_dirty = true;
                self.comment_save_failed = true;
                self.pending_finalize = None;
                return ReviewerNext::Idle;
            }
        };
        self.comment_save_failed = false;
        self.comments_dirty = save.body != self.comments_body();
        self.replace_session(session);
        match self.pending_finalize.clone() {
            Some(_) if self.comments_dirty => ReviewerNext::SaveComments,
            Some(mode) => self.begin_finalize(&mode),
            None => ReviewerNext::Idle,
        }
    }

    /// Finalize as `apply` or `skip`; unsaved or in-flight comments are saved first.
    pub fn request_finalize(&mut self, mode: &str) -> Result<ReviewerNext> {
        required_session_id(self.session_id())?;
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

    /// Settles a finalize request. `None` when it is stale (session switched or superseded).
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
            || self.session_id() != session_id
            || self.pending_finalize.as_deref() != Some(mode)
        {
            return None;
        }
        self.pending_finalize = None;
        Some(result.and_then(|outcome| {
            if outcome.finalize.feedback.is_empty() {
                bail!(
                    "{}",
                    localized(
                        "No feedback came back. Try again.",
                        "Отзыв не получен. Попробуйте снова.",
                    )
                );
            }
            self.generated_review = Some(outcome.generated);
            self.finalize_result = Some(outcome.finalize.clone());
            Ok(outcome.finalize)
        }))
    }

    /// Adopts a session response. Local comment drafts for the same session survive while
    /// dirty or saving; a different session resets every per-card state.
    pub fn replace_session(&mut self, session: ReviewerSession) {
        let same = session.session_id == self.session_id();
        let keep_local = same && (self.comments_dirty || self.comment_save_in_flight());
        if !same {
            let generated_review = self.generated_review.take();
            *self = Self {
                generated_review,
                search_generation: self.search_generation,
                ..Self::default()
            };
        }
        if !keep_local {
            self.session_comment = session.session_comment.clone();
            self.card_comments = session
                .cards
                .iter()
                .map(|card| (card.id.clone(), card.comment.clone()))
                .collect();
        }
        let ids: BTreeSet<&str> = session.cards.iter().map(|card| card.id.as_str()).collect();
        self.expanded.retain(|id| ids.contains(id.as_str()));
        self.template_search
            .retain(|id, _| ids.contains(id.as_str()));
        self.card_comments.retain(|id, _| ids.contains(id.as_str()));
        self.session = Some(session);
    }

    /// Analytics payload including this workspace's session and generated review.
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
        let session_id = self.session_id().to_string();
        self.pending_finalize = Some(mode.to_string());
        self.finalize_in_flight = Some((session_id.clone(), mode.to_string()));
        ReviewerNext::Finalize {
            session_id,
            mode: mode.to_string(),
        }
    }
}

/// The session id for a reviewer request; errors when the session is gone.
pub fn required_session_id(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "{}",
            localized(
                "The review session expired. Open Magic Review again.",
                "Сессия ревью истекла. Откройте Magic Review снова.",
            )
        ));
    }
    Ok(trimmed.to_string())
}
