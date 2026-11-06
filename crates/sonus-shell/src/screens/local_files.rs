//! Native project I/O is independent of all account and extension sessions.
use crate::{
    DocumentOrigin, ProviderEnvironment, ShellHandle, editor::model::EditorModel,
    services::AppServices,
};
use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::*;
use std::path::PathBuf;
pub type Saved = Box<dyn FnOnce(&mut Window, &mut App)>;
pub fn open_project(shell: ShellHandle, window: &mut Window, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open project".into()),
    });
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                cx.update(|window, cx| open_path(shell, path, window, cx))
                    .ok();
            }
        })
        .detach();
}
pub fn open_path(shell: ShellHandle, path: PathBuf, window: &mut Window, cx: &mut App) {
    let target = path.clone();
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(async move { studio_project::load_project(&target) })
                .await;
            cx.update(|window, cx| match result {
                Ok(project) => {
                    let environment =
                        ProviderEnvironment::local(project.id.clone(), Some(path.clone()));
                    match shell.open_document(
                        project,
                        DocumentOrigin::local(Some(path.clone())),
                        environment,
                        window,
                        cx,
                    ) {
                        Ok(_) => remember(&path, cx),
                        Err(e) => window.push_notification(Notification::error(e.to_string()), cx),
                    }
                }
                Err(error) => window
                    .push_notification(Notification::error(format!("Open failed: {error}")), cx),
            })
            .ok();
        })
        .detach();
}
pub fn recent(cx: &App) -> Vec<PathBuf> {
    cx.global::<AppServices>()
        .storage
        .value("sonus.recent-projects")
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_default()
}
fn remember(path: &PathBuf, cx: &mut App) {
    let mut paths = recent(cx);
    paths.retain(|p| p != path);
    paths.insert(0, path.clone());
    paths.truncate(12);
    if let Ok(value) = serde_json::to_string(&paths) {
        let _ = cx
            .global_mut::<AppServices>()
            .storage
            .set_value("sonus.recent-projects", value);
    }
}
pub fn save(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App, done: Option<Saved>) {
    if model.read(cx).doc().is_read_only() {
        window.push_notification(Notification::error("This document is read-only."), cx);
        return;
    }
    let mode_id = model.read(cx).origin().mode_id.clone();
    let handler = cx
        .global::<crate::ContributionRegistry>()
        .save_handler(&mode_id);
    if let Some(handler) = handler {
        handler(model.clone(), window, cx, done);
        return;
    }
    if model.read(cx).origin().instance.is_some() {
        window.push_notification(
            Notification::error(format!("Save handler unavailable for {mode_id}")),
            cx,
        );
        return;
    }
    if let Some(path) = model.read(cx).origin().native_path.clone() {
        save_to(model, path, window, cx, done);
        return;
    }
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Save native project here".into()),
    });
    let model = model.clone();
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                cx.update(|window, cx| save_to(&model, path, window, cx, done))
                    .ok();
            }
        })
        .detach();
}
fn save_to(
    model: &Entity<EditorModel>,
    path: PathBuf,
    window: &mut Window,
    cx: &mut App,
    done: Option<Saved>,
) {
    let snapshot = model.update(cx, |model, cx| {
        let result = model.update_doc(cx, |doc| doc.commit_pending_drafts("Save"));
        let ok = result.is_ok();
        model.apply_result(result, cx);
        if !ok {
            return None;
        }
        let revision = model.revision();
        let project = model.doc().project().clone();
        let base = model
            .audio_base_dir()
            .cloned()
            .unwrap_or_else(|| path.clone());
        model.update_doc(cx, |doc| doc.session_mut().begin_save());
        Some((project, revision, base))
    });
    let Some((project, revision, base)) = snapshot else {
        return;
    };
    let target = path.clone();
    let model = model.downgrade();
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(
                    async move { studio_project::save_project(&project, &target, &base) },
                )
                .await;
            cx.update(|window, cx| {
                let succeeded = result.is_ok();
                let detail = result
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "Saved".into());
                let current = model
                    .update(cx, |model, cx| {
                        model.update_doc(cx, |doc| {
                            doc.session_mut().complete_save_for_revision(
                                revision,
                                succeeded,
                                detail.clone(),
                            );
                        });
                        if succeeded {
                            model.set_native_path(path.clone());
                        }
                        model.set_status(detail.clone(), cx);
                        model.revision() == revision && !model.doc().has_pending_drafts()
                    })
                    .unwrap_or(false);
                if succeeded {
                    remember(&path, cx);
                    if current && let Some(done) = done {
                        done(window, cx);
                    }
                } else {
                    window.push_notification(Notification::error(detail), cx);
                }
            })
            .ok();
        })
        .detach();
}
pub fn confirm_leave(
    model: &Entity<EditorModel>,
    window: &mut Window,
    cx: &mut App,
    done: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    if !model.read(cx).doc().session().is_dirty && !model.read(cx).doc().has_pending_drafts() {
        window.defer(cx, done);
        return;
    }
    let prompt = window.prompt(
        PromptLevel::Warning,
        "Unsaved changes",
        Some("Save this document before leaving?"),
        &["Save", "Discard", "Cancel"],
        cx,
    );
    let model = model.clone();
    window
        .spawn(cx, async move |cx| match prompt.await {
            Ok(0) => {
                cx.update(|window, cx| save(&model, window, cx, Some(Box::new(done))))
                    .ok();
            }
            Ok(1) => {
                cx.update(|window, cx| done(window, cx)).ok();
            }
            _ => {}
        })
        .detach();
}

/// Writes a portable native copy without changing the document's origin or remote save state.
pub fn export_project_folder(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App) {
    let snapshot = match model.read(cx).doc().recovery_project() {
        Ok(project) => project,
        Err(error) => {
            window.push_notification(Notification::error(error.to_string()), cx);
            return;
        }
    };
    let base = model
        .read(cx)
        .audio_base_dir()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Save native copy here".into()),
    });
    let model = model.downgrade();
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let target = path.clone();
            let result = cx
                .background_spawn(
                    async move { studio_project::save_project(&snapshot, &target, &base) },
                )
                .await;
            cx.update(|window, cx| match result {
                Ok(()) => {
                    remember(&path, cx);
                    model
                        .update(cx, |model, cx| {
                            model.set_status(format!("Saved native copy to {}", path.display()), cx)
                        })
                        .ok();
                }
                Err(error) => window.push_notification(
                    Notification::error(format!("Native copy failed: {error}")),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}
