use crate::{
    Catalog, DeveloperInstallRequest, Error, HostState, Limits, MarketplaceClient,
    OperationControl, PermissionGrants, Release, Result, Stage,
    client::atomic_json,
    developer::read_package,
    invalid,
    package::{PackageMetadata, extract_contents},
    plan::dependency_blockers,
};
use exaudio_extension::{ExtensionManifest, Permission};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InstallProvenance {
    Reviewed {
        /// Immutable release data is shared across staged/rollback snapshots.
        release: Arc<Release>,
    },
    /// Explicitly approved local bytes, never described as reviewed or authenticated.
    Developer { source: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    pub manifest: ExtensionManifest,
    pub bytes: u64,
    pub sha256: String,
    pub provenance: InstallProvenance,
    /// Opaque basename under packages/, never an external path.
    pub directory: String,
    pub approved_permissions: BTreeSet<Permission>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationSnapshot {
    pub active: BTreeMap<String, Installed>,
    pub disabled: BTreeMap<String, Installed>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationState {
    pub generation: u64,
    pub active: BTreeMap<String, Installed>,
    pub disabled: BTreeMap<String, Installed>,
    pub previous: Option<InstallationSnapshot>,
}

impl InstallationState {
    fn snapshot(&self) -> InstallationSnapshot {
        InstallationSnapshot {
            active: self.active.clone(),
            disabled: self.disabled.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub releases: Vec<Release>,
    pub approved_permissions: PermissionGrants,
    pub expected_generation: u64,
}

struct PreparedInstall<'a> {
    packages: Vec<(Installed, &'a [u8])>,
    expected_generation: u64,
}

pub struct InstallationStore {
    root: PathBuf,
}

impl InstallationStore {
    /// The directory must be private to the OS user. Filesystem ownership is the trust
    /// boundary against another local process replacing package files or consent state.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("packages"))?;
        if std::fs::symlink_metadata(root.join("packages"))?
            .file_type()
            .is_symlink()
        {
            return Err(invalid("packages directory may not be a symlink"));
        }
        Ok(Self { root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state(&self) -> Result<InstallationState> {
        let file = match File::open(self.root.join("active.json")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(InstallationState::default());
            }
            Err(error) => return Err(error.into()),
        };
        if file.metadata()?.len() > 16 * 1024 * 1024 {
            return Err(invalid("oversized activation pointer"));
        }
        let state: InstallationState = serde_json::from_reader(file)?;
        validate_inventory(&state.active, &state.disabled)?;
        if let Some(previous) = &state.previous {
            validate_inventory(&previous.active, &previous.disabled)?;
        }
        Ok(state)
    }

    pub fn module_path(&self, installed: &Installed) -> Result<PathBuf> {
        Ok(self
            .package_directory(installed)?
            .join("content/module.wasm"))
    }

    /// Offline startup gate. The host supplies only enabled/available built-ins.
    /// Verifies archive and extracted bytes, not just the mutable activation pointer.
    pub fn validate_current(&self, host: &HostState, limits: Limits) -> Result<InstallationState> {
        self.validate_current_with_progress(host, limits, &OperationControl::default())
    }

    pub fn validate_current_with_progress(
        &self,
        host: &HostState,
        limits: Limits,
        control: &OperationControl,
    ) -> Result<InstallationState> {
        let _lock = self.lock()?;
        let state = self.state()?;
        check_set(&state.active, host)?;
        for (index, installed) in state.active.values().enumerate() {
            control.check()?;
            control.emit(
                Stage::Preparing,
                Some(&installed.manifest.id),
                index as u64,
                Some(state.active.len() as u64),
            );
            let directory = self.package_directory(installed)?;
            let bytes = read_package(&directory.join("archive.exaudio"), limits)?;
            let temporary = tempfile::tempdir()?;
            extract_contents(
                &bytes,
                PackageMetadata {
                    manifest: &installed.manifest,
                    bytes: installed.bytes,
                    sha256: &installed.sha256,
                },
                limits,
                temporary.path(),
                control,
            )?;
            compare_tree(temporary.path(), &directory.join("content"), control)?;
        }
        control.emit(
            Stage::Completed,
            None,
            state.active.len() as u64,
            Some(state.active.len() as u64),
        );
        Ok(state)
    }

    pub async fn install(
        &self,
        client: &MarketplaceClient,
        catalog: &Catalog,
        request: &InstallRequest,
        host: &HostState,
        limits: Limits,
    ) -> Result<InstallationState> {
        self.install_with_progress(
            client,
            catalog,
            request,
            host,
            limits,
            &OperationControl::default(),
        )
        .await
    }

    pub async fn install_with_progress(
        &self,
        client: &MarketplaceClient,
        catalog: &Catalog,
        request: &InstallRequest,
        host: &HostState,
        limits: Limits,
        control: &OperationControl,
    ) -> Result<InstallationState> {
        validate_selection(catalog, &request.releases)?;
        let packages = download_all(client, &request.releases, limits, control).await?;
        self.install_packages_with_progress(catalog, request, &packages, host, limits, control)
    }

    pub fn install_packages(
        &self,
        catalog: &Catalog,
        request: &InstallRequest,
        packages: &[Vec<u8>],
        host: &HostState,
        limits: Limits,
    ) -> Result<InstallationState> {
        self.install_packages_with_progress(
            catalog,
            request,
            packages,
            host,
            limits,
            &OperationControl::default(),
        )
    }

    pub fn install_packages_with_progress(
        &self,
        catalog: &Catalog,
        request: &InstallRequest,
        packages: &[Vec<u8>],
        host: &HostState,
        limits: Limits,
        control: &OperationControl,
    ) -> Result<InstallationState> {
        validate_selection(catalog, &request.releases)?;
        let prepared = PreparedInstall {
            packages: reviewed_records(&request.releases, packages, &request.approved_permissions)?,
            expected_generation: request.expected_generation,
        };
        self.apply(prepared, host, limits, control)
    }

    /// Atomically installs an explicitly approved developer archive and all selected
    /// reviewed dependencies. Source approval is distinct from capability approval.
    pub async fn install_developer_with_progress(
        &self,
        client: &MarketplaceClient,
        catalog: &Catalog,
        request: &DeveloperInstallRequest,
        host: &HostState,
        limits: Limits,
        control: &OperationControl,
    ) -> Result<InstallationState> {
        validate_selection(catalog, &request.dependencies)?;
        approve_developer(request)?;
        let packages = download_all(client, &request.dependencies, limits, control).await?;
        self.install_developer_packages(catalog, request, &packages, host, limits, control)
    }

    pub fn install_developer_packages(
        &self,
        catalog: &Catalog,
        request: &DeveloperInstallRequest,
        dependencies: &[Vec<u8>],
        host: &HostState,
        limits: Limits,
        control: &OperationControl,
    ) -> Result<InstallationState> {
        validate_selection(catalog, &request.dependencies)?;
        approve_developer(request)?;
        control.check()?;
        let bytes = read_package(&request.preview.source, limits)?;
        let mut packages = reviewed_records(
            &request.dependencies,
            dependencies,
            &request.approved_permissions,
        )?;
        let preview = &request.preview;
        packages.push((
            Installed {
                manifest: preview.manifest.clone(),
                bytes: preview.bytes,
                sha256: preview.sha256.clone(),
                provenance: InstallProvenance::Developer {
                    source: preview.source.clone(),
                },
                directory: String::new(),
                approved_permissions: approved(&preview.manifest, &request.approved_permissions)?,
            },
            &bytes,
        ));
        self.apply(
            PreparedInstall {
                packages,
                expected_generation: request.expected_generation,
            },
            host,
            limits,
            control,
        )
    }

    fn apply(
        &self,
        prepared: PreparedInstall<'_>,
        host: &HostState,
        limits: Limits,
        control: &OperationControl,
    ) -> Result<InstallationState> {
        control.check()?;
        if prepared.packages.is_empty() {
            return Err(invalid("transaction contains no packages"));
        }
        let _lock = self.lock()?;
        let old = self.expected_state(prepared.expected_generation)?;
        let mut next = old.snapshot();
        let mut identities = BTreeSet::new();
        let mut staged = Vec::new();
        for (mut installed, bytes) in prepared.packages {
            if !identities.insert(installed.manifest.id.clone()) {
                return Err(invalid("transaction selects an identity twice"));
            }
            installed
                .manifest
                .check_host_compatibility()
                .map_err(|error| invalid(error.to_string()))?;
            let staging = self.stage(&mut installed, bytes, limits, control)?;
            next.disabled.remove(&installed.manifest.id);
            next.active.insert(installed.manifest.id.clone(), installed);
            staged.push(staging);
        }
        check_set(&next.active, host)?;
        control.check()?;
        control.emit(Stage::Activating, None, 0, Some(1));
        // From this point cancellation cannot interrupt the single pointer commit.
        for directory in staged {
            let _ = directory.keep();
        }
        let state = self.activate(old, next)?;
        control.emit(
            Stage::Completed,
            None,
            identities.len() as u64,
            Some(identities.len() as u64),
        );
        Ok(state)
    }

    pub fn disable(
        &self,
        id: &str,
        generation: u64,
        host: &HostState,
    ) -> Result<InstallationState> {
        let _lock = self.lock()?;
        let old = self.expected_state(generation)?;
        let mut next = old.snapshot();
        let installed = next
            .active
            .remove(id)
            .ok_or_else(|| invalid("extension is not active"))?;
        next.disabled.insert(id.to_owned(), installed);
        check_set(&next.active, host)?;
        self.activate(old, next)
    }

    pub fn enable(
        &self,
        catalog: &Catalog,
        id: &str,
        generation: u64,
        host: &HostState,
        limits: Limits,
    ) -> Result<InstallationState> {
        self.enable_with_progress(
            catalog,
            id,
            generation,
            host,
            limits,
            &OperationControl::default(),
        )
    }

    pub fn enable_with_progress(
        &self,
        catalog: &Catalog,
        id: &str,
        generation: u64,
        host: &HostState,
        limits: Limits,
        control: &OperationControl,
    ) -> Result<InstallationState> {
        control.check()?;
        let _lock = self.lock()?;
        let old = self.expected_state(generation)?;
        let mut next = old.snapshot();
        let mut installed = next
            .disabled
            .remove(id)
            .ok_or_else(|| invalid("extension is not disabled"))?;
        validate_installed_source(catalog, &installed)?;
        let bytes = read_package(
            &self.package_directory(&installed)?.join("archive.exaudio"),
            limits,
        )?;
        let staging = self.stage(&mut installed, &bytes, limits, control)?;
        next.active.insert(id.to_owned(), installed);
        check_set(&next.active, host)?;
        control.check()?;
        control.emit(Stage::Activating, Some(id), 0, Some(1));
        let _ = staging.keep();
        let state = self.activate(old, next)?;
        control.emit(Stage::Completed, Some(id), 1, Some(1));
        Ok(state)
    }

    /// Removes managed inventory, never arbitrary source folders. Packages referenced
    /// by current/previous snapshots are retained for rollback; garbage collection only
    /// considers known managed directories released by the superseded previous snapshot.
    pub fn uninstall(
        &self,
        id: &str,
        generation: u64,
        host: &HostState,
    ) -> Result<InstallationState> {
        let _lock = self.lock()?;
        let old = self.expected_state(generation)?;
        let mut next = old.snapshot();
        if next
            .active
            .remove(id)
            .or_else(|| next.disabled.remove(id))
            .is_none()
        {
            return Err(invalid("extension is not installed"));
        }
        check_set(&next.active, host)?;
        self.activate(old, next)
    }

    pub fn rollback(
        &self,
        catalog: &Catalog,
        generation: u64,
        host: &HostState,
        limits: Limits,
    ) -> Result<InstallationState> {
        self.rollback_with_progress(
            catalog,
            generation,
            host,
            limits,
            &OperationControl::default(),
        )
    }

    pub fn rollback_with_progress(
        &self,
        catalog: &Catalog,
        generation: u64,
        host: &HostState,
        limits: Limits,
        control: &OperationControl,
    ) -> Result<InstallationState> {
        let _lock = self.lock()?;
        let old = self.expected_state(generation)?;
        let mut next = old
            .previous
            .clone()
            .ok_or_else(|| invalid("no previous installation generation"))?;
        catalog.validate()?;
        check_set(&next.active, host)?;
        let mut staging = Vec::new();
        for installed in next.active.values_mut().chain(next.disabled.values_mut()) {
            control.check()?;
            validate_installed_source(catalog, installed)?;
            let bytes = read_package(
                &self.package_directory(installed)?.join("archive.exaudio"),
                limits,
            )?;
            staging.push(self.stage(installed, &bytes, limits, control)?);
        }
        control.check()?;
        control.emit(Stage::Activating, None, 0, Some(1));
        for directory in staging {
            let _ = directory.keep();
        }
        let state = self.activate(old, next)?;
        control.emit(Stage::Completed, None, 1, Some(1));
        Ok(state)
    }

    /// Immediate host-runtime activation failure recovery, NOT user-requested rollback.
    /// The host must retain the exact pre-transaction state. CAS and snapshot equality
    /// forbid restoring an unrelated generation or introducing new source/grants.
    pub fn restore_failed_activation(
        &self,
        expected_generation: u64,
        prior: &InstallationState,
        host: &HostState,
    ) -> Result<InstallationState> {
        let _lock = self.lock()?;
        let current = self.expected_state(expected_generation)?;
        if prior.generation.checked_add(1) != Some(expected_generation)
            || current.previous.as_ref() != Some(&prior.snapshot())
        {
            return Err(Error::Blocked(
                "failure recovery does not match immediately prior activation".into(),
            ));
        }
        check_set(&prior.active, host)?;
        let mut restored = prior.clone();
        restored.generation = expected_generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation exhausted"))?;
        atomic_json(&self.root.join("active.json"), &restored)?;
        Ok(restored)
    }

    fn stage(
        &self,
        installed: &mut Installed,
        bytes: &[u8],
        limits: Limits,
        control: &OperationControl,
    ) -> Result<tempfile::TempDir> {
        let staging = tempfile::Builder::new()
            .prefix("package-")
            .tempdir_in(self.root.join("packages"))?;
        let content = staging.path().join("content");
        std::fs::create_dir(&content)?;
        extract_contents(
            bytes,
            PackageMetadata {
                manifest: &installed.manifest,
                bytes: installed.bytes,
                sha256: &installed.sha256,
            },
            limits,
            &content,
            control,
        )?;
        let mut archive = File::create(staging.path().join("archive.exaudio"))?;
        archive.write_all(bytes)?;
        archive.sync_all()?;
        installed.directory = staging
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("non-UTF8 staging directory"))?
            .to_owned();
        Ok(staging)
    }

    fn package_directory(&self, installed: &Installed) -> Result<PathBuf> {
        if !managed_directory(&installed.directory) {
            return Err(invalid("unsafe installed path"));
        }
        let path = self.root.join("packages").join(&installed.directory);
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(invalid("installed package directory is a symlink"));
        }
        Ok(path)
    }
    fn lock(&self) -> Result<File> {
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("transaction.lock"))?;
        lock.try_lock_exclusive()
            .map_err(|error| Error::Blocked(format!("installation store is busy: {error}")))?;
        Ok(lock)
    }
    fn expected_state(&self, expected: u64) -> Result<InstallationState> {
        let state = self.state()?;
        if state.generation != expected {
            return Err(Error::Blocked(
                "installation changed; refresh plan and consent".into(),
            ));
        }
        Ok(state)
    }
    fn activate(
        &self,
        old: InstallationState,
        next: InstallationSnapshot,
    ) -> Result<InstallationState> {
        let generation = old
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("installation generation exhausted"))?;
        let state = InstallationState {
            generation,
            active: next.active,
            disabled: next.disabled,
            previous: Some(old.snapshot()),
        };
        atomic_json(&self.root.join("active.json"), &state)?;
        // Do not collect old.previous here: host activation can still fail and restore
        // the full prior state. Explicit collection is safe after host acceptance.
        Ok(state)
    }

    /// Call after host runtime acceptance, with the pre-transaction state. Only its
    /// known managed directories unreferenced by current and rollback state are removed.
    /// A refused deletion is returned, never hidden as a successful cleanup.
    pub fn collect_unreferenced(&self, previous_state: &InstallationState) -> Result<usize> {
        let _lock = self.lock()?;
        let current = self.state()?;
        let referenced: BTreeSet<_> = all_installed(&current)
            .map(|installed| installed.directory.as_str())
            .collect();
        let candidates: BTreeSet<_> = all_installed(previous_state)
            .map(|installed| installed.directory.as_str())
            .collect();
        let mut removed = 0;
        for directory in candidates {
            if referenced.contains(directory) {
                continue;
            }
            if !managed_directory(directory) {
                return Err(invalid("unsafe cleanup path"));
            }
            let path = self.root.join("packages").join(directory);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(invalid("refusing to collect symlink"));
                }
                Ok(_) => {
                    std::fs::remove_dir_all(path)?;
                    removed += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(removed)
    }
}

fn managed_directory(name: &str) -> bool {
    name.starts_with("package-") && crate::catalog::safe_token(name)
}
fn all_installed(state: &InstallationState) -> impl Iterator<Item = &Installed> {
    state.active.values().chain(state.disabled.values()).chain(
        state
            .previous
            .iter()
            .flat_map(|snapshot| snapshot.active.values().chain(snapshot.disabled.values())),
    )
}
fn validate_inventory(
    active: &BTreeMap<String, Installed>,
    disabled: &BTreeMap<String, Installed>,
) -> Result<()> {
    for (id, installed) in active.iter().chain(disabled) {
        if id != &installed.manifest.id
            || !managed_directory(&installed.directory)
            || (active.contains_key(id) && disabled.contains_key(id))
        {
            return Err(invalid("corrupt activation identity or path"));
        }
        installed
            .manifest
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        if let InstallProvenance::Reviewed { release } = &installed.provenance
            && (release.manifest != installed.manifest
                || release.asset.bytes != installed.bytes
                || release.asset.sha256 != installed.sha256)
        {
            return Err(invalid(
                "installed review provenance does not match package",
            ));
        }
    }
    Ok(())
}
fn validate_installed_source(catalog: &Catalog, installed: &Installed) -> Result<()> {
    match &installed.provenance {
        InstallProvenance::Reviewed { release } => {
            validate_selection(catalog, std::slice::from_ref(release.as_ref()))
        }
        InstallProvenance::Developer { .. } => Ok(()), // Prior explicit source approval persists with pinned bytes.
    }
}
fn approve_developer(request: &DeveloperInstallRequest) -> Result<()> {
    if request.approved_sha256 != request.preview.sha256 || request.approved_sha256.len() != 64 {
        return Err(Error::Blocked(
            "explicit developer source digest approval is required".into(),
        ));
    }
    Ok(())
}
fn approved(
    manifest: &ExtensionManifest,
    grants: &PermissionGrants,
) -> Result<BTreeSet<Permission>> {
    let approved = grants.get(&manifest.id);
    if manifest
        .permissions
        .iter()
        .any(|permission| !approved.is_some_and(|set| set.contains(permission)))
    {
        return Err(Error::Blocked(format!(
            "{} has unapproved requested permissions",
            manifest.id
        )));
    }
    Ok(manifest.permissions.iter().cloned().collect())
}
fn reviewed_records<'a>(
    releases: &[Release],
    packages: &'a [Vec<u8>],
    grants: &PermissionGrants,
) -> Result<Vec<(Installed, &'a [u8])>> {
    if releases.len() != packages.len() {
        return Err(invalid("one package is required for each selected release"));
    }
    releases
        .iter()
        .zip(packages)
        .map(|(release, bytes)| {
            Ok((
                Installed {
                    manifest: release.manifest.clone(),
                    bytes: release.asset.bytes,
                    sha256: release.asset.sha256.clone(),
                    provenance: InstallProvenance::Reviewed {
                        release: Arc::new(release.clone()),
                    },
                    directory: String::new(),
                    approved_permissions: approved(&release.manifest, grants)?,
                },
                bytes.as_slice(),
            ))
        })
        .collect()
}
async fn download_all(
    client: &MarketplaceClient,
    releases: &[Release],
    limits: Limits,
    control: &OperationControl,
) -> Result<Vec<Vec<u8>>> {
    let mut packages = Vec::with_capacity(releases.len());
    for (index, release) in releases.iter().enumerate() {
        control.check()?;
        control.emit(
            Stage::Preparing,
            Some(&release.manifest.id),
            index as u64,
            Some(releases.len() as u64),
        );
        packages.push(
            client
                .download_with_progress(release, limits, control)
                .await?,
        );
    }
    Ok(packages)
}
fn validate_selection(catalog: &Catalog, releases: &[Release]) -> Result<()> {
    catalog.validate()?;
    let mut identities = BTreeSet::new();
    for release in releases {
        if !identities.insert(&release.manifest.id) {
            return Err(invalid("transaction selects an extension twice"));
        }
        if release.withdrawn.is_some()
            || catalog.release(&release.manifest.id, &release.version) != Some(release)
        {
            return Err(invalid(
                "release is withdrawn or differs from validated index",
            ));
        }
    }
    Ok(())
}
fn check_set(active: &BTreeMap<String, Installed>, host: &HostState) -> Result<()> {
    let manifests = active
        .iter()
        .map(|(id, installed)| (id.clone(), installed.manifest.clone()))
        .collect();
    let blockers = dependency_blockers(&manifests, host);
    if !blockers.is_empty() {
        return Err(Error::Blocked(blockers.join("; ")));
    }
    for installed in active.values() {
        installed
            .manifest
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        installed
            .manifest
            .check_host_compatibility()
            .map_err(|error| invalid(error.to_string()))?;
        if installed
            .manifest
            .permissions
            .iter()
            .any(|permission| !installed.approved_permissions.contains(permission))
        {
            return Err(Error::Blocked(format!(
                "{} has unapproved permissions",
                installed.manifest.id
            )));
        }
    }
    Ok(())
}

fn compare_tree(expected: &Path, actual: &Path, control: &OperationControl) -> Result<()> {
    control.check()?;
    let metadata = std::fs::symlink_metadata(actual)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid(
            "installed content directory is missing, replaced or a symlink",
        ));
    }
    let expected_names = std::fs::read_dir(expected)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    let actual_names = std::fs::read_dir(actual)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    if expected_names != actual_names {
        return Err(invalid(
            "installed content inventory differs from approved archive",
        ));
    }
    for name in expected_names {
        let expected = expected.join(&name);
        let actual = actual.join(&name);
        let expected_metadata = std::fs::metadata(&expected)?;
        let actual_metadata = std::fs::symlink_metadata(&actual)?;
        if actual_metadata.file_type().is_symlink() {
            return Err(invalid("installed content contains a symlink"));
        }
        if expected_metadata.is_dir() {
            compare_tree(&expected, &actual, control)?;
        } else {
            if !actual_metadata.is_file() || expected_metadata.len() != actual_metadata.len() {
                return Err(invalid("installed content differs from approved archive"));
            }
            let mut expected_file = File::open(expected)?;
            let mut actual_file = File::open(actual)?;
            let mut expected_bytes = [0; 16 * 1024];
            let mut actual_bytes = [0; 16 * 1024];
            loop {
                control.check()?;
                let count = expected_file.read(&mut expected_bytes)?;
                if count == 0 {
                    break;
                }
                actual_file.read_exact(&mut actual_bytes[..count])?;
                if expected_bytes[..count] != actual_bytes[..count] {
                    return Err(invalid(
                        "installed content bytes differ from approved archive",
                    ));
                }
            }
        }
    }
    Ok(())
}
