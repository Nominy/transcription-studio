use super::*;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::fs;

pub(super) struct Watch {
    _watcher: RecommendedWatcher,
    _task: Task<()>,
    wake: async_channel::Sender<()>,
    stopped: Arc<AtomicBool>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.wake.close();
    }
}
struct Prepared {
    package: Package,
    assets: BTreeMap<String, PathBuf>,
}
fn bounded(path: &std::path::Path, max: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max {
        bail!("The developer folder is missing a file, or the file is too large or a link.");
    }
    let mut value = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut value)?;
    if value.len() as u64 > max {
        bail!("A file in the developer folder grew too large while it was being read.");
    }
    Ok(value)
}
fn prepare(
    directory: &std::path::Path,
    expected: &ExtensionManifest,
    grants: &BTreeSet<Permission>,
    initial: bool,
) -> Result<Prepared> {
    let manifest: ExtensionManifest =
        serde_json::from_slice(&bounded(&directory.join("manifest.json"), 256 * 1024)?)?;
    manifest.validate()?;
    manifest.check_host_compatibility()?;
    if manifest.id != expected.id || initial && manifest != *expected {
        bail!("The extension's permissions changed. Review them again.");
    }
    if manifest.permissions.iter().any(|p| !grants.contains(p)) {
        bail!("The extension asks for new permissions. Review them again.");
    }
    if !matches!(manifest.runtime, exaudio_extension::Runtime::Wasm) {
        bail!("The developer folder has no extension in it.");
    }
    let component = bounded(&directory.join("module.wasm"), 16 * 1024 * 1024)?;
    let mut assets = BTreeMap::new();
    let asset_root = directory.join("assets");
    if asset_root.exists() {
        let root = asset_root.canonicalize()?;
        if !root.starts_with(directory) {
            bail!("The assets folder points outside the developer folder.");
        }
        let mut queue = vec![root.clone()];
        let mut bytes = 0u64;
        while let Some(path) = queue.pop() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if metadata.file_type().is_symlink() {
                    bail!("Links aren't allowed in the assets folder.");
                }
                if metadata.is_dir() {
                    queue.push(entry.path());
                } else if metadata.is_file() {
                    bytes = bytes.saturating_add(metadata.len());
                    if bytes > 32 * 1024 * 1024 || assets.len() >= 2048 {
                        bail!("The assets folder is too large.");
                    }
                    let path = entry.path().canonicalize()?;
                    let id = path
                        .strip_prefix(&root)?
                        .to_string_lossy()
                        .replace('\\', "/");
                    assets.insert(id, path);
                }
            }
        }
    }
    Ok(Prepared {
        package: Package {
            manifest,
            component,
            grants: grants.clone(),
            required_by_open_modes: vec![],
        },
        assets,
    })
}
/// The expected manifest is the exact object the permission UI showed. Re-reading a
/// different identity or expanded grants never silently follows the changed directory.
pub fn watch_development(
    shell: ShellHandle,
    directory: PathBuf,
    data_root: PathBuf,
    expected_manifest: ExtensionManifest,
    approved_grants: BTreeSet<Permission>,
    cx: &mut App,
) -> Result<()> {
    expected_manifest.validate()?;
    if expected_manifest
        .permissions
        .iter()
        .any(|p| !approved_grants.contains(p))
    {
        bail!("The extension's permissions haven't been approved.");
    }
    let directory = directory.canonicalize()?;
    if !directory.is_dir() {
        bail!("The developer folder can't be opened.");
    }
    if !cx.has_global::<WasmExtensions>() {
        cx.set_global(WasmExtensions::default());
    }
    let owner = expected_manifest.id.clone();
    stop_watching(&owner, cx);
    let (wake, events) = async_channel::bounded(1);
    let signal = wake.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|e| !matches!(e.kind, notify::EventKind::Access(_))) {
            let _ = signal.try_send(());
        }
    })?;
    watcher.watch(&directory, RecursiveMode::Recursive)?;
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let task_owner = owner.clone();
    let task_directory = directory.clone();
    let task = cx.spawn(async move |cx| {
        let mut initial = true;
        while events.recv().await.is_ok() {
            // A single wake is retained during this quiet-period debounce. A write storm
            // extends the quiet period instead of compiling each partially-written file.
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if events.try_recv().is_err() {
                    break;
                }
                if stop.load(Ordering::Acquire) {
                    return;
                }
            }
            if stop.load(Ordering::Acquire) {
                break;
            }
            let directory = task_directory.clone();
            let expected = expected_manifest.clone();
            let grants = approved_grants.clone();
            let prepared = cx
                .background_spawn(async move { prepare(&directory, &expected, &grants, initial) })
                .await;
            match prepared {
                Err(error) => {
                    cx.update(|cx| {
                        if cx
                            .global::<WasmExtensions>()
                            .entries
                            .contains_key(&task_owner)
                        {
                            record(&task_owner, "Reload failed", &error.to_string(), cx);
                        } else {
                            cx.update_global::<ContributionRegistry, _>(|r, _| {
                                r.errors.insert(task_owner.clone(), error.to_string());
                            });
                        }
                        cx.refresh_windows();
                    });
                }
                Ok(prepared) => {
                    let result = cx.update(|cx| {
                        activate(
                            shell.clone(),
                            prepared.package,
                            data_root.clone(),
                            prepared.assets,
                            cx,
                        )
                    });
                    match result {
                        Ok(ticket) => loop {
                            if stop.load(Ordering::Acquire) {
                                ticket.cancel();
                                break;
                            }
                            match ticket.try_recv() {
                                Ok(None) => {
                                    cx.background_executor()
                                        .timer(Duration::from_millis(10))
                                        .await
                                }
                                Ok(Some(Ok(()))) => {
                                    initial = false;
                                    cx.update(|cx| {
                                        cx.update_global::<WasmExtensions, _>(|state, _| {
                                            if let Some(entry) = state.entries.get_mut(&task_owner)
                                            {
                                                entry.status.development_directory =
                                                    Some(task_directory.clone());
                                                push_log(
                                                    &mut entry.status,
                                                    "Developer extension started",
                                                );
                                            }
                                        })
                                    });
                                    break;
                                }
                                Ok(Some(Err(error))) => {
                                    cx.update(|cx| {
                                        record(&task_owner, "Reload failed", &error.to_string(), cx)
                                    });
                                    break;
                                }
                                Err(_) => break,
                            }
                        },
                        Err(error) => {
                            cx.update(|cx| {
                                record(&task_owner, "Reload failed", &error.to_string(), cx)
                            });
                        }
                    }
                }
            }
        }
    });
    cx.update_global::<WasmExtensions, _>(|state, _| {
        state.watches.insert(
            owner,
            Watch {
                _watcher: watcher,
                _task: task,
                wake: wake.clone(),
                stopped,
            },
        );
    });
    let _ = wake.try_send(());
    Ok(())
}
pub fn reload_development(owner: &str, cx: &mut App) -> Result<()> {
    let watch = cx
        .global::<WasmExtensions>()
        .watches
        .get(owner)
        .ok_or_else(|| anyhow!("This developer folder isn't being watched."))?;
    let _ = watch.wake.try_send(());
    Ok(())
}
pub fn stop_watching(owner: &str, cx: &mut App) {
    if cx.has_global::<WasmExtensions>() {
        cx.update_global::<WasmExtensions, _>(|state, _| {
            state.watches.remove(owner);
        });
    }
}
