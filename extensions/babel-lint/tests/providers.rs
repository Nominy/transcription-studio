use exaudio_babel_lint::{
    BabelDiagnosticProvider,
    linter::{LintSeverity, ServerLintIssue, ServerLintSnapshot},
};
use exaudio_extension::document::{DiagnosticProvider, DocumentProviders, Severity};
use studio_core::Project;

fn empty_project() -> Project {
    Project {
        id: "task".into(),
        name: "Task".into(),
        speakers: Vec::new(),
        lanes: Vec::new(),
        segments: Vec::new(),
        metadata: Default::default(),
    }
}

#[test]
fn unassociated_server_issues_remain_blocking_and_keep_multiplicity() {
    let provider = BabelDiagnosticProvider {
        server: Some(ServerLintSnapshot::new(
            7,
            vec![
                ServerLintIssue {
                    annotation_id: "missing".into(),
                    reason: "Invalid annotation".into(),
                    severity: LintSeverity::Error,
                },
                ServerLintIssue {
                    annotation_id: "missing".into(),
                    reason: "Invalid annotation".into(),
                    severity: LintSeverity::Error,
                },
            ],
        )),
    };
    let collection = provider.diagnose(&empty_project(), 7);
    assert_eq!(collection.source, "babel.lint");
    assert_eq!(collection.revision, 7);
    assert_eq!(collection.diagnostics.len(), 2);
    assert!(
        collection
            .diagnostics
            .iter()
            .all(|issue| issue.segment_id == "missing"
                && issue.source == "babel.server"
                && issue.severity == Severity::Error)
    );
    assert!(
        provider
            .diagnose(&empty_project(), 8)
            .diagnostics
            .is_empty()
    );
}

#[test]
fn babel_diagnostics_require_explicit_registration() {
    let mut providers = DocumentProviders::default();
    assert!(providers.diagnostics.is_empty());
    exaudio_babel_lint::register(&mut providers);
    assert_eq!(providers.diagnostics.len(), 1);
}
