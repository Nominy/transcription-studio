# End-to-End Native Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the first slice into a usable native Babel correction editor that can load/import a Babel package, edit text and segment structure through typed commands, save/reopen local project state, and export Babel-compatible JSON.

**Architecture:** Keep mutation logic in `studio-core` and keep `studio-app` as state/render/dispatch. Add command coverage for create/delete/merge/change-speaker and app workflow helpers for opening, saving, exporting, and applying UI edits.

**Tech Stack:** Rust 1.93 nightly, Cargo workspace, `serde`, `serde_json`, `thiserror`, `anyhow`, `tempfile`, `iced`.

---

## Task 1: Complete Core Editing Commands

**Files:**
- Modify: `crates/studio-core/src/commands.rs`
- Modify: `crates/studio-core/tests/command_history.rs`

- [x] Add failing tests for `CreateSegment`, `DeleteSegment`, `MergeSegments`, and `ChangeSegmentSpeaker`.
- [x] Implement the minimal command variants and validation.
- [x] Run `cargo test -p studio-core`.
- [x] Commit `feat: complete core segment editing commands`.

## Task 2: Add App Workflow State

**Files:**
- Modify: `crates/studio-app/Cargo.toml`
- Modify: `crates/studio-app/src/lib.rs`
- Create: `crates/studio-app/src/workflow.rs`
- Modify: `crates/studio-app/tests/view_model.rs`

- [x] Add failing tests for loading a Babel package string, applying text/time/split/merge/delete/create/speaker edits, saving a local project, reloading it, and exporting Babel JSON.
- [x] Implement `StudioWorkflow` around `Project`, `CommandHistory`, and optional local project root.
- [x] Run `cargo test -p studio-app`.
- [x] Commit `feat: add app correction workflow`.

## Task 3: Make Iced Shell Editable

**Files:**
- Modify: `crates/studio-app/src/main.rs`

- [x] Replace display-only rows with editable text/start/end inputs and action buttons.
- [x] Route input changes through `StudioWorkflow` methods.
- [x] Add buttons for split, merge-next, delete, create-after, speaker toggle, save local, and export.
- [x] Run `cargo check -p studio-app`.
- [x] Commit `feat: make native shell editable`.

## Task 4: End-to-End Verification

**Files:**
- Modify only if verification exposes bugs.

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo check --workspace`.
- [ ] Fix and commit any issues.
