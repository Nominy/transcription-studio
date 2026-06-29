# Native Capability Architecture Design

## Goal

Transcription Studio should absorb the productivity value of Babel Helper, Reviewer, and Gold Drafting into the native Rust/Iced app without recreating the old Chrome-extension architecture. The app should feel like one coherent editor: fast timeline-first correction, native hotkeys, native lint, native review workflow, and service-backed AI where it helps real transcription work.

The decision is a hybrid:

- Bake editor workflow behavior into first-party native capability packs.
- Keep AI, backend, and provider-specific work behind service adapters.
- Defer a public third-party plugin ABI until the native capability seams have proven themselves.

## Context

The old extensions were necessary because Babel was a web app we could only augment from the outside. Helper patched DOM, React state, Wavesurfer, hotkeys, lint responses, and browser behavior. Reviewer layered a review workspace and backend suggestion client on top of Babel. Gold Drafting became the AI broker and audio-drafting surface, with `reviewer/review-backend` as the OpenRouter gateway.

Transcription Studio is different. It owns the editor loop, data model, renderer, hotkeys, waveform timeline, linter state, save/submit flow, and native Babel service bridge. Keeping these features as external "extensions" would preserve the workaround shape after the original constraint is gone.

The native app already has useful boundaries:

- `studio-core`: mutation commands and command history.
- `studio-babel`: Babel import/export plus unknown-field preservation.
- `studio-waveform`: audio decode, waveform peaks, viewport/render primitives.
- `studio-app::editor_model`: pure editor session behavior.
- `studio-app::transcript_model`, `timeline_model`, and `linter`: derived UI state and rule evaluation.
- `studio-app::service_callbacks` and `service_executor`: native Babel/TRPC service dispatch.

This design extends those seams rather than replacing them.

## Non-Goals

- Do not port the Chrome extension kernel, DOM hooks, content scripts, or `chrome.runtime` broker shape into the native app.
- Do not build a marketplace-grade plugin system in the first pass.
- Do not move OpenRouter keys, model-routing policy, or long-running AI request logic into Iced view code.
- Do not weaken Babel compatibility. Unknown Babel fields must remain preserved unless a typed native model owns them losslessly.
- Do not make AI features required for normal correction, save, export, or submit.

## Architecture

Add an internal capability layer inside `studio-app`, initially as `crates/studio-app/src/capabilities/`. This is a first-party registry, not a dynamic plugin ABI.

The capability layer owns feature registration and command routing, while existing modules keep owning domain behavior. A capability should declare what it contributes, then delegate real work to the right existing owner.

Core types:

- `CapabilityId`: stable id such as `helper.custom-linter`, `helper.text-move`, `reviewer.session`, or `drafting.redistribute-text`.
- `CapabilityDescriptor`: label, description, default-enabled state, settings keys, command ids, hotkey rows, panel ids, and service capabilities.
- `CapabilitySettings`: persisted enablement and small user-tunable options.
- `CapabilityCommand`: typed native command enum for capability-triggered behavior.
- `CapabilityEffect`: result of command routing, such as core mutation, workflow action, UI state update, clipboard task, service request, or status/error message.
- `CapabilityRegistry`: static registry built at startup and passed to hotkeys, toolbars, panels, linter, and service dispatch.

The registry should not become a second app state. `StudioState`, `EditorSession`, `StudioWorkflow`, and the existing model modules remain the source of truth. Capabilities are a typed way to organize contributions.

## Capability Packs

### Helper Pack

Helper features should become native editor capabilities because they touch focus, selection, lint, waveform, transcript rows, and undo semantics.

Initial Helper capability ids:

- `helper.hotkeys-help`: extend the native hotkeys dialog from the registry.
- `helper.row-actions`: delete, merge, speaker-lane shortcuts, rewind, and workflow reset commands.
- `helper.text-move`: move text before or after the caret between adjacent segments.
- `helper.focus-toggle`: pause/blur and resume/restore text focus.
- `helper.quick-region-autocomplete`: tag autocomplete and selected-text tag wrapping in quick region and row editors.
- `helper.selected-number-to-skaz`: digit-to-SKAZ replacement and explicit conversion command.
- `helper.timeline-selection`: cut/trim preview and timeline action commands.
- `helper.timeline-defaults`: persisted zoom, lane visibility, and workflow defaults.
- `helper.waveform-tools`: waveform scale range, magnifier, minimap, and tooltip improvements.
- `helper.custom-linter`: Helper lint rules and fix actions integrated with native lint rows and submit blocking.
- `helper.extended-diff`: recovered diff details as native diff toolbar/table behavior.

