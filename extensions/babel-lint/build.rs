//! Unpacks the Babel data bundle pinned by `babel-data.json` (Babel platform data Exaudio doesn't
//! carry in source, see `src/data.rs`) into `OUT_DIR/babel-data`.
//!
//! The bundle is private: it is not downloadable, so builds read it from the per-user build
//! cache (`%LOCALAPPDATA%\Exaudio\build-cache` on Windows, else `$XDG_CACHE_HOME/exaudio/build-cache`
//! or `~/.cache/exaudio/build-cache`), keyed and verified by its SHA-256.
//! `tools/update-babel-data.ps1 -Fetch` puts it there (SSH access to the bundle host required);
//! `tools/update-babel-data.ps1` publishes a new one.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Files `src/data.rs` embeds.
const EMBEDDED: [&str; 3] = [
    "babel-lint-data.json",
    "error-category-names.json",
    "lint-parity.json",
];

fn main() {
    println!("cargo:rerun-if-changed=babel-data.json");
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let pin: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(manifest_dir.join("babel-data.json")).expect("read babel-data.json"),
    )
    .expect("babel-data.json is valid JSON");
    let sha256 = pin["sha256"]
        .as_str()
        .expect("babel-data.json has a string `sha256`");

    let archive = cache_dir().join(format!("babel-data-{sha256}.tar"));
    println!("cargo:rerun-if-changed={}", archive.display());
    if !archive.is_file() {
        panic!(
            "the private Babel data bundle {sha256} is not in the build cache ({}); fetch it with \
             `powershell -File tools/update-babel-data.ps1 -Fetch` (needs SSH access to the bundle host)",
            archive.display()
        );
    }
    let actual = file_sha256(&archive);
    assert_eq!(
        actual,
        sha256,
        "{} is corrupt (SHA-256 {actual}); delete it and fetch it again",
        archive.display()
    );

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("babel-data");
    if out.exists() {
        fs::remove_dir_all(&out).expect("clear OUT_DIR/babel-data");
    }
    let file = fs::File::open(&archive).expect("open cached Babel data bundle");
    tar::Archive::new(file)
        .unpack(&out)
        .unwrap_or_else(|error| panic!("unpack {}: {error}", archive.display()));
    for name in EMBEDDED {
        assert!(
            out.join(name).is_file(),
            "Babel data bundle {sha256} lacks {name}"
        );
    }
}

fn cache_dir() -> PathBuf {
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("Exaudio").join("build-cache");
    }
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").expect("HOME or LOCALAPPDATA is set")).join(".cache")
        });
    cache.join("exaudio").join("build-cache")
}

fn file_sha256(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
