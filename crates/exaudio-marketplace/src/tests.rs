use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

use exaudio_extension::{ExtensionManifest, ModeManifest, Permission, Runtime};
use semver::Version;
use sha2::{Digest, Sha256};
use zip::{ZipWriter, write::SimpleFileOptions};

use crate::*;

fn manifest(id: &str, version: &str) -> ExtensionManifest {
    ExtensionManifest {
        id: id.into(),
        name: "Test extension".into(),
        version: version.parse().unwrap(),
        api: "^0.1".parse().unwrap(),
        runtime: Runtime::Wasm,
        description: "Test fixture only".into(),
        license: "MIT".into(),
        dependencies: BTreeMap::new(),
        permissions: Vec::new(),
        modes: Vec::new(),
        providers: Vec::new(),
    }
}

fn archive(manifest: &ExtensionManifest, extra: &[(&str, &[u8])]) -> Vec<u8> {
    archive_module(manifest, b"\0asm\x0d\0\x01\0", extra)
}

fn archive_module(manifest: &ExtensionManifest, module: &[u8], extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("manifest.json", options).unwrap();
    writer
        .write_all(&serde_json::to_vec(manifest).unwrap())
        .unwrap();
    writer.start_file("module.wasm", options).unwrap();
    writer.write_all(module).unwrap();
    for (name, bytes) in extra {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn release(manifest: ExtensionManifest, bytes: &[u8]) -> Release {
    let version = manifest.version.clone();
    Release {
        tag: format!("v{version}"),
        source_commit: "a".repeat(40),
        manifest,
        version,
        asset: ReleaseAsset {
            name: "extension.exaudio".into(),
            url: String::new(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        },
        withdrawn: None,
    }
}

fn catalog(mut releases: Vec<Release>) -> Catalog {
    let mut entries: BTreeMap<String, CatalogExtension> = BTreeMap::new();
    for release in &mut releases {
        let repository = release.manifest.id.replace('.', "-");
        release.asset.url = format!(
            "https://github.com/fixture/{repository}/releases/download/{}/extension.exaudio",
            release.tag
        );
        entries
            .entry(release.manifest.id.clone())
            .or_insert_with(|| CatalogExtension {
                id: release.manifest.id.clone(),
                publisher: "fixture".into(),
                repository: Repository {
                    owner: "fixture".into(),
                    name: repository,
                },
                keywords: vec!["fixture".into()],
                releases: Vec::new(),
            })
            .releases
            .push(release.clone());
    }
    Catalog {
        schema: 1,
        extensions: entries.into_values().collect(),
    }
}

fn request(index: &Catalog, id: &str, version: &str, generation: u64) -> InstallRequest {
    InstallRequest {
        releases: vec![
            index
                .release(id, &version.parse().unwrap())
                .unwrap()
                .clone(),
        ],
        approved_permissions: BTreeMap::new(),
        expected_generation: generation,
    }
}

#[test]
fn rejects_malicious_paths_native_files_and_bombs() {
    let manifest = manifest("fixture.paths", "1.0.0");
    for name in [
        "../escape",
        "/absolute",
        "C:/drive",
        "assets/../../escape",
        "assets\\escape",
        "assets/CON.txt",
        "assets/trailing.",
        "assets/file:stream",
        "assets/native.dll",
        "assets/run.ps1",
    ] {
        let bytes = archive(&manifest, &[(name, b"payload")]);
        assert!(
            validate_package(
                &bytes,
                &release(manifest.clone(), &bytes),
                Limits::default()
            )
            .is_err(),
            "accepted {name}"
        );
    }
    for executable in [
        b"MZbinary".as_slice(),
        b"\x7fELFbinary",
        b"\xcf\xfa\xed\xfebinary",
    ] {
        let bytes = archive(&manifest, &[("assets/innocent.dat", executable)]);
        assert!(
            validate_package(
                &bytes,
                &release(manifest.clone(), &bytes),
                Limits::default()
            )
            .is_err()
        );
    }
    let bomb = vec![0; 64 * 1024];
    let bytes = archive(&manifest, &[("assets/bomb", &bomb)]);
    let limits = Limits {
        expanded_bytes: 1024,
        ..Limits::default()
    };
    assert!(validate_package(&bytes, &release(manifest.clone(), &bytes), limits).is_err());
    let limits = Limits {
        files: 1,
        ..Limits::default()
    };
    assert!(validate_package(&bytes, &release(manifest, &bytes), limits).is_err());
}

#[test]
fn rejects_symlinks_case_collisions_and_wrong_identity() {
    let manifest = manifest("fixture.package", "1.0.0");
    let bytes = archive(&manifest, &[("assets/A", b"one"), ("assets/a", b"two")]);
    assert!(
        validate_package(
            &bytes,
            &release(manifest.clone(), &bytes),
            Limits::default()
        )
        .is_err()
    );
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("assets/link", "../../outside", SimpleFileOptions::default())
        .unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert!(
        validate_package(
            &bytes,
            &release(manifest.clone(), &bytes),
            Limits::default()
        )
        .unwrap_err()
        .to_string()
        .contains("symlinks")
    );
    let bytes = archive(&manifest, &[]);
    let mut expected = release(manifest.clone(), &bytes);
    expected.asset.sha256 = "0".repeat(64);
    assert!(validate_package(&bytes, &expected, Limits::default()).is_err());
    expected = release(manifest.clone(), &bytes);
    expected.manifest.description = "mutated".into();
    assert!(validate_package(&bytes, &expected, Limits::default()).is_err());
    expected = release(manifest, &bytes);
    expected.manifest.runtime = Runtime::Native;
    assert!(validate_package(&bytes, &expected, Limits::default()).is_err());
}

#[test]
fn validates_catalog_origins_history_and_schema() {
    let manifest = manifest("fixture.catalog", "1.0.0");
    let bytes = archive(&manifest, &[]);
    let index = catalog(vec![release(manifest, &bytes)]);
    index.validate().unwrap();
    assert_eq!(index.search("fixture").len(), 1);
    let mut next = index.clone();
    next.extensions[0].releases[0].asset.url = "https://127.0.0.1/private".into();
    assert!(next.validate().is_err());
    next = index.clone();
    next.schema = 2;
    assert!(next.validate().is_err());
    next = index.clone();
    next.extensions.push(next.extensions[0].clone());
    assert!(next.validate().is_err());
    next = index.clone();
    let duplicate = next.extensions[0].releases[0].clone();
    next.extensions[0].releases.push(duplicate);
    assert!(next.validate().is_err());
    next = index.clone();
    next.extensions[0].publisher = "other".into();
    assert!(next.validate().is_err());
    next = index.clone();
    next.extensions[0].releases[0].manifest.description = "changed".into();
    assert!(index.validate_successor(&next).is_err());
    next = index.clone();
    next.extensions[0].releases[0].withdrawn = Some("security withdrawal".into());
    index.validate_successor(&next).unwrap();
    next.extensions[0].releases.clear();
    assert!(index.validate_successor(&next).is_err());
    for url in [
        "http://release-assets.githubusercontent.com/github-production-release-asset/x",
        "https://release-assets.githubusercontent.com.evil.test/github-production-release-asset/x",
        "https://user@release-assets.githubusercontent.com/github-production-release-asset/x",
        "https://release-assets.githubusercontent.com:8443/github-production-release-asset/x",
        "https://release-assets.githubusercontent.com/unrelated",
        "https://127.0.0.1/private",
    ] {
        assert!(
            !crate::client::release_cdn(&url.parse().unwrap()),
            "accepted {url}"
        );
    }
    assert!(crate::client::release_cdn(&"https://release-assets.githubusercontent.com/github-production-release-asset/1/2?token=redacted".parse().unwrap()));
}

#[test]
fn failed_update_preserves_old_activation_and_rollback_is_atomic() {
    let root = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(root.path()).unwrap();
    let first = manifest("fixture.update", "1.0.0");
    let second = manifest("fixture.update", "2.0.0");
    let first_bytes = archive(&first, &[]);
    let second_bytes = archive(&second, &[]);
    let index = catalog(vec![
        release(first, &first_bytes),
        release(second, &second_bytes),
    ]);
    let host = HostState::default();
    let initial = store
        .install_packages(
            &index,
            &request(&index, "fixture.update", "1.0.0", 0),
            &[first_bytes],
            &host,
            Limits::default(),
        )
        .unwrap();
    let initial_pointer = std::fs::read(root.path().join("active.json")).unwrap();
    let second_request = request(&index, "fixture.update", "2.0.0", 1);
    assert!(
        store
            .install_packages(
                &index,
                &second_request,
                &[b"bad zip".to_vec()],
                &host,
                Limits::default()
            )
            .is_err()
    );
    assert_eq!(
        initial_pointer,
        std::fs::read(root.path().join("active.json")).unwrap()
    );
    let updated = store
        .install_packages(
            &index,
            &second_request,
            &[second_bytes],
            &host,
            Limits::default(),
        )
        .unwrap();
    assert_eq!(updated.generation, 2);
    assert_eq!(&updated.previous.as_ref().unwrap().active, &initial.active);
    assert!(
        store
            .module_path(&initial.active["fixture.update"])
            .unwrap()
            .exists()
    );
    let restored = store.rollback(&index, 2, &host, Limits::default()).unwrap();
    assert_eq!(
        restored.active["fixture.update"].manifest.version,
        Version::new(1, 0, 0)
    );
    assert!(store.disable("fixture.update", 2, &host).is_err());
}

#[test]
fn active_modes_and_unapproved_permissions_block_changes() {
    let root = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(root.path()).unwrap();
    let mut extension = manifest("fixture.mode", "1.0.0");
    extension.permissions.push(Permission::DocumentRead);
    extension.modes.push(ModeManifest {
        id: "fixture.edit".into(),
        name: "Edit".into(),
        requires: BTreeMap::new(),
        recommends: BTreeMap::new(),
        provider: None,
        launch: Default::default(),
    });
    let bytes = archive(&extension, &[]);
    let index = catalog(vec![release(extension.clone(), &bytes)]);
    let mut install = request(&index, "fixture.mode", "1.0.0", 0);
    assert!(
        store
            .install_packages(
                &index,
                &install,
                std::slice::from_ref(&bytes),
                &HostState::default(),
                Limits::default()
            )
            .is_err()
    );
    assert_eq!(store.state().unwrap().generation, 0);
    let diff = plan_mode(
        &index,
        &extension,
        "fixture.edit",
        &BTreeMap::new(),
        &HostState::default(),
        &BTreeMap::new(),
    );
    assert_eq!(
        diff.requested_permissions["fixture.mode"],
        BTreeSet::from([Permission::DocumentRead])
    );
    install.approved_permissions.insert(
        "fixture.mode".into(),
        BTreeSet::from([Permission::DocumentRead]),
    );
    store
        .install_packages(
            &index,
            &install,
            &[bytes],
            &HostState::default(),
            Limits::default(),
        )
        .unwrap();
    let host = HostState {
        requirements: vec![ActiveRequirement {
            required_by: "open document / fixture.edit".into(),
            extension: "fixture.mode".into(),
            version: "=1.0.0".parse().unwrap(),
        }],
        builtins: Vec::new(),
    };
    assert!(
        store
            .disable("fixture.mode", 1, &host)
            .unwrap_err()
            .to_string()
            .contains("open document")
    );
    assert!(store.rollback(&index, 1, &host, Limits::default()).is_err());
    assert_eq!(store.state().unwrap().generation, 1);
}

#[test]
fn dependency_transaction_is_all_or_nothing_and_future_api_is_display_only() {
    let root = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(root.path()).unwrap();
    let mut owner = manifest("fixture.owner", "1.0.0");
    owner
        .dependencies
        .insert("fixture.dependency".into(), "^1".parse().unwrap());
    let dependency = manifest("fixture.dependency", "1.0.0");
    let a = archive(&owner, &[]);
    let b = archive(&dependency, &[]);
    let index = catalog(vec![release(owner, &a), release(dependency, &b)]);
    let mut selection = request(&index, "fixture.owner", "1.0.0", 0);
    selection.releases.push(
        index
            .release("fixture.dependency", &Version::new(1, 0, 0))
            .unwrap()
            .clone(),
    );
    assert!(
        store
            .install_packages(
                &index,
                &selection,
                &[a.clone(), b"broken".to_vec()],
                &HostState::default(),
                Limits::default()
            )
            .is_err()
    );
    assert!(store.state().unwrap().active.is_empty());
    store
        .install_packages(
            &index,
            &selection,
            &[a, b],
            &HostState::default(),
            Limits::default(),
        )
        .unwrap();
    assert_eq!(store.state().unwrap().active.len(), 2);
    assert!(
        store
            .disable("fixture.dependency", 1, &HostState::default())
            .is_err()
    );
    let mut future = manifest("fixture.future", "1.0.0");
    future.api = ">=99".parse().unwrap();
    let bytes = archive(&future, &[]);
    let future_index = catalog(vec![release(future, &bytes)]);
    future_index.validate().unwrap();
    assert!(
        store
            .install_packages(
                &future_index,
                &request(&future_index, "fixture.future", "1.0.0", 1),
                &[bytes],
                &HostState::default(),
                Limits::default()
            )
            .is_err()
    );
    assert_eq!(store.state().unwrap().generation, 1);
}

fn read_http_headers(stream: &mut impl Read) {
    let mut request = [0_u8; 8192];
    let mut used = 0;
    loop {
        assert!(used < request.len(), "fixture request headers exceed 8 KiB");
        let count = stream.read(&mut request[used..]).unwrap();
        assert_ne!(
            count, 0,
            "fixture connection closed before complete headers"
        );
        let previous = used;
        used += count;
        if request[previous.saturating_sub(3)..used]
            .windows(4)
            .any(|bytes| bytes == b"\r\n\r\n")
        {
            break;
        }
    }
}

/// One bounded HTTP response, with no production override or insecure transport flag.
fn http_response(status: &str, body: &[u8], headers: &str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    );
    let body = body.to_vec();
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // Accepted sockets inherit the listener's non-blocking mode on Windows.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    read_http_headers(&mut stream);
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                    break;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::park_timeout(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture server timed out or failed: {error}"),
            }
        }
    });
    (format!("http://{address}/fixture"), handle)
}

