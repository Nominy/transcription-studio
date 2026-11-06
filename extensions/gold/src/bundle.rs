//! Gold's unpacked host build, downloaded on demand from the release pinned in
//! `gold-inference.json` (written by `tools/update-gold-inference.ps1`).
//!
//! The zip is SHA-256-verified while streaming, extracted into a staging folder, checked
//! (`inference.html`, manifest key → pinned extension id) and only then renamed into
//! `<root>/<version>-<commit>/`, so an interrupted download or extract never activates. An
//! installed folder is reused without network access. The active build and the most recent
//! previous one are kept; older builds and abandoned staging files are removed.

use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::Engine as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use studio_inference::{InferenceError, InferenceProgress, InferenceResult, ProgressSink};

/// The Gold build this Studio version runs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldBuildPin {
    /// Gold extension version (`manifest.json` `version`).
    pub version: String,
    /// Gold source commit (short sha) the build was made from.
    pub commit: String,
    pub url: String,
    /// Lower-case hex SHA-256 of the zip.
    pub sha256: String,
    pub bytes: u64,
    /// Chromium extension id derived from the build's manifest key.
    pub extension_id: String,
}

impl GoldBuildPin {
    /// The pin compiled into this Studio build.
    pub fn bundled() -> Self {
        serde_json::from_str(include_str!("../gold-inference.json"))
            .expect("extensions/gold/gold-inference.json is a valid Gold build pin")
    }

    /// Folder name of this build under the builds root.
    pub fn release(&self) -> String {
        format!("{}-{}", self.version, self.commit)
    }
}

/// Progress stage names reported while the Gold build is fetched.
pub const DOWNLOAD_STAGE: &str = "downloading-gold-runtime";
pub const EXTRACT_STAGE: &str = "extracting-gold-runtime";

/// Leftover `.download-*` / `.staging-*` entries older than this belong to dead processes.
const ABANDONED_AFTER: Duration = Duration::from_secs(60 * 60);

/// Installs and locates pinned Gold builds under `root`.
pub struct GoldBuilds {
    root: PathBuf,
    pin: GoldBuildPin,
    client: reqwest::Client,
    installing: tokio::sync::Mutex<()>,
}

impl GoldBuilds {
    pub fn new(root: PathBuf, pin: GoldBuildPin) -> Self {
        Self {
            root,
            pin,
            client: reqwest::Client::new(),
            installing: tokio::sync::Mutex::new(()),
        }
    }

    pub fn pin(&self) -> &GoldBuildPin {
        &self.pin
    }

    /// The installed folder of the pinned build, if present.
    pub fn installed(&self) -> Option<PathBuf> {
        let dir = self.root.join(self.pin.release());
        dir.join("inference.html").is_file().then_some(dir)
    }

    /// The pinned build's folder, downloading and installing it first when missing.
    pub async fn ensure(&self, progress: &ProgressSink) -> InferenceResult<PathBuf> {
        if let Some(dir) = self.installed() {
            return Ok(dir);
        }
        let _installing = self.installing.lock().await;
        if let Some(dir) = self.installed() {
            return Ok(dir);
        }
        std::fs::create_dir_all(&self.root)
            .map_err(|error| io_error("creating", &self.root, error))?;
        let release = self.pin.release();
        let archive = self
            .root
            .join(format!(".download-{release}-{}.zip", std::process::id()));
        let downloaded = self.download(&archive, progress).await;
        let installed = match downloaded {
            Ok(()) => {
                progress.emit(InferenceProgress::Stage {
                    stage: EXTRACT_STAGE.to_string(),
                    completed: 0,
                    total: None,
                    detail: Some(release.clone()),
                    clip: None,
                });
                let root = self.root.clone();
                let pin = self.pin.clone();
                let archive = archive.clone();
                tokio::task::spawn_blocking(move || install_archive(&root, &pin, &archive))
                    .await
                    .map_err(|error| {
                        InferenceError::other(format!("Couldn't install the AI engine. ({error})"))
                    })
                    .and_then(|result| result)
            }
            Err(error) => Err(error),
        };
        let _ = std::fs::remove_file(&archive);
        let dir = installed?;
        prune(&self.root, &release);
        Ok(dir)
    }

