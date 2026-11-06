use serde_json::json;
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
                metadata: Default::default(),
            },
            Segment {
                id: "segment-2".to_string(),
                source_annotation_id: Some("annotation-2".to_string()),
                speaker_id: "speaker-2".to_string(),
                lane_id: "track-2".to_string(),
                start_seconds: 3.2,
                end_seconds: 4.4,
                text: "original two".to_string(),
                metadata: Default::default(),
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
    assert!(project.segment("segment-1").unwrap().metadata.is_empty());
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
    assert!(edited.metadata.is_empty());

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
fn split_segment_with_text_keeps_allocated_content_through_undo_redo() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::SplitSegmentWithText {
                segment_id: "segment-1".to_string(),
                at_seconds: 2.2,
                new_segment_id: "segment-1b".to_string(),
                left_text: "left allocation".to_string(),
                right_text: "right allocation".to_string(),
            },
        )
        .expect("split with text should apply");

    assert_eq!(
        project.segment("segment-1").unwrap().text,
        "left allocation"
    );
    assert_eq!(
        project.segment("segment-1b").unwrap().text,
        "right allocation"
    );

    history.undo(&mut project).expect("undo should restore");
    assert!(!project.has_segment("segment-1b"));
    assert_eq!(project.segment("segment-1").unwrap().text, "original one");

    history
        .redo(&mut project)
        .expect("redo should restore split");
    assert_eq!(
        project.segment("segment-1").unwrap().text,
        "left allocation"
    );
    assert_eq!(
        project.segment("segment-1b").unwrap().text,
        "right allocation"
    );
}

#[test]
fn cut_lane_range_trims_edges_and_deletes_covered_segments() {
    let mut project = sample_project();
    project.segments = vec![
        Segment {
            id: "left-edge".to_string(),
            source_annotation_id: None,
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-1".to_string(),
            start_seconds: 1.0,
            end_seconds: 3.0,
            text: "left edge".to_string(),
            metadata: Default::default(),
        },
        Segment {
            id: "covered".to_string(),
            source_annotation_id: None,
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-1".to_string(),
            start_seconds: 3.2,
            end_seconds: 4.0,
            text: "delete me".to_string(),
            metadata: Default::default(),
        },
        Segment {
            id: "right-edge".to_string(),
            source_annotation_id: None,
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-1".to_string(),
            start_seconds: 6.5,
            end_seconds: 8.0,
            text: "right edge".to_string(),
            metadata: Default::default(),
        },
    ];
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::CutLaneRange {
                lane_id: "track-1".to_string(),
                start_seconds: 2.0,
                end_seconds: 7.0,
                split_texts: None,
            },
        )
        .expect("cut should apply");

    assert!(!project.has_segment("covered"));
    assert_eq!(project.segment("left-edge").unwrap().end_seconds, 2.0);
    assert_eq!(project.segment("left-edge").unwrap().text, "left edge");
    assert_eq!(project.segment("right-edge").unwrap().start_seconds, 7.0);
    assert_eq!(project.segment("right-edge").unwrap().text, "right edge");

    history.undo(&mut project).expect("undo should restore cut");
    assert!(project.has_segment("covered"));
    assert_eq!(project.segment("left-edge").unwrap().end_seconds, 3.0);
    assert_eq!(project.segment("right-edge").unwrap().start_seconds, 6.5);
}

#[test]
fn cut_lane_range_middle_split_applies_allocated_texts_and_redoes_as_one_command() {
    let mut project = sample_project();
    project.segments[0].start_seconds = 1.0;
    project.segments[0].end_seconds = 6.0;
    project.segments[0].text = "one two three four five six".to_string();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::CutLaneRange {
                lane_id: "track-1".to_string(),
                start_seconds: 2.5,
                end_seconds: 4.0,
                split_texts: Some(("one two three".to_string(), "four five six".to_string())),
            },
        )
        .expect("middle smart cut should apply");

    let left = project.segment("segment-1").unwrap();
    let right = project.segment("segment-1__studio_1").unwrap();
    assert_eq!(left.start_seconds, 1.0);
    assert_eq!(left.end_seconds, 2.5);
    assert_eq!(left.text, "one two three");
    assert_eq!(right.start_seconds, 4.0);
    assert_eq!(right.end_seconds, 6.0);
    assert_eq!(right.text, "four five six");

    history.undo(&mut project).expect("undo should restore cut");
    assert!(!project.has_segment("segment-1__studio_1"));
    assert_eq!(project.segment("segment-1").unwrap().start_seconds, 1.0);
    assert_eq!(project.segment("segment-1").unwrap().end_seconds, 6.0);
    assert_eq!(
        project.segment("segment-1").unwrap().text,
        "one two three four five six"
    );

    history.redo(&mut project).expect("redo should restore cut");
    assert_eq!(project.segment("segment-1").unwrap().text, "one two three");
    assert_eq!(
        project.segment("segment-1__studio_1").unwrap().text,
        "four five six"
    );
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
        metadata: Default::default(),
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
                metadata: Default::default(),
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
                metadata: Default::default(),
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
                metadata: Default::default(),
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
                metadata: Default::default(),
            },
        },
    );

    assert_eq!(result, Err(StudioCoreError::LaneNotFound));
    assert!(!project.has_segment("segment-3"));
}