These should route through native command/history paths. For example, text movement should become command-backed segment text edits, not direct string mutation in an Iced widget. Linter fix-all should produce undoable workflow changes and flush active drafts first.

### Reviewer Pack

Reviewer should become a built-in review assistant surface. Its UI belongs in native Iced panels and dialogs; its generation, template search, and session persistence belong behind a service client.

Initial Reviewer capability ids:

- `reviewer.generate`: generate review feedback for current versus original/recovered diff.
- `reviewer.session`: create/load/finalize review sessions.
- `reviewer.templates`: template search, card-template match, clear match, and suggestion approval/rejection.
- `reviewer.comments`: session and card comments.
- `reviewer.analytics`: submit review-action analytics payloads when configured.

Native state should store enough session metadata to render the assistant and recover from request failure. Generated comments must not mutate transcript text unless the user explicitly applies a suggestion.

### Drafting Pack

Gold Drafting should become a service-backed AI capability pack. Native UI should expose draft actions close to the timeline/transcript workflow, but provider policy should stay behind an adapter.

Initial Drafting capability ids:

- `drafting.ping`: report provider, remote configuration, and available operations.
- `drafting.transcribe-segment`: send selected segment audio/context and receive proposed text.
- `drafting.redistribute-text`: rebalance text across selected neighboring segments.
- `drafting.generate-rows`: generate draft rows for a selected range or loaded task context.
- `drafting.audio-cues`: request audio-aware cues through the drafting backend path.

AI operations should create previews first. Applying a draft must route through `StudioWorkflow` and `studio-core` commands so undo/redo, dirty state, lint, save, and submit behave normally.

## Service Boundary

Extend the existing service dispatch model instead of inventing a new transport for each feature.

Add a service namespace above `BabelServiceRequest`:

- `BabelServiceRequest`: existing live Babel dashboard/TRPC operations.
- `ReviewerServiceRequest`: review backend operations under `/api/review`.
- `DraftingServiceRequest`: drafting and AI broker operations under `/api/draft` or a compatible local adapter.

`service_executor` should remain the central async executor for configured command adapters, fixtures, live Babel calls, and review-backend calls. It should normalize responses into typed `Message` variants and never leak provider secrets into status text or logs.

Preferred service order:

1. Test fixture payload or fixture path.
2. Explicit command adapter for local development.
3. Live HTTP backend when configured.
4. Clear unavailable/error state in the native UI.

Long-running AI should support progress events. The first implementation can be request/response if that is faster to ship, but the request types should leave room for streaming progress such as accepted, capturing audio, calling backend, waiting, result, and error.

## Data Flow

User input flow:

1. Iced event or toolbar action enters `app::message` / `app::hotkeys`.
2. The app resolves a native command directly or asks `CapabilityRegistry` for a capability command.
3. The command router checks settings, read-only state, active modal/focus state, and project availability.
4. Pure behavior runs in `EditorSession`, `StudioWorkflow`, `studio-core`, `linter`, `transcript_model`, or `timeline_model`.
5. Async work becomes a service request and returns through typed `Message` settlement.
6. View models update from project/editor state, not from ad hoc capability-owned shadow state.

Lint flow:

1. Project plus active drafts become lint input.
2. Native linter registry runs built-in and Helper rules.
3. Asserted warnings, fix availability, and blocking state feed transcript rows, problem navigation, and submit controls.
4. Fix commands route through workflow/core mutations and create undoable changes.

AI flow:

1. User asks for generation from a selected row, range, diff, or session.
2. Native code snapshots the minimal Babel-compatible context and any needed audio slice references.
3. Service adapter calls the backend.
4. Native UI shows pending/progress/error state.
5. Result lands as a preview.
6. User applies, skips, edits, or discards the result.

