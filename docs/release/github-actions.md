# GitHub Actions

The private GitHub repository is expected to build and package the alpha channel without storing server credentials in git.

## CI

`CI` runs on pushes, pull requests, and manual dispatch:

- Gitleaks scans the full git history with redacted output.
- Windows and macOS builds run `cargo fmt --all --check` and `cargo test --workspace --locked` with `RUST_TEST_THREADS=4`. The test step compiles the same workspace graph, so CI skips a separate `cargo check` pass, and the capped libtest worker count avoids over-scheduling the heavy app smoke tests.

## Windows Alpha Package

`Windows Alpha Package` is a manual workflow. It builds the Velopack Windows artifacts and uploads `target/velopack/Releases/*` as a workflow artifact.

Defaults:

- Channel: `alpha`
- Feed URL: `https://reviewgen.ovh/transcription-studio/alpha/win-x64`
- Remote path: `/var/www/transcription-studio/alpha/win-x64`

The workflow downloads the current feed packages before packaging so Velopack can create delta packages when a previous full package exists.

## macOS Alpha Package

`macOS Alpha Package` is a manual workflow. It builds the Velopack macOS artifacts and uploads `target/velopack/Releases/*` as workflow artifacts. The workflow sets `MACOSX_DEPLOYMENT_TARGET=14.0` and compiles `STUDIO_DEFAULT_UPDATE_FEED_URL` into each package so the in-app updater checks the matching feed.

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

Publishing is disabled by default. To let Actions upload to the VPS, add these repository secrets:

- `ALPHA_SSH_HOST`: VPS host or IP.
- `ALPHA_SSH_USER`: SSH user with write access to the release directory.
- `ALPHA_SSH_PRIVATE_KEY`: private deploy key for that user.
- `ALPHA_SSH_PORT`: optional SSH port, defaults to `22`.

Keep `publish_to_vps` off when you only want a downloadable Actions artifact for local testing.