#[tokio::test]
async fn online_fetch_offline_cache_and_invalid_refresh_use_last_valid_catalog() {
    let root = tempfile::tempdir().unwrap();
    let source = "https://raw.githubusercontent.com/fixture/index/main/catalog.json";
    let mut client = MarketplaceClient::new(source, root.path().join("cache.json")).unwrap();
    let manifest = manifest("fixture.network", "1.0.0");
    let bytes = archive(&manifest, &[]);
    let index = catalog(vec![release(manifest, &bytes)]);
    let (url, server) = http_response("200 OK", &serde_json::to_vec(&index).unwrap(), "");
    client.fixture(source, url);
    assert!(!client.catalog(false).await.unwrap().from_cache);
    server.join().unwrap();
    let cached = client.catalog(true).await.unwrap();
    assert!(cached.from_cache);
    assert_eq!(cached.catalog, index);
    let (url, server) = http_response("200 OK", b"{\"schema\":999,\"extensions\":[]}", "");
    client.fixture(source, url);
    let fallback = client.catalog(false).await.unwrap();
    server.join().unwrap();
    assert!(fallback.from_cache && fallback.refresh_error.is_some());
    assert_eq!(fallback.catalog, index);
    let (url, server) = http_response("503 Service Unavailable", b"offline", "");
    client.fixture(source, url);
    assert!(client.catalog(false).await.unwrap().from_cache);
    server.join().unwrap();
    let different = MarketplaceClient::new(
        "https://raw.githubusercontent.com/fixture/other/main/catalog.json",
        root.path().join("cache.json"),
    )
    .unwrap();
    assert!(different.catalog(true).await.is_err());
}

