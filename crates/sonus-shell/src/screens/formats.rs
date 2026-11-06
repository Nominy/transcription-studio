//! Native file dialogs backed exclusively by enabled format contributions.

use std::{
    io::{Read, Write},
    sync::Arc,
};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt as _, v_flex};
use gpui_kit::*;
use sonus_extension::document::{DocumentEdit, DocumentTransaction};
use sonus_extension::format::{ExportOptions, FormatProvider, ImportContent};
use studio_core::{Project, Segment};

use crate::screens::ui::localized;
use crate::{ContributionRegistry, ShellHandle};

pub fn export_document(shell: ShellHandle, window: &mut Window, cx: &mut App) {
    choose_format(shell, false, window, cx);
}

pub fn import_document(shell: ShellHandle, window: &mut Window, cx: &mut App) {
    choose_format(shell, true, window, cx);
}

fn choose_format(shell: ShellHandle, importing: bool, window: &mut Window, cx: &mut App) {
    if shell.editor_model(cx).is_none() {
        window.push_notification(
            Notification::info(localized(
                cx,
                "Open an audio project first, then choose a format.",
                "Сначала откройте аудиопроект, затем выберите формат.",
            )),
            cx,
        );
        return;
    }
    let formats: Vec<_> = cx
        .global::<ContributionRegistry>()
        .formats
        .entries()
        .into_iter()
        .filter(|format| {
            if importing {
                format.can_import
            } else {
                format.can_export
            }
        })
        .collect();
    if formats.is_empty() {
        window.push_notification(
            Notification::info(localized(
                cx,
                "No format extension is enabled. Open Extensions to enable one.",
                "Нет активных расширений форматов. Включите их в разделе «Расширения».",
            )),
            cx,
        );
        return;
    }
    window.open_dialog(cx, move |dialog, _, cx| {
        let title = if importing {
            localized(cx, "Import transcript", "Импорт текста")
        } else {
            localized(cx, "Export transcript", "Экспорт текста")
        };
        dialog
            .title(title)
            .child(v_flex().gap_2().children(formats.iter().map(|format| {
                let id = format.id.clone();
                let shell = shell.clone();
                Button::new(SharedString::from(id.clone()))
                    .label(format.name.clone())
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let provider = cx
                            .global::<ContributionRegistry>()
                            .formats
                            .get(&id)
                            .cloned();
                        let Some(provider) = provider else {
                            window.push_notification(
                                Notification::warning("The format extension was disabled."),
                                cx,
                            );
                            return;
                        };
                        if importing {
                            pick_import(shell.clone(), id.clone(), provider, window, cx);
                        } else {
                            pick_export(shell.clone(), id.clone(), provider, window, cx);
                        }
                    })
            })))
    });
}

fn snapshot(
    shell: &ShellHandle,
    action: &'static str,
    cx: &mut App,
) -> Option<(Project, u64, Option<String>)> {
    let model = shell.editor_model(cx)?;
    model.update(cx, |model, cx| {
        let mut committed = false;
        model.edit(cx, |doc| {
            let result = doc.commit_pending_drafts(action);
            committed = result.is_ok();
            result
        });
        committed.then(|| {
            (
                model.doc().project().clone(),
                model.revision(),
                model.doc().session().selected_segment_id.clone(),
            )
        })
    })
}

fn pick_export(
    shell: ShellHandle,
    id: String,
    provider: Arc<dyn FormatProvider>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some((project, _, _)) = snapshot(&shell, "Export", cx) else {
        return;
    };
    let descriptor = provider.descriptor();
    let extension = descriptor
        .extensions
        .first()
        .map(String::as_str)
        .unwrap_or("txt");
    let safe_name: String = project
        .name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let filename = format!(
        "{}.{extension}",
        if safe_name.trim().is_empty() {
            "transcript"
        } else {
            safe_name.trim()
        }
    );
    let picked = cx.prompt_for_new_path(
        &std::env::current_dir().unwrap_or_default(),
        Some(&filename),
    );
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let still_enabled = cx
                .update(|_, cx| {
                    cx.global::<ContributionRegistry>()
                        .formats
                        .get(&id)
                        .is_some()
                })
                .unwrap_or(false);
            if !still_enabled {
                return;
            }
            let target = path.clone();
            let result = cx
                .background_spawn(async move {
                    let bytes = provider
                        .export(&project, &ExportOptions::default())
                        .map_err(|error| error.to_string())?;
                    let parent = target
                        .parent()
                        .ok_or_else(|| "Export path has no directory.".to_owned())?;
                    let mut temporary = tempfile::NamedTempFile::new_in(parent)
                        .map_err(|error| error.to_string())?;
                    temporary
                        .write_all(&bytes)
                        .map_err(|error| error.to_string())?;
                    temporary
                        .as_file()
                        .sync_all()
                        .map_err(|error| error.to_string())?;
                    temporary
                        .persist(&target)
                        .map_err(|error| error.to_string())?;
                    Ok::<(), String>(())
                })
                .await;
            cx.update(|window, cx| match result {
                Ok(()) => window.push_notification(
                    Notification::success(format!("Exported {}", path.display())),
                    cx,
                ),
                Err(error) => window.push_notification(Notification::error(error), cx),
            })
            .ok();
        })
        .detach();
}

