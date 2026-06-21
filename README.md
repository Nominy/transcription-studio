# Transcription Studio

Native Rust/Iced editor for Babel-shaped transcription correction.

The current editor establishes the standalone workspace, Babel-shaped core model, local project materialization, Babel package round trip, and a Babel-like native correction workspace. The native shell now has a zoom/speed toolbar, real viewport zooming, overview waveform, multi-track deterministic waveform timeline, transcript grid editing, track filters, draft-aware lint status, save/export/review controls, OS clipboard support ID copy, keyboard shortcuts, loop/diff state, and command-backed segment actions.

Exports preserve unmodeled Babel fields through sidecar metadata so Studio can round trip task/speaker/track/annotation payloads it does not edit yet. Actual PCM decoding/playback is intentionally behind a future transport backend; the current timeline is deterministic from Babel segment timing so editor workflow can be built and tested before audio plumbing.

## Commands

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
cargo run -p studio-app
```
