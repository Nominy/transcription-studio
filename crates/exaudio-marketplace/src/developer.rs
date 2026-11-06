use crate::{
    Limits, OperationControl, PermissionGrants, Release, Result, invalid,
    package::{PackageMetadata, extract_contents},
};
use exaudio_extension::ExtensionManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

/// Unreviewed local source identity. Preview does not grant trust or permissions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeveloperPreview {
    pub source: PathBuf,
    pub manifest: ExtensionManifest,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct DeveloperInstallRequest {
    pub preview: DeveloperPreview,
    /// Separate, explicit consent to the previewed source digest. Never auto-populated
    /// by the backend; the host collects it after displaying the unreviewed-source warning.
    pub approved_sha256: String,
    pub dependencies: Vec<Release>,
    pub approved_permissions: PermissionGrants,
    pub expected_generation: u64,
}

pub fn preview_developer_package(path: &Path, limits: Limits) -> Result<DeveloperPreview> {
    preview_developer_package_with_progress(path, limits, &OperationControl::default())
}

pub fn preview_developer_package_with_progress(
    path: &Path,
    limits: Limits,
    control: &OperationControl,
) -> Result<DeveloperPreview> {
    control.check()?;
    let source = std::fs::canonicalize(path)?;
    let bytes = read_package(&source, limits)?;
    let manifest = {
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes))?;
        if archive.len() > limits.files {
            return Err(invalid("package entry count exceeds limit"));
        }
        let entry = archive.by_name("manifest.json")?;
        if entry.size() > limits.manifest_bytes {
            return Err(invalid("oversized manifest"));
        }
        let mut manifest = Vec::new();
        entry
            .take(limits.manifest_bytes.saturating_add(1))
            .read_to_end(&mut manifest)?;
        if manifest.len() as u64 > limits.manifest_bytes {
            return Err(invalid("oversized manifest"));
        }
        serde_json::from_slice::<ExtensionManifest>(&manifest)?
    };
    let preview = DeveloperPreview {
        source,
        manifest,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    let temporary = tempfile::tempdir()?;
    extract_contents(
        &bytes,
        PackageMetadata {
            manifest: &preview.manifest,
            bytes: preview.bytes,
            sha256: &preview.sha256,
        },
        limits,
        temporary.path(),
        control,
    )?;
    Ok(preview)
}

pub(crate) fn read_package(path: &Path, limits: Limits) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > limits.compressed_bytes {
        return Err(invalid("package exceeds compressed byte limit"));
    }
    let mut bytes = Vec::new();
    file.take(limits.compressed_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limits.compressed_bytes {
        return Err(invalid("package exceeds compressed byte limit"));
    }
    Ok(bytes)
}
