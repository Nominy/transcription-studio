//! Native file dialogs backed exclusively by enabled format contributions.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use exaudio_extension::document::{DocumentEdit, DocumentTransaction};
use exaudio_extension::format::{ExportOptions, FormatDescriptor, FormatProvider, ImportContent};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt as _, v_flex};
use gpui_kit::*;
use studio_core::{Project, Segment};

use crate::screens::ui::localized;
use crate::{ContributionRegistry, ShellHandle};

pub fn export_document(shell: ShellHandle, window: &mut Window, cx: &mut App) {
    if shell.editor_model(cx).is_none() {
        window.push_notification(open_project_first(cx), cx);
        return;
    }
    let formats: Vec<_> = cx
        .global::<ContributionRegistry>()
        .formats
        .entries()
        .into_iter()
        .filter(|format| format.can_export)
        .collect();
    if formats.is_empty() {
        window.push_notification(
            Notification::info(localized(
                cx,
                "No export formats are enabled. Turn one on in Extensions.",
                "Форматы экспорта отключены. Включите их в Расширениях.",
            )),
            cx,
        );
        return;
    }
    window.open_dialog(cx, move |dialog, _, cx| {
        dialog
            .title(localized(cx, "Export as…", "Экспортировать как…"))
            .child(v_flex().gap_2().children(formats.iter().map(|format| {
                let id = format.id.clone();
                let shell = shell.clone();
                Button::new(SharedString::from(id.clone()))
                    .label(format.name.clone())
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let Some(provider) = enabled_provider(&id, cx) else {
                            window.push_notification(format_turned_off(cx), cx);
                            return;
                        };
                        pick_export(shell.clone(), id.clone(), provider, window, cx);
                    })
            })))
    });
}

/// One file picker for every enabled import format; the format is inferred from the chosen
/// file's extension (gpui's picker cannot filter by extension itself).
pub fn import_document(shell: ShellHandle, window: &mut Window, cx: &mut App) {
    if shell.editor_model(cx).is_none() {
        window.push_notification(open_project_first(cx), cx);
        return;
    }
    let formats: Vec<_> = cx
        .global::<ContributionRegistry>()
        .formats
        .entries()
        .into_iter()
        .filter(|format| format.can_import)
        .collect();
    if formats.is_empty() {
        window.push_notification(
            Notification::info(localized(
                cx,
                "No import formats are enabled. Turn one on in Extensions.",
                "Форматы импорта отключены. Включите их в Расширениях.",
            )),
            cx,
        );
        return;
    }
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
                "Select a segment on the target track first.",
                "Сначала выберите сегмент на нужной дорожке.",
            )),
            cx,
        );
        return;
    };
    let supported = supported_extensions(&formats);
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(
            localized(
                cx,
                "Import transcript ({types})",
                "Импортировать текст ({types})",
            )
            .replace("{types}", &supported)
            .into(),
        ),
    });
    let unsupported = localized(
        cx,
        "{file} isn't a supported transcript. Supported: {types}.",
        "{file} — неподдерживаемый формат. Поддерживаются: {types}.",
    )
    .replace("{types}", &supported);
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            cx.update(|window, cx| {
                let Some(id) = format_for_path(&formats, &path) else {
                    window.push_notification(
                        Notification::warning(unsupported.replace("{file}", &file_name(&path))),
                        cx,
                    );
                    return;
                };
                let Some(provider) = enabled_provider(id, cx) else {
                    window.push_notification(format_turned_off(cx), cx);
                    return;
                };
                let target = ImportTarget {
                    shell,
                    format: id.to_owned(),
                    project,
                    revision,
                    selected,
                    lane_id,
                };
                read_import(target, provider, path, window, cx);
            })
            .ok();
        })
        .detach();
}

fn open_project_first(cx: &App) -> Notification {
    Notification::info(localized(
        cx,
        "Open a project first.",
        "Сначала откройте проект.",
    ))
}

fn format_turned_off(cx: &App) -> Notification {
    Notification::warning(localized(
        cx,
        "That format was just turned off.",
        "Этот формат только что отключили.",
    ))
}

fn enabled_provider(id: &str, cx: &App) -> Option<Arc<dyn FormatProvider>> {
    cx.global::<ContributionRegistry>().formats.get(id).cloned()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `.srt, .vtt, …` over every import format, deduplicated in registration order.
fn supported_extensions(formats: &[FormatDescriptor]) -> String {
    let mut seen = Vec::new();
    for extension in formats.iter().flat_map(|format| &format.extensions) {
        if !seen.contains(&extension) {
            seen.push(extension);
        }
    }
    seen.iter()
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// First import format whose extensions include the file's (case-insensitively).
fn format_for_path<'a>(formats: &'a [FormatDescriptor], path: &Path) -> Option<&'a str> {
    let extension = path.extension()?.to_str()?.to_lowercase();
    formats
        .iter()
        .find(|format| format.extensions.contains(&extension))
        .map(|format| format.id.as_str())
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
            localized(cx, "transcript", "расшифровка")
        } else {
            safe_name.trim()
        }
    );
    let picked = cx.prompt_for_new_path(
        &std::env::current_dir().unwrap_or_default(),
        Some(&filename),
    );
    let export_dir = localized(
        cx,
        "Choose a folder to export to.",
        "Выберите папку для экспорта.",
    );
    let exported = localized(cx, "Exported to {file}", "Экспортировано в {file}");
    let export_failed = localized(cx, "Couldn't export.", "Не удалось экспортировать.");
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
                    let parent = target.parent().ok_or_else(|| export_dir.to_owned())?;
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
                    Notification::success(
                        exported.replace(
                            "{file}",
                            &path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        ),
                    ),
                    cx,
                ),
                Err(error) => window.push_notification(
                    if error == export_dir {
                        Notification::error(error)
                    } else {
                        crate::screens::ui::error_notification(export_failed, Some(error.into()))
                    },
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}

