# Windows Alpha Release

Transcription Studio uses Velopack for Windows alpha installs and in-app updates.

Default update feed:

```text
https://reviewgen.ovh/transcription-studio/alpha/win-x64
```

Set `STUDIO_UPDATE_FEED_URL` before launching the app to point an installed build at a different feed during testing.

## Build Installer

```powershell
.\tools\package-alpha-windows.ps1 -InstallVpk
```

The script builds `studio-app`, stages it as `TranscriptionStudio.exe`, and runs `vpk pack` on the `alpha` channel. Artifacts are written to:

```text
target\velopack\Releases
```

Expected files include `TranscriptionStudio-alpha-Setup.exe`, `.nupkg` packages, `RELEASES-alpha`, and `releases.alpha.json`.

GitHub Actions can run the same package flow from the manual `Windows Alpha Package` workflow. See `docs/release/github-actions.md`.

## Publish To VPS

```powershell
.\tools\publish-alpha-windows.ps1 -InstallVpk
```

The publish script uploads the release directory over `ssh ethernetservers` / `scp` to:

```text
/var/www/transcription-studio/alpha/win-x64
```

Keep old files in that directory. Velopack uses the existing release feed and package history to generate and serve delta updates.

## Tester Flow

Send testers:

```text
TranscriptionStudio-alpha-Setup.exe
```

After installation, the dashboard Updates section shows the current version, update feed, and buttons for `Check for updates` and `Install update`.
