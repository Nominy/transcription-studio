# Windows Alpha Release

Transcription Studio uses Velopack for Windows alpha installs and in-app updates.

Default update feed:

```text
https://reviewgen.ovh/transcription-studio/alpha/win-x64
```

Set `STUDIO_UPDATE_FEED_URL` before launching the app to point an installed build at a different feed during testing.

## Build Installer

Restore the repository-pinned Velopack CLI and package locally:

```powershell
dotnet tool restore
.\tools\package-alpha-windows.ps1
```

Without `-Version`, the local script uses the Cargo package version. A
non-Cargo version is accepted only when `STUDIO_BUILD_VERSION` exactly matches
it, ensuring the compiled application's displayed build identity matches the
Velopack package identity.

The script builds `studio-app`, stages it as `TranscriptionStudio.exe`, and runs `vpk pack` on the `alpha` channel. The complete local feed is written to:

```text
target\velopack\Releases
```

A clean deployment payload is written to:

```text
target\velopack\Publish
```

`Publish` contains only the assets named by `assets.alpha.json`, the current feed metadata, and `publish-manifest.json` with each file's byte count and SHA-256. Historical packages remain in `Releases` for delta generation but are not re-uploaded.

GitHub Actions runs entirely on a self-hosted Linux runner, no `windows-latest`: `cargo xwin build --release --target x86_64-pc-windows-msvc` cross-compiles both binaries (no Windows host needed), then `package-alpha-windows.ps1 -Target x86_64-pc-windows-msvc -VpkCommand tools/vpk-wine-wrapper.sh` runs `vpk pack` through a genuine Windows-native `dotnet.exe` executed via Wine -- `vpk pack` packages for whichever OS build of `dotnet` is actually hosting it, not an arbitrary `--runtime` target, so the Linux-native `dotnet` alone cannot produce a Windows release. See `docs/release/github-actions.md`.

## Automatic GitHub Release

Every push to `main` triggers the Windows alpha workflow on the self-hosted
homeserver and must publish to the VPS. There is one release per push event:
when one push contains multiple commits, the release is built from that push's
checked-out head rather than once per commit. Manual dispatch is also available
and may set `publish_to_vps: false`.

Unless a manual dispatch supplies an explicit version, the workflow increments
the Cargo patch version and produces
`<major>.<minor>.<next-patch>-alpha.<run_number>.<run_attempt>`. The resolved
value is exported as both `VERSION` and `STUDIO_BUILD_VERSION`, so the compiled
application and packaged release have the same identity.

Runs are serialized by the `velopack-windows-alpha` concurrency group and an
in-progress run is not cancelled. Publishing requires the repository secrets
`ALPHA_SSH_HOST`, `ALPHA_SSH_USER`, and `ALPHA_SSH_PRIVATE_KEY`;
`ALPHA_SSH_PORT` is optional. Because publication is mandatory for pushes, an
automatic push run fails at the publish step if any required secret is missing.
The feed remains
`https://reviewgen.ovh/transcription-studio/alpha/win-x64`, deployed at
`/var/www/transcription-studio/alpha/win-x64`.

macOS alpha packaging is optional and manual through its separate
`workflow_dispatch` workflow; a Windows automatic release does not trigger it.

## Publish To VPS

```powershell
.\tools\publish-alpha-windows.ps1
```

The publisher uploads only `target\velopack\Publish` to an unserved incoming directory. The VPS validates the manifest under a deployment lock, promotes package and installer assets first, and replaces `releases.alpha.json` last. Previous feed metadata is retained for rollback, and old versioned packages remain available.

The default destination remains:

```text
/var/www/transcription-studio/alpha/win-x64
```

## Tester Flow

Send testers:

```text
TranscriptionStudio-alpha-Setup.exe
```

After installation, the dashboard Updates section shows the current version, update feed, and buttons for `Check for updates` and `Install update`.
