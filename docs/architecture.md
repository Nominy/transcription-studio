# Sonus architecture

Implementation target for the platform cutover; this document does not claim that every
interface below is already available. Public release requires the shipped capabilities to
be implemented and exercised end to end.

Sonus is a segment-based transcript editor with multi-lane audio playback. That is the whole
core. Everything else — AI transcription, linting, file formats, task platforms such as Babel —
is an **extension**, and platforms are **modes** contributed by extensions. Built-in features use
exactly the same API that third-party extensions use; there is no private side door.

```mermaid
flowchart TD
    App["studio-app: bundled composition"] --> Shell["sonus-shell: editor UI and contribution registry"]
    App --> Extensions["First-party Rust extensions"]
    Extensions --> Shell
    Shell --> Core["Core document, audio and project crates"]
    Shell --> API["sonus-extension: neutral contracts"]
    Shell --> Wasm["sonus-wasm: component runtime"]
    Shell --> Marketplace["sonus-marketplace: GitHub index and packages"]
    Wasm --> API
    Marketplace --> API
```

## 1. Core

The core owns only what every transcript editor needs.

| Crate | Owns |
|---|---|
| `studio-core` | `Project`, speakers, audio lanes, segments, opaque extension metadata, transactions and undo history. No platform rules. |
| `studio-audio` | Common audio decoding, multi-lane playback, transport and processed-audio variants supplied by extensions. |
| `studio-waveform` | Waveform peaks and rendering. |
| `studio-project` | Versioned native project folders, copied audio, atomic save and recovery snapshots. |
| `studio-editor` | Segment editing, boundaries, cut/loop selection, find, playback-follow, caret navigation, keymaps and view state. Consumes diagnostics and presentation providers; contains no checking rules. |
| `sonus-extension` | Versioned manifests, dependency resolution and UI-independent document/format/provider contracts. |
| `sonus-wasm` | Sandboxed component instances and isolated compiler worker. |
| `sonus-marketplace` | Reviewed GitHub catalog, package validation, installation, updates and rollback. |
| `sonus-shell` | GPUI surfaces, owner-scoped contribution lifecycle, native/WASM bridges and global settings. |
| `studio-app` | Thin executable that selects and registers the bundled extensions. |

The core has **no** linter, no AI, no import/export besides its native format, no platform
logic, and no notion of "submit". "Read-only", "required checks" and "where Save goes" are
policies a mode supplies for the documents it opens.

Existing implementation crates retain their `studio-*` names. `studio-services` contains
only native HTTP/runtime/path/storage/updater services and native recovery delegated to
`studio-project` with an explicit audio source base. `studio-inference` contains provider
contracts, not an inference runtime. `extensions/gold` (`sonus-gold`) owns the local runtime,
remote coordinator, AI preferences and provider-neutral drafting/polish algorithms;
`extensions/babel-services` (`sonus-babel-services`) owns authentication, task/review clients,
relay hosting and Babel JSON recovery. Babel clients capture direct or relay sessions per
document origin; saved global login state never chooses a document's save route. Constructors
do not activate modes; the host owns the selected services and shuts down their runtime/relay
resources when deactivating them. Existing credential, model-cache and draft storage paths
and serialized preference keys remain unchanged.

## 2. Extensions

An extension is a package with a manifest and code:

`manifest.json` uses the same strict schema as `sonus_extension::ExtensionManifest`:

```json
{
  "id": "acme.filler-words",
  "name": "Filler words",
  "version": "1.0.0",
  "api": "^0.1",
  "description": "Highlights filler words and offers to remove them.",
  "license": "MIT",
  "runtime": "wasm",
  "dependencies": {},
  "permissions": [
    { "kind": "document-read" },
    { "kind": "document-edit" }
  ],
  "modes": []
}
```

WASM packages contain `module.wasm`; contributions are registered through the typed
activation API. A manifest declaration is not a permission grant. Catalog publisher and
repository identity are validated separately from the extension's own display metadata.

Built-in extensions live in `extensions/` as Rust crates and are compiled into the app.
Third-party packages run as WASM components. Installation state and versioned packages live
under the existing application-support directory's `extensions/` subtree; developer folders
are explicitly approved separately. Existing application-support paths remain unchanged.

### 2.1 Lifecycle

Native extensions register a manifest and activation factory with
`sonus_shell::contributions::register_native_extension`. The host retains that factory for
disable/re-enable, honors persisted preferences, and removes owner-scoped contributions
and resources on deactivation. Mode dependencies prevent unsafe removal.

