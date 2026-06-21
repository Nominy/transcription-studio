# Babel Replacement Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a substantially functional native Babel replacement editor surface from the screenshot: waveform-first timeline, transport/loop/zoom state, transcript grid editing, linter/action state, save/export/review workflow, and tests.

**Architecture:** Keep project mutation in `studio-core` and `StudioWorkflow`. Add a pure `editor_model` module for editor/session state and view projections, then rebuild the Iced shell around those projections. The UI stays dense and operational like Babel.

**Tech Stack:** Rust 1.93 nightly, Cargo workspace, `iced` 0.13, `serde`, `serde_json`, `anyhow`, `thiserror`, `tempfile`.

---

## Task 1: Editor Session Model

**Files:**
- Create: `crates/studio-app/src/editor_model.rs`
- Modify: `crates/studio-app/src/lib.rs`
- Test: `crates/studio-app/tests/editor_model.rs`

- [x] Add tests for default editor state, viewport bounds, track filter options, timeline lane projections, deterministic waveform blocks, linter rows, dirty/saved/review state, and submit blocking on lint errors.
- [x] Implement `EditorSession`, `TrackFilter`, `ReviewState`, `LintSeverity`, `TimelineLaneView`, `WaveformBlock`, and `TranscriptRowView`.
- [x] Export the model types from `studio-app`.
- [x] Run `cargo test -p studio-app --test editor_model`.
- [x] Commit `feat: add babel editor session model`.

## Task 2: Workflow Commands For Replacement UX

**Files:**
- Modify: `crates/studio-app/src/workflow.rs`
- Test: `crates/studio-app/tests/workflow.rs`

- [x] Add tested editor/session navigation and time bounds in `editor_model`.
- [x] Keep workflow helpers unchanged where existing command coverage already supports the shell.
- [x] Keep all mutations routed through existing core commands.
- [x] Run `cargo test -p studio-app`.
- [x] Commit `feat: rebuild native shell as babel editor`.

## Task 3: Babel-Like Native Shell

**Files:**
- Modify: `crates/studio-app/src/main.rs`

- [x] Replace the form-style layout with a Babel-like editor: top toolbar, overview waveform, diff switch, loop row, multi-track waveform lanes, transport/table toolbar, transcript grid.
- [x] Add messages for zoom, speed, play/pause, previous/next segment, loop toggle, diff toggle, lane mute, track filter, row select, row action menu, save, export, submit review, and hotkey/support actions.
- [x] Wire all row mutations through `StudioWorkflow` and mark the editor session dirty.
- [x] Show linter status and block submit on lint errors.
- [x] Run `cargo test -p studio-app`.
- [x] Commit `feat: rebuild native shell as babel editor`.

## Task 4: Verification And Review

**Files:**
- Modify only if verification or review exposes bugs.

- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo test --workspace`.
- [x] Run `cargo check --workspace`.
- [ ] Dispatch read-only spec and code-quality subagent reviews.
- [ ] Fix reviewed issues with failing tests first.
- [ ] Commit final fixes.
