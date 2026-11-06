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

use exaudio_extension::ExtensionManifest;
use exaudio_shell::screens::ui::{self, localized};
use exaudio_shell::screens::{local_files, recovery};
use exaudio_shell::{
    ContributionRegistry, DocumentOrigin, ModeContribution, ProviderEnvironment, ShellHandle,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

pub const ID: &str = "exaudio.local";

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

/// Folder name of a native project folder or recovery root, when nothing better is known.
fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

struct LocalHome {
    shell: ShellHandle,
    busy: bool,
    progress: Option<(usize, usize, String)>,
    cancellation: Arc<AtomicBool>,
    /// Recent project folders with their project names, loaded off the UI thread.
    recent: Vec<(PathBuf, String)>,
    /// Recovery snapshots to offer, loaded and pruned off the UI thread.
    unsaved: Vec<recovery::Unsaved>,
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
        let recent = local_files::recent(cx);
        let roots = recovery::entries(cx);
        if !recent.is_empty() || !roots.is_empty() {
            cx.spawn(async move |view, cx| {
                let (recent, collected) = cx
                    .background_spawn(async move {
                        let recent = recent
                            .into_iter()
                            .map(|path| {
                                let name = studio_project::read_project_name(&path)
                                    .unwrap_or_else(|_| folder_name(&path));
                                (path, name)
                            })
                            .collect::<Vec<_>>();
                        (recent, recovery::collect(roots))
                    })
                    .await;
                view.update(cx, |this, cx| {
                    this.recent = recent;
                    this.unsaved = collected.unsaved;
                    recovery::forget(&collected.removed, cx);
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
            recent: Vec::new(),
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
        let recent = self.recent.clone();
        v_flex()
            .size_full()
            .id("exaudio-local")
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
                                        exaudio_shell::root::open_demo(&demo, window, cx)
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
                    recent.into_iter().enumerate().map(|(i, (path, name))| {
                        let shell = self.shell.clone();
                        let target = path.clone();
                        Button::new(("recent", i))
                            .label(name)
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
                .children(self.unsaved.iter().enumerate().map(|(i, unsaved)| {
                    let shell = self.shell.clone();
                    let target = unsaved.root.clone();
                    let label = match unsaved.saved_at {
                        Some(saved_at) => {
                            format!("{} · {}", unsaved.name, recovery::saved_label(saved_at, cx))
                        }
                        None => unsaved.name.clone(),
                    };
                    let location = unsaved
                        .native_path
                        .as_ref()
                        .unwrap_or(&unsaved.root)
                        .display()
                        .to_string();
                    h_flex()
                        .gap_3()
                        .items_center()
                        .justify_between()
                        .child(div().min_w_0().text_sm().child(label))
                        .child(
                            Button::new(("restore", i))
                                .small()
                                .label(localized(cx, "Restore", "Восстановить"))
                                .tooltip(location)
                                .on_click(move |_, window, cx| {
                                    recovery::open(shell.clone(), target.clone(), window, cx)
                                }),
                        )
                }))
            }))
    }
}
