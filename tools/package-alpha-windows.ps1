[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$Channel = "alpha",
    [string]$PackId = "TranscriptionStudio",
    [string]$PackTitle = "Transcription Studio",
    [string]$StageDir = "",
    [string]$OutputDir = "",
    [string]$PublishDir = "",
    [string]$SourceExe = "",
    [string]$SourceRelayExe = "",
    [string]$VpkCommand = "",
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

function Invoke-Vpk {
    param(
        [string]$RepoRoot,
        [string]$Command,
        [string[]]$Arguments
    )

    if (-not [string]::IsNullOrWhiteSpace($Command)) {
        Invoke-CheckedCommand -Command $Command -Arguments $Arguments
        return
    }

    Push-Location -LiteralPath $RepoRoot
    try {
        $dotnetArguments = @("tool", "run", "vpk", "--") + $Arguments
        Invoke-CheckedCommand -Command "dotnet" -Arguments $dotnetArguments
    } finally {
        Pop-Location
    }
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

    if ($Channel -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') {
        throw "Channel is not safe for release metadata file names: $Channel"
    }

    $outputFull = [System.IO.Path]::GetFullPath($OutputPath)
    $publishFull = [System.IO.Path]::GetFullPath($PublishPath)
    if ($outputFull -eq $publishFull) {
        throw "PublishDir must differ from OutputDir."
    }

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

    foreach ($metadataName in @(
        $assetIndexName,
        "RELEASES-$Channel",
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
$cargoVersion = Get-StudioAppVersion -RepoRoot $repoRoot
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = $cargoVersion
} elseif (-not $Version.Equals($cargoVersion, [System.StringComparison]::Ordinal)) {
    throw "Version override '$Version' does not match studio-app Cargo version '$cargoVersion'."
}
if ([string]::IsNullOrWhiteSpace($StageDir)) {
    $StageDir = Join-Path $repoRoot "target\velopack\stage"
}
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "target\velopack\Releases"
}
if ([string]::IsNullOrWhiteSpace($PublishDir)) {
    $PublishDir = Join-Path $repoRoot "target\velopack\Publish"
}
if ([string]::IsNullOrWhiteSpace($SourceExe)) {
    $SourceExe = Join-Path $repoRoot "target\release\studio-app.exe"
}
if ([string]::IsNullOrWhiteSpace($SourceRelayExe)) {
    $SourceRelayExe = Join-Path $repoRoot "target\release\studio-relay.exe"
}

$stagePath = [System.IO.Path]::GetFullPath($StageDir)
$outputPath = [System.IO.Path]::GetFullPath($OutputDir)
$publishPath = [System.IO.Path]::GetFullPath($PublishDir)
$sourceExePath = [System.IO.Path]::GetFullPath($SourceExe)
$sourceRelayExePath = [System.IO.Path]::GetFullPath($SourceRelayExe)
Assert-ChildPath -Parent $repoRoot -Child $stagePath
Assert-ChildPath -Parent $repoRoot -Child $outputPath
Assert-ChildPath -Parent $repoRoot -Child $publishPath
Assert-ChildPath -Parent $repoRoot -Child $sourceExePath
Assert-ChildPath -Parent $repoRoot -Child $sourceRelayExePath
Assert-NonOverlappingDirectories -FirstName "StageDir" -First $stagePath -SecondName "OutputDir" -Second $outputPath
Assert-NonOverlappingDirectories -FirstName "StageDir" -First $stagePath -SecondName "PublishDir" -Second $publishPath
Assert-NonOverlappingDirectories -FirstName "OutputDir" -First $outputPath -SecondName "PublishDir" -Second $publishPath

if ([string]::IsNullOrWhiteSpace($VpkCommand)) {
    if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
        throw "The repo-local vpk tool requires .NET SDK 8. Run: dotnet tool restore"
    }
    if ($InstallVpk) {
        Push-Location -LiteralPath $repoRoot
        try {
            Invoke-CheckedCommand -Command "dotnet" -Arguments @("tool", "restore")
        } finally {
            Pop-Location
        }
    }
} elseif (-not (Get-Command $VpkCommand -ErrorAction SilentlyContinue)) {
    throw "vpk command does not exist: $VpkCommand"
}

if (-not $SkipBuild) {
    Invoke-CheckedCommand -Command "cargo" -Arguments @("build", "-p", "studio-app", "-p", "studio-relay", "--release", "--locked")
}

if (Test-Path -LiteralPath $stagePath) {
    Remove-Item -LiteralPath $stagePath -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stagePath | Out-Null
New-Item -ItemType Directory -Force -Path $outputPath | Out-Null

if (-not (Test-Path -LiteralPath $sourceExePath -PathType Leaf)) {
    throw "Missing release executable: $sourceExePath"
}
Copy-Item -LiteralPath $sourceExePath -Destination (Join-Path $stagePath "TranscriptionStudio.exe")
if (-not (Test-Path -LiteralPath $sourceRelayExePath -PathType Leaf)) {
    throw "Missing relay executable: $sourceRelayExePath"
}
Copy-Item -LiteralPath $sourceRelayExePath -Destination (Join-Path $stagePath "studio-relay.exe")

foreach ($file in @("README.md", "Cargo.lock")) {
    $source = Join-Path $repoRoot $file
    if (Test-Path -LiteralPath $source) {
        Copy-Item -LiteralPath $source -Destination $stagePath
    }
}

# vpk pack creates the current installer/portable assets, nupkg packages, and feed metadata.
Invoke-Vpk -RepoRoot $repoRoot -Command $VpkCommand -Arguments @(
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

New-PublishDirectory `
    -OutputPath $outputPath `
    -PublishPath $publishPath `
    -Channel $Channel `
    -Version $Version

Write-Host "Velopack alpha artifacts:"
Get-ChildItem -LiteralPath $publishPath |
    Sort-Object Name |
    Select-Object Name, Length, LastWriteTime |
    Format-Table -AutoSize
