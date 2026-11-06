use std::{
    collections::BTreeSet,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

use exaudio_extension::{ExtensionManifest, Runtime};
use sha2::{Digest, Sha256};

use crate::{OperationControl, Release, Result, Stage, invalid};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub compressed_bytes: u64,
    pub expanded_bytes: u64,
    pub files: usize,
    pub manifest_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            compressed_bytes: 32 * 1024 * 1024,
            expanded_bytes: 128 * 1024 * 1024,
            files: 4096,
            manifest_bytes: 256 * 1024,
        }
    }
}

pub fn check_digest(bytes: &[u8], release: &Release, limits: Limits) -> Result<()> {
    check_package_digest(bytes, release.asset.bytes, &release.asset.sha256, limits)
}

pub(crate) fn check_package_digest(
    bytes: &[u8],
    expected_bytes: u64,
    sha256: &str,
    limits: Limits,
) -> Result<()> {
    if bytes.len() as u64 != expected_bytes || bytes.len() as u64 > limits.compressed_bytes {
        return Err(invalid(
            "package compressed size differs from approved source or exceeds limit",
        ));
    }
    let digest = format!("{:x}", Sha256::digest(bytes));
    if digest != sha256 {
        return Err(invalid("package SHA256 differs from approved source"));
    }
    Ok(())
}

/// Validate a ZIP package without activating it. Uses a private temporary extraction
/// directory, with exactly the same checks as installation.
pub fn validate_package(
    bytes: &[u8],
    release: &Release,
    limits: Limits,
) -> Result<ExtensionManifest> {
    let temporary = tempfile::tempdir()?;
    extract_package(bytes, release, limits, temporary.path())
}

pub(crate) fn extract_package(
    bytes: &[u8],
    release: &Release,
    limits: Limits,
    root: &Path,
) -> Result<ExtensionManifest> {
    if release.manifest.version != release.version {
        return Err(invalid("release manifest version mismatch"));
    }
    extract_contents(
        bytes,
        PackageMetadata {
            manifest: &release.manifest,
            bytes: release.asset.bytes,
            sha256: &release.asset.sha256,
        },
        limits,
        root,
        &OperationControl::default(),
    )
}

pub(crate) struct PackageMetadata<'a> {
    pub manifest: &'a ExtensionManifest,
    pub bytes: u64,
    pub sha256: &'a str,
}

