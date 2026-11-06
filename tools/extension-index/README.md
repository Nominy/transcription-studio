# Extension index tooling and marketplace backend

`exaudio-marketplace` implements schema validation, GitHub transport/cache, dependency
plans, permission consent boundaries, and a transactional WASM-only installation store.
`exaudio-extension-index` is the real validation/inspection/install CLI. Neither executes
repository source or build scripts. There is no default public catalog URL until an
actual reviewed index is published; the host supplies an explicit GitHub raw URL.

The ready-to-copy public index repository layout is `template/`. Its README documents
the complete schema, author GitHub release → reviewed index PR process, withdrawal
policy, package layout and trust boundary. Its catalog is intentionally empty, not a
fake marketplace. `init-index` copies that layout without creating a GitHub repository.

The public index entrypoint is `index.json`, matching a configured raw GitHub
`/main/index.json` URL. The CLI accepts other explicitly configured filenames, but the
template never publishes a conflicting `catalog.json` default.

## Independently cloneable author repositories

`init-author OWNER REPOSITORY EXTENSION_ID VERSION SDK_GITHUB_URL FULL_SDK_COMMIT DESTINATION`
generates a standalone extension repository from `author-template/`. It supplies working
panel/command/diagnostic Rust source, an exact manifest, MIT notices, a pinned toolchain,
and a GitHub Actions release workflow. Cargo's SDK dependency uses an explicit GitHub
`git` URL and full `rev`, never `../../../sdk/rust`. The same revision supplies the
validator CLI. All template variables are resolved before the new directory is published.

The SDK GitHub repository and full source commit are mandatory generator inputs. Supply
a real published commit containing the SDK/WIT and validator; do not invent an available
`v0.3.0` tag while publication is still pending. This command does not verify remote
availability, download dependencies, create a GitHub repository or publish a release.
The generated repository has no filesystem dependency on the Exaudio checkout.

The workflow builds a real WASM component, packages/validates it, derives byte/hash/source
index metadata, and publishes a new versioned author-repository GitHub release. Only its
publish job has write permission. It attaches the exact build lockfile/source receipt and
refuses release replacement. The author submits `index-entry.json` in a separate reviewed
PR to the index's `index.json`; no cross-repository token or automatic index approval is used.
`wasm-tools` is pinned to published version 1.239.0. Commit the generated author's Cargo.lock
before tagging; if initially absent, CI records its actual generated lockfile in the release.

## Workspace and commands

Workspace members: `crates/exaudio-marketplace`, `tools/extension-index`.
Workspace dependency: `exaudio-marketplace = { path = "crates/exaudio-marketplace" }`.
The crates declare their own semver, sha2, zip and fs2 dependencies and reuse shared
exaudio-extension, reqwest, serde, serde_json, thiserror, tempfile, tokio and url dependencies.

Run from the Exaudio repository (replace uppercase arguments with real files/identities):

```text
cargo run -p exaudio-extension-index -- validate-index tools/extension-index/template/index.json
cargo run -p exaudio-extension-index -- init-index NEW_LOCAL_INDEX_DIRECTORY
cargo run -p exaudio-extension-index -- init-author OWNER REPOSITORY EXTENSION_ID VERSION SDK_GITHUB_URL FULL_SDK_COMMIT NEW_AUTHOR_DIRECTORY
cargo run -p exaudio-extension-index -- inspect INDEX.json QUERY
cargo run -p exaudio-extension-index -- pack manifest.json COMPONENT.wasm OUTPUT.exaudio LICENSE
cargo run -p exaudio-extension-index -- validate-index PROPOSED.json PREVIOUS.json
cargo run -p exaudio-extension-index -- release-entry OWNER REPO manifest.json PACKAGE.exaudio TAG SOURCE_COMMIT
cargo run -p exaudio-extension-index -- validate-package INDEX.json ID VERSION PACKAGE.exaudio
cargo run -p exaudio-extension-index -- fetch RAW_GITHUB_URL CACHE.json
cargo run -p exaudio-extension-index -- fetch RAW_GITHUB_URL CACHE.json --offline
cargo run -p exaudio-extension-index -- plan-release INDEX.json ID VERSION STORE HOST.json GRANTS.json
cargo run -p exaudio-extension-index -- plan-mode INDEX.json ID VERSION MODE STORE HOST.json GRANTS.json
cargo run -p exaudio-extension-index -- install INDEX.json SELECTION.json STORE HOST.json GRANTS.json
cargo run -p exaudio-extension-index -- install-release RAW_GITHUB_URL CACHE.json SELECTION.json STORE HOST.json GRANTS.json
cargo run -p exaudio-extension-index -- preview-developer PACKAGE.exaudio
cargo run -p exaudio-extension-index -- install-developer INDEX.json PACKAGE.exaudio APPROVED_SHA256 DEPENDENCIES.json STORE HOST.json GRANTS.json
cargo run -p exaudio-extension-index -- disable ID STORE HOST.json
cargo run -p exaudio-extension-index -- enable INDEX.json ID STORE HOST.json
cargo run -p exaudio-extension-index -- uninstall ID STORE HOST.json
cargo run -p exaudio-extension-index -- rollback INDEX.json STORE HOST.json
```