#[test]
fn create_segment_accepts_any_speaker_on_a_lane() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::CreateSegment {
                segment: Segment {
                    id: "segment-3".to_string(),
                    source_annotation_id: None,
                    speaker_id: "speaker-1".to_string(),
                    lane_id: "track-2".to_string(),
                    start_seconds: 4.5,
                    end_seconds: 5.0,
                    text: "guest on track two".to_string(),
                    metadata: Default::default(),
                },
            },
        )
        .expect("a speaker may be placed on any lane");

    let segment = project.segment("segment-3").unwrap();
    assert_eq!(segment.speaker_id, "speaker-1");
    assert_eq!(segment.lane_id, "track-2");
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
fn merge_segments_preserves_right_source_annotation_when_left_has_none() {
    let mut project = sample_project();
    project.segments[0].source_annotation_id = None;
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

    let merged = project.segment("segment-1").unwrap();
    assert_eq!(
        merged.source_annotation_id,
        Some("annotation-2".to_string())
    );
}

#[test]
fn merge_segments_keeping_right_preserves_active_babel_annotation_id() {
    let mut project = sample_project();
    project.segments[1].speaker_id = "speaker-1".to_string();
    project.segments[1].lane_id = "track-1".to_string();
    project.segments[1].start_seconds = 3.0;
    project.segments[1].end_seconds = 4.4;
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::MergeSegmentsKeepingRight {
                left_segment_id: "segment-1".to_string(),
                right_segment_id: "segment-2".to_string(),
            },
        )
        .expect("merge should apply");

    assert_eq!(project.segments.len(), 1);
    assert!(!project.has_segment("segment-1"));
    let merged = project.segment("segment-2").unwrap();
    assert_eq!(merged.start_seconds, 1.0);
    assert_eq!(merged.end_seconds, 4.4);
    assert_eq!(merged.text, "original one original two");
    assert_eq!(
        merged.source_annotation_id,
        Some("annotation-2".to_string())
    );
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
fn change_segment_speaker_keeps_lane_when_speaker_has_another_lane() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::ChangeSegmentSpeaker {
                segment_id: "segment-1".to_string(),
                speaker_id: "speaker-2".to_string(),
                lane_id: "track-1".to_string(),
            },
        )
        .expect("speaker change should apply on the segment's own lane");

    let segment = project.segment("segment-1").unwrap();
    assert_eq!(segment.speaker_id, "speaker-2");
    assert_eq!(segment.lane_id, "track-1");
}

#[test]
fn add_speaker_appends_and_rejects_duplicate_id() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::AddSpeaker {
                speaker: Speaker {
                    id: "speaker-3".to_string(),
                    label: "Speaker 3".to_string(),
                },
            },
        )
        .expect("new speaker should be added");
    assert_eq!(project.speaker("speaker-3").unwrap().label, "Speaker 3");

    let result = history.apply(
        &mut project,
        EditorCommand::AddSpeaker {
            speaker: Speaker {
                id: "speaker-3".to_string(),
                label: "Again".to_string(),
            },
        },
    );
    assert_eq!(result, Err(StudioCoreError::DuplicateSpeakerId));
    assert_eq!(project.speakers.len(), 3);

    history.undo(&mut project).expect("undo add");
    assert!(project.speaker("speaker-3").is_none());
}

#[test]
fn rename_speaker_updates_label_for_every_segment_label_lookup() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply(
            &mut project,
            EditorCommand::RenameSpeaker {
                speaker_id: "speaker-1".to_string(),
                label: "Alice".to_string(),
            },
        )
        .expect("rename should apply");
    assert_eq!(project.speaker("speaker-1").unwrap().label, "Alice");

    let result = history.apply(
        &mut project,
        EditorCommand::RenameSpeaker {
            speaker_id: "speaker-missing".to_string(),
            label: "Nobody".to_string(),
        },
    );
    assert_eq!(result, Err(StudioCoreError::SpeakerNotFound));

    history.undo(&mut project).expect("undo rename");
    assert_eq!(project.speaker("speaker-1").unwrap().label, "Speaker 1");
}

