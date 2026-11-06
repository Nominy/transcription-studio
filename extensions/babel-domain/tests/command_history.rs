use exaudio_babel_domain::workflow::BabelMutationPolicy;
use serde_json::json;
use std::collections::BTreeMap;
use studio_core::{AudioLane, CommandHistory, EditorCommand, Project, Segment, Speaker};

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
fn resolve_low_confidence_marks_segment_metadata_and_undo_restores_it() {
    let mut project = sample_project();
    project
        .segment_mut("segment-1")
        .unwrap()
        .metadata
        .insert("lowConfidenceResolved".to_string(), json!(false));
    let mut history = CommandHistory::default();

    history
        .apply_with_policy(
            &mut project,
            EditorCommand::UpdateSegmentMetadata {
                segment_id: "segment-1".to_string(),
                text: None,
                entries: BTreeMap::from([("lowConfidenceResolved".into(), json!(true))]),
            },
            &BabelMutationPolicy::default(),
        )
        .expect("low-confidence resolve should apply");

    assert_eq!(
        project
            .segment("segment-1")
            .unwrap()
            .metadata
            .get("lowConfidenceResolved"),
        Some(&json!(true))
    );

    history
        .undo(&mut project)
        .expect("undo should restore metadata");

    assert_eq!(
        project
            .segment("segment-1")
            .unwrap()
            .metadata
            .get("lowConfidenceResolved"),
        Some(&json!(false))
    );
}

