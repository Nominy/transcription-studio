# Native Capability First Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first native capability foundation for Transcription Studio: a static registry, settings model, hotkeys/help integration, and service-family scaffolding without changing existing editor behavior.

**Architecture:** Add a first-party `studio_app::capabilities` module that describes built-in Helper, Reviewer, and Drafting capabilities. Wire the existing hotkeys dialog to read shortcut sections from that registry while preserving current recovered Babel rows. Keep command behavior unchanged in this first slice.

**Tech Stack:** Rust 2024, Iced 0.13, existing `studio-app` crate tests, serde/serde_json for settings persistence shapes.

---

### Task 1: Capability Registry Foundation

**Files:**
- Create: `crates/studio-app/src/capabilities/mod.rs`
- Modify: `crates/studio-app/src/lib.rs`
- Test: `crates/studio-app/src/capabilities/mod.rs`

- [ ] **Step 1: Write failing registry tests**

Add `pub mod capabilities;` to `crates/studio-app/src/lib.rs`.

Create `crates/studio-app/src/capabilities/mod.rs` with only these tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_helper_reviewer_and_drafting_packs() {
        let registry = CapabilityRegistry::default();

        assert_eq!(registry.descriptors().len(), 20);
        assert!(registry.is_enabled(CapabilityId::new("helper.custom-linter")));
        assert!(registry.is_enabled(CapabilityId::new("reviewer.generate")));
        assert!(!registry.is_enabled(CapabilityId::new("drafting.transcribe-segment")));
    }

    #[test]
    fn descriptor_lookup_returns_stable_capability_metadata() {
        let registry = CapabilityRegistry::default();
        let descriptor = registry
            .descriptor(CapabilityId::new("helper.text-move"))
            .expect("helper text move capability should exist");

        assert_eq!(descriptor.pack, CapabilityPack::Helper);
        assert_eq!(descriptor.label, "Text Move");
        assert_eq!(descriptor.commands, &["helper.text-move.before-caret", "helper.text-move.after-caret"]);
        assert_eq!(descriptor.hotkeys.len(), 2);
    }

    #[test]
    fn settings_can_disable_default_enabled_capabilities_and_enable_ai() {
        let settings = CapabilitySettings::from_overrides([
            ("helper.custom-linter", false),
            ("drafting.transcribe-segment", true),
        ]);
        let registry = CapabilityRegistry::with_settings(settings);

        assert!(!registry.is_enabled(CapabilityId::new("helper.custom-linter")));
        assert!(registry.is_enabled(CapabilityId::new("drafting.transcribe-segment")));
        assert!(registry.enabled_descriptors().any(|descriptor| descriptor.id.as_str() == "drafting.transcribe-segment"));
    }
}
```

- [ ] **Step 2: Run registry test to verify it fails**

Run:

```powershell
cargo test -p studio-app capabilities --lib
```

Expected: compile fails because `CapabilityRegistry`, `CapabilityId`, `CapabilityPack`, and `CapabilitySettings` are not defined.

- [ ] **Step 3: Implement the minimal registry**

Replace `crates/studio-app/src/capabilities/mod.rs` with a focused static registry:

- `CapabilityId` newtype with `new` and `as_str`.
- `CapabilityPack` enum with `Helper`, `Reviewer`, `Drafting`.
- `CapabilityHotkey` with `keys` and `description`.
- `CapabilityDescriptor` with id, pack, label, description, default-enabled, commands, hotkeys.
- `CapabilitySettings` with a `BTreeMap<&'static str, bool>` override model.
- `CapabilityRegistry` with static descriptors and lookup/enabled helpers.

The static descriptors must include 20 capabilities:

- Helper: `helper.hotkeys-help`, `helper.row-actions`, `helper.text-move`, `helper.focus-toggle`, `helper.quick-region-autocomplete`, `helper.selected-number-to-skaz`, `helper.timeline-selection`, `helper.timeline-defaults`, `helper.waveform-tools`, `helper.custom-linter`, `helper.extended-diff`.
- Reviewer: `reviewer.generate`, `reviewer.session`, `reviewer.templates`, `reviewer.comments`, `reviewer.analytics`.
- Drafting: `drafting.ping`, `drafting.transcribe-segment`, `drafting.redistribute-text`, `drafting.generate-rows`, `drafting.audio-cues`.

Default enable Helper and Reviewer descriptors; default disable Drafting descriptors except `drafting.ping`.

- [ ] **Step 4: Run registry test to verify it passes**

Run:

```powershell
cargo test -p studio-app capabilities --lib
```

Expected: all capability registry tests pass.

### Task 2: Hotkeys Dialog Registry Integration

**Files:**
- Modify: `crates/studio-app/src/app/components/hotkeys_dialog.rs`
- Test: `crates/studio-app/src/app/tests.rs`

- [ ] **Step 1: Write failing hotkeys integration tests**

Add tests near the existing hotkeys dialog tests in `crates/studio-app/src/app/tests.rs`:

```rust
#[test]
fn hotkeys_dialog_reads_recovered_rows_from_capability_registry() {
    let registry_rows = studio_app::CapabilityRegistry::default().hotkey_sections();
    let dialog_rows = super::components::hotkeys_dialog::recovered_hotkey_registry();

    assert_eq!(dialog_rows, registry_rows);
}

#[test]
fn capability_registry_documents_helper_text_move_hotkeys_without_rendering_disabled_ai() {
    let registry = studio_app::CapabilityRegistry::default();
    let helper = registry
        .descriptor(studio_app::CapabilityId::new("helper.text-move"))
        .expect("helper text move descriptor");
    let drafting = registry
        .descriptor(studio_app::CapabilityId::new("drafting.transcribe-segment"))
        .expect("drafting segment descriptor");

    assert_eq!(
        helper
            .hotkeys
            .iter()
            .map(|hotkey| (hotkey.keys, hotkey.description))
            .collect::<Vec<_>>(),
        vec![
            (&["Alt", "["][..], "Move text before caret to previous segment"),
            (&["Alt", "]"][..], "Move text after caret to next segment"),
        ]
    );
    assert!(!registry.is_enabled(drafting.id));
    assert!(
        !registry
            .hotkey_sections()
            .iter()
            .any(|section| section.0.contains("Drafting"))
    );
}
```

- [ ] **Step 2: Run hotkeys tests to verify they fail**

Run:

```powershell
cargo test -p studio-app hotkeys_dialog_reads_recovered_rows_from_capability_registry capability_registry_documents_helper_text_move_hotkeys_without_rendering_disabled_ai
```

Expected: compile or assertion failure because `hotkey_sections()` is not implemented and the dialog still owns a separate hardcoded registry.

- [ ] **Step 3: Wire the dialog to the registry**

Change `hotkeys_dialog.rs` so:

- `ShortcutRow` and `ShortcutSection` are replaced or populated from `studio_app::CapabilityHotkeySection`.
- `view_hotkeys_dialog()` iterates `CapabilityRegistry::default().hotkey_sections()`.
- `recovered_hotkey_registry()` returns the same registry-derived rows.
- The visible rendered copy remains the same for existing recovered Babel sections.

Add `hotkey_sections()` to `CapabilityRegistry`. It must return these enabled sections in order:

- `Global Controls`
- `1-Second Overlay`
- `Loop Controls`
- `Segment Controls (Click on segment)`

Keep Helper text-move hotkeys available on the descriptor but not in rendered sections yet, so the UI does not change in this first slice.

- [ ] **Step 4: Run hotkeys tests to verify they pass**

Run:

```powershell
cargo test -p studio-app hotkeys_dialog -- --nocapture
```

Expected: all hotkeys dialog tests pass, including the existing recovered Babel registry test.

### Task 3: Service Family Scaffolding

**Files:**
- Modify: `crates/studio-app/src/capabilities/mod.rs`
- Test: `crates/studio-app/src/capabilities/mod.rs`

- [ ] **Step 1: Write failing service-family tests**

Add tests to `crates/studio-app/src/capabilities/mod.rs`:

```rust
#[test]
fn service_family_groups_ai_and_review_capabilities_without_changing_babel_requests() {
    let registry = CapabilityRegistry::default();

    assert_eq!(
        registry.service_family_for(CapabilityId::new("reviewer.generate")),
        Some(CapabilityServiceFamily::Reviewer)
    );
    assert_eq!(
        registry.service_family_for(CapabilityId::new("drafting.redistribute-text")),
        Some(CapabilityServiceFamily::Drafting)
    );
    assert_eq!(
        registry.service_family_for(CapabilityId::new("helper.custom-linter")),
        None
    );
}
```

- [ ] **Step 2: Run service-family test to verify it fails**

Run:

```powershell
cargo test -p studio-app service_family_groups_ai_and_review_capabilities_without_changing_babel_requests --lib
```

Expected: compile fails because `CapabilityServiceFamily` and `service_family_for` are not implemented.

- [ ] **Step 3: Implement service-family metadata**

Add `CapabilityServiceFamily` with `Babel`, `Reviewer`, and `Drafting` variants. Add `service_family: Option<CapabilityServiceFamily>` to descriptors and `service_family_for()` to the registry.

For this first slice:

- Reviewer descriptors use `Reviewer`.
- Drafting descriptors use `Drafting`.
- Helper descriptors use `None`.
- Existing `BabelServiceRequest` code is untouched.

- [ ] **Step 4: Run service-family test to verify it passes**

Run:

```powershell
cargo test -p studio-app service_family_groups_ai_and_review_capabilities_without_changing_babel_requests --lib
```

Expected: pass.

### Task 4: Verification And Documentation

**Files:**
- Modify: `docs/parity/babel-we-edit-parity-map.md`

- [ ] **Step 1: Update parity map**

Append a short dated note to `docs/parity/babel-we-edit-parity-map.md` recording that Helper/Reviewer/Drafting extension behavior is now classified through the native capability registry foundation, while behavior migration remains future work.

- [ ] **Step 2: Run focused tests**

Run:

```powershell
cargo test -p studio-app capabilities --lib
cargo test -p studio-app hotkeys_dialog -- --nocapture
```

Expected: pass.

- [ ] **Step 3: Run workspace gates**

Run:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
```

Expected: pass.

- [ ] **Step 4: Review and commit scoped changes**

Run:

```powershell
git diff --check
git status --short
git diff -- crates/studio-app/src/capabilities/mod.rs crates/studio-app/src/lib.rs crates/studio-app/src/app/components/hotkeys_dialog.rs crates/studio-app/src/app/tests.rs docs/parity/babel-we-edit-parity-map.md docs/superpowers/plans/2026-06-29-native-capability-first-slice.md
```

Stage and commit only files touched by this plan:

```powershell
git add -- crates/studio-app/src/capabilities/mod.rs crates/studio-app/src/lib.rs crates/studio-app/src/app/components/hotkeys_dialog.rs crates/studio-app/src/app/tests.rs docs/parity/babel-we-edit-parity-map.md docs/superpowers/plans/2026-06-29-native-capability-first-slice.md
git commit -m "feat: add native capability registry foundation"
```
