use std::{collections::BTreeMap, path::PathBuf};

use exaudio_babel_domain::workflow::BabelWorkflowExt;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::screens::ui;
use exaudio_shell::{DocumentOrigin, ModeLaunchContext, ProviderEnvironment, ShellHandle};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use crate::FILE_ID;
use crate::text::{failed, localized, localized_fmt};

/// `ModeLaunch::Direct`: the file picker opens immediately and the document, when chosen,
/// becomes the editor. There is no launch window; cancelling leaves the shell on Start.
/// The offline mode never reads account credentials or requests platform data.
pub fn launch(
    shell: ShellHandle,
    _context: ModeLaunchContext,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyView> {
    open_document(shell, window, cx);
    None
}

pub fn open_document(shell: ShellHandle, window: &mut Window, cx: &mut App) {
    open_babel_json(window, cx, move |result, window, cx| {
        let opened = result.map_err(anyhow::Error::msg).and_then(|file| {
            let document = exaudio_babel_domain::document_environment(
                &file.project,
                file.path.to_string_lossy().into_owned(),
                FILE_ID,
            )?;
            let origin = DocumentOrigin {
                mode_id: FILE_ID.to_string(),
                instance: None,
                read_only: document.policy.read_only,
                native_path: Some(file.path.clone()),
            };
            let audio_sources =
                crate::services::local_audio_sources(&file.project, Some(&file.audio_base_dir));
            let environment = ProviderEnvironment {
                audio_base_dir: Some(file.audio_base_dir),
                audio_sources,
                document,
                values: BTreeMap::new(),
            };
            let model = shell.open_document(file.project, origin, environment, window, cx)?;
            crate::services::set_file_document(&model, cx);
            let name = file_name(&file.path);
            model.update(cx, |model, cx| {
                model.set_status(
                    localized_fmt(cx, "Opened {name}", "Открыт {name}", &[("name", &name)]),
                    cx,
                )
            });
            Ok(())
        });
        if let Err(error) = opened {
            window.push_notification(
                ui::error_notification(
                    localized(cx, "Couldn't open the file.", "Не удалось открыть файл."),
                    Some(format!("{error:#}").into()),
                ),
                cx,
            );
        }
    });
}

pub struct BabelFile {
    pub project: studio_core::Project,
    pub path: PathBuf,
    pub audio_base_dir: PathBuf,
}

/// An explicit file import never creates a claimed remote task.
pub fn open_babel_json(
    window: &mut Window,
    cx: &mut App,
    on_loaded: impl FnOnce(Result<BabelFile, String>, &mut Window, &mut App) + 'static,
) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(localized(cx, "Open a Babel file", "Открыть файл Babel").into()),
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
            let loaded = cx
                .background_spawn(async move {
                    let text = std::fs::read_to_string(&path)
                        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
                    let workflow = studio_editor::workflow::StudioWorkflow::from_babel_json(&text)
                        .map_err(|error| format!("{error:#}"))?;
                    let audio_base_dir = path
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| PathBuf::from("."));
                    Ok(BabelFile {
                        project: workflow.into_project(),
                        path,
                        audio_base_dir,
                    })
                })
                .await;
            cx.update(|window, cx| on_loaded(loaded, window, cx)).ok();
        })
        .detach();
}

/// Commits pending row drafts and exports the task through Babel's codec.
pub fn export_babel_json(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App) {
    pick_save_path(model, false, window, cx, None);
}

pub type Saved = Box<dyn FnOnce(&mut Window, &mut App)>;

/// File-provider saves never contact Babel or the relay.
pub fn save_babel_file(
    model: &Entity<EditorModel>,
    window: &mut Window,
    cx: &mut App,
    done: Option<Saved>,
) {
    if model.read(cx).doc().is_read_only() {
        window.push_notification(
            Notification::error(localized(
                cx,
                "This file is read-only.",
                "Этот файл только для чтения.",
            )),
            cx,
        );
        return;
    }
    let path = model.read(cx).origin().native_path.clone();
    match path {
        Some(path) => save_to(model, path, true, window, cx, done),
        None => pick_save_path(model, true, window, cx, done),
    }
}