    async fn download(&self, archive: &Path, progress: &ProgressSink) -> InferenceResult<()> {
        let failed = |detail: String| {
            InferenceError::other(format!(
                "Couldn't download the AI engine. ({} from {}: {detail})",
                self.pin.release(),
                self.pin.url
            ))
        };
        let mut response = self
            .client
            .get(&self.pin.url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| failed(error.to_string()))?;
        let mut file =
            File::create(archive).map_err(|error| io_error("creating", archive, error))?;
        let mut hasher = Sha256::new();
        let mut received = 0u64;
        progress.emit(download_progress(0, self.pin.bytes));
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| failed(error.to_string()))?
        {
            received += chunk.len() as u64;
            if received > self.pin.bytes {
                return Err(failed(format!(
                    "more than the pinned {} bytes",
                    self.pin.bytes
                )));
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .map_err(|error| io_error("writing", archive, error))?;
            progress.emit(download_progress(received, self.pin.bytes));
        }
        file.sync_all()
            .map_err(|error| io_error("writing", archive, error))?;
        if received != self.pin.bytes {
            return Err(failed(format!(
                "received {received} of the pinned {} bytes",
                self.pin.bytes
            )));
        }
        let sha256 = hex(&hasher.finalize());
        if !sha256.eq_ignore_ascii_case(&self.pin.sha256) {
            return Err(failed(format!(
                "SHA-256 {sha256} does not match the pinned {}",
                self.pin.sha256
            )));
        }
        Ok(())
    }
}

fn download_progress(completed: u64, total: u64) -> InferenceProgress {
    InferenceProgress::Stage {
        stage: DOWNLOAD_STAGE.to_string(),
        completed,
        total: Some(total),
        detail: None,
        clip: None,
    }
}

/// Extracts the verified `archive` into a staging folder, checks it and activates it.
fn install_archive(root: &Path, pin: &GoldBuildPin, archive: &Path) -> InferenceResult<PathBuf> {
    let release = pin.release();
    let staging = root.join(format!(".staging-{release}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let result = extract(archive, &staging).and_then(|()| check_build(&staging, pin));
    let target = root.join(&release);
    let activated = result.and_then(|()| match std::fs::rename(&staging, &target) {
        Ok(()) => Ok(target.clone()),
        // Another Studio process activated the same release first.
        Err(_) if target.join("inference.html").is_file() => Ok(target.clone()),
        Err(error) => Err(io_error("activating", &target, error)),
    });
    let _ = std::fs::remove_dir_all(&staging);
    activated
}

fn extract(archive: &Path, destination: &Path) -> InferenceResult<()> {
    let invalid = |detail: String| {
        InferenceError::other(format!(
            "Couldn't install the AI engine. (archive is invalid: {detail})"
        ))
    };
    let file = File::open(archive).map_err(|error| io_error("opening", archive, error))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| invalid(error.to_string()))?;
    std::fs::create_dir_all(destination)
        .map_err(|error| io_error("creating", destination, error))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| invalid(error.to_string()))?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| invalid(format!("unsafe entry name {:?}", entry.name())))?;
        let path = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|error| io_error("creating", &path, error))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| io_error("creating", parent, error))?;
        }
        let mut output = File::create(&path).map_err(|error| io_error("creating", &path, error))?;
        std::io::copy(&mut entry, &mut output).map_err(|error| invalid(error.to_string()))?;
    }
    Ok(())
}

/// The extracted build must be a Gold host build with the pinned extension identity.
fn check_build(dir: &Path, pin: &GoldBuildPin) -> InferenceResult<()> {
    let invalid = |detail: String| {
        InferenceError::other(format!(
            "Couldn't install the AI engine. (archive is invalid: {detail})"
        ))
    };
    if !dir.join("inference.html").is_file() {
        return Err(invalid("inference.html is missing".to_string()));
    }
    #[derive(Deserialize)]
    struct Manifest {
        version: String,
        key: Option<String>,
    }
    let manifest = std::fs::read(dir.join("manifest.json"))
        .map_err(|error| invalid(format!("manifest.json: {error}")))?;
    let manifest: Manifest = serde_json::from_slice(&manifest)
        .map_err(|error| invalid(format!("manifest.json: {error}")))?;
    if manifest.version != pin.version {
        return Err(invalid(format!(
            "it is Gold {}, the pin says {}",
            manifest.version, pin.version
        )));
    }
    let id = manifest
        .key
        .as_deref()
        .and_then(extension_id)
        .ok_or_else(|| invalid("manifest.json has no valid key".to_string()))?;
    if id != pin.extension_id {
        return Err(invalid(format!(
            "its extension id is {id}, the pin says {}",
            pin.extension_id
        )));
    }
    Ok(())
}

