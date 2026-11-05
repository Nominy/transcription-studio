# Transcription Studio

Native Rust desktop editor for Babel transcription correction, built with [gpui](https://gpui.rs) via `gpui-kit`.

## Crates

| Crate | Role |
|---|---|
| `studio-app` | gpui application (`studio-app.exe`): auth, dashboard, settings, editor UI |
| `studio-editor` | UI-free editor domain: workflow + undo, linter (see Lint), transcript/timeline models, `edit::EditorDoc` editing operations, keymap, startup routes |
| `studio-services` | UI-free services on a shared tokio runtime: Babel client, native Clerk auth, review/drafting backend, feedback, sub-accounts, relay launcher, updater, preferences, recovery |
| `studio-audio` | rodio playback engine (multi-lane mix, seek, loop, audition, per-lane enhanced PCM blended live with the original) and the enhanced-audio disk cache |
| `studio-inference` | "This PC" AI: downloads the pinned Gold inference runtime (`gold-inference.json`), hosts its inference page in a hidden WebView2 with WebGPU and implements the shared `InferenceBackend` over it (timing, draft, enhance, align, model install/test) |
| `studio-core`, `studio-project`, `studio-babel`, `studio-waveform` | core model, project folders, Babel package import/export, waveform peaks/geometry |
| `studio-relay` | sub-account relay server |

The toolchain is pinned in `rust-toolchain.toml` (gpui needs a current stable Rust).

## Commands

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo run -p studio-app
```

Enable the local pre-push gate once per clone:

```powershell
git config core.hooksPath .githooks
```

`STUDIO_OPEN_DEMO_ON_START=1` opens the bundled two-speaker sample task (no network); `STUDIO_AUDIO_SILENT=1` mutes output.

## Lint

Studio lints exactly like the two linters it replaces: Babel's native linter ("Gold") with its `gold-non-bg` configuration for the task's language, plus the Babel Helper extension's custom-linter rules, merged the way Helper merges them. A task's language comes from its Babel project/queue id (`crates/studio-editor/assets/project-languages.json`); rule data is generated from the recovered Babel bundle and Helper sources by `tools/lint-parity/generate.mjs`, which also writes the parity fixture `crates/studio-editor/tests/fixtures/lint-parity.json` that the Rust linter must reproduce:

```powershell
node tools/lint-parity/generate.mjs ..\babel
```

Gold's `foreign-tag-dictionary` and `foreign-tag-code` rules need word dictionaries Babel fetches from its server, so Studio does not run them.

## Babel service

Studio talks to Babel directly with its own Clerk session. Initialize the native auth store once with the phone-code flow (also available in the app's sign-in screen):

```powershell
cargo run -p studio-app -- --babel-auth-start-login "+15555550123" --babel-auth-reset
cargo run -p studio-app -- --babel-auth-verify-login "123456"
```

The store lives at `%APPDATA%\TranscriptionStudio\babel-auth.json` (or next to the executable after "Make installation portable"); `STUDIO_BABEL_AUTH_STORE` overrides it. Startup routes open a task directly, e.g. `--review-action-id=<id>`, `--processed-transcription-id=<id> --stitched-queue-id=<id>`, `--chunked-queue-id=<id>`, `--task-lookup`.

Developer overrides:

- `STUDIO_BABEL_SERVICE_COMMAND`: route every Babel request through an external command (JSON envelope `{operation, payload}` on stdin; raw JSON, `{ "data": … }` or `{ "result": { "data": … } }` on stdout).
- `STUDIO_BABEL_SERVICE_FIXTURE_JSON` / `STUDIO_BABEL_SERVICE_FIXTURE_PATH`: answer every request with a fixed payload.
- `STUDIO_BABEL_SERVICE_BASE_URL`, `STUDIO_BABEL_CLERK_URL`, `STUDIO_BABEL_SERVICE_COOKIE`, `STUDIO_BABEL_SERVICE_AUTHORIZATION`, `STUDIO_BABEL_SERVICE_AUTH_COMMAND`, `STUDIO_BABEL_SERVICE_MODE=live`.
- `STUDIO_REVIEW_BACKEND_BASE_URL`, `STUDIO_DRAFT_OPENROUTER_API_KEY` / `OPENROUTER_API_KEY` for the reviewer/drafting assistant.
- `STUDIO_SUBACCOUNT_*` for sub-account sessions and the relay; `STUDIO_UPDATE_FEED_URL` for the update feed.

## Memory

Leaving the editor (back to projects, task switch, hide to tray) unloads the audio engine, decoded PCM and waveforms of the task.

## Denoise

The editor toolbar's **Denoise** button enhances both speaker tracks through the inference backend chosen by the denoise location setting (This PC or Remote GPU; remote uploads the original tracks). The status popover shows the stage, chunk progress and elapsed time and can cancel a run. The **Enhanced ← → Original** slider mixes the two in 10% steps (new tasks start at 90% enhanced); 100% enhanced asks for confirmation once per task per app session. Mixing happens live in the audio engine: playback, waveforms and timings keep the originals' clock, and word timing/drafting always receive the original audio.

Results are cached under `%APPDATA%\Transcription Studio\enhanced-audio`, keyed by task, enhancer model and the SHA-256 of each original track (latest result per task, 24 most recent tasks). Opening a task installs its cached result without inference; with automatic denoise enabled, a miss starts generation once the audio has loaded.