#[tokio::test]
async fn release_download_checks_real_transport_and_rejects_redirects() {
    let root = tempfile::tempdir().unwrap();
    let mut client = MarketplaceClient::new(
        "https://raw.githubusercontent.com/fixture/index/main/catalog.json",
        root.path().join("cache.json"),
    )
    .unwrap();
    let manifest = manifest("fixture.download", "1.0.0");
    let bytes = archive(&manifest, &[]);
    let index = catalog(vec![release(manifest, &bytes)]);
    let release = &index.extensions[0].releases[0];
    let (url, server) = http_response("200 OK", &bytes, "");
    client.fixture(&release.asset.url, url);
    assert_eq!(
        client.download(release, Limits::default()).await.unwrap(),
        bytes
    );
    server.join().unwrap();
    let (url, server) = http_response(
        "302 Found",
        b"",
        "Location: http://127.0.0.1:1/intranet\r\n",
    );
    client.fixture(&release.asset.url, url);
    assert!(client.download(release, Limits::default()).await.is_err());
    server.join().unwrap();
    let (url, server) = http_response("200 OK", &vec![0; bytes.len() + 1], "");
    client.fixture(&release.asset.url, url);
    assert!(client.download(release, Limits::default()).await.is_err());
    server.join().unwrap();
}

