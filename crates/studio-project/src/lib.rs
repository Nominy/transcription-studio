//! Portable, neutral native project persistence.
//!
//! `project.json` is a versioned envelope containing the complete `Project`, including
//! segments and opaque extension metadata. Audio is immutable, content-addressed data
//! under `audio/`; references on disk are always `audio/<filename>`. Loaded projects use
//! absolute paths so playback and Save As do not depend on the process working directory.
//!
//! # Commit, interruption, and restart boundary
//!
//! A save first copies and syncs new audio, then writes and syncs a temporary manifest in
//! the destination directory, then atomically replaces the single manifest. No previously
//! referenced audio is overwritten or deleted. Failure before replacement leaves the old
//! document intact; process interruption exposes either complete old or complete new data,
//! never mixed segments/metadata. Restart by loading the committed manifest and repeating
//! the save; unreferenced audio/temporary files left by a crash are harmless (not deleted
//! automatically). Simultaneous writers are last-commit-wins, not an edit-merging protocol.
//! On Unix directory entries are also synced. On other platforms file data is synced but
//! directory durability across sudden power loss is filesystem-dependent; this is not a
//! promise of hardware-failure durability. A post-commit directory-sync error is reported
//! distinctly as `DurabilityUncertain`: the new manifest has already been committed.
//!
//! # Paths and trust boundary
//!
//! Callers explicitly supply the base for relative input audio paths. Relative input paths
//! may not traverse parents; absolute input paths and caller-selected root/base symlinks
//! are allowed and resolved. External audio symlinks are resolved before copying. Native
//! manifest references must be exactly `audio/<portable filename>`: no absolute paths,
//! parent traversal, backslashes, drive prefixes, or nested directories. Symlinks/reparse
//! points inside the destination (audio, support directories, manifests, or owned files)
//! are rejected, including on read. Destination trees must not be concurrently modified
//! by an adversary: these checks are not an OS sandbox against filesystem TOCTOU attacks.
//! No account, mode, or extension metadata is interpreted by this crate.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use studio_core::{AudioLane, Project, Segment, Speaker};
use tempfile::NamedTempFile;
use thiserror::Error;

const PROJECT_FILE: &str = "project.json";
const RECOVERY_FILE: &str = "autosave/recovery.json";
const FORMAT: &str = "exaudio-project";
const VERSION: u32 = 1;
const SUPPORT_DIRS: [&str; 4] = ["audio", "rules", "journal", "autosave"];

#[derive(Serialize, Deserialize)]
struct ProjectManifest {
    format: String,
    version: u32,
    project: Project,
    /// Recovery snapshots only: the folder the project was saved to when it was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_path: Option<String>,
    /// Recovery snapshots only: when the snapshot was written (Unix milliseconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    saved_at_unix_ms: Option<u64>,
}

/// A recovery snapshot together with where and when it was taken.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoverySnapshot {
    pub project: Project,
    /// Folder the project had been saved to when the snapshot was taken, if any.
    pub native_path: Option<PathBuf>,
    /// When the snapshot was written. Snapshots without a recorded time use the file's
    /// modification time.
    pub saved_at: SystemTime,
}

/// A recovery snapshot and the recovery root it was read from.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryEntry {
    pub root: PathBuf,
    pub snapshot: RecoverySnapshot,
}