WASM activation and reload are acknowledged transactions. Traps, queue abuse and terminal
removal revoke that runtime generation; queued messages cannot reactivate it. Native Rust
code is trusted code with application privileges, not a sandbox: returned activation errors
are rolled back, but native panics must not be described as WASM-style containment.

### 2.2 Contribution points

Extensions extend every part of the editor — the same reach Unity editor extensions have over
the Unity editor.

**Workspace and modes**
- **Mode** — a platform or workflow: Home card, its own home view (dashboard/browser), sign-in
  and session, document sources (list/browse/claim/open → `Project` + origin), the save /
  submit pipeline for its documents, document policy (read-only, checks required before
  submit, which extensions are active in its documents), a settings page. `sonus.local` is a
  mode like any other.
- **Document handlers** — open-file and drag-and-drop handlers by extension/MIME.
- **Importers / exporters** — bytes ⇄ `Project` (SRT, VTT, TXT, platform JSON, …).

**UI**
- **Panels** — dockable views (left/right/bottom, tabbed), optionally scoped to a mode or
  document kind. Rust: any gpui view. WASM: declarative UI tree (§3.3).
- **Windows and dialogs** — modal/non-modal, with return values.
- **Inspectors** — sections in the segment inspector for the selected segment(s).
- **Transcript extensions** — extra columns/cells per row, row badges, row context-menu items,
  text decorations (styled spans: tags, highlights), gutter markers, hover tooltips,
  inline widgets after text.
- **Timeline extensions** — overlays drawn per lane or across lanes (ranges, markers, curves,
  labels, images; Unity "gizmos"), lane-header widgets, region context-menu items, custom
  region styling, hit-testable interactive handles that emit events.
- **Toolbar, menus, status bar** — buttons, toggles, dropdowns, menu items with `when` clauses,
  status-bar items with click actions.
- **Command palette and keybindings** — commands with titles, default chords, contexts
  (`transcript`, `timeline`, `global`, mode/kind-specific) and conflict reporting in the shared
  keymap editor.
- **Notifications** — toasts, progress, confirmation prompts.
- **Settings pages** — auto-generated from a JSON Schema, or a custom view.

**Editing intelligence**
- **Diagnostics** — providers run on document snapshots (incremental: changed segments only,
  debounced by the host) or publish asynchronously into a named collection (e.g. server-side
  checks). A diagnostic has rule id, severity (error/warning/info/hint), segment, optional char
  range, message, and optional quick fixes. The core shows them (row dots, counts, navigation,
  underlines); it never decides what a rule is.
- **Quick fixes and code actions** — edits offered for a diagnostic, a selection or a segment;
  "fix all" as a command.
- **Completions** — trigger characters and contexts (e.g. tag names after `[`), with insert
  text, ranges and documentation.
- **Formatting / on-type rules** — transforms applied on commit of a segment edit.

**Audio and AI**
- **Transcription providers** — produce segments/words for lanes or ranges (streaming).
- **Alignment providers** — word timings for existing text.
- **Audio processors** — processed lane variants (denoise, enhancement) mixed by the engine.
- **Audio analysis access** — decoded samples for a lane range (permission `audio:read`).

**Hooks and events**
- Document opened/closed/saved/changed (with change sets), before-save and before-submit
  (may veto with a reason), selection/focus changed, playback started/stopped/seeked, mode
  entered/left, app quitting (may request save).

**Services available to every extension**
- Document API: read snapshot, apply a transaction of edits (undoable, labelled), per-extension
  segment/project data (`ext.<id>.*`), selection, caret, playhead, transport control.
- Tasks: background jobs with progress, cancellation and the Tasks UI.
- Storage: settings (schema-validated), key-value state, secrets (OS keyring), cache directory.
- Network: HTTP client limited to manifest-declared hosts.
- Logging into the per-extension log shown on the Extensions page.
- Inter-extension services: an extension may publish a typed service (e.g. `sonus.babel`
  exposes its Babel client to `sonus.babel-relay`); consumers declare the dependency.

### 2.3 Modes and documents

