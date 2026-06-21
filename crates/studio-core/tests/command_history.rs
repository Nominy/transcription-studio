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
fn move_segment_boundary_rejects_non_finite_times() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::MoveSegmentBoundary {
            segment_id: "segment-1".to_string(),
            start_seconds: 1.0,
            end_seconds: f64::NAN,
        },
    );

    assert_eq!(result, Err(StudioCoreError::InvalidTimeRange));
    assert_eq!(project.segment("segment-1").unwrap().start_seconds, 1.0);
    assert_eq!(project.segment("segment-1").unwrap().end_seconds, 3.0);
}

#[test]
fn update_segment_changes_text_and_timing_as_one_undo_step() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::UpdateSegment {
                segment_id: "segment-1".to_string(),
                start_seconds: 1.5,
                end_seconds: 3.5,
                text: "combined edit".to_string(),
            },
        )
        .expect("combined update should apply");

    let edited = project.segment("segment-1").unwrap();
    assert_eq!(edited.start_seconds, 1.5);
    assert_eq!(edited.end_seconds, 3.5);
    assert_eq!(edited.text, "combined edit");

    history
        .undo(&mut project)
        .expect("undo should restore combined edit");

    let restored = project.segment("segment-1").unwrap();
    assert_eq!(restored.start_seconds, 1.0);
    assert_eq!(restored.end_seconds, 3.0);
    assert_eq!(restored.text, "original one");
}

#[test]
fn update_segment_rejects_invalid_time_range() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::UpdateSegment {
            segment_id: "segment-1".to_string(),
            start_seconds: 3.0,
            end_seconds: 3.0,
            text: "should not apply".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::InvalidTimeRange));
    let segment = project.segment("segment-1").unwrap();
    assert_eq!(segment.start_seconds, 1.0);
    assert_eq!(segment.end_seconds, 3.0);
    assert_eq!(segment.text, "original one");
}

#[test]
fn update_segment_rejects_missing_segment() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::UpdateSegment {
            segment_id: "missing".to_string(),
            start_seconds: 1.5,
            end_seconds: 3.5,
            text: "should not apply".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::SegmentNotFound));
    assert_eq!(project.segment("segment-1").unwrap().text, "original one");
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
fn split_segment_rejects_non_finite_split_point() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::SplitSegment {
            segment_id: "segment-1".to_string(),
            at_seconds: f64::NAN,
            new_segment_id: "segment-1b".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::InvalidTimeRange));
    assert!(!project.has_segment("segment-1b"));
    assert_eq!(project.segment("segment-1").unwrap().end_seconds, 3.0);
}

#[test]
fn create_segment_appends_valid_segment() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let segment = Segment {
        id: "segment-3".to_string(),
        source_annotation_id: Some("annotation-3".to_string()),
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 4.5,
        end_seconds: 5.0,
        text: "new segment".to_string(),
    };

    history
        .apply(
            &mut project,
            EditorCommand::CreateSegment {
                segment: segment.clone(),
            },
        )
        .expect("create should apply");

    assert_eq!(project.segments.last(), Some(&segment));
}

#[test]
fn create_segment_rejects_duplicate_id() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::CreateSegment {
            segment: Segment {
                id: "segment-1".to_string(),
                source_annotation_id: None,
                speaker_id: "speaker-1".to_string(),
                lane_id: "track-1".to_string(),
                start_seconds: 4.5,
                end_seconds: 5.0,
                text: "duplicate".to_string(),
            },
        },
    );

    assert_eq!(result, Err(StudioCoreError::DuplicateSegmentId));
    assert_eq!(project.segments.len(), 2);
    assert_eq!(project.segment("segment-1").unwrap().text, "original one");
}

#[test]
fn create_segment_rejects_invalid_time_range() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::CreateSegment {
            segment: Segment {
                id: "segment-3".to_string(),
                source_annotation_id: None,
                speaker_id: "speaker-1".to_string(),
                lane_id: "track-1".to_string(),
                start_seconds: 5.0,
                end_seconds: 4.5,
                text: "invalid".to_string(),
            },
        },
    );

    assert_eq!(result, Err(StudioCoreError::InvalidTimeRange));
    assert!(!project.has_segment("segment-3"));
}

#[test]
fn create_segment_rejects_unknown_speaker() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::CreateSegment {
            segment: Segment {
                id: "segment-3".to_string(),
                source_annotation_id: None,
                speaker_id: "speaker-missing".to_string(),
                lane_id: "track-1".to_string(),
                start_seconds: 4.5,
                end_seconds: 5.0,
                text: "invalid speaker".to_string(),
            },
        },
    );

    assert_eq!(result, Err(StudioCoreError::SpeakerNotFound));
    assert!(!project.has_segment("segment-3"));
}

#[test]
fn create_segment_rejects_unknown_lane() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::CreateSegment {
            segment: Segment {
                id: "segment-3".to_string(),
                source_annotation_id: None,
                speaker_id: "speaker-1".to_string(),
                lane_id: "track-missing".to_string(),
                start_seconds: 4.5,
                end_seconds: 5.0,
                text: "invalid lane".to_string(),
            },
        },
    );

    assert_eq!(result, Err(StudioCoreError::LaneNotFound));
    assert!(!project.has_segment("segment-3"));
}

