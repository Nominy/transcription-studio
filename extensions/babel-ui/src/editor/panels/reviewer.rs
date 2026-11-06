//! Magic Review and the Grader for L2 review tasks, matching the Babel Review Helper:
//!
//! - Magic Review captures the review pair (current task as edited here, stable L1 original
//!   from Babel) and grades it alongside. Interactive mode opens the review workspace
//!   ([`super::review_workspace`]); fast mode generates feedback and applies it right away.
//! - Finalizing (Apply & Sync / Apply without review) flushes comments, finalizes the session,
//!   then applies the returned category feedback and the prepared grades to the feedback form.
//! - The Grader ([`super::grader`]) previews five editable grades and applies scores only.
//!
//! Paid calls carry the Review OpenRouter key/model from Settings.

use std::collections::BTreeSet;
use std::time::Duration;

use exaudio_babel_services::review::{
    CommentSave, GradeResult, ProposalDecision, ReviewBackend, ReviewCredentials, ReviewSnapshot,
    ReviewerFeedback, ReviewerNext, ReviewerSession, ReviewerWorkspace, TEMPLATE_SEARCH_LIMIT,
    capture_review_snapshot, current_review_state, current_transcript_key,
    reviewer_capability_available,
};
use gpui_kit::*;

use super::task::TaskPanels;
use crate::services::BabelServices;
use crate::text;
use exaudio_gold::prefs::AiPreferences;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::services::spawn_service_in;

const AUTOSAVE_INTERVAL: Duration = Duration::from_millis(500);
/// Callers named in row-draft commit messages.
const MAGIC_REVIEW: &str = "Magic Review";
const GRADER: &str = "Review Grader";

fn review_changed(cx: &App) -> &'static str {
    text::localized(
        cx,
        "The review changed. Refresh Magic Review before applying.",
        "Проверка изменилась. Обновите Magic Review перед применением.",
    )
}

fn grades_stale(cx: &App) -> &'static str {
    text::localized(
        cx,
        "The transcript changed. Generate grades again.",
        "Расшифровка изменилась. Сгенерируйте оценки заново.",
    )
}

fn preparing_review(cx: &App) -> &'static str {
    text::localized(cx, "Preparing review…", "Подготовка проверки…")
}

fn no_feedback(cx: &App) -> &'static str {
    text::localized(
        cx,
        "No feedback came back. Try again.",
        "Отзыв не получен. Попробуйте снова.",
    )
}

/// Grader failure for the workspace status line (single line, technical text after the dash).
fn grader_failed(cx: &App, error: &str) -> String {
    text::failed(cx, "Grader didn't work.", "Grader: не удалось.", error)
}

/// Grading prepared alongside a Magic Review run.
enum GradeState {
    Pending,
    Ready(GradeResult),
    Failed(String),
}

/// One Magic Review run: its captured pair and the grades prepared for it.
struct ReviewRun {
    id: u64,
    snapshot: ReviewSnapshot,
    grade: GradeState,
    /// Feedback waiting for the grades before it is applied.
    awaiting_grades: Option<Vec<ReviewerFeedback>>,
}

/// A status line and whether it reports a failure.
pub type StatusLine = (SharedString, bool);

#[derive(Default)]
pub struct MagicReview {
    pub workspace: ReviewerWorkspace,
    /// The interactive workspace dialog is shown.
    pub open: bool,
    /// A Magic Review run is starting (button spinner).
    pub running: bool,
    /// Working label; mutations are disabled while set.
    pub busy: Option<SharedString>,
    pub status: Option<StatusLine>,
    pub grading_status: Option<StatusLine>,
    /// "Template ideas" section expanded.
    pub ideas_open: bool,
    run: Option<ReviewRun>,
    runs: u64,
    tasks: Vec<Task<()>>,
}

impl MagicReview {
    fn track(&mut self, task: Task<()>) {
        self.tasks.retain(|task| !task.is_ready());
        self.tasks.push(task);
    }

    fn cancel_run(&mut self) {
        self.runs += 1;
        self.run = None;
    }

    fn is_current(&self, run: u64) -> bool {
        self.run.as_ref().is_some_and(|current| current.id == run)
    }
}