#[test]
fn apply_batch_is_one_undo_step_and_atomic() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();
    let speaker = Speaker {
        id: project.next_speaker_id(),
        label: project.next_speaker_label("Speaker {n}"),
    };

    history
        .apply_batch_with_policy(
            &mut project,
            vec![
                EditorCommand::AddSpeaker { speaker },
                EditorCommand::ChangeSegmentSpeaker {
                    segment_id: "segment-1".to_string(),
                    speaker_id: "speaker-3".to_string(),
                    lane_id: "track-1".to_string(),
                },
            ],
            &studio_core::NativeSegmentMutationPolicy,
        )
        .expect("batch should apply");
    assert_eq!(project.speaker("speaker-3").unwrap().label, "Speaker 3");
    assert_eq!(
        project.segment("segment-1").unwrap().speaker_id,
        "speaker-3"
    );

    history.undo(&mut project).expect("one undo reverts both");
    assert!(project.speaker("speaker-3").is_none());
    assert_eq!(
        project.segment("segment-1").unwrap().speaker_id,
        "speaker-1"
    );
    assert!(!history.can_undo());

    let result = history.apply_batch_with_policy(
        &mut project,
        vec![
            EditorCommand::AddSpeaker {
                speaker: Speaker {
                    id: "speaker-3".to_string(),
                    label: "Speaker 3".to_string(),
                },
            },
            EditorCommand::ChangeSegmentSpeaker {
                segment_id: "missing".to_string(),
                speaker_id: "speaker-3".to_string(),
                lane_id: "track-1".to_string(),
            },
        ],
        &studio_core::NativeSegmentMutationPolicy,
    );
    assert_eq!(result, Err(StudioCoreError::SegmentNotFound));
    assert!(project.speaker("speaker-3").is_none());
    assert!(!history.can_undo());
}

#[test]
fn next_speaker_label_skips_taken_indexes() {
    let mut project = sample_project();
    assert_eq!(project.next_speaker_label("Speaker {n}"), "Speaker 3");
    assert_eq!(project.next_speaker_label("Спикер {n}"), "Спикер 1");
    project.speakers.remove(0);
    assert_eq!(project.next_speaker_label("Speaker {n}"), "Speaker 1");
    assert_eq!(project.next_speaker_id(), "speaker-1");
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

#[test]
fn command_history_reports_undo_redo_availability_like_babel_action_state() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    assert!(!history.can_undo());
    assert!(!history.can_redo());

    history
        .apply(
            &mut project,
            EditorCommand::ApplyTextEdit {
                segment_id: "segment-1".to_string(),
                text: "changed".to_string(),
            },
        )
        .expect("text edit should apply");

    assert!(history.can_undo());
    assert!(!history.can_redo());

    history.undo(&mut project).expect("undo should apply");

    assert!(!history.can_undo());
    assert!(history.can_redo());

    history.redo(&mut project).expect("redo should apply");

    assert!(history.can_undo());
    assert!(!history.can_redo());
}

#[test]
fn command_history_retains_only_the_newest_bounded_snapshots() {
    const COMMAND_COUNT: usize = 2_000;

    let mut project = sample_project();
    let mut history = CommandHistory::default();

    for command_index in 0..COMMAND_COUNT {
        history
            .apply(
                &mut project,
                EditorCommand::ApplyTextEdit {
                    segment_id: "segment-1".to_string(),
                    text: format!("edit-{command_index}"),
                },
            )
            .expect("text edit should apply");
    }

    for undo_count in 1..=CommandHistory::MAX_DEPTH {
        history
            .undo(&mut project)
            .expect("retained recent edit should undo");
        assert_eq!(
            project.segment("segment-1").unwrap().text,
            format!("edit-{}", COMMAND_COUNT - undo_count - 1)
        );
    }

    assert_eq!(
        history.undo(&mut project),
        Err(StudioCoreError::NothingToUndo)
    );

    for redo_count in 1..=CommandHistory::MAX_DEPTH {
        history
            .redo(&mut project)
            .expect("retained recent edit should redo");
        assert_eq!(
            project.segment("segment-1").unwrap().text,
            format!(
                "edit-{}",
                COMMAND_COUNT - CommandHistory::MAX_DEPTH + redo_count - 1
            )
        );
    }

    assert_eq!(
        history.redo(&mut project),
        Err(StudioCoreError::NothingToRedo)
    );
}