#[test]
fn create_segment_rejects_lane_for_another_speaker() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::CreateSegment {
            segment: Segment {
                id: "segment-3".to_string(),
                source_annotation_id: None,
                speaker_id: "speaker-1".to_string(),
                lane_id: "track-2".to_string(),
                start_seconds: 4.5,
                end_seconds: 5.0,
                text: "wrong lane".to_string(),
            },
        },
    );

    assert_eq!(result, Err(StudioCoreError::LaneSpeakerMismatch));
    assert!(!project.has_segment("segment-3"));
}

#[test]
fn delete_segment_removes_existing_segment() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::DeleteSegment {
                segment_id: "segment-1".to_string(),
            },
        )
        .expect("delete should apply");

    assert!(!project.has_segment("segment-1"));
    assert!(project.has_segment("segment-2"));
}

#[test]
fn delete_segment_rejects_missing_segment() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::DeleteSegment {
            segment_id: "missing".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::SegmentNotFound));
    assert_eq!(project.segments.len(), 2);
}

#[test]
fn merge_segments_combines_adjacent_segments_on_same_lane() {
    let mut project = sample_project();
    project.segments[1].speaker_id = "speaker-1".to_string();
    project.segments[1].lane_id = "track-1".to_string();
    project.segments[1].start_seconds = 3.0;
    project.segments[1].end_seconds = 4.4;
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::MergeSegments {
                left_segment_id: "segment-1".to_string(),
                right_segment_id: "segment-2".to_string(),
            },
        )
        .expect("merge should apply");

    assert_eq!(project.segments.len(), 1);
    let merged = project.segment("segment-1").unwrap();
    assert_eq!(merged.start_seconds, 1.0);
    assert_eq!(merged.end_seconds, 4.4);
    assert_eq!(merged.text, "original one original two");
    assert_eq!(
        merged.source_annotation_id,
        Some("annotation-1".to_string())
    );
    assert_eq!(merged.speaker_id, "speaker-1");
    assert_eq!(merged.lane_id, "track-1");
    assert!(!project.has_segment("segment-2"));
}

#[test]
fn merge_segments_rejects_missing_segment() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::MergeSegments {
            left_segment_id: "segment-1".to_string(),
            right_segment_id: "missing".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::SegmentNotFound));
    assert_eq!(project.segments.len(), 2);
}

#[test]
fn merge_segments_rejects_different_speaker_or_lane() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::MergeSegments {
            left_segment_id: "segment-1".to_string(),
            right_segment_id: "segment-2".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::SegmentTrackMismatch));
    assert_eq!(project.segments.len(), 2);
}

#[test]
fn merge_segments_rejects_right_segment_before_left_ends() {
    let mut project = sample_project();
    project.segments[1].speaker_id = "speaker-1".to_string();
    project.segments[1].lane_id = "track-1".to_string();
    project.segments[1].start_seconds = 2.9;
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::MergeSegments {
            left_segment_id: "segment-1".to_string(),
            right_segment_id: "segment-2".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::SegmentOrderInvalid));
    assert_eq!(project.segments.len(), 2);
}

#[test]
fn change_segment_speaker_updates_speaker_and_lane() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::ChangeSegmentSpeaker {
                segment_id: "segment-1".to_string(),
                speaker_id: "speaker-2".to_string(),
                lane_id: "track-2".to_string(),
            },
        )
        .expect("speaker change should apply");

    let segment = project.segment("segment-1").unwrap();
    assert_eq!(segment.speaker_id, "speaker-2");
    assert_eq!(segment.lane_id, "track-2");
}

#[test]
fn change_segment_speaker_rejects_missing_segment() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::ChangeSegmentSpeaker {
            segment_id: "missing".to_string(),
            speaker_id: "speaker-2".to_string(),
            lane_id: "track-2".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::SegmentNotFound));
}

#[test]
fn change_segment_speaker_rejects_unknown_speaker() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::ChangeSegmentSpeaker {
            segment_id: "segment-1".to_string(),
            speaker_id: "speaker-missing".to_string(),
            lane_id: "track-2".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::SpeakerNotFound));
    assert_eq!(
        project.segment("segment-1").unwrap().speaker_id,
        "speaker-1"
    );
    assert_eq!(project.segment("segment-1").unwrap().lane_id, "track-1");
}

#[test]
fn change_segment_speaker_rejects_unknown_lane() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::ChangeSegmentSpeaker {
            segment_id: "segment-1".to_string(),
            speaker_id: "speaker-2".to_string(),
            lane_id: "track-missing".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::LaneNotFound));
    assert_eq!(
        project.segment("segment-1").unwrap().speaker_id,
        "speaker-1"
    );
    assert_eq!(project.segment("segment-1").unwrap().lane_id, "track-1");
}

#[test]
fn change_segment_speaker_rejects_lane_for_another_speaker() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    let result = history.apply(
        &mut project,
        EditorCommand::ChangeSegmentSpeaker {
            segment_id: "segment-1".to_string(),
            speaker_id: "speaker-2".to_string(),
            lane_id: "track-1".to_string(),
        },
    );

    assert_eq!(result, Err(StudioCoreError::LaneSpeakerMismatch));
    assert_eq!(
        project.segment("segment-1").unwrap().speaker_id,
        "speaker-1"
    );
    assert_eq!(project.segment("segment-1").unwrap().lane_id, "track-1");
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