`SELECTION.json` and `DEPENDENCIES.json` are arrays of objects with `id`, exact `version`
and a `package` local archive path. For network installation only, `package` is optional.
An empty dependency array is valid for a dependency-free developer extension.
`HOST.json` has `requirements` and `builtins`; each requirement has `required_by`,
`extension`, and a semver `version` requirement. For an isolated smoke store with no open
documents use `{"requirements":[],"builtins":[]}`. Never use empty requirements as a
substitute for a live editor's actual document/mode dependencies.

`GRANTS.json` maps extension IDs to arrays of permission objects using the manifest's
permission schema. `{}` grants nothing. Populate grants only after explicit user approval.
`APPROVED_SHA256` is the digest shown by a separate developer preview and explicitly
approved by the user; changing source bytes between preview and install is rejected.

The CLI emits timestamped JSON progress to stderr (actual download bytes, package and
extraction entry counts) and result JSON to stdout. Ctrl-C requests graceful cancellation:
network waits are interrupted, extraction checks at each entry and each 16 KiB read, and
staging is removed before returning. Once `activating` starts, the single atomic pointer
commit completes rather than leaving a half-active dependency set. A force-kill before
commit may leave unreferenced package directories but leaves the prior pointer intact.
A cancelled download restarts that transfer; this CLI does not claim resumable HTTP byte
ranges. A completed installation is durable and is not repeated on ordinary app startup.
Run long smoke/build jobs under the project's durable psmux/logging policy.

## Host integration

1. `MarketplaceClient::new(source, cache)` restricts source to HTTPS GitHub raw index
   URLs. `catalog_with_progress(offline, control)` returns validated metadata, `from_cache`
   and an explicit `refresh_error` if online refresh failed. Offline mode never contacts
   the network. Cached metadata remains bound to its exact source URL. Display that stale
   metadata cannot reflect newly published withdrawals.
2. `InstallationStore::open(private_directory)` exposes `state()`: generation, active
   and disabled inventory, and the previous whole-set snapshot. Each `Installed` has the
   exact `manifest`, compressed bytes/digest, opaque managed directory, approved permissions
   and `InstallProvenance`. Reviewed provenance retains the exact index release. Developer
   provenance retains the explicitly approved source path and must never get a reviewed badge.
   At app startup, call `validate_current_with_progress(host, limits, control)` before loading
   any persisted component: it checks enabled built-in dependencies, grants, pinned archive
   integrity and exact extracted-file equality without requiring the network. `host.builtins`
   must exclude disabled native extensions. `state()` alone is inventory, not this integrity gate.
3. `plan_mode`, `plan_release`, and `plan_developer` invoke the shared exaudio-extension
   resolver. They produce exact release selections, install/update/keep changes, required-by
   reasons, unapproved permission requests and dependency/API/active-document blockers.
   Recommended extensions are not silently installed. The host supplies built-in manifests
   and all open-document requirements; those are not inferred from marketplace inventory.
