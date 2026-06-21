use thiserror::Error;

use crate::{Project, Segment};

#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    CreateSegment {
        segment: Segment,
    },
    DeleteSegment {
        segment_id: String,
    },
    MergeSegments {
        left_segment_id: String,
        right_segment_id: String,
    },
    ChangeSegmentSpeaker {
        segment_id: String,
        speaker_id: String,
        lane_id: String,
    },
    ApplyTextEdit {
        segment_id: String,
        text: String,
    },
    UpdateSegment {
        segment_id: String,
        start_seconds: f64,
        end_seconds: f64,
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
    #[error("segments must belong to the same speaker and lane")]
    SegmentTrackMismatch,
    #[error("left segment must end before the right segment starts")]
    SegmentOrderInvalid,
    #[error("speaker was not found")]
    SpeakerNotFound,
    #[error("lane was not found")]
    LaneNotFound,
    #[error("lane does not belong to the speaker")]
    LaneSpeakerMismatch,
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
        EditorCommand::CreateSegment { segment } => create_segment(project, segment),
        EditorCommand::DeleteSegment { segment_id } => delete_segment(project, &segment_id),
        EditorCommand::MergeSegments {
            left_segment_id,
            right_segment_id,
        } => merge_segments(project, &left_segment_id, &right_segment_id),
        EditorCommand::ChangeSegmentSpeaker {
            segment_id,
            speaker_id,
            lane_id,
        } => change_segment_speaker(project, &segment_id, speaker_id, lane_id),
        EditorCommand::ApplyTextEdit { segment_id, text } => {
            let segment = project
                .segment_mut(&segment_id)
                .ok_or(StudioCoreError::SegmentNotFound)?;
            segment.text = text;
            Ok(())
        }
        EditorCommand::UpdateSegment {
            segment_id,
            start_seconds,
            end_seconds,
            text,
        } => update_segment(project, &segment_id, start_seconds, end_seconds, text),
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

fn create_segment(project: &mut Project, segment: Segment) -> Result<(), StudioCoreError> {
    if project.has_segment(&segment.id) {
        return Err(StudioCoreError::DuplicateSegmentId);
    }
    validate_time_range(segment.start_seconds, segment.end_seconds)?;
    validate_track(project, &segment.speaker_id, &segment.lane_id)?;
    project.segments.push(segment);
    Ok(())
}

fn delete_segment(project: &mut Project, segment_id: &str) -> Result<(), StudioCoreError> {
    let index = project
        .segments
        .iter()
        .position(|segment| segment.id == segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    project.segments.remove(index);
    Ok(())
}

fn update_segment(
    project: &mut Project,
    segment_id: &str,
    start_seconds: f64,
    end_seconds: f64,
    text: String,
) -> Result<(), StudioCoreError> {
    validate_time_range(start_seconds, end_seconds)?;
    let segment = project
        .segment_mut(segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    segment.start_seconds = start_seconds;
    segment.end_seconds = end_seconds;
    segment.text = text;
    Ok(())
}

fn merge_segments(
    project: &mut Project,
    left_segment_id: &str,
    right_segment_id: &str,
) -> Result<(), StudioCoreError> {
    let left_index = project
        .segments
        .iter()
        .position(|segment| segment.id == left_segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    let right_index = project
        .segments
        .iter()
        .position(|segment| segment.id == right_segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;

    if left_index == right_index {
        return Err(StudioCoreError::SegmentOrderInvalid);
    }

    let left = project.segments[left_index].clone();
    let right = project.segments[right_index].clone();

    if left.speaker_id != right.speaker_id || left.lane_id != right.lane_id {
        return Err(StudioCoreError::SegmentTrackMismatch);
    }
    if left.end_seconds > right.start_seconds {
        return Err(StudioCoreError::SegmentOrderInvalid);
    }

    let merged_text = merge_text(&left.text, &right.text);
    let left_segment = &mut project.segments[left_index];
    left_segment.start_seconds = left.start_seconds;
    left_segment.end_seconds = right.end_seconds;
    left_segment.text = merged_text;
    left_segment.source_annotation_id = left.source_annotation_id;

    project.segments.remove(right_index);
    Ok(())
}

fn change_segment_speaker(
    project: &mut Project,
    segment_id: &str,
    speaker_id: String,
    lane_id: String,
) -> Result<(), StudioCoreError> {
    project
        .segment(segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;

    if !project
        .speakers
        .iter()
        .any(|speaker| speaker.id == speaker_id)
    {
        return Err(StudioCoreError::SpeakerNotFound);
    }

    let lane = project
        .lanes
        .iter()
        .find(|lane| lane.id == lane_id)
        .ok_or(StudioCoreError::LaneNotFound)?;
    if lane.speaker_id != speaker_id {
        return Err(StudioCoreError::LaneSpeakerMismatch);
    }

    let segment = project
        .segment_mut(segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    segment.speaker_id = speaker_id;
    segment.lane_id = lane_id;
    Ok(())
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

fn merge_text(left: &str, right: &str) -> String {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => String::new(),
        (true, false) => right.to_string(),
        (false, true) => left.to_string(),
        (false, false) => format!("{left} {right}"),
    }
}

fn validate_time_range(start_seconds: f64, end_seconds: f64) -> Result<(), StudioCoreError> {
    if start_seconds.is_finite() && end_seconds.is_finite() && start_seconds < end_seconds {
        Ok(())
    } else {
        Err(StudioCoreError::InvalidTimeRange)
    }
}

fn validate_track(
    project: &Project,
    speaker_id: &str,
    lane_id: &str,
) -> Result<(), StudioCoreError> {
    if !project
        .speakers
        .iter()
        .any(|speaker| speaker.id == speaker_id)
    {
        return Err(StudioCoreError::SpeakerNotFound);
    }

    let lane = project
        .lanes
        .iter()
        .find(|lane| lane.id == lane_id)
        .ok_or(StudioCoreError::LaneNotFound)?;
    if lane.speaker_id != speaker_id {
        return Err(StudioCoreError::LaneSpeakerMismatch);
    }

    Ok(())
}
