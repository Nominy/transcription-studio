# GitHub Actions

The private GitHub repository is expected to build and package the alpha channel without storing server credentials in git.

## CI

`CI` runs on pushes, pull requests, and manual dispatch as one job on a
self-hosted runner (a dedicated home server, label `[self-hosted, Linux,
homeserver]`): Gitleaks scans the full git history, then `cargo fmt --all
--check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo nextest run --workspace --all-targets --locked` run in that order,
cheapest checks first. The checkout step
uses `clean: false` so `target/` persists across runs instead of being wiped
by `actions/checkout`'s default `git clean -ffdx` -- that persistence, plus
`cargo nextest` running every test binary concurrently instead of the
default harness's one-binary-at-a-time execution, is what keeps a warm run
to well under a minute. `--locked` remains mandatory. Windows- and
macOS-only code (`#[cfg(windows)]`, `#[cfg(windows)]`-gated tests) doesn't
compile as part of this Linux-hosted run; real platform coverage comes from
the alpha package workflows below.

## Windows Alpha Package

`Windows Alpha Package` runs for every push to `main` and is also available
through manual dispatch. It runs entirely on the self-hosted homeserver
(`[self-hosted, Linux, homeserver]`; no `windows-latest` or GitHub-hosted
runner is involved). Each push event produces one release from the checked-out
head commit, so a push containing multiple commits does not release each commit
separately.

1. `cargo xwin build --release --target x86_64-pc-windows-msvc` cross-compiles
   `studio-app`/`studio-relay` (`cargo-xwin` downloads the MSVC CRT/Windows
   SDK from Microsoft's redistributable; no Visual Studio or Windows host
   needed for this step). gpui's DirectX renderer embeds HLSL bytecode that
   only `fxc.exe` on a Windows host can compile, so the workspace patches
   `gpui-pre-windows` with `vendor/gpui-pre-windows`, whose build script copies
   `prebuilt/shaders_bytes.rs` on non-Windows hosts. That file is the fxc output
   for the pinned crate version; when upgrading gpui, re-vendor the crate and
   regenerate it from a Windows release build
   (`target/release/build/gpui-pre-windows-*/out/shaders_bytes.rs`).
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

Windows alpha signing is not currently configured, so there is no
Windows-only signing step. VPS publication, however, requires the repository
secrets `ALPHA_SSH_HOST`, `ALPHA_SSH_USER`, and `ALPHA_SSH_PRIVATE_KEY`;
`ALPHA_SSH_PORT` is optional. Publication is mandatory for push-triggered runs,
and a push run fails rather than silently retaining an unpublished package when
the required secrets are missing. Manual dispatch remains available and may
skip publication with `publish_to_vps: false`.

The package step creates a clean `target/velopack/Publish` directory from
`assets.alpha.json`. It uploads only that directory to unserved incoming
storage, validates size and SHA-256, and promotes under a remote lock with feed
metadata last. The small publish directory is then retained as a three-day,
uncompressed workflow artifact. Workflow concurrency is serialized in the
`velopack-windows-alpha` group with in-progress runs never cancelled.

Defaults:

- Channel: `alpha`
- Feed URL: `https://reviewgen.ovh/transcription-studio/alpha/win-x64`
- Remote path: `/var/www/transcription-studio/alpha/win-x64`

When no manual version override is supplied, the workflow reads the Cargo
`major.minor.patch`, increments the patch, and appends
`-alpha.<run_number>.<run_attempt>`. It exports that same value as both
`VERSION` for Velopack and `STUDIO_BUILD_VERSION` for compilation, keeping the
displayed application build identity and package identity equal. An explicit
manual version is allowed and is likewise carried through both variables.

The separate `macOS Alpha Package` workflow remains optional and
`workflow_dispatch`-only; Windows pushes do not trigger a macOS release.

## macOS Alpha Package

`macOS Alpha Package` is a manual two-stage matrix workflow. For every selected logical target, the `compile` job runs on `[self-hosted, Linux, homeserver]` and uses osxcross to build the Rust binaries. It uploads those binaries as `macos-binaries-<logical-target>`. The matching macOS `package` job downloads that artifact, restores the pinned Velopack 1.2.0 CLI, and runs one full `vpk pack` pass without rebuilding Rust. The former redundant `vpk bundle` pass is not part of the release path.

The macOS runner remains necessary because Velopack 1.2.0 host-gates full macOS packaging to macOS. It is used only to create the application bundle and release payload and, when credentials are configured, to perform Apple signing and notarization. Optional VPS publication then uses per-runtime incoming storage, manifest validation, a remote lock, asset-first promotion, feed-last replacement, and rollback metadata. The Sonoma target remains separate until compatibility is proven on a real Sonoma machine.

Defaults:

- Channel: `alpha`
- Target: `all`
- Remote base path: `/var/www/transcription-studio/alpha`

One-time homeserver prerequisites:

- Obtain a macOS 11 or newer SDK legally; it must include arm64 support.
- Build osxcross once with `ENABLE_ARCHS="arm64 x86_64"` so both workflow runtimes use the same installation.
- Put the osxcross target `bin` directory on the self-hosted runner's `PATH`, or set `OSXCROSS_TARGET_DIR` to that target directory in the runner environment.
- Make `cargo` and `rustup` available on the runner's `PATH`.

To reproduce the compile stage directly from the repository root, use the logical runtime and an isolated Cargo target directory:

```bash
RUNTIME=osx-arm64 \
OSXCROSS_TARGET_DIR=/opt/osxcross/target \
CARGO_TARGET_DIR=target/osxcross/latest-arm64 \
MACOSX_DEPLOYMENT_TARGET=14.0 \
./tools/build-alpha-macos-osxcross.sh
```

Use `RUNTIME=osx-x64` and the appropriate logical target directory for an Intel target. The helper maps these runtimes to `aarch64-apple-darwin` and `x86_64-apple-darwin`, loads `osxcross-conf` when available, configures the target linker plus `CC`, `CXX`, `AR`, and `SDKROOT`, installs the Rust target with `rustup`, and runs the locked release build.

After artifact download, the package-stage interface is:

```bash
dotnet tool restore
RUNTIME=osx-arm64 \
SOURCE_DIR="$PWD/target/osxcross/latest-arm64/aarch64-apple-darwin/release" \
SKIP_BUILD=1 \
./tools/package-alpha-macos.sh
```

`SOURCE_DIR` must contain the downloaded `studio-app` and `studio-relay` executables. `SKIP_BUILD=1` makes the helper consume them instead of invoking Cargo.

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

Windows push runs always publish; manual Windows dispatches and macOS runs
leave publishing disabled by default. To let a manual run stage and atomically
promote a release on the VPS, add the same repository secrets required by
automatic Windows publication:

- `ALPHA_SSH_HOST`: VPS host or IP.
- `ALPHA_SSH_USER`: SSH user with write access to the incoming and release directories.
- `ALPHA_SSH_PRIVATE_KEY`: private deploy key for that user.
- `ALPHA_SSH_PORT`: optional SSH port, defaults to `22`.

Package jobs inject `STUDIO_BUILD_GIT_COMMIT=${{ github.sha }}` so installed builds retain exact source identity without making local Cargo builds watch Git refs. For artifact-only testing, use manual dispatch and leave `publish_to_vps` off; push-triggered Windows runs cannot skip publication.
