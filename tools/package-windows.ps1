<#
.SYNOPSIS
Builds Exaudio for Windows and packs it with Velopack.

.DESCRIPTION
-UpdateChannel stable packs Velopack channel `win` (GitHub Releases: Exaudio-win-Setup.exe,
Exaudio-win-Portable.zip); -UpdateChannel alpha packs channel `alpha` (VPS alpha feed). The
channel and version are baked into the binary (STUDIO_UPDATE_CHANNEL, STUDIO_BUILD_VERSION),
so the installed app updates from the feed it was packaged for.

Outputs: the full local feed in -OutputDir (default target/velopack/Releases) and, for
publication, the current release's indexed assets + feed metadata + publish-manifest.json in
-PublishDir (default target/velopack/Publish).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet("stable", "alpha")]
    [string]$UpdateChannel,
    [string]$Version = "",
    [string]$ReleaseNotes = "",
    [string]$StageDir = "",
    [string]$OutputDir = "",
    [string]$PublishDir = "",
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$PackId = "Exaudio"
$PackTitle = "Exaudio"
$PackAuthors = "Nominy"
$MainExe = "Exaudio.exe"
$Channel = if ($UpdateChannel -eq "stable") { "win" } else { "alpha" }

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
    $parentFull = [System.IO.Path]::GetFullPath($Parent).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    $childFull = [System.IO.Path]::GetFullPath($Child)
    $parentPrefix = "$parentFull$([System.IO.Path]::DirectorySeparatorChar)"
    if (-not $childFull.StartsWith($parentPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to operate outside repo root: $childFull"
    }
}

function Assert-NonOverlappingDirectories {
    param(
        [string]$FirstName,
        [string]$First,
        [string]$SecondName,
        [string]$Second
    )

    $firstFull = [System.IO.Path]::GetFullPath($First).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    $secondFull = [System.IO.Path]::GetFullPath($Second).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    $firstPrefix = "$firstFull$([System.IO.Path]::DirectorySeparatorChar)"
    $secondPrefix = "$secondFull$([System.IO.Path]::DirectorySeparatorChar)"
    if (
        $firstFull.Equals($secondFull, [System.StringComparison]::OrdinalIgnoreCase) -or
        $firstFull.StartsWith($secondPrefix, [System.StringComparison]::OrdinalIgnoreCase) -or
        $secondFull.StartsWith($firstPrefix, [System.StringComparison]::OrdinalIgnoreCase)
    ) {
        throw "$FirstName and $SecondName must not overlap: '$firstFull' and '$secondFull'"
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

function Get-ValidatedRelativeFile {
    param(
        [string]$Root,
        [string]$RelativePath
    )

    if ([string]::IsNullOrWhiteSpace($RelativePath)) {
        throw "Publish manifest contains an empty relative path."
    }
    if (
        [System.IO.Path]::IsPathRooted($RelativePath) -or
        $RelativePath.IndexOf([char]0) -ge 0
    ) {
        throw "Publish manifest path must be relative: $RelativePath"
    }

    $rootFull = [System.IO.Path]::GetFullPath($Root)
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $rootFull $RelativePath))
    Assert-ChildPath -Parent $rootFull -Child $candidate
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "Publish manifest references a missing file: $RelativePath"
    }

    $rootPrefix = $rootFull.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
    return $candidate.Substring($rootPrefix.Length).Replace("\", "/")
}

