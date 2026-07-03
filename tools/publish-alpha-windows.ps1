[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$Channel = "alpha",
    [string]$Remote = "ethernetservers",
    [string]$RemotePath = "/var/www/transcription-studio/alpha/win-x64",
    [string]$OutputDir = "",
    [switch]$SkipPackage,
    [switch]$InstallVpk
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "target\velopack\Releases"
}
$releaseDir = [System.IO.Path]::GetFullPath($OutputDir)
$repoRootFull = [System.IO.Path]::GetFullPath($repoRoot)
if (-not $releaseDir.StartsWith($repoRootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to publish from outside repo root: $releaseDir"
}

if (-not $SkipPackage) {
    $packageArgs = @(
        "-Channel", $Channel,
        "-OutputDir", $releaseDir
    )
    if (-not [string]::IsNullOrWhiteSpace($Version)) {
        $packageArgs += @("-Version", $Version)
    }
    if ($InstallVpk) {
        $packageArgs += "-InstallVpk"
    }
    & (Join-Path $PSScriptRoot "package-alpha-windows.ps1") @packageArgs
}

$feedFile = Join-Path $releaseDir "releases.$Channel.json"
$setupFile = Join-Path $releaseDir "TranscriptionStudio-$Channel-Setup.exe"
if (-not (Test-Path -LiteralPath $feedFile)) {
    throw "Missing Velopack feed: $feedFile"
}
if (-not (Test-Path -LiteralPath $setupFile)) {
    throw "Missing Velopack setup: $setupFile"
}

# Defaults to: ssh ethernetservers and /var/www/transcription-studio/alpha/win-x64.
Invoke-CheckedCommand -Command "ssh" -Arguments @($Remote, "mkdir -p '$RemotePath'")
Invoke-CheckedCommand -Command "scp" -Arguments @((Join-Path $releaseDir "*"), "${Remote}:$RemotePath/")
Invoke-CheckedCommand -Command "ssh" -Arguments @($Remote, "find '$RemotePath' -maxdepth 1 -type f -printf '%f %s bytes\n' | sort")
