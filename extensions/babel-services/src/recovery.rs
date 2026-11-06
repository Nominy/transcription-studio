//! Local crash/draft recovery snapshots of Babel tasks
//! (`<app support>/recovery/<review-action-id>.json`, Babel JSON export format).
//!
//! A Babel document's save pipeline writes a snapshot, removes it once that document's
//! direct or relay client confirms the save, and lists remaining drafts newest first.
//! These helpers never select a save route or inspect global login state.

use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use studio_core::Project;
use studio_editor::workflow::StudioWorkflow;

use crate::babel::META_REVIEW_ACTION_ID;
use studio_services::paths::app_support_dir;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryDraft {
    pub path: PathBuf,
    /// File stem, i.e. the sanitized review action id (or project id).
    pub label: String,
    /// Last modification time in milliseconds since the Unix epoch.
    pub modified_millis: u128,
}

/// `<app support>/recovery`.
pub fn recovery_dir() -> PathBuf {
    app_support_dir().join("recovery")
}

/// Snapshot path for `project`, keyed by its review action id (falls back to the project id).
pub fn snapshot_path(project: &Project) -> PathBuf {
    snapshot_path_in(&recovery_dir(), project)
}

pub fn snapshot_path_in(dir: &Path, project: &Project) -> PathBuf {
    let identifier = project
        .metadata
        .get(META_REVIEW_ACTION_ID)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&project.id);
    dir.join(format!("{}.json", sanitize_file_component(identifier)))
}

/// Writes `babel_json` (a workflow export) as the snapshot for `project`.
pub fn write_snapshot(project: &Project, babel_json: &str) -> Result<PathBuf> {
    write_snapshot_in(&recovery_dir(), project, babel_json)
}

pub fn write_snapshot_in(dir: &Path, project: &Project, babel_json: &str) -> Result<PathBuf> {
    let path = snapshot_path_in(dir, project);
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create recovery directory {}", dir.display()))?;
    fs::write(&path, babel_json)
        .with_context(|| format!("failed to write recovery draft {}", path.display()))?;
    Ok(path)
}

/// Removes the snapshot for `project`; missing files and IO errors are ignored.
pub fn remove_snapshot(project: &Project) {
    remove_snapshot_in(&recovery_dir(), project);
}

pub fn remove_snapshot_in(dir: &Path, project: &Project) {
    let _ = fs::remove_file(snapshot_path_in(dir, project));
}

/// Non-empty `*.json` snapshots, newest first.
pub fn discover_drafts() -> Vec<RecoveryDraft> {
    discover_drafts_in(&recovery_dir())
}

pub fn discover_drafts_in(dir: &Path) -> Vec<RecoveryDraft> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut drafts = entries
        .filter_map(Result::ok)
        .filter_map(|entry| draft_from_path(entry.path()))
        .collect::<Vec<_>>();
    drafts.sort_by_key(|draft| std::cmp::Reverse(draft.modified_millis));
    drafts
}

pub fn load_draft(draft: &RecoveryDraft) -> Result<StudioWorkflow> {
    let source = fs::read_to_string(&draft.path).context("failed to read recovery draft")?;
    StudioWorkflow::from_babel_json(&source)
}

fn draft_from_path(path: PathBuf) -> Option<RecoveryDraft> {
    if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
        return None;
    }
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 {
        return None;
    }
    let modified_millis = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let label = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Recovered task")
        .to_string();
    Some(RecoveryDraft {
        path,
        label,
        modified_millis,
    })
}

fn sanitize_file_component(value: &str) -> String {
    let mut output = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while output.contains("--") {
        output = output.replace("--", "-");
    }
    let output = output.trim_matches('-');
    if output.is_empty() {
        "task".to_string()
    } else {
        output.chars().take(96).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_file_component;

    #[test]
    fn sanitizer_never_emits_an_empty_or_nested_path() {
        assert_eq!(sanitize_file_component(" / \\ "), "task");
        assert_eq!(sanitize_file_component("a::b"), "a-b");
        assert!(sanitize_file_component(&"x".repeat(200)).len() <= 96);
    }
}
