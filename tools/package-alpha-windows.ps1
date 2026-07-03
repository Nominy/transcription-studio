[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$Channel = "alpha",
    [string]$PackId = "TranscriptionStudio",
    [string]$PackTitle = "Transcription Studio",
    [string]$StageDir = "",
    [string]$OutputDir = "",
    [switch]$SkipBuild,
    [switch]$InstallVpk
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RepoPath {
    param([string]$Path)
    return (Resolve-Path -LiteralPath $Path).Path
}

function Get-StudioAppVersion {
    param([string]$RepoRoot)
    $manifestPath = Join-Path $RepoRoot "crates\studio-app\Cargo.toml"
    $manifest = Get-Content -Raw -LiteralPath $manifestPath
    if ($manifest -match '(?m)^version\s*=\s*"([^"]+)"') {
        return $Matches[1]
    }
    throw "Could not read studio-app package version from $manifestPath"
}

function Assert-ChildPath {
    param(
        [string]$Parent,
        [string]$Child
    )
    $parentFull = [System.IO.Path]::GetFullPath($Parent)
    $childFull = [System.IO.Path]::GetFullPath($Child)
    if (-not $childFull.StartsWith($parentFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to operate outside repo root: $childFull"
    }
}

function Invoke-CheckedCommand {
    param(
        [string]$Command,
        [string[]]$Arguments
    )

    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

$repoRoot = Resolve-RepoPath (Join-Path $PSScriptRoot "..")
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = Get-StudioAppVersion -RepoRoot $repoRoot
}
if ([string]::IsNullOrWhiteSpace($StageDir)) {
    $StageDir = Join-Path $repoRoot "target\velopack\stage"
}
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "target\velopack\Releases"
}

$stagePath = [System.IO.Path]::GetFullPath($StageDir)
$outputPath = [System.IO.Path]::GetFullPath($OutputDir)
Assert-ChildPath -Parent $repoRoot -Child $stagePath
Assert-ChildPath -Parent $repoRoot -Child $outputPath

if ($InstallVpk -or -not (Get-Command vpk -ErrorAction SilentlyContinue)) {
    if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
        throw "vpk is missing and dotnet is not available. Install .NET SDK 8, then run: dotnet tool update -g vpk"
    }
    Invoke-CheckedCommand -Command "dotnet" -Arguments @("tool", "update", "-g", "vpk")
}
if (-not (Get-Command vpk -ErrorAction SilentlyContinue)) {
    throw "vpk is missing. Run this script with -InstallVpk or install it with: dotnet tool update -g vpk"
}

if (-not $SkipBuild) {
    Invoke-CheckedCommand -Command "cargo" -Arguments @("build", "-p", "studio-app", "--release")
}

if (Test-Path -LiteralPath $stagePath) {
    Remove-Item -LiteralPath $stagePath -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stagePath | Out-Null
New-Item -ItemType Directory -Force -Path $outputPath | Out-Null

$sourceExe = Join-Path $repoRoot "target\release\studio-app.exe"
if (-not (Test-Path -LiteralPath $sourceExe)) {
    throw "Missing release executable: $sourceExe"
}
Copy-Item -LiteralPath $sourceExe -Destination (Join-Path $stagePath "TranscriptionStudio.exe")

foreach ($file in @("README.md", "Cargo.lock")) {
    $source = Join-Path $repoRoot $file
    if (Test-Path -LiteralPath $source) {
        Copy-Item -LiteralPath $source -Destination $stagePath
    }
}

# vpk pack creates TranscriptionStudio-Setup.exe, nupkg packages, RELEASES, and releases.alpha.json.
Invoke-CheckedCommand -Command "vpk" -Arguments @(
    "pack",
    "--packId", $PackId,
    "--packVersion", $Version,
    "--packDir", $stagePath,
    "--mainExe", "TranscriptionStudio.exe",
    "--channel", $Channel,
    "--outputDir", $outputPath,
    "--packTitle", $PackTitle,
    "--packAuthors", "Nominy",
    "--shortcuts", "Desktop,StartMenuRoot"
)

Write-Host "Velopack alpha artifacts:"
Get-ChildItem -LiteralPath $outputPath |
    Sort-Object Name |
    Select-Object Name, Length, LastWriteTime |
    Format-Table -AutoSize
