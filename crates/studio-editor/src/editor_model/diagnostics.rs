use super::EditorSession;
use crate::transcript_model::diagnostic_key;
use exaudio_extension::document::Severity;
use studio_core::Project;

impl EditorSession {
    pub fn blocking_lint_count(&self, _project: &Project) -> usize {
        self.diagnostics()
            .iter()
            .flat_map(|collection| &collection.diagnostics)
            .filter(|issue| issue.severity == Severity::Error)
            .count()
    }

    pub fn unacknowledged_warning_count(&self, _project: &Project) -> usize {
        self.diagnostics()
            .iter()
            .flat_map(|collection| &collection.diagnostics)
            .filter(|issue| issue.severity == Severity::Warning)
            .filter(|issue| !self.asserted_warning_keys.contains(&diagnostic_key(issue)))
            .count()
    }

    pub fn problem_row_count(&self, project: &Project) -> usize {
        self.problem_rows(project).len()
    }
}
