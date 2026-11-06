use std::path::Path;

use anyhow::{Context, Result};
use studio_core::Project;
use studio_project::{load_project, save_project};

use super::StudioWorkflow;

impl StudioWorkflow {
    pub fn load_local_project(root: impl AsRef<Path>) -> Result<Self> {
        let project = load_project(root).context("failed to load local Exaudio project")?;
        Ok(Self::from_project(project))
    }

    pub fn save_local_project(
        &self,
        root: impl AsRef<Path>,
        source_base: impl AsRef<Path>,
    ) -> Result<()> {
        self.clear_pending_boundary_edit();
        save_project(&self.project, root, source_base)
            .context("failed to save local Exaudio project")
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn into_project(self) -> Project {
        self.project
    }

    /// Direct domain metadata access; undoable segment edits use `apply` instead.
    pub fn project_mut(&mut self) -> &mut Project {
        &mut self.project
    }

    pub fn merge_project_metadata(&mut self, entries: impl IntoIterator<Item = (String, String)>) {
        self.project.metadata.extend(entries);
    }
}