4. Collect source and capability approval. `preview_developer_package` validates a real
   local archive independently of any catalog record. Its `DeveloperInstallRequest` requires
   a separate approved digest, permission grants, exact selected reviewed dependencies and
   expected store generation. It cannot load a native library or arbitrary repository script.
5. Use `install_with_progress` or `install_developer_with_progress`. Offline/local callers
   use `install_packages_with_progress` or `install_developer_packages`. The backend checks
   all packages before atomically replacing `active.json`; every selected dependency is
   activated together. Concurrent changes are rejected by generation CAS and an exclusive
   filesystem transaction lock. Staging and immutable package directories are under
   `packages/`; permissions never come from declarations alone.
6. The host MUST compile/validate the actual WASM component ABI and register its runtime
   contributions before declaring activation successful. Package validation checks the
   component header and archive safety, not world imports or runtime semantics. Retain the
   exact prior `InstallationState` before changing the store. If host runtime/contribution
   activation fails, call `restore_failed_activation(new_generation, &prior, host)` and
   restore the prior runtime set. This operation verifies immediate-generation/snapshot
   identity; it is not the user-facing rollback path and does not introduce new grants.
7. `disable` moves inventory to disabled; `enable` revalidates/reextracts pinned retained
   bytes. `uninstall` removes managed inventory, with the same explicit document/dependency
   guards. User `rollback` restores the previous complete snapshot, rechecks current index
   withdrawals, requirements and permissions, and rebuilds extracted files from pinned
   archives. Previously approved developer provenance remains distinctly unreviewed.
8. After host activation succeeds, `collect_unreferenced(&prior)` can remove only known
   managed directories not referenced by current or previous snapshots. It never deletes
   developer source directories. A just-uninstalled package remains retained for rollback.
   Filesystem ownership/private ACLs protect this store from another local process; the
   backend is not a security boundary against an attacker who can rewrite the store itself.

`OperationControl` is cloneable. Its observer receives `Progress { stage, extension,
completed, total }`; totals are `None` where unknown. Preserve the last package-level
Preparing event while rendering inner Downloading/Extracting progress. `cancel()` works
independently of page lifetime. Attach it to app-owned workers, not a transient view.
Developer directory watching, compilation/reload, runtime logs and runtime contribution
rollback are host/WASM-shell responsibilities, not hidden marketplace build hooks.

## Verification and smoke expectations

No commands in this document imply that verification has already run. Main integration
should run `cargo nextest run -p exaudio-marketplace` and check the CLI crate once all siblings land.
Tests include traversal/absolute/drive paths, special filenames, symlinks, native payloads,
expansion limits, incorrect digests/manifests, invalid origins/schema/history, local HTTP
fetch and offline fallback, atomic multi-dependency failure, permission/source approval,
active-mode disable guards, cancellation, disabled inventory and rollback retention.
Synthetic release records and empty WASM components exist only in unit-test fixtures.

For a real runtime smoke, build the repository's actual WASM SDK example using its documented
toolchain, package `manifest.json`, `module.wasm`, and optional `assets/LICENSE`, and run
`preview-developer`. Explicitly approve its reported digest/capabilities; install into a
new chosen temporary STORE using an empty template catalog and no dependencies only if the
example has none. Inspect `active.json` for developer provenance and a complete pinned
package. Have the host load `module_path`, invoke the example command and observe its real
contribution/document effect. Then disable/uninstall with an active requirement: expect a
blocker and unchanged generation. Remove that requirement and disable/uninstall: expect
inventory change with rollback files retained. Rollback restores the prior snapshot. A
bad hash, changed manifest or cancelled extraction must never advance the active generation.

The generic `pack` command reads a real built component, writes the fixed archive layout,
fully validates it and prints the resulting source/digest preview. It refuses to overwrite
an existing output. For the real example, use its `manifest.json` and `LICENSE` under
`extensions/examples/hello-panel/`; its existing `package.ps1` is also a supported author
packaging route. Neither command substitutes an empty synthetic component for a real build.
