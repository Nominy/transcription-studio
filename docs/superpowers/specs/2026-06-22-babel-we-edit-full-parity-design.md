# Babel We Edit Full Parity Design

## Goal

Transcription Studio must become a native Rust/Iced replacement for Babel We Edit, with behavior parity measured against `C:\Users\User\Desktop\dev\babel\tools\babel-editor-rebuild`, not against the current native first slice. The app is complete only when every recovered Babel behavior, expected browser interaction, shortcut, smoothness expectation, audio/timeline behavior, and rendering performance requirement has a native equivalent with direct verification.

## Current Baseline

The existing native app has a useful foundation: Babel-shaped data, local project materialization, import/export, command-backed edits, transcript rows, toolbar controls, lint status, timeline lanes, loop state, PCM WAV decoding, and Rodio playback. It is not full parity. The previous design file was scoped to a first usable editor and explicitly left audio and many We Edit surfaces outside the pass. This design replaces that scope.

The target Babel rebuild currently proves the recovered behavior surface with:

- `npm run verify:ported`
- `npm run verify:fixtures`
- `npm run verify:adapters`

The native workspace currently has at least one red parity test in `studio-app` around diff presentation source structure. That failure should be treated as evidence that the native UI still has approximate ports where recovered Babel-specific presentation primitives are required.

## Non-Negotiables

- A feature is not parity because it looks similar. It is parity only when native state, input handling, output, and edge cases match the recovered Babel behavior or the deviation is explicitly documented and approved.
- The parity source of truth is `babel-editor-rebuild`, especially the verifier scripts and recovered `app/src/recovered/ported` modules.
- Native tests must be written before production behavior changes.
- Browser-provided behavior must be named and ported. If the web app got focus, selection, clipboard, key repeat, pointer capture, scrolling, text input, or default button behavior for free, the native app must implement an equivalent.
- Smoothness is part of parity. Dragging, scrolling, playhead movement, text entry, row selection, waveform rendering, and keyboard actions must feel immediate on realistic Babel projects.
- Babel compatibility must preserve unknown fields unless a typed native model owns and round-trips them losslessly.
- AI drafting is still out of v1 behavior unless added behind stable plugin seams. The parity port must not depend on AI.

## Parity Levels

Each row in the parity map has one status:

- `Missing`: no credible native equivalent.
- `Partial`: a visible or model-level equivalent exists but does not cover recovered edge cases.
- `Blocked by design`: the current native architecture cannot support parity without a boundary change.
- `Implemented`: native code exists and targeted tests pass.
- `Verified`: target Babel verifier passes, native targeted tests pass, and any needed runtime/manual evidence is recorded.

Only `Verified` counts as done for the full goal.

## Architecture

Keep the current workspace boundaries:

- `studio-core`: mutation commands, command history, segment and annotation invariants.
- `studio-babel`: Babel package import/export and unknown-field preservation.
- `studio-waveform`: decoded audio, waveform peaks, viewport math, render-cache primitives.
- `studio-app`: Iced app state, message dispatch, native input model, timeline/rendering, dialogs, transcript table, and review workflow.

Add focused parity modules instead of burying recovered behavior in large UI files:

- `app::parity::browser_input`: focus ownership, key routing, modifier handling, editable text capture, modal escape/default handling, pointer drag lifecycle, wheel/scroll behavior, clipboard commands.
- `app::parity::hotkeys`: recovered Babel shortcut registry and command mapping.
- `editor_model::review_action_flow`: claim, submit, drop, skip, admin flag, pending/error state.
- `editor_model::page_controller`: mode/read-only/admin/no-work/manual lookup transitions and recording URL mapping.
- `editor_model::submit_control`: recovered submit, skip, admin, assessment, and practice states.
- `editor_model::context_panels`: TTS script context panels and collapse/copy state.
- `linter`: complete recovered linter registry and issue mapping.
- `transcript_model`: row, table, diff, low-confidence, annotation operation, and reviewer projections.
- `timeline_model`: loop, track viewport, navigation, overlay, speed, split guards, autosave tick, and buffered playback range.
- `app::rendering`: cached waveform/timeline geometry, viewport invalidation, frame budgets, and GPU-friendly draw paths.

