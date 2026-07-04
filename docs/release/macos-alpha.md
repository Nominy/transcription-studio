# macOS Alpha Release

Transcription Studio uses Velopack for macOS alpha bundles, installer packages, and in-app updates.

Default update feeds:

```text
latest-arm64:  https://reviewgen.ovh/transcription-studio/alpha/mac-arm64
latest-amd64:  https://reviewgen.ovh/transcription-studio/alpha/mac-x64
sonoma-amd64:  https://reviewgen.ovh/transcription-studio/alpha/mac-sonoma-x64
```

Release builds set `STUDIO_DEFAULT_UPDATE_FEED_URL` at compile time so the installed app checks the same feed it was published from. Set `STUDIO_UPDATE_FEED_URL` before launching the app to temporarily point an installed build at a different feed during testing.

## Build Artifacts

Run this on macOS:

```bash
INSTALL_VPK=1 ./tools/package-alpha-macos.sh
```

The script builds `studio-app`, stages it as `TranscriptionStudio`, creates a Velopack `.app` bundle with `vpk bundle`, and creates release/update artifacts with `vpk pack`. Artifacts are written to:

```text
target/velopack/Releases
```

The script builds for the current Mac architecture. Use `RUNTIME=osx-arm64` on Apple Silicon and `RUNTIME=osx-x64` on Intel; it will fail instead of cross-compiling to the wrong Velopack runtime.

The script exports `MACOSX_DEPLOYMENT_TARGET=14.0` by default so Intel builds remain compatible with Sonoma 14.x. Override it only when intentionally dropping older macOS support.

Unsigned local builds skip `.pkg` generation and produce portable Velopack artifacts for trusted testers. Signed builds can produce the installer package and notarized release feed when these values are available:

```bash
APPLE_DEVELOPER_ID_APPLICATION="Developer ID Application: Example, Inc. (TEAMID)"
APPLE_DEVELOPER_ID_INSTALLER="Developer ID Installer: Example, Inc. (TEAMID)"
APPLE_NOTARY_PROFILE="notarytool-keychain-profile"
```

GitHub Actions can run the same package flow from the manual `macOS Alpha Package` workflow. See `docs/release/github-actions.md`.

## GitHub Targets

The manual workflow has a `target` input:

- `latest-arm64`: runs on `macos-26`, packages `osx-arm64`, and publishes to `alpha/mac-arm64`.
- `latest-amd64`: runs on `macos-26-intel`, packages `osx-x64`, and publishes to `alpha/mac-x64`.
- `sonoma-amd64`: runs on `macos-14-large`, packages `osx-x64`, and publishes to `alpha/mac-sonoma-x64`.
- `all`: runs every target above.

`sonoma-amd64` is for Intel Macs on Sonoma, including 14.8.x machines. GitHub currently provides the Sonoma image as a larger Intel runner, so repository billing/runner availability must allow that label.

## Tester Flow

For unsigned trusted-tester builds, send the portable macOS artifact from `target/velopack/Releases`. Testers may need to try opening the app once, then use:

```text
System Settings -> Privacy & Security -> Open Anyway
```

For signed and notarized builds, send the generated installer package when available. After installation, the dashboard Updates section shows the current version, update feed, and buttons for `Check for updates` and `Install update`.

Keep old files in the hosted feed directory. Velopack uses `releases.alpha.json` plus package history to serve updates.