#[test]
fn developer_consent_digest_and_provenance_are_distinct_from_review() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("local.exaudio");
    let mut local = manifest("fixture.local", "1.0.0");
    local.permissions.push(Permission::DocumentRead);
    let bytes = archive(&local, &[]);
    std::fs::write(&source, &bytes).unwrap();
    let preview = preview_developer_package(&source, Limits::default()).unwrap();
    let index = Catalog {
        schema: 1,
        extensions: Vec::new(),
    };
    let store = InstallationStore::open(root.path().join("store")).unwrap();
    let mut request = DeveloperInstallRequest {
        preview: preview.clone(),
        approved_sha256: String::new(),
        dependencies: Vec::new(),
        approved_permissions: BTreeMap::new(),
        expected_generation: 0,
    };
    let apply = |request: &DeveloperInstallRequest| {
        store.install_developer_packages(
            &index,
            request,
            &[],
            &HostState::default(),
            Limits::default(),
            &OperationControl::default(),
        )
    };
    assert!(apply(&request).is_err());
    request.approved_sha256 = preview.sha256.clone();
    assert!(apply(&request).is_err());
    request
        .approved_permissions
        .insert(local.id.clone(), BTreeSet::from([Permission::DocumentRead]));
    let mut changed = local.clone();
    changed.description = "bytes changed after consent".into();
    std::fs::write(&source, archive(&changed, &[])).unwrap();
    assert!(apply(&request).is_err());
    assert!(store.state().unwrap().active.is_empty());
    std::fs::write(&source, &bytes).unwrap();
    let state = apply(&request).unwrap();
    assert!(matches!(
        state.active[&local.id].provenance,
        InstallProvenance::Developer { .. }
    ));
    let diff = plan_developer(
        &index,
        &preview,
        &BTreeMap::new(),
        &HostState::default(),
        &BTreeMap::new(),
    );
    assert!(diff.blockers.is_empty());
    assert!(diff.developer.is_some());
    assert_eq!(
        diff.requested_permissions[&local.id],
        BTreeSet::from([Permission::DocumentRead])
    );
}

