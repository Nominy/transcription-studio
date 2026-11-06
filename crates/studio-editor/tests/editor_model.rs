use exaudio_extension::document::{
    Diagnostic, DiagnosticCollection, DiagnosticProvider, DocumentEnvironment, Severity,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use studio_core::{Project, Segment};
use studio_editor::editor_model::EditorSession;

fn project() -> Project {
    Project {
        id: "local".into(),
        name: "Local".into(),
        speakers: Vec::new(),
        lanes: Vec::new(),
        segments: vec![Segment {
            id: "segment".into(),
            source_annotation_id: None,
            speaker_id: "speaker".into(),
            lane_id: "lane".into(),
            start_seconds: 0.0,
            end_seconds: 1.0,
            text: String::new(),
            metadata: Default::default(),
        }],
        metadata: Default::default(),
    }
}

fn collection(revision: u64, severity: Severity) -> DiagnosticCollection {
    DiagnosticCollection {
        source: "test.provider".into(),
        revision,
        diagnostics: vec![Diagnostic {
            source: "test.provider".into(),
            rule: "empty-text".into(),
            severity,
            segment_id: "segment".into(),
            range: None,
            message: "Empty text".into(),
            fixes: Vec::new(),
        }],
    }
}

#[derive(Debug)]
struct CountingProvider(Arc<AtomicUsize>);
impl DiagnosticProvider for CountingProvider {
    fn diagnose(&self, _: &Project, revision: u64) -> DiagnosticCollection {
        self.0.fetch_add(1, Ordering::SeqCst);
        collection(revision, Severity::Warning)
    }
}

#[test]
fn local_documents_have_no_implicit_diagnostics_or_review_providers() {
    let project = project();
    let session = EditorSession::for_project(&project);
    assert!(session.environment().providers.diagnostics.is_empty());
    assert!(session.environment().providers.decorations.is_empty());
    assert!(session.environment().providers.commands.is_empty());
    assert!(session.diagnostics().is_empty());
    let row = session.transcript_rows(&project).remove(0);
    assert!(row.lint.is_none());
    assert!(row.lint_issues.is_empty());
    assert!(row.diff.is_none());
    assert!(row.reviewer.is_none());
}

#[test]
fn rendering_never_invokes_diagnostic_providers_and_edits_drop_stale_results() {
    let project = project();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut environment = DocumentEnvironment::local(project.id.clone());
    environment
        .providers
        .diagnostics
        .push(Arc::new(CountingProvider(calls.clone())));
    let mut session = EditorSession::for_project_with_environment(&project, environment);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        session.transcript_rows(&project)[0]
            .lint
            .as_ref()
            .unwrap()
            .severity,
        Severity::Warning
    );
    session.transcript_rows_for_render(&project);
    session.problem_counts(&project);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    session.mark_dirty();
    assert!(session.diagnostics().is_empty());
    assert!(!session.publish_diagnostics(&project, collection(0, Severity::Error)));
    session.refresh_diagnostics(&project);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(session.diagnostics()[0].revision, 1);
    session.remove_diagnostics(&project, "test.provider");
    assert!(session.diagnostics().is_empty());
}

#[test]
fn informational_and_hint_diagnostics_are_not_clean_results() {
    let project = project();
    let mut session = EditorSession::for_project(&project);
    for severity in [Severity::Info, Severity::Hint] {
        assert!(session.publish_diagnostics(&project, collection(0, severity)));
        let row = session.transcript_rows(&project).remove(0);
        assert_eq!(row.lint.as_ref().unwrap().severity, severity);
        assert_eq!(row.lint_issues.len(), 1);
        assert!(session.problem_counts(&project).is_empty());
    }
}

#[test]
fn read_only_policy_disables_row_mutations_without_metadata_interpretation() {
    let project = project();
    let mut environment = DocumentEnvironment::local(project.id.clone());
    environment.policy.read_only = true;
    let session = EditorSession::for_project_with_environment(&project, environment);
    let row = session.transcript_rows(&project).remove(0);
    assert!(row.is_read_only);
    assert!(!row.can_insert_above && !row.can_insert_below);
    assert!(!row.can_merge_above && !row.can_merge_below);
}