/// Reads and parses the chosen file off the UI thread, then hands over to the confirmation.
fn read_import(
    target: ImportTarget,
    provider: Arc<dyn FormatProvider>,
    path: PathBuf,
    window: &mut Window,
    cx: &mut App,
) {
    let name = file_name(&path);
    let too_large = localized(
        cx,
        "This file is too large to import (over 32 MB).",
        "Файл слишком большой для импорта (больше 32 МБ).",
    );
    let unreadable = localized(cx, "Couldn't read {file}.", "Не удалось прочитать {file}.");
    window
        .spawn(cx, async move |cx| {
            let parsed = cx
                .background_spawn(async move {
                    const MAX_BYTES: u64 = 32 * 1024 * 1024;
                    let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
                    let mut bytes = Vec::new();
                    file.take(MAX_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|error| error.to_string())?;
                    if bytes.len() as u64 > MAX_BYTES {
                        return Err(too_large.to_owned());
                    }
                    provider.import(&bytes).map_err(|error| error.to_string())
                })
                .await;
            cx.update(|window, cx| match parsed {
                Ok(content) => confirm_import(target, content, window, cx),
                Err(error) => window.push_notification(
                    if error == too_large {
                        Notification::error(error)
                    } else {
                        crate::screens::ui::error_notification(
                            unreadable.replace("{file}", &name),
                            Some(error.into()),
                        )
                    },
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}

/// Where an import lands, captured before the picker opens.
struct ImportTarget {
    shell: ShellHandle,
    format: String,
    project: Project,
    revision: u64,
    selected: Option<String>,
    lane_id: String,
}

fn confirm_import(target: ImportTarget, content: ImportContent, window: &mut Window, cx: &mut App) {
    let ImportTarget {
        shell,
        format,
        project,
        revision,
        selected,
        lane_id,
    } = target;
    let Some(lane) = project.lanes.iter().find(|lane| lane.id == lane_id) else {
        return;
    };
    let (edits, detail) = match content {
        ImportContent::Untimed(text) => {
            let Some(segment_id) = selected else {
                window.push_notification(
                    Notification::info(localized(
                        cx,
                        "Plain text has no timing. Select the segment to put it in.",
                        "В обычном тексте нет тайминга. Выберите сегмент, куда его вставить.",
                    )),
                    cx,
                );
                return;
            };
            let detail = localized(
                cx,
                "Replace the text of the selected segment?",
                "Заменить текст выбранного сегмента?",
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
            let detail = localized(
                cx,
                "Add {n} subtitles to “{track}”? Existing segments are kept. Speaker names from the file stay in the text.",
                "Добавить {n} субтитров на дорожку «{track}»? Существующие сегменты сохранятся. Имена спикеров из файла останутся в тексте.",
            )
            .replace("{n}", &cues.len().to_string())
            .replace("{track}", &lane.label);
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
        label: localized(cx, "Import transcript", "Импорт текста").into(),
        expected_revision: revision,
        edits,
    };
    window.open_dialog(cx, move |dialog, _, cx| {
        let shell = shell.clone();
        let format = format.clone();
        let transaction = transaction.clone();
        let project_id = project.id.clone();
        dialog.title(localized(cx, "Import", "Импорт")).child(
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
                                    Notification::error(localized(
                                        cx,
                                        "That format was just turned off.",
                                        "Этот формат только что отключили.",
                                    )),
                                    cx,
                                );
                                return;
                            }
                            let Some(model) = shell.editor_model(cx) else {
                                return;
                            };
                            if model.read(cx).doc().project().id != project_id {
                                window.push_notification(
                                    Notification::error(localized(
                                        cx,
                                        "The project changed meanwhile. Start the import again.",
                                        "Проект изменился. Начните импорт заново.",
                                    )),
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

#[cfg(test)]
mod tests {
    use super::{FormatDescriptor, format_for_path, supported_extensions};
    use std::path::Path;

    fn format(id: &str, extensions: &[&str]) -> FormatDescriptor {
        FormatDescriptor {
            id: id.into(),
            name: id.into(),
            extensions: extensions.iter().map(|s| s.to_string()).collect(),
            media_type: "text/plain".into(),
            can_import: true,
            can_export: false,
        }
    }

    #[test]
    fn format_is_inferred_from_the_extension() {
        let formats = [format("srt", &["srt"]), format("vtt", &["vtt", "webvtt"])];
        assert_eq!(format_for_path(&formats, Path::new("a/b.SRT")), Some("srt"));
        assert_eq!(
            format_for_path(&formats, Path::new("b.webvtt")),
            Some("vtt")
        );
        assert_eq!(format_for_path(&formats, Path::new("c.docx")), None);
        assert_eq!(format_for_path(&formats, Path::new("noext")), None);
        assert_eq!(supported_extensions(&formats), ".srt, .vtt, .webvtt");
    }
}