fn pick_import(
    shell: ShellHandle,
    id: String,
    provider: Arc<dyn FormatProvider>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some((project, revision, selected)) = snapshot(&shell, "Import", cx) else {
        return;
    };
    let lane = selected
        .as_deref()
        .and_then(|id| project.segment(id))
        .map(|segment| segment.lane_id.clone())
        .or_else(|| (project.lanes.len() == 1).then(|| project.lanes[0].id.clone()));
    let Some(lane_id) = lane else {
        window.push_notification(
            Notification::info(localized(
                cx,
                "Select a segment on the destination lane first.",
                "Сначала выберите сегмент на нужной дорожке.",
            )),
            cx,
        );
        return;
    };
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(localized(cx, "Import transcript", "Импорт текста").into()),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let parsed = cx
                .background_spawn(async move {
                    const MAX_BYTES: u64 = 32 * 1024 * 1024;
                    let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
                    let mut bytes = Vec::new();
                    file.take(MAX_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|error| error.to_string())?;
                    if bytes.len() as u64 > MAX_BYTES {
                        return Err("Transcript file exceeds 32 MiB.".to_owned());
                    }
                    provider.import(&bytes).map_err(|error| error.to_string())
                })
                .await;
            cx.update(|window, cx| match parsed {
                Ok(content) => confirm_import(
                    ImportPreview {
                        shell,
                        format: id,
                        project,
                        revision,
                        selected,
                        lane_id,
                        content,
                    },
                    window,
                    cx,
                ),
                Err(error) => window.push_notification(Notification::error(error), cx),
            })
            .ok();
        })
        .detach();
}

struct ImportPreview {
    shell: ShellHandle,
    format: String,
    project: Project,
    revision: u64,
    selected: Option<String>,
    lane_id: String,
    content: ImportContent,
}

fn confirm_import(preview: ImportPreview, window: &mut Window, cx: &mut App) {
    let ImportPreview {
        shell,
        format,
        project,
        revision,
        selected,
        lane_id,
        content,
    } = preview;
    let Some(lane) = project.lanes.iter().find(|lane| lane.id == lane_id) else {
        return;
    };
    let (edits, detail) = match content {
        ImportContent::Untimed(text) => {
            let Some(segment_id) = selected else {
                window.push_notification(Notification::info(localized(cx,
                    "Plain text has no timestamps. Select an existing segment to place it without inventing timing.",
                    "В обычном тексте нет таймкодов. Выберите существующий сегмент для вставки без выдуманных границ.")), cx);
                return;
            };
            let detail = localized(
                cx,
                "Replace the selected segment's text? Its existing timing is unchanged.",
                "Заменить текст выбранного сегмента? Его временные границы сохранятся.",
            )
            .to_owned();
            (
                vec![DocumentEdit::ReplaceText {
                    segment_id,
                    range: None,
                    text,
                }],
                detail,
            )
        }
        ImportContent::Timed(cues) => {
            let detail = format!(
                "{} {}. {}",
                cues.len(),
                localized(
                    cx,
                    "cues will be appended to this lane",
                    "сегментов будет добавлено на эту дорожку"
                ),
                localized(
                    cx,
                    "Existing segments stay unchanged; source speaker labels are kept in the text.",
                    "Существующие сегменты сохранятся; имена говорящих останутся в тексте."
                )
            );
            let mut next = 0_u64;
            let edits = cues
                .into_iter()
                .map(|cue| {
                    let id = loop {
                        let id = format!("{}-import-{revision}-{next}", project.id);
                        next += 1;
                        if !project.has_segment(&id) {
                            break id;
                        }
                    };
                    let text = match cue.speaker {
                        Some(speaker) => format!("{speaker}: {}", cue.text),
                        None => cue.text,
                    };
                    DocumentEdit::InsertSegment {
                        segment: Segment {
                            id,
                            source_annotation_id: None,
                            speaker_id: lane.speaker_id.clone(),
                            lane_id: lane_id.clone(),
                            start_seconds: cue.start_seconds,
                            end_seconds: cue.end_seconds,
                            text,
                            metadata: Default::default(),
                        },
                    }
                })
                .collect();
            (edits, detail)
        }
    };
    let transaction = DocumentTransaction {
        label: "Import transcript".into(),
        expected_revision: revision,
        edits,
    };
    window.open_dialog(cx, move |dialog, _, cx| {
        let shell = shell.clone();
        let format = format.clone();
        let transaction = transaction.clone();
        let project_id = project.id.clone();
        dialog
            .title(localized(cx, "Confirm import", "Подтвердите импорт"))
            .child(
                v_flex()
                    .gap_3()
                    .text_color(cx.theme().foreground)
                    .child(detail.clone())
                    .child(
                        Button::new("apply-transcript-import")
                            .primary()
                            .label(localized(cx, "Import", "Импортировать"))
                            .on_click(move |_, window, cx| {
                                if cx
                                    .global::<ContributionRegistry>()
                                    .formats
                                    .get(&format)
                                    .is_none()
                                {
                                    window.push_notification(
                                        Notification::error("The format extension was disabled."),
                                        cx,
                                    );
                                    return;
                                }
                                let Some(model) = shell.editor_model(cx) else {
                                    return;
                                };
                                if model.read(cx).doc().project().id != project_id {
                                    window.push_notification(
                                        Notification::error(
                                            "The active document changed. Start the import again.",
                                        ),
                                        cx,
                                    );
                                    return;
                                }
                                model.update(cx, |model, cx| {
                                    model.edit(cx, |doc| doc.apply_transaction(transaction.clone()))
                                });
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}
