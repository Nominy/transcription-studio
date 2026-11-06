use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_project::{
    ProjectIoError, RecoveryEntry, RecoverySnapshot, import_legacy_project, load_project,
    load_recovery, newest_per_project, read_project_name, save_project, save_recovery,
    superseded_by_save,
};

fn snapshot(
    project: Project,
    native_path: Option<&Path>,
    saved_at: SystemTime,
) -> RecoverySnapshot {
    RecoverySnapshot {
        project,
        native_path: native_path.map(Path::to_path_buf),
        saved_at,
    }
}

fn entry(root: &str, snapshot: RecoverySnapshot) -> RecoveryEntry {
    RecoveryEntry {
        root: PathBuf::from(root),
        snapshot,
    }
}

fn set_modified(path: &Path, time: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

fn sample_project() -> Project {
    Project {
        id: "local-project".to_string(),
        name: "Two speakers".to_string(),
        speakers: vec![Speaker {
            id: "speaker-1".into(),
            label: "Speaker 1".into(),
        }],
        lanes: vec![
            AudioLane {
                id: "lane-1".into(),
                speaker_id: "speaker-1".into(),
                label: "First".into(),
                source_path: "first/voice.wav".into(),
            },
            AudioLane {
                id: "lane-2".into(),
                speaker_id: "speaker-1".into(),
                label: "Second".into(),
                source_path: "second/voice.wav".into(),
            },
        ],
        segments: vec![Segment {
            id: "segment-1".into(),
            source_annotation_id: Some("opaque-source-key".into()),
            speaker_id: "speaker-1".into(),
            lane_id: "lane-1".into(),
            start_seconds: 1.2,
            end_seconds: 3.4,
            text: "Hello.".into(),
            metadata: BTreeMap::from([(
                "ext.example.annotation".into(),
                json!({"nested": [1, "two", null]}),
            )]),
        }],
        metadata: BTreeMap::from([("ext.example.state".into(), "{\"opaque\":true}".into())]),
    }
}

fn sources(base: &Path) {
    fs::create_dir_all(base.join("first")).unwrap();
    fs::create_dir_all(base.join("second")).unwrap();
    // Persistence is byte-preserving and deliberately independent of audio decoders.
    fs::write(base.join("first/voice.wav"), b"first audio bytes").unwrap();
    fs::write(base.join("second/voice.wav"), b"second audio bytes").unwrap();
}

fn manifest(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join("project.json")).unwrap()).unwrap()
}

fn audio_files(root: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(root.join("audio"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn save_uses_one_versioned_manifest_and_preserves_opaque_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("sources");
    let root = temp.path().join("project");
    sources(&base);
    let project = sample_project();
    save_project(&project, &root, &base).unwrap();
    assert!(!root.join("segments.json").exists());
    for name in ["audio", "rules", "journal", "autosave"] {
        assert!(root.join(name).is_dir());
    }
    let stored = manifest(&root);
    assert_eq!(stored["format"], "exaudio-project");
    assert_eq!(stored["version"], 1);
    assert_eq!(stored["project"]["segments"][0]["text"], "Hello.");
    let loaded = load_project(&root).unwrap();
    assert_eq!(loaded.metadata, project.metadata);
    assert_eq!(loaded.segments, project.segments);
    assert_eq!(loaded.speakers, project.speakers);
    assert_eq!(loaded.id, project.id);
    assert_eq!(loaded.name, project.name);
    assert_eq!(
        project.lanes[0].source_path, "first/voice.wav",
        "saving does not mutate the caller"
    );
}

#[test]
fn equal_source_basenames_keep_distinct_audio() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("sources");
    let root = temp.path().join("project");
    sources(&base);
    save_project(&sample_project(), &root, &base).unwrap();
    let stored = manifest(&root);
    let first = stored["project"]["lanes"][0]["source_path"]
        .as_str()
        .unwrap();
    let second = stored["project"]["lanes"][1]["source_path"]
        .as_str()
        .unwrap();
    assert_ne!(first, second);
    assert!(first.starts_with("audio/"));
    assert!(second.starts_with("audio/"));
    assert!(!first.contains('\\'));
    assert_eq!(fs::read(root.join(first)).unwrap(), b"first audio bytes");
    assert_eq!(fs::read(root.join(second)).unwrap(), b"second audio bytes");
    assert_eq!(
        fs::read(base.join("first/voice.wav")).unwrap(),
        b"first audio bytes"
    );
    assert_eq!(audio_files(&root).len(), 2);
}

#[test]
fn moved_folder_reopens_without_original_sources_or_original_root() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("sources");
    let root = temp.path().join("project");
    let moved = temp.path().join("moved-project");
    sources(&base);
    save_project(&sample_project(), &root, &base).unwrap();
    fs::rename(&root, &moved).unwrap();
    fs::remove_dir_all(&base).unwrap();
    let reopened = load_project(&moved).unwrap();
    let canonical = fs::canonicalize(&moved).unwrap();
    assert!(Path::new(&reopened.lanes[0].source_path).starts_with(&canonical));
    assert_eq!(
        fs::read(&reopened.lanes[0].source_path).unwrap(),
        b"first audio bytes"
    );
    assert_eq!(
        fs::read(&reopened.lanes[1].source_path).unwrap(),
        b"second audio bytes"
    );
    save_project(&reopened, &moved, &base).unwrap(); // Absolute owned paths need no old base.
    assert_eq!(load_project(&moved).unwrap(), reopened);
}

