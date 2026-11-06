use serde_json::{Value, json};
use studio_babel::{
    BabelAnnotation, BabelAudioTrack, BabelImportError, BabelSpeaker, BabelTaskPackage,
    export_json, export_package, import_json, import_package,
};

fn load_fixture() -> BabelTaskPackage {
    let source = include_str!("fixtures/sample-task.json");
    serde_json::from_str(source).expect("fixture should be valid package JSON")
}

#[test]
fn review_action_final_json_round_trips_payload() {
    let final_json = include_str!("fixtures/review-action-final.json");

    let project = import_json(final_json).expect("review action final json should import");
    assert_eq!(project.segments.len(), 24);
    assert_eq!(project.lanes.len(), 2);
    let exported = export_json(&project).expect("review action final json should export");
    assert_eq!(
        exported
            .pointer("/0/result/data/json/annotations")
            .and_then(Value::as_array)
            .expect("exported annotations")
            .len(),
        24
    );
    assert_eq!(
        exported.pointer("/0/result/data/json/success"),
        Some(&json!(true))
    );
}

#[test]
fn import_json_accepts_babel_review_action_envelope() {
    let source = include_str!("fixtures/review-action-final.json");

    let project = import_json(source).expect("Babel review action should import");

    assert_eq!(project.id, "f5b736b9-60ff-5355-ae5a-e909aa644e72");
    assert_eq!(project.speakers.len(), 2);
    assert_eq!(project.lanes.len(), 2);
    assert_eq!(project.segments.len(), 24);
    assert_eq!(
        project.metadata.get("babel.format").map(String::as_str),
        Some("review-action")
    );

    let first = project
        .segment("6219876d-eba7-529f-b6c9-d949dbabfd12")
        .expect("first annotation should exist");
    assert_eq!(first.speaker_id, "speaker-2");
    assert_eq!(first.lane_id, "track-speaker-2");
    assert_eq!(first.start_seconds, 0.78);
    assert!(first.text.starts_with("Слушай"));
    assert_eq!(
        first.metadata.get("lowConfidenceResolved"),
        Some(&json!(true))
    );
    assert_eq!(
        first.metadata.get("reviewActionId"),
        Some(&json!("e823c3bb-2697-55fc-9272-c083a55874da"))
    );
}

#[test]
fn review_action_top_level_studio_metadata_imports_as_project_metadata() {
    let source = json!([
        {
            "result": {
                "data": {
                    "json": {
                        "actionId": "action-feedback",
                        "actionLevel": 1,
                        "studio.babel.feedback.showButton": true,
                        "studio.babel.feedback.rubricFlags": "tx-word-accuracy,tx-other-feedback",
                        "studio.babel.feedback.formStepResponseId": "form-step-response-1",
                        "studio.babel.feedback.inputs": [
                            { "id": "rating", "label": "Word Accuracy" },
                            { "id": "comment", "label": "Word Accuracy Comment" },
                            { "id": "other", "label": "Other Feedback" }
                        ],
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
                                "id": "annotation-feedback",
                                "reviewActionId": "action-feedback",
                                "type": "transcription",
                                "content": "Feedback import smoke",
                                "processedRecordingId": "processed-recording-1",
                                "startTimeInSeconds": 0.0,
                                "endTimeInSeconds": 1.0,
                                "metadata": null
                            }
                        ],
                        "lintErrors": []
                    }
                }
            }
        }
    ]);

    let project = import_json(&source.to_string()).expect("feedback review action should import");

    assert_eq!(
        project
            .metadata
            .get("studio.babel.feedback.showButton")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        project
            .metadata
            .get("studio.babel.feedback.rubricFlags")
            .map(String::as_str),
        Some("tx-word-accuracy,tx-other-feedback")
    );
    assert_eq!(
        project
            .metadata
            .get("studio.babel.feedback.formStepResponseId")
            .map(String::as_str),
        Some("form-step-response-1")
    );
    assert!(
        project
            .metadata
            .get("studio.babel.feedback.inputs")
            .is_some_and(|value| value.contains("\"Word Accuracy Comment\""))
    );
}

