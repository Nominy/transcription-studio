# Transcription Studio

Native Rust/Iced editor for Babel-shaped transcription correction.

The current editor slice establishes the standalone workspace, Babel-shaped core model, local project materialization, Babel package round trip, and a Babel-like native correction workspace. The native shell now has a zoom/speed toolbar, overview waveform, multi-track deterministic waveform timeline, transcript grid editing, track filters, lint status, save/export/review controls, and command-backed segment actions.

Actual PCM decoding/playback is intentionally behind a future transport backend; the current timeline is deterministic from Babel segment timing so editor workflow can be built and tested before audio plumbing.

## Commands

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
cargo run -p studio-app
```
