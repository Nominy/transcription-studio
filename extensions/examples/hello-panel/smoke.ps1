# Run this script in the project's detached, observable psmux verification runner.
# Cargo compilation is the restart boundary; its incremental cache is preserved.
# The test itself is offline, makes no paid requests, and writes only fixture state.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$Component)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$env:EXAUDIO_TEST_COMPONENT = (Resolve-Path -LiteralPath $Component).Path
Push-Location $root
try {
    & cargo build -p exaudio-wasm --bin exaudio-wasm-worker
    if ($LASTEXITCODE -ne 0) { throw "Required compiler helper build failed: $LASTEXITCODE" }
    & cargo nextest run -p exaudio-wasm --run-ignored only -E "test(compiler_process_)"
    if ($LASTEXITCODE -ne 0) { throw "Compiler process isolation tests failed: $LASTEXITCODE" }
    & cargo nextest run -p exaudio-wasm --run-ignored only -E "test(real_component_activation_ui_transactions_diagnostics_reload_and_isolation)"
    if ($LASTEXITCODE -ne 0) { throw "Real component lifecycle smoke failed: $LASTEXITCODE" }
} finally {
    Pop-Location
    Remove-Item Env:EXAUDIO_TEST_COMPONENT -ErrorAction SilentlyContinue
}
