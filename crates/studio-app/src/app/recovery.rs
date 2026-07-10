use std::{fs, path::PathBuf, time::UNIX_EPOCH};

use studio_core::Project;

use super::platform_paths::application_support_dir;
use super::service_callbacks::META_REVIEW_ACTION_ID;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecoveryDraft {
    pub(super) path: PathBuf,
    pub(super) label: String,
    modified_millis: u128,
}

pub(super) fn recovery_snapshot_path(project: &Project) -> PathBuf {
    let identifier = project
        .metadata
        .get(META_REVIEW_ACTION_ID)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&project.id);
    recovery_directory().join(format!("{}.json", sanitize_file_component(identifier)))
}

pub(super) fn discover_recovery_drafts() -> Vec<RecoveryDraft> {
    let directory = recovery_directory();
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut drafts = entries
        .filter_map(Result::ok)
        .filter_map(|entry| recovery_draft_from_path(entry.path()))
        .collect::<Vec<_>>();
    drafts.sort_by_key(|draft| std::cmp::Reverse(draft.modified_millis));
    drafts
}

pub(super) fn remove_recovery_snapshot(project: &Project) {
    let path = recovery_snapshot_path(project);
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}

fn recovery_directory() -> PathBuf {
    application_support_dir().join("recovery")
}

fn recovery_draft_from_path(path: PathBuf) -> Option<RecoveryDraft> {
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
    use std::{collections::BTreeMap, path::Path};

    use super::*;

    #[test]
    fn recovery_paths_are_stable_and_safe_for_review_action_ids() {
        let project = Project {
            id: "fallback/project".to_string(),
            name: "Task".to_string(),
            speakers: Vec::new(),
            lanes: Vec::new(),
            segments: Vec::new(),
            metadata: BTreeMap::from([(
                META_REVIEW_ACTION_ID.to_string(),
                "review/action:42".to_string(),
            )]),
        };

        let path = recovery_snapshot_path(&project);
        assert_eq!(path.file_name().unwrap(), "review-action-42.json");
        assert!(path.ends_with(Path::new("recovery").join("review-action-42.json")));
    }

    #[test]
    fn sanitizer_never_emits_an_empty_or_nested_path() {
        assert_eq!(sanitize_file_component(" / \\ "), "task");
        assert_eq!(sanitize_file_component("a::b"), "a-b");
        assert!(sanitize_file_component(&"x".repeat(200)).len() <= 96);
    }
}
