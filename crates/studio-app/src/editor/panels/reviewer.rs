//! Reviewer backend operations on the shared [`ReviewerWorkspace`] (L2 review tasks):
//! generate review ("Magic Review"), sessions, comments (autosaved every 500 ms), templates,
//! suggestions, finalize → feedback form, analytics and history.

use std::time::Duration;

use gpui_kit::*;
use studio_services::review::{
    CommentSave, ReviewBackend, ReviewerNext, ReviewerSession, build_reviewer_payload,
    parse_history_query, persist_reviewer_session_id, reviewer_capability_available,
};

use super::task::{Drawer, TaskPanels};
use crate::services::{AppServices, spawn_service_in};

const AUTOSAVE_INTERVAL: Duration = Duration::from_millis(500);
const L2_ONLY: &str = "Reviewer is only available for L2 review tasks.";

impl TaskPanels {
    pub fn reviewer_is_pending(&self, operation: &str) -> bool {
        self.reviewer_pending.contains(&operation)
    }

    pub fn reviewer_error(&self) -> Option<&str> {
        self.reviewer_error.as_deref()
    }

    pub fn reviewer_available(&self, cx: &App) -> bool {
        reviewer_capability_available(self.model.read(cx).doc().project())
    }

    fn backend(cx: &App) -> ReviewBackend {
        cx.global::<AppServices>().review.clone()
    }