function New-PublishDirectory {
    param(
        [string]$OutputPath,
        [string]$PublishPath,
        [string]$Channel,
        [string]$Version
    )

    $outputFull = [System.IO.Path]::GetFullPath($OutputPath)
    $publishFull = [System.IO.Path]::GetFullPath($PublishPath)

    $assetIndexName = "assets.$Channel.json"
    $assetIndexPath = Join-Path $outputFull $assetIndexName
    if (-not (Test-Path -LiteralPath $assetIndexPath -PathType Leaf)) {
        throw "Missing Velopack asset index: $assetIndexPath"
    }

    $assetIndex = Get-Content -Raw -LiteralPath $assetIndexPath | ConvertFrom-Json
    $files = [System.Collections.Generic.List[object]]::new()
    $seen = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    foreach ($asset in @($assetIndex)) {
        $relativePath = Get-ValidatedRelativeFile `
            -Root $outputFull `
            -RelativePath ([string]$asset.RelativeFileName)
        if (-not $seen.Add($relativePath)) {
            throw "Velopack asset index contains a duplicate path: $relativePath"
        }
        $files.Add([pscustomobject]@{
            RelativeFileName = $relativePath
            Kind = "asset"
        })
    }
    if ($files.Count -eq 0) {
        throw "Velopack asset index contains no current release assets: $assetIndexPath"
    }

    $legacyIndexName = if ($Channel -eq "win") { "RELEASES" } else { "RELEASES-$Channel" }
    foreach ($metadataName in @(
        $assetIndexName,
        $legacyIndexName,
        "releases.$Channel.json"
    )) {
        $relativePath = Get-ValidatedRelativeFile -Root $outputFull -RelativePath $metadataName
        if (-not $seen.Add($relativePath)) {
            throw "Publish file list contains a duplicate path: $relativePath"
        }
        $files.Add([pscustomobject]@{
            RelativeFileName = $relativePath
            Kind = "metadata"
        })
    }

    if (Test-Path -LiteralPath $publishFull) {
        Remove-Item -LiteralPath $publishFull -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $publishFull | Out-Null

    $manifestFiles = [System.Collections.Generic.List[object]]::new()
    foreach ($file in $files) {
        $source = Join-Path $outputFull $file.RelativeFileName
        $destination = Join-Path $publishFull $file.RelativeFileName
        $destinationParent = Split-Path -Parent $destination
        New-Item -ItemType Directory -Force -Path $destinationParent | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination

        $copied = Get-Item -LiteralPath $destination
        $manifestFiles.Add([ordered]@{
            RelativeFileName = $file.RelativeFileName
            Kind = $file.Kind
            Size = $copied.Length
            SHA256 = Get-Sha256Hex -Path $destination
        })
    }

    $publishManifest = [ordered]@{
        Version = $Version
        Channel = $Channel
        Files = $manifestFiles
    }
    $manifestPath = Join-Path $publishFull "publish-manifest.json"
    $manifestJson = $publishManifest | ConvertTo-Json -Depth 4
    [System.IO.File]::WriteAllText(
        $manifestPath,
        $manifestJson,
        [System.Text.UTF8Encoding]::new($false)
    )

    $publishBytes = (
        Get-ChildItem -LiteralPath $publishFull -File -Recurse |
            Measure-Object -Property Length -Sum
    ).Sum
    Write-Host "PublishDir contains $($manifestFiles.Count) indexed files plus its manifest ($publishBytes bytes): $publishFull"
}

$repoRoot = Resolve-RepoPath (Join-Path $PSScriptRoot "..")
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = Get-StudioAppVersion -RepoRoot $repoRoot
}
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$') {
    throw "Version must be SemVer (major.minor.patch[-prerelease]): $Version"
}
if ([string]::IsNullOrWhiteSpace($StageDir)) {
    $StageDir = Join-Path $repoRoot "target/velopack/stage"
}
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "target/velopack/Releases"
}
if ([string]::IsNullOrWhiteSpace($PublishDir)) {
    $PublishDir = Join-Path $repoRoot "target/velopack/Publish"
}

$stagePath = [System.IO.Path]::GetFullPath($StageDir)
$outputPath = [System.IO.Path]::GetFullPath($OutputDir)
$publishPath = [System.IO.Path]::GetFullPath($PublishDir)
# Cargo's own CARGO_TARGET_DIR, when set, decides where the release binaries land.
$cargoTargetDir = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    Join-Path $repoRoot "target"
} else {
    $env:CARGO_TARGET_DIR
}
$sourceExePath = Join-Path $cargoTargetDir "release/studio-app.exe"
$sourceRelayExePath = Join-Path $cargoTargetDir "release/studio-relay.exe"
# Isolated compiler child for untrusted extension .wasm; the app resolves it as its sibling.
$sourceWasmWorkerPath = Join-Path $cargoTargetDir "release/exaudio-wasm-worker.exe"
$iconPath = Join-Path $repoRoot "assets/app/exaudio.ico"
Assert-ChildPath -Parent $repoRoot -Child $stagePath
Assert-ChildPath -Parent $repoRoot -Child $outputPath
Assert-ChildPath -Parent $repoRoot -Child $publishPath
Assert-NonOverlappingDirectories -FirstName "StageDir" -First $stagePath -SecondName "OutputDir" -Second $outputPath
Assert-NonOverlappingDirectories -FirstName "StageDir" -First $stagePath -SecondName "PublishDir" -Second $publishPath
Assert-NonOverlappingDirectories -FirstName "OutputDir" -First $outputPath -SecondName "PublishDir" -Second $publishPath
if (-not (Test-Path -LiteralPath $iconPath -PathType Leaf)) {
    throw "Missing app icon: $iconPath"
}
if (-not [string]::IsNullOrWhiteSpace($ReleaseNotes) -and -not (Test-Path -LiteralPath $ReleaseNotes -PathType Leaf)) {
    throw "Release notes file does not exist: $ReleaseNotes"
}
if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
    throw "The repo-local vpk tool requires .NET SDK 8. Run: dotnet tool restore"
}