#[test]
fn import_json_accepts_single_trpc_review_action_result_object() {
    let source = json!({
        "result": {
            "data": {
                "json": {
                    "actionId": "action-feedback-object",
                    "actionLevel": 1,
                    "studio.babel.feedback.showButton": true,
                    "studio.babel.feedback.initialOpen": true,
                    "studio.babel.feedback.rubricFlags": "tx-word-accuracy,tx-other-feedback",
                    "studio.babel.feedback.formStepResponseId": "object-form-step-response",
                    "studio.babel.feedback.inputs": [
                        { "id": "rating", "label": "Word Accuracy" },
                        { "id": "comment", "label": "Word Accuracy Comment" },
                        { "id": "other", "label": "Other Feedback" }
                    ],
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
                            "id": "annotation-feedback-object",
                            "reviewActionId": "action-feedback-object",
                            "type": "transcription",
                            "content": "Feedback object import smoke",
                            "processedRecordingId": "processed-recording-1",
                            "startTimeInSeconds": 0.0,
                            "endTimeInSeconds": 1.0,
                            "metadata": null
                        }
                    ],
                    "lintErrors": []
                }
            }
        }
    });

    let project = import_json(&source.to_string()).expect("single tRPC result should import");

    assert_eq!(project.segments.len(), 1);
    assert_eq!(
        project
            .metadata
            .get("studio.babel.feedback.formStepResponseId")
            .map(String::as_str),
        Some("object-form-step-response")
    );
    let exported = export_json(&project).expect("single tRPC result should export");
    assert_eq!(
        exported.pointer("/result/data/json/annotations/0/content"),
        Some(&json!("Feedback object import smoke"))
    );
}

#[test]
fn import_json_accepts_normalized_trpc_review_action_data_object() {
    let source = json!({
        "json": {
            "actionId": "action-feedback-normalized",
            "actionLevel": 1,
            "studio.babel.feedback.showButton": true,
            "studio.babel.feedback.formStepResponseId": "normalized-form-step-response",
            "studio.babel.feedback.inputs": [
                { "id": "rating", "label": "Word Accuracy" },
                { "id": "comment", "label": "Word Accuracy Comment" }
            ],
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
                    "id": "annotation-feedback-normalized",
                    "reviewActionId": "action-feedback-normalized",
                    "type": "transcription",
                    "content": "Feedback normalized import smoke",
                    "processedRecordingId": "processed-recording-1",
                    "startTimeInSeconds": 0.0,
                    "endTimeInSeconds": 1.0,
                    "metadata": null
                }
            ],
            "lintErrors": []
        }
    });

    let project = import_json(&source.to_string()).expect("normalized tRPC data should import");

    assert_eq!(
        project
            .metadata
            .get("studio.babel.feedback.formStepResponseId")
            .map(String::as_str),
        Some("normalized-form-step-response")
    );
    let exported = export_json(&project).expect("normalized tRPC data should export");
    assert_eq!(
        exported.pointer("/json/annotations/0/content"),
        Some(&json!("Feedback normalized import smoke"))
    );
}

