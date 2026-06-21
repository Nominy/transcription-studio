use std::collections::BTreeMap;

use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_project::{load_project, save_project};

fn sample_project() -> Project {
    let mut metadata = BTreeMap::new();
    metadata.insert("project_preset".to_string(), "ru-gold-2sp-v1".to_string());

    Project {
        id: "babel-task-ru-demo".to_string(),
        name: "Babel task babel-task-ru-demo".to_string(),
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
                id: "track-speaker-1".to_string(),
                speaker_id: "speaker-1".to_string(),
                label: "Speaker 1 track".to_string(),
                source_path: "audio/speaker-1.wav".to_string(),
            },
            AudioLane {
                id: "track-speaker-2".to_string(),
                speaker_id: "speaker-2".to_string(),
                label: "Speaker 2 track".to_string(),
                source_path: "audio/speaker-2.wav".to_string(),
            },
        ],
        segments: vec![
            Segment {
                id: "annotation-1".to_string(),
                source_annotation_id: Some("annotation-1".to_string()),
                speaker_id: "speaker-1".to_string(),
                lane_id: "track-speaker-1".to_string(),
                start_seconds: 1.2,
                end_seconds: 3.4,
                text: "Здравствуйте.".to_string(),
            },
            Segment {
                id: "annotation-2".to_string(),
                source_annotation_id: Some("annotation-2".to_string()),
                speaker_id: "speaker-2".to_string(),
                lane_id: "track-speaker-2".to_string(),
                start_seconds: 3.5,
                end_seconds: 5.1,
                text: "Да, слушаю.".to_string(),
            },
        ],
        metadata,
    }
}

#[test]
fn save_project_materializes_expected_files_and_directories() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let project = sample_project();

    save_project(&project, temp.path()).expect("project should save");

    assert!(temp.path().join("project.json").is_file());
    assert!(temp.path().join("segments.json").is_file());
    assert!(temp.path().join("audio").is_dir());
    assert!(temp.path().join("rules").is_dir());
    assert!(temp.path().join("journal").is_dir());
    assert!(temp.path().join("autosave").is_dir());
}

#[test]
fn load_project_restores_saved_segments() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let project = sample_project();

    save_project(&project, temp.path()).expect("project should save");
    let loaded = load_project(temp.path()).expect("project should load");

    assert_eq!(loaded, project);
    assert_eq!(loaded.segments.len(), 2);
    assert_eq!(loaded.segment("annotation-1").unwrap().text, "Здравствуйте.");
}

#[test]
fn load_project_rejects_missing_manifest_files() {
    let temp = tempfile::tempdir().expect("temp dir should be created");

    assert!(load_project(temp.path()).is_err());
}

