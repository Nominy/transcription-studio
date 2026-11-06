//! Babel Gold and Helper rules, language configuration, and document diagnostics.
pub mod data;
pub mod linter;
pub mod project_lint_tag_config;

use std::sync::Arc;

use exaudio_extension::document::{
    Diagnostic, DiagnosticCollection, DiagnosticProvider, DocumentProviders, Severity,
};
use linter::{
    LintKind, LintSeverity, ProjectLintAnalysis, ServerLintSnapshot,
    combine_server_and_local_issues, server_lint_issues_for_segment,
};
use studio_core::Project;

pub const ID: &str = "exaudio.babel-lint";
pub const MANIFEST_JSON: &str = include_str!("../manifest.json");

pub fn manifest() -> Result<exaudio_extension::manifest::ExtensionManifest, serde_json::Error> {
    serde_json::from_str(MANIFEST_JSON)
}

/// Server issues retain their revision; stale server results never shadow local lint.
#[derive(Debug, Default)]
pub struct BabelDiagnosticProvider {
    pub server: Option<ServerLintSnapshot>,
}

impl DiagnosticProvider for BabelDiagnosticProvider {
    fn diagnose(&self, project: &Project, revision: u64) -> DiagnosticCollection {
        let analysis = ProjectLintAnalysis::new(project);
        let server = self
            .server
            .as_ref()
            .filter(|snapshot| snapshot.revision == revision);
        let mut diagnostics = Vec::new();
        for segment in &project.segments {
            for issue in combine_server_and_local_issues(
                server_lint_issues_for_segment(server, segment),
                analysis.lint_segment_issues(segment),
            ) {
                let severity = match issue.severity {
                    LintSeverity::Pass => continue,
                    LintSeverity::Warning => Severity::Warning,
                    LintSeverity::Error => Severity::Error,
                };
                let (source, rule) = match issue.kind {
                    LintKind::Pass => continue,
                    LintKind::BabelServer => ("babel.server", "server"),
                    LintKind::Gold(rule) => ("babel.gold", rule),
                    LintKind::Helper(rule) => ("babel.helper", rule),
                };
                diagnostics.push(Diagnostic {
                    source: source.into(),
                    rule: rule.into(),
                    severity,
                    segment_id: segment.id.clone(),
                    range: None,
                    message: issue.message,
                    fixes: Vec::new(),
                });
            }
        }
        if let Some(server) = server {
            let associated_ids = project
                .segments
                .iter()
                .flat_map(|segment| {
                    std::iter::once(segment.id.as_str())
                        .chain(segment.source_annotation_id.as_deref())
                })
                .collect::<std::collections::HashSet<_>>();
            for issue in &server.issues {
                if associated_ids.contains(issue.annotation_id.as_str()) {
                    continue;
                }
                let severity = match issue.severity {
                    LintSeverity::Pass => continue,
                    LintSeverity::Warning => Severity::Warning,
                    LintSeverity::Error => Severity::Error,
                };
                diagnostics.push(Diagnostic {
                    source: "babel.server".into(),
                    rule: "server".into(),
                    severity,
                    segment_id: issue.annotation_id.clone(),
                    range: None,
                    message: issue.reason.clone(),
                    fixes: Vec::new(),
                });
            }
        }
        DiagnosticCollection {
            source: "babel.lint".into(),
            revision,
            diagnostics,
        }
    }
}

/// Explicit registration: local documents do not acquire Babel behavior by importing the host.
pub fn register(providers: &mut DocumentProviders) {
    providers
        .diagnostics
        .push(Arc::new(BabelDiagnosticProvider::default()));
}
