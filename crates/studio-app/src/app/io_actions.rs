use std::{fs, path::PathBuf};

use studio_app::{EditorSession, StudioWorkflow, load_sample_project};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::StudioState;
use super::formatting::write_text_file;
use super::recovery::{discover_recovery_drafts, recovery_snapshot_path, remove_recovery_snapshot};
use super::service_callbacks::{
    BabelServiceRequest, SaveRequestToken, build_save_annotations_request_with_asserted_warnings,
};
use super::service_executor::babel_remote_mutations_configured;

const EDITOR_AUTOSAVE_TRACE_ENV: &str = "STUDIO_TRACE_EDITOR_AUTOSAVE";

pub(super) fn editor_auto_save_runtime_trace_line(
    path: &str,
    dirty_before: bool,
    dirty_after: bool,
    save_has_error: bool,
) -> String {
    format!(
        "Transcription Studio editor autosave: action=save path={path} dirty={dirty_before}->{dirty_after} error={save_has_error}"
    )
}

fn editor_auto_save_trace_enabled() -> bool {
    std::env::var(EDITOR_AUTOSAVE_TRACE_ENV).is_ok_and(|value| value != "0")
}

fn save_failure_detail(status: &str) -> String {
    status
        .strip_prefix("Save failed: ")
        .unwrap_or(status)
        .to_string()
}

impl StudioState {
    pub(super) fn refresh_recovery_drafts(&mut self) {
        self.dashboard.recovery_drafts = discover_recovery_drafts();
    }

    pub(super) fn recover_latest_draft(&mut self) {
        self.refresh_recovery_drafts();
        let Some(draft) = self.dashboard.recovery_drafts.first().cloned() else {
            self.dashboard.error = Some("No local recovery draft is available.".to_string());
            return;
        };
        let base = draft
            .path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let result = fs::read_to_string(&draft.path)
            .map_err(|error| format!("failed to read recovery draft: {error}"))
            .and_then(|source| {
                StudioWorkflow::from_babel_json(&source).map_err(|error| error.to_string())
            });
        match result {
            Ok(workflow) => {
                self.replace_workflow(workflow, format!("Recovered local draft {}.", draft.label));
                let loaded = self.load_lane_waveforms_from_base(base);
                self.append_waveform_status(loaded);
                self.mark_dirty();
                self.dashboard.error = None;
            }
            Err(error) => {
                self.dashboard.error = Some(format!("Recovery failed: {error}"));
            }
        }
    }

    pub(super) fn load_bundled_demo(&mut self) {
        match load_sample_project()
            .map(StudioWorkflow::from_project)
            .map_err(|error| error.to_string())
        {
            Ok(workflow) => {
                self.replace_workflow(workflow, "Loaded bundled Babel demo.".to_string());
                self.load_bundled_sample_waveforms_for_runtime();
            }
            Err(error) => {
                self.workflow = Err(error.clone());
                self.editor = None;
                self.segment_drafts.clear();
                self.status = format!("Bundled demo failed: {error}");
            }
        }
    }

    pub(super) fn import_babel_json(&mut self) {
        let import_path = PathBuf::from(&self.import_path);
        let import_base = import_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        match fs::read_to_string(&self.import_path)
            .map_err(|error| format!("failed to read `{}`: {error}", self.import_path))
            .and_then(|source| {
                StudioWorkflow::from_babel_json(&source).map_err(|error| error.to_string())
            }) {
            Ok(workflow) => {
                self.replace_workflow(workflow, format!("Imported `{}`.", self.import_path));
                let loaded = self.load_lane_waveforms_from_base(import_base);
                self.append_waveform_status(loaded);
            }
            Err(error) => {
                self.status = format!("Import failed: {error}");
            }
        }
    }

    pub(super) fn export_babel_json(&mut self) {
        if !self.apply_pending_drafts("Export") {
            return;
        }

        let path = self.export_path.clone();
        if self.with_workflow_mut("Export", |workflow| {
            let json = workflow
                .export_babel_json()
                .map_err(|error| error.to_string())?;
            write_text_file(&path, &json)?;
            Ok(format!("Exported Babel JSON `{path}`."))
        }) {
            self.mark_saved();
        }
    }

    pub(super) fn save_progress(&mut self) {
        if self.block_read_only_edit("Save") {
            return;
        }

        let Some(editor) = self.editor.as_ref() else {
            self.status = "Save unavailable: no editor session.".to_string();
            return;
        };
        if editor.is_saving {
            return;
        }
        if !editor.is_dirty && !editor.save_has_error {
            return;
        }

        if !self.apply_pending_drafts("Save") {
            let error = save_failure_detail(&self.status);
            self.with_editor_mut(|editor| {
                editor.begin_save();
                editor.complete_save(false, error);
            });
            return;
        }
        self.with_editor_mut(EditorSession::begin_save);
        let Some(token) = self.current_save_request_token() else {
            self.status = "Save unavailable: could not start a save operation.".to_string();
            return;
        };

        let path = self.export_path.clone();
        let remote_save = babel_remote_mutations_configured();
        let recovery_path = remote_save
            .then(|| self.project().map(recovery_snapshot_path))
            .flatten();
        let save_result = self.with_workflow_mut("Save", |workflow| {
            let json = workflow
                .export_babel_json()
                .map_err(|error| error.to_string())?;
            write_text_file(&path, &json)?;
            if let Some(recovery_path) = recovery_path.as_ref() {
                write_text_file(&recovery_path.to_string_lossy(), &json)?;
            }
            Ok("Progress saved.".to_string())
        });
        if !save_result {
            let error = save_failure_detail(&self.status);
            self.with_editor_mut(|editor| {
                editor.complete_save_for_revision(token.revision, false, error);
            });
            return;
        }
        if recovery_path.is_some() {
            self.refresh_recovery_drafts();
        }

        let asserted_warning_keys = self
            .editor
            .as_ref()
            .map(|editor| editor.asserted_warning_keys.clone())
            .unwrap_or_default();
        if remote_save {
            let mut request = match self
                .workflow
                .as_ref()
                .map_err(|error| format!("no workflow loaded: {error}"))
                .and_then(|workflow| {
                    build_save_annotations_request_with_asserted_warnings(
                        workflow,
                        &asserted_warning_keys,
                    )
                }) {
                Ok(request) => request,
                Err(error) => {
                    self.with_editor_mut(|editor| {
                        editor.complete_save_for_revision(token.revision, false, error.clone());
                    });
                    self.status = format!("Save failed: {error}");
                    return;
                }
            };
            request.token = token;
            self.babel_service_requests
                .push(BabelServiceRequest::SaveAnnotations(request));
            self.status = "Saving progress...".to_string();
            return;
        }

        let last_saved_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
        self.complete_save_progress_success(token, last_saved_at, None);
    }

