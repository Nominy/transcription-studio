# Exaudio Rust component SDK

The API is `exaudio:extension/extension@0.1.0` in `wit/exaudio-extension.wit`.
`exaudio-sdk` generates guest bindings with wit-bindgen 0.46; `exaudio-wasm` generates
host bindings with Wasmtime 38. There is no WASI linker, ambient filesystem,
process spawning, networking, clock or random source. Use the typed host imports.
The example is entirely offline.

## Build and package

For an independently cloneable author repository, use the repository generator:

```text
exaudio-extension-index init-author OWNER REPOSITORY EXTENSION_ID VERSION SDK_GITHUB_URL FULL_SDK_COMMIT DESTINATION
```

Supply an actual accessible SDK repository URL and full published commit; there is
no invented default release. The generated standalone repository has its own
`src/lib.rs`, manifest and release workflow. Its Cargo dependency pins `exaudio-sdk`
by Git URL and revision; Cargo's complete SDK repository checkout supplies the WIT
package. Keep the MIT notices and commit the generated Cargo.lock file.

To build the in-tree example instead, install the `wasm32-unknown-unknown` Rust
target and `wasm-tools`, then run these commands from this repository root:

```powershell
cargo build --manifest-path extensions/examples/hello-panel/Cargo.toml --target wasm32-unknown-unknown --release
wasm-tools component new extensions/examples/hello-panel/target/wasm32-unknown-unknown/release/exaudio_example_hello_panel.wasm -o module.wasm
pwsh -File extensions/examples/hello-panel/package.ps1 -Component module.wasm
```

Run build/test work in the project's detached psmux runner with durable logs,
status, cancellation and restart boundaries. These commands do not start hidden
background work. Cargo compilation is a restart boundary (incremental artifacts
survive cancellation); a failed smoke test restarts the test, not an external or
paid operation. Do not publish the `runtime-probes` feature build.

`package.ps1` emits an immutable `example.hello-panel-0.1.0.exaudio`, plus its byte
size and SHA-256 receipt. The ZIP has `manifest.json`, `module.wasm`, and
`assets/LICENSE`, matching the GitHub marketplace installer. It refuses to replace
an existing release artifact. Upload that file to the author's versioned GitHub
Release and submit its exact manifest, release source commit, size and digest to
the index. The script does not create repositories, upload releases, or approve
permissions. The component header check is not a substitute for the host's full
component/world validation.

## Native host integration

Implement `exaudio_wasm::HostServices` (`crates/exaudio-wasm/src/services.rs`) and create
`Runtime::spawn(manifest.id, Arc<dyn HostServices>, Limits)`. `load(Package)` takes
verified component bytes, the shared `ExtensionManifest`, explicit user grants,
and every constraint from modes currently open. Declared permissions are not grants.
`load`, `invoke`, and `unload` return `Ticket`; poll `try_recv` from an application
async/UI task. Never busy-wait on the UI thread. `Ticket::cancel` cancels queued or
running work. Guest calls and synchronous host imports run on a dedicated
bounded-queue worker; untrusted compilation runs in the separate limited compiler
process described below. Dropping Runtime signals cancellation without joining
on the UI thread. Native services must honor `CallContext::check` and the deadline,
including immediately before committing a queued editor mutation. Wasmtime cannot
interrupt arbitrary Rust code in a native service; that cooperative boundary is
part of the HostServices contract, not an implied OS sandbox for native code.

Scope each service to its extension and allowed document(s). Reject document IDs
outside that scope. Storage and secrets use the extension ID namespace and quota.
HTTP backends perform exactly one exchange, without redirects, ambient cookies or
credentials. The runtime validates exact HTTPS hostname grants and port 443 before
every hop, checks redirected destinations before sending, drops credentials on
cross-host redirects, limits hops, and enforces response/output byte budgets. Native
HTTP implementations must limit bytes while streaming, not after buffering.

