# GitHub Actions

The private GitHub repository is expected to build and package the alpha channel without storing server credentials in git.

## CI

`CI` runs on pushes, pull requests, and manual dispatch:

- Gitleaks scans the full git history with redacted output.
- One Ubuntu job runs `cargo fmt --all --check`.
- Windows and macOS Rust jobs wait for formatting, restore OS-specific Cargo caches, run strict Clippy for all workspace targets, and run all workspace tests with `RUST_TEST_THREADS=4`.

The Rust cache primarily reuses the expensive Iced/WGPU, networking, audio, and platform dependency graph. `--locked` remains mandatory.

## Windows Alpha Package

`Windows Alpha Package` is a manual workflow. It restores the repository-pinned Velopack 1.2.0 tool and an OS-specific Rust release cache. `vpk download http` primes only the latest full release needed for delta generation instead of downloading every historical package.

The package step creates a clean `target/velopack/Publish` directory from `assets.alpha.json`. When VPS publication is enabled, only that directory is uploaded to unserved incoming storage, validated by size and SHA-256, and promoted under a remote lock with feed metadata last. The small publish directory is then retained as a three-day, uncompressed workflow artifact.

Defaults:

- Channel: `alpha`
- Feed URL: `https://reviewgen.ovh/transcription-studio/alpha/win-x64`
- Remote path: `/var/www/transcription-studio/alpha/win-x64`

The package version must equal `crates/studio-app/Cargo.toml`; workflow overrides cannot create a package/application version mismatch.

## macOS Alpha Package

`macOS Alpha Package` is a manual matrix workflow. Each runtime restores its own Rust release cache and the pinned Velopack tool, runs one `vpk pack` pass, and creates the same clean `Publish` payload used by Windows. The former redundant `vpk bundle` pass is not part of the release path.

Optional VPS publication uses per-runtime incoming storage, manifest validation, a remote lock, asset-first promotion, feed-last replacement, and rollback metadata. The Sonoma target remains separate until compatibility is proven on a real Sonoma machine.

Defaults:

- Channel: `alpha`
- Target: `all`
- Remote base path: `/var/www/transcription-studio/alpha`

Targets:

- `latest-arm64`: `macos-26`, `osx-arm64`, `https://reviewgen.ovh/transcription-studio/alpha/mac-arm64`.
- `latest-amd64`: `macos-26-intel`, `osx-x64`, `https://reviewgen.ovh/transcription-studio/alpha/mac-x64`.
- `sonoma-amd64`: `macos-14-large`, `osx-x64`, `https://reviewgen.ovh/transcription-studio/alpha/mac-sonoma-x64`.

The Sonoma target exists for Intel Macs on Sonoma 14.x. `macos-14-large` is a larger Intel runner label, so it requires the repository account to have larger macOS runners available; if that label is unavailable, use `latest-amd64` for a macOS 14 deployment-targeted Intel package or add an Intel Sonoma self-hosted runner.

Unsigned runs produce trusted-tester artifacts. If signing secrets are present, the workflow imports the Developer ID certificate, stores a notarytool profile, and passes signing/notarization inputs to `tools/package-alpha-macos.sh`.

Optional macOS signing secrets:

- `APPLE_CERTIFICATE_P12_BASE64`: base64-encoded `.p12` containing Developer ID Application and Installer identities.
- `APPLE_CERTIFICATE_PASSWORD`: password for the `.p12`.
- `APPLE_KEYCHAIN_PASSWORD`: temporary CI keychain password.
- `APPLE_DEVELOPER_ID_APPLICATION`: signing subject, for example `Developer ID Application: Example, Inc. (TEAMID)`.
- `APPLE_DEVELOPER_ID_INSTALLER`: signing subject, for example `Developer ID Installer: Example, Inc. (TEAMID)`.
- `APPLE_NOTARY_PROFILE`: notarytool keychain profile name.
- `APPLE_NOTARY_APPLE_ID`, `APPLE_NOTARY_TEAM_ID`, `APPLE_NOTARY_PASSWORD`: credentials used to create the notarytool profile in CI.

## VPS Publish Secrets

Publishing is disabled by default. To let Actions stage and atomically promote a release on the VPS, add these repository secrets:

- `ALPHA_SSH_HOST`: VPS host or IP.
- `ALPHA_SSH_USER`: SSH user with write access to the incoming and release directories.
- `ALPHA_SSH_PRIVATE_KEY`: private deploy key for that user.
- `ALPHA_SSH_PORT`: optional SSH port, defaults to `22`.

Package jobs inject `STUDIO_BUILD_GIT_COMMIT=${{ github.sha }}` so installed builds retain exact source identity without making local Cargo builds watch Git refs. Keep `publish_to_vps` off when you only want a downloadable Actions artifact for local testing.