#[test]
fn saving_reopened_owned_audio_does_not_recopy_or_replace_it() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("sources");
    let root = temp.path().join("project");
    sources(&base);
    save_project(&sample_project(), &root, &base).unwrap();
    let files = audio_files(&root);
    let before = files
        .iter()
        .map(|path| fs::metadata(path).unwrap().modified().unwrap())
        .collect::<Vec<_>>();
    let mut loaded = load_project(&root).unwrap();
    loaded.segments[0].text = "Edited.".into();
    save_project(&loaded, &root, &root).unwrap();
    let after = files
        .iter()
        .map(|path| fs::metadata(path).unwrap().modified().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(audio_files(&root), files);
    assert_eq!(before, after);
    assert_eq!(load_project(&root).unwrap().segments[0].text, "Edited.");
}

#[test]
fn same_source_references_and_repeated_external_saves_share_owned_files() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    let mut project = sample_project();
    project.lanes[1].source_path = project.lanes[0].source_path.clone();
    save_project(&project, &root, temp.path()).unwrap();
    let files = audio_files(&root);
    save_project(&project, &root, temp.path()).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(audio_files(&root), files);
    let loaded = load_project(&root).unwrap();
    assert_eq!(loaded.lanes[0].source_path, loaded.lanes[1].source_path);
}

#[test]
fn missing_audio_after_a_copy_leaves_old_manifest_and_segments_intact() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    let mut project = sample_project();
    save_project(&project, &root, temp.path()).unwrap();
    let old_manifest = fs::read(root.join("project.json")).unwrap();
    let old_project = load_project(&root).unwrap();
    fs::write(temp.path().join("first/voice.wav"), b"new first audio").unwrap();
    project.name = "Uncommitted name".into();
    project.segments[0].text = "Uncommitted text".into();
    project.lanes[1].source_path = "missing.wav".into();
    let error = save_project(&project, &root, temp.path()).unwrap_err();
    assert!(matches!(error, ProjectIoError::AudioSource { .. }));
    assert!(error.to_string().contains("lane-2"));
    assert!(error.to_string().contains("missing.wav"));
    assert_eq!(fs::read(root.join("project.json")).unwrap(), old_manifest);
    assert_eq!(load_project(&root).unwrap(), old_project);
    assert_eq!(
        fs::read(&old_project.lanes[0].source_path).unwrap(),
        b"first audio bytes"
    );
}

#[test]
fn corrupt_content_addressed_audio_is_not_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    let project = sample_project();
    save_project(&project, &root, temp.path()).unwrap();
    let stored = load_project(&root).unwrap();
    fs::write(&stored.lanes[0].source_path, b"externally modified").unwrap();
    let before = fs::read(root.join("project.json")).unwrap();
    assert!(save_project(&project, &root, temp.path()).is_err());
    assert_eq!(fs::read(root.join("project.json")).unwrap(), before);
    assert_eq!(
        fs::read(&stored.lanes[0].source_path).unwrap(),
        b"externally modified"
    );
}

