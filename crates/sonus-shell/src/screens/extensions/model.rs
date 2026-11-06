use crate::{
    ShellHandle,
    contributions::ContributionRegistry,
    services::{AppServices, spawn_service},
};
use anyhow::{Context as _, Result, anyhow, bail};
use gpui_kit::*;
use sonus_extension::{ExtensionManifest, Permission, Runtime};
use sonus_marketplace::{
    self as marketplace, ActiveRequirement, Catalog, DeveloperPreview, HostState, InstallRequest,
    InstallationState, InstallationStore, Limits, MarketplaceClient, OperationControl,
    PermissionGrants, PlanDiff, Release,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

pub const DEFAULT_INDEX: &str =
    "https://raw.githubusercontent.com/Nominy/sonus-extensions/main/index.json";
const INDEX_KEY: &str = "sonus.marketplace.index";
struct SharedExtensions(Entity<ExtensionsModel>);
impl Global for SharedExtensions {}

pub fn shared(shell: ShellHandle, cx: &mut App) -> Entity<ExtensionsModel> {
    if !cx.has_global::<SharedExtensions>() {
        let index = cx
            .global::<AppServices>()
            .storage
            .value(INDEX_KEY)
            .unwrap_or(DEFAULT_INDEX)
            .to_owned();
        let root = studio_services::paths::app_support_dir().join("extensions");
        let model = cx.new(|_| ExtensionsModel {
            shell,
            index,
            root,
            catalog: None,
            installed: InstallationState::default(),
            preview: None,
            busy: false,
            offline: true,
            from_cache: false,
            status: String::new(),
            error: None,
            logs: VecDeque::new(),
            control: None,
            task: None,
            progress_task: None,
            started: Instant::now(),
            developer_folders: BTreeMap::new(),
            global_document_access: false,
            document_scopes: BTreeMap::new(),
        });
        cx.set_global(SharedExtensions(model.clone()));
        model.update(cx, |model, cx| model.restore_startup(cx));
    }
    cx.global::<SharedExtensions>().0.clone()
}

#[derive(Clone, Copy, Debug)]
pub enum Removal {
    Enable,
    Disable,
    Uninstall,
}
pub enum Preview {
    Install {
        owner: String,
        plan: Box<PlanDiff>,
        developer: Option<DeveloperPreview>,
        generation: u64,
    },
    Folder {
        directory: PathBuf,
        manifest: ExtensionManifest,
        blockers: Vec<String>,
    },
    Removal {
        id: String,
        action: Removal,
        generation: u64,
        blockers: Vec<String>,
        details: Vec<String>,
    },
    Rollback {
        generation: u64,
        lines: Vec<String>,
        blockers: Vec<String>,
    },
}
impl Preview {
    pub fn can_apply(&self) -> bool {
        match self {
            Self::Install {
                plan, developer, ..
            } => plan.blockers.is_empty() && (!plan.releases.is_empty() || developer.is_some()),
            Self::Folder { blockers, .. }
            | Self::Removal { blockers, .. }
            | Self::Rollback { blockers, .. } => blockers.is_empty(),
        }
    }
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![];
        match self {
            Self::Install {
                owner,
                plan,
                developer,
                generation,
            } => {
                lines.push(format!("Installation generation / Поколение установки: {generation} · selected / выбрано: {owner}"));
                if let Some(source) = developer {
                    lines.push(format!("UNREVIEWED DEVELOPER SOURCE / НЕПРОВЕРЕННЫЙ ИСТОЧНИК: {}\nSHA256: {}\n{} bytes", source.source.display(), source.sha256, source.bytes));
                    manifest_lines(&source.manifest, &mut lines);
                }
                for change in &plan.changes {
                    lines.push(format!("{change:?}"));
                }
                for release in &plan.releases {
                    lines.push(format!(
                        "{} · {} bytes · SHA256 {}",
                        release.asset.url, release.asset.bytes, release.asset.sha256
                    ));
                    manifest_lines(&release.manifest, &mut lines);
                }
                for (id, permissions) in &plan.requested_permissions {
                    lines.push(format!("NEW GRANTS / НОВЫЕ РАЗРЕШЕНИЯ: {id}"));
                    lines.extend(permissions.iter().map(permission_label));
                }
                for (id, reasons) in &plan.required_by {
                    lines.push(format!(
                        "{id} ← {}",
                        reasons.iter().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
                lines.extend(
                    plan.blockers
                        .iter()
                        .map(|error| format!("BLOCKED / ЗАПРЕЩЕНО: {error}")),
                );
            }
            Self::Folder {
                directory,
                manifest,
                blockers,
            } => {
                lines.push(format!(
                    "UNREVIEWED WATCHED FOLDER / НЕПРОВЕРЕННАЯ ОТСЛЕЖИВАЕМАЯ ПАПКА: {}",
                    directory.display()
                ));
                lines.push("Approval trusts future code changes in this folder within the approved identity and grants. / Разрешение распространяется на изменения кода этой папки в пределах ID и разрешений.".into());
                manifest_lines(manifest, &mut lines);
                lines.extend(
                    blockers
                        .iter()
                        .map(|error| format!("BLOCKED / ЗАПРЕЩЕНО: {error}")),
                );
            }
            Self::Removal {
                id,
                action,
                generation,
                blockers,
                details,
            } => {
                lines.push(format!("{action:?}: {id} · generation {generation}"));
                lines.extend(details.iter().cloned());
                lines.push("Runtime contributions, publications and subscriptions are removed on disable/uninstall. Retained rollback packages are not deleted. / При отключении и удалении очищаются команды, панели и подписки. Пакеты отката сохраняются.".into());
                lines.extend(blockers.iter().cloned());
            }
            Self::Rollback {
                generation,
                lines: changes,
                blockers,
            } => {
                lines.push(format!("Restore previous COMPLETE dependency set / Восстановить ПОЛНЫЙ предыдущий набор, generation {generation}"));
                lines.extend(changes.iter().cloned());
                lines.extend(blockers.iter().cloned());
            }
        }
        lines
    }
}
fn manifest_lines(manifest: &ExtensionManifest, lines: &mut Vec<String>) {
    lines.push(format!(
        "{} ({}) {} · {} · API {}",
        manifest.name, manifest.id, manifest.version, manifest.license, manifest.api
    ));
    if manifest.permissions.is_empty() {
        lines.push("Permissions: none / Разрешений нет".into());
    }
    lines.extend(manifest.permissions.iter().map(permission_label));
    for (id, version) in &manifest.dependencies {
        lines.push(format!("Dependency / Зависимость: {id} {version}"));
    }
}
pub(super) fn permission_label(permission: &Permission) -> String {
    match permission {
        Permission::DocumentRead => "Read document / Чтение документа".into(),
        Permission::DocumentEdit => "Edit document / Изменение документа".into(),
        Permission::AudioRead => "Read audio samples / Чтение аудиоданных".into(),
        Permission::TransportControl => {
            "Control playback and seek / Управление воспроизведением".into()
        }
        Permission::SelectionControl => "Change editor selection / Изменение выделения".into(),
        Permission::FilePicker => "Request file picker / Открытие диалога выбора файла".into(),
        Permission::Storage => "Private extension storage / Хранилище расширения".into(),
        Permission::Secrets => "Private extension secrets / Секреты расширения".into(),
        Permission::Network(host) => {
            format!("HTTPS network access / Сетевой доступ: {host} (exact host / точный адрес)")
        }
    }
}

/// Emitted after a successful catalog/package transaction with the newly activated
/// extension ids, so the UI can offer follow-up steps (provider sign-in).
pub enum ExtensionsEvent {
    Activated(Vec<String>),
}
impl EventEmitter<ExtensionsEvent> for ExtensionsModel {}
pub struct ExtensionsModel {
    shell: ShellHandle,
    pub index: String,
    root: PathBuf,
    pub catalog: Option<Catalog>,
    pub installed: InstallationState,
    pub preview: Option<Preview>,
    pub busy: bool,
    pub offline: bool,
    pub from_cache: bool,
    pub status: String,
    pub error: Option<String>,
    pub logs: VecDeque<String>,
    pub developer_folders: BTreeMap<String, (PathBuf, ExtensionManifest)>,
    pub global_document_access: bool,
    document_scopes: BTreeMap<String, bool>,
    control: Option<OperationControl>,
    task: Option<Task<()>>,
    progress_task: Option<Task<()>>,
    started: Instant,
}
impl ExtensionsModel {
    fn log(&mut self, value: impl Into<String>) {
        if self.logs.len() == 128 {
            self.logs.pop_front();
        }
        self.logs
            .push_back(format!("{:?} {}", SystemTime::now(), value.into()));
    }
    pub fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        self.log(&error);
        self.error = Some(error);
        self.busy = false;
        self.control = None;
        self.progress_task = None;
        cx.notify();
    }
    fn begin(&mut self, status: &str, cx: &mut Context<Self>) -> OperationControl {
        self.busy = true;
        self.error = None;
        self.status = status.into();
        self.started = Instant::now();
        let (sender, receiver) = async_channel::bounded(64);
        let control = OperationControl::new(move |progress| {
            let _ = sender.try_send(progress);
        });
        self.control = Some(control.clone());
        self.progress_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(progress) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        let total = progress
                            .total
                            .map(|total| total.to_string())
                            .unwrap_or_else(|| "?".into());
                        let unit = match progress.stage {
                            marketplace::Stage::Downloading => "bytes / байт",
                            marketplace::Stage::Extracting => "entries / файлов",
                            _ => "units / единиц",
                        };
                        this.status = format!(
                            "{:?} · {} · {}/{total} {unit} · {:.1}s",
                            progress.stage,
                            progress.extension.as_deref().unwrap_or("index"),
                            progress.completed,
                            this.started.elapsed().as_secs_f64()
                        );
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
        control
    }
    fn finish(&mut self, status: &str, cx: &mut Context<Self>) {
        self.log(status);
        self.status = status.into();
        self.busy = false;
        self.control = None;
        self.progress_task = None;
        cx.notify();
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(control) = &self.control {
            control.cancel();
        }
        self.status = "Stopping / Остановка: network and staging cancel; an atomic commit already in progress finishes or recovers.".into();
        cx.notify();
    }
    pub fn set_index(&mut self, source: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Err(error) = MarketplaceClient::new(&source, self.root.join("catalog-cache.json")) {
            self.fail(error.to_string(), cx);
            return;
        }
        if let Err(error) = cx
            .global_mut::<AppServices>()
            .storage
            .set_value(INDEX_KEY, source.clone())
        {
            self.fail(error.to_string(), cx);
            return;
        }
        self.index = source;
        self.catalog = None;
        self.preview = None;
        self.refresh(false, cx);
    }
    pub fn refresh(&mut self, offline: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.preview = None;
        self.offline = offline;
        let control = self.begin(
            "Loading validated index / Загрузка проверенного индекса",
            cx,
        );
        let root = self.root.clone();
        let source = self.index.clone();
        self.task = Some(spawn_service(
            cx,
            async move {
                let state = InstallationStore::open(&root).and_then(|store| store.state());
                let catalog = match MarketplaceClient::new(&source, root.join("catalog-cache.json"))
                {
                    Ok(client) => client.catalog_with_progress(offline, &control).await,
                    Err(error) => Err(error),
                };
                (state, catalog)
            },
            |this, (state, catalog), cx| {
                if let Ok(state) = state.as_ref() {
                    this.installed = state.clone();
                }
                match catalog {
                    Ok(snapshot) => {
                        this.catalog = Some(snapshot.catalog);
                        this.from_cache = snapshot.from_cache;
                        this.error = snapshot.refresh_error;
                    }
                    Err(error) => {
                        this.from_cache = this.catalog.is_some();
                        this.error = Some(format!(
                            "{}: {error}. Bundled extensions remain available. / Встроенные расширения доступны.",
                            this.index
                        ));
                    }
                }
                if let Err(error) = state {
                    this.error = Some(format!(
                        "Installation state: {error}. Do not modify extensions until the store is readable."
                    ));
                }
                this.finish(
                    "Catalog refresh finished / Обновление каталога завершено",
                    cx,
                );
            },
        ));
    }
    /// Restores only explicitly approved managed packages. No catalog network fetch,
    /// developer-directory discovery, or inferred permission approval occurs at boot.
    pub fn restore_startup(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let root = self.root.clone();
        let source = self.index.clone();
        let host = self.host(cx);
        let control = self.begin(
            "Restoring installed extensions / Восстановление расширений",
            cx,
        );
        self.task = Some(spawn_service(
            cx,
            async move {
                let store = InstallationStore::open(&root)?;
                let state = store.state()?;
                let packages = store
                    .validate_current_with_progress(&host, Limits::default(), &control)
                    .map_err(anyhow::Error::from)
                    .and_then(|validated| prepare_all(&store, &validated, &host));
                let catalog = match MarketplaceClient::new(&source, root.join("catalog-cache.json"))
                {
                    Ok(client) => client.catalog_with_progress(true, &control).await,
                    Err(error) => Err(error),
                };
                Ok::<_, anyhow::Error>((state, catalog, packages))
            },
            |this, result, cx| match result {
                Ok((state, catalog, packages)) => {
                    this.installed = state;
                    match catalog {
                        Ok(snapshot) => {
                            this.catalog = Some(snapshot.catalog);
                            this.from_cache = true;
                        }
                        Err(marketplace::Error::Io(error))
                            if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => this.log(format!("Offline index: {error}")),
                    }
                    match packages {
                        Ok(packages) => this.activate_startup(packages, cx),
                        Err(error) => this.fail(format!("Installed extension validation failed: {error:#}. Enable missing dependencies or disable the affected package, then retry activation."), cx),
                    }
                }
                Err(error) => this.fail(
                    format!("Cannot restore installed extensions: {error:#}"),
                    cx,
                ),
            },
        ));
    }
    fn activate_startup(&mut self, packages: Vec<Prepared>, cx: &mut Context<Self>) {
        self.progress_task = None;
        let already_enabled: BTreeSet<_> = self
            .installed
            .active
            .keys()
            .filter(|id| cx.global::<ContributionRegistry>().is_enabled(id))
            .cloned()
            .collect();
        let shell = self.shell.clone();
        let root = self.root.clone();
        let total = packages.len();
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut added = Vec::new();
            let mut failure = None;
            for (index, prepared) in packages.into_iter().enumerate() {
                let owner = prepared.package.manifest.id.clone();
                let ticket = this.update(cx, |this, cx| {
                    if this.control.as_ref().is_some_and(OperationControl::is_cancelled) { bail!("Startup activation cancelled"); }
                    this.status = format!("Restoring / Восстановление {}/{total}: {owner}", index + 1); cx.notify();
                    crate::wasm_host::activate(shell.clone(), prepared.package, root.join("data"), prepared.assets, cx)
                });
                let result = match ticket {
                    Ok(Ok(ticket)) => wait_activation(&ticket, cx).await,
                    Ok(Err(error)) => Err(error), Err(error) => Err(error),
                };
                if !already_enabled.contains(&owner) { added.push(owner); }
                if let Err(error) = result { failure = Some(error); break; }
            }
            if let Some(error) = failure {
                let mut cleanup_errors = Vec::new();
                for owner in added.into_iter().rev() {
                    match this.update(cx, |_, cx| crate::wasm_host::disable(&owner, cx)) {
                        Ok(Ok(ticket)) => if let Err(error) = wait_activation(&ticket, cx).await { cleanup_errors.push(error.to_string()); },
                        Ok(Err(error)) => cleanup_errors.push(error.to_string()),
                        Err(error) => cleanup_errors.push(error.to_string()),
                    }
                }
                this.update(cx, |this, cx| this.fail(format!("Startup activation failed: {error:#}. Store unchanged; use Retry activation after correcting the error. {}", cleanup_errors.join("; ")), cx)).ok();
            } else {
                this.update(cx, |this, cx| this.finish(&format!("Restored {total}/{total} managed extensions / Восстановлено {total}/{total} расширений"), cx)).ok();
            }
        }));
    }
    fn host(&self, cx: &App) -> HostState {
        let registry = cx.global::<ContributionRegistry>();
        let manifests = registry.manifests();
        let required = registry.required_by();
        let mut host = HostState::default();
        for manifest in manifests {
            if let Some(reasons) = required.get(&manifest.id) {
                for reason in reasons {
                    host.requirements.push(ActiveRequirement {
                        required_by: reason.clone(),
                        extension: manifest.id.clone(),
                        version: format!("={}", manifest.version)
                            .parse()
                            .expect("manifest version"),
                    });
                }
            }
            if registry.is_enabled(&manifest.id)
                && !self.installed.active.contains_key(&manifest.id)
                && !self.installed.disabled.contains_key(&manifest.id)
            {
                host.builtins.push(manifest);
            }
        }
        host
    }
    fn grants(&self) -> PermissionGrants {
        self.installed
            .active
            .iter()
            .chain(self.installed.disabled.iter())
            .map(|(id, installed)| (id.clone(), installed.approved_permissions.clone()))
            .collect()
    }
    fn catalog_or_empty(&self) -> Catalog {
        self.catalog.clone().unwrap_or(Catalog {
            schema: marketplace::CATALOG_SCHEMA,
            extensions: vec![],
        })
    }
    pub fn global_access_for(&self, owner: &str) -> bool {
        self.document_scopes.get(owner).copied().unwrap_or(false)
    }
    pub fn document_scope_available(&self) -> bool {
        match &self.preview {
            Some(Preview::Install { .. } | Preview::Folder { .. }) => true,
            Some(Preview::Removal {
                id,
                action: Removal::Enable,
                ..
            }) => self.installed.disabled.contains_key(id),
            _ => false,
        }
    }
    pub fn preview_release(&mut self, release: Release, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.global_document_access = false;
        let Some(catalog) = &self.catalog else {
            self.fail("Refresh a validated catalog before reviewing a release. / Обновите проверенный каталог.".into(), cx);
            return;
        };
        let plan = marketplace::plan_release(
            catalog,
            &release,
            &self.installed.active,
            &self.host(cx),
            &self.grants(),
        );
        self.preview = Some(Preview::Install {
            owner: release.manifest.id.clone(),
            plan: Box::new(plan),
            developer: None,
            generation: self.installed.generation,
        });
        self.error = None;
        cx.notify();
    }
    pub fn preview_url(&mut self, url: &str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let release = self
            .catalog
            .as_ref()
            .and_then(|catalog| {
                catalog
                    .extensions
                    .iter()
                    .flat_map(|entry| &entry.releases)
                    .find(|release| release.asset.url == url)
            })
            .cloned();
        match release { Some(release) => self.preview_release(release, cx), None => self.fail("URL is not an exact reviewed release URL in the current index. Refresh the index or choose a local developer package for explicit source review. / URL отсутствует в проверенном индексе.".into(), cx) }
    }
    pub fn preview_path(&mut self, path: PathBuf, directory: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.global_document_access = false;
        if !directory
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("sonusext"))
        {
            self.fail(
                "Choose a .sonusext package. / Выберите пакет .sonusext.".into(),
                cx,
            );
            return;
        }
        let control = self.begin(
            "Inspecting local source / Проверка локального источника",
            cx,
        );
        self.task = Some(spawn_service(
            cx,
            async move {
                if directory {
                    read_folder(&path).map(|(path, manifest)| LocalPreview::Folder(path, manifest))
                } else {
                    marketplace::preview_developer_package_with_progress(
                        &path,
                        Limits::default(),
                        &control,
                    )
                    .map(LocalPreview::Archive)
                    .map_err(Into::into)
                }
            },
            |this, result: Result<LocalPreview>, cx| {
                if this
                    .control
                    .as_ref()
                    .is_some_and(OperationControl::is_cancelled)
                {
                    this.finish("Inspection cancelled; no changes applied / Проверка отменена; изменений нет", cx);
                    return;
                }
                match result {
                    Ok(LocalPreview::Archive(source)) => {
                        let catalog = this.catalog_or_empty();
                        // A matching file still uses the local bytes and digest approval. Its
                        // developer provenance must not masquerade as a catalog download.
                        let plan = marketplace::plan_developer(
                            &catalog,
                            &source,
                            &this.installed.active,
                            &this.host(cx),
                            &this.grants(),
                        );
                        this.preview = Some(Preview::Install {
                            owner: source.manifest.id.clone(),
                            plan: Box::new(plan),
                            developer: Some(source),
                            generation: this.installed.generation,
                        });
                        this.finish(
                            "Local package ready for review / Пакет готов к проверке",
                            cx,
                        );
                    }
                    Ok(LocalPreview::Folder(directory, manifest)) => {
                        let registry = cx.global::<ContributionRegistry>();
                        let available = registry.manifests();
                        let mut blockers = vec![];
                        if this.installed.active.contains_key(&manifest.id)
                            || this.installed.disabled.contains_key(&manifest.id)
                            || available
                                .iter()
                                .any(|m| m.id == manifest.id && m.runtime == Runtime::Native)
                        {
                            blockers.push("This identity is installed/bundled; use a distinct developer ID or uninstall the managed package first. / ID уже установлен или встроен.".into());
                        }
                        for (id, version) in &manifest.dependencies {
                            if !available.iter().any(|m| {
                                &m.id == id
                                    && version.matches(&m.version)
                                    && registry.is_enabled(id)
                            }) {
                                blockers.push(format!(
                                    "Enable dependency first / Включите зависимость: {id} {version}"
                                ));
                            }
                        }
                        this.preview = Some(Preview::Folder {
                            directory,
                            manifest,
                            blockers,
                        });
                        this.finish(
                            "Developer folder ready for review / Папка готова к проверке",
                            cx,
                        );
                    }
                    Err(error) => this.fail(format!("Local source rejected: {error:#}"), cx),
                }
            },
        ));
    }
    pub fn preview_removal(&mut self, id: String, action: Removal, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.global_document_access = false;
        let mut blockers = vec![];
        let registry = cx.global::<ContributionRegistry>();
        let manifests = registry.manifests();
        let manifest = manifests
            .iter()
            .find(|manifest| manifest.id == id)
            .or_else(|| {
                self.installed
                    .active
                    .get(&id)
                    .or_else(|| self.installed.disabled.get(&id))
                    .map(|installed| &installed.manifest)
            });
        let mut details = Vec::new();
        if let Some(manifest) = manifest {
            manifest_lines(manifest, &mut details);
            if matches!(action, Removal::Uninstall) && manifest.runtime == Runtime::Native {
                blockers.push("Bundled native files cannot be uninstalled. / Встроенные файлы нельзя удалить.".into());
            }
            if matches!(action, Removal::Enable) {
                for (dependency, version) in &manifest.dependencies {
                    if !manifests.iter().any(|candidate| {
                        &candidate.id == dependency
                            && version.matches(&candidate.version)
                            && registry.is_enabled(dependency)
                    }) {
                        blockers.push(format!(
                            "Enable dependency first / Включите зависимость: {dependency} {version}"
                        ));
                    }
                }
            }
        }
        if !matches!(action, Removal::Enable)
            && cx
                .global::<ContributionRegistry>()
                .required_by_active_mode(&id)
        {
            blockers.push("Required by the active mode; close its document first. / Требуется активному режиму; закройте документ.".into());
        }
        if !matches!(action, Removal::Enable) {
            for installed in self
                .installed
                .active
                .values()
                .filter(|installed| installed.manifest.id != id)
            {
                if let Some(version) = installed.manifest.dependencies.get(&id) {
                    blockers.push(format!("{} requires {id} {version}; disable the dependent extension first. / Сначала отключите зависимое расширение.", installed.manifest.id));
                }
            }
            for manifest in manifests
                .iter()
                .filter(|manifest| manifest.id != id && registry.is_enabled(&manifest.id))
            {
                if let Some(version) = manifest.dependencies.get(&id) {
                    blockers.push(format!(
                        "{} requires {id} {version}; disable the dependent extension first.",
                        manifest.id
                    ));
                }
            }
        }
        self.preview = Some(Preview::Removal {
            id,
            action,
            generation: self.installed.generation,
            blockers,
            details,
        });
        cx.notify();
    }
    pub fn preview_rollback(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(previous) = &self.installed.previous else {
            return;
        };
        let mut lines = vec![];
        for installed in previous.active.values() {
            manifest_lines(&installed.manifest, &mut lines);
        }
        for id in previous.disabled.keys() {
            lines.push(format!("Disabled / Отключено: {id}"));
        }
        for id in self
            .installed
            .active
            .keys()
            .chain(self.installed.disabled.keys())
        {
            if !previous.active.contains_key(id) && !previous.disabled.contains_key(id) {
                lines.push(format!("Remove / Удалить: {id}"));
            }
        }
        let host = self.host(cx);
        let blockers = host
            .requirements
            .iter()
            .filter(|requirement| {
                !previous
                    .active
                    .get(&requirement.extension)
                    .is_some_and(|installed| {
                        requirement.version.matches(&installed.manifest.version)
                    })
                    && !host.builtins.iter().any(|manifest| {
                        manifest.id == requirement.extension
                            && requirement.version.matches(&manifest.version)
                    })
            })
            .map(|requirement| {
                format!(
                    "{} requires {} {}",
                    requirement.required_by, requirement.extension, requirement.version
                )
            })
            .collect();
        self.preview = Some(Preview::Rollback {
            generation: self.installed.generation,
            lines,
            blockers,
        });
        cx.notify();
    }
    pub fn approve(&mut self, cx: &mut Context<Self>) {
        if self.busy
            || self
                .preview
                .as_ref()
                .is_none_or(|preview| !preview.can_apply())
        {
            return;
        }
        let global_access = self.global_document_access && self.document_scope_available();
        self.global_document_access = false;
        let preview = self.preview.take().expect("reviewed plan");
        let scope_owner = match &preview {
            Preview::Install { owner, .. } => Some(owner.clone()),
            Preview::Folder { manifest, .. } => Some(manifest.id.clone()),
            Preview::Removal {
                id,
                action: Removal::Enable,
                ..
            } => Some(id.clone()),
            _ => None,
        };
        if let Preview::Removal { id, action, .. } = &preview
            && cx
                .global::<ContributionRegistry>()
                .manifests()
                .iter()
                .any(|manifest| &manifest.id == id && manifest.runtime == Runtime::Native)
        {
            let result = match action {
                Removal::Enable => crate::contributions::enable_extension(id, cx),
                Removal::Disable => crate::contributions::unload_extension(id, cx),
                Removal::Uninstall => Err(anyhow!("Bundled files cannot be uninstalled")),
            };
            match result {
                Ok(()) => self.finish(
                    "Bundled extension state changed / Состояние встроенного расширения изменено",
                    cx,
                ),
                Err(error) => self.fail(error.to_string(), cx),
            }
            return;
        }
        if let Preview::Folder {
            directory,
            manifest,
            ..
        } = preview
        {
            let grants = manifest.permissions.iter().cloned().collect();
            let access = if global_access {
                crate::wasm_host::DocumentAccess::Global
            } else {
                crate::wasm_host::DocumentAccess::DeclaredModes
            };
            if let Err(error) =
                crate::wasm_host::set_document_access(&manifest.id, access, self.shell.clone(), cx)
            {
                self.fail(error.to_string(), cx);
                return;
            }
            match crate::wasm_host::watch_development(
                self.shell.clone(),
                directory.clone(),
                self.root.join("data"),
                manifest.clone(),
                grants,
                cx,
            ) {
                Ok(()) => {
                    self.document_scopes
                        .insert(manifest.id.clone(), global_access);
                    self.developer_folders
                        .insert(manifest.id.clone(), (directory, manifest));
                    self.finish("Developer watcher started; activation/reload status is shown in extension logs. / Наблюдение запущено; результат активации в журнале.", cx);
                }
                Err(error) => {
                    let prior = if self.global_access_for(&manifest.id) {
                        crate::wasm_host::DocumentAccess::Global
                    } else {
                        crate::wasm_host::DocumentAccess::DeclaredModes
                    };
                    let restored = crate::wasm_host::set_document_access(
                        &manifest.id,
                        prior,
                        self.shell.clone(),
                        cx,
                    );
                    self.fail(
                        format!(
                            "Developer activation: {error:#}{}",
                            restored
                                .err()
                                .map(|error| format!("; scope restoration failed: {error}"))
                                .unwrap_or_default()
                        ),
                        cx,
                    );
                }
            }
            return;
        }
        let host = self.host(cx);
        let root = self.root.clone();
        let source = self.index.clone();
        let catalog = self.catalog_or_empty();
        let before = self.installed.clone();
        let mut grants = self.grants();
        let control = self.begin("Preparing transaction / Подготовка транзакции", cx);
        self.task = Some(spawn_service(
            cx,
            async move {
                let store = InstallationStore::open(&root)?;
                let next = match preview {
                    Preview::Install {
                        plan,
                        developer,
                        generation,
                        ..
                    } => {
                        for release in &plan.releases {
                            grants.insert(
                                release.manifest.id.clone(),
                                release.manifest.permissions.iter().cloned().collect(),
                            );
                        }
                        let client =
                            MarketplaceClient::new(&source, root.join("catalog-cache.json"))?;
                        if let Some(preview) = developer {
                            grants.insert(
                                preview.manifest.id.clone(),
                                preview.manifest.permissions.iter().cloned().collect(),
                            );
                            let request = marketplace::DeveloperInstallRequest {
                                approved_sha256: preview.sha256.clone(),
                                preview,
                                dependencies: plan.releases,
                                approved_permissions: grants,
                                expected_generation: generation,
                            };
                            store
                                .install_developer_with_progress(
                                    &client,
                                    &catalog,
                                    &request,
                                    &host,
                                    Limits::default(),
                                    &control,
                                )
                                .await?
                        } else {
                            let request = InstallRequest {
                                releases: plan.releases,
                                approved_permissions: grants,
                                expected_generation: generation,
                            };
                            store
                                .install_with_progress(
                                    &client,
                                    &catalog,
                                    &request,
                                    &host,
                                    Limits::default(),
                                    &control,
                                )
                                .await?
                        }
                    }
                    Preview::Removal {
                        id,
                        action,
                        generation,
                        ..
                    } => {
                        if control.is_cancelled() {
                            bail!("Cancelled before transaction");
                        }
                        match action {
                            Removal::Enable => store.enable_with_progress(
                                &catalog,
                                &id,
                                generation,
                                &host,
                                Limits::default(),
                                &control,
                            )?,
                            Removal::Disable => store.disable(&id, generation, &host)?,
                            Removal::Uninstall => store.uninstall(&id, generation, &host)?,
                        }
                    }
                    Preview::Rollback { generation, .. } => {
                        if control.is_cancelled() {
                            bail!("Cancelled before rollback");
                        }
                        store.rollback_with_progress(
                            &catalog,
                            generation,
                            &host,
                            Limits::default(),
                            &control,
                        )?
                    }
                    Preview::Folder { .. } => unreachable!(),
                };
                match prepare_transition(&store, &before, &next, &host) {
                    Ok(packages) => Ok(Transaction {
                        before,
                        next,
                        packages,
                        host,
                        scope_owner,
                        global_access,
                    }),
                    Err(error) => {
                        let restored =
                            store.restore_failed_activation(next.generation, &before, &host);
                        match restored {
                            Ok(_) => Err(error.context(
                                "Cannot read activation package; prior installation restored",
                            )),
                            Err(recovery) => {
                                Err(anyhow!("{error:#}; RESTORATION FAILED: {recovery}"))
                            }
                        }
                    }
                }
            },
            |this, result: Result<Transaction>, cx| match result {
                Ok(transaction) => this.activate_transaction(transaction, cx),
                Err(error) => {
                    this.reload_state();
                    this.fail(format!("Extension operation failed: {error:#}"), cx);
                }
            },
        ));
    }
    fn reload_state(&mut self) {
        if let Ok(state) = InstallationStore::open(&self.root).and_then(|store| store.state()) {
            self.installed = state;
        }
    }
    pub fn stop_development(&mut self, owner: &str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if cx
            .global::<ContributionRegistry>()
            .required_by_active_mode(owner)
        {
            self.fail(
                "The developer extension is required by the active mode. Close its document first."
                    .into(),
                cx,
            );
            return;
        }
        if !crate::wasm_host::status(cx)
            .iter()
            .any(|status| status.owner == owner)
        {
            crate::wasm_host::stop_watching(owner, cx);
            self.developer_folders.remove(owner);
            self.document_scopes.remove(owner);
            self.finish("Developer watcher stopped / Наблюдение остановлено", cx);
            return;
        }
        match crate::wasm_host::disable(owner, cx) {
            Ok(ticket) => {
                self.developer_folders.remove(owner);
                self.document_scopes.remove(owner);
                self.begin("Stopping developer extension / Остановка расширения", cx);
                self.await_single(ticket, cx);
            }
            Err(error) => self.fail(error.to_string(), cx),
        }
    }
    fn await_single(&mut self, ticket: crate::wasm_host::ActivationTicket, cx: &mut Context<Self>) {
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = wait_activation(&ticket, cx).await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.finish("Extension stopped / Расширение остановлено", cx),
                Err(error) => this.fail(error.to_string(), cx),
            })
            .ok();
        }));
    }
    fn activate_transaction(&mut self, transaction: Transaction, cx: &mut Context<Self>) {
        self.progress_task = None;
        self.status = "Activating runtime and contributions / Активация среды и компонентов".into();
        cx.notify();
        let shell = self.shell.clone();
        let root = self.root.clone();
        let previous_scopes = self.document_scopes.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let Transaction { before, next, packages, host, scope_owner, global_access } = transaction;
            let mut changed = Vec::new();
            let mut error = None;
            for prepared in packages {
                let owner = prepared.package.manifest.id.clone();
                let ticket = this.update(cx, |this, cx| {
                    this.status = format!("Activating / Активация: {owner}"); cx.notify();
                    if scope_owner.as_deref() == Some(owner.as_str()) {
                        let access = if global_access { crate::wasm_host::DocumentAccess::Global } else { crate::wasm_host::DocumentAccess::DeclaredModes };
                        crate::wasm_host::set_document_access(&owner, access, shell.clone(), cx)?;
                    }
                    crate::wasm_host::activate(shell.clone(), prepared.package, root.join("data"), prepared.assets, cx)
                });
                let result = match ticket { Ok(Ok(ticket)) => wait_activation(&ticket, cx).await, Ok(Err(error)) => Err(error), Err(error) => Err(error) };
                changed.push(owner);
                if let Err(failure) = result { error = Some(failure); break; }
            }
            if error.is_none() {
                for owner in before.active.keys().filter(|owner| !next.active.contains_key(*owner)) {
                    let ticket = this.update(cx, |_, cx| {
                        if cx.global::<ContributionRegistry>().is_enabled(owner) { crate::wasm_host::disable(owner, cx).map(Some) } else { Ok(None) }
                    });
                    let result = match ticket { Ok(Ok(Some(ticket))) => wait_activation(&ticket, cx).await, Ok(Ok(None)) => Ok(()), Ok(Err(error)) => Err(error), Err(error) => Err(error) };
                    changed.push(owner.clone());
                    if let Err(failure) = result { error = Some(failure); break; }
                }
            }
            if error.is_none()
                && let Some(owner) = scope_owner.as_ref().filter(|owner| !changed.contains(owner)) {
                    let result = this.update(cx, |_, cx| {
                        let access = if global_access { crate::wasm_host::DocumentAccess::Global } else { crate::wasm_host::DocumentAccess::DeclaredModes };
                        crate::wasm_host::set_document_access(owner, access, shell.clone(), cx)
                    });
                    let result = match result { Ok(result) => result, Err(error) => Err(error) };
                    if let Err(failure) = result { error = Some(failure); }
                }
            if let Some(failure) = error {
                this.update(cx, |this, cx| { this.status = format!("Recovering prior installation / Восстановление: {failure:#}"); cx.notify(); }).ok();
                let recovery_root = root.clone(); let failed = next.generation; let prior = before.clone(); let host = host.clone();
                let restored = cx.background_spawn(async move {
                    let store = InstallationStore::open(&recovery_root)?;
                    let state = store.restore_failed_activation(failed, &prior, &host)?;
                    let packages = prepare_all(&store, &prior, &host)?;
                    Ok::<_, anyhow::Error>((state, packages))
                }).await;
                let mut recovery_errors = vec![];
                match restored {
                    Ok((state, packages)) => {
                        for owner in changed.iter().filter(|owner| !before.active.contains_key(*owner)) {
                            match this.update(cx, |_, cx| crate::wasm_host::disable(owner, cx)) {
                                Ok(Ok(ticket)) => if let Err(error) = wait_activation(&ticket, cx).await { recovery_errors.push(error.to_string()); },
                                Ok(Err(error)) => recovery_errors.push(error.to_string()), Err(error) => recovery_errors.push(error.to_string()),
                            }
                        }
                        for prepared in packages {
                            match this.update(cx, |_, cx| {
                                let owner = &prepared.package.manifest.id;
                                let access = if previous_scopes.get(owner).copied().unwrap_or(false) { crate::wasm_host::DocumentAccess::Global } else { crate::wasm_host::DocumentAccess::DeclaredModes };
                                crate::wasm_host::set_document_access(owner, access, shell.clone(), cx)?;
                                crate::wasm_host::activate(shell.clone(), prepared.package, root.join("data"), prepared.assets, cx)
                            }) {
                                Ok(Ok(ticket)) => if let Err(error) = wait_activation(&ticket, cx).await { recovery_errors.push(error.to_string()); },
                                Ok(Err(error)) => recovery_errors.push(error.to_string()), Err(error) => recovery_errors.push(error.to_string()),
                            }
                        }
                        this.update(cx, |this, _| this.installed = state).ok();
                    }
                    Err(error) => recovery_errors.push(error.to_string()),
                }
                this.update(cx, |this, cx| {
                    let recovery = if recovery_errors.is_empty() { "Prior installation restored / Предыдущая установка восстановлена".into() } else { format!("RECOVERY ERROR / ОШИБКА ВОССТАНОВЛЕНИЯ: {}. Restart after repairing the installation store.", recovery_errors.join("; ")) };
                    this.fail(format!("Activation failed: {failure:#}. {recovery}"), cx);
                }).ok();
            } else {
                let added: Vec<String> = next
                    .active
                    .keys()
                    .filter(|owner| !before.active.contains_key(*owner))
                    .cloned()
                    .collect();
                let cleanup_root = root.clone();
                let cleanup = cx.background_spawn(async move {
                    InstallationStore::open(cleanup_root)?.collect_unreferenced(&before)
                }).await;
                this.update(cx, |this, cx| {
                    this.installed = next;
                    if let Some(owner) = scope_owner { this.document_scopes.insert(owner, global_access); }
                    this.document_scopes.retain(|owner, _| this.installed.active.contains_key(owner) || this.developer_folders.contains_key(owner));
                    this.finish("Extension transaction completed and runtime activated / Транзакция завершена, расширения активированы", cx);
                    if let Err(error) = cleanup {
                        this.error = Some(format!("Activation succeeded; unused package cleanup failed: {error}. Referenced packages were preserved."));
                        this.log(format!("Cleanup: {error}"));
                    }
                    if !added.is_empty() {
                        cx.emit(ExtensionsEvent::Activated(added));
                    }
                }).ok();
            }
        }));
    }
}
impl Drop for ExtensionsModel {
    fn drop(&mut self) {
        if let Some(control) = &self.control {
            control.cancel();
        }
    }
}