Every open document records its **origin**: the mode that opened it and an opaque key.
Save, recovery, submit, read-only state and the set of active document-scoped extensions
are resolved through that origin — never from global sign-in state. Leaving the editor
returns to the origin mode's home view. Modes may activate extensions only for their own
documents (e.g. Babel's rule sets apply only to Babel tasks).

The shell retains the active `EditorModel` independently of its rendering view.
Extension panels may query the document during rendering without reentering
`EditorScreen`'s GPUI entity borrow.
Settings and Extensions open as overlays: the active document and its mode requirements
remain intact. Disabling optional providers removes their diagnostics from the open document;
empty-text checks are not built into core draft validation.

Modes supply ordered local audio candidates through `ProviderEnvironment.audio_sources`.
Core loading never guesses platform filenames. Babel supplies its cache/export naming rules
without rewriting canonical remote source references; decoding still runs off the UI thread.

A mode declares **required extensions** (with version ranges) and optional **recommended**
ones in its manifest:

```json
{
  "id": "sonus.babel",
  "name": "Babel",
  "requires": {
    "sonus.babel-lint": "^0.1",
    "sonus.babel-tags": "^0.1"
  },
  "recommends": { "sonus.gold": "^0.1" }
}
```

Entering a mode resolves all required extensions and version ranges before opening its
documents. Bundled packages satisfy requirements without network access. Missing packages
are offered through the marketplace; new permissions require user approval. Required
extensions activate for the mode's documents, not globally, and cannot be disabled while
those documents are open. Activation is transactional: an incompatible dependency, cycle,
denied permission or failed activation leaves the previous workspace usable. The Extensions
page shows which mode requires each extension and why.

## 3. Extension APIs

### 3.1 Rust (`sonus-extension`)

The UI-independent contracts are in `sonus-extension`; native UI registration is exposed by
`sonus-shell`. For example, the bundled basic-checks extension is explicitly registered:

```rust
use gpui_kit::{App, BorrowAppContext};
use sonus_shell::ContributionRegistry;

fn activate(cx: &mut App) -> anyhow::Result<()> {
    cx.update_global::<ContributionRegistry, _>(|registry, _| {
        registry.register_document_providers(
            sonus_lint_basic::ID,
            "sonus.lint-basic.checks",
            vec!["sonus.local".into()],
            sonus_lint_basic::providers(),
        )
    })
}

pub fn register(cx: &mut App) -> anyhow::Result<()> {
    sonus_shell::contributions::register_native_extension(
        sonus_lint_basic::manifest()?,
        activate,
        cx,
    )
}
```

Rust extensions may also return arbitrary gpui views for panels, windows, inspectors and mode
home views. Built-ins are Rust extensions; they never reach into the shell or core internals.

### 3.2 WASM (`wit/sonus-extension.wit`, world `sonus:extension/extension@0.1.0`)

The guest exports lifecycle, command, UI-event and document-event callbacks. Providers
publish typed diagnostics, completions, decorations, UI trees and overlays through host
imports; mode and format requests are typed events. `sdk/rust` wraps the generated bindings.

Host calls enforce explicit grants and document scope. Guest document IDs identify an
individual open incarnation, not a persisted project ID; stale revisions and handles from a
closed document cannot mutate its replacement. Instance calls have fuel, epoch, memory and
output limits. Untrusted component compilation runs in the separate `sonus-wasm-worker`
process with memory and termination limits; externally supplied native/AOT artifacts are not
accepted as components.

Guest widget trees are constructed bottom-up, preserving sibling order and skipping inactive
tab branches without recursive widget construction. Docked guest panels have bounded width;
extension mode pages retain the full workspace. The Windows UI executable reserves an 8 MiB
main stack for composed GPUI views; this does not change isolated worker limits.

### 3.3 Declarative UI

WASM extensions (and Rust extensions that prefer it) describe UI as a tree the host renders
with native components: layout (`stack`, `row`, `scroll`, `split`, `tabs`), content (`text`,
`markdown`, `icon`, `image`, `waveform-snippet`), inputs (`button`, `toggle`, `text-input`,
`number`, `select`, `slider`, `list`, `table`), feedback (`progress`, `badge`, `callout`).
Events (click, change, submit, row-select) are routed back to the extension, which returns an
updated tree or a patch. Timeline overlays use a draw list (rect/range/line/curve/marker/label
in time-and-lane coordinates, with hit regions).

## 4. Built-in extensions

| Extension | Kind | Contributes |
|---|---|---|
| `sonus.local` | mode | Open common audio files, one lane per file, without an account. Core native project save/reopen remains available without this extension. |
| `sonus.formats` | formats | SRT, WebVTT and plain text import/export. Plain text imports explicitly have no timing. |
| `sonus.lint-basic` | diagnostics | Optional language-neutral whitespace, empty-segment, timing and same-lane overlap checks with safe fixes. Active in Local documents by default, but not required by the mode. |
| `sonus.gold` | AI | Local Gold runtime: transcription, word alignment, denoise; model manager and settings. |
| `sonus.babel` | mode | Babel sign-in, dashboard, claims, task open/save/submit, review and feedback, error tasks, AI review. Requires `sonus.babel-lint` and `sonus.babel-tags`; recommends `sonus.gold`. |
| `sonus.babel-lint` | diagnostics | Babel's Gold (gold-non-bg) rules and the Helper rule set, with their autofixes; active in Babel documents. |
| `sonus.babel-tags` | editing | Babel tag/curly/angle conventions: completions, decorations, tag-aware autofixes and number→СКАЗ conversion. |
| `sonus.babel-relay` | mode | Subaccount sign-in through a relay, relay host management; uses `sonus.babel`'s client service and has the same requirements. |

The Babel extension also supplies `sonus.babel-file`: offline Babel JSON editing with
the same required rule/tag extensions and a file-only save target. It never selects a
remote client from whichever account happens to be signed in.

## 5. Distribution

Sonus ships **batteries included**: the release bundles the first-party extensions above.
Users can open audio, edit, check, export and enter available modes without separately finding
extensions. AI runtime/model downloads remain explicit, with download size, licensing and
progress shown before installation; bundled extension code does not mean all model weights
are bundled. Extensions remain independently managed; required extensions activate within
their modes rather than becoming unconditional core features.

Third-party extensions are `.sonusext` files: a zip with `manifest.json`, `module.wasm`
and optional assets. Install from the Extensions page (file or URL); the page lists
extensions with version, source (bundled/installed/dev), permissions, required-by, settings,
logs and enable/disable/uninstall. Developer mode loads an unpacked folder and reloads it on
change. `extensions/examples/` holds templates.

## 6. GitHub marketplace

Each extension is a GitHub repository owned by its author. Its repository contains
`manifest.json`, source, license, documentation and versioned GitHub Releases with
`.sonusext` assets. A public index repository (`Nominy/sonus-extensions`) is the discovery
and reviewed release catalog, not a server that executes third-party repository code.
Index contributions are pull requests.

### Catalog records

The index has a versioned schema. Each extension record identifies the stable extension ID,
GitHub owner/repository, display metadata, license, categories and released versions.
Each version records the exact manifest, Sonus API compatibility, dependencies, declared
permissions, release tag, immutable source commit, asset name, byte size and SHA-256 digest.
Records may mark a release withdrawn with a reason. Renames or transfers cannot silently
reassign an existing extension ID.

The app caches a validated catalog and displays source repository, publisher, version,
compatibility, permissions, dependencies and bundled/installed status. Search, install,
update, disable and uninstall are available in the Extensions screen. Repository README
links provide documentation; repository Issues links provide support.

### Install and update

The host resolves compatible dependencies from the catalog, presents permission changes,
downloads the pinned release asset, verifies its size and SHA-256, and validates that its
manifest matches the catalog entry. A checksum detects substituted assets; it is not an
independent publisher signature. Catalog changes therefore remain a trust boundary reviewed
through the index repository.

Extraction rejects path traversal, absolute paths, symlinks and oversized expanded packages.
Installation is staged and switched atomically; the previous working version remains
available for rollback. A moved tag or replaced GitHub asset cannot silently change an
installed version. Updates never activate halfway through an editing transaction or remove
a dependency required by an open document.

Mode requirements use the same resolver for bundled, installed and catalog packages. When
offline, already-installed compatible dependencies continue to work; unavailable
requirements produce a precise error rather than silently disabling required behavior.

Marketplace packages run as WASM components under the host's permission and resource limits.
Native Rust extensions are trusted compiled-in components, not arbitrary downloaded DLLs.
Unpacked developer extensions are a separate, explicitly enabled development source and are
not represented as reviewed marketplace releases.

### Publishing and verification

The SDK provides a GitHub repository template and release workflow that builds a real WASM
component, validates the package, and emits the index version record. Index CI validates
schema, identity, compatibility and dependency references and verifies release-asset hashes;
it does not run contributed build scripts with repository secrets or on self-hosted runners.
A first-party example must exercise repository → release → index → in-app discovery →
installation → invocation → update/rollback before the marketplace is considered complete.