#[derive(Default)]
pub struct Grader {
    pub open: bool,
    pub busy: bool,
    pub status: Option<StatusLine>,
    pub result: Option<GradeResult>,
    /// Categories whose examples are expanded.
    pub expanded: BTreeSet<&'static str>,
    snapshot: Option<ReviewSnapshot>,
    revision: u64,
    task: Option<Task<()>>,
}

impl Grader {
    fn cancel(&mut self) {
        self.revision += 1;
        self.task = None;
        self.busy = false;
    }
}

impl TaskPanels {
    pub fn reviewer_available(&self, cx: &App) -> bool {
        reviewer_capability_available(self.model.read(cx).doc().project())
    }

    /// Backend with the Review key/model from Settings (paid routes need them).
    fn keyed_backend(cx: &App) -> anyhow::Result<ReviewBackend> {
        let settings = cx
            .global::<exaudio_shell::services::AppServices>()
            .storage
            .ai_settings();
        let credentials =
            ReviewCredentials::new(&settings.review_openrouter_key, &settings.review_model)?;
        cx.global::<BabelServices>()
            .review
            .with_credentials(credentials)
    }

    fn backend(cx: &App) -> ReviewBackend {
        cx.global::<BabelServices>().review.clone()
    }

    /// Commits pending row drafts and captures the current L2 state.
    fn capture_current(
        &mut self,
        action: &'static str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<(String, serde_json::Value)> {
        commit_row_drafts(&self.model, action, cx).map_err(anyhow::Error::msg)?;
        current_review_state(self.model.read(cx).doc().project())
    }