These modules should expose pure functions where possible. Iced widgets should call already-derived view models rather than recomputing Babel behavior during rendering.

## Browser Behavior Parity

Babel ran inside a browser, so the native app must replace these implicit behaviors:

- Focus graph: active row, active cell, active text input, active modal, active popover, active timeline drag, and active global workspace must be mutually clear.
- Keyboard capture: text fields keep normal editing keys; global shortcuts run only when Babel would route them globally; Escape clears the deepest active state first.
- Modifier handling: Ctrl on Windows/Linux and Command on macOS must map to the same recovered intent where Babel used platform meta behavior.
- Key repeat: held navigation and transport keys must not create duplicate mutations unless Babel allowed repeated actions.
- Clipboard: support-id copy, submission-id copy, row text copy, and normal text-field clipboard behavior must work through the OS clipboard.
- Pointer lifecycle: drag start, drag move, drag release, cancel-on-exit, nested select close guards, and popover outside-click handling must match Babel.
- Scrolling: transcript scrolling, timeline scrolling, horizontal viewport changes, wheel deltas, touchpad deltas, and scrollbar drags must not fight each other.
- Text editing: selection, caret movement, commit-on-enter, cancel-on-escape, blur commit rules, and invalid time input behavior must match the recovered row/edit-popover/create-region logic.
- Modal defaults: submit, skip, admin, no-work, task lookup, hotkeys, create-region, and edit-popover must preserve default button, focus trap, non-dismissible, and close sequencing semantics.

## Rendering And Performance

Iced/wgpu can be fast enough, but the app must be engineered for retained rendering:

- Waveform drawing must use cached geometry keyed by lane, decoded audio identity, viewport, zoom, vertical zoom, and pixel width.
- Hover and drag updates must invalidate only the overlays or ranges that changed.
- Large peak arrays must not be cloned per frame.
- Timeline lanes should render through canvas/cache surfaces where practical instead of hundreds of small widgets rebuilt on every mouse move.
- Playback state should update the playhead without recomputing all transcript rows.
- Audio speed changes should update the playback backend and UI model together.
- Performance verification should include large-project fixtures, dragging loop handles, scrolling transcript rows, moving the playhead, editing text, and repeated hotkeys.

Acceptance target: no visible input lag during normal editing, smooth playhead movement, responsive drag/scroll on realistic two-speaker Babel projects, and no obvious CPU spikes from avoidable per-frame allocation.

## Verification

Target verification remains:

```powershell
cd C:\Users\User\Desktop\dev\babel\tools\babel-editor-rebuild\app
npm run verify:ported
npm run verify:fixtures
npm run verify:adapters
```

Native verification for every slice must include the narrow test that proves the behavior and the workspace gates:

```powershell
cd C:\Users\User\Desktop\dev\transcription-studio
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
```

When a slice involves visual or interaction behavior, add a runtime smoke note to `docs/parity/babel-we-edit-parity-map.md` after manually running `cargo run -p studio-app` and verifying the exact interaction.

## Implementation Order

1. Establish and maintain `docs/parity/babel-we-edit-parity-map.md` as the source-of-truth ledger.
2. Fix current red native parity test for diff presentation with test-first changes.
3. Port the recovered hotkey registry and browser input routing, because this controls the "keys just work" feel.
4. Complete row, table, edit-popover, create-region, linter, diff, and low-confidence behavior.
5. Complete review action, submit, task lookup, page controller, no-work, feedback, and assessment flows.
6. Complete TTS/script context panels and support/copy surfaces.
7. Complete timeline, loop, track viewport, playback speed, buffered playback, autosave tick, and waveform performance work.
8. Re-run full target and native verification, then perform an end-to-end native Babel package smoke test.

## Completion Definition

The goal is complete only when every parity-map row is `Verified`, `cargo test --workspace` is green, the Babel rebuild verifier suite is green, the native app opens and runs the recovered fixture with audio/timeline/transcript interactions, and no approved scope gaps remain.
