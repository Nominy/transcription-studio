use studio_babel::{BabelTaskPackage, export_package, import_package};

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
