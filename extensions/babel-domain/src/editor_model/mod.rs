pub mod context_panels;
pub mod diff_toolbar;
pub mod feedback_assessment;
mod review;
pub mod review_actions;
pub mod submit_control;
pub mod task_lookup;
use exaudio_babel_lint::linter::ServerLintSnapshot;
use studio_core::Project;
use studio_editor::editor_model::EditorSession as CoreSession;

#[derive(Clone, Debug, PartialEq)]
pub struct BabelDocumentState {
    pub review_state: ReviewState,
    pub support_id: String,
    server_lint_snapshot: Option<ServerLintSnapshot>,
    review_revision: u64,
}
impl BabelDocumentState {
    pub fn for_project(project: &Project) -> Self {
        Self {
            review_state: ReviewState::from_project(project),
            support_id: support_id_for_project(project),
            server_lint_snapshot: crate::babel_metadata::server_lint_snapshot_from_project(
                project, 0,
            ),
            review_revision: 0,
        }
    }
    pub fn review_state(&self, core: &CoreSession) -> ReviewState {
        if self.review_state == ReviewState::Submitted
            && self.review_revision != core.edit_revision()
        {
            ReviewState::Draft
        } else {
            self.review_state
        }
    }
    pub fn mark_submitted(&mut self, core: &CoreSession) {
        self.review_state = ReviewState::Submitted;
        self.review_revision = core.edit_revision();
    }
    pub fn server_lint_snapshot(&self, core: &CoreSession) -> Option<&ServerLintSnapshot> {
        self.server_lint_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.revision == core.edit_revision())
    }
    pub fn view<'a>(&'a self, core: &'a CoreSession) -> EditorSessionView<'a> {
        EditorSessionView { core, state: self }
    }
    pub fn bind<'a>(&'a mut self, core: &'a mut CoreSession) -> EditorSession<'a> {
        self.review_state = self.review_state(core);
        EditorSession { core, state: self }
    }
}

/// A scoped domain view over the document's single, authoritative core session.
pub struct EditorSession<'a> {
    pub core: &'a mut CoreSession,
    pub state: &'a mut BabelDocumentState,
}
impl std::ops::Deref for EditorSession<'_> {
    type Target = CoreSession;
    fn deref(&self) -> &Self::Target {
        self.core
    }
}
impl std::ops::DerefMut for EditorSession<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.core
    }
}
impl EditorSession<'_> {
    pub fn server_lint_snapshot(&self) -> Option<&ServerLintSnapshot> {
        self.state
            .server_lint_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.revision == self.core.edit_revision())
    }
    pub fn set_server_lint_snapshot(
        &mut self,
        project: &Project,
        snapshot: ServerLintSnapshot,
    ) -> bool {
        if snapshot.revision != self.core.edit_revision() {
            return false;
        }
        use exaudio_extension::document::DiagnosticProvider;
        let provider = crate::server_diagnostics::ServerDiagnosticProvider {
            snapshot: Some(snapshot.clone()),
        };
        self.core
            .publish_diagnostics(project, provider.diagnose(project, snapshot.revision));
        self.state.server_lint_snapshot = Some(snapshot);
        true
    }
    pub fn mark_dirty(&mut self) {
        self.core.mark_dirty();
        self.state.server_lint_snapshot = None;
        if self.state.review_state == ReviewState::Submitted {
            self.state.review_state = ReviewState::Draft;
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    Draft,
    InProgress,
    Submitted,
}

impl ReviewState {
    fn from_project(project: &Project) -> Self {
        if project.metadata.get("babel.format").map(String::as_str) != Some("review-action") {
            return Self::Draft;
        }

        match project
            .metadata
            .get("babel.review_action.actionDecision")
            .map(String::as_str)
        {
            Some("pass") => Self::Submitted,
            Some("in-progress") => Self::InProgress,
            _ => Self::Draft,
        }
    }
}

/// Practice/assessment phase of an assessment task (from project metadata).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssessmentPhase {
    Practice,
    Assessment,
}

impl AssessmentPhase {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "practice" => Some(Self::Practice),
            "assessment" => Some(Self::Assessment),
            _ => None,
        }
    }
}

fn support_id_for_project(project: &Project) -> String {
    project
        .segments
        .iter()
        .find_map(|segment| {
            segment
                .metadata
                .get("reviewActionId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| project.metadata.get("action_id").cloned())
        .unwrap_or_else(|| format!("{}:{}", project.id, project.segments.len()))
}

pub struct EditorSessionView<'a> {
    pub core: &'a CoreSession,
    pub state: &'a BabelDocumentState,
}
impl std::ops::Deref for EditorSessionView<'_> {
    type Target = CoreSession;
    fn deref(&self) -> &Self::Target {
        self.core
    }
}
impl EditorSessionView<'_> {
    pub fn review_state(&self) -> ReviewState {
        self.state.review_state(self.core)
    }
    pub fn server_lint_snapshot(&self) -> Option<&ServerLintSnapshot> {
        self.state
            .server_lint_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.revision == self.core.edit_revision())
    }
}
impl EditorSession<'_> {
    pub fn view(&self) -> EditorSessionView<'_> {
        self.state.view(self.core)
    }
    pub fn submit_eligibility(&self, project: &Project) -> submit_control::SubmitEligibility {
        self.view().submit_eligibility(project)
    }
    pub fn diff_toolbar(&self, project: &Project) -> diff_toolbar::DiffToolbarState {
        self.view().diff_toolbar(project)
    }
    pub fn can_submit_review(&self, project: &Project) -> bool {
        self.view().can_submit_review(project)
    }
    pub fn unresolved_low_confidence_count(&self, project: &Project) -> usize {
        self.view().unresolved_low_confidence_count(project)
    }
}