    /// Starts the 500 ms reviewer-comment autosave tick for L2 tasks.
    pub(super) fn start_reviewer_autosave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.reviewer_available(cx) {
            return;
        }
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSAVE_INTERVAL).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    if this.reviewer.should_autosave() {
                        this.save_reviewer_comments(window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        self.reviewer_tasks.push(task);
    }

    /// Runs one reviewer request: dedupes by `operation`, shows `queued`, reports failures as
    /// `{failure}: {error}`.
    // Every reviewer call differs in exactly these pieces; a struct would only rename them.
    #[allow(clippy::too_many_arguments)]
    fn run_reviewer<R: Send + 'static>(
        &mut self,
        operation: &'static str,
        queued: &str,
        failure: &'static str,
        request: impl Future<Output = anyhow::Result<R>> + Send + 'static,
        on_success: impl FnOnce(&mut Self, R, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reviewer_is_pending(operation) {
            self.status(format!("{queued} Request is already pending."), cx);
            return;
        }
        self.reviewer_pending.push(operation);
        self.reviewer_error = None;
        self.status(queued.to_string(), cx);
        let generation = self.generation;
        let task = spawn_service_in(window, cx, request, move |this, result, window, cx| {
            this.reviewer_pending
                .retain(|pending| *pending != operation);
            if this.generation != generation {
                return;
            }
            match result {
                Ok(value) => on_success(this, value, window, cx),
                Err(error) => {
                    let error = format!("{error:#}");
                    this.status(format!("{failure}: {error}"), cx);
                    this.reviewer_error = Some(error);
                }
            }
            cx.notify();
        });
        self.reviewer_tasks.retain(|task| !task.is_ready());
        self.reviewer_tasks.push(task);
        cx.notify();
    }

    /// L2 guard + pending-draft commit + reviewer payload for generate/create.
    fn reviewer_payload(
        &mut self,
        action: &str,
        cx: &mut Context<Self>,
    ) -> Option<serde_json::Value> {
        if !self.reviewer_available(cx) {
            self.status(format!("{action} unavailable: {L2_ONLY}"), cx);
            return None;
        }
        self.model.update(cx, |model, cx| {
            if let Ok(outcome) = model.update_doc(cx, |doc| doc.commit_pending_drafts("Reviewer")) {
                model.apply_outcome(outcome, cx);
            }
        });
        match build_reviewer_payload(self.model.read(cx).doc().project()) {
            Ok(payload) => Some(payload),
            Err(error) => {
                self.status(format!("{action} unavailable: {error:#}"), cx);
                None
            }
        }
    }

    fn guard_l2(&mut self, action: &str, cx: &mut Context<Self>) -> bool {
        let available = self.reviewer_available(cx);
        if !available {
            self.status(format!("{action} unavailable: {L2_ONLY}"), cx);
        }
        available
    }

    /// "Magic Review" / "Generate Review": generated feedback is applied to the feedback form.
    pub fn generate_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(payload) = self.reviewer_payload("Reviewer generate", cx) else {
            return;
        };
        let backend = Self::backend(cx);
        self.run_reviewer(
            "generate",
            "Reviewer generate queued...",
            "Reviewer generate failed",
            async move { backend.generate_review(payload).await },
            |this, review, window, cx| {
                let feedback = review.feedback.clone();
                this.reviewer.generated_review = Some(review);
                let applied = this.apply_reviewer_feedback(&feedback, window, cx);
                if applied > 0 {
                    this.status(
                        format!(
                            "Generated review automatically applied {applied} of {} feedback item(s); saving draft...",
                            feedback.len()
                        ),
                        cx,
                    );
                }
            },
            window,
            cx,
        );
    }

    fn adopt_session(&mut self, session: ReviewerSession, cx: &mut Context<Self>) {
        let cards = session.cards.len();
        self.reviewer.replace_session(session);
        self.persist_session_id(cx);
        self.status(format!("Loaded review session with {cards} cards."), cx);
    }

    fn persist_session_id(&self, cx: &mut App) {
        let session_id = self.reviewer.session_id().to_string();
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.with_workflow(|workflow| persist_reviewer_session_id(workflow, &session_id));
            });
        });
    }

    pub fn create_reviewer_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(payload) = self.reviewer_payload("Reviewer session", cx) else {
            return;
        };
        let backend = Self::backend(cx);
        self.run_reviewer(
            "session",
            "Reviewer session queued...",
            "Reviewer session failed",
            async move { backend.create_session(payload).await },
            |this, session, _, cx| this.adopt_session(session, cx),
            window,
            cx,
        );
    }

    pub fn load_reviewer_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.guard_l2("Reviewer session", cx) {
            return;
        }
        let session_id = match self.reviewer.require_session_id() {
            Ok(id) => id,
            Err(error) => return self.status(format!("{error:#}"), cx),
        };
        let backend = Self::backend(cx);
        self.run_reviewer(
            "session",
            "Loading review session...",
            "Reviewer session failed",
            async move { backend.get_session(&session_id).await },
            |this, session, _, cx| this.adopt_session(session, cx),
            window,
            cx,
        );
    }

    pub fn set_reviewer_session_id(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.reviewer.session_id() == session_id.trim() {
            return;
        }
        self.reviewer.set_session_id(session_id);
        self.persist_session_id(cx);
        cx.notify();
    }

    /// Manual "Save Comments" and the autosave tick.
    pub fn save_reviewer_comments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.guard_l2("Reviewer comments", cx) {
            return;
        }
        let save = match self.reviewer.begin_comment_save() {
            Ok(save) => save,
            Err(error) => return self.status(format!("{error:#}"), cx),
        };
        self.status("Saving review comments...", cx);
        let backend = Self::backend(cx);
        let generation = self.generation;
        let request = save.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move {
                backend
                    .update_comments(&request.session_id, request.body)
                    .await
            },
            move |this, result, window, cx| {
                if this.generation == generation {
                    this.finish_comment_save(&save, result, window, cx);
                }
            },
        );
        self.reviewer_tasks.retain(|task| !task.is_ready());
        self.reviewer_tasks.push(task);
        cx.notify();
    }

    fn finish_comment_save(
        &mut self,
        save: &CommentSave,
        result: anyhow::Result<ReviewerSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let error = result.as_ref().err().map(|error| format!("{error:#}"));
        let next = self.reviewer.finish_comment_save(save, result);
        match error {
            Some(error) => {
                self.status(
                    format!(
                        "Reviewer comments save failed: {error}. Drafts remain unsaved; use Save comments to retry."
                    ),
                    cx,
                );
                self.reviewer_error = Some(error);
            }
            None if !self.reviewer.comments_dirty() => {
                self.status("Session and card comments are saved.", cx);
            }
            None => {}
        }
        self.follow_reviewer_next(next, window, cx);
        cx.notify();
    }

    fn follow_reviewer_next(
        &mut self,
        next: ReviewerNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match next {
            ReviewerNext::Idle => {}
            ReviewerNext::SaveComments => self.save_reviewer_comments(window, cx),
            ReviewerNext::Finalize { session_id, mode } => {
                self.send_finalize(session_id, mode, window, cx);
            }
        }
    }

    /// "Apply Review" (`apply`) / "Skip Review" (`skip`); comments are saved first.
    pub fn finalize_review(
        &mut self,
        mode: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_l2("Reviewer finalize", cx) {
            return;
        }
        match self.reviewer.request_finalize(mode) {
            Ok(ReviewerNext::Finalize { session_id, mode }) => {
                self.send_finalize(session_id, mode, window, cx);
            }
            Ok(next) => {
                self.status(
                    format!("Saving all reviewer comments before finalizing as {mode}..."),
                    cx,
                );
                self.follow_reviewer_next(next, window, cx);
            }
            Err(error) => self.status(format!("{error:#}"), cx),
        }
        cx.notify();
    }

    fn send_finalize(
        &mut self,
        session_id: String,
        mode: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status("Finalizing review session...", cx);
        let backend = Self::backend(cx);
        let generation = self.generation;
        let (request_session, request_mode) = (session_id.clone(), mode.clone());
        let task = spawn_service_in(
            window,
            cx,
            async move {
                backend
                    .finalize_session(&request_session, &request_mode)
                    .await
            },
            move |this, result, window, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(result) = this.reviewer.finish_finalize(&session_id, &mode, result) else {
                    return;
                };
                match result {
                    Ok(finalize) => {
                        let applied = this.apply_reviewer_feedback(&finalize.feedback, window, cx);
                        if this.drawer == Drawer::Assistant {
                            this.drawer = if applied > 0 {
                                Drawer::Feedback
                            } else {
                                Drawer::Closed
                            };
                        }
                        this.status(
                            format!(
                                "Finalized review session as {mode}; applied {applied} of {} feedback items.",
                                finalize.feedback.len()
                            ),
                            cx,
                        );
                    }
                    Err(error) => {
                        let error = format!("{error:#}");
                        this.status(format!("Reviewer finalize failed: {error}"), cx);
                        this.reviewer_error = Some(error);
                    }
                }
                cx.notify();
            },
        );
        self.reviewer_tasks.retain(|task| !task.is_ready());
        self.reviewer_tasks.push(task);
    }

    pub fn search_reviewer_templates(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_l2("Template search", cx) {
            return;
        }
        let backend = Self::backend(cx);
        self.run_reviewer(
            "templates",
            "Searching review templates...",
            "Template search failed",
            async move { backend.search_templates(query.trim(), Some(20)).await },
            |this, templates, _, cx| {
                this.status(
                    format!("Loaded {} review template results.", templates.len()),
                    cx,
                );
                this.reviewer.template_results = templates;
            },
            window,
            cx,
        );
    }

    pub fn match_reviewer_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.guard_l2("Template match", cx) {
            return;
        }
        let (session_id, card_id, template_id) = match self.reviewer.template_match_target() {
            Ok(target) => target,
            Err(error) => return self.status(format!("{error:#}"), cx),
        };
        let backend = Self::backend(cx);
        self.run_reviewer(
            "template-match",
            "Setting review template match...",
            "Template match failed",
            async move {
                backend
                    .set_template_match(&session_id, &card_id, &template_id)
                    .await
            },
            |this, session, _, cx| this.adopt_session(session, cx),
            window,
            cx,
        );
    }

    pub fn clear_reviewer_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.guard_l2("Template match", cx) {
            return;
        }
        let (session_id, card_id) = match self.reviewer.card_target() {
            Ok(target) => target,
            Err(error) => return self.status(format!("{error:#}"), cx),
        };
        let backend = Self::backend(cx);
        self.run_reviewer(
            "template-match",
            "Clearing review template match...",
            "Template match failed",
            async move { backend.clear_template_match(&session_id, &card_id).await },
            |this, session, _, cx| this.adopt_session(session, cx),
            window,
            cx,
        );
    }

    pub fn suggest_reviewer_templates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.guard_l2("Template suggestions", cx) {
            return;
        }
        let session_id = match self.reviewer.require_session_id() {
            Ok(id) => id,
            Err(error) => return self.status(format!("{error:#}"), cx),
        };
        let backend = Self::backend(cx);
        self.run_reviewer(
            "suggestions",
            "Generating template suggestions...",
            "Template suggestions failed",
            async move { backend.generate_template_suggestions(&session_id).await },
            |this, session, _, cx| this.adopt_session(session, cx),
            window,
            cx,
        );
    }

    /// Approve (`approved`) or reject (`rejected`) the selected template suggestion.
    pub fn decide_reviewer_suggestion(
        &mut self,
        decision: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_l2("Template suggestion", cx) {
            return;
        }
        let (session_id, proposal_id) = match self.reviewer.suggestion_target() {
            Ok(target) => target,
            Err(error) => return self.status(format!("{error:#}"), cx),
        };
        let backend = Self::backend(cx);
        self.run_reviewer(
            "suggestions",
            "Submitting template suggestion decision...",
            "Template suggestion failed",
            async move {
                backend
                    .decide_template_suggestion(&session_id, &proposal_id, decision)
                    .await
            },
            |this, session, _, cx| this.adopt_session(session, cx),
            window,
            cx,
        );
    }

    pub fn submit_reviewer_analytics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.guard_l2("Reviewer analytics", cx) {
            return;
        }
        self.model.update(cx, |model, cx| {
            if let Ok(outcome) = model.update_doc(cx, |doc| doc.commit_pending_drafts("Reviewer")) {
                model.apply_outcome(outcome, cx);
            }
        });
        let payload = match self
            .reviewer
            .analytics_payload(self.model.read(cx).doc().project())
        {
            Ok(payload) => payload,
            Err(error) => {
                return self.status(format!("Reviewer analytics unavailable: {error:#}"), cx);
            }
        };
        let backend = Self::backend(cx);
        self.run_reviewer(
            "analytics",
            "Submitting Reviewer analytics...",
            "Reviewer analytics failed",
            async move { backend.submit_analytics(payload).await },
            |this, response, _, cx| {
                this.reviewer_last_response = Some(response);
                this.status("Reviewer analytics submitted.", cx);
            },
            window,
            cx,
        );
    }

    pub fn load_reviewer_history(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_l2("Reviewer history", cx) {
            return;
        }
        let query = parse_history_query(query);
        let backend = Self::backend(cx);
        self.run_reviewer(
            "history",
            "Loading review history...",
            "Review history failed",
            async move { backend.history(query).await },
            |this, response, _, cx| {
                this.reviewer_last_response = Some(response);
                this.status("Loaded review history.", cx);
            },
            window,
            cx,
        );
    }

    /// "Apply Feedback": copies the selected reviewer card into its rubric field.
    pub fn apply_selected_card_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((card, feedback)) = self
            .reviewer
            .selected_card_feedback()
            .map(|(card, feedback)| (card.clone(), feedback.cloned()))
        else {
            let selected = self.reviewer.selected_card_id().trim().to_string();
            let message = if selected.is_empty() {
                "Select a reviewer card before applying feedback.".to_string()
            } else {
                format!("Reviewer card {selected} was not found.")
            };
            return self.status(message, cx);
        };
        let Some(form) = self.feedback.as_mut() else {
            return self.status("Feedback panel is unavailable for this Babel task.", cx);
        };
        match form.apply_reviewer_card(&card, feedback.as_ref()) {
            Ok(label) => {
                self.status(
                    format!("Applied reviewer card {} to {label} feedback.", card.id),
                    cx,
                );
                self.drawer = Drawer::Feedback;
                self.edit_feedback(window, cx, |_| true);
            }
            Err(error) => self.status(format!("{error:#}"), cx),
        }
    }
}
