[CmdletBinding()]
param(
    # Copy the pinned bundle from the bundle host into the build cache and unpack it into the
    # stage, instead of publishing the stage.
    [switch]$Fetch,
    # Unpacked bundle (default target/babel-data): what -Fetch fills,
    # tools/lint-parity/generate.mjs regenerates and tools/waveform-parity/run.ps1 reads.
    [string]$Stage,
    # SSH host keeping the bundles in $RemoteRoot (root-only, not served over HTTP).
    [string]$Remote = "ethernetservers",
    [string]$RemoteRoot = "/root/sonus-data-private"
)

# Parity fixtures Exaudio can't carry in its public source ship as a private tar bundle pinned by
# SHA-256 in tools/babel-data.json: the lint parity corpus with Babel's expected issues
# (lint-parity.json, read by the babel-lint parity tests, see extensions/babel-lint/src/data.rs)
# and the copied Babel waveform code tools/waveform-parity/run.ps1 drives. Builds never need the
# bundle: the linter rule data lives in extensions/babel-lint/assets. The bundle is not
# redistributable, so it is never published over HTTP.
#
#   tools/update-babel-data.ps1 -Fetch      bundle host -> build cache, unpacked into the stage
#   node tools/lint-parity/generate.mjs     regenerate the corpus in the stage (optional)
#   tools/update-babel-data.ps1             stage -> bundle host + build cache, and pin it

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Every bundle carries the corpus; the waveform-parity files are optional.
$Required = @("lint-parity.json")
$Optional = @(
    "waveform-parity/display-signal.js",
    "waveform-parity/wavesurfer.js"
)

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments)
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

function Get-Sha256Hex {
    param([string]$Path)
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

# Per-user cache of fetched bundles.
function Get-BuildCache {
    if ($env:LOCALAPPDATA) {
        return Join-Path $env:LOCALAPPDATA "Exaudio/build-cache"
    }
    $cache = if ($env:XDG_CACHE_HOME) { $env:XDG_CACHE_HOME } else { Join-Path $HOME ".cache" }
    return Join-Path $cache "exaudio/build-cache"
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$pinPath = Join-Path $repoRoot "tools/babel-data.json"
$tar = Join-Path $env:SystemRoot "System32/tar.exe"
if (-not $Stage) {
    $Stage = Join-Path $repoRoot "target/babel-data"
}
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
$Stage = (Resolve-Path -LiteralPath $Stage).Path
$cache = Get-BuildCache
New-Item -ItemType Directory -Force -Path $cache | Out-Null

if ($Fetch) {
    $pin = Get-Content -Raw -LiteralPath $pinPath | ConvertFrom-Json
    $name = "babel-data-$($pin.sha256).tar"
    $cached = Join-Path $cache $name
    if (-not ((Test-Path -LiteralPath $cached) -and (Get-Sha256Hex -Path $cached) -eq $pin.sha256)) {
        $part = "$cached.$PID.part"
        Invoke-Checked -Command "scp" -Arguments @("-q", "${Remote}:$RemoteRoot/$name", $part)
        $sha256 = Get-Sha256Hex -Path $part
        if ($sha256 -ne $pin.sha256) {
            Remove-Item -LiteralPath $part -Force
            throw "${Remote}:$RemoteRoot/$name has SHA-256 $sha256, expected $($pin.sha256)."
        }
        Move-Item -Force -LiteralPath $part -Destination $cached
    }
    Get-ChildItem -LiteralPath $Stage | Remove-Item -Recurse -Force
    Invoke-Checked -Command $tar -Arguments @("-x", "-f", $cached, "-C", $Stage)
    # Bundles published before the rule data moved into extensions/babel-lint/assets also carry
    # the in-tree files; keep only what the stage contract allows.
    Get-ChildItem -LiteralPath $Stage -Recurse -File | Where-Object {
        $name = $_.FullName.Substring($Stage.Length + 1).Replace('\', '/')
        ($Required -notcontains $name) -and ($Optional -notcontains $name)
    } | ForEach-Object {
        Write-Host "Dropping $($_.FullName): tracked in the repository now."
        Remove-Item -LiteralPath $_.FullName -Force
    }
    Write-Host "Babel data $($pin.release) is in the build cache ($cached) and unpacked into $Stage."
    return
}

$present = @(Get-ChildItem -LiteralPath $Stage -Recurse -File | ForEach-Object {
    $_.FullName.Substring($Stage.Length + 1).Replace('\', '/')
} | Sort-Object)
$missing = @($Required | Where-Object { $present -notcontains $_ })
$extra = @($present | Where-Object { ($Required -notcontains $_) -and ($Optional -notcontains $_) })
if ($missing.Count -gt 0 -or $extra.Count -gt 0) {
    throw "Stage $Stage must hold $($Required -join ', ') and at most $($Optional -join ', '). Missing: $($missing -join ', '); unexpected: $($extra -join ', ')."
}

$archive = Join-Path ([System.IO.Path]::GetTempPath()) "babel-data-$([guid]::NewGuid()).tar"
# ustar: plain headers every tar reader understands; `/`-separated names.
Invoke-Checked -Command $tar -Arguments (@("--format", "ustar", "-c", "-f", $archive, "-C", $Stage) + $present)

$sha256 = Get-Sha256Hex -Path $archive
$bytes = (Get-Item -LiteralPath $archive).Length
$release = "$((Get-Date).ToUniversalTime().ToString('yyyyMMdd'))-$($sha256.Substring(0, 12))"
$name = "babel-data-$sha256.tar"
$remoteFile = "$RemoteRoot/$name"

Invoke-Checked -Command "ssh" -Arguments @($Remote, "mkdir -p '$RemoteRoot' && chmod 700 '$RemoteRoot'")
Invoke-Checked -Command "scp" -Arguments @("-q", $archive, "${Remote}:$remoteFile.part")
Invoke-Checked -Command "ssh" -Arguments @($Remote, "chmod 600 '$remoteFile.part' && mv '$remoteFile.part' '$remoteFile'")
$stored = "$(& ssh $Remote "sha256sum '$remoteFile' | cut -d' ' -f1")".Trim()
if ($stored -ne $sha256) {
    throw "${Remote}:$remoteFile has SHA-256 $stored, expected $sha256."
}
Move-Item -Force -LiteralPath $archive -Destination (Join-Path $cache $name)

$pin = [ordered]@{
    release = $release
    sha256 = $sha256
    bytes = $bytes
}
# One `"key": value` per line (Windows PowerShell's ConvertTo-Json pads with odd spacing).
$fields = foreach ($entry in $pin.GetEnumerator()) {
    '  "{0}": {1}' -f $entry.Key, (ConvertTo-Json -Compress -InputObject $entry.Value)
}
[System.IO.File]::WriteAllText($pinPath, "{`n" + ($fields -join ",`n") + "`n}`n", [System.Text.UTF8Encoding]::new($false))
Write-Host "Stored Babel data $release ($bytes bytes, sha256 $sha256) at ${Remote}:$remoteFile and in $cache, and pinned it in tools/babel-data.json."
