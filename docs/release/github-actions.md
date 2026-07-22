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

`Windows Alpha Package` is a manual workflow, entirely on the self-hosted
home server (no `windows-latest`, no GitHub-hosted runner involved at all):

1. `cargo xwin build --release --target x86_64-pc-windows-msvc` cross-compiles
   `studio-app`/`studio-relay` (`cargo-xwin` downloads the MSVC CRT/Windows
   SDK from Microsoft's redistributable; no Visual Studio or Windows host
   needed for this step).
2. `tools/package-alpha-windows.ps1 -Target x86_64-pc-windows-msvc -VpkCommand
   tools/vpk-wine-wrapper.sh` runs `vpk pack` through
   `tools/vpk-wine-wrapper.sh`, which hosts the pinned `vpk.dll` (a portable,
   framework-dependent .NET assembly) under a genuine **Windows-native**
   `dotnet.exe` executed via Wine, instead of the Linux-native `dotnet`.
   This matters because `vpk pack` decides which OS to package for based on
   which runtime build is executing it, not a `--runtime`/`-r` flag --
   confirmed empirically: `vpk pack --runtime win-x64` under the Linux-native
   `dotnet` refuses with "the target rid must be Linux" on both the pinned
   1.2.0 and the latest 1.2.x prerelease, while the identical `vpk.dll`
   hosted by a Windows-native `dotnet.exe` under Wine correctly exposes the
   Windows-only options (`--shortcuts`, `--msi`, `--signParams`) and produces
   a real `Setup.exe`/`.nupkg`/portable zip. Verified end to end: produced a
   working, correctly-signed-format `TranscriptionStudio-alpha-Setup.exe`
   this way and confirmed it as a genuine `PE32 executable for MS Windows`
   via `file`.

Wine setup on the home server (one-time, mirrors the rust/node/gitleaks
pattern -- not repeated per run):

```bash
dpkg --add-architecture i386
apt-get update && apt-get install -y wine wine64 winetricks

# Windows-native .NET 8 runtime, used only to host vpk.dll under Wine.
curl -fsSL -o dotnet-runtime-win-x64.zip \
  https://builds.dotnet.microsoft.com/dotnet/Runtime/8.0.20/dotnet-runtime-8.0.20-win-x64.zip
mkdir -p ~/win-dotnet/runtime
unzip dotnet-runtime-win-x64.zip -d ~/win-dotnet/runtime
```

`tools/vpk-wine-wrapper.sh` requires `VPK_DLL` (set from `dotnet tool list`
after `dotnet tool restore`, since the pinned version can change) and
`WINE_DOTNET_EXE` (points at the extracted `~/win-dotnet/runtime/dotnet.exe`
above) as environment variables.

No signing secrets are configured for Windows alpha builds today (unlike
macOS below), so there is no Windows-only signing step blocking this either.

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
