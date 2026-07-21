# Transcription Studio

Native Rust/Iced editor for Babel-shaped transcription correction.

The current editor establishes the standalone workspace, Babel-shaped core model, local project materialization, Babel package round trip, recovered Babel review-action import, and a Babel-like native correction workspace. The native shell now opens on a dashboard with project input/settings, then jumps into a zoom/speed toolbar, overview waveform, multi-track waveform timeline, transcript grid editing, track filters, draft-aware lint status, save/export/review controls, OS clipboard support ID copy, keyboard shortcuts, loop/diff state, and command-backed segment actions.

Imports accept both the Studio task-package fixture and the recovered Babel `result.data.json` review-action envelope exported from the live editor bundle. Exports preserve unmodeled Babel fields through sidecar metadata so Studio can round trip task/speaker/track/annotation payloads it does not edit yet. The app has PCM16 WAV decoding, waveform peak modeling, and a native Rodio playback backend, but the full target is now tracked as recovered Babel We Edit parity rather than a first-slice editor.

Full parity tracking lives in:

- `docs/superpowers/specs/2026-06-22-babel-we-edit-full-parity-design.md`
- `docs/parity/babel-we-edit-parity-map.md`

## Commands

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
cargo run -p studio-app
```

Enable the local pre-push gate once per clone so a failing `cargo fmt`/`cargo clippy` never reaches CI (or main) in the first place:

```powershell
git config core.hooksPath .githooks
```

## Native Babel Service Bridge

Set `STUDIO_BABEL_SERVICE_COMMAND` to a command line such as `node bridge.js` or `"C:\path with spaces\bridge.cmd" --profile local` to connect load/claim/submit/skip/drop/admin/feedback actions to an adapter. Studio sends a JSON envelope on stdin with `operation` and `payload`, and accepts either a raw JSON response, `{ "data": ... }`, or `{ "result": { "data": ... } }` on stdout.

For direct live Babel backend calls without Chrome cookie reuse, initialize Studio's native Clerk auth store once with the app-owned phone-code flow:

```powershell
cargo run -p studio-app -- --babel-auth-start-login "+15555550123" --babel-auth-reset
cargo run -p studio-app -- --babel-auth-verify-login "123456"
```

Then run Studio in live mode. Studio reads the native auth store directly, mints a fresh Babel backend session cookie through Clerk for each live request, and does not need a Chrome cookie bridge:

```powershell
$env:STUDIO_BABEL_SERVICE_MODE = "live"
cargo run -p studio-app -- --reviewActionId=<review-action-id>
```

Studio stores Clerk client cookies under `%APPDATA%\TranscriptionStudio\babel-auth.json`. `STUDIO_BABEL_AUTH_STORE` can point Studio at a different native store, and `STUDIO_BABEL_SERVICE_AUTH_COMMAND`, `STUDIO_BABEL_SERVICE_COOKIE`, or `STUDIO_BABEL_SERVICE_AUTHORIZATION` remain explicit developer overrides. The older `tools/babel-native-auth.mjs` and `tools/babel-live-auth-cookie.mjs` helpers remain only debugging fallbacks; live Studio auth does not attach to Chrome.

For playback smoke checks, set `STUDIO_LOG_AUDIO_PLAYBACK=1` to print the native playback track/start/speed line. Set `STUDIO_SMOKE_PLAYBACK_ON_START=1` as well to auto-start playback from the bundled recovered two-speaker sample during app startup.
