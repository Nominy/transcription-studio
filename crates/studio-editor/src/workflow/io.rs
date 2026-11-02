use std::path::Path;

use crate::editor_model::submit_control::AdminFlagPayload;
use crate::linter::{LintSeverity, ServerLintIssue};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use studio_babel::{export_json, import_json};
use studio_core::Project;
use studio_project::{load_project, save_project};

use super::StudioWorkflow;
use crate::babel_metadata::META_DROP_CLAIM_DROPPED;

const META_SELECTED_COMPARE_ACTION_ID: &str = "studio.babel.diff.selectedCompareActionId";
const META_SHOW_REVIEWERS: &str = "studio.babel.diff.showReviewers";
const META_ADMIN_FLAG_RATING: &str = "studio.babel.adminFlag.rating";
const META_ADMIN_FLAG_REASON: &str = "studio.babel.adminFlag.reason";
const META_SKIP_REQUESTED: &str = "studio.babel.skip.requested";
const META_SUBMIT_FEEDBACK_INCOMPLETE: &str = "studio.babel.submit.hasFeedbackIncomplete";
const META_SUBMIT_FEEDBACK_SAVING: &str = "studio.babel.submit.isFeedbackSaving";
const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";

impl StudioWorkflow {
    pub fn from_babel_json(source: &str) -> Result<Self> {
        let project = import_json(source).context("failed to import Babel JSON")?;
        Ok(Self::from_project(project))
    }

    pub fn load_local_project(root: impl AsRef<Path>) -> Result<Self> {
        let project = load_project(root).context("failed to load local Studio project")?;
        Ok(Self::from_project(project))
    }

    pub fn save_local_project(&self, root: impl AsRef<Path>) -> Result<()> {
        self.clear_pending_boundary_edit();
        save_project(&self.project, root).context("failed to save local Studio project")
    }

    pub fn export_babel_json(&self) -> Result<String> {
        self.clear_pending_boundary_edit();
        let value = export_json(&self.project).context("failed to export Babel JSON")?;
        serde_json::to_string_pretty(&value).context("failed to encode Babel task JSON")
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn merge_project_metadata(&mut self, entries: impl IntoIterator<Item = (String, String)>) {
        self.project.metadata.extend(entries);
    }

    pub fn clear_babel_lint_snapshot_metadata(&mut self) {
        update_babel_lint_snapshot_metadata(&mut self.project, None);
    }

    pub fn set_babel_lint_snapshot_metadata(&mut self, issues: &[ServerLintIssue]) {
        update_babel_lint_snapshot_metadata(&mut self.project, Some(issues));
    }

    pub fn set_diff_compare_action(&mut self, action_id: impl Into<String>) {
        self.project.metadata.insert(
            META_SELECTED_COMPARE_ACTION_ID.to_string(),
            action_id.into(),
        );
    }

    pub fn toggle_diff_reviewers(&mut self) -> bool {
        let next = !self
            .project
            .metadata
            .get(META_SHOW_REVIEWERS)
            .is_some_and(|value| value == "true" || value == "1");
        self.project
            .metadata
            .insert(META_SHOW_REVIEWERS.to_string(), next.to_string());
        next
    }

    pub fn record_admin_flag(&mut self, payload: AdminFlagPayload) {
        self.project.metadata.insert(
            META_ADMIN_FLAG_RATING.to_string(),
            payload.rating.to_string(),
        );
        if let Some(reason) = payload.rating_reason {
            self.project
                .metadata
                .insert(META_ADMIN_FLAG_REASON.to_string(), reason);
        } else {
            self.project.metadata.remove(META_ADMIN_FLAG_REASON);
        }
    }

    pub fn mark_skip_requested(&mut self) {
        self.project
            .metadata
            .insert(META_SKIP_REQUESTED.to_string(), "true".to_string());
    }

    pub fn mark_claim_dropped(&mut self) {
        self.project
            .metadata
            .insert(META_DROP_CLAIM_DROPPED.to_string(), "true".to_string());
    }

    pub fn set_feedback_submit_state(&mut self, is_saving: bool, is_incomplete: bool) {
        self.project.metadata.insert(
            META_SUBMIT_FEEDBACK_SAVING.to_string(),
            is_saving.to_string(),
        );
        self.project.metadata.insert(
            META_SUBMIT_FEEDBACK_INCOMPLETE.to_string(),
            is_incomplete.to_string(),
        );
    }
}

fn update_babel_lint_snapshot_metadata(project: &mut Project, issues: Option<&[ServerLintIssue]>) {
    update_json_metadata_value(project, META_EXTRA_PACKAGE, |value| {
        let Some(object) = value.as_object_mut() else {
            return;
        };
        object.remove("lintErrors");
        match issues {
            Some(issues) => {
                object.insert("lint_errors".to_string(), server_lint_issues_value(issues));
            }
            None => {
                object.remove("lint_errors");
            }
        }
    });
    update_json_metadata_value(project, META_REVIEW_ACTION_ENVELOPE, |envelope| {
        if let Some(data) = review_action_data_object_mut(envelope) {
            data.remove("lint_errors");
            match issues {
                Some(issues) => {
                    data.insert("lintErrors".to_string(), server_lint_issues_value(issues));
                }
                None => {
                    data.remove("lintErrors");
                }
            }
        }
    });
}

fn update_json_metadata_value(project: &mut Project, key: &str, update: impl FnOnce(&mut Value)) {
    let Some(encoded) = project.metadata.get(key).cloned() else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&encoded) else {
        return;
    };
    update(&mut value);
    if let Ok(encoded) = serde_json::to_string(&value) {
        project.metadata.insert(key.to_string(), encoded);
    }
}

fn review_action_data_object_mut(
    envelope: &mut Value,
) -> Option<&mut serde_json::Map<String, Value>> {
    if envelope.is_array() {
        return envelope
            .as_array_mut()?
            .iter_mut()
            .find_map(review_action_data_object_mut);
    }
    if envelope.pointer("/result/data/json").is_some() {
        return envelope
            .pointer_mut("/result/data/json")
            .and_then(Value::as_object_mut);
    }
    if envelope.pointer("/data/json").is_some() {
        return envelope
            .pointer_mut("/data/json")
            .and_then(Value::as_object_mut);
    }
    envelope.pointer_mut("/json").and_then(Value::as_object_mut)
}

fn server_lint_issues_value(issues: &[ServerLintIssue]) -> Value {
    Value::Array(
        issues
            .iter()
            .map(|issue| {
                json!({
                    "annotationId": issue.annotation_id,
                    "reason": issue.reason,
                    "severity": match issue.severity {
                        LintSeverity::Warning => "warning",
                        LintSeverity::Error | LintSeverity::Pass => "error",
                    },
                })
            })
            .collect(),
    )
}