/// Chromium's extension id: the first 128 bits of SHA-256(public key DER), one letter `a`–`p`
/// per nibble.
fn extension_id(base64_key: &str) -> Option<String> {
    let key = base64::engine::general_purpose::STANDARD
        .decode(base64_key)
        .ok()?;
    let hash = Sha256::digest(&key);
    Some(
        hash[..16]
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 15])
            .map(|nibble| char::from(b'a' + nibble))
            .collect(),
    )
}

/// Keeps `active` and the most recently activated other build; removes older builds and
/// abandoned downloads/staging folders.
fn prune(root: &Path, active: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut builds = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok();
        if name.starts_with('.') {
            let abandoned = modified
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > ABANDONED_AFTER);
            if abandoned {
                remove_path(&entry.path());
            }
        } else if name != active {
            builds.push((modified, entry.path()));
        }
    }
    builds.sort_by_key(|build| std::cmp::Reverse(build.0));
    for (_, path) in builds.into_iter().skip(1) {
        remove_path(&path);
    }
}

fn remove_path(path: &Path) {
    let _ = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> InferenceError {
    InferenceError::other(format!(
        "Couldn't install the AI engine. ({action} {}: {error})",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Cursor,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{Router, routing::get};

    /// A manifest key whose Chromium id is computed by [`extension_id`] (checked against
    /// Gold's real key below).
    const KEY: &str = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA8/EczXM+oDWjjP2AbSlWFre68CfcO9m0I/B4NWefskYGYTxxG5U1uXT9cYwKy+G9ggzUUbJSU5wLxOUkxP4B1Wao7amYn1T4/DUXGp+/FvWf/oheaH93b5n+3vgHQGG4ycmVBnN7h909YHwtcYsqrRKud/thsEz385m+ODlkTKzNSVpXGkQxxBx8KnM7BMubXpsqTX37TLWrRvfmZ2ElwjZuvg86P56R1RiG+AuLW1GnVCA952Rpach2UblgYxLWv97ugqdf146HQSrDYvyX9DyrBcIIu5yqY6vZBCB+zstiyQK8JASlsagws8yOkIWIaIKB3I1pGQloHIIO3kNMtQIDAQAB";
    const KEY_ID: &str = "lpheeelinpiegglaogcfhnenmjiebadd";

    fn build_zip(version: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(format!(r#"{{"version":"{version}","key":"{KEY}"}}"#).as_bytes())
            .unwrap();
        zip.start_file("inference.html", options).unwrap();
        zip.write_all(b"<!doctype html>").unwrap();
        zip.start_file("dist/workers/a.js", options).unwrap();
        zip.write_all(b"1").unwrap();
        zip.finish().unwrap().into_inner()
    }

    fn pin_for(body: &[u8], url: String) -> GoldBuildPin {
        GoldBuildPin {
            version: "1.0.0".to_string(),
            commit: "abc123".to_string(),
            url,
            sha256: hex(&Sha256::digest(body)),
            bytes: body.len() as u64,
            extension_id: KEY_ID.to_string(),
        }
    }

    /// Serves `body` at `/build.zip`; returns its URL and a request counter.
    async fn serve(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = Router::new().route(
            "/build.zip",
            get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                let body = body.clone();
                async move { body }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}/build.zip"), hits)
    }

    fn visible_entries(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn extension_id_matches_chromium() {
        assert_eq!(extension_id(KEY).as_deref(), Some(KEY_ID));
    }

    #[test]
    fn bundled_pin_is_valid() {
        let pin = GoldBuildPin::bundled();
        assert_eq!(pin.sha256.len(), 64);
        assert!(
            pin.url
                .ends_with(&format!("/{}/gold-inference.zip", pin.release()))
        );
    }

    #[tokio::test]
    async fn installs_verified_build_and_reuses_it_offline() {
        let body = build_zip("1.0.0");
        let (url, hits) = serve(body.clone()).await;
        let root = tempfile::tempdir().unwrap();
        let pin = pin_for(&body, url);
        let builds = GoldBuilds::new(root.path().to_path_buf(), pin.clone());
        let dir = builds.ensure(&ProgressSink::none()).await.unwrap();
        assert_eq!(dir, root.path().join("1.0.0-abc123"));
        assert!(dir.join("dist/workers/a.js").is_file());
        assert_eq!(visible_entries(root.path()), ["1.0.0-abc123"]);

        // Installed: no network at all, even with an unreachable URL.
        let offline = GoldBuilds::new(
            root.path().to_path_buf(),
            GoldBuildPin {
                url: "http://127.0.0.1:9/unreachable.zip".to_string(),
                ..pin
            },
        );
        assert_eq!(offline.ensure(&ProgressSink::none()).await.unwrap(), dir);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sha_mismatch_is_rejected() {
        let body = build_zip("1.0.0");
        let (url, _) = serve(body.clone()).await;
        let root = tempfile::tempdir().unwrap();
        let mut pin = pin_for(&body, url);
        pin.sha256 = "0".repeat(64);
        let builds = GoldBuilds::new(root.path().to_path_buf(), pin);
        let error = builds.ensure(&ProgressSink::none()).await.unwrap_err();
        assert!(error.message.contains("does not match"), "{error}");
        assert!(builds.installed().is_none());
        assert!(visible_entries(root.path()).is_empty());
    }

    #[tokio::test]
    async fn size_mismatch_is_rejected() {
        let body = build_zip("1.0.0");
        let (url, _) = serve(body.clone()).await;
        let root = tempfile::tempdir().unwrap();
        let mut pin = pin_for(&body, url);
        pin.bytes -= 1;
        let builds = GoldBuilds::new(root.path().to_path_buf(), pin);
        assert!(builds.ensure(&ProgressSink::none()).await.is_err());
        assert!(visible_entries(root.path()).is_empty());
    }

    #[tokio::test]
    async fn failed_extract_does_not_activate() {
        // A pinned (hash-correct) but truncated archive: extraction fails mid-way.
        let mut body = build_zip("1.0.0");
        body.truncate(body.len() / 2);
        let (url, _) = serve(body.clone()).await;
        let root = tempfile::tempdir().unwrap();
        // Leftover staging folder of an earlier interrupted extract, complete-looking.
        let leftover = root.path().join(".staging-1.0.0-abc123-1");
        std::fs::create_dir_all(&leftover).unwrap();
        std::fs::write(leftover.join("inference.html"), b"x").unwrap();
        let builds = GoldBuilds::new(root.path().to_path_buf(), pin_for(&body, url));
        assert!(builds.installed().is_none());
        let error = builds.ensure(&ProgressSink::none()).await.unwrap_err();
        assert!(error.message.contains("archive is invalid"), "{error}");
        assert!(builds.installed().is_none());
        assert!(!root.path().join("1.0.0-abc123").exists());
    }

    #[tokio::test]
    async fn foreign_build_is_rejected() {
        let body = build_zip("2.0.0");
        let (url, _) = serve(body.clone()).await;
        let root = tempfile::tempdir().unwrap();
        let builds = GoldBuilds::new(root.path().to_path_buf(), pin_for(&body, url));
        let error = builds.ensure(&ProgressSink::none()).await.unwrap_err();
        assert!(error.message.contains("pin says 1.0.0"), "{error}");
        assert!(builds.installed().is_none());
    }

    #[tokio::test]
    async fn keeps_active_and_previous_build() {
        let body = build_zip("1.0.0");
        let (url, _) = serve(body.clone()).await;
        let root = tempfile::tempdir().unwrap();
        for (index, old) in ["0.8.0-old", "0.9.0-prev"].iter().enumerate() {
            let dir = root.path().join(old);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("inference.html"), b"x").unwrap();
            if index == 0 {
                // Make the older build clearly older.
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let builds = GoldBuilds::new(root.path().to_path_buf(), pin_for(&body, url));
        builds.ensure(&ProgressSink::none()).await.unwrap();
        assert_eq!(visible_entries(root.path()), ["0.9.0-prev", "1.0.0-abc123"]);
    }
}