`Output::Registered` and `Output::Ui` create owner-scoped native contributions.
`UiTree` is a parent-indexed tree with typed `WidgetConfig`; parent indices precede
children. Choices, table cells, numeric bounds, package asset handles and waveform
lane/channel ranges have explicit types. The runtime validates graph depth, node
IDs, widget/config matching, finite numeric bounds and draw geometry. The renderer
must treat strings as content, never executable markup; package asset IDs are not
paths or URLs. Dispatch `Invocation::Ui(UiEvent)` for enabled nodes/overlay hit IDs;
the guest routes events to its commands. `Invocation::Command` also supports typed
arguments and revision-pinned byte-range selection. Mode/import/export requests
are typed Event variants, with corresponding typed Output responses; they never
provide an arbitrary host-function execution escape hatch.

Document text ranges are **UTF-8 byte offsets**, including completions, diagnostics,
selection and edits. Convert to native character indices only after validating
UTF-8 boundaries. Timeline positions are milliseconds; audio ranges are frames in
the identified lane and channel. Snapshots include speakers, lanes, source handles,
read-only state and typed metadata. `apply_transaction` must atomically compare the
expected revision, validate every edit, and commit all edits as one undo unit or
none. The runtime's preliminary revision check does not prevent a race by itself.
The authoritative owner also rejects stale asynchronous diagnostics/completions/
decorations/overlays when consuming outputs. Validate imported/mode documents before
opening them; resolve their declared mode dependencies before enabling editing.

Diagnostics are asynchronous with respect to the UI: callbacks execute on the
worker and publish revision-stamped collections through the output queue. Long
providers should split work into bounded callbacks, retain private guest state,
report progress, check cancellation, and continue on host-scheduled `Event::Tick`.
There are no arbitrary guest threads and no linter policy in the runtime. Selection
changes require SelectionControl; playback/seek requires TransportControl (AudioRead
alone cannot start playback).

## Provider types (accounts)

A manifest may declare `providers` (`{ "id", "name", "multiple" }`) and modes may name one
in `provider`. The guest registers `ContributionKind::Provider` for each declared id
(`exaudio_sdk::register_provider`) and implements `exaudio_sdk::ProviderHandler`, routing
`Guest::on_event` through `exaudio_sdk::provider_event::<Self>(&event)`:

- `Event::ProviderSetup { provider, request_id, instance }` — the shell is opening a mode
  and needs an account. Publish UI for `contribution = provider` (the shell shows it as the
  sign-in step) and finish with `provider_setup_done(provider, request_id, id, label)` or
  `provider_setup_cancel`. `id` is stable per account and `[a-z0-9-]`
  (`exaudio_sdk::instance_id` derives one); `label` is what users see. `instance` is set when
  re-authenticating an existing account and the id must stay the same.
- `Event::ProviderStatus(instance)` — answer with `publish_status(instance, Ready |
  NeedsSignIn | Error, detail)` before returning; the shell waits one call deadline and
  treats silence as an error. `NeedsSignIn` re-runs setup for that instance.
- `Event::ProviderSignOut(instance)` — delete the account's credentials.
- `Event::ModeOpen { instance, .. }` carries the instance the user chose for the mode.

Credentials live in per-instance namespaces: `instance_secret_get/set` (grant `secrets`)
and `instance_storage_get/set` (grant `storage`). The host scopes both to instances whose
provider this extension declares — another extension holding the same grants cannot read
them — and stores them under the instance directory (`providers/<provider>/<id>/wasm/<ext>/`),
which is deleted with the instance. Ordinary `storage-*`/`secret-get` remain extension-wide.
Host side: `HostServices::instance_*` receive only pre-validated instances; the shell's
`Bridge` re-checks the declaration before touching the filesystem.

## Lifecycle, limits and hot reload

Every call receives fresh fuel, output-byte and elapsed-time budgets. Wasmtime memory,
table, instance and stack limits are per instance. A 10 ms epoch ticker interrupts
noncooperative guest execution, including cancellation; native service deadlines
remain mandatory. A VM trap, exhausted fuel or failed memory growth discards that
instance and emits `Output::Removed`, which must remove all of its contributions.
A guest-returned ordinary error preserves the active instance.
`Removed` is terminal for that runtime generation: unload, trap, and failed initial
activation stop its worker and ticker and discard queued requests. Re-enabling
requires a new Runtime. Successful replacement instead emits `Output::Activated`,
which resets owned contributions but keeps the generation live. Native queues must
reject late outputs from terminal generations.

