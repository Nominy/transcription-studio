use std::{collections::BTreeMap, path::Path};

use studio_core::Project;
use studio_services::{
    babel::META_REVIEW_ACTION_ID,
    prefs::{
        EditorLayout, ORIGINAL_SCRIPT_HIDDEN_KEY, Preferences, SCRIPT_CONTEXT_HIDDEN_KEY, UiStorage,
    },
    recovery,
};

fn project(id: &str, review_action_id: Option<&str>) -> Project {
    Project {
        id: id.to_string(),
        name: "Task".to_string(),
        speakers: Vec::new(),
        lanes: Vec::new(),
        segments: Vec::new(),
        metadata: review_action_id
            .map(|value| BTreeMap::from([(META_REVIEW_ACTION_ID.to_string(), value.to_string())]))
            .unwrap_or_default(),
    }
}

#[test]
fn panel_collapse_state_persists_across_restart() {
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().join("ui-local-storage.json");
    let mut storage = UiStorage::load(&path).expect("missing file loads empty");
    assert!(!storage.is_collapsed(SCRIPT_CONTEXT_HIDDEN_KEY));

    storage
        .set_collapsed(SCRIPT_CONTEXT_HIDDEN_KEY, true)
        .expect("persist");

    let restarted = UiStorage::load(&path).expect("reload");
    assert!(restarted.is_collapsed(SCRIPT_CONTEXT_HIDDEN_KEY));
    assert!(!restarted.is_collapsed(ORIGINAL_SCRIPT_HIDDEN_KEY));
}

#[test]
fn collapse_flags_use_exact_true_string_semantics() {
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().join("ui-local-storage.json");
    std::fs::write(
        &path,
        serde_json::json!({
            SCRIPT_CONTEXT_HIDDEN_KEY: "TRUE",
            ORIGINAL_SCRIPT_HIDDEN_KEY: "true"
        })
        .to_string(),
    )
    .expect("seed");

    let storage = UiStorage::load(&path).expect("load");
    assert!(!storage.is_collapsed(SCRIPT_CONTEXT_HIDDEN_KEY));
    assert!(storage.is_collapsed(ORIGINAL_SCRIPT_HIDDEN_KEY));
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

#[test]
fn recovery_paths_are_stable_and_safe_for_review_action_ids() {
    let dir = Path::new("recovery");
    let path =
        recovery::snapshot_path_in(dir, &project("fallback/project", Some("review/action:42")));
    assert_eq!(path, dir.join("review-action-42.json"));
    assert_eq!(
        recovery::snapshot_path_in(dir, &project("fallback/project", Some("  "))),
        dir.join("fallback-project.json")
    );
}

#[test]
fn recovery_snapshots_are_written_discovered_loaded_and_removed() {
    let temp = tempfile::tempdir().expect("temp dir");
    let dir = temp.path().join("recovery");
    let source = include_str!("../../studio-babel/tests/fixtures/review-action-final.json");
    let older = project("older", None);
    let newer = project("newer", Some("review-1"));

    recovery::write_snapshot_in(&dir, &older, source).expect("write older");
    std::thread::sleep(std::time::Duration::from_millis(20));
    recovery::write_snapshot_in(&dir, &newer, source).expect("write newer");
    std::fs::write(dir.join("empty.json"), "").expect("empty file");
    std::fs::write(dir.join("notes.txt"), "x").expect("other file");

    let drafts = recovery::discover_drafts_in(&dir);
    let labels: Vec<_> = drafts.iter().map(|draft| draft.label.as_str()).collect();
    assert_eq!(labels, ["review-1", "older"]);

    let workflow = recovery::load_draft(&drafts[0]).expect("load draft");
    assert_eq!(workflow.project().segments.len(), 24);

    recovery::remove_snapshot_in(&dir, &newer);
    recovery::remove_snapshot_in(&dir, &newer);
    assert_eq!(recovery::discover_drafts_in(&dir).len(), 1);
}