fn pick_save_path(
    model: &Entity<EditorModel>,
    adopt_path: bool,
    window: &mut Window,
    cx: &mut App,
    done: Option<Saved>,
) {
    let name = format!("{}.json", file_stem(model, cx));
    let picked = cx.prompt_for_new_path(&start_dir(model, cx), Some(&name));
    let model = model.clone();
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(path))) = picked.await {
                cx.update(|window, cx| save_to(&model, path, adopt_path, window, cx, done))
                    .ok();
            }
        })
        .detach();
}

fn save_to(
    model: &Entity<EditorModel>,
    path: PathBuf,
    adopt_path: bool,
    window: &mut Window,
    cx: &mut App,
    done: Option<Saved>,
) {
    let snapshot = model.update(cx, |model, cx| {
        if model.doc().session().is_saving {
            model.set_status(localized(cx, "Already saving…", "Сохранение уже идёт…"), cx);
            return None;
        }
        let action = localized(cx, "Save", "Сохранить");
        let committed = model.update_doc(cx, |doc| doc.commit_pending_drafts(action));
        let ok = committed.is_ok();
        model.apply_result(committed, cx);
        if !ok {
            return None;
        }
        let json = match model.doc().workflow().export_babel_json() {
            Ok(json) => json,
            Err(error) => {
                model.set_status(
                    failed(
                        cx,
                        "Couldn't save the file.",
                        "Не удалось сохранить файл.",
                        format!("{error:#}"),
                    ),
                    cx,
                );
                return None;
            }
        };
        let revision = model.revision();
        model.update_doc(cx, |doc| doc.session_mut().begin_save());
        Some((json, revision))
    });
    let Some((json, revision)) = snapshot else {
        return;
    };
    let target = path.clone();
    let model = model.downgrade();
    window
        .spawn(cx, async move |cx| {
            let written = cx
                .background_spawn(async move { std::fs::write(&target, json) })
                .await;
            cx.update(|window, cx| {
                let succeeded = written.is_ok();
                let detail = match written {
                    Ok(()) => {
                        let name = file_name(&path);
                        localized_fmt(cx, "Saved {name}", "Сохранён {name}", &[("name", &name)])
                    }
                    Err(error) => failed(
                        cx,
                        "Couldn't save the file.",
                        "Не удалось сохранить файл.",
                        error,
                    ),
                };
                let current = model
                    .update(cx, |model, cx| {
                        let accepted = model.update_doc(cx, |doc| {
                            doc.session_mut().complete_save_for_revision(
                                revision,
                                succeeded,
                                detail.clone(),
                            )
                        });
                        if succeeded && adopt_path {
                            model.set_native_path(path);
                        }
                        model.set_status(detail.clone(), cx);
                        accepted
                            && model.revision() == revision
                            && !model.doc().has_pending_drafts()
                    })
                    .unwrap_or(false);
                if succeeded {
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
    let file = match model.read(cx).origin().native_path.as_deref() {
        Some(path) => file_name(path),
        None => format!("{}.json", file_stem(model, cx)),
    };
    let question = localized_fmt(
        cx,
        "Save changes to “{file}”?",
        "Сохранить изменения в «{file}»?",
        &[("file", &file)],
    );
    let prompt = window.prompt(
        PromptLevel::Warning,
        localized(cx, "Unsaved changes", "Несохранённые изменения"),
        Some(&question),
        &[
            localized(cx, "Save", "Сохранить"),
            localized(cx, "Don't save", "Не сохранять"),
            localized(cx, "Cancel", "Отмена"),
        ],
        cx,
    );
    let model = model.clone();
    window
        .spawn(cx, async move |cx| match prompt.await {
            Ok(0) => {
                cx.update(|window, cx| save_babel_file(&model, window, cx, Some(Box::new(done))))
                    .ok();
            }
            Ok(1) => {
                cx.update(|window, cx| done(window, cx)).ok();
            }
            _ => {}
        })
        .detach();
}

/// The file's own name; paths never appear in a sentence.
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn file_stem(model: &Entity<EditorModel>, cx: &App) -> String {
    let project = model.read(cx).doc().project();
    let raw = if project.name.trim().is_empty() {
        &project.id
    } else {
        &project.name
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