#[test]
fn disabled_inventory_uninstall_retention_and_failed_runtime_recovery() {
    let root = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(root.path()).unwrap();
    let manifest = manifest("fixture.lifecycle", "1.0.0");
    let bytes = archive(&manifest, &[]);
    let index = catalog(vec![release(manifest, &bytes)]);
    let host = HostState::default();
    let before = store.state().unwrap();
    let installed = store
        .install_packages(
            &index,
            &request(&index, "fixture.lifecycle", "1.0.0", 0),
            std::slice::from_ref(&bytes),
            &host,
            Limits::default(),
        )
        .unwrap();
    let recovered = store
        .restore_failed_activation(installed.generation, &before, &host)
        .unwrap();
    assert!(recovered.active.is_empty());
    assert_eq!(recovered.generation, 2);
    assert!(
        store
            .restore_failed_activation(2, &installed, &host)
            .is_err()
    );
    let installed = store
        .install_packages(
            &index,
            &request(&index, "fixture.lifecycle", "1.0.0", 2),
            &[bytes],
            &host,
            Limits::default(),
        )
        .unwrap();
    let path = store
        .module_path(&installed.active["fixture.lifecycle"])
        .unwrap();
    let disabled = store.disable("fixture.lifecycle", 3, &host).unwrap();
    assert!(disabled.active.is_empty());
    assert!(disabled.disabled.contains_key("fixture.lifecycle"));
    let removed = store.uninstall("fixture.lifecycle", 4, &host).unwrap();
    assert!(removed.disabled.is_empty());
    assert!(path.exists());
    store.collect_unreferenced(&disabled).unwrap();
    assert!(path.exists(), "rollback referenced package was removed");
    let restored = store.rollback(&index, 5, &host, Limits::default()).unwrap();
    assert!(restored.disabled.contains_key("fixture.lifecycle"));
    let active = store
        .enable(&index, "fixture.lifecycle", 6, &host, Limits::default())
        .unwrap();
    assert!(active.active.contains_key("fixture.lifecycle"));
}