fn sample_project_with_segment_text(text: String) -> Project {
    let mut project = sample_project();
    project.segment_mut("segment-1").unwrap().text = text;
    project
}

fn apply_segment_text(history: &mut CommandHistory, project: &mut Project, text: String) {
    history
        .apply(
            project,
            EditorCommand::ApplyTextEdit {
                segment_id: "segment-1".to_string(),
                text,
            },
        )
        .expect("text edit should apply");
}

#[test]
fn project_estimated_heap_bytes_tracks_text_and_metadata_payloads() {
    let small = sample_project();
    let mut large = sample_project();
    large.segment_mut("segment-1").unwrap().text = "x".repeat(1024 * 1024);

    assert!(small.estimated_heap_bytes() > 0);
    assert_eq!(
        large.estimated_heap_bytes() - small.estimated_heap_bytes(),
        1024 * 1024 - "original one".len()
    );

    let mut with_metadata = sample_project();
    with_metadata
        .metadata
        .insert("key".to_string(), "value".to_string());
    assert!(
        with_metadata.estimated_heap_bytes()
            >= small.estimated_heap_bytes() + "key".len() + "value".len()
    );
}

#[test]
fn command_history_byte_budget_evicts_oldest_snapshots_below_the_count_cap() {
    // 40 edits stay well under MAX_DEPTH, but each ~2 MiB snapshot charges
    // against the 64 MiB byte budget, so most of them must be evicted.
    const EDITS: usize = 40;
    const TEXT_BYTES: usize = 2 * 1024 * 1024;
    const { assert!(EDITS < CommandHistory::MAX_DEPTH) };

    let mut project = sample_project_with_segment_text("x".repeat(TEXT_BYTES));
    let mut history = CommandHistory::default();

    for edit_index in 0..EDITS {
        apply_segment_text(
            &mut history,
            &mut project,
            format!("{}{}", "y".repeat(TEXT_BYTES), edit_index),
        );
    }

    let mut undo_count = 0usize;
    while history.undo(&mut project).is_ok() {
        undo_count += 1;
    }

    // The byte budget evicted snapshots well before the count cap could.
    assert!(undo_count < EDITS);
    assert!(undo_count < CommandHistory::MAX_DEPTH);
    // Roughly 31 of the ~2 MiB states fit into the 64 MiB budget; allow
    // slack for the per-item overhead constants in the estimator.
    assert!((28..=33).contains(&undo_count));

    // The retained snapshots are the newest states: undoing all of them
    // lands on the state right before the oldest retained edit.
    let expected_marker = (EDITS - undo_count - 1).to_string();
    let text = &project.segment("segment-1").unwrap().text;
    assert!(text.ends_with(&expected_marker));
    assert_eq!(text.len(), TEXT_BYTES + expected_marker.len());
}

#[test]
fn command_history_count_cap_still_bounds_small_edits() {
    const EXTRA_EDITS: usize = 25;

    let mut project = sample_project();
    let mut history = CommandHistory::default();

    for edit_index in 0..CommandHistory::MAX_DEPTH + EXTRA_EDITS {
        apply_segment_text(
            &mut history,
            &mut project,
            format!("small edit {edit_index}"),
        );
    }

    let mut undo_count = 0usize;
    while history.undo(&mut project).is_ok() {
        undo_count += 1;
    }
    assert_eq!(undo_count, CommandHistory::MAX_DEPTH);
    // The retained snapshots are the newest: after MAX_DEPTH undos the
    // project shows the state right before the oldest retained edit.
    assert_eq!(
        project.segment("segment-1").unwrap().text,
        format!("small edit {}", EXTRA_EDITS - 1)
    );
}

#[test]
fn command_history_undo_redo_survive_byte_budget_evictions() {
    const EDITS: usize = 40;
    const TEXT_BYTES: usize = 2 * 1024 * 1024;

    let mut project = sample_project_with_segment_text("x".repeat(TEXT_BYTES));
    let mut history = CommandHistory::default();

    for edit_index in 0..EDITS {
        apply_segment_text(
            &mut history,
            &mut project,
            format!("{}{}", "y".repeat(TEXT_BYTES), edit_index),
        );
    }

    history
        .undo(&mut project)
        .expect("newest retained edit should undo");
    assert!(project.segment("segment-1").unwrap().text.ends_with("38"));
    history
        .undo(&mut project)
        .expect("second newest retained edit should undo");
    assert!(project.segment("segment-1").unwrap().text.ends_with("37"));

    history.redo(&mut project).expect("undone edit should redo");
    assert!(project.segment("segment-1").unwrap().text.ends_with("38"));
    history
        .redo(&mut project)
        .expect("second undone edit should redo");
    assert!(project.segment("segment-1").unwrap().text.ends_with("39"));

    // Applying a fresh edit still clears the redo stack.
    apply_segment_text(&mut history, &mut project, "z".repeat(TEXT_BYTES));
    assert!(!history.can_redo());
    assert_eq!(
        history.redo(&mut project),
        Err(StudioCoreError::NothingToRedo)
    );
}

