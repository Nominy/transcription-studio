use super::*;
use crate::screens::ui::error_notification;
use std::{
    io::Read,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
pub(super) static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
struct ImportPending {
    provider: String,
    reply: async_channel::Sender<w::ImportResult>,
}
struct ExportPending {
    provider: String,
    reply: async_channel::Sender<w::ExportResult>,
}
#[derive(Default)]
struct FileRequests {
    imports: BTreeMap<(String, u64), ImportPending>,
    exports: BTreeMap<(String, u64), ExportPending>,
}
impl Global for FileRequests {}
fn init(cx: &mut App) {
    if !cx.has_global::<FileRequests>() {
        cx.set_global(FileRequests::default());
    }
}
async fn response_until<T>(
    receive: &async_channel::Receiver<T>,
    deadline: impl Future<Output = ()>,
) -> Option<T> {
    futures_lite::future::or(async { receive.recv().await.ok() }, async {
        deadline.await;
        None
    })
    .await
}
pub(super) fn register(owner: &str, registration: &w::Registration, cx: &mut App) -> Result<()> {
    let owner = owner.to_owned();
    let provider = registration.id.clone();
    let importer = matches!(registration.kind, w::ContributionKind::Importer);
    let key = namespaced(&owner, &provider);
    let title = registration.title.clone();
    let registry_owner = owner.clone();
    cx.update_global::<ContributionRegistry, _>(|registry, _| {
        registry.register_command(
            &registry_owner,
            CommandContribution {
                id: key,
                label: title,
                shortcut: None,
                toolbar: false,
                menu: true,
                run: Rc::new(move |shell, window, cx| {
                    if !crate::wasm_host::has_permission(
                        &owner,
                        exaudio_extension::Permission::FilePicker,
                        cx,
                    ) {
                        return;
                    }
                    if importer {
                        import(owner.clone(), provider.clone(), shell, window, cx);
                    } else {
                        export(owner.clone(), provider.clone(), shell, window, cx);
                    }
                }),
            },
        )
    })
}
fn import(owner: String, provider: String, shell: ShellHandle, window: &mut Window, cx: &mut App) {
    init(cx);
    if cx.global::<FileRequests>().imports.len() >= 32 {
        use gpui_kit::component::WindowExt as _;
        window.push_notification(
            localized(
                cx,
                "Wait for the current import to finish.",
                "Дождитесь завершения текущего импорта.",
            ),
            cx,
        );
        return;
    }
    let too_large = localized(
        cx,
        "This file is too large to import (over 16 MB).",
        "Файл слишком большой для импорта (больше 16 МБ).",
    );
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(localized(cx, "Import", "Импорт").into()),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let bytes = cx
                .background_spawn(async move {
                    let file = std::fs::File::open(path)?;
                    let mut bytes = Vec::new();
                    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                    anyhow::ensure!(bytes.len() <= 16 * 1024 * 1024, "{too_large}");
                    Ok::<_, anyhow::Error>(bytes)
                })
                .await;
            let bytes = match bytes {
                Ok(bytes) => bytes,
                Err(error) => {
                    let _ = cx.update(|window, cx| {
                        use gpui_kit::component::WindowExt as _;
                        window.push_notification(
                            error_notification(
                                localized(cx, "Couldn't import.", "Не удалось импортировать."),
                                Some(error.to_string().into()),
                            ),
                            cx,
                        );
                    });
                    return;
                }
            };
            let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
            let (reply, receive) = async_channel::bounded(1);
            if cx
                .update(|_, cx| {
                    cx.update_global::<FileRequests, _>(|state, _| {
                        state.imports.insert(
                            (owner.clone(), request),
                            ImportPending {
                                provider: provider.clone(),
                                reply,
                            },
                        );
                    });
                    crate::wasm_host::invoke(
                        &owner,
                        Invocation::Event(w::Event::ImportRequest(w::ImportRequest {
                            provider: provider.clone(),
                            request_id: request,
                            name,
                            media_type: "application/octet-stream".into(),
                            bytes,
                        })),
                        cx,
                    );
                })
                .is_err()
            {
                return;
            }
            let timer = cx.background_executor().timer(Duration::from_secs(5));
            let result = response_until(&receive, timer).await;
            if let Some(value) = result {
                let _ = cx.update(|window, cx| {
                    let result =
                        crate::wasm_host::document::project(&value.document).and_then(|project| {
                            let environment =
                                crate::ProviderEnvironment::local(project.id.clone(), None);
                            shell
                                .open_document(
                                    project,
                                    crate::DocumentOrigin::local(None),
                                    environment,
                                    window,
                                    cx,
                                )
                                .map(|_| ())
                        });
                    if let Err(error) = result {
                        use gpui_kit::component::WindowExt as _;
                        window.push_notification(
                            error_notification(
                                localized(cx, "Couldn't import.", "Не удалось импортировать."),
                                Some(error.to_string().into()),
                            ),
                            cx,
                        );
                    }
                });
            } else {
                let _ = cx.update(|window, cx| {
                    cx.update_global::<FileRequests, _>(|state, _| {
                        state.imports.remove(&(owner.clone(), request));
                    });
                    crate::wasm_host::invoke(
                        &owner,
                        Invocation::Event(w::Event::Cancelled(request)),
                        cx,
                    );
                    use gpui_kit::component::WindowExt as _;
                    window.push_notification(
                        error_notification(
                            localized(cx, "The import didn't finish.", "Импорт не завершился."),
                            None,
                        ),
                        cx,
                    );
                });
            }
        })
        .detach();
}
fn export(owner: String, provider: String, shell: ShellHandle, window: &mut Window, cx: &mut App) {
    init(cx);
    if cx.global::<FileRequests>().exports.len() >= 32 {
        use gpui_kit::component::WindowExt as _;
        window.push_notification(
            localized(
                cx,
                "Wait for the current export to finish.",
                "Дождитесь завершения текущего экспорта.",
            ),
            cx,
        );
        return;
    }
    let Some(model) = shell.editor_model(cx) else {
        return;
    };
    if !crate::wasm_host::document_associated(&owner, &model, cx) {
        use gpui_kit::component::WindowExt as _;
        window.push_notification(
            error_notification(
                localized(
                    cx,
                    "This extension can't export this project.",
                    "Расширение не может экспортировать этот проект.",
                ),
                None,
            ),
            cx,
        );
        return;
    }
    let document_id = crate::wasm_host::document_handle(&model);
    let revision = model.read(cx).doc().session().current_edit_revision();
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let (reply, receive) = async_channel::bounded(1);
    cx.update_global::<FileRequests, _>(|state, _| {
        state.exports.insert(
            (owner.clone(), request),
            ExportPending {
                provider: provider.clone(),
                reply,
            },
        );
    });
    crate::wasm_host::invoke(
        &owner,
        Invocation::Event(w::Event::ExportRequest(w::ExportRequest {
            provider,
            request_id: request,
            document_id,
            revision,
            media_type: "application/octet-stream".into(),
        })),
        cx,
    );
    window
        .spawn(cx, async move |cx| {
            let timer = cx.background_executor().timer(Duration::from_secs(5));
            let value = response_until(&receive, timer).await;
            let Some(value) = value else {
                let _ = cx.update(|window, cx| {
                    cx.update_global::<FileRequests, _>(|state, _| {
                        state.exports.remove(&(owner.clone(), request));
                    });
                    crate::wasm_host::invoke(
                        &owner,
                        Invocation::Event(w::Event::Cancelled(request)),
                        cx,
                    );
                    use gpui_kit::component::WindowExt as _;
                    window.push_notification(
                        error_notification(
                            localized(cx, "The export didn't finish.", "Экспорт не завершился."),
                            None,
                        ),
                        cx,
                    );
                });
                return;
            };
            let picked = match cx.update(|_, cx| {
                cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some(localized(cx, "Choose a folder", "Выберите папку").into()),
                })
            }) {
                Ok(picked) => picked,
                Err(_) => return,
            };
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(directory) = paths.into_iter().next() else {
                return;
            };
            // A provider chooses a basename, never a path. Never overwrite an existing file.
            let name = std::path::Path::new(&value.name);
            if name.components().count() != 1
                || !matches!(
                    name.components().next(),
                    Some(std::path::Component::Normal(_))
                )
            {
                return;
            }
            let path = directory.join(name);
            let Some(parent) = path.parent().map(std::path::Path::to_path_buf) else {
                let _ = cx.update(|window, cx| {
                    use gpui_kit::component::WindowExt as _;
                    window.push_notification(
                        localized(
                            cx,
                            "Choose a folder to export to.",
                            "Выберите папку для экспорта.",
                        ),
                        cx,
                    );
                });
                return;
            };
            let result = cx
                .background_spawn(async move {
                    use std::io::Write;
                    let mut file = tempfile::NamedTempFile::new_in(parent)?;
                    file.write_all(&value.bytes)?;
                    file.as_file().sync_all()?;
                    file.persist_noclobber(path).map_err(|error| error.error)?;
                    Ok::<_, std::io::Error>(())
                })
                .await;
            let _ = cx.update(|window, cx| {
                use gpui_kit::component::WindowExt as _;
                match result {
                    Ok(()) => {
                        window.push_notification(localized(cx, "Exported", "Экспортировано"), cx)
                    }
                    Err(error) => window.push_notification(
                        error_notification(
                            localized(cx, "Couldn't export.", "Не удалось экспортировать."),
                            Some(error.to_string().into()),
                        ),
                        cx,
                    ),
                }
            });
        })
        .detach();
}
pub(super) fn accept_import(owner: &str, value: w::ImportResult, cx: &mut App) -> Result<()> {
    init(cx);
    crate::wasm_host::document::project(&value.document)?;
    let pending = cx
        .update_global::<FileRequests, _>(|state, _| {
            state.imports.remove(&(owner.to_owned(), value.request_id))
        })
        .ok_or_else(|| anyhow!("unsolicited import result"))?;
    if pending.provider != value.provider {
        bail!("import provider mismatch");
    }
    pending
        .reply
        .try_send(value)
        .map_err(|_| anyhow!("import consumer closed"))
}
pub(super) fn accept_export(owner: &str, value: w::ExportResult, cx: &mut App) -> Result<()> {
    init(cx);
    let pending = cx
        .update_global::<FileRequests, _>(|state, _| {
            state.exports.remove(&(owner.to_owned(), value.request_id))
        })
        .ok_or_else(|| anyhow!("unsolicited export result"))?;
    if pending.provider != value.provider {
        bail!("export provider mismatch");
    }
    pending
        .reply
        .try_send(value)
        .map_err(|_| anyhow!("export consumer closed"))
}
pub(super) fn remove(owner: &str, cx: &mut App) {
    if cx.has_global::<FileRequests>() {
        cx.update_global::<FileRequests, _>(|state, _| {
            state.imports.retain(|(id, _), _| id != owner);
            state.exports.retain(|(id, _), _| id != owner);
        });
    }
}
pub(super) fn request_failed(owner: &str, request: u64, cx: &mut App) {
    if cx.has_global::<FileRequests>() {
        cx.update_global::<FileRequests, _>(|state, _| {
            state.imports.remove(&(owner.to_owned(), request));
            state.exports.remove(&(owner.to_owned(), request));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[test]
    fn silent_guest_response_expires_without_dropping_sender() {
        let (_sender, receiver) = async_channel::bounded::<w::ImportResult>(1);
        assert!(
            futures_lite::future::block_on(response_until(&receiver, std::future::ready(())))
                .is_none()
        );
    }
    #[test]
    fn completed_response_wins_without_waiting_for_deadline() {
        let (sender, receiver) = async_channel::bounded(1);
        sender.try_send(7u32).unwrap();
        assert_eq!(
            futures_lite::future::block_on(response_until(&receiver, std::future::pending())),
            Some(7)
        );
    }
    #[test]
    fn unload_closes_waiting_response() {
        let (sender, receiver) = async_channel::bounded::<u32>(1);
        drop(sender);
        assert!(
            futures_lite::future::block_on(response_until(&receiver, std::future::pending()))
                .is_none()
        );
    }
}