#[derive(Deserialize)]
struct LegacyManifest {
    id: String,
    name: String,
    speakers: Vec<Speaker>,
    lanes: Vec<AudioLane>,
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ProjectIoError {
    #[error("{operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to encode project JSON: {source}")]
    EncodeJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to decode `{path}`: {source}")]
    DecodeJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid native project at `{path}`: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("audio for lane `{lane_id}` is unavailable at `{path}`: {source}")]
    AudioSource {
        lane_id: String,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("manifest `{path}` was committed, but directory durability is uncertain: {source}")]
    DurabilityUncertain {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Save a portable project. Relative lane paths are resolved only against `source_base`.
/// Existing unversioned/split-format projects must be explicitly imported and saved to a
/// new destination; they are never silently overwritten with an incompatible format.
pub fn save_project(
    project: &Project,
    root: impl AsRef<Path>,
    source_base: impl AsRef<Path>,
) -> Result<(), ProjectIoError> {
    save_document(
        project,
        root.as_ref(),
        source_base.as_ref(),
        PROJECT_FILE,
        None,
    )
}

/// Reopen a native project, resolving its checked audio references to absolute paths.
pub fn load_project(root: impl AsRef<Path>) -> Result<Project, ProjectIoError> {
    load_document(root.as_ref(), PROJECT_FILE).map(|manifest| manifest.project)
}

/// The project name stored in a native folder's manifest, without resolving audio.
pub fn read_project_name(root: impl AsRef<Path>) -> Result<String, ProjectIoError> {
    #[derive(Deserialize)]
    struct NameOnly {
        project: ProjectName,
    }
    #[derive(Deserialize)]
    struct ProjectName {
        name: String,
    }
    let path = root.as_ref().join(PROJECT_FILE);
    check_regular_file(&path)?;
    read_json::<NameOnly>(&path).map(|manifest| manifest.project.name)
}

/// Atomically replace a recovery snapshot without changing the saved project manifest.
/// It uses exactly the native project format and shared project-owned audio, not a
/// platform envelope, plus the snapshot's save location and time. A recovery-only
/// folder can be created before the first manual save.
pub fn save_recovery(
    snapshot: &RecoverySnapshot,
    root: impl AsRef<Path>,
    source_base: impl AsRef<Path>,
) -> Result<(), ProjectIoError> {
    save_document(
        &snapshot.project,
        root.as_ref(),
        source_base.as_ref(),
        RECOVERY_FILE,
        Some(snapshot),
    )
}

/// Read the neutral recovery snapshot. Missing snapshots return an I/O error; callers
/// decide whether and when to offer recovery. Reading never changes the saved project.
pub fn load_recovery(root: impl AsRef<Path>) -> Result<RecoverySnapshot, ProjectIoError> {
    let root = root.as_ref();
    let manifest = load_document(root, RECOVERY_FILE)?;
    let saved_at = match manifest.saved_at_unix_ms {
        Some(ms) => UNIX_EPOCH + Duration::from_millis(ms),
        None => fs::metadata(root.join(RECOVERY_FILE))
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH),
    };
    Ok(RecoverySnapshot {
        project: manifest.project,
        native_path: manifest.native_path.map(PathBuf::from),
        saved_at,
    })
}

/// True when the project was saved to its folder after the snapshot was taken: the
/// manifest already holds everything the snapshot does. Snapshots of never-saved
/// projects, or whose folder is gone, are never superseded.
pub fn superseded_by_save(snapshot: &RecoverySnapshot) -> bool {
    let Some(path) = &snapshot.native_path else {
        return false;
    };
    fs::metadata(path.join(PROJECT_FILE))
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|saved| saved > snapshot.saved_at)
}

/// Keeps the newest snapshot of each project, newest first, and returns the older
/// duplicates it supersedes. Snapshots belong to the same project when their IDs match
/// and they were saved to the same folder; a snapshot taken before the first save
/// (no folder) is superseded by any newer snapshot of that project.
pub fn newest_per_project(
    mut entries: Vec<RecoveryEntry>,
) -> (Vec<RecoveryEntry>, Vec<RecoveryEntry>) {
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.snapshot.saved_at));
    let mut kept: Vec<RecoveryEntry> = Vec::new();
    let mut superseded = Vec::new();
    for entry in entries {
        let same_project = |newer: &RecoveryEntry| {
            newer.snapshot.project.id == entry.snapshot.project.id
                && (entry.snapshot.native_path.is_none()
                    || newer.snapshot.native_path == entry.snapshot.native_path)
        };
        if kept.iter().any(same_project) {
            superseded.push(entry);
        } else {
            kept.push(entry);
        }
    }
    (kept, superseded)
}

/// Explicitly import the old unversioned `project.json` + `segments.json` format.
/// This does not modify the source folder. Save the returned project to a new native
/// folder; all legacy audio must exist and is resolved relative to the legacy root.
pub fn import_legacy_project(root: impl AsRef<Path>) -> Result<Project, ProjectIoError> {
    let root = canonical_directory(root.as_ref())?;
    let path = root.join(PROJECT_FILE);
    check_regular_file(&path)?;
    let value: serde_json::Value = read_json(&path)?;
    if value.get("version").is_some() || value.get("format").is_some() {
        return Err(invalid(&path, "expected an unversioned legacy manifest"));
    }
    let manifest: LegacyManifest = serde_json::from_value(value)
        .map_err(|source| ProjectIoError::DecodeJson { path, source })?;
    let segments_path = root.join("segments.json");
    check_regular_file(&segments_path)?;
    let segments: Vec<Segment> = read_json(&segments_path)?;
    let mut project = Project {
        id: manifest.id,
        name: manifest.name,
        speakers: manifest.speakers,
        lanes: manifest.lanes,
        segments,
        metadata: manifest.metadata,
    };
    for lane in &mut project.lanes {
        lane.source_path = path_string(&resolve_source(lane, &root, None)?)?;
    }
    Ok(project)
}

