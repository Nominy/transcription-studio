<#
.SYNOPSIS
Publishes the Windows alpha payload (target/velopack/Publish) to the VPS alpha feed.

.DESCRIPTION
Uploads into an incoming directory, verifies every file against publish-manifest.json, then
promotes packages before feed metadata under a remote lock, keeping the previous metadata
for rollback. Packages first (tools/package-windows.ps1 -UpdateChannel alpha) unless
-SkipPackage. -KnownHostsPath pins the server's host key (CI); without it, the user's own
known_hosts applies with accept-new.
#>
[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$Remote = "ethernetservers",
    [string]$RemotePath = "/var/www/exaudio/alpha/win-x64",
    [string]$RemoteIncomingRoot = "/var/tmp/exaudio-alpha-incoming",
    [string]$OutputDir = "",
    [string]$PublishDir = "",
    [string]$SshKeyPath = "",
    [string]$KnownHostsPath = "",
    [int]$SshPort = 22,
    [switch]$SkipPackage
)

$Channel = "alpha"

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

function Get-Sha256Hex {
    param([string]$Path)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $stream = [System.IO.File]::OpenRead($Path)
        try {
            $bytes = $sha256.ComputeHash($stream)
        } finally {
            $stream.Dispose()
        }
    } finally {
        $sha256.Dispose()
    }
    return -join ($bytes | ForEach-Object { $_.ToString("X2") })
}

function Assert-SafeRemotePath {
    param(
        [string]$Name,
        [string]$Path
    )

    if (
        $Path -notmatch '^/[A-Za-z0-9._/-]+$' -or
        $Path -match '(^|/)\.\.(/|$)' -or
        $Path -match '//'
    ) {
        throw "$Name must be an absolute remote path without traversal or shell metacharacters: $Path"
    }
}

function Assert-NonOverlappingRemotePaths {
    param(
        [string]$FirstName,
        [string]$First,
        [string]$SecondName,
        [string]$Second
    )

    $firstPath = $First.TrimEnd("/")
    $secondPath = $Second.TrimEnd("/")
    if (
        $firstPath.Equals($secondPath, [System.StringComparison]::Ordinal) -or
        $firstPath.StartsWith("$secondPath/", [System.StringComparison]::Ordinal) -or
        $secondPath.StartsWith("$firstPath/", [System.StringComparison]::Ordinal)
    ) {
        throw "$FirstName and $SecondName must not overlap: '$firstPath' and '$secondPath'"
    }
}

function Assert-RepoChildPath {
    param(
        [string]$RepoRoot,
        [string]$Path
    )

    $rootFull = [System.IO.Path]::GetFullPath($RepoRoot).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    $pathFull = [System.IO.Path]::GetFullPath($Path)
    $rootPrefix = "$rootFull$([System.IO.Path]::DirectorySeparatorChar)"
    if (-not $pathFull.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to publish from outside repo root: $Path"
    }
}

