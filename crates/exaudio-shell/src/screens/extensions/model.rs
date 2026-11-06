use crate::{
    ShellHandle,
    contributions::ContributionRegistry,
    screens::ui::localized,
    services::{AppServices, spawn_service},
};
use anyhow::{Context as _, Result, anyhow, bail};
use exaudio_extension::{ExtensionManifest, Permission, Runtime};
use exaudio_marketplace::{
    self as marketplace, ActiveRequirement, Catalog, DeveloperPreview, HostState, InstallRequest,
    InstallationState, InstallationStore, Limits, MarketplaceClient, OperationControl,
    PermissionGrants, PlanDiff, Release,
};
use gpui_kit::*;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const DEFAULT_INDEX: &str =
    "https://raw.githubusercontent.com/Nominy/exaudio-extensions/main/index.json";
const INDEX_KEY: &str = "exaudio.marketplace.index";
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        name: String,
        version: String,
        action: Removal,
        generation: u64,
        blockers: Vec<String>,
    },
    Rollback {
        generation: u64,
        /// Human lines describing the previous set (`{name} {version}`, `Remove {name}`).
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
    /// Everything that stops the action, as plain sentences.
    pub fn blockers(&self) -> &[String] {
        match self {
            Self::Install { plan, .. } => &plan.blockers,
            Self::Folder { blockers, .. }
            | Self::Removal { blockers, .. }
            | Self::Rollback { blockers, .. } => blockers,
        }
    }
}
/// What a permission lets the extension do, as the tail of "This extension can:".
pub(super) fn permission_label(permission: &Permission, cx: &App) -> String {
    use crate::screens::ui::localized;
    match permission {
        Permission::DocumentRead => {
            localized(cx, "read your transcript", "читать расшифровку").into()
        }
        Permission::DocumentEdit => {
            localized(cx, "edit your transcript", "изменять расшифровку").into()
        }
        Permission::AudioRead => localized(cx, "read the audio", "читать аудио").into(),
        Permission::TransportControl => {
            localized(cx, "control playback", "управлять воспроизведением").into()
        }
        Permission::SelectionControl => {
            localized(cx, "change the selection", "менять выделение").into()
        }
        Permission::FilePicker => {
            localized(cx, "ask you to pick files", "запрашивать выбор файлов").into()
        }
        Permission::Storage => {
            localized(cx, "keep its own settings", "хранить свои настройки").into()
        }
        Permission::Secrets => {
            localized(cx, "store keys and passwords", "хранить ключи и пароли").into()
        }
        Permission::Network(host) => {
            localized(cx, "connect to {host}", "подключаться к {host}").replace("{host}", host)
        }
    }
}

/// Display name of an extension id: the registry or catalog name, else the id itself.
pub(super) fn extension_name(id: &str, catalog: Option<&Catalog>, cx: &App) -> String {
    if let Some(manifest) = cx
        .global::<ContributionRegistry>()
        .manifests()
        .into_iter()
        .find(|manifest| manifest.id == id)
    {
        return manifest.name;
    }
    catalog
        .and_then(|catalog| catalog.extensions.iter().find(|entry| entry.id == id))
        .and_then(|entry| {
            entry
                .releases
                .iter()
                .max_by(|a, b| a.version.cmp(&b.version))
        })
        .map(|release| release.manifest.name.clone())
        .unwrap_or_else(|| id.to_owned())
}