#[test]
fn command_history_clear_empties_both_stacks() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    apply_segment_text(&mut history, &mut project, "first".to_string());
    apply_segment_text(&mut history, &mut project, "second".to_string());
    history.undo(&mut project).expect("edit should undo");
    assert!(history.can_undo());
    assert!(history.can_redo());

    history.clear();

    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert_eq!(
        history.undo(&mut project),
        Err(StudioCoreError::NothingToUndo)
    );
    assert_eq!(
        history.redo(&mut project),
        Err(StudioCoreError::NothingToRedo)
    );

    // The history stays usable after a clear.
    apply_segment_text(&mut history, &mut project, "third".to_string());
    assert!(history.can_undo());
    history
        .undo(&mut project)
        .expect("post-clear edit should undo");
    assert_eq!(project.segment("segment-1").unwrap().text, "first");
}

#[test]
fn native_split_preserves_metadata_without_domain_gap_or_stamps() {
    let mut project = sample_project();
    project.segments[0]
        .metadata
        .insert("opaque".into(), json!({"source": 7}));
    let original_metadata = project.segments[0].metadata.clone();
    CommandHistory::default()
        .apply(
            &mut project,
            EditorCommand::SplitSegment {
                segment_id: "segment-1".into(),
                at_seconds: 2.0,
                new_segment_id: "split".into(),
            },
        )
        .unwrap();
    let left = project.segment("segment-1").unwrap();
    let right = project.segment("split").unwrap();
    assert_eq!(left.end_seconds, 2.0);
    assert_eq!(right.start_seconds, 2.0);
    assert_eq!(left.metadata, original_metadata);
    assert_eq!(right.metadata, original_metadata);
}

#[test]
fn project_transaction_rolls_back_failed_partial_mutation_and_preserves_redo() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();
    history
        .apply(
            &mut project,
            EditorCommand::ApplyTextEdit {
                segment_id: "segment-1".into(),
                text: "edited".into(),
            },
        )
        .unwrap();
    history.undo(&mut project).unwrap();
    let previous = project.clone();
    let result = history.apply_transaction(&mut project, |project| {
        project.segments[0].text = "partial".into();
        project.metadata.insert("opaque".into(), "partial".into());
        Err::<(), _>("rejected")
    });
    assert_eq!(result, Err("rejected"));
    assert_eq!(project, previous);
    assert!(!history.can_undo());
    assert!(history.can_redo());
}

#[test]
fn metadata_transaction_commits_as_one_undo_unit() {
    let mut project = sample_project();
    let original = project.clone();
    let mut history = CommandHistory::default();
    history
        .apply_transaction(&mut project, |project| {
            project.segments[0].text = "first".into();
            project.segments[1]
                .metadata
                .insert("opaque".into(), json!(17));
            Ok::<(), StudioCoreError>(())
        })
        .unwrap();
    let committed = project.clone();
    history.undo(&mut project).unwrap();
    assert_eq!(project, original);
    assert!(!history.can_undo());
    history.redo(&mut project).unwrap();
    assert_eq!(project, committed);
}

#[test]
fn opaque_segment_metadata_command_preserves_unmentioned_fields_and_is_undoable() {
    let mut project = sample_project();
    project.segments[0]
        .metadata
        .insert("preserved".into(), json!([1, 2]));
    let original = project.clone();
    let mut history = CommandHistory::default();
    history
        .apply(
            &mut project,
            EditorCommand::UpdateSegmentMetadata {
                segment_id: "segment-1".into(),
                text: Some("edited".into()),
                entries: std::collections::BTreeMap::from([("opaque".into(), json!({"value": 3}))]),
            },
        )
        .unwrap();
    assert_eq!(project.segments[0].text, "edited");
    assert_eq!(project.segments[0].metadata.len(), 2);
    assert_eq!(
        project.segments[0].metadata.get("preserved"),
        Some(&json!([1, 2]))
    );
    history.undo(&mut project).unwrap();
    assert_eq!(project, original);
}
