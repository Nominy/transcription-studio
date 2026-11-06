//! Periodic native recovery, independent of provider-owned documents and save routes.
use crate::{
    DocumentOrigin, ProviderEnvironment, ShellHandle, editor::model::EditorModel, screens::ui,
    services::AppServices,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use std::{
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use studio_project::{RecoveryEntry, RecoverySnapshot};

const STORAGE_KEY: &str = "exaudio.native-recovery";
const SNAPSHOT_FILE: &str = "autosave/recovery.json";

/// Remembered snapshot roots, in the order they were first written.
pub fn entries(cx: &App) -> Vec<PathBuf> {
    cx.global::<AppServices>()
        .storage
        .value(STORAGE_KEY)
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_default()
}
fn store(roots: &[PathBuf], cx: &mut App) -> anyhow::Result<()> {
    cx.global_mut::<AppServices>()
        .storage
        .set_value(STORAGE_KEY, serde_json::to_string(roots)?)?;
    Ok(())
}
fn remember(root: &PathBuf, cx: &mut App) -> anyhow::Result<()> {
    let mut roots = entries(cx);
    if !roots.contains(root) {
        roots.push(root.clone());
        store(&roots, cx)?;
    }
    Ok(())
}
/// Stops offering roots whose snapshots were deleted from disk.
pub fn forget(removed: &[PathBuf], cx: &mut App) {
    if removed.is_empty() {
        return;
    }
    let mut roots = entries(cx);
    let before = roots.len();
    roots.retain(|root| !removed.contains(root));
    if roots.len() != before {
        store(&roots, cx).ok();
    }
}

/// One snapshot root per project and save folder, so repeated snapshots of the same
/// document replace each other instead of piling up.
fn snapshot_root(project_id: &str, native_path: Option<&PathBuf>) -> PathBuf {
    let mut hasher = std::hash::DefaultHasher::new();
    project_id.hash(&mut hasher);
    native_path.hash(&mut hasher);
    studio_services::paths::app_support_dir()
        .join("native-recovery")
        .join(format!("{:016x}", hasher.finish()))
}

/// Deletes the snapshots a successful save to `path` makes redundant: the one taken
/// while the project was still unsaved and the one for this folder. Audio the open
/// document was restored from stays on disk (only the snapshot manifest goes) until the
/// next Unsaved-work listing cleans the root. Blocking; returns the roots fully removed.
pub fn discard_after_save(project_id: &str, path: &Path, open_files: &[PathBuf]) -> Vec<PathBuf> {
    let path = path.to_path_buf();
    [
        snapshot_root(project_id, None),
        snapshot_root(project_id, Some(&path)),
    ]
    .into_iter()
    .filter(|root| {
        // Loaded lanes carry canonical paths (`\\?\` on Windows); the root may not.
        let canonical = fs::canonicalize(root).ok();
        let in_use = open_files.iter().any(|file| {
            file.starts_with(root) || canonical.as_ref().is_some_and(|c| file.starts_with(c))
        });
        if in_use {
            // A failed delete just leaves the snapshot to the next listing.
            fs::remove_file(root.join(SNAPSHOT_FILE)).ok();
            false
        } else {
            remove_root(root)
        }
    })
    .collect()
}

/// True when the root is gone afterwards.
fn remove_root(root: &Path) -> bool {
    fs::remove_dir_all(root).ok();
    !root.exists()
}

/// A row of the Unsaved-work list.
pub struct Unsaved {
    pub root: PathBuf,
    pub name: String,
    /// Folder the project was saved to when the snapshot was taken, if any.
    pub native_path: Option<PathBuf>,
    /// `None` for a snapshot that exists but could not be read.
    pub saved_at: Option<SystemTime>,
}
pub struct Collected {
    /// Newest first; one row per project.
    pub unsaved: Vec<Unsaved>,
    /// Roots deleted while building the list; pass to [`forget`].
    pub removed: Vec<PathBuf>,
}

/// Blocking: reads every remembered root for the Unsaved-work list. Older duplicates of a
/// project, snapshots already covered by a later save, and roots without a snapshot are
/// deleted; roots that no longer exist are dropped. Call off the UI thread.
pub fn collect(roots: Vec<PathBuf>) -> Collected {
    let mut removed = Vec::new();
    let mut unreadable = Vec::new();
    let mut loaded = Vec::new();
    for root in roots {
        if !root.is_dir() {
            removed.push(root);
            continue;
        }
        if !root.join(SNAPSHOT_FILE).is_file() {
            if remove_root(&root) {
                removed.push(root);
            }
            continue;
        }
        match studio_project::load_recovery(&root) {
            Ok(snapshot) => loaded.push(RecoveryEntry { root, snapshot }),
            Err(_) => unreadable.push(root),
        }
    }
    let (kept, superseded) = studio_project::newest_per_project(loaded);
    let (stale, current): (Vec<_>, Vec<_>) = kept
        .into_iter()
        .partition(|entry| studio_project::superseded_by_save(&entry.snapshot));
    for entry in superseded.into_iter().chain(stale) {
        if remove_root(&entry.root) {
            removed.push(entry.root);
        }
    }
    let mut unsaved: Vec<Unsaved> = current
        .into_iter()
        .map(|entry| Unsaved {
            root: entry.root,
            name: entry.snapshot.project.name,
            native_path: entry.snapshot.native_path,
            saved_at: Some(entry.snapshot.saved_at),
        })
        .collect();
    unsaved.extend(unreadable.into_iter().map(|root| {
        Unsaved {
            name: root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string()),
            root,
            native_path: None,
            saved_at: None,
        }
    }));
    Collected { unsaved, removed }
}

/// "saved 14:32" for today, otherwise "saved 5 Nov 14:32", in local time.
pub fn saved_label(saved_at: SystemTime, cx: &App) -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let at = time::OffsetDateTime::from(saved_at).to_offset(now.offset());
    let clock = format!("{:02}:{:02}", at.hour(), at.minute());
    let when = if at.date() == now.date() {
        clock
    } else {
        let month = ui::localized(
            cx,
            "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec",
            "янв фев мар апр мая июн июл авг сен окт ноя дек",
        )
        .split(' ')
        .nth(usize::from(u8::from(at.month())) - 1)
        .unwrap_or_default();
        format!("{} {month} {clock}", at.day())
    };
    ui::localized(cx, "saved {time}", "сохранено {time}").replace("{time}", &when)
}

