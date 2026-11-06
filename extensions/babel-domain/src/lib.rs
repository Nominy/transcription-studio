//! Babel document policies, workflow and review presentation. The generic editor never imports this crate.
pub mod babel_metadata;
pub mod capabilities;
pub mod edit;
pub mod editor_model;
pub mod i18n;
pub mod presentation;
pub mod route;
pub mod server_diagnostics;
pub mod workflow;

use exaudio_extension::document::{
    DocumentEnvironment, DocumentOrigin, DocumentPolicy, DocumentProviders,
};
use std::sync::Arc;
use studio_core::Project;

/// Domain-only contributions. The host registers this factory under the Babel owner;
/// lint and tags remain independently registered extension factories.
pub fn domain_providers(project: &Project) -> DocumentProviders {
    let mut providers = DocumentProviders::default();
    providers
        .diagnostic_postprocessors
        .push(Arc::new(server_diagnostics::BabelDiagnosticPostprocessor));
    providers.mutation_policy = Some(Arc::new(workflow::BabelMutationPolicy::default()));
    providers
        .diagnostics
        .push(Arc::new(server_diagnostics::ServerDiagnosticProvider {
            snapshot: babel_metadata::server_lint_snapshot_from_project(project, 0),
        }));
    let acknowledgements = Arc::new(workflow::BabelLowConfidenceCommandProvider::default());
    let presentation = Arc::new(presentation::BabelPresentationProvider {
        acknowledgements: Arc::clone(&acknowledgements),
    });
    providers.decorations.push(presentation.clone());
    providers.segment_kinds.push(presentation);
    providers.commands.push(acknowledgements);
    providers
        .commands
        .push(Arc::new(edit::commands::BabelAnnotationCommandProvider));
    providers
}

/// Origin and policy only; activation and provider lifetime belong to the public registry.
pub fn document_environment(
    project: &Project,
    key: impl Into<String>,
    mode_id: &str,
) -> anyhow::Result<DocumentEnvironment> {
    anyhow::ensure!(
        matches!(
            mode_id,
            "exaudio.babel" | "exaudio.babel.errors" | "exaudio.babel-file"
        ),
        "Unknown Babel mode `{mode_id}`"
    );
    Ok(DocumentEnvironment {
        origin: DocumentOrigin {
            mode: mode_id.into(),
            key: key.into(),
        },
        policy: DocumentPolicy {
            read_only: babel_metadata::project_is_read_only(project),
            segment_edit_command: Some("exaudio.babel.edit-annotation".into()),
            minimum_cut_milliseconds: 1000,
            region_creation_command: edit::create_region::project_uses_annotation_mode(project)
                .then(|| "exaudio.babel.open-create-region".into()),
            number_markup: true,
            speakers_bound_to_lanes: true,
        },
        providers: DocumentProviders::default(),
    })
}