/// One failed operation: a plain headline and the technical text behind "Details".
#[derive(Clone, Debug)]
pub struct Failure {
    pub headline: String,
    pub detail: Option<String>,
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
    pub error: Option<Failure>,
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
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        self.logs.push_back(format!(
            "{:02}:{:02}:{:02}  {}",
            now.hour(),
            now.minute(),
            now.second(),
            value.into()
        ));
    }
    /// Ends the current operation with a plain `headline`; `detail` is the technical text.
    pub fn fail(&mut self, headline: String, detail: Option<String>, cx: &mut Context<Self>) {
        match &detail {
            Some(detail) => self.log(format!("{headline} — {detail}")),
            None => self.log(&headline),
        }
        self.error = Some(Failure { headline, detail });
        self.busy = false;
        self.control = None;
        self.progress_task = None;
        cx.notify();
    }
    /// [`fail`](Self::fail) with "Something went wrong." as the headline.
    fn fail_with(&mut self, error: impl std::fmt::Display, cx: &mut Context<Self>) {
        let headline = localized(cx, "Something went wrong.", "Что-то пошло не так.").to_owned();
        self.fail(headline, Some(error.to_string()), cx);
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
                        let name = progress
                            .extension
                            .as_deref()
                            .map(|id| extension_name(id, this.catalog.as_ref(), cx))
                            .unwrap_or_else(|| localized(cx, "catalog", "каталог").to_owned());
                        this.status = match progress.stage {
                            marketplace::Stage::Downloading => {
                                let mb = |bytes: u64| format!("{:.1}", bytes as f64 / 1_048_576.0);
                                match progress.total {
                                    Some(total) => localized(
                                        cx,
                                        "Downloading {name} — {done} of {total} MB",
                                        "Загрузка {name} — {done} из {total} МБ",
                                    )
                                    .replace("{total}", &mb(total)),
                                    None => localized(
                                        cx,
                                        "Downloading {name} — {done} MB",
                                        "Загрузка {name} — {done} МБ",
                                    )
                                    .to_owned(),
                                }
                                .replace("{name}", &name)
                                .replace("{done}", &mb(progress.completed))
                            }
                            marketplace::Stage::Extracting => {
                                localized(cx, "Unpacking…", "Распаковка…").to_owned()
                            }
                            marketplace::Stage::Preparing => {
                                localized(cx, "Verifying…", "Проверка…").to_owned()
                            }
                            marketplace::Stage::Activating => {
                                localized(cx, "Starting {name}…", "Запуск {name}…")
                                    .replace("{name}", &name)
                            }
                            marketplace::Stage::Completed => {
                                localized(cx, "Finishing…", "Завершение…").to_owned()
                            }
                            marketplace::Stage::Cancelled => {
                                localized(cx, "Cancelling…", "Отмена…").to_owned()
                            }
                        };
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
        if !status.is_empty() {
            self.log(status);
        }
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
        self.status = localized(cx, "Cancelling…", "Отмена…").into();
        cx.notify();
    }
    pub fn set_index(&mut self, source: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Err(error) = MarketplaceClient::new(&source, self.root.join("catalog-cache.json")) {
            self.fail(
                localized(
                    cx,
                    "This catalog address can't be used.",
                    "Этот адрес каталога нельзя использовать.",
                )
                .into(),
                Some(error.to_string()),
                cx,
            );
            return;
        }
        if let Err(error) = cx
            .global_mut::<AppServices>()
            .storage
            .set_value(INDEX_KEY, source.clone())
        {
            self.fail(
                localized(
                    cx,
                    "Couldn't save this setting.",
                    "Не удалось сохранить настройку.",
                )
                .into(),
                Some(error.to_string()),
                cx,
            );
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
            localized(cx, "Refreshing catalog…", "Обновление каталога…"),
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
                        this.error = snapshot.refresh_error.map(|error| Failure {
                            headline: localized(
                                cx,
                                "Couldn't reach the catalog.",
                                "Не удалось открыть каталог.",
                            )
                            .into(),
                            detail: Some(error),
                        });
                    }
                    Err(error) => {
                        this.from_cache = this.catalog.is_some();
                        this.error = Some(Failure {
                            headline: localized(
                                cx,
                                "Couldn't reach the catalog.",
                                "Не удалось открыть каталог.",
                            )
                            .into(),
                            detail: Some(format!("{}: {error}", this.index)),
                        });
                    }
                }
                if let Err(error) = state {
                    this.error = Some(Failure {
                        headline: localized(
                            cx,
                            "Extension settings couldn't be read. Restart Exaudio before changing extensions.",
                            "Не удалось прочитать настройки расширений. Перезапустите Exaudio.",
                        )
                        .into(),
                        detail: Some(error.to_string()),
                    });
                }
                this.finish(localized(cx, "Catalog up to date", "Каталог обновлён"), cx);
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
            localized(cx, "Starting extensions…", "Запуск расширений…"),
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
                        Err(error) => this.fail(
                            localized(
                                cx,
                                "Some extensions couldn't start.",
                                "Некоторые расширения не запустились.",
                            )
                            .into(),
                            Some(format!("{error:#}")),
                            cx,
                        ),
                    }
                }
                Err(error) => this.fail(
                    localized(
                        cx,
                        "Some extensions couldn't start.",
                        "Некоторые расширения не запустились.",
                    )
                    .into(),
                    Some(format!("{error:#}")),
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
                    if this
                        .control
                        .as_ref()
                        .is_some_and(OperationControl::is_cancelled)
                    {
                        bail!("startup activation cancelled");
                    }
                    let name = extension_name(&owner, this.catalog.as_ref(), cx);
                    this.status = localized(
                        cx,
                        "Starting {name} ({i} of {n})…",
                        "Запуск {name} ({i} из {n})…",
                    )
                    .replace("{name}", &name)
                    .replace("{i}", &(index + 1).to_string())
                    .replace("{n}", &total.to_string());
                    cx.notify();
                    crate::wasm_host::activate(
                        shell.clone(),
                        prepared.package,
                        root.join("data"),
                        prepared.assets,
                        cx,
                    )
                });
                let result = match ticket {
                    Ok(Ok(ticket)) => wait_activation(&ticket, cx).await,
                    Ok(Err(error)) => Err(error),
                    Err(error) => Err(error),
                };
                if !already_enabled.contains(&owner) {
                    added.push(owner);
                }
                if let Err(error) = result {
                    failure = Some(error);
                    break;
                }
            }
            if let Some(error) = failure {
                let mut cleanup_errors = Vec::new();
                for owner in added.into_iter().rev() {
                    match this.update(cx, |_, cx| crate::wasm_host::disable(&owner, cx)) {
                        Ok(Ok(ticket)) => {
                            if let Err(error) = wait_activation(&ticket, cx).await {
                                cleanup_errors.push(error.to_string());
                            }
                        }
                        Ok(Err(error)) => cleanup_errors.push(error.to_string()),
                        Err(error) => cleanup_errors.push(error.to_string()),
                    }
                }
                this.update(cx, |this, cx| {
                    let mut detail = format!("{error:#}");
                    if !cleanup_errors.is_empty() {
                        detail.push_str("\ncleanup: ");
                        detail.push_str(&cleanup_errors.join("; "));
                    }
                    this.fail(
                        localized(
                            cx,
                            "Some extensions couldn't start.",
                            "Некоторые расширения не запустились.",
                        )
                        .into(),
                        Some(detail),
                        cx,
                    )
                })
                .ok();
            } else {
                this.update(cx, |this, cx| this.finish("", cx)).ok();
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
            self.fail(
                localized(
                    cx,
                    "Refresh the catalog first.",
                    "Сначала обновите каталог.",
                )
                .into(),
                None,
                cx,
            );
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
        match release {
            Some(release) => self.preview_release(release, cx),
            None => self.fail(
                localized(
                    cx,
                    "This link isn't in the catalog. To install anyway, download the file and use “Install from file”.",
                    "Этой ссылки нет в каталоге. Чтобы установить всё равно, скачайте файл и используйте «Установить из файла».",
                )
                .into(),
                None,
                cx,
            ),
        }
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
                .is_none_or(|extension| {
                    !extension.eq_ignore_ascii_case(exaudio_marketplace::PACKAGE_EXTENSION)
                })
        {
            self.fail(
                localized(cx, "Choose an .exaudio file.", "Выберите файл .exaudio.").into(),
                None,
                cx,
            );
            return;
        }
        let control = self.begin(localized(cx, "Checking file…", "Проверка файла…"), cx);
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
                    this.finish(
                        localized(
                            cx,
                            "Cancelled. Nothing changed.",
                            "Отменено. Ничего не изменилось.",
                        ),
                        cx,
                    );
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
                        this.finish("", cx);
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
                            blockers.push(
                                localized(
                                    cx,
                                    "An extension with this ID is already installed.",
                                    "Расширение с таким ID уже установлено.",
                                )
                                .into(),
                            );
                        }
                        for (id, version) in &manifest.dependencies {
                            if !available.iter().any(|m| {
                                &m.id == id
                                    && version.matches(&m.version)
                                    && registry.is_enabled(id)
                            }) {
                                let name = extension_name(id, this.catalog.as_ref(), cx);
                                blockers.push(
                                    localized(
                                        cx,
                                        "Turn on {name} first.",
                                        "Сначала включите {name}.",
                                    )
                                    .replace("{name}", &format!("{name} {version}")),
                                );
                            }
                        }
                        this.preview = Some(Preview::Folder {
                            directory,
                            manifest,
                            blockers,
                        });
                        this.finish("", cx);
                    }
                    Err(error) => this.fail(
                        localized(
                            cx,
                            "This file can't be installed.",
                            "Этот файл нельзя установить.",
                        )
                        .into(),
                        Some(format!("{error:#}")),
                        cx,
                    ),
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
        let (name, version) = manifest
            .map(|manifest| (manifest.name.clone(), manifest.version.to_string()))
            .unwrap_or_else(|| (id.clone(), String::new()));
        if let Some(manifest) = manifest {
            if matches!(action, Removal::Uninstall) && manifest.runtime == Runtime::Native {
                blockers.push(
                    localized(
                        cx,
                        "Built-in extensions can be turned off but not removed.",
                        "Встроенные расширения можно выключить, но не удалить.",
                    )
                    .into(),
                );
            }
            if matches!(action, Removal::Enable) {
                for (dependency, version) in &manifest.dependencies {
                    if !manifests.iter().any(|candidate| {
                        &candidate.id == dependency
                            && version.matches(&candidate.version)
                            && registry.is_enabled(dependency)
                    }) {
                        let dependency_name = extension_name(dependency, self.catalog.as_ref(), cx);
                        blockers.push(
                            localized(cx, "Turn on {name} first.", "Сначала включите {name}.")
                                .replace("{name}", &format!("{dependency_name} {version}")),
                        );
                    }
                }
            }
        }
        if !matches!(action, Removal::Enable)
            && cx
                .global::<ContributionRegistry>()
                .required_by_active_mode(&id)
        {
            blockers.push(
                localized(
                    cx,
                    "The open project needs it. Close the project first.",
                    "Оно нужно открытому проекту. Сначала закройте проект.",
                )
                .into(),
            );
        }
        if !matches!(action, Removal::Enable) {
            let needs_it = localized(
                cx,
                "{a} needs it. Turn off {a} first.",
                "Оно нужно для «{a}». Сначала выключите «{a}».",
            );
            let dependents = self
                .installed
                .active
                .values()
                .map(|installed| &installed.manifest)
                .filter(|manifest| manifest.id != id)
                .chain(
                    manifests
                        .iter()
                        .filter(|manifest| manifest.id != id && registry.is_enabled(&manifest.id)),
                );
            for dependent in dependents {
                if dependent.dependencies.contains_key(&id) {
                    let line = needs_it.replace("{a}", &dependent.name);
                    if !blockers.contains(&line) {
                        blockers.push(line);
                    }
                }
            }
        }
        self.preview = Some(Preview::Removal {
            id,
            name,
            version,
            action,
            generation: self.installed.generation,
            blockers,
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
            lines.push(format!(
                "{} {}",
                installed.manifest.name, installed.manifest.version
            ));
        }
        for id in previous.disabled.keys() {
            lines.push(
                localized(cx, "{name} (off)", "{name} (выключено)")
                    .replace("{name}", &extension_name(id, self.catalog.as_ref(), cx)),
            );
        }
        for id in self
            .installed
            .active
            .keys()
            .chain(self.installed.disabled.keys())
        {
            if !previous.active.contains_key(id) && !previous.disabled.contains_key(id) {
                lines.push(
                    localized(cx, "Remove {name}", "Удалить {name}")
                        .replace("{name}", &extension_name(id, self.catalog.as_ref(), cx)),
                );
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
                localized(cx, "{a} needs {b} {v}.", "Для «{a}» нужно {b} {v}.")
                    .replace(
                        "{a}",
                        &extension_name(&requirement.required_by, self.catalog.as_ref(), cx),
                    )
                    .replace(
                        "{b}",
                        &extension_name(&requirement.extension, self.catalog.as_ref(), cx),
                    )
                    .replace("{v}", &requirement.version.to_string())
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
        if let Preview::Removal {
            id, name, action, ..
        } = &preview
            && cx
                .global::<ContributionRegistry>()
                .manifests()
                .iter()
                .any(|manifest| &manifest.id == id && manifest.runtime == Runtime::Native)
        {
            let result = match action {
                Removal::Enable => crate::contributions::enable_extension(id, cx),
                Removal::Disable => crate::contributions::unload_extension(id, cx),
                Removal::Uninstall => Err(anyhow!("built-in extensions cannot be removed")),
            };
            match result {
                Ok(()) => {
                    let status = if matches!(action, Removal::Enable) {
                        localized(cx, "{name} turned on", "{name} включено")
                    } else {
                        localized(cx, "{name} turned off", "{name} выключено")
                    }
                    .replace("{name}", name);
                    self.finish(&status, cx)
                }
                Err(error) => self.fail(
                    localized(
                        cx,
                        "This extension can't start.",
                        "Это расширение не может запуститься.",
                    )
                    .into(),
                    Some(format!("{error:#}")),
                    cx,
                ),
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
                self.fail_with(format!("{error:#}"), cx);
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
                    let folder = directory
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| directory.display().to_string());
                    let status = localized(cx, "Watching {folder}", "Слежу за {folder}")
                        .replace("{folder}", &folder);
                    self.developer_folders
                        .insert(manifest.id.clone(), (directory, manifest));
                    self.finish(&status, cx);
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
                    let mut detail = format!("{error:#}");
                    if let Err(error) = restored {
                        detail.push_str(&format!("; scope restoration failed: {error}"));
                    }
                    self.fail(
                        localized(
                            cx,
                            "This extension can't start.",
                            "Это расширение не может запуститься.",
                        )
                        .into(),
                        Some(detail),
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
        let control = self.begin(localized(cx, "Installing…", "Установка…"), cx);
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
                                "cannot read activation package; prior installation restored",
                            )),
                            Err(recovery) => {
                                Err(anyhow!("{error:#}; restoration failed: {recovery}"))
                            }
                        }
                    }
                }
            },
            |this, result: Result<Transaction>, cx| match result {
                Ok(transaction) => this.activate_transaction(transaction, cx),
                Err(error) => {
                    this.reload_state();
                    let cancelled = error.to_string().starts_with("Cancelled before");
                    if cancelled {
                        this.finish(
                            localized(
                                cx,
                                "Cancelled. Nothing changed.",
                                "Отменено. Ничего не изменилось.",
                            ),
                            cx,
                        );
                    } else {
                        this.fail_with(format!("{error:#}"), cx);
                    }
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
                localized(
                    cx,
                    "The open project needs it. Close the project first.",
                    "Оно нужно открытому проекту. Сначала закройте проект.",
                )
                .into(),
                None,
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
            self.finish(
                localized(cx, "Stopped watching", "Слежение остановлено"),
                cx,
            );
            return;
        }
        match crate::wasm_host::disable(owner, cx) {
            Ok(ticket) => {
                self.developer_folders.remove(owner);
                self.document_scopes.remove(owner);
                self.begin(localized(cx, "Stopping…", "Остановка…"), cx);
                self.await_single(ticket, cx);
            }
            Err(error) => self.fail_with(format!("{error:#}"), cx),
        }
    }
    fn await_single(&mut self, ticket: crate::wasm_host::ActivationTicket, cx: &mut Context<Self>) {
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = wait_activation(&ticket, cx).await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.finish(localized(cx, "Stopped", "Остановлено"), cx),
                Err(error) => this.fail_with(format!("{error:#}"), cx),
            })
            .ok();
        }));
    }
    fn activate_transaction(&mut self, transaction: Transaction, cx: &mut Context<Self>) {
        self.progress_task = None;
        self.status = localized(cx, "Starting…", "Запуск…").into();
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
                    let name = extension_name(&owner, this.catalog.as_ref(), cx);
                    this.status = localized(cx, "Starting {name}…", "Запуск {name}…").replace("{name}", &name);
                    cx.notify();
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
                this.update(cx, |this, cx| {
                    this.status = localized(cx, "Something went wrong; restoring the previous state…", "Произошла ошибка; возвращаю предыдущее состояние…").into();
                    cx.notify();
                }).ok();
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
                    let (headline, status) = if recovery_errors.is_empty() {
                        (
                            localized(cx, "Something went wrong.", "Что-то пошло не так."),
                            localized(cx, "Previous state restored", "Предыдущее состояние восстановлено"),
                        )
                    } else {
                        (
                            localized(
                                cx,
                                "Couldn't restore the previous state. Restart Exaudio.",
                                "Не удалось восстановить предыдущее состояние. Перезапустите Exaudio.",
                            ),
                            "",
                        )
                    };
                    let mut detail = format!("{failure:#}");
                    if !recovery_errors.is_empty() {
                        detail.push_str("\nrecovery: ");
                        detail.push_str(&recovery_errors.join("; "));
                    }
                    this.fail(headline.into(), Some(detail), cx);
                    this.status = status.into();
                }).ok();
            } else {
                let added: Vec<String> = next
                    .active
                    .keys()
                    .filter(|owner| !before.active.contains_key(*owner))
                    .cloned()
                    .collect();
                let changed_ids: Vec<String> = next
                    .active
                    .iter()
                    .filter(|(owner, installed)| before.active.get(*owner) != Some(*installed))
                    .map(|(owner, _)| owner.clone())
                    .collect();
                let cleanup_root = root.clone();
                let cleanup = cx.background_spawn(async move {
                    InstallationStore::open(cleanup_root)?.collect_unreferenced(&before)
                }).await;
                this.update(cx, |this, cx| {
                    let installed = changed_ids
                        .iter()
                        .map(|owner| extension_name(owner, this.catalog.as_ref(), cx))
                        .collect::<Vec<_>>();
                    let status = if installed.is_empty() {
                        localized(cx, "Done", "Готово").to_owned()
                    } else {
                        localized(cx, "{name} installed", "{name} установлено").replace("{name}", &installed.join(", "))
                    };
                    this.installed = next;
                    if let Some(owner) = scope_owner { this.document_scopes.insert(owner, global_access); }
                    this.document_scopes.retain(|owner, _| this.installed.active.contains_key(owner) || this.developer_folders.contains_key(owner));
                    this.finish(&status, cx);
                    if let Err(error) = cleanup {
                        this.log(format!("cleanup: {error}"));
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
    package: exaudio_wasm::Package,
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
        let max = exaudio_wasm::Limits::default().component_bytes;
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
            package: exaudio_wasm::Package {
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
