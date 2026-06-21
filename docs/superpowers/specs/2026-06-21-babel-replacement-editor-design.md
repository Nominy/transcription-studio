# Babel Replacement Editor Design

## Goal

Transcription Studio must become a native replacement for the Babel correction workspace, not a CRUD form. The first screen is the editor: top transport and overview waveform, per-track timeline lanes, transcript grid, linter/action state, save/export/review controls, and durable local/Babel-compatible project state.

## Target Surface

- Top workspace toolbar with zoom, playback speed, overview waveform, copy support id, hotkeys, import/load/export paths.
- Diff view switch and loop context row above the waveform lanes.
- Multi-track timeline with speaker labels, mute controls, deterministic waveform blocks from segment timing, playhead, selected segment highlight, and lane-specific colors.
- Transcript toolbar with track filter, transport buttons, undo/redo, save state, and submit review.
- Transcript grid with id, speaker, editable track, editable start/end/split times, editable text, linter status, and row action menu.
- Actions for select, apply, split, merge next same lane, delete, create after, track change, save local project, load local project, import Babel JSON, export Babel JSON, and submit review.

## Behavior

- All transcript mutations route through `StudioWorkflow` and `studio-core` command history.
- One visible Apply action is one undo step.
- Draft edits in other rows survive unrelated timeline/table actions.
- Track changes are explicit lane changes and derive the speaker from the lane.
- Linter state is computed from real project/draft state: invalid timing, missing text, missing lane, and overlapping same-lane segments must be visible before submit.
- Submit review succeeds only when there are no blocking lint errors.
- Save/export write actual files and update status/dirty state.
- The app remains Babel-format compatible on import/export.

## Non-Goals For This Pass

- Real audio decoding/playback is not added until audio assets and a playback backend are introduced.
- AI drafting or auto-segmentation is not added, but the command/workflow boundary must stay plugin-ready.

## Architecture

- Keep `studio-core` as the mutation authority.
- Keep `studio-app::StudioWorkflow` as the bridge from UI actions to core commands and project IO.
- Add `studio-app::editor_model` for pure editor state/view-model logic: transport, viewport, filter, lints, waveform blocks, row projections, and submit/save state. This module is heavily unit-tested.
- Keep `studio-app/src/main.rs` focused on Iced rendering and message dispatch.

## Verification

- Unit-test editor state and view-model behavior.
- Unit-test workflow/core behavior for any command changes.
- Run `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo check --workspace`.
