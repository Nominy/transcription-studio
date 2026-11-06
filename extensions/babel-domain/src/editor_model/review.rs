use super::submit_control::SubmitBlocker;
use super::{EditorSession, EditorSessionView};
use crate::babel_metadata::project_is_babel_backed;
use crate::i18n::{count, localized_fmt};
use exaudio_babel_lint::linter::{
    LintSeverity, ServerLintIssue, UNRESOLVED_LOW_CONFIDENCE_RULE,
    server_lint_issue_matches_segment,
};
use exaudio_extension::document::{Diagnostic, Severity};
use studio_core::Project;
use studio_editor::transcript_model::diagnostic_key;

impl EditorSession<'_> {
    pub fn submit_review(&mut self, project: &Project) -> Result<(), String> {
        let blocking = self.view().submission_blocking_lint_count(project);
        if blocking > 0 {
            let remaining = count(
                blocking as u64,
                ("problem", "problems"),
                ("проблема", "проблемы", "проблем"),
            );
            return Err(localized_fmt(
                "Fix the problems first ({remaining} left)",
                "Сначала исправьте проблемы (осталось: {remaining})",
                &[("remaining", &remaining)],
            ));
        }
        if self.view().submission_unacknowledged_warning_count(project) > 0 {
            return Err(SubmitBlocker::UnacknowledgedWarnings.reason().into());
        }
        if self
            .view()
            .submission_unresolved_low_confidence_count(project)
            > 0
        {
            return Err(SubmitBlocker::UnresolvedLowConfidence.reason().into());
        }
        self.state.mark_submitted(self.core);
        Ok(())
    }
    pub fn toggle_warning_assertion(&mut self, project: &Project) -> bool {
        let Some(segment_id) = self.core.selected_segment_id.as_deref() else {
            return false;
        };
        if project.segment(segment_id).is_none() {
            return false;
        }
        let active_key = self
            .core
            .diagnostics()
            .iter()
            .flat_map(|collection| &collection.diagnostics)
            .filter(|issue| {
                issue.segment_id == segment_id
                    && issue.severity == Severity::Warning
                    && !(issue.source == "babel.gold"
                        && issue.rule == UNRESOLVED_LOW_CONFIDENCE_RULE)
            })
            .map(diagnostic_key)
            .find(|key| !self.core.asserted_warning_keys.contains(key));
        if let Some(key) = active_key {
            self.core.asserted_warning_keys.insert(key);
        } else {
            let prefix = format!("{segment_id}::");
            let Some(key) = self
                .core
                .asserted_warning_keys
                .iter()
                .find(|key| key.starts_with(&prefix))
                .cloned()
            else {
                return false;
            };
            self.core.asserted_warning_keys.remove(&key);
        }
        self.mark_dirty();
        self.core.refresh_diagnostics(project);
        true
    }
}

impl EditorSessionView<'_> {
    pub fn blocking_lint_count(&self, project: &Project) -> usize {
        self.core.blocking_lint_count(project)
    }
    pub fn unacknowledged_warning_count(&self, _project: &Project) -> usize {
        self.core
            .diagnostics()
            .iter()
            .flat_map(|collection| &collection.diagnostics)
            .filter(|issue| issue.severity == Severity::Warning)
            .filter(|issue| {
                !(issue.source == "babel.gold" && issue.rule == UNRESOLVED_LOW_CONFIDENCE_RULE)
            })
            .filter(|issue| {
                !self
                    .core
                    .asserted_warning_keys
                    .contains(&diagnostic_key(issue))
            })
            .count()
    }
    pub fn unresolved_low_confidence_count(&self, _project: &Project) -> usize {
        self.core
            .diagnostics()
            .iter()
            .flat_map(|collection| &collection.diagnostics)
            .filter(|issue| {
                issue.source == "babel.gold" && issue.rule == UNRESOLVED_LOW_CONFIDENCE_RULE
            })
            .count()
    }
    pub fn can_submit_review(&self, project: &Project) -> bool {
        self.submission_blocking_lint_count(project) == 0
            && self.submission_unacknowledged_warning_count(project) == 0
            && self.submission_unresolved_low_confidence_count(project) == 0
    }
    pub(super) fn submission_blocking_lint_count(&self, project: &Project) -> usize {
        if !project_is_babel_backed(project) {
            return self.blocking_lint_count(project);
        }
        self.server_lint_snapshot()
            .into_iter()
            .flat_map(|snapshot| &snapshot.issues)
            .filter(|issue| issue.severity == LintSeverity::Error)
            .count()
    }
    pub(super) fn submission_unacknowledged_warning_count(&self, project: &Project) -> usize {
        if !project_is_babel_backed(project) {
            return self.unacknowledged_warning_count(project);
        }
        self.server_lint_snapshot()
            .into_iter()
            .flat_map(|snapshot| &snapshot.issues)
            .filter(|issue| issue.severity == LintSeverity::Warning)
            .filter(|issue| !self.server_warning_is_acknowledged(project, issue))
            .count()
    }
    pub(super) fn submission_unresolved_low_confidence_count(&self, project: &Project) -> usize {
        if project_is_babel_backed(project) {
            0
        } else {
            self.unresolved_low_confidence_count(project)
        }
    }
    fn server_warning_is_acknowledged(&self, project: &Project, issue: &ServerLintIssue) -> bool {
        project
            .segments
            .iter()
            .find(|segment| server_lint_issue_matches_segment(issue, segment))
            .is_some_and(|segment| {
                self.core
                    .asserted_warning_keys
                    .contains(&diagnostic_key(&Diagnostic {
                        source: "babel.server".into(),
                        rule: "server".into(),
                        severity: Severity::Warning,
                        segment_id: segment.id.clone(),
                        range: None,
                        message: issue.reason.clone(),
                        fixes: Vec::new(),
                    }))
            })
    }
}
