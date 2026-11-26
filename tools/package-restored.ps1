param(
    [string]$BuildDirectory = 'target\debug',
    [string]$OutputDirectory = 'dist\restored-preview'
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$buildRoot = Join-Path $projectRoot $BuildDirectory
$packageRoot = Join-Path $projectRoot $OutputDirectory
$artifacts = @{
    'transcription-studio.exe' = 'TranscriptionStudio.exe'
    'studio-babel-service.exe' = 'TranscriptionStudio.BabelService.exe'
    'studio-relay.exe' = 'studio-relay.exe'
}
foreach ($name in $artifacts.Keys) {
    if (-not (Test-Path -LiteralPath (Join-Path $buildRoot $name) -PathType Leaf)) {
        throw "Build artifact is missing: $name"
    }
}
New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
foreach ($name in $artifacts.Keys) {
    Copy-Item -LiteralPath (Join-Path $buildRoot $name) -Destination (Join-Path $packageRoot $artifacts[$name])
}
Copy-Item -LiteralPath (Join-Path $projectRoot 'RESTORATION.txt') -Destination $packageRoot
Get-ChildItem -LiteralPath $packageRoot -File | Where-Object Name -ne 'SHA256.json' | Get-FileHash -Algorithm SHA256 |
    Select-Object @{Name='File';Expression={Split-Path $_.Path -Leaf}},Hash |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot 'SHA256.json') -Encoding UTF8
Write-Output "Restored preview packaged at $packageRoot. No installed files were changed."