## Settings And UI

Use one native settings surface for capability enablement and advanced options. Defaults should match the productive Helper setup: workflow helpers enabled, AI features visible only when configured or explicitly enabled.

Settings should be persisted locally with the same care as existing panel collapse state. Capability settings must be read by hotkey routing, toolbar visibility, lint registry, and service availability.

Native UI placement:

- Helper workflow controls stay in the editor toolbar, transcript row actions, timeline overlays, hotkeys dialog, and lint cells.
- Reviewer lives in a right-side assistant panel or modal workflow, depending on density.
- Drafting lives near transcript/timeline selection: selected row actions, range actions, and an AI/drafting panel.
- Service configuration belongs in settings and dashboard-level connection state, not inside every feature panel.

## Error Handling

Errors should be precise and recoverable:

- Disabled capability: command is unavailable and hotkeys/help reflect disabled state.
- Missing project/selection/audio: show a local validation message and do not call a backend.
- Read-only task: block mutations but allow safe review/diff inspection.
- Backend unavailable: preserve current editor state and show retry/configuration guidance.
- AI result malformed: keep the preview empty, show the parse failure, and keep the original transcript untouched.
- Apply failure: leave preview intact, report the command failure, and preserve undo history.

No service error should cause transcript edits to be partially applied. AI features are preview-first for this reason.

## Testing

Every capability slice should start with narrow tests before production behavior changes.

Required tests by layer:

- Capability registry: descriptors, default settings, enable/disable behavior, hotkey/help rows.
- Command routing: read-only guards, focus/modal guards, missing selection guards, and generated effects.
- Core/workflow: undoable mutations for text move, linter fixes, AI apply, row actions, and timeline actions.
- Linter: each Helper rule, rule errors, warning assertion, fix-one, and fix-all.
- Service clients: request payloads, response normalization, embedded error payloads, auth/header redaction, fixture mode, and command-adapter mode.
- UI models: panels, pending/error/success states, previews, disabled controls, and toolbar visibility.
- Runtime smokes: only for behavior that depends on real Iced input/rendering, audio playback, OS clipboard, or long-running async flow.

When a behavior is ported from recovered Babel, update `docs/parity/babel-we-edit-parity-map.md` with evidence. Continue running the recovered Babel verifier suite when the slice claims recovered parity:

```powershell
cd C:\Users\User\Desktop\dev\babel\tools\babel-editor-rebuild\app
npm run verify:ported
npm run verify:fixtures
npm run verify:adapters
```

Native gates remain:

```powershell
cd C:\Users\User\Desktop\dev\transcription-studio
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
```

## Rollout Order

1. Add the internal capability registry, settings model, and hotkeys/help integration without changing behavior.
2. Migrate already-native Helper-like behaviors into capability descriptors so the registry reflects the real product surface.
3. Port missing Helper workflow features in small slices: text move, selected-number-to-SKAZ, linter fix commands, timeline selection, magnifier/minimap, and extended diff controls.
4. Add `ReviewerServiceRequest` and a minimal native review assistant panel with generate/session/template operations behind fixtures and mock tests.
5. Add `DraftingServiceRequest` and preview-first native drafting commands for selected segment and redistribute text.
6. Add progress/streaming support for long-running drafting and reviewer requests.
7. Reassess whether any capability needs an external ABI. Only expose sidecar/plugin APIs after the first-party registry has stayed stable through real usage.

## Acceptance Criteria

The architecture pass is complete when:

- `studio-app::capabilities` exists with a static registry and persisted settings model.
- The hotkeys dialog, toolbar/panel availability, and command routing can read capability descriptors.
- Existing native Helper-like behavior is represented in the registry without behavior regressions.
- Service request routing has distinct Babel, Reviewer, and Drafting request families.
- The first migrated Helper slice has tests proving command routing, undo/redo, lint/save/submit interaction, and disabled/read-only behavior.
- The parity map records which old extension behaviors are native core, service-backed native capability, deferred, or intentionally obsolete.

The product direction is complete only when Helper, Reviewer, and Drafting no longer feel like add-ons. They should feel like Transcription Studio features that happen to preserve the hard-won Babel workflows.
