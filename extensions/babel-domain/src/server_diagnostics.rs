//! Authoritative review-server diagnostics, independent of optional local lint engines.
use exaudio_babel_lint::linter::{LintSeverity, ServerLintSnapshot};
use exaudio_extension::document::{Diagnostic, DiagnosticCollection, DiagnosticProvider, Severity};
use std::collections::BTreeMap;
use studio_core::Project;

#[derive(Debug, Default)]
pub struct ServerDiagnosticProvider {
    pub snapshot: Option<ServerLintSnapshot>,
}

impl DiagnosticProvider for ServerDiagnosticProvider {
    fn diagnose(&self, project: &Project, revision: u64) -> DiagnosticCollection {
        let mut collection = DiagnosticCollection {
            source: "babel.server".into(),
            revision,
            diagnostics: Vec::new(),
        };
        let Some(snapshot) = self
            .snapshot
            .as_ref()
            .filter(|snapshot| snapshot.revision == revision)
        else {
            return collection;
        };
        if snapshot.issues.is_empty() {
            return collection;
        }
        let mut segment_ids: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for segment in &project.segments {
            segment_ids
                .entry(&segment.id)
                .or_default()
                .push(&segment.id);
            if let Some(source) = segment
                .source_annotation_id
                .as_deref()
                .filter(|source| *source != segment.id)
            {
                segment_ids.entry(source).or_default().push(&segment.id);
            }
        }
        for issue in &snapshot.issues {
            let severity = match issue.severity {
                LintSeverity::Error => Severity::Error,
                LintSeverity::Warning => Severity::Warning,
                LintSeverity::Pass => continue,
            };
            let diagnostic = |segment_id: &str| Diagnostic {
                source: "babel.server".into(),
                rule: "server".into(),
                severity,
                segment_id: segment_id.to_owned(),
                range: None,
                message: issue.reason.clone(),
                fixes: Vec::new(),
            };
            if let Some(ids) = segment_ids.get(issue.annotation_id.as_str()) {
                collection
                    .diagnostics
                    .extend(ids.iter().map(|id| diagnostic(id)));
            } else {
                // Global and malformed server blockers remain visible to problem counts even
                // when no transcript row can display them.
                collection
                    .diagnostics
                    .push(diagnostic(&issue.annotation_id));
            }
        }
        collection
    }
}

/// Server results lead Babel's local suggestions; unrelated extension diagnostics retain
/// their identity, even when they happen to have the same message.
#[derive(Debug)]
pub struct BabelDiagnosticPostprocessor;

impl exaudio_extension::document::DiagnosticPostprocessor for BabelDiagnosticPostprocessor {
    fn process(
        &self,
        _project: &Project,
        revision: u64,
        collections: &mut Vec<DiagnosticCollection>,
    ) {
        let Some(index) = collections.iter().position(|collection| {
            collection.source == "babel.server" && collection.revision == revision
        }) else {
            return;
        };
        if index != 0 {
            let server = collections.remove(index);
            collections.insert(0, server);
        }
        let Some((server, remaining)) = collections.split_first_mut() else {
            return;
        };
        let reported: std::collections::BTreeSet<_> = server
            .diagnostics
            .iter()
            .map(|issue| {
                (
                    issue.segment_id.as_str(),
                    issue.message.as_str(),
                    issue.severity,
                )
            })
            .collect();
        for collection in remaining {
            if collection.source != "babel.lint" {
                continue;
            }
            collection.diagnostics.retain(|issue| {
                !matches!(issue.source.as_str(), "babel.gold" | "babel.helper")
                    || !reported.contains(&(
                        issue.segment_id.as_str(),
                        issue.message.as_str(),
                        issue.severity,
                    ))
            });
        }
    }
}
