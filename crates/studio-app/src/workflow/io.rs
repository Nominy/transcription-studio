use std::path::Path;

use crate::AdminFlagPayload;
use anyhow::{Context, Result};
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
