//! Service-backed task flows: save, submit (Babel validates lint on submit), skip chunk, drop claim, admin
//! flag, read-only actions and leaving the task.
//!
//! There is no transcript autosave (the old app had none); the feedback form and reviewer
//! comments autosave on their own.

use exaudio_babel_domain::editor_model::EditorSession;
use exaudio_babel_domain::editor_model::review_actions::ReadOnlyActionCommand;
use exaudio_babel_domain::editor_model::submit_control::SubmitKind;
use exaudio_babel_lint::linter::{ServerLintIssue, ServerLintSnapshot};
use exaudio_babel_services::babel::{
    BabelError, SaveAnnotationsRequest, SaveOutcome, SubmitReviewActionRequest, active_queue_id,
    review_action_id,
};
use exaudio_babel_services::recovery;
use exaudio_babel_services::review::{
    build_reviewer_analytics_payload, reviewer_capability_available,
};
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::dialogs;
use super::task::{Drawer, TaskPanels, TaskPanelsEvent};
use crate::services::BabelServices;
use crate::text::{failed, localized, localized_fmt};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::services::{spawn_service, spawn_service_in};

type LeaveCallback = Box<dyn FnOnce(&mut Window, &mut App)>;

/// Pending flows and their in-flight requests.
#[derive(Default)]
pub(crate) struct Flows {
    pub submit: SubmitFlow,
    pub skip: ConfirmFlow,
    pub admin_flag: ConfirmFlow,
    pub drop_pending: bool,
    /// "Back to projects" waiting on the exit-confirm dialog.
    leave: Option<LeaveCallback>,
    /// Save & Continue: leave once a save leaves the task clean.
    continue_after_save: bool,
    saved: Option<exaudio_shell::screens::local_files::Saved>,
    save_task: Option<Task<()>>,
    submit_task: Option<Task<()>>,
    other_task: Option<Task<()>>,
}

#[derive(Default)]
pub(crate) struct SubmitFlow {
    pub pending: bool,
    pub error: bool,
    /// Edit revision the pending submit was linted for.
    revision: u64,
}

#[derive(Default)]
pub(crate) struct ConfirmFlow {
    pub pending: bool,
    pub error: Option<SharedString>,
}

/// Leave the task: leaves immediately when everything is saved, otherwise asks to save or
/// discard first, then calls `on_confirmed`.
///
/// `on_confirmed` always runs deferred, after the current update: callers (the editor
/// screen, the root's window-close handler) are mid-update when they ask, and leaving
/// updates them again.
pub fn request_leave(
    model: &Entity<EditorModel>,
    window: &mut Window,
    cx: &mut App,
    on_confirmed: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let on_confirmed: LeaveCallback = Box::new(move |window, cx| window.defer(cx, on_confirmed));
    match TaskPanels::find(model, cx) {
        Some(panels) => panels.update(cx, |panels, cx| {
            panels.request_leave(on_confirmed, window, cx);
        }),
        None => on_confirmed(window, cx),
    }
}

impl TaskPanels {
    fn babel(&self, cx: &App) -> exaudio_babel_services::babel::BabelClient {
        crate::services::document_client(&self.model, cx)
    }