    pub(super) fn complete_save_progress_success(
        &mut self,
        token: SaveRequestToken,
        last_saved: String,
        issues: Option<Vec<studio_app::ServerLintIssue>>,
    ) -> bool {
        if token.workspace_generation != self.workspace_generation {
            return false;
        }
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::SaveAnnotations(request) if request.token == token)
        });
        let mut completed = false;
        self.with_editor_mut(|editor| {
            completed = editor.complete_save_for_revision(token.revision, true, last_saved);
        });
        if let Some(issues) = issues {
            self.install_server_lint_snapshot(token.revision, issues);
        }
        if !completed {
            return false;
        }
        if let Some(project) = self.project().cloned() {
            remove_recovery_snapshot(&project);
            self.refresh_recovery_drafts();
        }
        self.status = if self.editor.as_ref().is_some_and(|editor| editor.is_dirty) {
            "Earlier changes saved; newer edits are still pending.".to_string()
        } else {
            "Progress saved.".to_string()
        };
        true
    }

    pub(super) fn complete_save_progress_failure(
        &mut self,
        token: SaveRequestToken,
        error: String,
        issues: Option<Vec<studio_app::ServerLintIssue>>,
    ) -> bool {
        if token.workspace_generation != self.workspace_generation {
            return false;
        }
        self.drain_babel_service_requests_matching(|request| {
            matches!(request, BabelServiceRequest::SaveAnnotations(request) if request.token == token)
        });
        let mut completed = false;
        self.with_editor_mut(|editor| {
            completed = editor.complete_save_for_revision(token.revision, false, error.clone());
        });
        if let Some(issues) = issues {
            self.install_server_lint_snapshot(token.revision, issues);
        }
        if !completed {
            return false;
        }
        self.status = format!("Save failed: {error}");
        true
    }

    pub(super) fn should_run_editor_auto_save_tick(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|editor| editor.should_run_auto_save_tick(self.submit_is_pending))
    }

    pub(super) fn auto_save_progress(&mut self) {
        if self.should_run_editor_auto_save_tick() {
            let dirty_before = self.editor.as_ref().is_some_and(|editor| editor.is_dirty);
            self.save_progress();

            if editor_auto_save_trace_enabled() {
                let (dirty_after, save_has_error) = self
                    .editor
                    .as_ref()
                    .map(|editor| (editor.is_dirty, editor.save_has_error))
                    .unwrap_or((false, false));
                eprintln!(
                    "{}",
                    editor_auto_save_runtime_trace_line(
                        &self.export_path,
                        dirty_before,
                        dirty_after,
                        save_has_error
                    )
                );
            }
        }
        self.auto_save_reviewer_comments();
    }

    #[cfg(test)]
    pub(super) fn submit_review(&mut self) {
        if self.block_read_only_edit("Submit") {
            return;
        }

        self.submit_has_error = false;
        self.submit_is_pending = false;
        if !self.apply_pending_drafts("Submit") {
            self.submit_has_error = true;
            return;
        }

        let Some(project) = self.project().cloned() else {
            self.status = "Submit unavailable: no project loaded.".to_string();
            self.submit_has_error = true;
            return;
        };
        let Some(editor) = &mut self.editor else {
            self.status = "Submit unavailable: no editor session.".to_string();
            self.submit_has_error = true;
            return;
        };
        let submit = editor.submit_control(
            &project,
            self.show_submit_dialog,
            self.submit_is_pending,
            self.submit_has_error,
        );
        if submit.trigger_disabled {
            self.status = format!("Submit blocked: {}", submit.trigger_tooltip_content);
            self.show_submit_dialog = true;
            self.submit_has_error = true;
            return;
        }

        match editor.submit_review(&project) {
            Ok(()) => {
                if self.with_workflow_mut("Submit", |workflow| {
                    workflow.mark_review_passed();
                    Ok("Review submitted; Babel export will mark the action as pass.".to_string())
                }) {
                    self.mark_saved();
                    self.show_submit_dialog = false;
                    self.submit_has_error = false;
                }
            }
            Err(error) => {
                self.status = format!("Submit blocked: {error}");
                self.show_submit_dialog = true;
                self.submit_has_error = true;
            }
        }
    }
}