#[test]
fn cancellation_before_commit_preserves_entire_generation() {
    use std::sync::{Arc, Mutex};
    let root = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(root.path()).unwrap();
    let manifest = manifest("fixture.cancel", "1.0.0");
    let bytes = archive(&manifest, &[]);
    let index = catalog(vec![release(manifest, &bytes)]);
    let token: Arc<Mutex<Option<OperationControl>>> = Arc::new(Mutex::new(None));
    let captured = token.clone();
    let control = OperationControl::new(move |progress| {
        if matches!(progress.stage, Stage::Extracting)
            && let Some(control) = captured.lock().unwrap().as_ref()
        {
            control.cancel();
        }
    });
    *token.lock().unwrap() = Some(control.clone());
    assert!(matches!(
        store.install_packages_with_progress(
            &index,
            &request(&index, "fixture.cancel", "1.0.0", 0),
            &[bytes],
            &HostState::default(),
            Limits::default(),
            &control
        ),
        Err(Error::Cancelled)
    ));
    assert!(store.state().unwrap().active.is_empty());
    // Drop the observer's token reference so this test does not retain an Arc cycle.
    *token.lock().unwrap() = None;
}

#[tokio::test]
async fn cancellation_interrupts_catalog_request_and_reports_download_bytes() {
    use std::sync::{Arc, Mutex};
    let root = tempfile::tempdir().unwrap();
    let source = "https://raw.githubusercontent.com/fixture/index/main/catalog.json";
    let mut client = MarketplaceClient::new(source, root.path().join("cache.json")).unwrap();
    let control = OperationControl::default();
    control.cancel();
    assert!(matches!(
        client.catalog_with_progress(false, &control).await,
        Err(Error::Cancelled)
    ));
    let manifest = manifest("fixture.progress", "1.0.0");
    let bytes = archive(&manifest, &[]);
    let index = catalog(vec![release(manifest, &bytes)]);
    let release = &index.extensions[0].releases[0];
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let control = OperationControl::new(move |progress| captured.lock().unwrap().push(progress));
    let (url, server) = http_response("200 OK", &bytes, "");
    client.fixture(&release.asset.url, url);
    client
        .download_with_progress(release, Limits::default(), &control)
        .await
        .unwrap();
    server.join().unwrap();
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.completed == bytes.len() as u64
                && event.total == Some(bytes.len() as u64)
                && event.extension.as_deref() == Some("fixture.progress"))
    );
}