if (-not $SkipBuild) {
    # Baked into the binary: the version it reports and the update channel it follows.
    $env:STUDIO_BUILD_VERSION = $Version
    $env:STUDIO_UPDATE_CHANNEL = $UpdateChannel
    # One resolution keeps shared Wasmtime dependencies from being rebuilt with a second feature set.
    Invoke-CheckedCommand -Command "cargo" -Arguments @("build", "-p", "studio-app", "-p", "studio-relay", "-p", "exaudio-wasm", "--release", "--locked")
}

foreach ($binary in @($sourceExePath, $sourceRelayExePath, $sourceWasmWorkerPath)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "Missing release executable: $binary"
    }
}

if (Test-Path -LiteralPath $stagePath) {
    Remove-Item -LiteralPath $stagePath -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stagePath | Out-Null
New-Item -ItemType Directory -Force -Path $outputPath | Out-Null
Copy-Item -LiteralPath $sourceExePath -Destination (Join-Path $stagePath $MainExe)
Copy-Item -LiteralPath $sourceRelayExePath -Destination (Join-Path $stagePath "studio-relay.exe")
Copy-Item -LiteralPath $sourceWasmWorkerPath -Destination (Join-Path $stagePath "exaudio-wasm-worker.exe")
$demoSource = Join-Path $repoRoot "crates/studio-editor/assets/demo"
$demoDestination = Join-Path $stagePath "assets/demo"
New-Item -ItemType Directory -Force -Path $demoDestination | Out-Null
foreach ($file in @("speaker_1.wav", "speaker_2.wav")) {
    Copy-Item -LiteralPath (Join-Path $demoSource $file) -Destination (Join-Path $demoDestination $file)
}
foreach ($file in @("README.md", "LICENSE")) {
    $source = Join-Path $repoRoot $file
    if (Test-Path -LiteralPath $source) {
        Copy-Item -LiteralPath $source -Destination $stagePath
    }
}

# vpk pack creates the installer, portable bundle, nupkg packages and feed metadata.
$packArguments = @(
    "pack",
    "--packId", $PackId,
    "--packVersion", $Version,
    "--packDir", $stagePath,
    "--mainExe", $MainExe,
    "--channel", $Channel,
    "--outputDir", $outputPath,
    "--packTitle", $PackTitle,
    "--packAuthors", $PackAuthors,
    "--icon", $iconPath,
    "--shortcuts", "Desktop,StartMenuRoot"
)
if (-not [string]::IsNullOrWhiteSpace($ReleaseNotes)) {
    $packArguments += @("--releaseNotes", (Resolve-Path -LiteralPath $ReleaseNotes).Path)
}
Push-Location -LiteralPath $repoRoot
try {
    Invoke-CheckedCommand -Command "dotnet" -Arguments (@("tool", "run", "vpk", "--") + $packArguments)
} finally {
    Pop-Location
}

foreach ($asset in @("$PackId-$Channel-Setup.exe", "$PackId-$Channel-Portable.zip")) {
    if (-not (Test-Path -LiteralPath (Join-Path $outputPath $asset) -PathType Leaf)) {
        throw "vpk pack did not produce $asset"
    }
}

New-PublishDirectory `
    -OutputPath $outputPath `
    -PublishPath $publishPath `
    -Channel $Channel `
    -Version $Version

Write-Host "Velopack $UpdateChannel artifacts (channel $Channel):"
Get-ChildItem -LiteralPath $publishPath |
    Sort-Object Name |
    Select-Object Name, Length, LastWriteTime |
    Format-Table -AutoSize
