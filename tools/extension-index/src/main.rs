mod author;

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

use exaudio_extension::ExtensionManifest;
use exaudio_marketplace::{
    Catalog, CatalogExtension, DeveloperInstallRequest, HostState, InstallRequest,
    InstallationStore, Limits, MarketplaceClient, OperationControl, PermissionGrants, Release,
    ReleaseAsset, Repository,
};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

const USAGE: &str = "exaudio-extension-index commands:
  init-index DESTINATION
  init-author OWNER REPOSITORY EXTENSION_ID VERSION SDK_GITHUB_URL FULL_SDK_COMMIT DESTINATION
  validate-index INDEX.json [PREVIOUS.json]
  inspect INDEX.json [QUERY]
  fetch RAW_GITHUB_URL CACHE.json [--offline]
  pack MANIFEST.json COMPONENT.wasm OUTPUT.exaudio [LICENSE]
  release-entry OWNER REPOSITORY MANIFEST.json PACKAGE.exaudio TAG SOURCE_COMMIT
  validate-package INDEX.json ID VERSION PACKAGE.exaudio
  plan-mode INDEX.json ID VERSION MODE STORE HOST.json GRANTS.json
  plan-release INDEX.json ID VERSION STORE HOST.json GRANTS.json
  preview-developer PACKAGE.exaudio
  install-developer INDEX.json PACKAGE.exaudio APPROVED_SHA256 DEPENDENCIES.json STORE HOST.json GRANTS.json
  install INDEX.json SELECTION.json STORE HOST.json GRANTS.json
  install-release RAW_GITHUB_URL CACHE.json SELECTION.json STORE HOST.json GRANTS.json
  rollback INDEX.json STORE HOST.json
  disable ID STORE HOST.json
  enable INDEX.json ID STORE HOST.json
  uninstall ID STORE HOST.json

