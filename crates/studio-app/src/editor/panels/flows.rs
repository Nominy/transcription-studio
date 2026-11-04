//! Service-backed task flows: save, submit (Babel validates lint on submit), skip chunk, drop claim, admin
//! flag, read-only actions and leaving the task.
//!
//! There is no transcript autosave (the old app had none); the feedback form and reviewer
//! comments autosave on their own.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use studio_editor::editor_model::EditorSession;
use studio_editor::editor_model::review_actions::ReadOnlyActionCommand;
use studio_editor::linter::{ServerLintIssue, ServerLintSnapshot};
use studio_services::babel::{
    BabelError, SaveAnnotationsRequest, SaveOutcome, SubmitReviewActionRequest, active_queue_id,
    review_action_id,
};
use studio_services::recovery;
use studio_services::review::{build_reviewer_analytics_payload, reviewer_capability_available};

use super::dialogs;
use super::task::{Drawer, TaskPanels, TaskPanelsEvent};
use crate::editor::model::EditorModel;
use crate::services::{AppServices, spawn_service, spawn_service_in};

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
pub fn request_leave<T: 'static>(
    model: &Entity<EditorModel>,
    window: &mut Window,
    cx: &mut Context<T>,
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
    fn babel(&self, cx: &App) -> studio_services::babel::BabelClient {
        cx.global::<AppServices>().babel.clone()
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
                let accepted = doc.session_mut().set_server_lint_snapshot(snapshot);
                if accepted {
                    doc.with_workflow(|workflow| workflow.set_babel_lint_snapshot_metadata(issues));
                }
                accepted
            })
        })
    }

    // ---- save -----------------------------------------------------------------------------

    /// Save progress: commit drafts, export, recovery snapshot, then `save_annotations` when
    /// remote mutations are configured (otherwise the recovery snapshot is the local save).
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.model.read(cx).doc();
        if doc.is_read_only() {
            self.status("Save unavailable: project is read-only.", cx);
            return;
        }
        let session = doc.session();
        let needs_save = session.is_dirty || session.save_has_error || doc.has_pending_drafts();
        if session.is_saving || !needs_save {
            return;
        }
        if let Err(error) = self.commit_drafts("Save", cx) {
            self.model.update(cx, |model, cx| {
                model.update_doc(cx, |doc| {
                    let session = doc.session_mut();
                    session.begin_save();
                    session.complete_save(false, error.clone());
                });
                model.set_status(format!("Save failed: {error}"), cx);
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
        let remote = cx
            .global::<AppServices>()
            .babel
            .remote_mutations_configured();
        let snapshot = export
            .and_then(|json| recovery::write_snapshot(self.model.read(cx).doc().project(), &json));
        if let Err(error) = snapshot {
            self.finish_save(
                revision,
                false,
                Err(BabelError::new(format!("{error:#}"))),
                window,
                cx,
            );
            return;
        }
        if !remote {
            let outcome = SaveOutcome {
                last_saved: utc_now_rfc3339(),
                lint_issues: None,
            };
            self.finish_save(revision, false, Ok(outcome), window, cx);
            return;
        }
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                self.finish_save(
                    revision,
                    true,
                    Err(BabelError::new(format!("{error:#}"))),
                    window,
                    cx,
                );
                return;
            }
        };
        self.status("Saving progress...", cx);
        let babel = self.babel(cx);
        let generation = self.generation;
        self.flows.save_task = Some(spawn_service_in(
            window,
            cx,
            async move { babel.save_annotations(request).await },
            move |this, result, window, cx| {
                if this.generation == generation {
                    this.finish_save(revision, true, result, window, cx);
                }
            },
        ));
        cx.notify();
    }

    fn finish_save(
        &mut self,
        revision: u64,
        remote: bool,
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
            self.status(format!("Save failed: {detail}"), cx);
            return;
        }
        let project = self.model.read(cx).doc().project();
        if remote {
            recovery::remove_snapshot(project);
        }
        let still_dirty = self.model.read(cx).doc().session().is_dirty;
        let message = match (still_dirty, remote) {
            (true, _) => "Earlier changes saved; newer edits are still pending.",
            (false, true) => "Progress saved.",
            (false, false) => "Progress saved locally.",
        };
        self.status(message, cx);
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
            self.status("Submit unavailable: project is read-only.", cx);
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
        let eligibility = doc.session().submit_eligibility(doc.project());
        if let Some(blocker) = eligibility.blocker {
            let reason = blocker.reason();
            if reason.contains("feedback") {
                self.open_drawer(Drawer::Feedback, cx);
                self.status(
                    format!("Submit blocked: {reason} Opened the required feedback panel."),
                    cx,
                );
            } else {
                self.status(reason, cx);
            }
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
            self.status("Submit unavailable: project is read-only.", cx);
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
        self.flows.submit.error = false;
        if let Err(error) = self.commit_drafts("Submit", cx) {
            self.submit_failed(error, cx);
            return;
        }
        let doc = self.model.read(cx).doc();
        if let Some(blocker) = doc.session().submit_eligibility(doc.project()).blocker {
            self.submit_failed(format!("Submit blocked: {}", blocker.reason()), cx);
            return;
        }
        let session = doc.session();
        let revision = session.edit_revision();
        let submit = match SubmitReviewActionRequest::from_workflow(
            doc.workflow(),
            &session.asserted_warning_keys,
        ) {
            Ok(submit) => submit,
            Err(error) => {
                self.submit_failed(format!("Submit unavailable: {error:#}"), cx);
                return;
            }
        };
        self.flows.submit.pending = true;
        self.flows.submit.revision = revision;
        self.status("Submitting review...", cx);
        self.send_submit(submit, window, cx);
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
        self.submit_failed(format!("Submit blocked by Babel lint: {reasons}"), cx);
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
                        _ => this.submit_failed(format!("Submit failed: {}", error.message), cx),
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
                let review = cx.global::<AppServices>().review.clone();
                spawn_service(
                    cx,
                    async move { review.submit_analytics(payload).await },
                    |_, _, _| {},
                )
                .detach();
            }
            Err(error) => self.status(format!("Reviewer analytics skipped: {error:#}"), cx),
        }
    }

    fn finish_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flows.submit = SubmitFlow::default();
        self.flows.submit_task = None;
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.with_workflow(|workflow| workflow.mark_review_passed());
                let session = doc.session_mut();
                session.review_state = studio_editor::editor_model::ReviewState::Submitted;
                session.mark_saved();
            });
        });
        let project = self.model.read(cx).doc().project();
        recovery::remove_snapshot(project);
        self.finish_task(
            "Review submitted; Babel export will mark the action as pass.",
            window,
            cx,
        );
    }

    /// Any edit while a submit is pending invalidates its lint check.
    pub(super) fn cancel_submit_for_edit(&mut self, cx: &mut Context<Self>) {
        let revision = self.model.read(cx).doc().session().edit_revision();
        if !self.flows.submit.pending || self.flows.submit.revision == revision {
            return;
        }
        self.flows.submit = SubmitFlow::default();
        self.flows.submit_task = None;
        self.status("Transcript changed; submit again.", cx);
        cx.notify();
    }

    // ---- skip / admin flag / drop claim ---------------------------------------------------

    pub fn request_skip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.model.read(cx).doc().project();
        if !EditorSession::skip_chunk_available(project) {
            self.status("Skip unavailable for this task.", cx);
            return;
        }
        if !self.flows.skip.pending && !EditorSession::skip_chunk_allowed(project) {
            self.status(SKIP_DISABLED_TOOLTIP, cx);
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
            self.status("Skip unavailable: project is read-only.", cx);
            return;
        }
        let project = doc.project();
        if !EditorSession::skip_chunk_available(project)
            || !EditorSession::skip_chunk_allowed(project)
        {
            self.status("Skip unavailable for this task.", cx);
            return;
        }
        let review_action_id = review_action_id(project);
        self.flows.skip = ConfirmFlow {
            pending: true,
            error: None,
        };
        self.status("Skipping chunk...", cx);
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
                        this.finish_task("Skip requested.", window, cx);
                    }
                    Err(error) => {
                        let message = format!("Skip failed: {}", error.message);
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
            self.status("Flag recording unavailable for this task.", cx);
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
            self.status("Flag recording unavailable: project is read-only.", cx);
            return;
        }
        let project = doc.project();
        if !EditorSession::admin_flag_available(project) {
            self.status("Flag recording unavailable for this task.", cx);
            return;
        }
        let payload = EditorSession::admin_flag_payload(reason);
        let review_action_id = review_action_id(project);
        self.flows.admin_flag = ConfirmFlow {
            pending: true,
            error: None,
        };
        self.status("Flagging recording...", cx);
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
                        this.finish_task("Recording flagged.", window, cx);
                    }
                    Err(error) => {
                        let message = format!("Flag recording failed: {}", error.message);
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
            self.status("Drop claim unavailable: project is read-only.", cx);
            return;
        }
        let queue_id = active_queue_id(doc.project());
        let review_action_id = review_action_id(doc.project());
        self.flows.drop_pending = true;
        self.status("Dropping claim...", cx);
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
                        this.finish_task("Claim dropped.", window, cx);
                    }
                    Err(error) => this.status(format!("Drop claim failed: {}", error.message), cx),
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    /// Read-only task actions. Retry/assessment transitions are handled by Babel's web flow;
    /// the app only acknowledges them (as the old app did).
    pub fn read_only_action(&mut self, command: ReadOnlyActionCommand, cx: &mut Context<Self>) {
        match command {
            ReadOnlyActionCommand::RetryPractice => self.status("Practice retry requested.", cx),
            ReadOnlyActionCommand::TakeAssessment => {
                self.status("Assessment transition requested.", cx);
            }
            ReadOnlyActionCommand::RetryAssessment => {
                self.status("Assessment retry requested.", cx);
            }
            ReadOnlyActionCommand::BackToProjects => cx.emit(TaskPanelsEvent::Leave),
        }
    }
}

pub(super) const SKIP_DISABLED_TOOLTIP: &str =
    "Listen to this chunk for a bit longer before skipping.";

/// Current UTC time as RFC 3339 (`2026-11-01T17:05:09Z`) for locally completed saves.
fn utc_now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}