fn save_document(
    project: &Project,
    root: &Path,
    source_base: &Path,
    manifest_name: &str,
    snapshot: Option<&RecoverySnapshot>,
) -> Result<(), ProjectIoError> {
    if project
        .segments
        .iter()
        .any(|segment| !segment.start_seconds.is_finite() || !segment.end_seconds.is_finite())
    {
        return Err(invalid(
            &root.join(manifest_name),
            "non-finite segment times cannot round-trip through native JSON",
        ));
    }
    fs::create_dir_all(root).map_err(|e| io_error("create directory", root, e))?;
    let root = canonical_directory(root)?;
    for name in SUPPORT_DIRS {
        ensure_directory(&root.join(name))?;
    }
    let manifest_path = root.join(manifest_name);
    if path_exists(&manifest_path)? {
        read_manifest(&manifest_path)?;
    } else if manifest_name == PROJECT_FILE && path_exists(&root.join("segments.json"))? {
        return Err(invalid(
            &manifest_path,
            "legacy split-format folder: explicitly import and save to a new folder",
        ));
    }

    let audio_dir = root.join("audio");
    let mut stored = project.clone();
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    for lane in &mut stored.lanes {
        let source = resolve_source(lane, source_base, Some(&audio_dir))?;
        let reference = if let Some(reference) = sources.get(&source) {
            reference.clone()
        } else {
            let reference = materialize_audio(&source, &audio_dir)?;
            sources.insert(source, reference.clone());
            reference
        };
        lane.source_path = reference;
    }
    sync_directory(&audio_dir).map_err(|e| io_error("sync audio directory", &audio_dir, e))?;
    sync_directory(&root).map_err(|e| io_error("sync project directory", &root, e))?;

    let native_path = match snapshot.and_then(|snapshot| snapshot.native_path.as_deref()) {
        Some(path) => Some(path_string(path)?),
        None => None,
    };
    let manifest = ProjectManifest {
        format: FORMAT.to_string(),
        version: VERSION,
        project: stored,
        native_path,
        saved_at_unix_ms: snapshot.map(|snapshot| {
            u64::try_from(
                snapshot
                    .saved_at
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            )
            .unwrap_or(u64::MAX)
        }),
    };
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|source| ProjectIoError::EncodeJson { source })?;
    let parent = manifest_path.parent().expect("manifest has a parent");
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|e| io_error("create temporary manifest", parent, e))?;
    temporary
        .write_all(&bytes)
        .map_err(|e| io_error("write temporary manifest", &manifest_path, e))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| io_error("sync temporary manifest", &manifest_path, e))?;
    // NamedTempFile::persist atomically replaces on Windows as well as Unix. Never
    // remove the old destination first: it is the only commit record.
    temporary
        .persist(&manifest_path)
        .map_err(|e| io_error("commit manifest", &manifest_path, e.error))?;
    sync_directory(parent).map_err(|source| ProjectIoError::DurabilityUncertain {
        path: manifest_path.clone(),
        source,
    })?;
    Ok(())
}

fn load_document(root: &Path, manifest_name: &str) -> Result<ProjectManifest, ProjectIoError> {
    let root = canonical_directory(root)?;
    check_directory(&root.join("audio"))?;
    if manifest_name == RECOVERY_FILE {
        check_directory(&root.join("autosave"))?;
    }
    let mut manifest = read_manifest(&root.join(manifest_name))?;
    for lane in &mut manifest.project.lanes {
        let name = lane
            .source_path
            .strip_prefix("audio/")
            .filter(|name| portable_filename(name))
            .ok_or_else(|| {
                invalid(
                    &root.join(&lane.source_path),
                    "audio reference must be audio/<portable filename>",
                )
            })?;
        let path = root.join("audio").join(name);
        check_regular_file(&path)?;
        lane.source_path = path_string(&path)?;
    }
    Ok(manifest)
}

fn read_manifest(path: &Path) -> Result<ProjectManifest, ProjectIoError> {
    check_regular_file(path)?;
    let value: serde_json::Value = read_json(path)?;
    if value.get("format").and_then(|v| v.as_str()) != Some(FORMAT)
        || value.get("version").and_then(|v| v.as_u64()) != Some(u64::from(VERSION))
    {
        return Err(invalid(
            path,
            "unsupported native format/version; unversioned projects require explicit legacy import",
        ));
    }
    serde_json::from_value(value).map_err(|source| ProjectIoError::DecodeJson {
        path: path.to_path_buf(),
        source,
    })
}

