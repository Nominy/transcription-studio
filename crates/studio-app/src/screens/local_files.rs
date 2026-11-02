//! Local task files: open a Babel JSON export or a Studio project folder, and export the open
//! task to either format. Paths come from the platform file pickers.

use std::path::{Path, PathBuf};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use studio_editor::workflow::StudioWorkflow;

use crate::editor::model::EditorModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalSource {
    BabelJson,
    ProjectFolder,
}

/// A local task read from disk; lane audio resolves against `audio_base_dir`.
pub struct LocalTask {
    pub workflow: StudioWorkflow,
    pub audio_base_dir: PathBuf,
    pub path: PathBuf,
}

/// Prompts for a Babel JSON file or project folder and loads it off the UI thread.
/// `on_loaded` is not called when the picker is cancelled.
pub fn open(
    source: LocalSource,
    window: &mut Window,
    cx: &mut App,
    on_loaded: impl FnOnce(Result<LocalTask, String>, &mut Window, &mut App) + 'static,
) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: source == LocalSource::BabelJson,
        directories: source == LocalSource::ProjectFolder,
        multiple: false,
        prompt: Some("Open".into()),
    });
    window
        .spawn(cx, async move |cx| {
            let path = match picked.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    cx.update(|window, cx| {
                        on_loaded(Err(format!("File picker failed: {error:#}")), window, cx)
                    })
                    .ok();
                    return;
                }
            };
            let loaded = cx.background_spawn(async move { load(source, path) }).await;
            cx.update(|window, cx| on_loaded(loaded, window, cx)).ok();
        })
        .detach();
}

fn load(source: LocalSource, path: PathBuf) -> Result<LocalTask, String> {
    let (workflow, audio_base_dir) = match source {
        LocalSource::BabelJson => {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
            let workflow =
                StudioWorkflow::from_babel_json(&text).map_err(|error| format!("{error:#}"))?;
            let base = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            (workflow, base)
        }
        LocalSource::ProjectFolder => {
            let workflow =
                StudioWorkflow::load_local_project(&path).map_err(|error| format!("{error:#}"))?;
            (workflow, path.clone())
        }
    };
    Ok(LocalTask {
        workflow,
        audio_base_dir,
        path,
    })
}

/// Saves the open task as Babel JSON at a path picked by the user.
pub fn export_babel_json(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App) {
    let Some(json) = exported(model, "Export", cx, |workflow| {
        workflow
            .export_babel_json()
            .map_err(|error| format!("{error:#}"))
    }) else {
        return;
    };
    let name = format!("{}.json", file_stem(model, cx));
    let picked = cx.prompt_for_new_path(&start_dir(model, cx), Some(&name));
    let model = model.downgrade();
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let target = path.clone();
            let written = cx
                .background_spawn(async move { std::fs::write(&target, json) })
                .await;
            cx.update(|window, cx| match written {
                Ok(()) => {
                    model
                        .update(cx, |model, cx| {
                            model.update_doc(cx, |doc| doc.session_mut().mark_saved());
                            model.set_status(
                                format!("Exported Babel JSON to {}.", path.display()),
                                cx,
                            );
                        })
                        .ok();
                }
                Err(error) => window
                    .push_notification(Notification::error(format!("Export failed: {error}")), cx),
            })
            .ok();
        })
        .detach();
}

/// Saves the open task as a Studio project folder picked by the user.
pub fn export_project_folder(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App) {
    if exported(model, "Export", cx, |_| Ok(())).is_none() {
        return;
    }
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Export here".into()),
    });
    let model = model.downgrade();
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            cx.update(|window, cx| {
                let saved = model.update(cx, |model, cx| {
                    let saved = model.doc().workflow().save_local_project(&dir);
                    if saved.is_ok() {
                        model.set_status(
                            format!("Exported project folder to {}.", dir.display()),
                            cx,
                        );
                    }
                    saved
                });
                if let Ok(Err(error)) = saved {
                    window.push_notification(
                        Notification::error(format!("Export failed: {error:#}")),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
}

/// Commits pending row drafts (one undo step) and extracts the export payload. A failed commit
/// is reported in the editor status and aborts the export.
fn exported<T>(
    model: &Entity<EditorModel>,
    action: &'static str,
    cx: &mut App,
    extract: impl FnOnce(&StudioWorkflow) -> Result<T, String>,
) -> Option<T> {
    model.update(cx, |model, cx| {
        let committed = model.update_doc(cx, |doc| doc.commit_pending_drafts(action));
        let ok = committed.is_ok();
        model.apply_result(committed, cx);
        if !ok {
            return None;
        }
        match extract(model.doc().workflow()) {
            Ok(value) => Some(value),
            Err(error) => {
                model.set_status(format!("{action} failed: {error}"), cx);
                None
            }
        }
    })
}

fn file_stem(model: &Entity<EditorModel>, cx: &App) -> String {
    let project = model.read(cx).doc().project();
    let raw = if project.name.trim().is_empty() {
        project.id.as_str()
    } else {
        project.name.as_str()
    };
    let stem: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if stem.trim_matches('-').is_empty() {
        "babel-task".to_string()
    } else {
        stem
    }
}

fn start_dir(model: &Entity<EditorModel>, cx: &App) -> PathBuf {
    model
        .read(cx)
        .audio_base_dir()
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .cloned()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}