enum LocalPreview {
    Archive(DeveloperPreview),
    Folder(PathBuf, ExtensionManifest),
}
struct Prepared {
    package: sonus_wasm::Package,
    assets: BTreeMap<String, PathBuf>,
}
struct Transaction {
    before: InstallationState,
    next: InstallationState,
    packages: Vec<Prepared>,
    host: HostState,
    scope_owner: Option<String>,
    global_access: bool,
}
fn read_folder(path: &Path) -> Result<(PathBuf, ExtensionManifest)> {
    let directory = path.canonicalize()?;
    let manifest_path = directory.join("manifest.json");
    anyhow::ensure!(
        !std::fs::symlink_metadata(&manifest_path)?
            .file_type()
            .is_symlink(),
        "manifest symlinks are forbidden"
    );
    let file = std::fs::File::open(manifest_path)?;
    let mut bytes = Vec::new();
    file.take(Limits::default().manifest_bytes + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= Limits::default().manifest_bytes,
        "manifest too large"
    );
    let manifest: ExtensionManifest = serde_json::from_slice(&bytes)?;
    manifest.validate()?;
    manifest.check_host_compatibility()?;
    anyhow::ensure!(
        manifest.runtime == Runtime::Wasm,
        "developer folders must contain sandboxed WASM, never native DLLs"
    );
    Ok((directory, manifest))
}
fn prepare_transition(
    store: &InstallationStore,
    before: &InstallationState,
    next: &InstallationState,
    host: &HostState,
) -> Result<Vec<Prepared>> {
    let changed: BTreeSet<_> = next
        .active
        .iter()
        .filter(|(id, installed)| before.active.get(*id) != Some(*installed))
        .map(|(id, _)| id.clone())
        .collect();
    prepare_selected(store, next, host, &changed)
}
fn prepare_all(
    store: &InstallationStore,
    state: &InstallationState,
    host: &HostState,
) -> Result<Vec<Prepared>> {
    prepare_selected(store, state, host, &state.active.keys().cloned().collect())
}
fn prepare_selected(
    store: &InstallationStore,
    state: &InstallationState,
    host: &HostState,
    selected: &BTreeSet<String>,
) -> Result<Vec<Prepared>> {
    let mut remaining = selected.clone();
    let mut packages = vec![];
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .find(|id| {
                state.active[*id]
                    .manifest
                    .dependencies
                    .keys()
                    .all(|dependency| !remaining.contains(dependency))
            })
            .cloned()
            .ok_or_else(|| anyhow!("cyclic extension dependency set"))?;
        let installed = &state.active[&ready];
        let path = store.module_path(installed)?;
        let file = std::fs::File::open(&path)?;
        let max = sonus_wasm::Limits::default().component_bytes;
        let mut component = Vec::new();
        file.take(max as u64 + 1).read_to_end(&mut component)?;
        anyhow::ensure!(
            component.len() <= max,
            "component exceeds runtime byte limit"
        );
        let mut assets = BTreeMap::new();
        let assets_root = path.parent().context("module parent")?.join("assets");
        if assets_root.exists() {
            collect_assets(&assets_root, &assets_root, &mut assets)?;
        }
        packages.push(Prepared {
            package: sonus_wasm::Package {
                manifest: installed.manifest.clone(),
                component,
                grants: installed.approved_permissions.clone(),
                required_by_open_modes: host
                    .requirements
                    .iter()
                    .filter(|r| r.extension == ready)
                    .map(|r| r.version.clone())
                    .collect(),
            },
            assets,
        });
        remaining.remove(&ready);
    }
    Ok(packages)
}
fn collect_assets(
    root: &Path,
    directory: &Path,
    assets: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    anyhow::ensure!(
        !std::fs::symlink_metadata(directory)?
            .file_type()
            .is_symlink(),
        "asset directory symlink rejected"
    );
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        anyhow::ensure!(!kind.is_symlink(), "asset symlink rejected");
        if kind.is_dir() {
            collect_assets(root, &entry.path(), assets)?;
        } else if kind.is_file() {
            anyhow::ensure!(assets.len() < Limits::default().files, "too many assets");
            let path = entry.path();
            let key = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            assets.insert(key, path);
        } else {
            bail!("special asset file rejected");
        }
    }
    Ok(())
}
async fn wait_activation(
    ticket: &crate::wasm_host::ActivationTicket,
    cx: &mut AsyncApp,
) -> Result<()> {
    loop {
        match ticket.try_recv() {
            Ok(Some(result)) => return result,
            Ok(None) => {}
            Err(error) => return Err(error.into()),
        }
        cx.background_executor()
            .timer(Duration::from_millis(16))
            .await;
    }
}