pub fn start(model: &Entity<EditorModel>, cx: &mut App) -> Task<()> {
    let model = model.downgrade();
    cx.spawn(async move |cx| {
        let mut previous = None;
        loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let snapshot = model.update(cx, |model, cx| {
                if model.origin().instance.is_some()
                    || model.doc().is_read_only()
                    || (!model.doc().session().is_dirty && !model.doc().has_pending_drafts())
                {
                    return None;
                }
                let project = match model.doc().recovery_project() {
                    Ok(project) => project,
                    Err(error) => {
                        backup_not_saved(model, &error.to_string(), cx);
                        return None;
                    }
                };
                let native_path = model.origin().native_path.clone();
                let root = snapshot_root(&project.id, native_path.as_ref());
                let base = model
                    .audio_base_dir()
                    .cloned()
                    .unwrap_or_else(|| root.clone());
                Some((project, native_path, root, base))
            });
            let Ok(snapshot) = snapshot else { break };
            let Some((project, native_path, root, base)) = snapshot else {
                continue;
            };
            if previous.as_ref() == Some(&project) {
                continue;
            }
            let target = root.clone();
            let result = cx
                .background_spawn(async move {
                    let snapshot = RecoverySnapshot {
                        project,
                        native_path,
                        saved_at: SystemTime::now(),
                    };
                    let result =
                        studio_services::recovery::write_snapshot(&snapshot, &target, &base);
                    (snapshot.project, result)
                })
                .await;
            match result {
                (project, Ok(())) => {
                    previous = Some(project);
                    model
                        .update(cx, |model, cx| {
                            if let Err(error) = remember(&root, cx) {
                                backup_not_saved(model, &error.to_string(), cx);
                            }
                        })
                        .ok();
                }
                (_, Err(error)) => {
                    model
                        .update(cx, |model, cx| {
                            backup_not_saved(model, &error.to_string(), cx)
                        })
                        .ok();
                }
            }
        }
    })
}
/// Restores a snapshot as a dirty document. A project that had been saved keeps its
/// folder, so Save writes back there without asking; after that save the snapshot goes.
pub fn open(shell: ShellHandle, root: PathBuf, window: &mut Window, cx: &mut App) {
    let target = root.clone();
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(async move { studio_services::recovery::load_snapshot(&target) })
                .await;
            cx.update(|window, cx| match result {
                Ok(snapshot) => {
                    let project = snapshot.project;
                    let env = ProviderEnvironment::local(project.id.clone(), Some(root));
                    let origin = DocumentOrigin::local(snapshot.native_path);
                    match shell.open_document(project, origin, env, window, cx) {
                        Ok(model) => model.update(cx, |model, cx| {
                            model.update_doc(cx, |doc| doc.session_mut().mark_dirty())
                        }),
                        Err(error) => window.push_notification(ui::anyhow_notification(&error), cx),
                    }
                }
                Err(error) => window.push_notification(
                    ui::error_notification(
                        ui::localized(
                            cx,
                            "Couldn't restore the unsaved work.",
                            "Не удалось восстановить несохранённую работу.",
                        ),
                        Some(format!("{error:#}").into()),
                    ),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}

/// Status line for a failed backup write; the I/O error stays behind the hover.
fn backup_not_saved(model: &mut EditorModel, error: &str, cx: &mut Context<EditorModel>) {
    model.set_status_with_detail(
        studio_editor::i18n::localized(
            "Couldn't save a backup of unsaved work.",
            "Не удалось сохранить резервную копию.",
        ),
        error.to_string(),
        cx,
    );
}
