use crate::Result;
use exaudio_extension::{ExtensionManifest, Permission, Runtime};
use exaudio_marketplace::{Catalog, CatalogExtension, Repository};
use semver::Version;
use std::{collections::BTreeMap, path::Path};

pub struct Spec<'a> {
    pub owner: &'a str,
    pub repository: &'a str,
    pub id: &'a str,
    pub version: &'a str,
    pub sdk_repository: &'a str,
    pub sdk_commit: &'a str,
    pub destination: &'a Path,
}

pub fn create(spec: Spec<'_>) -> Result<()> {
    let files = render(&spec)?;
    if spec.destination.try_exists()? {
        return Err("author destination already exists; refusing to overwrite it".into());
    }
    let parent = spec
        .destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staging = tempfile::tempdir_in(parent)?;
    for (name, content) in files {
        let path = staging.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    // The complete generated repository becomes visible together. No GitHub API, build,
    // download, repository initialization or publication happens in this command.
    std::fs::rename(staging.path(), spec.destination)?;
    println!(
        "Created standalone author repository layout at {}",
        spec.destination.display()
    );
    println!(
        "SDK/validator pinned to {} at {}; no remote publication was performed.",
        spec.sdk_repository, spec.sdk_commit
    );
    Ok(())
}

fn render(spec: &Spec<'_>) -> Result<BTreeMap<&'static str, String>> {
    validate_repository(spec.owner, spec.repository, spec.id)?;
    let sdk_path = spec
        .sdk_repository
        .strip_prefix("https://github.com/")
        .ok_or("SDK source must be an explicit HTTPS github.com repository URL")?;
    let sdk_path = sdk_path.strip_suffix(".git").unwrap_or(sdk_path);
    let (sdk_owner, sdk_repository) = sdk_path
        .split_once('/')
        .ok_or("SDK source requires owner/repository")?;
    validate_repository(sdk_owner, sdk_repository, spec.id)?;
    if !matches!(spec.sdk_commit.len(), 40 | 64)
        || !spec
            .sdk_commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("SDK commit must be a full lowercase Git commit ID, never a moving branch or invented default tag".into());
    }
    let version: Version = spec.version.parse()?;
    if !version.build.is_empty() || format!("{}-{version}.exaudio", spec.id).len() > 128 {
        return Err(
            "author release version/ID must form a path-safe asset name of at most 128 bytes"
                .into(),
        );
    }
    let manifest = ExtensionManifest {
        id: spec.id.to_owned(), name: spec.id.to_owned(), version: version.clone(),
        api: format!("^{}", exaudio_extension::API_VERSION).parse()?, runtime: Runtime::Wasm,
        description: "An independently authored panel and command with undoable greeting insertion and empty-segment diagnostics.".into(),
        license: "MIT".into(), dependencies: BTreeMap::new(),
        permissions: vec![Permission::DocumentRead, Permission::DocumentEdit], modes: Vec::new(),
        providers: Vec::new(),
    };
    manifest.validate()?;
    let crate_name = spec.id.replace('.', "-");
    let artifact = crate_name.replace('-', "_");
    let cargo = format!(
        "[package]\nname = {crate_name:?}\nversion = {version:?}\nedition = \"2024\"\nlicense = \"MIT\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nexaudio-sdk = {{ git = {sdk:?}, rev = {revision:?} }}\n\n[workspace]\n\n[profile.release]\nlto = true\nopt-level = \"s\"\npanic = \"abort\"\nstrip = true\n",
        version = version.to_string(),
        sdk = spec.sdk_repository,
        revision = spec.sdk_commit
    );
    let replacements = [
        ("@@OWNER@@", spec.owner),
        ("@@REPOSITORY@@", spec.repository),
        ("@@EXTENSION_ID@@", spec.id),
        ("@@VERSION@@", spec.version),
        ("@@SDK_URL@@", spec.sdk_repository),
        ("@@SDK_COMMIT@@", spec.sdk_commit),
        ("@@CRATE_ARTIFACT@@", artifact.as_str()),
    ];
    let render_template = |template: &str| {
        replacements
            .iter()
            .fold(template.to_owned(), |text, (token, replacement)| {
                text.replace(*token, replacement)
            })
    };
    let license = format!(
        "Copyright (c) {}\n\n{}",
        spec.owner,
        include_str!("../../../extensions/examples/hello-panel/LICENSE")
    );
    Ok(BTreeMap::from([
        ("Cargo.toml", cargo),
        (
            "manifest.json",
            format!("{}\n", serde_json::to_string_pretty(&manifest)?),
        ),
        (
            "src/lib.rs",
            include_str!("../author-template/lib.rs").to_owned(),
        ),
        (
            "rust-toolchain.toml",
            include_str!("../../../rust-toolchain.toml").to_owned(),
        ),
        (
            ".github/workflows/release.yml",
            render_template(include_str!("../author-template/release.yml.in")),
        ),
        (
            "README.md",
            render_template(include_str!("../author-template/README.md.in")),
        ),
        ("LICENSE", license),
        (
            ".gitignore",
            "/target/\n/dist/\n/module.wasm\n/*.exaudio\n".to_owned(),
        ),
    ]))
}

fn validate_repository(owner: &str, repository: &str, id: &str) -> Result<()> {
    Catalog {
        schema: 1,
        extensions: vec![CatalogExtension {
            id: id.to_owned(),
            publisher: owner.to_owned(),
            repository: Repository {
                owner: owner.to_owned(),
                name: repository.to_owned(),
            },
            keywords: Vec::new(),
            releases: Vec::new(),
        }],
    }
    .validate()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_repository_has_pinned_git_sdk_and_complete_release_workflow() {
        let spec = Spec {
            owner: "fixture",
            repository: "author-extension",
            id: "fixture.author",
            version: "1.0.0",
            sdk_repository: "https://github.com/fixture/exaudio",
            sdk_commit: "0123456789abcdef0123456789abcdef01234567",
            destination: Path::new("unused"),
        };
        let files = render(&spec).unwrap();
        assert!(
            files["Cargo.toml"]
                .contains("exaudio-sdk = { git = \"https://github.com/fixture/exaudio\"")
        );
        assert!(files["Cargo.toml"].contains(spec.sdk_commit));
        assert!(!files["Cargo.toml"].contains("path ="));
        assert!(files.values().all(|content| !content.contains("@@")));
        let workflow = &files[".github/workflows/release.yml"];
        for required in [
            "wasm-tools component new",
            "exaudio-extension-index pack",
            "exaudio-extension-index release-entry",
            "gh release create",
            "--verify-tag",
            "contents: read",
            "contents: write",
        ] {
            assert!(workflow.contains(required), "missing {required}");
        }
        assert!(files["src/lib.rs"].contains("host::propose"));
        assert!(!files["src/lib.rs"].contains("runtime-probes"));
        let manifest: ExtensionManifest = serde_json::from_str(&files["manifest.json"]).unwrap();
        assert_eq!(manifest.id, spec.id);
    }

    #[test]
    fn rejects_unpinned_or_injected_author_sources() {
        let mut spec = Spec {
            owner: "fixture",
            repository: "author-extension",
            id: "fixture.author",
            version: "1.0.0",
            sdk_repository: "https://github.com/fixture/exaudio",
            sdk_commit: "main",
            destination: Path::new("unused"),
        };
        assert!(render(&spec).is_err());
        spec.sdk_commit = "0123456789abcdef0123456789abcdef01234567";
        spec.sdk_repository = "https://github.com/fixture/exaudio'; echo injected";
        assert!(render(&spec).is_err());
        spec.sdk_repository = "http://127.0.0.1/sdk";
        assert!(render(&spec).is_err());
    }
}