#[test]
fn update_annotation_metadata_preserves_other_fields_and_undo_restores() {
    let mut project = sample_project();
    let segment = project.segment_mut("segment-1").unwrap();
    segment
        .metadata
        .insert("type".to_string(), json!("background_noise"));
    segment
        .metadata
        .insert("reviewActionId".to_string(), json!("review-1"));
    segment.metadata.insert(
        "content".to_string(),
        json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    segment.metadata.insert("intensity".to_string(), json!(2));
    let mut history = CommandHistory::default();

    history
        .apply_with_policy(
            &mut project,
            EditorCommand::UpdateSegmentMetadata {
                segment_id: "segment-1".to_string(),
                text: Some("background-noise / chair scrape".to_string()),
                entries: BTreeMap::from([
                    ("intensity".into(), json!(4)),
                    (
                        "content".into(),
                        json!({
                            "primaryLabel": "background-noise",
                            "secondaryLabel": "chair scrape"
                        }),
                    ),
                ]),
            },
            &BabelMutationPolicy::with_clock(|| "2026-06-21T12:34:56.000Z".into()),
        )
        .expect("annotation metadata edit should apply");

    let edited = project.segment("segment-1").unwrap();
    assert_eq!(edited.text, "background-noise / chair scrape");
    assert_eq!(
        edited.metadata.get("type"),
        Some(&json!("background_noise"))
    );
    assert_eq!(
        edited.metadata.get("reviewActionId"),
        Some(&json!("review-1"))
    );
    assert_eq!(edited.metadata.get("intensity"), Some(&json!(4)));
    assert_eq!(
        edited.metadata.get("content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "chair scrape"
        }))
    );
    assert_eq!(
        edited.metadata.get("lowConfidenceResolved"),
        Some(&json!(true)),
        "recovered Babel annotation edits resolve low-confidence metadata"
    );
    assert_eq!(
        edited.metadata.get("updatedAt"),
        Some(&json!("2026-06-21T12:34:56.000Z")),
        "recovered Babel annotation edits store the mutation timestamp"
    );

    history
        .undo(&mut project)
        .expect("undo should restore annotation metadata");

    let restored = project.segment("segment-1").unwrap();
    assert_eq!(restored.text, "original one");
    assert_eq!(restored.metadata.get("intensity"), Some(&json!(2)));
    assert_eq!(
        restored.metadata.get("content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );
}

#[test]
fn timestamped_annotation_commands_stamp_babel_updated_at_metadata() {
    let timestamp = "2026-06-21T12:34:56.000Z";

    let mut text_project = sample_project();
    CommandHistory::default()
        .apply_with_policy(
            &mut text_project,
            EditorCommand::ApplyTextEdit {
                segment_id: "segment-1".to_string(),
                text: "edited text".to_string(),
            },
            &BabelMutationPolicy::with_clock(|| "2026-06-21T12:34:56.000Z".into()),
        )
        .expect("text edit should apply");
    assert_eq!(
        text_project
            .segment("segment-1")
            .unwrap()
            .metadata
            .get("updatedAt"),
        Some(&json!(timestamp))
    );

    let mut time_project = sample_project();
    CommandHistory::default()
        .apply_with_policy(
            &mut time_project,
            EditorCommand::MoveSegmentBoundary {
                segment_id: "segment-1".to_string(),
                start_seconds: 1.25,
                end_seconds: 3.25,
            },
            &BabelMutationPolicy::with_clock(|| "2026-06-21T12:34:56.000Z".into()),
        )
        .expect("time edit should apply");
    assert_eq!(
        time_project
            .segment("segment-1")
            .unwrap()
            .metadata
            .get("updatedAt"),
        Some(&json!(timestamp))
    );

    let mut track_project = sample_project();
    CommandHistory::default()
        .apply_with_policy(
            &mut track_project,
            EditorCommand::ChangeSegmentSpeaker {
                segment_id: "segment-1".to_string(),
                speaker_id: "speaker-2".to_string(),
                lane_id: "track-2".to_string(),
            },
            &BabelMutationPolicy::with_clock(|| "2026-06-21T12:34:56.000Z".into()),
        )
        .expect("track edit should apply");
    assert_eq!(
        track_project
            .segment("segment-1")
            .unwrap()
            .metadata
            .get("updatedAt"),
        Some(&json!(timestamp))
    );

    let mut create_project = sample_project();
    CommandHistory::default()
        .apply_with_policy(
            &mut create_project,
            EditorCommand::CreateSegment {
                segment: Segment {
                    id: "created".to_string(),
                    source_annotation_id: None,
                    speaker_id: "speaker-1".to_string(),
                    lane_id: "track-1".to_string(),
                    start_seconds: 4.5,
                    end_seconds: 5.0,
                    text: String::new(),
                    metadata: Default::default(),
                },
            },
            &BabelMutationPolicy::with_clock(|| "2026-06-21T12:34:56.000Z".into()),
        )
        .expect("create should apply");
    let created = create_project.segment("created").unwrap();
    assert_eq!(created.metadata.get("createdAt"), Some(&json!(timestamp)));
    assert_eq!(created.metadata.get("updatedAt"), Some(&json!(timestamp)));

    let mut split_project = sample_project();
    CommandHistory::default()
        .apply_with_policy(
            &mut split_project,
            EditorCommand::SplitSegment {
                segment_id: "segment-1".to_string(),
                at_seconds: 2.0,
                new_segment_id: "segment-1b".to_string(),
            },
            &BabelMutationPolicy::with_clock(|| "2026-06-21T12:34:56.000Z".into()),
        )
        .expect("split should apply");
    for segment_id in ["segment-1", "segment-1b"] {
        let segment = split_project.segment(segment_id).unwrap();
        assert_eq!(segment.metadata.get("createdAt"), Some(&json!(timestamp)));
        assert_eq!(segment.metadata.get("updatedAt"), Some(&json!(timestamp)));
    }

    let mut merge_project = sample_project();
    merge_project.segments[1].speaker_id = "speaker-1".to_string();
    merge_project.segments[1].lane_id = "track-1".to_string();
    merge_project.segments[1].start_seconds = 3.0;
    CommandHistory::default()
        .apply_with_policy(
            &mut merge_project,
            EditorCommand::MergeSegments {
                left_segment_id: "segment-1".to_string(),
                right_segment_id: "segment-2".to_string(),
            },
            &BabelMutationPolicy::with_clock(|| "2026-06-21T12:34:56.000Z".into()),
        )
        .expect("merge should apply");
    assert_eq!(
        merge_project
            .segment("segment-1")
            .unwrap()
            .metadata
            .get("updatedAt"),
        Some(&json!(timestamp))
    );
}

#[test]
fn split_segment_leaves_babel_gap_and_clones_recovered_content() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply_with_policy(
            &mut project,
            EditorCommand::SplitSegment {
                segment_id: "segment-1".to_string(),
                at_seconds: 2.2,
                new_segment_id: "segment-1b".to_string(),
            },
            &BabelMutationPolicy::default(),
        )
        .expect("split should apply");

    let left = project.segment("segment-1").unwrap();
    let right = project.segment("segment-1b").unwrap();
    assert_eq!(left.start_seconds, 1.0);
    assert!((left.end_seconds - 2.15).abs() < f64::EPSILON * 4.0);
    assert_eq!(left.text, "original one");
    assert!((right.start_seconds - 2.25).abs() < f64::EPSILON * 4.0);
    assert_eq!(right.end_seconds, 3.0);
    assert_eq!(right.text, "original one");
    assert_eq!(right.speaker_id, "speaker-1");
    assert_eq!(right.lane_id, "track-1");
}

