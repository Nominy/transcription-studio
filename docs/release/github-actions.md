# GitHub Actions

The private GitHub repository is expected to build and package the alpha channel without storing server credentials in git.

## CI

`CI` runs on pushes, pull requests, and manual dispatch as one job on a
self-hosted runner (a dedicated home server, label `[self-hosted, Linux,
homeserver]`): Gitleaks scans the full git history, then `cargo fmt --all
--check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo nextest run --workspace --all-targets --locked`, and the native Babel
auth Node test run in that order, cheapest checks first. The checkout step
uses `clean: false` so `target/` persists across runs instead of being wiped
by `actions/checkout`'s default `git clean -ffdx` -- that persistence, plus
`cargo nextest` running every test binary concurrently instead of the
default harness's one-binary-at-a-time execution, is what keeps a warm run
to well under a minute. `--locked` remains mandatory. Windows- and
macOS-only code (`#[cfg(windows)]`, `#[cfg(windows)]`-gated tests) doesn't
compile as part of this Linux-hosted run; real platform coverage comes from
the alpha package workflows below.

## Windows Alpha Package

`Windows Alpha Package` is a manual workflow split across two jobs:

1. `build` cross-compiles `studio-app`/`studio-relay` for
   `x86_64-pc-windows-msvc` on the same self-hosted home server as `CI`,
   using `cargo xwin build` (downloads the MSVC CRT/Windows SDK from
   Microsoft's redistributable via `cargo-xwin`; no Visual Studio or
   Windows host required for this step). The resulting `.exe` files are
   uploaded as a workflow artifact.
2. `package` runs on `windows-latest` and does only the fast part: download
   the cross-compiled `.exe`s, `dotnet tool restore` the pinned Velopack
   CLI, `vpk download http` to prime deltas, and
   `tools/package-alpha-windows.ps1 -SkipBuild` to run `vpk pack`. This is
   the one step that genuinely cannot move off Windows -- `vpk pack`
   packages for whatever OS it runs on, confirmed empirically (tried both
   the pinned 1.2.0 and the latest 1.2.x prerelease with `--runtime
   win-x64` on Linux; both refuse: "the target rid must be Linux"). Moving
   the compile off `windows-latest` cuts that job from several minutes to
   seconds, since packaging itself is just hashing/zipping already-built
   files.

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