#[test]
fn review_action_annotation_object_content_round_trips_through_metadata() {
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
                                "intensity": 3,
                                "metadata": null,
                                "createdAt": null,
                                "updatedAt": null
                            }
                        ],
                        "lintErrors": []
                    }
                }
            }
        }
    ]);

    let project = import_json(&source.to_string()).expect("annotation review action should import");
    let segment = project
        .segment("annotation-object")
        .expect("annotation object segment should exist");

    assert_eq!(
        segment.metadata.get("type"),
        Some(&json!("background_noise"))
    );
    assert_eq!(segment.metadata.get("intensity"), Some(&json!(3)));
    assert_eq!(
        segment.metadata.get("content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );

    let exported = export_json(&project).expect("annotation review action should export");

    assert_eq!(
        exported.pointer("/0/result/data/json/annotations/0/content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );
    assert_eq!(
        exported.pointer("/0/result/data/json/annotations/0/type"),
        Some(&json!("background_noise"))
    );
}

#[test]
fn export_json_preserves_review_action_envelope_after_edits() {
    let source = include_str!("fixtures/review-action-final.json");
    let mut project = import_json(source).expect("Babel review action should import");
    let segment = project
        .segment_mut("6219876d-eba7-529f-b6c9-d949dbabfd12")
        .expect("editable annotation should exist");
    segment.text = "Edited native review text.".to_string();
    segment.start_seconds = 1.25;
    segment.end_seconds = 2.75;

    let exported = export_json(&project).expect("review action should export as JSON");

    assert!(exported.is_array());
    assert_eq!(
        exported.pointer("/0/result/data/json/actionId"),
        Some(&json!("e823c3bb-2697-55fc-9272-c083a55874da"))
    );
    assert_eq!(
        exported.pointer("/0/result/data/json/success"),
        Some(&json!(true))
    );
    assert!(exported.get("task_id").is_none());

    let annotations = exported
        .pointer("/0/result/data/json/annotations")
        .and_then(serde_json::Value::as_array)
        .expect("review action annotations should remain in the envelope");
    assert_eq!(annotations.len(), project.segments.len());
    let edited = annotations
        .iter()
        .find(|annotation| {
            annotation.get("id") == Some(&json!("6219876d-eba7-529f-b6c9-d949dbabfd12"))
        })
        .expect("edited annotation should export in review-action shape");
    assert_eq!(edited["content"], "Edited native review text.");
    assert_eq!(edited["startTimeInSeconds"], 1.25);
    assert_eq!(edited["endTimeInSeconds"], 2.75);
    assert_eq!(
        edited["processedRecordingId"],
        "8aec034b-1486-546a-89de-47ee0ab3ee60"
    );
    assert_eq!(
        exported
            .pointer(
                "/0/result/data/json/processedRecordingUriMap/7a54b1cc-d57a-52b2-a2a6-d00ab89ab96b"
            )
            .and_then(serde_json::Value::as_str),
        Some(
            "s3://exaudio-demo-audio/transcription-chunks/demo/afc30681-b4f7-56b5-9fa0-e2fb5ec7b00b/f5b736b9-60ff-5355-ae5a-e909aa644e72/8aec034b-1486-546a-89de-47ee0ab3ee60.wav"
        )
    );
}

#[test]
fn export_json_updates_review_action_decision_from_project_metadata() {
    let source = include_str!("fixtures/review-action-final.json");
    let mut project = import_json(source).expect("Babel review action should import");
    project.metadata.insert(
        "babel.review_action.actionDecision".to_string(),
        "pass".to_string(),
    );

    let exported = export_json(&project).expect("review action should export as JSON");

    assert_eq!(
        exported.pointer("/0/result/data/json/actionDecision"),
        Some(&json!("pass"))
    );
}

#[test]
fn import_json_normalizes_review_action_decision_metadata() {
    let source = include_str!("fixtures/review-action-final.json");
    let project = import_json(source).expect("review action should import");

    assert_eq!(
        project.metadata.get("babel.review_action.actionDecision"),
        Some(&"in-progress".to_string())
    );
}

#[test]
fn import_normalizes_babel_package_into_project() {
    let package = load_fixture();

    let project = import_package(package).expect("package should import");

    assert_eq!(project.id, "f5b736b9-60ff-5355-ae5a-e909aa644e72");
    assert_eq!(
        project.name,
        "Babel task f5b736b9-60ff-5355-ae5a-e909aa644e72"
    );
    assert_eq!(project.speakers.len(), 2);
    assert_eq!(project.lanes.len(), 2);
    assert_eq!(project.segments.len(), 24);
    assert_eq!(
        project.metadata.get("project_preset").map(String::as_str),
        Some("babel-review-action")
    );

    let first = project.segment("annotation-1").unwrap();
    assert_eq!(first.source_annotation_id.as_deref(), Some("annotation-1"));
    assert_eq!(first.speaker_id, "speaker-2");
    assert_eq!(first.lane_id, "track-speaker-2");
    assert_eq!(first.start_seconds, 0.78);
    assert_eq!(first.end_seconds, 15.45);
    assert!(first.text.starts_with("Слушай"));
    assert_eq!(
        first.metadata.get("studio.babel.original.text"),
        Some(&json!(first.text.as_str()))
    );
    assert_eq!(
        first.metadata.get("studio.babel.original.startSeconds"),
        Some(&json!(0.78))
    );
    assert_eq!(
        first.metadata.get("studio.babel.original.endSeconds"),
        Some(&json!(15.45))
    );
    assert_eq!(
        first.metadata.get("studio.babel.original.speakerId"),
        Some(&json!("speaker-2"))
    );
    assert_eq!(
        first.metadata.get("studio.babel.original.laneId"),
        Some(&json!("track-speaker-2"))
    );

    assert_eq!(
        project
            .metadata
            .get("babel.extra.annotation.annotation-1")
            .map(|extra| {
                extra.contains("reviewActionId")
                    && extra.contains("6219876d-eba7-529f-b6c9-d949dbabfd12")
            }),
        Some(true)
    );
}

#[test]
fn export_preserves_babel_round_trip_fields() {
    let package = load_fixture();
    let project = import_package(package.clone()).expect("package should import");

    let exported = export_package(&project).expect("project should export");

    assert_eq!(exported.task_id, package.task_id);
    assert_eq!(exported.project_preset, package.project_preset);
    assert_eq!(exported.speakers, package.speakers);
    assert_eq!(exported.audio_tracks, package.audio_tracks);
    assert_eq!(exported.annotations, package.annotations);
}

#[test]
fn export_preserves_unknown_babel_fields_from_imported_package() {
    let source = json!({
        "task_id": "task-extra",
        "project_preset": "ru-gold-2sp-v1",
        "review_batch": { "queue": "gold", "priority": 2 },
        "speakers": [
            {
                "speaker_key": "speaker-1",
                "label": "Speaker 1",
                "display_color": "violet"
            }
        ],
        "audio_tracks": [
            {
                "track_id": "track-1",
                "speaker_key": "speaker-1",
                "path": "audio/speaker-1.wav",
                "waveform_bucket": "wf-1"
            }
        ],
        "annotations": [
            {
                "annotation_id": "annotation-1",
                "speaker_key": "speaker-1",
                "track_id": "track-1",
                "start_seconds": 1.0,
                "end_seconds": 2.0,
                "text": "hello",
                "rule_state": { "lint": "ok" }
            }
        ]
    });
    let package: BabelTaskPackage = serde_json::from_value(source).unwrap();
    let project = import_package(package).expect("package should import");

    let exported = export_package(&project).expect("project should export");
    let value = serde_json::to_value(exported).unwrap();

    assert_eq!(value["review_batch"]["queue"], "gold");
    assert_eq!(value["speakers"][0]["display_color"], "violet");
    assert_eq!(value["audio_tracks"][0]["waveform_bucket"], "wf-1");
    assert_eq!(value["annotations"][0]["rule_state"]["lint"], "ok");
}

#[test]
fn import_rejects_audio_track_with_unknown_speaker() {
    let mut package = load_fixture();
    package.audio_tracks.push(BabelAudioTrack {
        track_id: "orphan-track".to_string(),
        speaker_key: "missing-speaker".to_string(),
        path: "audio/orphan.wav".to_string(),
        extra: Default::default(),
    });

    let result = import_package(package);

    assert_eq!(
        result,
        Err(BabelImportError::UnknownTrackSpeaker {
            track_id: "orphan-track".to_string(),
            speaker_key: "missing-speaker".to_string()
        })
    );
}

#[test]
fn import_rejects_annotation_using_track_from_another_speaker() {
    let mut package = load_fixture();
    package.annotations.push(BabelAnnotation {
        annotation_id: "annotation-crossed".to_string(),
        speaker_key: "speaker-1".to_string(),
        track_id: "track-speaker-2".to_string(),
        start_seconds: 8.0,
        end_seconds: 9.0,
        text: "Wrong lane.".to_string(),
        extra: Default::default(),
    });

    let result = import_package(package);

    assert_eq!(
        result,
        Err(BabelImportError::AnnotationTrackSpeakerMismatch {
            annotation_id: "annotation-crossed".to_string(),
            speaker_key: "speaker-1".to_string(),
            track_id: "track-speaker-2".to_string(),
            track_speaker_key: "speaker-2".to_string()
        })
    );
}

#[test]
fn import_rejects_duplicate_speaker_track_and_annotation_ids() {
    let mut duplicate_speaker = load_fixture();
    duplicate_speaker.speakers.push(BabelSpeaker {
        speaker_key: "speaker-1".to_string(),
        label: "Duplicate speaker".to_string(),
        extra: Default::default(),
    });
    assert_eq!(
        import_package(duplicate_speaker),
        Err(BabelImportError::DuplicateSpeaker("speaker-1".to_string()))
    );

    let mut duplicate_track = load_fixture();
    duplicate_track.audio_tracks.push(BabelAudioTrack {
        track_id: "track-speaker-1".to_string(),
        speaker_key: "speaker-1".to_string(),
        path: "audio/duplicate.wav".to_string(),
        extra: Default::default(),
    });
    assert_eq!(
        import_package(duplicate_track),
        Err(BabelImportError::DuplicateTrack(
            "track-speaker-1".to_string()
        ))
    );

    let mut duplicate_annotation = load_fixture();
    duplicate_annotation.annotations.push(BabelAnnotation {
        annotation_id: "annotation-1".to_string(),
        speaker_key: "speaker-1".to_string(),
        track_id: "track-speaker-1".to_string(),
        start_seconds: 8.0,
        end_seconds: 9.0,
        text: "Duplicate annotation.".to_string(),
        extra: Default::default(),
    });
    assert_eq!(
        import_package(duplicate_annotation),
        Err(BabelImportError::DuplicateAnnotation(
            "annotation-1".to_string()
        ))
    );
}
