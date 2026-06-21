use studio_babel::{
    BabelAnnotation, BabelAudioTrack, BabelImportError, BabelSpeaker, BabelTaskPackage,
    export_package, import_package,
};

fn load_fixture() -> BabelTaskPackage {
    let source = include_str!("fixtures/sample-task.json");
    serde_json::from_str(source).expect("fixture should be valid package JSON")
}

#[test]
fn import_normalizes_babel_package_into_project() {
    let package = load_fixture();

    let project = import_package(package).expect("package should import");

    assert_eq!(project.id, "babel-task-ru-demo");
    assert_eq!(project.name, "Babel task babel-task-ru-demo");
    assert_eq!(project.speakers.len(), 2);
    assert_eq!(project.lanes.len(), 2);
    assert_eq!(project.segments.len(), 3);
    assert_eq!(
        project.metadata.get("project_preset").map(String::as_str),
        Some("ru-gold-2sp-v1")
    );

    let first = project.segment("annotation-1").unwrap();
    assert_eq!(first.source_annotation_id.as_deref(), Some("annotation-1"));
    assert_eq!(first.speaker_id, "speaker-1");
    assert_eq!(first.lane_id, "track-speaker-1");
    assert_eq!(first.start_seconds, 1.2);
    assert_eq!(first.end_seconds, 3.4);
    assert_eq!(first.text, "Здравствуйте.");
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
fn import_rejects_audio_track_with_unknown_speaker() {
    let mut package = load_fixture();
    package.audio_tracks.push(BabelAudioTrack {
        track_id: "orphan-track".to_string(),
        speaker_key: "missing-speaker".to_string(),
        path: "audio/orphan.wav".to_string(),
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
    });
    assert_eq!(
        import_package(duplicate_annotation),
        Err(BabelImportError::DuplicateAnnotation(
            "annotation-1".to_string()
        ))
    );
}
