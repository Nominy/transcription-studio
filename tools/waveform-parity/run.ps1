# Waveform parity benchmark: Babel's browser waveform vs Studio's timeline waveform.
#
#   powershell -File tools/waveform-parity/run.ps1 [-BabelDisplaySignal <file>] [-Out <dir>] [-Fixtures] [-App]
#
# Steps (each logged to <Out>/events.jsonl, status in <Out>/status.json):
#   audio     make_audio.py: case audio cut from the synthetic demo speakers (+ the Rust test
#             fixtures' WAVs)
#   babel     render.cjs babel: wavesurfer (upstream 7.12.1) + Babel's downsampler in headless
#             Chromium (screenshots, canvas outlines, display signals)
#   studio    examples/line_parity.rs: the same cases through studio-waveform
#   geometry  render.cjs studio: Studio's outlines drawn through the same wavesurfer page
#   app       (-App) app.ps1: the running Studio's lanes (PrintWindow) vs Babel at the same view
#   compare   compare.py: report.md / report.json / heatmaps
#   -Fixtures also regenerates crates/studio-waveform/tests/fixtures/babel_line/*.json.
#
# Needs: python (numpy, Pillow), node, Playwright (NODE_PATH defaults to the node_modules of the
# Babel monorepo beside this checkout), cargo, and Babel's proprietary display-signal downsampler
# (the editor-chunk excerpt defining window.BabelDisplaySignal), which is never committed or
# published: -BabelDisplaySignal names a local copy, defaulting to the one the Babel data bundle
# unpacks into target/babel-data/waveform-parity (`tools/update-babel-data.ps1 -Fetch`).
param(
    [string]$BabelDisplaySignal,
    [string]$Out,
    [string]$NodeModules,
    [switch]$Fixtures,
    [switch]$App
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/../..").Path
if (-not $Out) { $Out = Join-Path $repo 'target/waveform-parity' }
if (-not $NodeModules) { $NodeModules = Join-Path $repo '../babel/shared/babel-extension-platform/node_modules' }
if (-not $BabelDisplaySignal) { $BabelDisplaySignal = Join-Path $repo 'target/babel-data/waveform-parity/display-signal.js' }
if (-not (Test-Path -LiteralPath $BabelDisplaySignal)) {
    throw "No Babel display-signal downsampler at $BabelDisplaySignal; pass -BabelDisplaySignal or fetch the bundle with tools/update-babel-data.ps1 -Fetch."
}
$env:BABEL_DISPLAY_SIGNAL = (Resolve-Path -LiteralPath $BabelDisplaySignal).Path
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$env:NODE_PATH = $NodeModules
$events = Join-Path $Out 'events.jsonl'

function Status($state, $step) {
    @{ state = $state; step = $step; at = (Get-Date -Format o) } | ConvertTo-Json -Compress |
        Set-Content -Encoding utf8 (Join-Path $Out 'status.json')
}

function Step($name, [scriptblock]$body) {
    $started = Get-Date
    Status 'running' $name
    @{ event = 'start'; step = $name; at = (Get-Date -Format o) } | ConvertTo-Json -Compress |
        Add-Content -Encoding utf8 $events
    Write-Host "== $name"
    & $body
    if ($LASTEXITCODE -ne 0) {
        @{ event = 'failed'; step = $name; exit = $LASTEXITCODE; at = (Get-Date -Format o) } |
            ConvertTo-Json -Compress | Add-Content -Encoding utf8 $events
        Status 'failed' $name
        throw "$name failed ($LASTEXITCODE)"
    }
    $seconds = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
    @{ event = 'done'; step = $name; seconds = $seconds; at = (Get-Date -Format o) } |
        ConvertTo-Json -Compress | Add-Content -Encoding utf8 $events
    Write-Host "   $name done in $seconds s"
}

Push-Location $repo
try {
    Step 'audio' { python "$PSScriptRoot/make_audio.py" "$Out/audio" }
    if ($Fixtures) {
        Step 'fixtures' { node "$PSScriptRoot/render.cjs" fixtures "$repo/crates/studio-waveform/tests/fixtures/babel_line" }
    }
    Step 'babel' { node "$PSScriptRoot/render.cjs" babel "$PSScriptRoot/cases.json" "$Out/audio" $Out }
    Step 'studio' { cargo run --release -q -p studio-waveform --example line_parity -- $Out }
    Step 'geometry' { node "$PSScriptRoot/render.cjs" studio $Out }
    if ($App) {
        Step 'app-capture' { powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot/app.ps1" -Out $Out -Mode shots }
        Step 'app-prepare' { python "$PSScriptRoot/app_compare.py" prepare $Out }
        Step 'app-babel' { node "$PSScriptRoot/render.cjs" app $Out "$repo/crates/studio-editor/assets/demo" }
        Step 'app-compare' { python "$PSScriptRoot/app_compare.py" compare $Out }
    }
    Step 'compare' { python "$PSScriptRoot/compare.py" $Out }
    Status 'completed' 'compare'
    @{ event = 'completed'; at = (Get-Date -Format o) } | ConvertTo-Json -Compress |
        Add-Content -Encoding utf8 $events
    Write-Host "report: $Out/report.md"
} finally {
    Pop-Location
}