#[test]
fn set_segment_annotation_type_updates_metadata_and_undo_restores_it() {
    let mut project = sample_project();
    let mut history = CommandHistory::default();

    history
        .apply_with_policy(
            &mut project,
            EditorCommand::UpdateSegmentMetadata {
                segment_id: "segment-1".to_string(),
                text: None,
                entries: BTreeMap::from([("type".into(), json!("non-transcription"))]),
            },
            &BabelMutationPolicy::default(),
        )
        .expect("annotation type change should apply");

    assert_eq!(
        project.segment("segment-1").unwrap().metadata.get("type"),
        Some(&json!("non-transcription"))
    );

    history
        .undo(&mut project)
        .expect("undo should restore annotation metadata");

    assert_eq!(
        project.segment("segment-1").unwrap().metadata.get("type"),
        None
    );
}

#[test]
fn review_action_annotation_metadata_edit_exports_low_confidence_resolution() {
    let source = json!([
        {
            "result": {
                "data": {
                    "json": {
                        "actionId": "action-annotation",
                        "actionLevel": 1,
                        "processedRecordingUriMap": {
                            "chunked-recording-1": "audio/recording.wav"
                        },
                        "transcriptionChunkProcessedRecordings": [
                            {
                                "transcriptionChunkId": "chunk-1",
                                "chunkedProcessedRecordingId": "chunked-recording-1",
                                "processedRecordingId": "processed-recording-1",
                                "speaker": 1
                            }
                        ],
                        "annotations": [
                            {
                                "id": "annotation-object",
                                "reviewActionId": "action-annotation",
                                "type": "background_noise",
                                "content": {
                                    "primaryLabel": "background-noise",
                                    "secondaryLabel": "rustle"
                                },
                                "processedRecordingId": "processed-recording-1",
                                "startTimeInSeconds": 1.25,
                                "endTimeInSeconds": 2.5,
                                "intensity": 2,
                                "metadata": {
                                    "lowConfidenceResolved": false,
                                    "marker": "kept"
                                },
                                "createdAt": "2026-06-21T12:00:00.000Z",
                                "updatedAt": "2026-06-21T12:00:00.000Z"
                            }
                        ],
                        "lintErrors": []
                    }
                }
            }
        }
    ]);

    let mut project =
        studio_babel::import_json(&source.to_string()).expect("review action should import");
    let mut history = CommandHistory::default();
    history
        .apply_with_policy(
            &mut project,
            EditorCommand::UpdateSegmentMetadata {
                segment_id: "annotation-object".to_string(),
                text: Some("background-noise / chair scrape".to_string()),
                entries: BTreeMap::from([
                    ("intensity".into(), json!(4)),
                    (
                        "content".into(),
                        json!({
                            "primaryLabel": "background-noise",
                            "secondaryLabel": "chair scrape"
                        }),
                    ),
                ]),
            },
            &BabelMutationPolicy::with_clock(|| "2026-06-21T12:34:56.000Z".into()),
        )
        .expect("native annotation metadata edit should apply");

    let exported = studio_babel::export_json(&project).expect("review action should export");

    assert_eq!(
        exported.pointer("/0/result/data/json/annotations/0/content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "chair scrape"
        }))
    );
    assert_eq!(
        exported.pointer("/0/result/data/json/annotations/0/intensity"),
        Some(&json!(4))
    );
    assert_eq!(
        exported.pointer("/0/result/data/json/annotations/0/metadata/marker"),
        Some(&json!("kept"))
    );
    assert_eq!(
        exported.pointer("/0/result/data/json/annotations/0/metadata/lowConfidenceResolved"),
        Some(&json!(true))
    );
    assert_eq!(
        exported.pointer("/0/result/data/json/annotations/0/updatedAt"),
        Some(&json!("2026-06-21T12:34:56.000Z"))
    );
}
