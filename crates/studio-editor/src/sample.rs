use anyhow::{Context, Result};
use studio_babel::{BabelTaskPackage, import_package};
use studio_core::Project;

const SAMPLE_TASK_JSON: &str = include_str!("../assets/sample-task.json");

/// Bundled two-speaker demo task (demo mode / UI development).
pub fn load_sample_project() -> Result<Project> {
    let package: BabelTaskPackage =
        serde_json::from_str(SAMPLE_TASK_JSON).context("sample Babel fixture should parse")?;
    import_package(package).context("sample Babel fixture should import")
}