#[test]
fn recovery_uses_native_contract_without_changing_manual_save() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    let moved = temp.path().join("moved");
    let mut project = sample_project();
    save_project(&project, &root, temp.path()).unwrap();
    let before = fs::read(root.join("project.json")).unwrap();
    project.segments[0].text = "Recovered edit".into();
    let taken = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
    save_recovery(
        &snapshot(project.clone(), Some(&root), taken),
        &root,
        temp.path(),
    )
    .unwrap();
    assert_eq!(fs::read(root.join("project.json")).unwrap(), before);
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("autosave/recovery.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], "exaudio-project");
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["project"]["segments"][0]["text"], "Recovered edit");
    fs::rename(&root, &moved).unwrap();
    let recovered = load_recovery(&moved).unwrap();
    assert_eq!(recovered.project.segments, project.segments);
    assert_eq!(recovered.project.metadata, project.metadata);
    assert_eq!(recovered.native_path.as_deref(), Some(root.as_path()));
    assert_eq!(recovered.saved_at, taken);
    assert!(Path::new(&recovered.project.lanes[0].source_path).is_file());
    assert_eq!(load_project(&moved).unwrap().segments[0].text, "Hello.");
}

#[test]
fn saved_project_manifest_never_carries_snapshot_metadata() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    save_project(&sample_project(), &root, temp.path()).unwrap();
    let manifest = manifest(&root);
    assert!(manifest.get("native_path").is_none());
    assert!(manifest.get("saved_at_unix_ms").is_none());
    assert_eq!(read_project_name(&root).unwrap(), "Two speakers");
}

#[test]
fn snapshot_without_recorded_time_or_folder_uses_file_time() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    let taken = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    save_recovery(&snapshot(sample_project(), None, taken), &root, temp.path()).unwrap();
    let path = root.join("autosave/recovery.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(manifest.get("native_path").is_none());
    manifest.as_object_mut().unwrap().remove("saved_at_unix_ms");
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let file_time = UNIX_EPOCH + Duration::from_secs(1_650_000_000);
    set_modified(&path, file_time);
    let recovered = load_recovery(&root).unwrap();
    assert_eq!(recovered.native_path, None);
    assert_eq!(recovered.saved_at, file_time);
}

#[test]
fn failed_recovery_keeps_previous_snapshot_and_recovery_can_precede_manual_save() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    let mut project = sample_project();
    let taken = SystemTime::now();
    save_recovery(&snapshot(project.clone(), None, taken), &root, temp.path()).unwrap();
    assert!(!root.join("project.json").exists());
    let before = load_recovery(&root).unwrap();
    project.lanes[0].source_path = "missing.wav".into();
    assert!(save_recovery(&snapshot(project, None, taken), &root, temp.path()).is_err());
    assert_eq!(load_recovery(&root).unwrap(), before);
}

#[test]
fn snapshot_is_superseded_only_by_a_later_save_to_its_folder() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    save_project(&sample_project(), &root, temp.path()).unwrap();
    let saved = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    set_modified(&root.join("project.json"), saved);
    let before_save = snapshot(
        sample_project(),
        Some(&root),
        saved - Duration::from_secs(60),
    );
    let after_save = snapshot(
        sample_project(),
        Some(&root),
        saved + Duration::from_secs(60),
    );
    let never_saved = snapshot(sample_project(), None, saved - Duration::from_secs(60));
    let folder_gone = snapshot(
        sample_project(),
        Some(&temp.path().join("deleted")),
        saved - Duration::from_secs(60),
    );
    assert!(superseded_by_save(&before_save));
    assert!(!superseded_by_save(&after_save));
    assert!(!superseded_by_save(&never_saved));
    assert!(!superseded_by_save(&folder_gone));
}

