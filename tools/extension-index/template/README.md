# Exaudio extension release index

This repository is a reviewed catalog, not an extension source monorepo. `index.json`
is the published schema-1 index. Each entry belongs to one author's GitHub repository;
each immutable release points at an actual `.exaudio` GitHub release asset. Start with
this empty index and add only published, reviewed releases. There are no example releases.

## Repository layout and review policy

- `index.json`: the single validated, append-only release index.
- `README.md`: this contribution and trust policy; adapt the maintainer contact and
  review process before publishing your index.
- Protect the default branch and require maintainer review of pull requests. Configure
  your GitHub repository settings; the local template does not create those settings.
- CI should run the trusted Exaudio `exaudio-extension-index validate-index` executable on
  both the proposed catalog and the previous default-branch catalog. Pin the validator
  source commit or a verified tool release in your own CI. Never execute a contributor's
  build scripts or untrusted CI changes with privileged credentials.

## Author repository and package layout

An author's repository contains their source, license, `manifest.json`, and their own
build instructions. Build and test the extension as a WASM component matching the
Exaudio extension host ABI. This index does not build or execute submitted repository source.
See the Exaudio repository's WASM SDK/examples for the actual host ABI.

Publish a ZIP archive with a `.exaudio` filename containing exactly:

```
manifest.json
module.wasm
assets/              (optional, inert data only)
```

No enclosing top-level directory, native libraries, executables, symlinks, special
files, absolute/drive paths, `..`, or Windows-ambiguous filenames. Scripts with executable
suffixes are rejected. Default limits are 32 MiB compressed, 128 MiB expanded, 4096
entries and a 256 KiB manifest. The module must be a WASM component; runtime compilation
and ABI validation are the host's responsibility before executing it.

`manifest.json` uses the exact `exaudio-extension::ExtensionManifest` JSON schema: `id`,
`name`, semver `version`, API semver requirement `api`, `runtime: "wasm"`, `description`,
`license`, `dependencies` (ID to version requirement), `permissions` (capability objects),
and `modes` (ID/name/requires/recommends). Unknown fields are rejected. IDs are dotted
lowercase identifiers. Capability declarations request access; they do not grant it.

## Publish a release and submit an index PR

1. Commit the author's source and manifest. Record the full Git source commit ID.
2. Build locally using the author's toolchain. Package the real WASM component and exact
   manifest into `.exaudio`. The trusted CLI can do this with
   `exaudio-extension-index pack manifest.json COMPONENT.wasm OUTPUT.exaudio LICENSE`.
   It validates the archive and refuses to overwrite existing artifacts. Do not include
   native binaries or build scripts.
3. Create a versioned GitHub release in the author's repository and upload the archive.
   Use a path-safe tag and asset filename (ASCII letters/digits/dot/underscore/hyphen).
4. Generate a real entry from those exact bytes with the trusted validator:

   ```text
   exaudio-extension-index release-entry OWNER REPOSITORY manifest.json PACKAGE.exaudio TAG SOURCE_COMMIT
   ```

   It validates the package and prints an entry; it does not upload anything or prove
   the release exists. The author must publish the release first.
5. Add that entry to `extensions`, or append the release to the existing entry. Submit
   an index PR. Never rewrite the manifest, tag, commit, URL, byte count or checksum of
   a published indexed version. Publish a new version for changes.
6. Maintainers verify repository control by the submitting author, license, source/tag
   correspondence, exact release bytes, permissions, ABI/runtime compatibility, and
   malicious behavior independently. Run `validate-index` against the previous index,
   then download and run `validate-package` on the actual asset. Automated structural
   validation is not a substitute for source/provenance review.

## Catalog schema 1

The root has `schema: 1` and `extensions: []`. Every extension entry contains:

| Field | Meaning |
| --- | --- |
| `id` | Stable ID matching every contained release manifest |
| `publisher` | GitHub account name, exactly equal to `repository.owner` |
| `repository` | Object with `owner` and `name`; one repository per extension |
| `keywords` | Optional string list for search |
| `releases` | Versioned release records |

Every release record contains:

| Field | Meaning |
| --- | --- |
| `version` | Semver, exactly matching `manifest.version` |
| `tag` | Actual GitHub release tag |
| `source_commit` | Full lowercase 40- or 64-hex Git object ID |
| `manifest` | Complete, immutable `ExtensionManifest`, not a summary |
| `asset.name` | `.exaudio` archive filename |
| `asset.url` | Exact `https://github.com/OWNER/REPOSITORY/releases/download/TAG/NAME` |
| `asset.bytes` | Positive exact compressed byte count |
| `asset.sha256` | Lowercase 64-hex SHA256 of archive bytes |
| `withdrawn` | Optional null or nonempty withdrawal reason |

Duplicate IDs/versions and shared repositories are invalid. Newer host-API requirements
are valid catalog metadata but incompatible packages cannot be activated by older hosts.
To withdraw a release, retain the full immutable record and set `withdrawn` to a reason.
Do not delete history. A withdrawal blocks new installation and rollback to that release;
it does not secretly delete already installed code or invalidate open documents.

## Trust boundary

The configured GitHub index and its maintainers are trusted to review provenance. A
SHA256 checksum binds the downloaded bytes to the reviewed index; **it is not a publisher
signature or proof of authorship**. Repository ownership, tag-to-commit mapping, and
source-to-binary correspondence require review; the schema cannot authenticate them.
GitHub release tags/assets can be replaced upstream; the immutable hash detects changed
bytes but cannot make GitHub releases cryptographically immutable.

Only HTTPS GitHub release URLs are fetched. Redirects are restricted to the actual
GitHub release-asset CDN host/path combinations, never arbitrary hosts, intranet addresses,
or custom ports. No repository source, installer hook, native DLL, or build script is
executed by the marketplace. WASM host permissions must still be enforced on every call.

Offline use reads the last validated cache for the same source URL. Offline clients cannot
know about subsequent withdrawals; the host must show cache/freshness status. An invalid
refresh does not replace a valid cache and is surfaced as a refresh error.
