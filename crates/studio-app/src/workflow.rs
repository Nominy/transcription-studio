use std::path::Path;

use anyhow::{Context, Result};
use studio_babel::{BabelTaskPackage, export_package, import_package};
use studio_core::{CommandHistory, EditorCommand, Project, Segment};
use studio_project::{load_project, save_project};

#[derive(Clone, Debug)]
pub struct StudioWorkflow {
    project: Project,
    history: CommandHistory,
}

impl StudioWorkflow {
    pub fn from_babel_json(source: &str) -> Result<Self> {
        let package: BabelTaskPackage =
            serde_json::from_str(source).context("failed to parse Babel task JSON")?;
        let project = import_package(package).context("failed to import Babel task package")?;
        Ok(Self::from_project(project))
    }

    pub fn from_project(project: Project) -> Self {
        Self {
            project,
            history: CommandHistory::default(),
        }
    }

    pub fn load_local_project(root: impl AsRef<Path>) -> Result<Self> {
        let project = load_project(root).context("failed to load local Studio project")?;
        Ok(Self::from_project(project))
    }

    pub fn save_local_project(&self, root: impl AsRef<Path>) -> Result<()> {
        save_project(&self.project, root).context("failed to save local Studio project")
    }

    pub fn export_babel_json(&self) -> Result<String> {
        let package =
            export_package(&self.project).context("failed to export Babel task package")?;
        serde_json::to_string_pretty(&package).context("failed to encode Babel task JSON")
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn apply_text_edit(&mut self, segment_id: &str, text: impl Into<String>) -> Result<()> {
        self.apply(EditorCommand::ApplyTextEdit {
            segment_id: segment_id.to_string(),
            text: text.into(),
        })
    }

    pub fn move_segment_boundary(
        &mut self,
        segment_id: &str,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Result<()> {
        self.apply(EditorCommand::MoveSegmentBoundary {
            segment_id: segment_id.to_string(),
            start_seconds,
            end_seconds,
        })
    }

    pub fn split_segment(&mut self, segment_id: &str, at_seconds: f64) -> Result<String> {
        let new_segment_id = self.next_derived_segment_id(segment_id);
        self.apply(EditorCommand::SplitSegment {
            segment_id: segment_id.to_string(),
            at_seconds,
            new_segment_id: new_segment_id.clone(),
        })?;
        Ok(new_segment_id)
    }

    pub fn merge_with_next(&mut self, left_segment_id: &str) -> Result<()> {
        let left_index = self
            .project
            .segments
            .iter()
            .position(|segment| segment.id == left_segment_id)
            .with_context(|| format!("segment `{left_segment_id}` was not found"))?;
        let left = &self.project.segments[left_index];
        let right_segment_id = self
            .project
            .segments
            .iter()
            .skip(left_index + 1)
            .find(|segment| {
                segment.speaker_id == left.speaker_id
                    && segment.lane_id == left.lane_id
                    && left.end_seconds <= segment.start_seconds
            })
            .map(|segment| segment.id.clone())
            .with_context(|| {
                format!("segment `{left_segment_id}` has no next segment on its lane")
            })?;

        self.apply(EditorCommand::MergeSegments {
            left_segment_id: left_segment_id.to_string(),
            right_segment_id,
        })
    }

    pub fn delete_segment(&mut self, segment_id: &str) -> Result<()> {
        self.apply(EditorCommand::DeleteSegment {
            segment_id: segment_id.to_string(),
        })
    }

    pub fn create_segment_after(&mut self, anchor_segment_id: &str) -> Result<String> {
        let anchor = self
            .project
            .segment(anchor_segment_id)
            .with_context(|| format!("segment `{anchor_segment_id}` was not found"))?
            .clone();
        let new_segment_id = self.next_derived_segment_id(anchor_segment_id);
        let segment = Segment {
            id: new_segment_id.clone(),
            source_annotation_id: None,
            speaker_id: anchor.speaker_id,
            lane_id: anchor.lane_id,
            start_seconds: anchor.end_seconds,
            end_seconds: anchor.end_seconds + 1.0,
            text: String::new(),
        };

        self.apply(EditorCommand::CreateSegment { segment })?;
        Ok(new_segment_id)
    }

    pub fn change_segment_speaker(&mut self, segment_id: &str, speaker_id: &str) -> Result<()> {
        let lane_id = self
            .project
            .lanes
            .iter()
            .find(|lane| lane.speaker_id == speaker_id)
            .map(|lane| lane.id.clone())
            .with_context(|| format!("speaker `{speaker_id}` has no audio lane"))?;

        self.apply(EditorCommand::ChangeSegmentSpeaker {
            segment_id: segment_id.to_string(),
            speaker_id: speaker_id.to_string(),
            lane_id,
        })
    }

    pub fn undo(&mut self) -> Result<()> {
        self.history
            .undo(&mut self.project)
            .context("failed to undo command")
    }

    pub fn redo(&mut self) -> Result<()> {
        self.history
            .redo(&mut self.project)
            .context("failed to redo command")
    }

    fn apply(&mut self, command: EditorCommand) -> Result<()> {
        self.history
            .apply(&mut self.project, command)
            .context("failed to apply editor command")
    }

    fn next_derived_segment_id(&self, base_segment_id: &str) -> String {
        for index in 1.. {
            let candidate = format!("{base_segment_id}__studio_{index}");
            if !self.project.has_segment(&candidate) {
                return candidate;
            }
        }

        unreachable!("failed to allocate derived segment id")
    }
}