#[test]
fn newest_snapshot_per_project_wins_and_pre_save_snapshots_are_absorbed() {
    let t = |seconds: u64| UNIX_EPOCH + Duration::from_secs(seconds);
    let folder = Path::new("/projects/interview");
    let other_folder = Path::new("/projects/interview-copy");
    let mut other = sample_project();
    other.id = "other-project".into();
    let entries = vec![
        entry("a", snapshot(sample_project(), None, t(100))),
        entry("b", snapshot(sample_project(), Some(folder), t(300))),
        entry("c", snapshot(sample_project(), Some(folder), t(200))),
        entry("d", snapshot(sample_project(), Some(other_folder), t(250))),
        entry("e", snapshot(other.clone(), None, t(50))),
        entry("f", snapshot(other, None, t(400))),
    ];
    let (kept, superseded) = newest_per_project(entries);
    let roots = |entries: &[RecoveryEntry]| {
        entries
            .iter()
            .map(|entry| entry.root.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(roots(&kept), ["f", "b", "d"]);
    assert_eq!(roots(&superseded), ["c", "a", "e"]);
}

#[test]
fn extension_or_account_metadata_does_not_route_native_saves() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("chosen-native-folder");
    let mut project = sample_project();
    project
        .metadata
        .insert("account_id".into(), "opaque-account".into());
    project
        .metadata
        .insert("ext.example.remote_task".into(), "opaque-task".into());
    save_project(&project, &root, temp.path()).unwrap();
    assert_eq!(load_project(&root).unwrap().metadata, project.metadata);
    assert!(root.join("project.json").is_file());
}

#[test]
fn rejects_traversal_absolute_and_nonportable_stored_paths() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    save_project(&sample_project(), &root, temp.path()).unwrap();
    let mut stored = manifest(&root);
    for source in [
        "../outside.wav",
        "audio/../outside.wav",
        "/outside.wav",
        "C:/outside.wav",
        "audio/C:outside.wav",
        "audio\\outside.wav",
        "audio/sub/file.wav",
        "audio/CON.wav",
    ] {
        stored["project"]["lanes"][0]["source_path"] = json!(source);
        fs::write(
            root.join("project.json"),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();
        assert!(
            load_project(&root).is_err(),
            "accepted unsafe reference: {source}"
        );
    }
}

#[test]
fn relative_source_paths_cannot_escape_the_explicit_base() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let mut project = sample_project();
    project.lanes[0].source_path = "../first/voice.wav".into();
    assert!(
        save_project(
            &project,
            temp.path().join("project"),
            temp.path().join("second")
        )
        .is_err()
    );
}

#[test]
fn missing_native_audio_is_rejected_on_open() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    save_project(&sample_project(), &root, temp.path()).unwrap();
    let loaded = load_project(&root).unwrap();
    fs::remove_file(&loaded.lanes[0].source_path).unwrap();
    assert!(load_project(&root).is_err());
}

#[test]
fn unsupported_versions_are_not_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    let project = sample_project();
    save_project(&project, &root, temp.path()).unwrap();
    let mut stored = manifest(&root);
    stored["version"] = json!(999);
    let bytes = serde_json::to_vec(&stored).unwrap();
    fs::write(root.join("project.json"), &bytes).unwrap();
    assert!(load_project(&root).is_err());
    assert!(save_project(&project, &root, temp.path()).is_err());
    assert_eq!(fs::read(root.join("project.json")).unwrap(), bytes);
}

#[test]
fn legacy_split_format_requires_explicit_import_to_new_destination() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let project = sample_project();
    let mut legacy = serde_json::to_value(&project).unwrap();
    legacy.as_object_mut().unwrap().remove("segments");
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(temp.path().join("project.json"), &bytes).unwrap();
    fs::write(
        temp.path().join("segments.json"),
        serde_json::to_vec(&project.segments).unwrap(),
    )
    .unwrap();
    assert!(load_project(temp.path()).is_err());
    assert!(save_project(&project, temp.path(), temp.path()).is_err());
    let imported = import_legacy_project(temp.path()).unwrap();
    assert_eq!(imported.segments, project.segments);
    assert_eq!(imported.metadata, project.metadata);
    let destination = temp.path().join("converted");
    save_project(&imported, &destination, temp.path()).unwrap();
    assert_eq!(
        load_project(&destination).unwrap().segments,
        project.segments
    );
    assert_eq!(fs::read(temp.path().join("project.json")).unwrap(), bytes);
}

