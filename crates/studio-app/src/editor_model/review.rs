use super::EditorSession;
use crate::linter::{LintKind, LintSeverity, lint_segment_issues, warning_key};
use studio_core::Project;

impl EditorSession {
    pub fn blocking_lint_count(&self, project: &Project) -> usize {
        self.transcript_rows_for_filter(project, &crate::track_filter::TrackFilter::All)
            .iter()
            .map(|row| {
                row.lint_issues
                    .iter()
                    .filter(|issue| issue.severity == LintSeverity::Error)
                    .count()
            })
            .sum()
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
            .filter(|issue| issue.kind == LintKind::UnresolvedLowConfidence)
            .collect()
    }

    pub fn unacknowledged_warning_count(&self, project: &Project) -> usize {
        self.unacknowledged_warning_issues(project).len()
    }

    pub fn unresolved_low_confidence_count(&self, project: &Project) -> usize {
        self.unresolved_low_confidence_issues(project).len()
    }

    pub fn problem_row_count(&self, project: &Project) -> usize {
        self.problem_rows(project).len()
    }

    pub fn can_submit_review(&self, project: &Project) -> bool {
        self.blocking_lint_count(project) == 0
            && self.unacknowledged_warning_count(project) == 0
            && self.unresolved_low_confidence_count(project) == 0
    }

    pub fn submit_review(&mut self, project: &Project) -> Result<(), String> {
        let blocking = self.blocking_lint_count(project);
        if blocking > 0 {
            return Err(format!(
                "Please fix linter errors ({blocking} blocking lint issue(s) remain)"
            ));
        }
        if self.unacknowledged_warning_count(project) > 0 {
            return Err("Please acknowledge all warnings".to_string());
        }
        if self.unresolved_low_confidence_count(project) > 0 {
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
        let Some(key) = lint_segment_issues(project, segment)
            .iter()
            .find_map(|issue| warning_key(segment, issue))
        else {
            return false;
        };

        if !self.asserted_warning_keys.insert(key.clone()) {
            self.asserted_warning_keys.remove(&key);
        }
        true
    }
}