    /// Starts the 500 ms comment autosave tick for L2 tasks.
    pub(super) fn start_reviewer_autosave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.reviewer_available(cx) {
            return;
        }
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSAVE_INTERVAL).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    if this.review.workspace.should_autosave() {
                        this.save_review_comments(window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        self.review.tasks.push(task);
    }

    // ---- Magic Review -------------------------------------------------------------------------

    /// "Magic Review": interactive workspace, or fast mode per Settings.
    pub fn magic_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.running {
            return;
        }
        let fast = cx
            .global::<exaudio_shell::services::AppServices>()
            .storage
            .ai_settings()
            .review_fast_mode;
        let started = Self::keyed_backend(cx).and_then(|backend| {
            let (id, current) = self.capture_current(MAGIC_REVIEW, cx)?;
            Ok((backend, id, current))
        });
        let (backend, review_action_id, current) = match started {
            Ok(started) => started,
            Err(error) => {
                return self.status(
                    text::failed(
                        cx,
                        "Magic Review didn't work.",
                        "Magic Review: не удалось.",
                        error,
                    ),
                    cx,
                );
            }
        };
        self.review.cancel_run();
        self.review.workspace.clear();
        self.review.running = true;
        self.review.status = None;
        self.review.grading_status = Some((preparing_review(cx).into(), false));
        if fast {
            self.status(
                text::localized(
                    cx,
                    "Magic Review: generating feedback…",
                    "Magic Review: генерация отзыва…",
                ),
                cx,
            );
        } else {
            self.review.open = true;
            self.review.busy = Some(preparing_review(cx).into());
        }
        self.start_review_run(backend, review_action_id, current, fast, window, cx);
    }

    /// "Refresh": a new session from freshly captured transcripts.
    pub fn refresh_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.busy.is_some() || self.review.workspace.session().is_none() {
            return;
        }
        let started = Self::keyed_backend(cx).and_then(|backend| {
            let (id, current) = self.capture_current(MAGIC_REVIEW, cx)?;
            Ok((backend, id, current))
        });
        let (backend, review_action_id, current) = match started {
            Ok(started) => started,
            Err(error) => {
                self.review.status = Some((format!("{error:#}").into(), true));
                return cx.notify();
            }
        };
        self.review.cancel_run();
        self.review.workspace.clear();
        self.review.busy = Some(text::localized(cx, "Refreshing…", "Обновление…").into());
        self.review.grading_status = Some((preparing_review(cx).into(), false));
        self.start_review_run(backend, review_action_id, current, false, window, cx);
    }

    fn start_review_run(
        &mut self,
        backend: ReviewBackend,
        review_action_id: String,
        current: serde_json::Value,
        fast: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let attempt = self.review.runs;
        let babel = crate::services::document_client(&self.model, cx);
        let task = spawn_service_in(
            window,
            cx,
            async move { capture_review_snapshot(&babel, review_action_id, current).await },
            move |this, result, window, cx| {
                if this.review.runs != attempt {
                    return;
                }
                match result {
                    Ok(snapshot) => this.begin_run(backend, snapshot, fast, window, cx),
                    Err(error) => this.fail_review(format!("{error:#}"), cx),
                }
                cx.notify();
            },
        );
        self.review.track(task);
        cx.notify();
    }

    fn begin_run(
        &mut self,
        backend: ReviewBackend,
        snapshot: ReviewSnapshot,
        fast: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.review.runs += 1;
        let run = self.review.runs;
        self.review.run = Some(ReviewRun {
            id: run,
            snapshot: snapshot.clone(),
            grade: GradeState::Pending,
            awaiting_grades: None,
        });
        self.review.grading_status = Some((
            text::localized(cx, "Generating grades…", "Генерация оценок…").into(),
            false,
        ));

        let grader = backend.clone();
        let grade_snapshot = snapshot.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { grader.grade(&grade_snapshot).await },
            move |this, result, window, cx| this.finish_run_grades(run, result, window, cx),
        );
        self.review.track(task);

        let task = if fast {
            spawn_service_in(
                window,
                cx,
                async move { backend.generate_review(&snapshot).await },
                move |this, result, window, cx| {
                    if !this.review.is_current(run) {
                        return;
                    }
                    match result {
                        Ok(review) if review.feedback.is_empty() => {
                            this.fail_review(no_feedback(cx).to_string(), cx);
                        }
                        Ok(review) => {
                            let feedback = review.feedback.clone();
                            this.review.workspace.generated_review = Some(review);
                            this.apply_review_when_graded(run, feedback, window, cx);
                        }
                        Err(error) => this.fail_review(format!("{error:#}"), cx),
                    }
                    cx.notify();
                },
            )
        } else {
            spawn_service_in(
                window,
                cx,
                async move { backend.create_session(&snapshot).await },
                move |this, result, _, cx| {
                    if !this.review.is_current(run) {
                        return;
                    }
                    match result {
                        Ok(session) => {
                            this.review.workspace.replace_session(session);
                            this.review.running = false;
                            this.review.busy = None;
                            this.review.status = Some((
                                text::localized(
                                    cx,
                                    "Open a change to see the suggestion.",
                                    "Откройте изменение, чтобы увидеть предложение.",
                                )
                                .into(),
                                false,
                            ));
                            this.status(
                                text::localized(cx, "Review opened.", "Проверка открыта."),
                                cx,
                            );
                        }
                        Err(error) => this.fail_review(format!("{error:#}"), cx),
                    }
                    cx.notify();
                },
            )
        };
        self.review.track(task);
    }

    fn finish_run_grades(
        &mut self,
        run: u64,
        result: anyhow::Result<GradeResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.review.run.as_mut().filter(|current| current.id == run) else {
            return;
        };
        match result {
            Ok(grade) => {
                current.grade = GradeState::Ready(grade);
                self.review.grading_status = Some((
                    text::localized(cx, "Grades ready to apply", "Оценки готовы к применению")
                        .into(),
                    false,
                ));
            }
            Err(error) => {
                let error = format!("{error:#}");
                current.grade = GradeState::Failed(error.clone());
                self.review.grading_status = Some((
                    text::localized(
                        cx,
                        "Couldn't grade. Run Magic Review again.",
                        "Не удалось оценить. Запустите Magic Review снова.",
                    )
                    .into(),
                    true,
                ));
                if self.review.open {
                    self.review.status = Some((grader_failed(cx, &error).into(), true));
                }
            }
        }
        if let Some(feedback) = self
            .review
            .run
            .as_mut()
            .and_then(|current| current.awaiting_grades.take())
        {
            self.apply_review_results(run, feedback, window, cx);
        }
        cx.notify();
    }

    /// Applies `feedback` once the run's grades settled.
    fn apply_review_when_graded(
        &mut self,
        run: u64,
        feedback: Vec<ReviewerFeedback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.review.run.as_mut().filter(|current| current.id == run) else {
            return;
        };
        if matches!(current.grade, GradeState::Pending) {
            current.awaiting_grades = Some(feedback);
            self.review.busy = self
                .review
                .open
                .then(|| text::localized(cx, "Waiting for grades…", "Ожидание оценок…").into());
            self.status(
                text::localized(
                    cx,
                    "Magic Review: waiting for grades…",
                    "Magic Review: ожидание оценок…",
                ),
                cx,
            );
            return;
        }
        self.apply_review_results(run, feedback, window, cx);
    }

    /// Revalidates the captured pair, then applies category feedback and the prepared grades.
    fn apply_review_results(
        &mut self,
        run: u64,
        feedback: Vec<ReviewerFeedback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.review.run.as_ref().filter(|current| current.id == run) else {
            return;
        };
        let unchanged = transcript_matches(&self.model, MAGIC_REVIEW, &current.snapshot, cx);
        let grades = match (&current.grade, unchanged) {
            (_, false) => Err(review_changed(cx).to_string()),
            (GradeState::Ready(grade), true) => Ok(grade.grades.clone()),
            (GradeState::Failed(error), true) => Err(grader_failed(cx, error)),
            (GradeState::Pending, true) => return,
        };
        let grades = match grades {
            Ok(grades) => grades,
            Err(error) => {
                self.review.grading_status = Some((
                    text::localized(
                        cx,
                        "Grades not applied. Run Magic Review again.",
                        "Оценки не применены. Запустите Magic Review снова.",
                    )
                    .into(),
                    true,
                ));
                return self.fail_review(error, cx);
            }
        };
        match self.apply_feedback_and_grades(&feedback, &grades, window, cx) {
            Ok(applied) => {
                self.review.cancel_run();
                self.review.workspace.clear();
                self.review.open = false;
                self.review.running = false;
                self.review.busy = None;
                self.review.grading_status = Some((
                    text::localized(cx, "Grades applied", "Оценки применены").into(),
                    false,
                ));
                self.status(
                    text::localized_fmt(
                        cx,
                        "Applied feedback to {count}.",
                        "Отзыв применён: {count}.",
                        &[(
                            "count",
                            &text::count(
                                applied as u64,
                                ("category", "categories"),
                                ("категория", "категории", "категорий"),
                            ),
                        )],
                    ),
                    cx,
                );
            }
            Err(error) => self.fail_review(error, cx),
        }
    }

    fn fail_review(&mut self, error: String, cx: &mut Context<Self>) {
        self.review.running = false;
        self.review.busy = None;
        if let Some(run) = self.review.run.as_mut() {
            run.awaiting_grades = None;
        }
        if self.review.open {
            self.review.status = Some((error.clone().into(), true));
        }
        self.status(
            text::failed(
                cx,
                "Magic Review didn't work.",
                "Magic Review: не удалось.",
                &error,
            ),
            cx,
        );
        cx.notify();
    }

    /// Closes the workspace; unsaved comments are saved first, the run is discarded.
    pub fn close_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.workspace.comments_dirty() && !self.review.workspace.comment_save_failed() {
            self.save_review_comments(window, cx);
        }
        if self.review.run.is_some() {
            self.review.grading_status = Some((
                text::localized(
                    cx,
                    "Grading cancelled. Run Magic Review again.",
                    "Оценка отменена. Запустите Magic Review снова.",
                )
                .into(),
                false,
            ));
        }
        self.review.cancel_run();
        self.review.open = false;
        self.review.running = false;
        self.review.busy = None;
        self.review.status = None;
        cx.notify();
    }

    // ---- workspace session operations -------------------------------------------------------

    /// Runs one session mutation with the workspace busy, adopting the returned session.
    fn mutate_session(
        &mut self,
        busy: &'static str,
        done: &'static str,
        request: impl Future<Output = anyhow::Result<ReviewerSession>> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.review.busy.is_some() {
            return;
        }
        let session_id = self.review.workspace.session_id().to_string();
        self.review.busy = Some(busy.into());
        let task = spawn_service_in(window, cx, request, move |this, result, _, cx| {
            if this.review.workspace.session_id() != session_id {
                return;
            }
            this.review.busy = None;
            match result {
                Ok(session) => {
                    this.review.workspace.replace_session(session);
                    this.review.status = Some((done.into(), false));
                }
                Err(error) => this.review.status = Some((format!("{error:#}").into(), true)),
            }
            cx.notify();
        });
        self.review.track(task);
        cx.notify();
    }

    pub fn match_card_template(
        &mut self,
        card_id: String,
        template_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let backend = Self::backend(cx);
        let session_id = self.review.workspace.session_id().to_string();
        let card = card_id.clone();
        self.review
            .workspace
            .set_template_search_open(&card_id, false);
        self.mutate_session(
            text::localized(cx, "Matching template…", "Подбор шаблона…"),
            text::localized(cx, "Template matched.", "Шаблон подобран."),
            async move {
                backend
                    .set_template_match(&session_id, &card, &template_id)
                    .await
            },
            window,
            cx,
        );
    }

    pub fn clear_card_template(
        &mut self,
        card_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let backend = Self::backend(cx);
        let session_id = self.review.workspace.session_id().to_string();
        self.mutate_session(
            text::localized(cx, "Removing match…", "Удаление совпадения…"),
            text::localized(cx, "Template removed.", "Шаблон удалён."),
            async move { backend.clear_template_match(&session_id, &card_id).await },
            window,
            cx,
        );
    }

    /// "Rescan for suggestions" (paid).
    pub fn rescan_template_ideas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend = match Self::keyed_backend(cx) {
            Ok(backend) => backend,
            Err(error) => {
                self.review.status = Some((format!("{error:#}").into(), true));
                return cx.notify();
            }
        };
        let session_id = self.review.workspace.session_id().to_string();
        self.review.ideas_open = true;
        self.mutate_session(
            text::localized(cx, "Generating ideas…", "Генерация идей…"),
            text::localized(cx, "Ideas ready.", "Идеи готовы."),
            async move { backend.generate_template_suggestions(&session_id).await },
            window,
            cx,
        );
    }

    pub fn decide_template_idea(
        &mut self,
        proposal_id: String,
        decision: ProposalDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let backend = Self::backend(cx);
        let session_id = self.review.workspace.session_id().to_string();
        let (busy, done) = match decision {
            ProposalDecision::Approved => (
                text::localized(cx, "Approving…", "Одобрение…"),
                text::localized(cx, "Suggestion approved.", "Предложение одобрено."),
            ),
            ProposalDecision::Rejected => (
                text::localized(cx, "Rejecting…", "Отклонение…"),
                text::localized(cx, "Suggestion rejected.", "Предложение отклонено."),
            ),
        };
        self.mutate_session(
            busy,
            done,
            async move {
                backend
                    .decide_template_suggestion(&session_id, &proposal_id, decision)
                    .await
            },
            window,
            cx,
        );
    }

    /// Live per-card template search; only the latest query of a card is shown.
    pub fn search_card_templates(
        &mut self,
        card_id: &str,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.review.workspace.begin_template_search(card_id, query) else {
            return cx.notify();
        };
        let backend = Self::backend(cx);
        let query = request.query.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move {
                backend
                    .search_templates(&query, Some(TEMPLATE_SEARCH_LIMIT))
                    .await
            },
            move |this, result, _, cx| {
                if this
                    .review
                    .workspace
                    .finish_template_search(&request, result)
                {
                    cx.notify();
                }
            },
        );
        self.review.track(task);
        cx.notify();
    }

    // ---- comments and finalize ----------------------------------------------------------------

    /// Autosave tick and the save before finalize/close.
    pub fn save_review_comments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let save = match self.review.workspace.begin_comment_save() {
            Ok(save) => save,
            Err(_) => return,
        };
        let backend = Self::backend(cx);
        let request = save.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move {
                backend
                    .update_comments(&request.session_id, request.body)
                    .await
            },
            move |this, result, window, cx| this.finish_comment_save(&save, result, window, cx),
        );
        self.review.track(task);
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
        let finalizing = self.review.workspace.finalize_pending().is_some();
        let next = self.review.workspace.finish_comment_save(save, result);
        if let Some(error) = error {
            if finalizing {
                self.review.busy = None;
            }
            self.review.status = Some((
                text::failed(
                    cx,
                    "Couldn't save the comments. Edit a comment to retry.",
                    "Не удалось сохранить комментарии. Измените комментарий, чтобы повторить.",
                    error,
                )
                .into(),
                true,
            ));
        }
        self.follow_review_next(next, window, cx);
        cx.notify();
    }

    fn follow_review_next(
        &mut self,
        next: ReviewerNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match next {
            ReviewerNext::Idle => {}
            ReviewerNext::SaveComments => self.save_review_comments(window, cx),
            ReviewerNext::Finalize { session_id, mode } => {
                self.send_finalize(session_id, mode, window, cx);
            }
        }
    }

    /// "Apply and sync" / top "Apply" (`apply`) and "Apply without review" (`skip`).
    pub fn finalize_review(
        &mut self,
        mode: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.review.busy.is_some() {
            return;
        }
        if self.review.run.is_none() {
            self.review.status = Some((review_changed(cx).into(), true));
            return cx.notify();
        }
        match self.review.workspace.request_finalize(mode) {
            Ok(next) => {
                self.review.busy = Some(text::localized(cx, "Applying…", "Применение…").into());
                self.follow_review_next(next, window, cx);
            }
            Err(error) => self.review.status = Some((format!("{error:#}").into(), true)),
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
        let backend = Self::backend(cx);
        let Some(run) = self.review.run.as_ref().map(|run| run.id) else {
            return;
        };
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
                let Some(result) =
                    this.review
                        .workspace
                        .finish_finalize(&session_id, &mode, result)
                else {
                    return;
                };
                match result {
                    Ok(finalize) => {
                        this.apply_review_when_graded(run, finalize.feedback, window, cx)
                    }
                    Err(error) => {
                        this.review.busy = None;
                        this.review.status = Some((format!("{error:#}").into(), true));
                        this.review.grading_status = Some((
                            text::localized(
                                cx,
                                "Grades not applied. Refresh Magic Review.",
                                "Оценки не применены. Обновите Magic Review.",
                            )
                            .into(),
                            true,
                        ));
                    }
                }
                cx.notify();
            },
        );
        self.review.track(task);
    }

    // ---- Grader -------------------------------------------------------------------------------

    pub fn open_grader(&mut self, cx: &mut Context<Self>) {
        self.grader.open = true;
        let stale = self
            .grader
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !transcript_matches(&self.model, GRADER, snapshot, cx));
        if stale {
            self.grader.result = None;
            self.grader.snapshot = None;
            self.grader.status = Some((grades_stale(cx).into(), true));
        } else if self.grader.result.is_none() {
            self.grader.status = Some((
                text::localized(
                    cx,
                    "Grades compare the original and reviewed transcript. 1 is best.",
                    "Оценки сравнивают исходную и проверенную расшифровку. 1 — лучшая.",
                )
                .into(),
                false,
            ));
        }
        cx.notify();
    }

    /// Close cancels an active request; its late result is dropped.
    pub fn close_grader(&mut self, cx: &mut Context<Self>) {
        self.grader.cancel();
        self.grader.open = false;
        cx.notify();
    }

    /// "Generate grades" / "Generate again".
    pub fn generate_grades(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.grader.busy {
            return;
        }
        let started = Self::keyed_backend(cx).and_then(|backend| {
            let (id, current) = self.capture_current(GRADER, cx)?;
            Ok((backend, id, current))
        });
        let (backend, review_action_id, current) = match started {
            Ok(started) => started,
            Err(error) => {
                self.grader.status = Some((format!("{error:#}").into(), true));
                return cx.notify();
            }
        };
        self.grader.cancel();
        self.grader.busy = true;
        self.grader.result = None;
        self.grader.snapshot = None;
        self.grader.status = Some((
            text::localized(cx, "Reading transcripts…", "Чтение расшифровок…").into(),
            false,
        ));
        let revision = self.grader.revision;
        let babel = crate::services::document_client(&self.model, cx);
        let task = spawn_service_in(
            window,
            cx,
            async move {
                let snapshot = capture_review_snapshot(&babel, review_action_id, current).await?;
                anyhow::Ok(snapshot)
            },
            move |this, result, window, cx| {
                if this.grader.revision != revision {
                    return;
                }
                match result {
                    Ok(snapshot) => this.send_grade(backend, snapshot, window, cx),
                    Err(error) => this.grader_failed(format!("{error:#}"), cx),
                }
            },
        );
        self.grader.task = Some(task);
        cx.notify();
    }

    fn send_grade(
        &mut self,
        backend: ReviewBackend,
        snapshot: ReviewSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.grader.status = Some((text::localized(cx, "Grading…", "Оценка…").into(), false));
        let revision = self.grader.revision;
        let request = snapshot.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { backend.grade(&request).await },
            move |this, result, _, cx| {
                if this.grader.revision != revision {
                    return;
                }
                let result = result
                    .map_err(|error| format!("{error:#}"))
                    .and_then(|grade| {
                        if grade.review_action_id != snapshot.review_action_id {
                            return Err(text::localized(
                                cx,
                                "The grades are for another review. Generate grades again.",
                                "Эти оценки для другой проверки. Сгенерируйте оценки заново.",
                            )
                            .to_string());
                        }
                        if !transcript_matches(&this.model, GRADER, &snapshot, cx) {
                            return Err(grades_stale(cx).to_string());
                        }
                        Ok(grade)
                    });
                match result {
                    Ok(grade) => {
                        this.grader.busy = false;
                        this.grader.task = None;
                        this.grader.result = Some(grade);
                        this.grader.snapshot = Some(snapshot);
                        this.grader.status = Some((
                            text::localized(
                                cx,
                                "Check the grades and adjust any before applying.",
                                "Проверьте оценки и при необходимости измените их перед применением.",
                            )
                            .into(),
                            false,
                        ));
                        cx.notify();
                    }
                    Err(error) => this.grader_failed(error, cx),
                }
            },
        );
        self.grader.task = Some(task);
        cx.notify();
    }

    fn grader_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.grader.busy = false;
        self.grader.task = None;
        self.grader.status = Some((error.into(), true));
        cx.notify();
    }

    pub fn set_grade_score(&mut self, category: &str, score: u8, cx: &mut Context<Self>) {
        if self.grader.busy {
            return;
        }
        if let Some(grade) = self.grader.result.as_mut().and_then(|result| {
            result
                .grades
                .iter_mut()
                .find(|grade| grade.category == category)
        }) {
            grade.score = score.clamp(1, 3);
            cx.notify();
        }
    }

    pub fn toggle_grade_examples(&mut self, category: &'static str, cx: &mut Context<Self>) {
        if !self.grader.expanded.remove(category) {
            self.grader.expanded.insert(category);
        }
        cx.notify();
    }

    /// "Apply grades": refuses a changed transcript; applies scores only.
    pub fn apply_grades(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.grader.busy {
            return;
        }
        let (Some(result), Some(snapshot)) = (&self.grader.result, &self.grader.snapshot) else {
            return;
        };
        if !transcript_matches(&self.model, GRADER, snapshot, cx) {
            self.grader.result = None;
            self.grader.snapshot = None;
            self.grader.status = Some((grades_stale(cx).into(), true));
            return cx.notify();
        }
        let grades = result.grades.clone();
        match self.apply_feedback_and_grades(&[], &grades, window, cx) {
            Ok(_) => {
                self.grader.result = None;
                self.grader.snapshot = None;
                self.grader.status = Some((
                    text::localized(cx, "Grades applied.", "Оценки применены.").into(),
                    false,
                ));
            }
            Err(error) => self.grader.status = Some((error.into(), true)),
        }
        cx.notify();
    }
}

/// Commits pending row drafts: an edit still in a row editor is part of the transcript.
fn commit_row_drafts(
    model: &Entity<EditorModel>,
    action: &'static str,
    cx: &mut App,
) -> Result<(), String> {
    model.update(cx, |model, cx| {
        let outcome = model
            .update_doc(cx, |doc| doc.commit_pending_drafts(action))
            .map_err(|error| error.to_string())?;
        model.apply_outcome(outcome, cx);
        Ok(())
    })
}

/// Whether the transcript, row drafts included, is still the one `snapshot` captured.
fn transcript_matches(
    model: &Entity<EditorModel>,
    action: &'static str,
    snapshot: &ReviewSnapshot,
    cx: &mut App,
) -> bool {
    commit_row_drafts(model, action, cx).is_ok()
        && current_transcript_key(model.read(cx).doc().project(), &snapshot.original)
            .is_ok_and(|key| key == snapshot.transcript_key())
}