SELECTION.json: [{\"id\":\"author.extension\",\"version\":\"1.0.0\",\"package\":\"local.exaudio\"}]
The package field is required for install and ignored for install-release.
HOST.json: {\"requirements\":[],\"builtins\":[]}; GRANTS.json: {\"author.extension\":[{\"kind\":\"document-read\"}]}
Grants must represent explicit user consent, never copied automatically from the manifest.
";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    id: String,
    version: Version,
    #[serde(default)]
    package: Option<PathBuf>,
}

fn bytes(path: impl AsRef<Path>, maximum: u64) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > maximum {
        return Err("input exceeds byte limit".into());
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err("input exceeds byte limit".into());
    }
    Ok(bytes)
}

fn json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T> {
    Ok(serde_json::from_slice(&bytes(path, 8 * 1024 * 1024)?)?)
}

fn catalog(path: &str) -> Result<Catalog> {
    Ok(Catalog::parse(&bytes(path, 8 * 1024 * 1024)?)?)
}
fn print(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
fn release<'a>(catalog: &'a Catalog, id: &str, version: &str) -> Result<&'a Release> {
    catalog
        .release(id, &version.parse()?)
        .ok_or_else(|| "release is absent from index".into())
}
fn request(
    catalog: &Catalog,
    selection: &[Selection],
    grants: PermissionGrants,
    generation: u64,
) -> Result<InstallRequest> {
    let releases = selection
        .iter()
        .map(|item| {
            catalog
                .release(&item.id, &item.version)
                .cloned()
                .ok_or_else(|| {
                    format!("release {}@{} is absent from index", item.id, item.version).into()
                })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(InstallRequest {
        releases,
        approved_permissions: grants,
        expected_generation: generation,
    })
}

#[tokio::main]
async fn main() {
    let control = OperationControl::new(|progress| {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        eprintln!(
            "{}",
            serde_json::json!({"timestamp": timestamp, "progress": progress})
        );
    });
    let cancellation = control.clone();
    let signal = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancellation.cancel();
        }
    });
    let result = run(&control).await;
    signal.abort();
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run(control: &OperationControl) -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let limits = Limits::default();
    match args.as_slice() {
        ["init-index", destination] => {
            std::fs::create_dir(destination)?;
            std::fs::write(
                Path::new(destination).join("index.json"),
                include_str!("../template/index.json"),
            )?;
            std::fs::write(
                Path::new(destination).join("README.md"),
                include_str!("../template/README.md"),
            )?;
            println!(
                "Created an empty index repository layout; no repository was created on GitHub."
            );
        }
        [
            "init-author",
            owner,
            repository,
            id,
            version,
            sdk_repository,
            sdk_commit,
            destination,
        ] => {
            author::create(author::Spec {
                owner,
                repository,
                id,
                version,
                sdk_repository,
                sdk_commit,
                destination: Path::new(destination),
            })?;
        }
        ["pack", manifest, component, output] => pack(manifest, component, output, None, control)?,
        ["pack", manifest, component, output, license] => {
            pack(manifest, component, output, Some(license), control)?
        }
        ["validate-index", path] => {
            catalog(path)?;
            println!("valid catalog schema and release origins");
        }
        ["validate-index", path, previous] => {
            catalog(previous)?.validate_successor(&catalog(path)?)?;
            println!("valid catalog and immutable release history");
        }
        ["inspect", path] => print(&catalog(path)?)?,
        ["inspect", path, query] => print(&catalog(path)?.search(query))?,
        ["fetch", source, cache] | ["fetch", source, cache, "--offline"] => {
            let client = MarketplaceClient::new(source, cache)?;
            let snapshot = client
                .catalog_with_progress(args.len() == 4, control)
                .await?;
            eprintln!(
                "from_cache={} refresh_error={:?}",
                snapshot.from_cache, snapshot.refresh_error
            );
            print(&snapshot.catalog)?;
        }
        [
            "release-entry",
            owner,
            repository,
            manifest,
            package,
            tag,
            commit,
        ] => {
            let manifest: ExtensionManifest = json(manifest)?;
            let package_bytes = bytes(package, limits.compressed_bytes)?;
            let name = Path::new(package)
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("invalid package filename")?
                .to_owned();
            let entry = CatalogExtension {
                id: manifest.id.clone(),
                publisher: (*owner).to_owned(),
                repository: Repository {
                    owner: (*owner).to_owned(),
                    name: (*repository).to_owned(),
                },
                keywords: Vec::new(),
                releases: vec![Release {
                    version: manifest.version.clone(),
                    tag: (*tag).to_owned(),
                    source_commit: (*commit).to_owned(),
                    manifest,
                    asset: ReleaseAsset {
                        url: format!(
                            "https://github.com/{owner}/{repository}/releases/download/{tag}/{name}"
                        ),
                        name,
                        bytes: package_bytes.len() as u64,
                        sha256: format!("{:x}", Sha256::digest(&package_bytes)),
                    },
                    withdrawn: None,
                }],
            };
            Catalog {
                schema: 1,
                extensions: vec![entry.clone()],
            }
            .validate()?;
            exaudio_marketplace::validate_package(&package_bytes, &entry.releases[0], limits)?;
            print(&entry)?;
        }
        ["validate-package", path, id, version, package] => {
            let index = catalog(path)?;
            print(&exaudio_marketplace::validate_package(
                &bytes(package, limits.compressed_bytes)?,
                release(&index, id, version)?,
                limits,
            )?)?;
        }
        ["plan-mode", path, id, version, mode, store, host, grants] => {
            let index = catalog(path)?;
            let installed = InstallationStore::open(store)?.state()?;
            let diff = exaudio_marketplace::plan_mode(
                &index,
                &release(&index, id, version)?.manifest,
                mode,
                &installed.active,
                &json(host)?,
                &json(grants)?,
            );
            print(&diff)?;
            if !diff.blockers.is_empty() {
                return Err("plan contains blockers".into());
            }
        }
        ["plan-release", path, id, version, store, host, grants] => {
            let index = catalog(path)?;
            let installed = InstallationStore::open(store)?.state()?;
            let diff = exaudio_marketplace::plan_release(
                &index,
                release(&index, id, version)?,
                &installed.active,
                &json(host)?,
                &json(grants)?,
            );
            print(&diff)?;
            if !diff.blockers.is_empty() {
                return Err("plan contains blockers".into());
            }
        }
        ["preview-developer", package] => {
            print(
                &exaudio_marketplace::preview_developer_package_with_progress(
                    Path::new(package),
                    limits,
                    control,
                )?,
            )?;
        }
        [
            "install-developer",
            path,
            package,
            digest,
            dependencies,
            store,
            host,
            grants,
        ] => {
            let index = catalog(path)?;
            let selection: Vec<Selection> = json(dependencies)?;
            let store = InstallationStore::open(store)?;
            let dependencies =
                request(&index, &selection, json(grants)?, store.state()?.generation)?;
            let request = DeveloperInstallRequest {
                preview: exaudio_marketplace::preview_developer_package_with_progress(
                    Path::new(package),
                    limits,
                    control,
                )?,
                approved_sha256: (*digest).to_owned(),
                dependencies: dependencies.releases,
                approved_permissions: dependencies.approved_permissions,
                expected_generation: dependencies.expected_generation,
            };
            let packages = selection
                .iter()
                .map(|item| {
                    bytes(
                        item.package
                            .as_ref()
                            .ok_or("dependency requires local package path")?,
                        limits.compressed_bytes,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            print(&store.install_developer_packages(
                &index,
                &request,
                &packages,
                &json(host)?,
                limits,
                control,
            )?)?;
        }
        ["install", path, selection, store, host, grants] => {
            let index = catalog(path)?;
            let selection: Vec<Selection> = json(selection)?;
            let store = InstallationStore::open(store)?;
            let request = request(&index, &selection, json(grants)?, store.state()?.generation)?;
            let packages = selection
                .iter()
                .map(|item| {
                    bytes(
                        item.package
                            .as_ref()
                            .ok_or("local installation requires package path")?,
                        limits.compressed_bytes,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            print(&store.install_packages_with_progress(
                &index,
                &request,
                &packages,
                &json(host)?,
                limits,
                control,
            )?)?;
        }
        [
            "install-release",
            source,
            cache,
            selection,
            store,
            host,
            grants,
        ] => {
            let client = MarketplaceClient::new(source, cache)?;
            let snapshot = client.catalog_with_progress(false, control).await?;
            if let Some(error) = &snapshot.refresh_error {
                eprintln!("using cached index: {error}");
            }
            let selection: Vec<Selection> = json(selection)?;
            let store = InstallationStore::open(store)?;
            let request = request(
                &snapshot.catalog,
                &selection,
                json(grants)?,
                store.state()?.generation,
            )?;
            print(
                &store
                    .install_with_progress(
                        &client,
                        &snapshot.catalog,
                        &request,
                        &json(host)?,
                        limits,
                        control,
                    )
                    .await?,
            )?;
        }
        ["rollback", path, store, host] => {
            let store = InstallationStore::open(store)?;
            print(&store.rollback_with_progress(
                &catalog(path)?,
                store.state()?.generation,
                &json::<HostState>(host)?,
                limits,
                control,
            )?)?;
        }
        ["disable", id, store, host] => {
            let store = InstallationStore::open(store)?;
            print(&store.disable(id, store.state()?.generation, &json::<HostState>(host)?)?)?;
        }
        ["enable", path, id, store, host] => {
            let store = InstallationStore::open(store)?;
            print(&store.enable_with_progress(
                &catalog(path)?,
                id,
                store.state()?.generation,
                &json::<HostState>(host)?,
                limits,
                control,
            )?)?;
        }
        ["uninstall", id, store, host] => {
            let store = InstallationStore::open(store)?;
            print(&store.uninstall(id, store.state()?.generation, &json::<HostState>(host)?)?)?;
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn pack(
    manifest: &str,
    component: &str,
    output: &str,
    license: Option<&str>,
    control: &OperationControl,
) -> Result<()> {
    let output = Path::new(output);
    if output.extension().and_then(|extension| extension.to_str()) != Some("exaudio") {
        return Err("output must have .exaudio extension".into());
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut writer = zip::ZipWriter::new(temporary);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut sources = vec![("manifest.json", manifest), ("module.wasm", component)];
    if let Some(license) = license {
        sources.push(("assets/LICENSE", license));
    }
    let limits = Limits::default();
    let mut expanded = 0_u64;
    for (name, source) in sources {
        let metadata = std::fs::symlink_metadata(source)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("package inputs must be regular files, not symlinks".into());
        }
        expanded = expanded
            .checked_add(metadata.len())
            .ok_or("package size overflow")?;
        if expanded > limits.expanded_bytes
            || (name == "manifest.json" && metadata.len() > limits.manifest_bytes)
        {
            return Err("package input exceeds expanded or manifest byte limit".into());
        }
        writer.start_file(name, options)?;
        let mut input = std::fs::File::open(source)?;
        let mut buffer = [0; 16 * 1024];
        let mut written = 0_u64;
        loop {
            if control.is_cancelled() {
                return Err("packaging cancelled".into());
            }
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            written = written
                .checked_add(count as u64)
                .ok_or("package size overflow")?;
            if written > metadata.len() {
                return Err("input changed while packaging".into());
            }
            writer.write_all(&buffer[..count])?;
            if written == metadata.len() || written.is_multiple_of(256 * 1024) {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_secs());
                eprintln!(
                    "{}",
                    serde_json::json!({"timestamp":timestamp,"stage":"packaging","entry":name,"completed":written,"total":metadata.len()})
                );
            }
        }
        if written != metadata.len() {
            return Err("input changed while packaging".into());
        }
    }
    let temporary = writer.finish()?;
    temporary.as_file().sync_all()?;
    let mut preview = exaudio_marketplace::preview_developer_package_with_progress(
        temporary.path(),
        limits,
        control,
    )?;
    // Never replace an existing archive without an explicit separate user action.
    temporary
        .persist_noclobber(output)
        .map_err(|error| error.error)?;
    preview.source = std::fs::canonicalize(output)?;
    print(&preview)
}