fn resolve_source(
    lane: &AudioLane,
    base: &Path,
    owned_audio: Option<&Path>,
) -> Result<PathBuf, ProjectIoError> {
    let path = Path::new(&lane.source_path);
    if lane.source_path.is_empty()
        || (!path.is_absolute()
            && path
                .components()
                .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)))
    {
        return Err(invalid(
            path,
            format!(
                "lane `{}` has an empty or escaping relative source path",
                lane.id
            ),
        ));
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let source_error = |source| ProjectIoError::AudioSource {
        lane_id: lane.id.clone(),
        path: path.clone(),
        source,
    };
    if let (Some(owned_audio), Some(parent)) = (owned_audio, path.parent()) {
        let parent = fs::canonicalize(parent).map_err(source_error)?;
        if parent == owned_audio {
            check_regular_file(&path)?;
        }
    }
    let canonical = fs::canonicalize(&path).map_err(source_error)?;
    if !fs::metadata(&canonical).map_err(source_error)?.is_file() {
        return Err(source_error(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source is not a regular file",
        )));
    }
    Ok(canonical)
}

fn materialize_audio(source: &Path, audio_dir: &Path) -> Result<String, ProjectIoError> {
    if source.parent() == Some(audio_dir) {
        check_regular_file(source)?;
        if let Some(name) = source
            .file_name()
            .and_then(|v| v.to_str())
            .filter(|name| portable_filename(name))
        {
            return Ok(format!("audio/{name}"));
        }
    }
    let mut input = File::open(source).map_err(|e| io_error("open audio source", source, e))?;
    let mut temporary = NamedTempFile::new_in(audio_dir)
        .map_err(|e| io_error("create temporary audio", audio_dir, e))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|e| io_error("read audio source", source, e))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        temporary
            .write_all(&buffer[..count])
            .map_err(|e| io_error("copy audio", audio_dir, e))?;
    }
    let hash = format!("{:x}", digest.finalize());
    let extension = source
        .extension()
        .and_then(|v| v.to_str())
        .filter(|ext| {
            !ext.is_empty() && ext.len() <= 16 && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase);
    let name = match extension {
        Some(ext) => format!("{hash}.{ext}"),
        None => hash.clone(),
    };
    let target = audio_dir.join(&name);
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| io_error("sync audio", &target, e))?;
    match temporary.persist_noclobber(&target) {
        Ok(_) => {}
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            // Repeated saves of an external source share existing bytes; never replace
            // an audio file referenced by an earlier committed manifest.
            check_regular_file(&target)?;
            let mut existing =
                File::open(&target).map_err(|e| io_error("open owned audio", &target, e))?;
            let mut digest = Sha256::new();
            loop {
                let count = existing
                    .read(&mut buffer)
                    .map_err(|e| io_error("read owned audio", &target, e))?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            if format!("{:x}", digest.finalize()) != hash {
                return Err(invalid(
                    &target,
                    "existing content-addressed audio has different bytes",
                ));
            }
        }
        Err(error) => return Err(io_error("commit audio", &target, error.error)),
    }
    Ok(format!("audio/{name}"))
}

fn portable_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .iter()
        .any(|reserved| {
            name.split('.')
                .next()
                .unwrap_or("")
                .eq_ignore_ascii_case(reserved)
        })
}

fn path_string(path: &Path) -> Result<String, ProjectIoError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid(path, "audio path cannot be represented as UTF-8"))
}

fn canonical_directory(path: &Path) -> Result<PathBuf, ProjectIoError> {
    let root = fs::canonicalize(path).map_err(|e| io_error("resolve directory", path, e))?;
    check_directory(&root)?;
    Ok(root)
}

fn ensure_directory(path: &Path) -> Result<(), ProjectIoError> {
    if !path_exists(path)? {
        fs::create_dir(path).map_err(|e| io_error("create directory", path, e))?;
    }
    check_directory(path)
}

fn path_exists(path: &Path) -> Result<bool, ProjectIoError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_error("inspect path", path, e)),
    }
}

fn check_directory(path: &Path) -> Result<(), ProjectIoError> {
    check_type(path, true)
}

fn check_regular_file(path: &Path) -> Result<(), ProjectIoError> {
    check_type(path, false)
}

fn check_type(path: &Path, directory: bool) -> Result<(), ProjectIoError> {
    let metadata = fs::symlink_metadata(path).map_err(|e| io_error("inspect path", path, e))?;
    let linked = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        linked || metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    };
    if linked || (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(invalid(
            path,
            if directory {
                "expected a directory, not a link/reparse point"
            } else {
                "expected a regular file, not a link/reparse point"
            },
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn invalid(path: &Path, reason: impl Into<String>) -> ProjectIoError {
    ProjectIoError::Invalid {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> ProjectIoError {
    ProjectIoError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ProjectIoError> {
    let bytes = fs::read(path).map_err(|e| io_error("read file", path, e))?;
    serde_json::from_slice(&bytes).map_err(|source| ProjectIoError::DecodeJson {
        path: path.to_path_buf(),
        source,
    })
}