#[test]
fn empty_project_and_missing_manifest_are_handled() {
    let temp = tempfile::tempdir().unwrap();
    assert!(load_project(temp.path()).is_err());
    assert!(load_recovery(temp.path()).is_err());
    let mut project = sample_project();
    project.lanes.clear();
    project.segments.clear();
    save_project(&project, temp.path(), temp.path().join("unused-base")).unwrap();
    assert_eq!(load_project(temp.path()).unwrap(), project);
}

#[cfg(unix)]
#[test]
fn native_audio_directory_and_files_cannot_be_symlinks() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    symlink(temp.path().join("first"), root.join("audio")).unwrap();
    assert!(save_project(&sample_project(), &root, temp.path()).is_err());
    fs::remove_file(root.join("audio")).unwrap();
    save_project(&sample_project(), &root, temp.path()).unwrap();
    let loaded = load_project(&root).unwrap();
    fs::remove_file(&loaded.lanes[0].source_path).unwrap();
    symlink(
        temp.path().join("first/voice.wav"),
        &loaded.lanes[0].source_path,
    )
    .unwrap();
    assert!(load_project(&root).is_err());
    assert!(save_project(&sample_project(), &root, temp.path()).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_manifest_or_recovery_directory_is_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    save_project(&sample_project(), &root, temp.path()).unwrap();
    let outside = temp.path().join("outside.json");
    fs::rename(root.join("project.json"), &outside).unwrap();
    let bytes = fs::read(&outside).unwrap();
    symlink(&outside, root.join("project.json")).unwrap();
    assert!(load_project(&root).is_err());
    assert!(save_project(&sample_project(), &root, temp.path()).is_err());
    assert_eq!(fs::read(&outside).unwrap(), bytes);
    fs::remove_dir(root.join("autosave")).unwrap();
    symlink(temp.path(), root.join("autosave")).unwrap();
    let snapshot = snapshot(sample_project(), None, SystemTime::now());
    assert!(save_recovery(&snapshot, &root, temp.path()).is_err());
    assert!(load_recovery(&root).is_err());
}

#[test]
fn nonfinite_segment_times_cannot_replace_a_readable_save() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    let mut project = sample_project();
    save_project(&project, &root, temp.path()).unwrap();
    let before = fs::read(root.join("project.json")).unwrap();
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        project.segments[0].start_seconds = invalid;
        assert!(save_project(&project, &root, temp.path()).is_err());
        assert_eq!(fs::read(root.join("project.json")).unwrap(), before);
        assert_eq!(load_project(&root).unwrap().segments[0].text, "Hello.");
    }
}

#[test]
fn uncommitted_crash_artifacts_are_not_treated_as_a_new_save() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let root = temp.path().join("project");
    save_project(&sample_project(), &root, temp.path()).unwrap();
    let before = load_project(&root).unwrap();
    fs::write(root.join(".uncommitted-manifest"), b"{\"partial\":").unwrap();
    fs::write(root.join("audio/.uncommitted-audio"), b"partial bytes").unwrap();
    assert_eq!(load_project(&root).unwrap(), before);
    save_project(&before, &root, &root).unwrap();
    assert_eq!(load_project(&root).unwrap(), before);
}

#[test]
fn save_as_copies_owned_audio_to_an_independent_project() {
    let temp = tempfile::tempdir().unwrap();
    sources(temp.path());
    let first = temp.path().join("first-project");
    let second = temp.path().join("second-project");
    save_project(&sample_project(), &first, temp.path()).unwrap();
    let loaded = load_project(&first).unwrap();
    save_project(&loaded, &second, &first).unwrap();
    fs::remove_dir_all(&first).unwrap();
    let copied = load_project(&second).unwrap();
    assert_eq!(copied.segments, loaded.segments);
    assert_eq!(
        fs::read(&copied.lanes[0].source_path).unwrap(),
        b"first audio bytes"
    );
    assert_eq!(
        fs::read(&copied.lanes[1].source_path).unwrap(),
        b"second audio bytes"
    );
}