    fn mark_saved(&self, cx: &mut App) {
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| doc.session_mut().mark_saved());
        });
    }

    /// Commits every pending row draft as one undo step.
    fn commit_drafts(&self, action: &'static str, cx: &mut App) -> Result<(), String> {
        self.model.update(cx, |model, cx| {
            match model.update_doc(cx, |doc| doc.commit_pending_drafts(action)) {
                Ok(outcome) => {
                    model.apply_outcome(outcome, cx);
                    Ok(())
                }
                Err(error) => Err(error.to_string()),
            }
        })
    }

    /// Installs a server lint snapshot for `revision` (rejected when the transcript changed).
    fn install_lint_snapshot(
        &self,
        revision: u64,
        issues: &[ServerLintIssue],
        cx: &mut App,
    ) -> bool {
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                let snapshot = ServerLintSnapshot::new(revision, issues.to_vec());
                let accepted = doc.with_session(|project, session| {
                    self.state
                        .borrow_mut()
                        .bind(session)
                        .set_server_lint_snapshot(project, snapshot)
                });
                if accepted {
                    doc.with_workflow(|workflow| workflow.set_babel_lint_snapshot_metadata(issues));
                }
                accepted
            })
        })
    }

    // ---- save -----------------------------------------------------------------------------
    pub fn save_with_callback(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        saved: Option<exaudio_shell::screens::local_files::Saved>,
    ) {
        self.flows.saved = saved;
        let doc = self.model.read(cx).doc();
        if !doc.session().is_dirty && !doc.has_pending_drafts() && !doc.session().is_saving {
            if let Some(saved) = self.flows.saved.take() {
                window.defer(cx, saved);
            }
            return;
        }
        self.save(window, cx);
    }

    /// Save progress to the transport captured when this Babel document was opened.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.model.read(cx).doc();
        if doc.is_read_only() {
            self.status(read_only_message(cx), cx);
            return;
        }
        let session = doc.session();
        let needs_save = session.is_dirty || session.save_has_error || doc.has_pending_drafts();
        if session.is_saving || !needs_save {
            return;
        }
        if let Err(error) = self.commit_drafts(localized(cx, "Save", "Сохранить"), cx) {
            let message = failed(cx, "Couldn't save.", "Не удалось сохранить.", &error);
            self.model.update(cx, |model, cx| {
                model.update_doc(cx, |doc| {
                    let session = doc.session_mut();
                    session.begin_save();
                    session.complete_save(false, error.clone());
                });
                model.set_status(message, cx);
            });
            cx.notify();
            return;
        }

        let (revision, export, request) = self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.session_mut().begin_save();
                let session = doc.session();
                let revision = session.saving_revision().unwrap_or(session.edit_revision());
                let export = doc.workflow().export_babel_json();
                let request = SaveAnnotationsRequest::from_workflow(
                    doc.workflow(),
                    &session.asserted_warning_keys,
                );
                (revision, export, request)
            })
        });
        let snapshot = export
            .and_then(|json| recovery::write_snapshot(self.model.read(cx).doc().project(), &json));
        if let Err(error) = snapshot {
            self.finish_save(
                revision,
                Err(BabelError::new(format!("{error:#}"))),
                window,
                cx,
            );
            return;
        }
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                self.finish_save(
                    revision,
                    Err(BabelError::new(format!("{error:#}"))),
                    window,
                    cx,
                );
                return;
            }
        };
        self.status(localized(cx, "Saving…", "Сохранение…"), cx);
        let babel = self.babel(cx);
        let generation = self.generation;
        self.flows.save_task = Some(spawn_service_in(
            window,
            cx,
            async move { babel.save_annotations(request).await },
            move |this, result, window, cx| {
                if this.generation == generation {
                    this.finish_save(revision, result, window, cx);
                }
            },
        ));
        cx.notify();
    }

    fn finish_save(
        &mut self,
        revision: u64,
        result: Result<SaveOutcome, BabelError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flows.save_task = None;
        let lint_issues = match &result {
            Ok(outcome) => outcome.lint_issues.clone(),
            Err(error) => error.lint_issues.clone(),
        };
        if let Some(issues) = lint_issues {
            self.install_lint_snapshot(revision, &issues, cx);
        }
        let (succeeded, detail) = match &result {
            Ok(outcome) => (true, outcome.last_saved.clone()),
            Err(error) => (false, error.message.clone()),
        };
        let accepted = self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.session_mut()
                    .complete_save_for_revision(revision, succeeded, detail.clone())
            })
        });
        if !accepted {
            return;
        }
        cx.notify();
        if !succeeded {
            self.flows.continue_after_save = false;
            self.flows.saved = None;
            let message = failed(cx, "Couldn't save.", "Не удалось сохранить.", detail);
            self.status(message, cx);
            return;
        }
        let project = self.model.read(cx).doc().project();
        recovery::remove_snapshot(project);
        let doc = self.model.read(cx).doc();
        let still_dirty = doc.session().is_dirty || doc.has_pending_drafts();
        let message = match still_dirty {
            true => localized(
                cx,
                "Earlier changes saved; newer edits are still pending.",
                "Ранние изменения сохранены; новые правки ещё не сохранены.",
            ),
            false => localized(cx, "Saved", "Сохранено"),
        };
        self.status(message, cx);
        if !still_dirty && let Some(saved) = self.flows.saved.take() {
            window.defer(cx, saved);
        }
        if self.flows.continue_after_save && self.flows.leave.is_some() {
            if still_dirty {
                self.save(window, cx);
            } else {
                self.flows.continue_after_save = false;
                window.close_dialog(cx);
                if let Some(leave) = self.flows.leave.take() {
                    leave(window, cx);
                }
            }
        }
    }

    // ---- leave ----------------------------------------------------------------------------

    pub(super) fn request_leave(
        &mut self,
        on_confirmed: LeaveCallback,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let doc = self.model.read(cx).doc();
        let session = doc.session();
        let clean = !session.is_dirty && !session.is_saving && !doc.has_pending_drafts();
        if clean || doc.is_read_only() {
            on_confirmed(window, cx);
            return;
        }
        let already_open = self.flows.leave.replace(on_confirmed).is_some();
        self.flows.continue_after_save = session.is_saving;
        if !already_open {
            dialogs::exit_confirm::open(cx.entity(), self.model.clone(), window, cx);
        }
        cx.notify();
    }

    /// Exit confirm: "Keep Editing" (also Escape).
    pub(super) fn keep_editing(&mut self, cx: &mut Context<Self>) {
        self.flows.leave = None;
        self.flows.continue_after_save = false;
        cx.notify();
    }

    /// Exit confirm: "Discard Changes". Pending work is dropped; edits are not reverted.
    pub(super) fn discard_and_leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(leave) = self.flows.leave.take() else {
            return;
        };
        self.generation += 1;
        self.flows.continue_after_save = false;
        self.flows.save_task = None;
        self.flows.submit_task = None;
        self.flows.submit = SubmitFlow::default();
        self.mark_saved(cx);
        window.close_dialog(cx);
        leave(window, cx);
    }

    /// Exit confirm: "Save & Continue".
    pub(super) fn save_and_leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flows.continue_after_save = true;
        if self.model.read(cx).doc().session().is_saving {
            cx.notify();
            return;
        }
        self.save(window, cx);
        // A save with nothing to do (e.g. only an earlier failure was cleared) leaves at once.
        let doc = self.model.read(cx).doc();
        if !doc.session().is_saving && !doc.session().is_dirty && !doc.has_pending_drafts() {
            self.flows.continue_after_save = false;
            window.close_dialog(cx);
            if let Some(leave) = self.flows.leave.take() {
                leave(window, cx);
            }
        }
        cx.notify();
    }

    fn finish_task(&mut self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        window.close_all_dialogs(cx);
        window.push_notification(Notification::success(message.to_string()), cx);
        cx.emit(TaskPanelsEvent::Leave);
    }

    // ---- submit ---------------------------------------------------------------------------

    /// Submit button: explains blockers (opening the feedback drawer when it blocks) or opens
    /// the confirmation dialog.
    pub fn request_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flows.submit.pending {
            dialogs::submit::open(cx.entity(), window, cx);
            return;
        }
        let doc = self.model.read(cx).doc();
        if doc.is_read_only() {
            self.status(read_only_message(cx), cx);
            return;
        }
        if let Some(reason) = self
            .feedback
            .as_ref()
            .and_then(|form| form.submit_block_reason())
        {
            self.open_drawer(Drawer::Feedback, cx);
            self.status(reason, cx);
            return;
        }
        let eligibility = self
            .state
            .borrow()
            .view(doc.session())
            .submit_eligibility(doc.project());
        if let Some(blocker) = eligibility.blocker {
            if blocker.is_feedback() {
                self.open_drawer(Drawer::Feedback, cx);
            }
            self.status(blocker.reason(), cx);
            return;
        }
        self.flows.submit.error = false;
        dialogs::submit::open(cx.entity(), window, cx);
        cx.notify();
    }

    /// Submit dialog confirm: commit drafts, then submit. Lint runs locally (as in Babel's
    /// editor); Babel validates again and answers lint errors in the submit response.
    pub(super) fn confirm_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flows.submit.pending {
            return;
        }
        if self.model.read(cx).doc().is_read_only() {
            self.status(read_only_message(cx), cx);
            return;
        }
        if let Some(reason) = self
            .feedback
            .as_ref()
            .and_then(|form| form.submit_block_reason())
        {
            self.open_drawer(Drawer::Feedback, cx);
            self.status(reason, cx);
            return;
        }
        if let Err(error) = self.commit_drafts(localized(cx, "Submit", "Отправить"), cx) {
            self.submit_failed(error, cx);
            return;
        }
        let doc = self.model.read(cx).doc();
        let eligibility = self
            .state
            .borrow()
            .view(doc.session())
            .submit_eligibility(doc.project());
        if let Some(blocker) = eligibility.blocker {
            self.submit_failed(blocker.reason().to_string(), cx);
            return;
        }
        let session = doc.session();
        let revision = session.edit_revision();
        if eligibility.kind != SubmitKind::Review {
            let request = SaveAnnotationsRequest::from_workflow(
                doc.workflow(),
                &session.asserted_warning_keys,
            );
            match request {
                Ok(request) => {
                    self.send_assessment(request, eligibility.kind, revision, window, cx)
                }
                Err(error) => {
                    let message = submit_unavailable(cx, error);
                    self.submit_failed(message, cx)
                }
            }
            return;
        }
        let submit = match SubmitReviewActionRequest::from_workflow(
            doc.workflow(),
            &session.asserted_warning_keys,
        ) {
            Ok(submit) => submit,
            Err(error) => {
                let message = submit_unavailable(cx, error);
                self.submit_failed(message, cx);
                return;
            }
        };
        self.flows.submit.pending = true;
        self.flows.submit.revision = revision;
        self.status(localized(cx, "Submitting…", "Отправка…"), cx);
        self.send_submit(submit, window, cx);
        cx.notify();
    }

    fn send_assessment(
        &mut self,
        request: SaveAnnotationsRequest,
        kind: SubmitKind,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flows.submit.pending = true;
        self.flows.submit.revision = revision;
        let generation = self.generation;
        let babel = self.babel(cx);
        self.status(
            localized(cx, "Submitting the attempt…", "Отправка попытки…"),
            cx,
        );
        self.flows.submit_task = Some(spawn_service_in(
            window,
            cx,
            async move {
                match kind {
                    SubmitKind::Practice => babel.submit_practice_attempt(request).await,
                    _ => babel.submit_assessment(request).await,
                }
            },
            move |this, result, window, cx| {
                if !this.submit_is_current(generation, revision) {
                    return;
                }
                match result {
                    Err(error) => {
                        let message = failed(
                            cx,
                            "Couldn't submit the attempt.",
                            "Не удалось отправить попытку.",
                            error.message,
                        );
                        this.submit_failed(message, cx)
                    }
                    Ok(result) => {
                        this.flows.submit.pending = false;
                        this.flows.submit_task = None;
                        this.model.update(cx, |model, cx| {
                            model.update_doc(cx, |doc| {
                                doc.with_workflow(|workflow| {
                                    let metadata = &mut workflow.project_mut().metadata;
                                    metadata.insert(
                                        "studio.babel.assessment.isAssessmentReadOnly".into(),
                                        "true".into(),
                                    );
                                    metadata.insert(
                                        "studio.babel.assessment.isFinalSubmitted".into(),
                                        (kind == SubmitKind::FinalAssessment).to_string(),
                                    );
                                    metadata.insert(
                                        "studio.babel.assessment.lastResult".into(),
                                        result.to_string(),
                                    );
                                    let retries = metadata
                                        .get("studio.babel.assessment.retryCount")
                                        .and_then(|value| value.parse::<usize>().ok())
                                        .unwrap_or_default();
                                    metadata.insert(
                                        "studio.babel.assessment.retryCount".into(),
                                        (retries + 1).to_string(),
                                    );
                                });
                                doc.session_mut().mark_saved();
                            });
                            model.set_read_only(true, cx);
                        });
                        window.close_dialog(cx);
                        let outcome = result
                            .get("passed")
                            .and_then(serde_json::Value::as_bool)
                            .map(|passed| {
                                if passed {
                                    localized(cx, "Attempt passed.", "Попытка засчитана.")
                                } else {
                                    localized(cx, "Attempt not passed.", "Попытка не засчитана.")
                                }
                            })
                            .unwrap_or(localized(cx, "Attempt submitted.", "Попытка отправлена."));
                        let details = [
                            ("attemptNumber", ("Attempt {v}", "Попытка {v}")),
                            ("wordErrorRate", ("Word errors {v}", "Ошибки слов {v}")),
                            ("tagErrorRate", ("Tag errors {v}", "Ошибки тегов {v}")),
                            (
                                "timestampShiftMs",
                                ("Timing shift {v} ms", "Сдвиг времени {v} мс"),
                            ),
                        ]
                        .into_iter()
                        .filter_map(|(key, (en, ru))| {
                            result.get(key).map(|value| {
                                localized_fmt(cx, en, ru, &[("v", &value.to_string())])
                            })
                        })
                        .collect::<Vec<_>>()
                        .join(" · ");
                        this.status(format!("{outcome} {details}"), cx);
                        cx.notify();
                    }
                }
            },
        ));
        cx.notify();
    }

    fn submit_is_current(&self, generation: u64, revision: u64) -> bool {
        self.generation == generation
            && self.flows.submit.pending
            && self.flows.submit.revision == revision
    }

    fn submit_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.flows.submit.pending = false;
        self.flows.submit.error = true;
        self.flows.submit_task = None;
        self.status(message, cx);
        cx.notify();
    }

    /// Any lint issue blocks the submit: install the snapshot and select the first row.
    fn block_submit_on_lint(&mut self, issues: &[ServerLintIssue], cx: &mut Context<Self>) {
        let revision = self.flows.submit.revision;
        self.install_lint_snapshot(revision, issues, cx);
        if let Some(first) = issues.first() {
            let segment_id = first.annotation_id.clone();
            self.model.update(cx, |model, cx| {
                let exists = model
                    .doc()
                    .project()
                    .segments
                    .iter()
                    .any(|segment| segment.id == segment_id);
                if exists {
                    model.run(cx, |doc| doc.select_segment(&segment_id));
                }
            });
        }
        let reasons = issues
            .iter()
            .map(|issue| issue.reason.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        let message = failed(
            cx,
            "Babel found problems. Fix them first.",
            "Babel нашёл проблемы. Сначала исправьте их.",
            reasons,
        );
        self.submit_failed(message, cx);
    }

    fn send_submit(
        &mut self,
        request: SubmitReviewActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_reviewer_analytics(cx);
        let babel = self.babel(cx);
        let generation = self.generation;
        let revision = self.flows.submit.revision;
        self.flows.submit_task = Some(spawn_service_in(
            window,
            cx,
            async move { babel.submit_review_action(request).await },
            move |this, result, window, cx| {
                if !this.submit_is_current(generation, revision) {
                    return;
                }
                match result {
                    Ok(()) => this.finish_submit(window, cx),
                    Err(error) => match error.lint_issues.as_deref() {
                        Some(issues) if !issues.is_empty() => this.block_submit_on_lint(issues, cx),
                        _ => {
                            let message = failed(
                                cx,
                                "Couldn't submit.",
                                "Не удалось отправить.",
                                error.message,
                            );
                            this.submit_failed(message, cx)
                        }
                    },
                }
            },
        ));
    }

    /// L2 tasks report reviewer analytics alongside the submit; failures never block it.
    fn send_reviewer_analytics(&mut self, cx: &mut Context<Self>) {
        let project = self.model.read(cx).doc().project();
        if !reviewer_capability_available(project) {
            return;
        }
        let payload = build_reviewer_analytics_payload(
            project,
            self.review.workspace.session(),
            self.review.workspace.generated_review.as_ref(),
        );
        match payload {
            Ok(payload) => {
                let review = cx.global::<BabelServices>().review.clone();
                spawn_service(
                    cx,
                    async move { review.submit_analytics(payload).await },
                    |_, _, _| {},
                )
                .detach();
            }
            Err(error) => {
                let message = failed(
                    cx,
                    "Reviewer statistics were not sent.",
                    "Статистика ревью не отправлена.",
                    format!("{error:#}"),
                );
                self.status(message, cx)
            }
        }
    }

    fn finish_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flows.submit = SubmitFlow::default();
        self.flows.submit_task = None;
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.with_workflow(|workflow| workflow.mark_review_passed());
                let session = doc.session_mut();
                self.state.borrow_mut().mark_submitted(session);
                session.mark_saved();
            });
        });
        let project = self.model.read(cx).doc().project();
        recovery::remove_snapshot(project);
        self.finish_task(localized(cx, "Submitted", "Отправлено"), window, cx);
    }

    /// Any edit while a submit is pending invalidates its lint check.
    pub(super) fn cancel_submit_for_edit(&mut self, cx: &mut Context<Self>) {
        let revision = self.model.read(cx).doc().session().edit_revision();
        if !self.flows.submit.pending || self.flows.submit.revision == revision {
            return;
        }
        self.flows.submit = SubmitFlow::default();
        self.flows.submit_task = None;
        self.status(
            localized(
                cx,
                "The transcript changed. Submit again.",
                "Расшифровка изменилась. Отправьте снова.",
            ),
            cx,
        );
        cx.notify();
    }

    // ---- skip / admin flag / drop claim ---------------------------------------------------

    pub fn request_skip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.model.read(cx).doc().project();
        if !EditorSession::skip_chunk_available(project) {
            self.status(skip_unavailable_message(cx), cx);
            return;
        }
        if !self.flows.skip.pending && !EditorSession::skip_chunk_allowed(project) {
            self.status(skip_disabled_tooltip(cx), cx);
            return;
        }
        self.flows.skip.error = None;
        dialogs::skip::open(cx.entity(), window, cx);
    }

    pub(super) fn confirm_skip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flows.skip.pending {
            return;
        }
        let doc = self.model.read(cx).doc();
        if doc.is_read_only() {
            self.status(read_only_message(cx), cx);
            return;
        }
        let project = doc.project();
        if !EditorSession::skip_chunk_available(project)
            || !EditorSession::skip_chunk_allowed(project)
        {
            self.status(skip_unavailable_message(cx), cx);
            return;
        }
        let review_action_id = review_action_id(project);
        self.flows.skip = ConfirmFlow {
            pending: true,
            error: None,
        };
        self.status(localized(cx, "Skipping…", "Пропуск…"), cx);
        let babel = self.babel(cx);
        let generation = self.generation;
        self.flows.other_task = Some(spawn_service_in(
            window,
            cx,
            async move { babel.skip_chunk(review_action_id).await },
            move |this, result, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.flows.skip.pending = false;
                match result {
                    Ok(()) => {
                        this.model.update(cx, |model, cx| {
                            model.update_doc(cx, |doc| {
                                doc.with_workflow(|workflow| workflow.mark_skip_requested());
                                doc.session_mut().mark_saved();
                            });
                        });
                        this.finish_task(localized(cx, "Skipped", "Пропущено"), window, cx);
                    }
                    Err(error) => {
                        let message = failed(
                            cx,
                            "Couldn't skip.",
                            "Не удалось пропустить.",
                            error.message,
                        );
                        this.flows.skip.error = Some(message.clone().into());
                        this.status(message, cx);
                    }
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    pub fn request_admin_flag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.model.read(cx).doc().project();
        if !EditorSession::admin_flag_available(project) {
            self.status(flag_unavailable_message(cx), cx);
            return;
        }
        if !self.flows.admin_flag.pending {
            self.flows.admin_flag.error = None;
        }
        dialogs::admin_flag::open(cx.entity(), window, cx);
    }

    pub(super) fn confirm_admin_flag(
        &mut self,
        reason: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.flows.admin_flag.pending {
            return;
        }
        let doc = self.model.read(cx).doc();
        if doc.is_read_only() {
            self.status(read_only_message(cx), cx);
            return;
        }
        let project = doc.project();
        if !EditorSession::admin_flag_available(project) {
            self.status(flag_unavailable_message(cx), cx);
            return;
        }
        let payload = EditorSession::admin_flag_payload(reason);
        let review_action_id = review_action_id(project);
        self.flows.admin_flag = ConfirmFlow {
            pending: true,
            error: None,
        };
        self.status(
            localized(cx, "Flagging the recording…", "Пометка записи…"),
            cx,
        );
        let babel = self.babel(cx);
        let generation = self.generation;
        let request = payload.clone();
        self.flows.other_task = Some(spawn_service_in(
            window,
            cx,
            async move {
                babel
                    .admin_flag(review_action_id, request.rating, request.rating_reason)
                    .await
            },
            move |this, result, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.flows.admin_flag.pending = false;
                match result {
                    Ok(()) => {
                        this.model.update(cx, |model, cx| {
                            model.update_doc(cx, |doc| {
                                doc.with_workflow(|workflow| workflow.record_admin_flag(payload));
                            });
                        });
                        // The transcript itself is not saved by flagging: leaving still asks
                        // about unsaved edits.
                        this.finish_task(
                            localized(cx, "Recording flagged", "Запись помечена"),
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        let message = failed(
                            cx,
                            "Couldn't flag the recording.",
                            "Не удалось пометить запись.",
                            error.message,
                        );
                        this.flows.admin_flag.error = Some(message.clone().into());
                        this.status(message, cx);
                    }
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    pub fn drop_claim(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flows.drop_pending {
            return;
        }
        let doc = self.model.read(cx).doc();
        if doc.is_read_only() {
            self.status(read_only_message(cx), cx);
            return;
        }
        let queue_id = active_queue_id(doc.project());
        let review_action_id = review_action_id(doc.project());
        self.flows.drop_pending = true;
        self.status(
            localized(cx, "Releasing the task…", "Задача отпускается…"),
            cx,
        );
        let babel = self.babel(cx);
        let generation = self.generation;
        self.flows.other_task = Some(spawn_service_in(
            window,
            cx,
            async move { babel.drop_claim(queue_id, review_action_id).await },
            move |this, result, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.flows.drop_pending = false;
                match result {
                    Ok(()) => {
                        this.model.update(cx, |model, cx| {
                            model.update_doc(cx, |doc| {
                                doc.with_workflow(|workflow| workflow.mark_claim_dropped());
                                doc.session_mut().mark_saved();
                            });
                        });
                        this.finish_task(
                            localized(cx, "Task released", "Задача отпущена"),
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        let message = failed(
                            cx,
                            "Couldn't release the task.",
                            "Не удалось отпустить задачу.",
                            error.message,
                        );
                        this.status(message, cx)
                    }
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    /// Assessment retries retain current annotations and unlock the result view.
    pub fn read_only_action(
        &mut self,
        command: ReadOnlyActionCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            ReadOnlyActionCommand::RetryPractice | ReadOnlyActionCommand::RetryAssessment => {
                self.model.update(cx, |model, cx| {
                    model.update_doc(cx, |doc| {
                        doc.with_workflow(|workflow| {
                            workflow.project_mut().metadata.insert(
                                "studio.babel.assessment.isAssessmentReadOnly".into(),
                                "false".into(),
                            );
                            workflow.project_mut().metadata.insert(
                                "studio.babel.assessment.isFinalSubmitted".into(),
                                "false".into(),
                            );
                        });
                        if doc.session().diff_view {
                            doc.session_mut().toggle_diff_view();
                        }
                    });
                    model.set_read_only(false, cx);
                });
                self.status(
                    localized(
                        cx,
                        "Attempt reopened. Your annotations are kept.",
                        "Попытка открыта снова. Ваши аннотации сохранены.",
                    ),
                    cx,
                );
            }
            ReadOnlyActionCommand::TakeAssessment => {
                if self.flows.other_task.is_some() {
                    return;
                }
                let queue_id = active_queue_id(self.model.read(cx).doc().project());
                if queue_id.is_empty() {
                    self.status(
                        localized(
                            cx,
                            "This task has no assessment.",
                            "У этой задачи нет теста.",
                        ),
                        cx,
                    );
                    return;
                }
                let babel = self.babel(cx);
                self.status(
                    localized(cx, "Opening the assessment…", "Открытие теста…"),
                    cx,
                );
                self.flows.other_task = Some(spawn_service_in(
                    window,
                    cx,
                    async move { babel.transition_to_assessment(&queue_id).await },
                    |this, result, window, cx| {
                        this.flows.other_task = None;
                        match result {
                            Ok(()) => crate::root::reload_task(&this.model, window, cx),
                            Err(error) => {
                                let message = failed(
                                    cx,
                                    "Couldn't open the assessment.",
                                    "Не удалось открыть тест.",
                                    error.message,
                                );
                                this.status(message, cx)
                            }
                        }
                    },
                ));
            }
            ReadOnlyActionCommand::BackToProjects => cx.emit(TaskPanelsEvent::Leave),
        }
    }
}

pub(super) fn skip_disabled_tooltip(cx: &App) -> &'static str {
    localized(
        cx,
        "Listen to this chunk a bit longer before skipping.",
        "Послушайте этот чанк ещё немного, прежде чем пропускать.",
    )
}

fn read_only_message(cx: &App) -> &'static str {
    localized(
        cx,
        "This task is read only.",
        "Эта задача только для чтения.",
    )
}

fn skip_unavailable_message(cx: &App) -> &'static str {
    localized(
        cx,
        "This task can't be skipped.",
        "Эту задачу нельзя пропустить.",
    )
}

fn flag_unavailable_message(cx: &App) -> &'static str {
    localized(
        cx,
        "This recording can't be flagged.",
        "Эту запись нельзя пометить.",
    )
}

fn submit_unavailable(cx: &App, error: anyhow::Error) -> String {
    failed(
        cx,
        "Couldn't submit.",
        "Не удалось отправить.",
        format!("{error:#}"),
    )
}