pub(crate) fn extract_contents(
    bytes: &[u8],
    expected: PackageMetadata<'_>,
    limits: Limits,
    root: &Path,
    control: &OperationControl,
) -> Result<ExtensionManifest> {
    control.check()?;
    check_package_digest(bytes, expected.bytes, expected.sha256, limits)?;
    expected
        .manifest
        .validate()
        .map_err(|e| invalid(e.to_string()))?;
    if expected.manifest.runtime != Runtime::Wasm {
        return Err(invalid("only WASM packages can be installed"));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    if archive.len() > limits.files {
        return Err(invalid("package entry count exceeds limit"));
    }
    let entries = archive.len() as u64;
    control.emit(
        Stage::Extracting,
        Some(&expected.manifest.id),
        0,
        Some(entries),
    );
    let mut paths = BTreeSet::new();
    let mut expanded = 0_u64;
    let mut manifest = None;
    let mut module = false;
    for index in 0..archive.len() {
        control.check()?;
        control.emit(
            Stage::Extracting,
            Some(&expected.manifest.id),
            index as u64,
            Some(entries),
        );
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_owned();
        let directory = entry.is_dir();
        let relative = package_path(&name, directory)?;
        if !paths.insert(name.trim_end_matches('/').to_ascii_lowercase()) {
            return Err(invalid("duplicate or case-colliding package path"));
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            if kind != 0 && kind != if directory { 0o040000 } else { 0o100000 } {
                return Err(invalid("symlinks and special files are forbidden"));
            }
        }
        if directory {
            if name != "assets/" && !name.starts_with("assets/") {
                return Err(invalid("unexpected package directory"));
            }
            if entry.size() != 0 {
                return Err(invalid("directory entry contains data"));
            }
            std::fs::create_dir_all(root.join(relative))?;
            continue;
        }
        if name != "manifest.json" && name != "module.wasm" && !name.starts_with("assets/") {
            return Err(invalid(
                "package must contain only manifest.json, module.wasm and assets/",
            ));
        }
        let lower = name.to_ascii_lowercase();
        if [
            ".dll", ".exe", ".so", ".dylib", ".node", ".com", ".bat", ".cmd", ".ps1", ".sh", ".msi",
        ]
        .iter()
        .any(|extension| lower.ends_with(extension))
            || lower.contains(".so.")
        {
            return Err(invalid(
                "native binaries and executable scripts cannot be installed",
            ));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| invalid("expanded size overflow"))?;
        if expanded > limits.expanded_bytes {
            return Err(invalid("package expanded size exceeds limit"));
        }
        if name == "manifest.json" && entry.size() > limits.manifest_bytes {
            return Err(invalid("oversized manifest"));
        }
        let output = root.join(relative);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        let mut remaining = entry.size();
        let mut buffer = [0_u8; 16 * 1024];
        let mut prefix = Vec::with_capacity(8);
        let mut manifest_bytes = Vec::new();
        loop {
            control.check()?;
            let count = entry.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if count as u64 > remaining {
                return Err(invalid("ZIP inflated beyond declared size"));
            }
            remaining -= count as u64;
            let prefix_count = (8 - prefix.len()).min(count);
            prefix.extend_from_slice(&buffer[..prefix_count]);
            if name == "manifest.json" {
                manifest_bytes.extend_from_slice(&buffer[..count]);
            }
            file.write_all(&buffer[..count])?;
        }
        if remaining != 0 {
            return Err(invalid("ZIP entry was truncated"));
        }
        if native_magic(&prefix) {
            return Err(invalid("native executable content is forbidden"));
        }
        if name == "module.wasm" {
            if prefix.as_slice() != b"\0asm\x0d\0\x01\0" {
                return Err(invalid("module.wasm must be a WebAssembly component"));
            }
            module = true;
        }
        if name == "manifest.json" {
            let parsed: ExtensionManifest = serde_json::from_slice(&manifest_bytes)?;
            if &parsed != expected.manifest {
                return Err(invalid(
                    "package manifest does not exactly match approved source",
                ));
            }
            manifest = Some(parsed);
        }
        file.sync_all()?;
    }
    control.emit(
        Stage::Extracting,
        Some(&expected.manifest.id),
        entries,
        Some(entries),
    );
    if !module {
        return Err(invalid("package has no module.wasm"));
    }
    manifest.ok_or_else(|| invalid("package has no manifest.json"))
}

fn native_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ")
        || bytes.starts_with(b"\x7fELF")
        || [
            b"\xfe\xed\xfa\xce",
            b"\xce\xfa\xed\xfe",
            b"\xfe\xed\xfa\xcf",
            b"\xcf\xfa\xed\xfe",
            b"\xca\xfe\xba\xbe",
            b"\xbe\xba\xfe\xca",
        ]
        .iter()
        .any(|magic| bytes.starts_with(*magic))
}

fn package_path(name: &str, directory: bool) -> Result<PathBuf> {
    if name.is_empty()
        || name.len() > 240
        || name.contains('\\')
        || name.starts_with('/')
        || name.contains(':')
        || name.chars().any(|c| c.is_control())
    {
        return Err(invalid(
            "absolute, drive, backslash or invalid package path",
        ));
    }
    let name = if directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    for component in name.split('/') {
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.contains(['<', '>', '"', '|', '?', '*'])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(invalid("non-portable or traversing package path"));
        }
    }
    Ok(PathBuf::from(name))
}