function Get-ValidatedPublishManifest {
    param(
        [string]$PublishPath,
        [string]$ExpectedChannel,
        [string]$ExpectedVersion
    )

    $manifestPath = Join-Path $PublishPath "publish-manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "Missing publish manifest: $manifestPath"
    }

    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    if ([string]$manifest.Channel -cne $ExpectedChannel) {
        throw "Publish manifest channel '$($manifest.Channel)' does not match '$ExpectedChannel'."
    }
    if (
        -not [string]::IsNullOrWhiteSpace($ExpectedVersion) -and
        [string]$manifest.Version -cne $ExpectedVersion
    ) {
        throw "Publish manifest version '$($manifest.Version)' does not match '$ExpectedVersion'."
    }
    if ([string]::IsNullOrWhiteSpace([string]$manifest.Version)) {
        throw "Publish manifest has no version."
    }

    $publishFull = [System.IO.Path]::GetFullPath($PublishPath)
    $seen = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    $assetCount = 0
    $metadata = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::Ordinal
    )
    foreach ($file in @($manifest.Files)) {
        $relativePath = [string]$file.RelativeFileName
        if (
            [string]::IsNullOrWhiteSpace($relativePath) -or
            [System.IO.Path]::IsPathRooted($relativePath) -or
            $relativePath.Contains("\") -or
            $relativePath.Contains("`n") -or
            $relativePath.Contains("`r")
        ) {
            throw "Publish manifest contains an unsafe relative path: $relativePath"
        }

        $candidate = [System.IO.Path]::GetFullPath((Join-Path $publishFull $relativePath))
        $publishPrefix = $publishFull.TrimEnd(
            [System.IO.Path]::DirectorySeparatorChar,
            [System.IO.Path]::AltDirectorySeparatorChar
        ) + [System.IO.Path]::DirectorySeparatorChar
        if (-not $candidate.StartsWith($publishPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Publish manifest path escapes PublishDir: $relativePath"
        }
        if (-not $seen.Add($relativePath)) {
            throw "Publish manifest contains a duplicate path: $relativePath"
        }
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            throw "Publish manifest references a missing file: $relativePath"
        }

        $item = Get-Item -LiteralPath $candidate
        if ($item.Length -ne [long]$file.Size) {
            throw "Publish file size mismatch for $relativePath"
        }
        $actualHash = Get-Sha256Hex -Path $candidate
        if (-not $actualHash.Equals([string]$file.SHA256, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Publish file SHA-256 mismatch for $relativePath"
        }

        switch ([string]$file.Kind) {
            "asset" { $assetCount += 1 }
            "metadata" { [void]$metadata.Add($relativePath) }
            default { throw "Publish manifest has an unknown file kind for $relativePath" }
        }
    }

    if ($assetCount -eq 0) {
        throw "Publish manifest contains no package or installer assets."
    }
    foreach ($metadataName in @(
        "assets.$ExpectedChannel.json",
        "RELEASES-$ExpectedChannel",
        "releases.$ExpectedChannel.json"
    )) {
        if (-not $metadata.Contains($metadataName)) {
            throw "Publish manifest is missing required feed metadata: $metadataName"
        }
    }
    if ($metadata.Count -ne 3) {
        throw "Publish manifest contains unexpected feed metadata."
    }

    $expectedFiles = [System.Collections.Generic.HashSet[string]]::new(
        $seen,
        [System.StringComparer]::OrdinalIgnoreCase
    )
    [void]$expectedFiles.Add("publish-manifest.json")
    $publishPrefix = $publishFull.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
    foreach ($actualFile in Get-ChildItem -LiteralPath $publishFull -File -Recurse) {
        $relative = $actualFile.FullName.Substring($publishPrefix.Length).Replace("\", "/")
        if (-not $expectedFiles.Remove($relative)) {
            throw "PublishDir contains a file not listed in its manifest: $relative"
        }
    }
    if ($expectedFiles.Count -ne 0) {
        throw "PublishDir is missing one or more manifest files."
    }

    return $manifest
}

if ($SshPort -lt 1 -or $SshPort -gt 65535) {
    throw "SshPort must be between 1 and 65535."
}
Assert-SafeRemotePath -Name "RemotePath" -Path $RemotePath
Assert-SafeRemotePath -Name "RemoteIncomingRoot" -Path $RemoteIncomingRoot
Assert-NonOverlappingRemotePaths `
    -FirstName "RemotePath" `
    -First $RemotePath `
    -SecondName "RemoteIncomingRoot" `
    -Second $RemoteIncomingRoot

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "target/velopack/Releases"
}
if ([string]::IsNullOrWhiteSpace($PublishDir)) {
    $PublishDir = Join-Path $repoRoot "target/velopack/Publish"
}
$releaseDir = [System.IO.Path]::GetFullPath($OutputDir)
$publishPath = [System.IO.Path]::GetFullPath($PublishDir)
$repoRootFull = [System.IO.Path]::GetFullPath($repoRoot)
Assert-RepoChildPath -RepoRoot $repoRootFull -Path $releaseDir
Assert-RepoChildPath -RepoRoot $repoRootFull -Path $publishPath

if (-not $SkipPackage) {
    $packageArgs = @{
        UpdateChannel = "alpha"
        OutputDir = $releaseDir
        PublishDir = $publishPath
    }
    if (-not [string]::IsNullOrWhiteSpace($Version)) {
        $packageArgs.Version = $Version
    }
    & (Join-Path $PSScriptRoot "package-windows.ps1") @packageArgs
}

$manifest = Get-ValidatedPublishManifest `
    -PublishPath $publishPath `
    -ExpectedChannel $Channel `
    -ExpectedVersion $Version
$Version = [string]$manifest.Version
if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z.+-]*$') {
    throw "Publish manifest version is not safe for a deployment directory: $Version"
}

$manifestHash = (Get-Sha256Hex -Path (Join-Path $publishPath "publish-manifest.json")).ToLowerInvariant()
$deploymentVersion = $Version.Replace("+", "_")
$deploymentId = "$deploymentVersion-$($manifestHash.Substring(0, 16))"
$remotePlatformRoot = "$($RemoteIncomingRoot.TrimEnd('/'))/win-x64"
$remoteDeployment = "$remotePlatformRoot/deployments/$deploymentId"
$remotePayload = "$remoteDeployment/payload"
$remoteRollbackRoot = "$remotePlatformRoot/rollbacks"
$remoteLock = "$remotePlatformRoot/publish.lock"
foreach ($path in @($remotePlatformRoot, $remoteDeployment, $remotePayload, $remoteRollbackRoot, $remoteLock)) {
    Assert-SafeRemotePath -Name "generated remote path" -Path $path
}

$hostKeyArgs = if ([string]::IsNullOrWhiteSpace($KnownHostsPath)) {
    @("-o", "StrictHostKeyChecking=accept-new")
} else {
    $knownHostsFull = (Resolve-Path -LiteralPath $KnownHostsPath).Path
    @("-o", "StrictHostKeyChecking=yes", "-o", "UserKnownHostsFile=$knownHostsFull")
}
$sshArgs = @("-p", [string]$SshPort) + $hostKeyArgs
$scpArgs = @("-P", [string]$SshPort) + $hostKeyArgs
if (-not [string]::IsNullOrWhiteSpace($SshKeyPath)) {
    $keyFull = (Resolve-Path -LiteralPath $SshKeyPath).Path
    $sshArgs = @("-i", $keyFull) + $sshArgs
    $scpArgs = @("-i", $keyFull) + $scpArgs
}

Invoke-CheckedCommand -Command "ssh" -Arguments ($sshArgs + @(
    $Remote,
    "rm -rf '$remoteDeployment' && mkdir -p '$remotePayload'"
))
$uploadEntries = @(Get-ChildItem -LiteralPath $publishPath | ForEach-Object { $_.FullName })
if ($uploadEntries.Count -eq 0) {
    throw "PublishDir is empty: $publishPath"
}
Invoke-CheckedCommand -Command "scp" -Arguments ($scpArgs + @("-r") + $uploadEntries + @(
    "${Remote}:$remotePayload/"
))

$remoteScript = @'
set -eu
incoming=$1
live=$2
rollback_root=$3
lock_path=$4
channel=$5
deployment_id=$6
manifest="$incoming/publish-manifest.json"

verify_manifest() {
    root=$1
    kind=${2:-all}
    python3 - "$root" "$manifest" "$kind" <<'PY'
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1]).resolve()
manifest_path = pathlib.Path(sys.argv[2])
kind = sys.argv[3]
with manifest_path.open("r", encoding="utf-8-sig") as handle:
    manifest = json.load(handle)

for entry in manifest["Files"]:
    if kind != "all" and entry["Kind"] != kind:
        continue
    relative = entry["RelativeFileName"]
    pure = pathlib.PurePosixPath(relative)
    if (
        not relative
        or pure.is_absolute()
        or any(part in ("", ".", "..") for part in pure.parts)
        or "\\" in relative
        or "\n" in relative
        or "\r" in relative
    ):
        raise SystemExit(f"unsafe publish path: {relative!r}")
    path = (root / pathlib.Path(*pure.parts)).resolve()
    try:
        path.relative_to(root)
    except ValueError as error:
        raise SystemExit(f"publish path escapes root: {relative}") from error
    if not path.is_file():
        raise SystemExit(f"missing publish file: {relative}")
    if path.stat().st_size != int(entry["Size"]):
        raise SystemExit(f"publish size mismatch: {relative}")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest.casefold() != str(entry["SHA256"]).casefold():
        raise SystemExit(f"publish SHA-256 mismatch: {relative}")
PY
}

list_kind() {
    python3 - "$manifest" "$1" <<'PY'
import json
import sys
with open(sys.argv[1], "r", encoding="utf-8-sig") as handle:
    manifest = json.load(handle)
for entry in manifest["Files"]:
    if entry["Kind"] == sys.argv[2]:
        print(entry["RelativeFileName"])
PY
}

promote_file() {
    relative=$1
    source="$incoming/$relative"
    destination="$live/$relative"
    destination_parent=$(dirname "$destination")
    mkdir -p "$destination_parent"
    if [ -f "$destination" ] && cmp -s "$source" "$destination"; then
        return
    fi
    temporary="$destination.incoming-$deployment_id"
    cp -p "$source" "$temporary"
    mv -f "$temporary" "$destination"
}

verify_manifest "$incoming" all
mkdir -p "$live" "$rollback_root" "$(dirname "$lock_path")"
exec 9>"$lock_path"
flock 9

# Versioned packages are immutable: old feed hashes and rollback metadata depend on them.
list_kind asset | while IFS= read -r relative; do
    case "$relative" in
        *.nupkg)
            if [ -f "$live/$relative" ] && ! cmp -s "$incoming/$relative" "$live/$relative"; then
                printf 'Refusing to replace existing versioned package with different bytes: %s\n' "$relative" >&2
                exit 1
            fi
            ;;
    esac
done

rollback="$rollback_root/$(date -u +%Y%m%dT%H%M%SZ)-$deployment_id"
mkdir -p "$rollback"
for relative in "assets.$channel.json" "RELEASES-$channel" "releases.$channel.json"; do
    if [ -f "$live/$relative" ]; then
        cp -p "$live/$relative" "$rollback/$relative"
    fi
done

list_kind asset | while IFS= read -r relative; do
    promote_file "$relative"
done
verify_manifest "$live" asset

# The primary update feed is promoted last, after every referenced package exists.
promote_file "assets.$channel.json"
promote_file "RELEASES-$channel"
promote_file "releases.$channel.json"
verify_manifest "$live" metadata
rm -rf "$incoming"
printf 'Published %s to %s; previous metadata: %s\n' "$deployment_id" "$live" "$rollback"
'@
$remoteScriptBytes = [System.Text.Encoding]::UTF8.GetBytes($remoteScript.Replace("`r`n", "`n"))
$encodedRemoteScript = [Convert]::ToBase64String($remoteScriptBytes)
$remoteCommand = "printf '%s' '$encodedRemoteScript' | base64 -d | sh -s -- '$remotePayload' '$RemotePath' '$remoteRollbackRoot' '$remoteLock' '$Channel' '$deploymentId'"
Invoke-CheckedCommand -Command "ssh" -Arguments ($sshArgs + @($Remote, $remoteCommand))
Invoke-CheckedCommand -Command "ssh" -Arguments ($sshArgs + @(
    $Remote,
    "find '$RemotePath' -maxdepth 1 -type f -printf '%f %s bytes\n' | sort"
))