#[tokio::test]
async fn cancellation_interrupts_an_inflight_http_response() {
    let root = tempfile::tempdir().unwrap();
    let source = "https://raw.githubusercontent.com/fixture/index/main/catalog.json";
    let mut client = MarketplaceClient::new(source, root.path().join("cache.json")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let (started, receiving) = tokio::sync::oneshot::channel();
    let (finished, finish) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::park_timeout(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture server failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        read_http_headers(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n")
            .unwrap();
        started.send(()).unwrap();
        // Wait for the actual cancellation result, not an arbitrary delayed response.
        finish.recv_timeout(Duration::from_secs(3)).unwrap();
    });
    client.fixture(source, format!("http://{address}/catalog"));
    let control = OperationControl::default();
    let worker_control = control.clone();
    let fetch =
        tokio::spawn(async move { client.catalog_with_progress(false, &worker_control).await });
    receiving.await.unwrap();
    control.cancel();
    assert!(matches!(fetch.await.unwrap(), Err(Error::Cancelled)));
    finished.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn release_plan_resolves_dependencies_and_uninstall_honors_open_document() {
    let mut owner = manifest("fixture.owner", "1.0.0");
    owner
        .dependencies
        .insert("fixture.dep".into(), "^1".parse().unwrap());
    let dependency = manifest("fixture.dep", "1.0.0");
    let owner_bytes = archive(&owner, &[]);
    let dependency_bytes = archive(&dependency, &[]);
    let index = catalog(vec![
        release(owner, &owner_bytes),
        release(dependency, &dependency_bytes),
    ]);
    let root = index
        .release("fixture.owner", &Version::new(1, 0, 0))
        .unwrap();
    let plan = plan_release(
        &index,
        root,
        &BTreeMap::new(),
        &HostState::default(),
        &BTreeMap::new(),
    );
    assert!(plan.blockers.is_empty());
    assert_eq!(plan.releases.len(), 2);
    assert_eq!(plan.releases[0].manifest.id, "fixture.dep");
    let temp = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(temp.path()).unwrap();
    let request = InstallRequest {
        releases: plan.releases,
        approved_permissions: BTreeMap::new(),
        expected_generation: 0,
    };
    store
        .install_packages(
            &index,
            &request,
            &[dependency_bytes, owner_bytes],
            &HostState::default(),
            Limits::default(),
        )
        .unwrap();
    let host = HostState {
        requirements: vec![ActiveRequirement {
            required_by: "open document".into(),
            extension: "fixture.owner".into(),
            version: "^1".parse().unwrap(),
        }],
        builtins: Vec::new(),
    };
    assert!(store.uninstall("fixture.owner", 1, &host).is_err());
    assert!(store.uninstall("fixture.dep", 1, &host).is_err());
    assert_eq!(store.state().unwrap().generation, 1);
}

#[test]
fn startup_checks_enabled_builtins_and_extracted_bytes_offline() {
    let temp = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(temp.path()).unwrap();
    let mut builtin = manifest("fixture.native", "1.0.0");
    builtin.runtime = Runtime::Native;
    let mut extension = manifest("fixture.startup", "1.0.0");
    extension
        .dependencies
        .insert(builtin.id.clone(), "^1".parse().unwrap());
    let bytes = archive(&extension, &[]);
    let index = catalog(vec![release(extension, &bytes)]);
    let enabled_host = HostState {
        requirements: Vec::new(),
        builtins: vec![builtin],
    };
    let installed = store
        .install_packages(
            &index,
            &request(&index, "fixture.startup", "1.0.0", 0),
            &[bytes],
            &enabled_host,
            Limits::default(),
        )
        .unwrap();
    assert!(
        store
            .validate_current(&HostState::default(), Limits::default())
            .is_err()
    );
    store
        .validate_current(&enabled_host, Limits::default())
        .unwrap();
    let module = store
        .module_path(&installed.active["fixture.startup"])
        .unwrap();
    std::fs::write(&module, b"replaced").unwrap();
    assert!(
        store
            .validate_current(&enabled_host, Limits::default())
            .is_err()
    );
    assert_eq!(store.state().unwrap().generation, 1);
}

#[test]
fn explicit_reviewed_plan_replaces_equal_manifest_developer_bytes() {
    let root = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(root.path().join("store")).unwrap();
    let extension = manifest("fixture.provenance", "1.0.0");
    let reviewed_bytes = archive(&extension, &[]);
    let developer_bytes = archive_module(&extension, b"\0asm\x0d\0\x01\0\0\x02\x01x", &[]);
    let source = root.path().join("local.exaudio");
    std::fs::write(&source, &developer_bytes).unwrap();
    let preview = preview_developer_package(&source, Limits::default()).unwrap();
    let index = catalog(vec![release(extension, &reviewed_bytes)]);
    let request = DeveloperInstallRequest {
        approved_sha256: preview.sha256.clone(),
        preview,
        dependencies: Vec::new(),
        approved_permissions: BTreeMap::new(),
        expected_generation: 0,
    };
    let installed = store
        .install_developer_packages(
            &index,
            &request,
            &[],
            &HostState::default(),
            Limits::default(),
            &OperationControl::default(),
        )
        .unwrap();
    let reviewed = &index.extensions[0].releases[0];
    assert_ne!(
        installed.active["fixture.provenance"].sha256,
        reviewed.asset.sha256
    );
    assert_eq!(
        installed.active["fixture.provenance"].manifest,
        reviewed.manifest
    );
    let plan = plan_release(
        &index,
        reviewed,
        &installed.active,
        &HostState::default(),
        &BTreeMap::new(),
    );
    assert!(plan.blockers.is_empty());
    assert!(matches!(&plan.changes[0], Change::Update { .. }));
    assert_eq!(plan.releases, vec![reviewed.clone()]);
    let applied = store
        .install_packages(
            &index,
            &InstallRequest {
                releases: plan.releases,
                approved_permissions: BTreeMap::new(),
                expected_generation: 1,
            },
            &[reviewed_bytes],
            &HostState::default(),
            Limits::default(),
        )
        .unwrap();
    assert_eq!(
        applied.active["fixture.provenance"].sha256,
        reviewed.asset.sha256
    );
    assert!(matches!(&applied.active["fixture.provenance"].provenance,
        InstallProvenance::Reviewed { release } if release.as_ref() == reviewed));
}

#[test]
fn reviewed_dependency_candidate_cannot_keep_different_developer_bytes() {
    let root = tempfile::tempdir().unwrap();
    let store = InstallationStore::open(root.path().join("store")).unwrap();
    let dependency = manifest("fixture.provenance-dependency", "1.0.0");
    let reviewed_bytes = archive(&dependency, &[]);
    let developer_bytes = archive_module(&dependency, b"\0asm\x0d\0\x01\0\0\x02\x01x", &[]);
    let source = root.path().join("local.exaudio");
    std::fs::write(&source, developer_bytes).unwrap();
    let preview = preview_developer_package(&source, Limits::default()).unwrap();
    let mut owner = manifest("fixture.provenance-owner", "1.0.0");
    owner
        .dependencies
        .insert(dependency.id.clone(), "^1".parse().unwrap());
    let owner_bytes = archive(&owner, &[]);
    let index = catalog(vec![
        release(dependency, &reviewed_bytes),
        release(owner, &owner_bytes),
    ]);
    let request = DeveloperInstallRequest {
        approved_sha256: preview.sha256.clone(),
        preview,
        dependencies: Vec::new(),
        approved_permissions: BTreeMap::new(),
        expected_generation: 0,
    };
    let installed = store
        .install_developer_packages(
            &index,
            &request,
            &[],
            &HostState::default(),
            Limits::default(),
            &OperationControl::default(),
        )
        .unwrap();
    let selected = index
        .release("fixture.provenance-owner", &Version::new(1, 0, 0))
        .unwrap();
    let plan = plan_release(
        &index,
        selected,
        &installed.active,
        &HostState::default(),
        &BTreeMap::new(),
    );
    assert!(plan.blockers.is_empty());
    assert_eq!(plan.releases.len(), 2);
    assert_eq!(
        plan.releases[0].manifest.id,
        "fixture.provenance-dependency"
    );
    assert!(matches!(&plan.changes[0], Change::Update { .. }));
}
