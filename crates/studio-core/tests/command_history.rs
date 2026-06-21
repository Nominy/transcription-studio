use studio_core::{
    AudioLane, CommandHistory, EditorCommand, Project, Segment, Speaker, StudioCoreError,
};

fn sample_project() -> Project {
    Project {
        id: "task:ru-demo".to_string(),
        name: "RU demo correction".to_string(),
        speakers: vec![
            Speaker {
                id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
            },
            Speaker {
                id: "speaker-2".to_string(),
                label: "Speaker 2".to_string(),
            },
        ],
        lanes: vec![
            AudioLane {
                id: "track-1".to_string(),
                speaker_id: "speaker-1".to_string(),
                label: "Speaker 1 track".to_string(),
                source_path: "audio/speaker-1.wav".to_string(),
            },
            AudioLane {
                id: "track-2".to_string(),
                speaker_id: "speaker-2".to_string(),
                label: "Speaker 2 track".to_string(),
                source_path: "audio/speaker-2.wav".to_string(),
            },
        ],
        segments: vec![
            Segment {
                id: "segment-1".to_string(),
                source_annotation_id: Some("annotation-1".to_string()),
                speaker_id: "speaker-1".to_string(),
                lane_id: "track-1".to_string(),
                start_seconds: 1.0,
                end_seconds: 3.0,
                text: "original one".to_string(),
            },
            Segment {
                id: "segment-2".to_string(),
                source_annotation_id: Some("annotation-2".to_string()),
                speaker_id: "speaker-2".to_string(),
                lane_id: "track-2".to_string(),
                start_seconds: 3.2,
                end_seconds: 4.4,
                text: "original two".to_string(),
            },
        ],
        metadata: Default::default(),
    }
}

#[test]
fn apply_text_edit_changes_only_target_segment() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::ApplyTextEdit {
                segment_id: "segment-1".to_string(),
                text: "corrected text".to_string(),
            },
        )
        .expect("text edit should apply");

    assert_eq!(project.segment("segment-1").unwrap().text, "corrected text");
    assert_eq!(project.segment("segment-2").unwrap().text, "original two");
}

#[test]
fn move_segment_boundary_rejects_non_positive_duration() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::MoveSegmentBoundary {
            segment_id: "segment-1".to_string(),
            start_seconds: 3.0,
            end_seconds: 3.0,
        },
    );

    assert_eq!(result, Err(StudioCoreError::InvalidTimeRange));
    assert_eq!(project.segment("segment-1").unwrap().start_seconds, 1.0);
    assert_eq!(project.segment("segment-1").unwrap().end_seconds, 3.0);
}

#[test]
fn split_segment_creates_adjacent_right_segment_and_keeps_text_on_left() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::SplitSegment {
                segment_id: "segment-1".to_string(),
                at_seconds: 2.2,
                new_segment_id: "segment-1b".to_string(),
            },
        )
        .expect("split should apply");

    let left = project.segment("segment-1").unwrap();
    let right = project.segment("segment-1b").unwrap();
    assert_eq!(left.start_seconds, 1.0);
    assert_eq!(left.end_seconds, 2.2);
    assert_eq!(left.text, "original one");
    assert_eq!(right.start_seconds, 2.2);
    assert_eq!(right.end_seconds, 3.0);
    assert_eq!(right.text, "");
    assert_eq!(right.speaker_id, "speaker-1");
    assert_eq!(right.lane_id, "track-1");
}

#[test]
fn undo_restores_previous_project_state() {
    let mut project = sample_project();
    let original = project.clone();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::ApplyTextEdit {
                segment_id: "segment-1".to_string(),
                text: "changed".to_string(),
            },
        )
        .expect("text edit should apply");

    assert_ne!(project, original);
    history
        .undo(&mut project)
        .expect("undo should restore state");
    assert_eq!(project, original);
}

#[test]
fn redo_reapplies_undone_change() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::ApplyTextEdit {
                segment_id: "segment-1".to_string(),
                text: "redo text".to_string(),
            },
        )
        .expect("text edit should apply");

    history
        .undo(&mut project)
        .expect("undo should restore state");
    assert_eq!(project.segment("segment-1").unwrap().text, "original one");

    history
        .redo(&mut project)
        .expect("redo should restore change");
    assert_eq!(project.segment("segment-1").unwrap().text, "redo text");
}
