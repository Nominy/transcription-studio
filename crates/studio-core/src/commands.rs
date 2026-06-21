use thiserror::Error;

use crate::{Project, Segment};

#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    ApplyTextEdit {
        segment_id: String,
        text: String,
    },
    MoveSegmentBoundary {
        segment_id: String,
        start_seconds: f64,
        end_seconds: f64,
    },
    SplitSegment {
        segment_id: String,
        at_seconds: f64,
        new_segment_id: String,
    },
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StudioCoreError {
    #[error("segment was not found")]
    SegmentNotFound,
    #[error("time range must have start before end")]
    InvalidTimeRange,
    #[error("new segment id already exists")]
    DuplicateSegmentId,
    #[error("split point must be inside the segment")]
    SplitOutsideSegment,
    #[error("there is no command to undo")]
    NothingToUndo,
    #[error("there is no command to redo")]
    NothingToRedo,
}

#[derive(Clone, Debug, Default)]
pub struct CommandHistory {
    undo_stack: Vec<Project>,
    redo_stack: Vec<Project>,
}

impl CommandHistory {
    pub fn apply(
        &mut self,
        project: &mut Project,
        command: EditorCommand,
    ) -> Result<(), StudioCoreError> {
        let previous = project.clone();
        apply_command(project, command)?;
        self.undo_stack.push(previous);
        self.redo_stack.clear();
        Ok(())
    }

    pub fn undo(&mut self, project: &mut Project) -> Result<(), StudioCoreError> {
        let previous = self
            .undo_stack
            .pop()
            .ok_or(StudioCoreError::NothingToUndo)?;
        let current = std::mem::replace(project, previous);
        self.redo_stack.push(current);
        Ok(())
    }

    pub fn redo(&mut self, project: &mut Project) -> Result<(), StudioCoreError> {
        let next = self
            .redo_stack
            .pop()
            .ok_or(StudioCoreError::NothingToRedo)?;
        let current = std::mem::replace(project, next);
        self.undo_stack.push(current);
        Ok(())
    }
}

fn apply_command(project: &mut Project, command: EditorCommand) -> Result<(), StudioCoreError> {
    match command {
        EditorCommand::ApplyTextEdit { segment_id, text } => {
            let segment = project
                .segment_mut(&segment_id)
                .ok_or(StudioCoreError::SegmentNotFound)?;
            segment.text = text;
            Ok(())
        }
        EditorCommand::MoveSegmentBoundary {
            segment_id,
            start_seconds,
            end_seconds,
        } => {
            validate_time_range(start_seconds, end_seconds)?;
            let segment = project
                .segment_mut(&segment_id)
                .ok_or(StudioCoreError::SegmentNotFound)?;
            segment.start_seconds = start_seconds;
            segment.end_seconds = end_seconds;
            Ok(())
        }
        EditorCommand::SplitSegment {
            segment_id,
            at_seconds,
            new_segment_id,
        } => split_segment(project, &segment_id, at_seconds, new_segment_id),
    }
}

fn split_segment(
    project: &mut Project,
    segment_id: &str,
    at_seconds: f64,
    new_segment_id: String,
) -> Result<(), StudioCoreError> {
    if project.has_segment(&new_segment_id) {
        return Err(StudioCoreError::DuplicateSegmentId);
    }

    let index = project
        .segments
        .iter()
        .position(|segment| segment.id == segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    let original_end = project.segments[index].end_seconds;
    let original_start = project.segments[index].start_seconds;

    validate_time_range(original_start, at_seconds)?;
    validate_time_range(at_seconds, original_end)?;

    if at_seconds <= original_start || at_seconds >= original_end {
        return Err(StudioCoreError::SplitOutsideSegment);
    }

    let right = Segment {
        id: new_segment_id,
        source_annotation_id: None,
        speaker_id: project.segments[index].speaker_id.clone(),
        lane_id: project.segments[index].lane_id.clone(),
        start_seconds: at_seconds,
        end_seconds: original_end,
        text: String::new(),
    };

    project.segments[index].end_seconds = at_seconds;
    project.segments.insert(index + 1, right);
    Ok(())
}

fn validate_time_range(start_seconds: f64, end_seconds: f64) -> Result<(), StudioCoreError> {
    if start_seconds.is_finite() && end_seconds.is_finite() && start_seconds < end_seconds {
        Ok(())
    } else {
        Err(StudioCoreError::InvalidTimeRange)
    }
}
