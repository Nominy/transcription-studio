use std::time::{Duration, UNIX_EPOCH};
use studio_core::Project;
use studio_project::RecoverySnapshot;
use studio_services::{
    prefs::{EditorLayout, Preferences, UiStorage},
    recovery,
};

#[test]
fn native_recovery_round_trips_without_a_platform_session() {
    let root = tempfile::tempdir().expect("root");
    let source = tempfile::tempdir().expect("audio source base");
    let snapshot = RecoverySnapshot {
        project: Project {
            id: "local-draft".into(),
            name: "Untitled".into(),
            speakers: Vec::new(),
            lanes: Vec::new(),
            segments: Vec::new(),
            metadata: Default::default(),
        },
        native_path: None,
        saved_at: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    };
    recovery::write_snapshot(&snapshot, root.path(), source.path()).expect("native snapshot");
    let recovered = recovery::load_snapshot(root.path()).expect("native recovery");
    assert_eq!(recovered, snapshot);
    assert!(!root.path().join("project.json").exists());
}
#[test]
fn malformed_storage_is_an_error() {
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().join("ui-local-storage.json");
    std::fs::write(&path, "{not json").expect("seed");
    assert!(UiStorage::load(&path).is_err());
}

#[test]
fn preferences_round_trip_and_tolerate_unknown_or_missing_fields() {
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().join("ui-local-storage.json");
    let mut storage = UiStorage::load(&path).expect("load");
    assert_eq!(storage.preferences().editor_layout, EditorLayout::Classic);

    storage
        .set_preferences(&Preferences {
            editor_layout: EditorLayout::SplitSideBySide,
        })
        .expect("save");
    assert_eq!(
        UiStorage::load(&path).expect("reload").preferences(),
        Preferences {
            editor_layout: EditorLayout::SplitSideBySide
        }
    );

    assert_eq!(
        Preferences::from_storage(Some(r#"{"editorLayout":"splitStacked","futureField":1}"#)),
        Preferences {
            editor_layout: EditorLayout::SplitStacked
        }
    );
    assert_eq!(
        Preferences::from_storage(Some("{}")),
        Preferences::default()
    );
    assert_eq!(
        Preferences::from_storage(Some("garbage")),
        Preferences::default()
    );
    assert_eq!(
        Preferences::from_storage(Some(r#"{"editorLayout":"babel"}"#)),
        Preferences::default()
    );
}