The application must ship `exaudio-wasm-worker(.exe)` beside its actual executable
(macOS: `Contents/MacOS`). Build it with
`cargo build --release -p exaudio-wasm --bin exaudio-wasm-worker`.
Runtime never searches PATH; an absolute `Limits::compiler_path` override is only
for a trusted host installation/test harness, never extension-controlled input.
The child waits for a start gate until Windows Job Object memory limits have been
assigned; Unix installs an address-space rlimit before exec. Compilation has its
own deadline, memory and artifact-size limits. Cancellation/deadline kills and reaps
the process rather than merely timing out its caller. The temporary input is private;
artifacts arrive over the child's pipe with source SHA-256 and config receipts,
and Wasmtime verifies serialization-version/config compatibility. The runtime never
accepts arbitrary user-supplied AOT artifacts. Fuel/epoch limits apply to guest
execution, **not** to native compilation.

Host imports use lazy `WasmStr`/`WasmList` lifting, charging list storage and strings
before retaining owned values. This blocks canonical-ABI alias amplification (many
guest pointers referencing the same large string). UTF-16 decoding can transiently
allocate one string bounded by linear memory; it is charged before retention.
The fixture includes a real component that supplies aliased strings and must trap
at this allocation budget. One linear memory is allowed; small adapter/shim core
instances are separately capped.

Replacement validation checks identity, manifest/API, declared grants, open-mode
version constraints, component imports, instantiation, activation and required
`HostServices::preflight` validation by the authoritative shell registry. Activation
contributions are staged. Preflight checks the complete batch before old guest
deactivation, retaining internal registry reservations until activation/cancellation.
Mutating document/storage/transport/selection/HTTP imports
are unavailable during activation/deactivation, so a failed candidate cannot alter
the working document or installed UI. If validation/activation fails, the previous
instance and contributions remain. On success the old guest is deactivated under
limits, its contributions removed, and the replacement's staged contributions
committed. Guest memory is not migrated; use explicitly granted extension storage
for durable settings outside activation. Deactivation traps cannot prevent cleanup.

An open mode's direct and transitive requirements must be passed in
`Package::required_by_open_modes`. Incompatible replacements fail without dropping
the old instance. The shell must refuse disable/uninstall/unload while a mode needs
the extension; the runtime cannot infer documents it was not told about. Close or
migrate those documents before removing a required extension.

## Real component smoke

Build the developer-only probe component and then run the script under the same
observable verification runner:

```powershell
cargo build --manifest-path extensions/examples/hello-panel/Cargo.toml --target wasm32-unknown-unknown --release --features runtime-probes
wasm-tools component new extensions/examples/hello-panel/target/wasm32-unknown-unknown/release/exaudio_example_hello_panel.wasm -o probe-component.wasm
pwsh -File extensions/examples/hello-panel/smoke.ps1 -Component probe-component.wasm
```

The actual Wasmtime test activates the panel, publishes an empty-segment diagnostic,
dispatches a button click, observes `Hello from Exaudio! ` in the document with exactly
one undo entry, and observes updated diagnostics. It rejects invalid/incompatible
replacements while the previous guest remains callable, checks a successful reload,
denies ungranted document access, isolates explicit trap/fuel/memory probes, and
unloads the instance. Normal unit tests cover denied capabilities, exact-host and
redirect checks, stale revisions including the compare/commit race, activation
mutation denial, output limits and malformed/deep UI trees. The real component test
is explicitly ignored without the built fixture; `smoke.ps1` runs it deliberately.
The smoke script also builds the required compiler helper and exercises actual child
cancellation after launch, compiler deadline termination, memory-limit failure and
subsequent successful compilation. Reload tests inject a native preflight rejection
and assert that neither the prior instance nor its contributions are removed; queued
loads cannot revive a trapped terminal generation.
