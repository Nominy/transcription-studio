//! Bundled Local mode: opens audio and native projects on this computer. Its launch window
//! is the Start-screen content for local work: open audio, open or restore a project, demo.

pub mod audio;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use sonus_extension::ExtensionManifest;
use sonus_shell::screens::ui::{self, localized};
use sonus_shell::screens::{local_files, recovery};
use sonus_shell::{
    ContributionRegistry, DocumentOrigin, ModeContribution, ProviderEnvironment, ShellHandle,
};

pub const ID: &str = "sonus.local";

pub fn manifest() -> Result<ExtensionManifest, serde_json::Error> {
    serde_json::from_str(include_str!("../manifest.json"))
}

pub fn register(cx: &mut App) -> anyhow::Result<()> {
    let label = localized(cx, "Local", "Локально");
    let description = localized(
        cx,
        "Open audio and projects on this computer",
        "Аудио и проекты на этом компьютере",
    );
    cx.update_global::<ContributionRegistry, _>(|registry, _| {
        registry.register_mode(
            ID,
            ModeContribution::with_window(ID, label, description, |shell, _, _, cx| {
                cx.new(|cx| LocalHome::new(shell, cx)).into()
            }),
        )
    })
}

/// Display name of a native project folder or recovery root.
fn project_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

struct LocalHome {
    shell: ShellHandle,
    busy: bool,
    progress: Option<(usize, usize, String)>,
    cancellation: Arc<AtomicBool>,
    /// Recovery snapshot roots with their project names, loaded off the UI thread.
    unsaved: Vec<(PathBuf, String)>,
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
    fn new(shell: ShellHandle, cx: &mut Context<Self>) -> Self {
        let roots = recovery::entries(cx);
        if !roots.is_empty() {
            cx.spawn(async move |view, cx| {
                let unsaved = cx
                    .background_spawn(async move {
                        roots
                            .into_iter()
                            .map(|root| {
                                let name = studio_project::load_recovery(&root)
                                    .map(|project| project.name)
                                    .unwrap_or_else(|_| project_name(&root));
                                (root, name)
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                view.update(cx, |this, cx| {
                    this.unsaved = unsaved;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        Self {
            shell,
            busy: false,
            progress: None,
            cancellation: Arc::new(AtomicBool::new(false)),
            unsaved: Vec::new(),
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
        let strings = if localized(cx, "en", "ru") == "en" {
            audio::ImportStrings::EN
        } else {
            audio::ImportStrings::RU
        };
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
                    let result = audio::import_audio(&paths, strings, |done, total, path| {
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
                                        this.progress = Some((done, total, file));
                                    }
                                    ImportEvent::Finished(result) => {
                                        this.busy = false;
                                        this.progress = None;
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
        let open_project = self.shell.clone();
        let demo = self.shell.clone();
        let cancel = self.cancellation.clone();
        let recent = local_files::recent(cx);
        v_flex()
            .size_full()
            .id("sonus-local")
            .overflow_y_scroll()
            .p_8()
            .gap_4()
            .text_color(cx.theme().foreground)
            .child(
                ui::card(
                    localized(cx, "Start", "Начать"),
                    Some(localized(
                        cx,
                        "Open a recording and start editing.",
                        "Откройте запись и начните редактировать.",
                    )),
                    cx,
                )
                .child(
                    h_flex()
                        .gap_3()
                        .flex_wrap()
                        .child(
                            Button::new("local-audio")
                                .primary()
                                .label(localized(cx, "Open audio…", "Открыть аудио…"))
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.pick_audio(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("local-project")
                                .label(localized(cx, "Open project…", "Открыть проект…"))
                                .disabled(self.busy)
                                .on_click(move |_, window, cx| {
                                    local_files::open_project(open_project.clone(), window, cx)
                                }),
                        )
                        .child(
                            Button::new("local-demo")
                                .label(localized(cx, "Try the demo", "Попробовать демо"))
                                .disabled(self.busy)
                                .on_click(move |_, window, cx| {
                                    if let Err(error) =
                                        sonus_shell::root::open_demo(&demo, window, cx)
                                    {
                                        window.push_notification(
                                            Notification::error(error.to_string()),
                                            cx,
                                        );
                                    }
                                }),
                        ),
                )
                .child(ui::muted(
                    "WAV · MP3 · FLAC · OGG · M4A · MP4 · WebM · MKV",
                    cx,
                ))
                .children(self.busy.then(|| {
                    let progress = self.progress.as_ref().map(|(done, total, file)| {
                        localized(
                            cx,
                            "Opening {done} of {total}: {file}",
                            "Открытие {done} из {total}: {file}",
                        )
                        .replace("{done}", &done.to_string())
                        .replace("{total}", &total.to_string())
                        .replace("{file}", file)
                    });
                    h_flex().gap_3().items_center().children(progress).child(
                        Button::new("cancel-import")
                            .small()
                            .label(localized(cx, "Cancel", "Отмена"))
                            .on_click(move |_, _, _| {
                                cancel.store(true, Ordering::Release);
                            }),
                    )
                })),
            )
            .children((!recent.is_empty()).then(|| {
                ui::card(localized(cx, "Recent", "Недавние"), None, cx).children(
                    recent.into_iter().enumerate().map(|(i, path)| {
                        let shell = self.shell.clone();
                        let target = path.clone();
                        Button::new(("recent", i))
                            .label(project_name(&path))
                            .tooltip(path.display().to_string())
                            .on_click(move |_, window, cx| {
                                local_files::open_path(shell.clone(), target.clone(), window, cx)
                            })
                    }),
                )
            }))
            .children((!self.unsaved.is_empty()).then(|| {
                ui::card(
                    localized(cx, "Unsaved work", "Несохранённая работа"),
                    None,
                    cx,
                )
                .children(self.unsaved.iter().enumerate().map(
                    |(i, (root, name))| {
                        let shell = self.shell.clone();
                        let target = root.clone();
                        h_flex()
                            .gap_3()
                            .items_center()
                            .justify_between()
                            .child(div().min_w_0().text_sm().child(name.clone()))
                            .child(
                                Button::new(("restore", i))
                                    .small()
                                    .label(localized(cx, "Restore", "Восстановить"))
                                    .tooltip(root.display().to_string())
                                    .on_click(move |_, window, cx| {
                                        recovery::open(shell.clone(), target.clone(), window, cx)
                                    }),
                            )
                    },
                ))
            }))
    }
}
