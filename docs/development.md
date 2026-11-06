# Developing Exaudio

For application usage, see [Get started / Начать работу](getting-started.md). For the component contract, see the [Rust extension SDK](../sdk/rust/README.md). Architecture and release procedures live in [architecture.md](architecture.md) and [releasing.md](releasing.md).

## Windows prerequisites

- Git.
- [Rustup](https://rustup.rs/), using the toolchain declared in the repository's `rust-toolchain.toml`. Do not replace the pinned version to work around a compiler error without checking the dependency requirements.
- Visual Studio Build Tools with **Desktop development with C++**, the MSVC x64 toolchain and Windows SDK.
- CMake on `PATH` for native audio dependencies, including libopus.
- WebView2 Runtime and a WebGPU-capable GPU/driver only when exercising Windows local inference. The inference runtime and models install separately from application compilation.
- `cargo-nextest` for the workspace test command. The .NET 8 SDK and `dotnet tool restore` are packaging prerequisites, not requirements for end users.

Use the repository root as the working directory. Cargo commands below are foreground commands; none silently starts a background job. A full first build can be long. In the managed development environment, run long work in the established detached **psmux** runner, with persistent stdout/stderr, timestamped events, atomic status, inspectable progress, a graceful stop route and a unique session name. Discover that installation's syntax rather than copying an invented multiplexer command. Cargo cancellation restarts at a compilation boundary while retaining completed artifacts; it is not a checkpoint inside a compiler invocation. Provider/model work needs its own durable unit checkpoints and must not be retried as if it were compilation.

## Select the build you need

```powershell
# General-purpose toolkit, including Gold AI tools, without the Babel bundle
cargo run -p studio-app --no-default-features --features local,formats,basic-checks,gold --locked

# Local editing, subtitle formats and basic diagnostics; no Babel or Gold AI
cargo run -p studio-app --no-default-features --features local,formats,basic-checks --locked

# Core shell only: deliberately no bundled modes, formats, checks or AI
cargo run -p studio-app --no-default-features --locked

# All batteries, including Babel: requires the pinned bundle described below
cargo run -p studio-app --locked
```

`studio-app` remains the Cargo package/binary name used during development. Packaged releases use the Exaudio name. The default `batteries-included` feature selects `local`, `formats`, `basic-checks`, `gold` and `babel`; changing the build feature set is different from enabling/disabling an installed extension.

**First-party code and build inputs:** Helper, reviewer/drafting and Babel integration extensions are the Exaudio author's own developments, not proprietary third-party extensions. The general-purpose `local,formats,basic-checks,gold` build above does not require the Babel data bundle. The default all-batteries build and builds enabling `babel` currently do require it; a fresh checkout alone is not sufficient. Only concrete data/assets copied verbatim from the upstream Babel service need a separate provenance review; do not confuse those inputs with the author's extension code. Keep credentials and private task content out of source and public fixtures.

Building the app does not install AI model weights. Source builds are not Velopack installs and do not perform packaged-app update checks. Do not expect `cargo run` to follow the stable or alpha feeds.

### Babel bundle prerequisite

The build script at `extensions/babel-lint/build.rs` reads the SHA-256 pin in `extensions/babel-lint/babel-data.json` and verifies `babel-data-<sha256>.tar` in the per-user cache. On Windows that cache is `%LOCALAPPDATA%\Sonus\build-cache`; elsewhere it is `$XDG_CACHE_HOME/sonus/build-cache`, falling back to `~/.cache/sonus/build-cache`. The bundle contains `babel-lint-data.json`, `error-category-names.json` and `lint-parity.json`.

The archive is currently available only from the author's existing cache or private artifact store; there is no public bundle URL. Maintainers with authorized SSH access can prepare the exact pinned archive using:

```powershell
powershell -File tools/update-babel-data.ps1 -Fetch
cargo run -p studio-app --locked
```

The script's `-Remote` and `-RemoteRoot` parameters select the configured SSH host/store. `-Fetch` verifies the SHA, fills the build cache and unpacks the staging copy into `target/babel-data`. A matching cached archive avoids a network fetch. Do not omit `-Fetch` for ordinary setup: without it, the script publishes staged data and updates the pin.

Without that access or a matching cached archive, use the general-purpose build above rather than expecting the default build to fetch missing data automatically. Public all-batteries release remains on hold pending resolution of the actual upstream-copied assets and integrated clean-build/release verification; this is distinct from ownership of the first-party extension code.

## Synthetic development material

For an account-free, synthetic demo, set these variables in a PowerShell terminal before launching the appropriate app build:

```powershell
$env:STUDIO_OPEN_DEMO_ON_START = '1'
$env:STUDIO_AUDIO_SILENT = '1'
cargo run -p studio-app --no-default-features --features local,formats,basic-checks --locked
```

Unset them afterward with `Remove-Item Env:STUDIO_OPEN_DEMO_ON_START, Env:STUDIO_AUDIO_SILENT` when returning to normal use. The demo is an original Russian dialogue synthesized with Piper voices; [the generator](../tools/demo-audio/generate.py) records the source voices and dataset provenance. Do not regenerate it just to run the app, and do not substitute private platform tasks in public screenshots or test fixtures.

## Checks for contributors

Run verification once after related edits have landed, not concurrently against a half-edited workspace. These are commands to run, **not a claim that this document's changes passed them**:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --all-targets --locked
```

The workspace-wide commands include Babel crates and require the pinned bundle described above. For narrower app integration checks that do not require it:

```powershell
cargo check -p studio-app --no-default-features --features local,formats,basic-checks,gold --locked
cargo check -p studio-app --no-default-features --features local,formats,basic-checks --locked
cargo check -p studio-app --no-default-features --locked
```

A narrow check is not a passing full-workspace test run. Native GUI behavior, audio playback, model setup and packaging need their corresponding real smoke runs. Never use private service credentials or paid provider requests as a default test prerequisite.

## Extension development

Start with the [Rust SDK build/package guide](../sdk/rust/README.md) and [`hello-panel`](../extensions/examples/hello-panel). It describes the actual toolchain, component conversion, manifest, package layout, explicit grants and real-component smoke. Keep the [WIT contract](../wit/sonus-extension.wit) and SDK compatible with the host version you target.

The [marketplace index repository](https://github.com/Nominy/sonus-extensions) references packages released from each author's GitHub repository. The endpoint is `https://raw.githubusercontent.com/Nominy/sonus-extensions/main/index.json`. Publish immutable versioned artifacts with their source identity, byte size, SHA-256 and requested permissions; an index record is not a license or security endorsement. Do not embed API keys or assume that manifest declarations automatically grant host access.

Native Rust extensions have application privileges. Installable WASM components use explicit typed host capabilities and resource limits, not arbitrary ambient filesystem/process/network access. Modes declare required extensions; removing a dependency while its mode is open is not a supported way to reduce the workspace's capabilities.

## Website development

`site/` is plain HTML, CSS and JavaScript with no build framework, package manager, external fonts or analytics. Russian is the no-JavaScript/default language. The explicit RU/EN controls persist `sonus.site.language` in local storage when storage is available; system/browser language does not override the Russian default. Colors and existing brand headers follow `prefers-color-scheme`. `site/assets/` contains copies of the three source SVGs in `assets/brand/`; the Pages workflow refreshes those copies while assembling the artifact.

Open `site/index.html` for a quick static preview. For browser automation that requires HTTP, serve `site/` through the development environment's observable server runner; for example the actual foreground workload can be `python -m http.server 4173 --directory site --bind 127.0.0.1`. A long-lived server belongs in a named psmux session and must be stopped when finished.

`.github/workflows/pages.yml` runs only for `Nominy/exaudio` on `main`, uses a hosted Linux runner and Pages OIDC permissions, and substitutes the **actual** `actions/configure-pages` `base_url` into canonical/OG URLs, `sitemap.xml` and `robots.txt`. Raw source contains the deliberate `__BASE_URL__` build token; the published artifact must not. The assembly step fails on missing or unresolved tokens. Pages must use **GitHub Actions** as its source. A local file preview is not proof of canonical URL correctness or deployment.

Visual acceptance should cover RU/EN, retained language after reload, light/dark themes, 320/375/768/1440 px widths, keyboard-only navigation, visible focus, 200% zoom, reduced motion, blocked local storage and the no-JavaScript Russian page. Check that all download/issue/SDK links resolve once their repositories and releases exist. A visually successful preview does not establish that a release exists.

The layout does not pretend that brand artwork is an application screenshot. Add a real integrated-app capture using the synthetic demo as `site/assets/editor-synthetic.webp`, then insert a `figure.app-screenshot` after the hero in `site/index.html`, with actual image dimensions and bilingual caption/alt text. Do not add an image reference before the file exists. Never show private tasks or credentials.

## Licenses and acknowledgements

- Exaudio source code is licensed under the root [MIT license](../LICENSE), consistent with `workspace.package.license`. Copyright and license notices must remain in redistributed copies.
- Rust dependencies retain their individual licenses. `Cargo.lock` identifies exact versions; review each upstream crate's license and include required notices with binary distribution. This short attribution list is **not** a complete dependency-license inventory.
- The interface uses [GPUI](https://gpui.rs/) and [gpui-kit](https://crates.io/crates/gpui-kit). The vendored Windows patch retains its [Apache 2.0 notice](../vendor/gpui-pre-windows/LICENSE-APACHE).
- Extension execution uses [Wasmtime](https://wasmtime.dev/). Audio and native codec dependencies retain their upstream licenses independently of Exaudio.
- The Exaudio wordmark uses outlined [Outfit](https://github.com/Outfitio/Outfit-Fonts) Regular lettering, licensed under [SIL Open Font License 1.1](https://github.com/Outfitio/Outfit-Fonts/blob/main/OFL.txt); `tools/brand/render.py` outlines it and renders every derived brand asset. The site ships SVG outlines, not a webfont or an external font request.
- The synthetic demo generator identifies Piper's `ru_RU-denis-medium` and `ru_RU-dmitri-medium` voices and their [CC0 source datasets](https://github.com/OHF-Voice/voice-datasets). Model/software licenses and generated recording provenance are distinct; retain the applicable notices if redistributing voice files. No voice model weights are implied to be part of the installer.
- Downloaded inference runtimes, model weights and third-party extension packages have their own licenses and distribution terms. Review them before redistribution; a runtime download is not covered automatically by the app's MIT license.
- Helper, reviewer/drafting and Babel integration are first-party developments by the Exaudio author. Review provenance separately for any concrete data/assets copied verbatim from the upstream Babel service. Service access still requires authorization; Exaudio does not claim official Babel affiliation.

For contributions, prefer a focused issue/PR with reproducible steps and synthetic fixtures. Include the feature selection and checks you actually ran; report blocked or unexercised checks explicitly rather than treating them as passing.
