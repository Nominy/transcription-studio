use std::collections::{BTreeMap, VecDeque};

use thiserror::Error;

use crate::{Project, Segment, Speaker};

/// Domain-owned side effects applied inside the same undo unit as a segment edit.
pub trait SegmentMutationPolicy: std::fmt::Debug + Send + Sync {
    fn updated(&self, _segment: &mut Segment) {}
    fn created(&self, _segment: &mut Segment) {}
    fn split(&self, segment: &mut Segment) {
        self.updated(segment);
    }
    fn split_gap_seconds(&self) -> f64 {
        0.0
    }
}

#[derive(Debug)]
pub struct NativeSegmentMutationPolicy;
impl SegmentMutationPolicy for NativeSegmentMutationPolicy {}

#[derive(Clone, Debug, PartialEq)]
pub struct SegmentUpdate {
    pub segment_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

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
    MergeSegmentsKeepingRight {
        left_segment_id: String,
        right_segment_id: String,
    },
    /// Assigns a speaker to a segment and places it on `lane_id`. A lane's own `speaker_id`
    /// is only the default for new segments: any existing speaker may be assigned on any lane.
    ChangeSegmentSpeaker {
        segment_id: String,
        speaker_id: String,
        lane_id: String,
    },
    AddSpeaker {
        speaker: Speaker,
    },
    RenameSpeaker {
        speaker_id: String,
        label: String,
    },
    UpdateSegmentMetadata {
        segment_id: String,
        text: Option<String>,
        entries: BTreeMap<String, serde_json::Value>,
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
    UpdateSegments {
        updates: Vec<SegmentUpdate>,
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
    SplitSegmentWithText {
        segment_id: String,
        at_seconds: f64,
        new_segment_id: String,
        left_text: String,
        right_text: String,
    },
    /// Removes `[start, end)` from a lane. A segment covering both edges is split around the
    /// range: with `split_texts` its halves get those texts (smart split), otherwise both keep
    /// the full text.
    CutLaneRange {
        lane_id: String,
        start_seconds: f64,
        end_seconds: f64,
        split_texts: Option<(String, String)>,
    },
    /// Removes `remove_segment_ids` and adds `segments` as one undo step (Gold Draft writing
    /// drafted rows over a lane's transcript). Validated before any change.
    ReplaceSegments {
        remove_segment_ids: Vec<String>,
        segments: Vec<Segment>,
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
    #[error("speaker id already exists")]
    DuplicateSpeakerId,
    #[error("lane was not found")]
    LaneNotFound,
    #[error("there is no command to undo")]
    NothingToUndo,
    #[error("there is no command to redo")]
    NothingToRedo,
}

/// Undo/redo snapshots, retaining at most [`CommandHistory::MAX_DEPTH`] projects and
/// at most [`CommandHistory::MAX_HISTORY_BYTES`] estimated heap bytes per stack.
#[derive(Clone, Debug, Default)]
pub struct CommandHistory {
    undo_stack: VecDeque<HistorySnapshot>,
    redo_stack: VecDeque<HistorySnapshot>,
    undo_stack_bytes: usize,
    redo_stack_bytes: usize,
}

/// A retained project state plus the heap estimate charged for it, so that
/// byte-budget eviction of the oldest entry stays O(1).
#[derive(Clone, Debug)]
struct HistorySnapshot {
    project: Project,
    estimated_bytes: usize,
}

impl CommandHistory {
    /// Hard limit for each snapshot stack. The newest project states are retained.
    pub const MAX_DEPTH: usize = 100;

    /// Hard limit on the total estimated heap bytes retained by each snapshot
    /// stack (see [`Project::estimated_heap_bytes`]). A real project is
    /// 0.5–5 MiB, so the count cap alone could pin hundreds of megabytes of
    /// deep clones during normal editing; 64 MiB bounds that per stack. The
    /// newest snapshot is always kept, even if it alone exceeds the budget,
    /// so at least one undo/redo step remains available.
    pub const MAX_HISTORY_BYTES: usize = 64 * 1024 * 1024;

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn apply(
        &mut self,
        project: &mut Project,
        command: EditorCommand,
    ) -> Result<(), StudioCoreError> {
        self.apply_with_policy(project, command, &NativeSegmentMutationPolicy)
    }

    pub fn apply_with_policy(
        &mut self,
        project: &mut Project,
        command: EditorCommand,
        policy: &dyn SegmentMutationPolicy,
    ) -> Result<(), StudioCoreError> {
        self.apply_transaction(project, |project| apply_command(project, command, policy))
    }

    /// Applies `commands` in order as one undo unit; a failing command leaves the project
    /// untouched.
    pub fn apply_batch_with_policy(
        &mut self,
        project: &mut Project,
        commands: Vec<EditorCommand>,
        policy: &dyn SegmentMutationPolicy,
    ) -> Result<(), StudioCoreError> {
        self.apply_transaction(project, |project| {
            commands
                .into_iter()
                .try_for_each(|command| apply_command(project, command, policy))
        })
    }

    /// Stages all changes, then commits one undo unit only when the update succeeds.
    pub fn apply_transaction<E>(
        &mut self,
        project: &mut Project,
        update: impl FnOnce(&mut Project) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut staged = project.clone();
        update(&mut staged)?;
        let previous = std::mem::replace(project, staged);
        Self::push_bounded(&mut self.undo_stack, &mut self.undo_stack_bytes, previous);
        self.redo_stack.clear();
        self.redo_stack_bytes = 0;
        Ok(())
    }

    pub fn undo(&mut self, project: &mut Project) -> Result<(), StudioCoreError> {
        let previous = self
            .undo_stack
            .pop_back()
            .ok_or(StudioCoreError::NothingToUndo)?;
        self.undo_stack_bytes = self
            .undo_stack_bytes
            .saturating_sub(previous.estimated_bytes);
        let current = std::mem::replace(project, previous.project);
        Self::push_bounded(&mut self.redo_stack, &mut self.redo_stack_bytes, current);
        Ok(())
    }

    pub fn redo(&mut self, project: &mut Project) -> Result<(), StudioCoreError> {
        let next = self
            .redo_stack
            .pop_back()
            .ok_or(StudioCoreError::NothingToRedo)?;
        self.redo_stack_bytes = self.redo_stack_bytes.saturating_sub(next.estimated_bytes);
        let current = std::mem::replace(project, next.project);
        Self::push_bounded(&mut self.undo_stack, &mut self.undo_stack_bytes, current);
        Ok(())
    }

    /// Drops every retained snapshot from both stacks, freeing the retained
    /// project clones. Used when the workspace is torn down (e.g. on the
    /// return to the dashboard) so history memory is not pinned while the
    /// project is no longer being edited.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.undo_stack_bytes = 0;
        self.redo_stack_bytes = 0;
    }

    fn push_bounded(
        stack: &mut VecDeque<HistorySnapshot>,
        stack_bytes: &mut usize,
        snapshot: Project,
    ) {
        let estimated_bytes = snapshot.estimated_heap_bytes();
        stack.push_back(HistorySnapshot {
            project: snapshot,
            estimated_bytes,
        });
        *stack_bytes += estimated_bytes;
        // Evict the oldest snapshots while either budget is exceeded. The
        // newest snapshot is never evicted, so a single project larger than
        // the byte budget still leaves one undo/redo step available.
        while stack.len() > Self::MAX_DEPTH
            || (*stack_bytes > Self::MAX_HISTORY_BYTES && stack.len() > 1)
        {
            let evicted = stack
                .pop_front()
                .expect("over-budget snapshot stack is non-empty");
            *stack_bytes = stack_bytes.saturating_sub(evicted.estimated_bytes);
        }
    }
}

fn apply_command(
    project: &mut Project,
    command: EditorCommand,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    match command {
        EditorCommand::CreateSegment { segment } => create_segment(project, segment, policy),
        EditorCommand::DeleteSegment { segment_id } => delete_segment(project, &segment_id),
        EditorCommand::MergeSegments {
            left_segment_id,
            right_segment_id,
        } => merge_segments(
            project,
            &left_segment_id,
            &right_segment_id,
            MergeKeep::Left,
            policy,
        ),
        EditorCommand::MergeSegmentsKeepingRight {
            left_segment_id,
            right_segment_id,
        } => merge_segments(
            project,
            &left_segment_id,
            &right_segment_id,
            MergeKeep::Right,
            policy,
        ),
        EditorCommand::ChangeSegmentSpeaker {
            segment_id,
            speaker_id,
            lane_id,
        } => change_segment_speaker(project, &segment_id, speaker_id, lane_id, policy),
        EditorCommand::AddSpeaker { speaker } => {
            if project.speaker(&speaker.id).is_some() {
                return Err(StudioCoreError::DuplicateSpeakerId);
            }
            project.speakers.push(speaker);
            Ok(())
        }
        EditorCommand::RenameSpeaker { speaker_id, label } => {
            let speaker = project
                .speaker_mut(&speaker_id)
                .ok_or(StudioCoreError::SpeakerNotFound)?;
            speaker.label = label;
            Ok(())
        }
        EditorCommand::UpdateSegmentMetadata {
            segment_id,
            text,
            entries,
        } => {
            let segment = project
                .segment_mut(&segment_id)
                .ok_or(StudioCoreError::SegmentNotFound)?;
            if let Some(text) = text {
                segment.text = text;
            }
            segment.metadata.extend(entries);
            policy.updated(segment);
            Ok(())
        }
        EditorCommand::ApplyTextEdit { segment_id, text } => {
            let segment = project
                .segment_mut(&segment_id)
                .ok_or(StudioCoreError::SegmentNotFound)?;
            segment.text = text;
            policy.updated(segment);
            Ok(())
        }
        EditorCommand::UpdateSegment {
            segment_id,
            start_seconds,
            end_seconds,
            text,
        } => update_segment(
            project,
            &segment_id,
            start_seconds,
            end_seconds,
            text,
            policy,
        ),
        EditorCommand::UpdateSegments { updates } => update_segments(project, updates, policy),
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
            policy.updated(segment);
            Ok(())
        }
        EditorCommand::SplitSegment {
            segment_id,
            at_seconds,
            new_segment_id,
        } => split_segment(project, &segment_id, at_seconds, new_segment_id, policy),
        EditorCommand::SplitSegmentWithText {
            segment_id,
            at_seconds,
            new_segment_id,
            left_text,
            right_text,
        } => split_segment_with_text(
            project,
            &segment_id,
            at_seconds,
            new_segment_id,
            left_text,
            right_text,
            policy,
        ),
        EditorCommand::CutLaneRange {
            lane_id,
            start_seconds,
            end_seconds,
            split_texts,
        } => cut_lane_range(
            project,
            &lane_id,
            start_seconds,
            end_seconds,
            split_texts,
            policy,
        ),
        EditorCommand::ReplaceSegments {
            remove_segment_ids,
            segments,
        } => replace_segments(project, &remove_segment_ids, segments, policy),
    }
}

fn create_segment(
    project: &mut Project,
    mut segment: Segment,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    if project.has_segment(&segment.id) {
        return Err(StudioCoreError::DuplicateSegmentId);
    }
    validate_time_range(segment.start_seconds, segment.end_seconds)?;
    validate_track(project, &segment.speaker_id, &segment.lane_id)?;
    policy.created(&mut segment);
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

fn replace_segments(
    project: &mut Project,
    remove_segment_ids: &[String],
    segments: Vec<Segment>,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    if remove_segment_ids.iter().any(|id| !project.has_segment(id)) {
        return Err(StudioCoreError::SegmentNotFound);
    }
    let removed = |id: &str| remove_segment_ids.iter().any(|removed| removed == id);
    for (index, segment) in segments.iter().enumerate() {
        if (project.has_segment(&segment.id) && !removed(&segment.id))
            || segments[..index].iter().any(|other| other.id == segment.id)
        {
            return Err(StudioCoreError::DuplicateSegmentId);
        }
        validate_time_range(segment.start_seconds, segment.end_seconds)?;
        validate_track(project, &segment.speaker_id, &segment.lane_id)?;
    }
    project.segments.retain(|segment| !removed(&segment.id));
    for mut segment in segments {
        policy.created(&mut segment);
        project.segments.push(segment);
    }
    Ok(())
}

fn update_segment(
    project: &mut Project,
    segment_id: &str,
    start_seconds: f64,
    end_seconds: f64,
    text: String,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    validate_time_range(start_seconds, end_seconds)?;
    let segment = project
        .segment_mut(segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    segment.start_seconds = start_seconds;
    segment.end_seconds = end_seconds;
    segment.text = text;
    policy.updated(segment);
    Ok(())
}

fn update_segments(
    project: &mut Project,
    updates: Vec<SegmentUpdate>,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    for update in &updates {
        validate_time_range(update.start_seconds, update.end_seconds)?;
        project
            .segment(&update.segment_id)
            .ok_or(StudioCoreError::SegmentNotFound)?;
    }

    for update in updates {
        let segment = project
            .segment_mut(&update.segment_id)
            .ok_or(StudioCoreError::SegmentNotFound)?;
        segment.start_seconds = update.start_seconds;
        segment.end_seconds = update.end_seconds;
        segment.text = update.text;
        policy.updated(segment);
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MergeKeep {
    Left,
    Right,
}

fn merge_segments(
    project: &mut Project,
    left_segment_id: &str,
    right_segment_id: &str,
    keep: MergeKeep,
    policy: &dyn SegmentMutationPolicy,
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
    match keep {
        MergeKeep::Left => {
            let left_segment = &mut project.segments[left_index];
            left_segment.start_seconds = left.start_seconds;
            left_segment.end_seconds = right.end_seconds;
            left_segment.text = merged_text;
            left_segment.source_annotation_id =
                left.source_annotation_id.or(right.source_annotation_id);
            policy.updated(left_segment);
            project.segments.remove(right_index);
        }
        MergeKeep::Right => {
            let right_segment = &mut project.segments[right_index];
            right_segment.start_seconds = left.start_seconds;
            right_segment.end_seconds = right.end_seconds;
            right_segment.text = merged_text;
            right_segment.source_annotation_id =
                right.source_annotation_id.or(left.source_annotation_id);
            policy.updated(right_segment);
            project.segments.remove(left_index);
        }
    }
    Ok(())
}

fn change_segment_speaker(
    project: &mut Project,
    segment_id: &str,
    speaker_id: String,
    lane_id: String,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    project
        .segment(segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    validate_track(project, &speaker_id, &lane_id)?;

    let segment = project
        .segment_mut(segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    segment.speaker_id = speaker_id;
    segment.lane_id = lane_id;
    policy.updated(segment);
    Ok(())
}

fn split_segment(
    project: &mut Project,
    segment_id: &str,
    at_seconds: f64,
    new_segment_id: String,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    if project.has_segment(&new_segment_id) {
        return Err(StudioCoreError::DuplicateSegmentId);
    }

    let index = project
        .segments
        .iter()
        .position(|segment| segment.id == segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    let original_start = project.segments[index].start_seconds;
    let original_end = project.segments[index].end_seconds;
    let left_end_seconds = at_seconds - (policy.split_gap_seconds() / 2.0);
    let right_start_seconds = at_seconds + (policy.split_gap_seconds() / 2.0);

    validate_time_range(original_start, left_end_seconds)?;
    validate_time_range(right_start_seconds, original_end)?;

    if at_seconds <= original_start || at_seconds >= original_end - policy.split_gap_seconds() {
        return Err(StudioCoreError::SplitOutsideSegment);
    }

    let mut right = Segment {
        id: new_segment_id,
        source_annotation_id: None,
        speaker_id: project.segments[index].speaker_id.clone(),
        lane_id: project.segments[index].lane_id.clone(),
        start_seconds: right_start_seconds,
        end_seconds: original_end,
        text: project.segments[index].text.clone(),
        metadata: project.segments[index].metadata.clone(),
    };

    project.segments[index].end_seconds = left_end_seconds;
    policy.split(&mut project.segments[index]);
    policy.split(&mut right);
    project.segments.insert(index + 1, right);
    Ok(())
}

fn split_segment_with_text(
    project: &mut Project,
    segment_id: &str,
    at_seconds: f64,
    new_segment_id: String,
    left_text: String,
    right_text: String,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    split_segment(
        project,
        segment_id,
        at_seconds,
        new_segment_id.clone(),
        policy,
    )?;
    let left = project
        .segment_mut(segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    left.text = left_text;
    let right = project
        .segment_mut(&new_segment_id)
        .ok_or(StudioCoreError::SegmentNotFound)?;
    right.text = right_text;
    Ok(())
}

#[derive(Clone)]
struct CutSegmentSnapshot {
    id: String,
    start_seconds: f64,
    end_seconds: f64,
    text: String,
}

fn cut_lane_range(
    project: &mut Project,
    lane_id: &str,
    start_seconds: f64,
    end_seconds: f64,
    split_texts: Option<(String, String)>,
    policy: &dyn SegmentMutationPolicy,
) -> Result<(), StudioCoreError> {
    validate_time_range(start_seconds, end_seconds)?;
    if !project.lanes.iter().any(|lane| lane.id == lane_id) {
        return Err(StudioCoreError::LaneNotFound);
    }

    let overlapping = project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == lane_id)
        .filter(|segment| {
            segment.end_seconds > start_seconds && segment.start_seconds < end_seconds
        })
        .map(|segment| CutSegmentSnapshot {
            id: segment.id.clone(),
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
            text: segment.text.clone(),
        })
        .collect::<Vec<_>>();

    for segment in overlapping {
        let covers_left =
            segment.start_seconds < start_seconds && segment.end_seconds > start_seconds;
        let covers_right = segment.start_seconds < end_seconds && segment.end_seconds > end_seconds;
        let fully_inside =
            segment.start_seconds >= start_seconds && segment.end_seconds <= end_seconds;

        if covers_left && covers_right {
            let new_segment_id = next_derived_segment_id(project, &segment.id);
            let (left_text, right_text) = split_texts
                .clone()
                .unwrap_or_else(|| (segment.text.clone(), segment.text.clone()));

            split_segment_with_text(
                project,
                &segment.id,
                start_seconds,
                new_segment_id.clone(),
                left_text,
                right_text,
                policy,
            )?;
            let left = project
                .segment(&segment.id)
                .ok_or(StudioCoreError::SegmentNotFound)?
                .clone();
            update_segment(
                project,
                &segment.id,
                left.start_seconds,
                start_seconds,
                left.text,
                policy,
            )?;
            let right = project
                .segment(&new_segment_id)
                .ok_or(StudioCoreError::SegmentNotFound)?
                .clone();
            update_segment(
                project,
                &new_segment_id,
                end_seconds,
                right.end_seconds,
                right.text,
                policy,
            )?;
        } else if fully_inside {
            delete_segment(project, &segment.id)?;
        } else if covers_left {
            update_segment(
                project,
                &segment.id,
                segment.start_seconds,
                start_seconds,
                segment.text,
                policy,
            )?;
        } else if covers_right {
            update_segment(
                project,
                &segment.id,
                end_seconds,
                segment.end_seconds,
                segment.text,
                policy,
            )?;
        }
    }

    Ok(())
}

fn next_derived_segment_id(project: &Project, base_segment_id: &str) -> String {
    for index in 1.. {
        let candidate = format!("{base_segment_id}__studio_{index}");
        if !project.has_segment(&candidate) {
            return candidate;
        }
    }

    unreachable!("failed to allocate derived segment id")
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

/// The speaker and lane both exist; a lane's `speaker_id` is only its default speaker.
fn validate_track(
    project: &Project,
    speaker_id: &str,
    lane_id: &str,
) -> Result<(), StudioCoreError> {
    if project.speaker(speaker_id).is_none() {
        return Err(StudioCoreError::SpeakerNotFound);
    }
    if !project.lanes.iter().any(|lane| lane.id == lane_id) {
        return Err(StudioCoreError::LaneNotFound);
    }
    Ok(())
}
