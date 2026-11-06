use exaudio_local::audio::{ImportStrings, import_audio};
use std::{fs, path::PathBuf};

const TONE: &[u8] =
    include_bytes!("../../../crates/studio-waveform/tests/fixtures/babel_line/tone-8k-mono.wav");

fn source(root: &std::path::Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, TONE).unwrap();
    path
}

#[test]
fn same_basename_sources_become_distinct_lanes_without_segments() {
    let temp = tempfile::tempdir().unwrap();
    let paths = [
        source(temp.path(), "first/voice.wav"),
        source(temp.path(), "second/voice.wav"),
    ];
    let imported = import_audio(&paths, ImportStrings::EN, |_, _, _| true).unwrap();
    assert_ne!(imported.project.lanes[0].id, imported.project.lanes[1].id);
    assert_ne!(
        imported.project.lanes[0].source_path,
        imported.project.lanes[1].source_path
    );
    assert_eq!(imported.project.lanes.len(), 2);
    assert_eq!(imported.project.speakers.len(), 2);
    assert!(
        imported.project.segments.is_empty(),
        "opening audio starts with an empty transcript; the user draws or drafts segments"
    );
    for (lane_id, decoded) in &imported.audio {
        let lane = imported
            .project
            .lanes
            .iter()
            .find(|lane| &lane.id == lane_id)
            .unwrap();
        assert!(
            imported
                .project
                .speakers
                .iter()
                .any(|speaker| speaker.id == lane.speaker_id)
        );
        assert!(decoded.audio.duration() > 0.0);
        assert_eq!(decoded.path, PathBuf::from(&lane.source_path));
        assert_eq!(fs::read(&decoded.path).unwrap(), TONE);
    }
    assert!(
        imported.project.metadata.is_empty(),
        "local imports must not acquire task-platform policy"
    );
}

#[test]
fn duplicate_selected_file_does_not_create_duplicate_lane() {
    let temp = tempfile::tempdir().unwrap();
    let path = source(temp.path(), "voice.wav");
    let mut completed = Vec::new();
    let imported = import_audio(
        &[path.clone(), path],
        ImportStrings::EN,
        |done, total, _| {
            completed.push((done, total));
            true
        },
    )
    .unwrap();
    assert_eq!(completed, [(0, 1), (1, 1)]);
    assert_eq!(imported.project.lanes.len(), 1);
    assert_eq!(imported.audio.len(), 1);
    assert_eq!(imported.audio[0].0, imported.project.lanes[0].id);
}

#[test]
fn cancelled_or_corrupt_import_does_not_modify_any_input() {
    let temp = tempfile::tempdir().unwrap();
    let good = source(temp.path(), "voice.wav");
    let bad = temp.path().join("broken.mp3");
    fs::write(&bad, b"not audio").unwrap();
    let cancelled = import_audio(
        &[good.clone(), bad.clone()],
        ImportStrings::EN,
        |done, _, _| done == 0,
    );
    assert!(cancelled.is_err());
    let invalid = import_audio(
        &[good.clone(), bad.clone()],
        ImportStrings::EN,
        |_, _, _| true,
    );
    assert!(invalid.err().unwrap().to_string().contains("broken.mp3"));
    assert_eq!(fs::read(good).unwrap(), TONE);
    assert_eq!(fs::read(bad).unwrap(), b"not audio");
}
