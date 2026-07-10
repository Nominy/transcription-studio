# Windows Alpha Release

Transcription Studio uses Velopack for Windows alpha installs and in-app updates.

Default update feed:

```text
https://reviewgen.ovh/transcription-studio/alpha/win-x64
```

Set `STUDIO_UPDATE_FEED_URL` before launching the app to point an installed build at a different feed during testing.

## Build Installer

Restore the repository-pinned Velopack CLI and package the current Cargo version:

```powershell
dotnet tool restore
.\tools\package-alpha-windows.ps1
```

The script builds `studio-app`, stages it as `TranscriptionStudio.exe`, and runs `vpk pack` on the `alpha` channel. The complete local feed is written to:

```text
target\velopack\Releases
```

A clean deployment payload is written to:

```text
target\velopack\Publish
```

`Publish` contains only the assets named by `assets.alpha.json`, the current feed metadata, and `publish-manifest.json` with each file's byte count and SHA-256. Historical packages remain in `Releases` for delta generation but are not re-uploaded.

GitHub Actions can run the same package flow from the manual `Windows Alpha Package` workflow. See `docs/release/github-actions.md`.

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
