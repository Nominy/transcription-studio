# Releasing Exaudio

Exaudio uses [Velopack](https://velopack.io): channel-specific installers, delta updates,
and in-app self-update. Release workflows target the planned public `Nominy/exaudio`
repository and use GitHub-hosted runners; they do not run on self-hosted machines.

The Babel integrations and browser extensions authored in the companion Babel checkout are
first-party developments. Their ownership is distinct from that of any files copied verbatim
from an upstream service. Review those external assets and runtime dependencies individually;
do not classify the entire Babel integration as third-party proprietary code.

Release and CI runners need no private build input: the Babel linter's rule data is tracked in
`extensions/babel-lint/assets`, and the private Babel data bundle (`tools/babel-data.json`) only
feeds the parity tooling and the self-skipping parity tests (see
[development.md](development.md#babel-data-bundle-parity-tooling-only)).

| Channel | Who gets it | Built by | Velopack channel | Feed |
| --- | --- | --- | --- | --- |
| Stable | everyone (default for release downloads) | `v*` tag → `release.yml` | `win` | GitHub Releases of `Nominy/exaudio` (pre-releases excluded) |
| Alpha | testers | every push to `main` → `alpha.yml` | `alpha` | `https://reviewgen.ovh/exaudio/alpha/win-x64` |

Stable download endpoints, available after the first public release:

- Installer: `https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Setup.exe`
- Portable: `https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Portable.zip`

## In-app updates

- A build follows the channel it was packaged for (`STUDIO_UPDATE_CHANNEL=stable|alpha`,
  baked in at compile time; plain `cargo build` means stable). Settings → Updates switches
  channel; the choice is stored in the UI storage and checked immediately.
- Installed builds check on launch and then at most every 6 hours, in the background. When an
  update exists, a small card in the dashboard sidebar offers **Install** (with download
  progress) or **Skip** (hides that version; Settings still offers it). Editing is never
  interrupted.
- Switching from Alpha to Stable never downgrades: the app moves to Stable with the first
  stable release newer than the installed alpha.
- Source builds (`cargo run`) are not Velopack installs; the Updates card says so and no
  check runs.

## Cut a stable release

1. Bump `version` in `crates/studio-app/Cargo.toml` (SemVer, e.g. `0.3.1`) and run any cargo
   command (`cargo check -p studio-app`) so `Cargo.lock` follows.
2. Commit and push to `main`; wait for CI.
3. Tag and push the tag:

   ```powershell
   git tag v0.3.1
   git push origin v0.3.1
   ```

`release.yml` then, on `windows-latest`:

1. fails unless the tag is exactly `v` + the `studio-app` version;
2. builds `studio-app`, `studio-relay` and `exaudio-wasm` together in one Cargo invocation,
   keeping shared dependency features unified, with `STUDIO_UPDATE_CHANNEL=stable`;
   `exaudio-wasm-worker` is the isolated compiler child for untrusted extension `.wasm` and
   ships beside `Exaudio.exe` (or the main executable in macOS `Contents/MacOS`);
3. generates release notes (GitHub's generated notes plus download/SmartScreen lines);
4. downloads the previous release's full package (`vpk download github`) so the new release
   carries a delta package;
5. runs `tools/package-windows.ps1 -UpdateChannel stable`: `vpk pack` with pack id `Exaudio`,
   title `Exaudio`, main exe `Exaudio.exe`, icon `assets/app/exaudio.ico`, channel `win`;
6. uploads with `vpk upload github` as a draft named `Exaudio <version>`, attaches `SHA256SUMS`
   (installer, portable zip, full and delta packages), then publishes it.

Release assets: `Exaudio-win-Setup.exe`, `Exaudio-win-Portable.zip`,
`Exaudio-<version>-full.nupkg`, `Exaudio-<version>-delta.nupkg` (from the second release on),
`SHA256SUMS`, plus the Velopack feed file `releases.win.json` that installed apps read.

A tag with a hyphen (`v0.4.0-rc.1`, version `0.4.0-rc.1` in Cargo.toml) becomes a GitHub
pre-release; stable installs ignore pre-releases.

To rebuild an existing tag, delete its GitHub release (draft or published) and run the
**Release** workflow manually with the tag as input. The workflow only runs in
`Nominy/exaudio`.

### Package locally

Needs the .NET 8 SDK, the MSVC toolchain and CMake (the bundled libopus is built from source;
GitHub's Windows and macOS runners ship both C toolchains and CMake):

```powershell
dotnet tool restore
.\tools\package-windows.ps1 -UpdateChannel stable
```

`target\velopack\Releases` receives the full local feed (installer, portable zip, packages,
feed metadata); `target\velopack\Publish` the current release's assets plus
`publish-manifest.json` (byte size and SHA-256 of each file). `-Version`, `-ReleaseNotes`,
`-OutputDir`, `-PublishDir`, `-SkipBuild` are optional.

Stable Windows feed metadata includes `RELEASES`, `assets.win.json` and `releases.win.json`.
Non-default channels use the suffixed legacy index, for example `RELEASES-alpha`.

## Alpha

Every push to `main` runs `alpha.yml` on `windows-latest` (one build per push, runs
serialized, never cancelled):

- version `<major>.<minor>.<patch+1>-alpha.<run number>.<run attempt>`, which sorts above the
  current stable and below the next one; it is compiled in as `STUDIO_BUILD_VERSION`;
- the previous alpha is downloaded from the feed for delta packages;
- `tools/package-windows.ps1 -UpdateChannel alpha` packs channel `alpha`
  (`Exaudio-alpha-Setup.exe` is the tester installer);
- `tools/publish-alpha-windows.ps1 -SkipPackage` publishes `target/velopack/Publish` to the
  VPS (`/var/www/exaudio/alpha/win-x64`, served as `https://reviewgen.ovh/exaudio/alpha/win-x64`):
  upload to an unserved incoming directory (`/var/tmp/exaudio-alpha-incoming`), verify every
  file against the manifest under a remote lock, promote packages before
  `releases.alpha.json`, keep the previous feed metadata for rollback. Versioned packages are
  immutable on the server.

Push runs always publish (and fail if the secrets are missing); a manual dispatch publishes
only with `publish_to_vps: true`. Each run also keeps the payload as a 3-day artifact.

Publish from a developer machine with an SSH alias for the server:

```powershell
.\tools\publish-alpha-windows.ps1 -Remote <ssh-alias>
```

It packages first unless `-SkipPackage` is given.

### One-time reinstall after the rename

Up to 0.2.x the private alpha was packaged as `TranscriptionStudio` and published to
`https://reviewgen.ovh/transcription-studio/alpha/...`. Exaudio is a new Velopack package
(`Exaudio`, installed into `%LocalAppData%\Exaudio`) on its own feeds under
`https://reviewgen.ovh/exaudio/alpha/...`; nothing publishes to the old feeds any more, so old
installs stay frozen and never see an Exaudio package. Install Exaudio once from
`Exaudio-alpha-Setup.exe` (or a stable release), then uninstall the old `TranscriptionStudio`
package. Sign-in, drafts and settings are not carried over: Exaudio reads its application-support
directory `Exaudio`, not the old `Transcription Studio` one.

Before the first Exaudio alpha publish, the web server must serve `/var/www/exaudio/` as
`https://reviewgen.ovh/exaudio/` (the publisher creates the feed directories itself).

### macOS alpha

`macos-alpha.yml` is manual (`workflow_dispatch`). It builds natively on GitHub-hosted macOS
runners with `tools/package-alpha-macos.sh` (pack id `Exaudio`, bundle id
`io.github.nominy.exaudio`, channel `alpha`, deployment target macOS 14.0) and can publish to:

| Target | Runner | Runtime | Feed |
| --- | --- | --- | --- |
| `latest-arm64` | `macos-15` | `osx-arm64` | `https://reviewgen.ovh/exaudio/alpha/mac-arm64` |
| `latest-amd64` | `macos-15-intel` | `osx-x64` | `https://reviewgen.ovh/exaudio/alpha/mac-x64` |
| `sonoma-amd64` | `macos-14-large` | `osx-x64` | `https://reviewgen.ovh/exaudio/alpha/mac-sonoma-x64` |

`STUDIO_ALPHA_FEED_URL` bakes each target's feed into its build. `macos-14-large` is a
larger runner and is billed even for public repositories; skip `sonoma-amd64` unless that is
intended. There is no stable macOS release; on macOS the Stable channel reports that it has
no releases for the platform.

## Secrets

| Secret | Used by | Required |
| --- | --- | --- |
| `ALPHA_SSH_HOST` | `alpha.yml`, `macos-alpha.yml` publish | yes, for publishing |
| `ALPHA_SSH_USER` | same | yes, for publishing |
| `ALPHA_SSH_PRIVATE_KEY` | same; deploy key with write access to the incoming and feed directories | yes, for publishing |
| `ALPHA_SSH_KNOWN_HOSTS` | same; the server's `known_hosts` line(s), e.g. from `ssh-keyscan -p <port> <host>` checked against the server | yes, for publishing |
| `ALPHA_SSH_PORT` | same | no (default 22) |
| `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_KEYCHAIN_PASSWORD`, `APPLE_DEVELOPER_ID_APPLICATION`, `APPLE_DEVELOPER_ID_INSTALLER`, `APPLE_NOTARY_PROFILE`, `APPLE_NOTARY_APPLE_ID`, `APPLE_NOTARY_TEAM_ID`, `APPLE_NOTARY_PASSWORD` | `macos-alpha.yml` signing/notarization | no (unsigned without them) |

`release.yml` needs no secret: it uses the workflow's `GITHUB_TOKEN` (`contents: write`).

## Signing

- **Windows: unsigned.** SmartScreen shows "Windows protected your PC" on first launch of the
  installer or portable exe until the download builds reputation; users choose
  **More info → Run anyway**. Every release lists SHA-256 checksums in `SHA256SUMS`.
- **macOS: optional.** With the Apple secrets the workflow signs with Developer ID and
  notarizes; without them it produces unsigned portable bundles (no `.pkg`), which testers
  open via System Settings → Privacy & Security → Open Anyway.

## CI

`ci.yml` runs on every push and pull request on `windows-latest`: gitleaks over the tree and
the full history (`tools/scan-secrets.ps1`), `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo nextest run --workspace --all-targets --locked`. The `.githooks/pre-push` hook runs
the same fmt and clippy gate locally (`git config core.hooksPath .githooks`).

## Local inference runtime

"This PC" AI runs a separately published inference runtime. It is not part of the installer
or of this repository: `extensions/gold/gold-inference.json` pins exactly one
published build (version, source commit, URL, SHA-256, size, extension id). Exaudio downloads
it on first local use (or Settings → Install), verifies size and SHA-256 while streaming and
extracts it atomically into its data directory; the active and the previous build are kept.

Prerequisites for shipping a pin in a release:

- the pinned URL is public and immutable (a re-upload must be byte-identical), so every
  released build keeps downloading exactly what it pins;
- the runtime's licence allows downloading it onto users' machines from Exaudio;
- changing the pin is a reviewed change to `gold-inference.json`, made by a maintainer with
  access to the runtime's source; the end-to-end check is the `local_smoke` example of
  `exaudio-gold` (`extensions/gold`) on a WebGPU machine.
