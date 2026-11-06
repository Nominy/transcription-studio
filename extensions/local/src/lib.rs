//! Bundled Local mode: opens user-selected audio without credentials or a task platform.

pub mod audio;

use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use sonus_extension::ExtensionManifest;
use sonus_shell::screens::ui::localized;
use sonus_shell::{
    ContributionRegistry, DocumentOrigin, ModeContribution, ProviderEnvironment, ShellHandle,
};

pub const ID: &str = "sonus.local";

pub fn manifest() -> Result<ExtensionManifest, serde_json::Error> {
    serde_json::from_str(include_str!("../manifest.json"))
}

pub fn register(cx: &mut App) -> anyhow::Result<()> {
    cx.update_global::<ContributionRegistry, _>(|registry, _| {
        registry.register_mode(
            ID,
            ModeContribution {
                id: ID.into(),
                label: "Local workspace".into(),
                description: "Open audio and edit projects without an account.".into(),
                home: Rc::new(|shell, _, cx| cx.new(|_| LocalHome::new(shell)).into()),
            },
        )
    })
}

struct LocalHome {
    shell: ShellHandle,
    busy: bool,
    progress: String,
    cancellation: Arc<AtomicBool>,
}

impl Drop for LocalHome {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
    }
}

enum ImportEvent {
    Progress {
        done: usize,
        total: usize,
        file: String,
    },
    Finished(Result<audio::ImportedAudioProject, String>),
}

impl LocalHome {
    fn new(shell: ShellHandle) -> Self {
        Self {
            shell,
            busy: false,
            progress: String::new(),
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }

    fn pick_audio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(localized(cx, "Open audio", "Открыть аудио").into()),
        });
        self.busy = true;
        self.cancellation = Arc::new(AtomicBool::new(false));
        let cancel = self.cancellation.clone();
        let view = cx.entity().downgrade();
        cx.notify();
        window
            .spawn(cx, async move |cx| {
                let paths = match picked.await {
                    Ok(Ok(Some(paths))) if !paths.is_empty() => paths,
                    _ => {
                        cx.update(|_, cx| {
                            view.update(cx, |this, cx| {
                                this.busy = false;
                                cx.notify();
                            })
                            .ok();
                        })
                        .ok();
                        return;
                    }
                };
                let (sender, receiver) = async_channel::bounded(8);
                cx.background_spawn(async move {
                    let result = audio::import_audio(&paths, |done, total, path| {
                        if cancel.load(Ordering::Acquire) {
                            return false;
                        }
                        sender
                            .send_blocking(ImportEvent::Progress {
                                done,
                                total,
                                file: path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                            })
                            .is_ok()
                    })
                    .map_err(|error| format!("{error:#}"));
                    let _ = sender.send_blocking(ImportEvent::Finished(result));
                })
                .detach();
                while let Ok(event) = receiver.recv().await {
                    let terminal = matches!(event, ImportEvent::Finished(_));
                    let delivered = cx
                        .update(|window, cx| {
                            view.update(cx, |this, cx| {
                                match event {
                                    ImportEvent::Progress { done, total, file } => {
                                        this.progress = format!("{done}/{total} — {file}");
                                    }
                                    ImportEvent::Finished(result) => {
                                        this.busy = false;
                                        this.progress.clear();
                                        if this.cancellation.load(Ordering::Acquire) {
                                            cx.notify();
                                            return;
                                        }
                                        match result {
                                            Ok(imported) => {
                                                let environment = ProviderEnvironment::local(
                                                    imported.project.id.clone(),
                                                    Some(imported.audio_base),
                                                );
                                                if let Err(error) =
                                                    this.shell.open_document_with_audio(
                                                        imported.project,
                                                        DocumentOrigin::local(None),
                                                        environment,
                                                        imported.audio,
                                                        window,
                                                        cx,
                                                    )
                                                {
                                                    window.push_notification(
                                                        Notification::error(format!("{error:#}")),
                                                        cx,
                                                    );
                                                }
                                            }
                                            Err(error) => window
                                                .push_notification(Notification::error(error), cx),
                                        }
                                    }
                                }
                                cx.notify();
                            })
                            .is_ok()
                        })
                        .unwrap_or(false);
                    if terminal || !delivered {
                        break;
                    }
                }
            })
            .detach();
    }
}

impl Render for LocalHome {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = self.shell.clone();
        let cancel = self.cancellation.clone();
        v_flex()
            .size_full()
            .p_8()
            .gap_4()
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .text_2xl()
                    .child(localized(cx, "Local workspace", "Локальные проекты")),
            )
            .child(localized(
                cx,
                "Open one or more audio files. Each file becomes a lane. No account needed.",
                "Откройте аудиофайлы: каждый файл станет дорожкой. Вход в аккаунт не нужен.",
            ))
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("local-audio")
                            .primary()
                            .label(localized(cx, "Open audio", "Открыть аудио"))
                            .disabled(self.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_audio(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("local-project")
                            .label(localized(cx, "Open project", "Открыть проект"))
                            .disabled(self.busy)
                            .on_click(move |_, window, cx| {
                                sonus_shell::screens::local_files::open_project(
                                    shell.clone(),
                                    window,
                                    cx,
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .child("WAV · MP3 · FLAC · OGG · M4A · MP4 · WebM · Matroska"),
            )
            .children(self.busy.then(|| {
                h_flex().gap_3().child(self.progress.clone()).child(
                    Button::new("cancel-import")
                        .label(localized(cx, "Cancel", "Отмена"))
                        .on_click(move |_, _, _| {
                            cancel.store(true, Ordering::Release);
                        }),
                )
            }))
    }
}
