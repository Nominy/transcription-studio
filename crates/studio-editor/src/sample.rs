use anyhow::{Context, Result};
use studio_core::Project;

const SAMPLE_PROJECT_JSON: &str = include_str!("../assets/sample-project.json");

/// Bundled native synthetic demo; opens without any installed extensions.
pub fn load_sample_project() -> Result<Project> {
    serde_json::from_str(SAMPLE_PROJECT_JSON).context("native synthetic demo should parse")
}
