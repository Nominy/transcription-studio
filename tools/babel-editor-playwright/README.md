# Babel editor day-to-day Playwright contract

This harness exercises the recovered Babel browser editor for the normal
transcription workflow and shares its behavior contract with the Rust/Iced app
tests.

## Run

```powershell
npm install
npx playwright install chromium
npm test
```

By default the suite starts Babel from:

```text
C:/Users/User/Desktop/dev/babel/tools/babel-editor-rebuild/app
```

Override that when needed:

```powershell
$env:BABEL_EDITOR_APP = "C:/path/to/babel-editor/app"
npm test
```

Or point the tests at an already running editor:

```powershell
$env:BABEL_EDITOR_URL = "http://127.0.0.1:53213/"
npm test
```

The shared contract lives at:

```text
crates/studio-app/tests/fixtures/babel-day-to-day-contract.json
```

The native Rust/Iced parity check is:

```powershell
cargo test -p studio-app native_editor_matches_babel_day_to_day_contract -- --nocapture
```
