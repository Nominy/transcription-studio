use super::EditorSession;
use crate::babel_metadata::project_is_babel_backed;
use crate::linter::{
    LintKind, LintSeverity, ServerLintIssue, UNRESOLVED_LOW_CONFIDENCE_RULE,
    server_lint_issue_matches_segment, warning_key,
};
use studio_core::Project;

impl EditorSession {
    pub fn blocking_lint_count(&self, project: &Project) -> usize {
        let row_issue_count = self
            .transcript_rows_for_filter(project, &crate::track_filter::TrackFilter::All)
            .iter()
            .map(|row| {
                row.lint_issues
                    .iter()
                    .filter(|issue| issue.severity == LintSeverity::Error)
                    .count()
            })
            .sum::<usize>();
        row_issue_count
            + self
                .server_lint_snapshot()
                .into_iter()
                .flat_map(|snapshot| &snapshot.issues)
                .filter(|issue| issue.severity == LintSeverity::Error)
                .filter(|issue| !server_issue_has_row(project, issue))
                .count()
    }

    fn unacknowledged_warning_issues(&self, project: &Project) -> Vec<crate::linter::LintStatus> {
        self.transcript_rows_for_filter(project, &crate::track_filter::TrackFilter::All)
            .into_iter()
            .flat_map(|row| row.lint_issues)
            .filter(|issue| issue.is_assertable_warning())
            .collect()
    }

    fn unresolved_low_confidence_issues(
        &self,
        project: &Project,
    ) -> Vec<crate::linter::LintStatus> {
        self.transcript_rows_for_filter(project, &crate::track_filter::TrackFilter::All)
            .into_iter()
            .flat_map(|row| row.lint_issues)
            .filter(|issue| issue.kind == LintKind::Gold(UNRESOLVED_LOW_CONFIDENCE_RULE))
            .collect()
    }

    pub fn unacknowledged_warning_count(&self, project: &Project) -> usize {
        self.unacknowledged_warning_issues(project).len()
            + self
                .server_lint_snapshot()
                .into_iter()
                .flat_map(|snapshot| &snapshot.issues)
                .filter(|issue| issue.severity == LintSeverity::Warning)
                .filter(|issue| !server_issue_has_row(project, issue))
                .count()
    }

    pub fn unresolved_low_confidence_count(&self, project: &Project) -> usize {
        self.unresolved_low_confidence_issues(project).len()
    }

    pub fn problem_row_count(&self, project: &Project) -> usize {
        self.problem_rows(project).len()
    }

    pub fn can_submit_review(&self, project: &Project) -> bool {
        self.submission_blocking_lint_count(project) == 0
            && self.submission_unacknowledged_warning_count(project) == 0
            && self.submission_unresolved_low_confidence_count(project) == 0
    }

    pub fn submit_review(&mut self, project: &Project) -> Result<(), String> {
        let blocking = self.submission_blocking_lint_count(project);
        if blocking > 0 {
            return Err(format!(
                "Please fix linter errors ({blocking} blocking lint issue(s) remain)"
            ));
        }
        if self.submission_unacknowledged_warning_count(project) > 0 {
            return Err("Please acknowledge all warnings".to_string());
        }
        if self.submission_unresolved_low_confidence_count(project) > 0 {
            return Err("Please review all low-confidence words".to_string());
        }
        self.review_state = super::ReviewState::Submitted;
        Ok(())
    }

    pub fn toggle_warning_assertion(&mut self, project: &Project) -> bool {
        let Some(segment_id) = self.selected_segment_id.as_deref() else {
            return false;
        };
        let Some(segment) = project.segment(segment_id) else {
            return false;
        };
        let active_key = self
            .transcript_rows_for_filter(project, &crate::track_filter::TrackFilter::All)
            .into_iter()
            .find(|row| row.segment_id == segment.id)
            .and_then(|row| {
                row.lint_issues
                    .iter()
                    .find_map(|issue| warning_key(segment, issue))
            });
        if let Some(key) = active_key {
            self.asserted_warning_keys.insert(key);
            self.mark_dirty();
            return true;
        }

        let key_prefix = format!("{}::", segment.id);
        let Some(key) = self
            .asserted_warning_keys
            .iter()
            .find(|key| key.starts_with(&key_prefix))
            .cloned()
        else {
            return false;
        };
        self.asserted_warning_keys.remove(&key);
        self.mark_dirty();
        true
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
                self.asserted_warning_keys
                    .contains(&format!("{}::{}", segment.id, issue.reason))
            })
    }
}

fn server_issue_has_row(project: &Project, issue: &ServerLintIssue) -> bool {
    project
        .segments
        .iter()
        .any(|segment| server_lint_issue_matches_segment(issue, segment))
}
