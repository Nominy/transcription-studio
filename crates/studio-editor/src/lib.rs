//! UI-free transcription editor domain: workflow, lint, models, editing operations.

mod babel_metadata;
pub mod capabilities;
pub mod edit;
pub mod editor_model;
pub mod linter;
pub mod project_lint_tag_config;
mod sample;
pub mod timeline_model;
pub mod track_filter;
pub mod transcript_model;
pub mod workflow;

pub use sample::load_sample_project;
