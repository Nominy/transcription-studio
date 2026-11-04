# Waveform parity benchmark: Babel's browser wavesurfer vs Studio's timeline waveform.
#
#   powershell -File tools/waveform-parity/run.ps1 [-Out <dir>] [-Fixtures] [-App]
#
# Steps (each logged to <Out>/events.jsonl, status in <Out>/status.json):
#   audio     make_audio.py: case audio cut from the demo speakers (+ the Rust test fixtures' WAVs)
#   babel     render.cjs babel: Babel's own wavesurfer build + downsampler in headless Chromium
#             (screenshots, canvas outlines, display signals)
#   studio    examples/line_parity.rs: the same cases through studio-waveform
#   geometry  render.cjs studio: Studio's outlines drawn through the same wavesurfer page
#   app       (-App) app.ps1: the running Studio's lanes (PrintWindow) vs Babel at the same view
#   compare   compare.py: report.md / report.json / heatmaps
#   -Fixtures also regenerates crates/studio-waveform/tests/fixtures/babel_line/*.json.
#
# Needs: python (numpy, Pillow), node, Playwright (NODE_PATH defaults to the Babel monorepo's
# node_modules), cargo.
param(
    [string]$Out = "$PSScriptRoot/../../target/waveform-parity",
    [string]$NodeModules = 'C:/Users/User/Desktop/dev/babel/shared/babel-extension-platform/node_modules',
    [switch]$Fixtures,
    [switch]$App
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/../..").Path
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
        Step 'app-babel' { node "$PSScriptRoot/render.cjs" app $Out "$repo/crates/studio-babel/tests/fixtures/recovered/babel-fixture" }
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
