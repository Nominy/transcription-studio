[CmdletBinding()]
param(
    [string]$GitleaksVersion = "8.30.1",
    [switch]$SkipHistory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RepoPath {
    return (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
}

function Get-Gitleaks {
    param([string]$Version)

    $existing = Get-Command gitleaks -ErrorAction SilentlyContinue
    if ($existing) {
        return $existing.Source
    }

    $toolRoot = Join-Path $env:TEMP "transcription-studio-gitleaks-$Version"
    $exe = Join-Path $toolRoot "gitleaks.exe"
    if (Test-Path -LiteralPath $exe) {
        return $exe
    }

    New-Item -ItemType Directory -Force -Path $toolRoot | Out-Null
    $zip = Join-Path $toolRoot "gitleaks.zip"
    $uri = "https://github.com/gitleaks/gitleaks/releases/download/v$Version/gitleaks_$Version" + "_windows_x64.zip"
    Invoke-WebRequest -Uri $uri -OutFile $zip
    Expand-Archive -LiteralPath $zip -DestinationPath $toolRoot -Force
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Gitleaks executable was not found after extracting $zip"
    }
    return $exe
}

function Copy-CommittableTree {
    param(
        [string]$RepoRoot,
        [string]$Destination
    )

    if (Test-Path -LiteralPath $Destination) {
        Remove-Item -LiteralPath $Destination -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null

    $files = & git -C $RepoRoot ls-files --cached --others --exclude-standard
    if ($LASTEXITCODE -ne 0) {
        throw "git ls-files failed"
    }

    foreach ($relative in $files) {
        if ([string]::IsNullOrWhiteSpace($relative)) {
            continue
        }
        $source = Join-Path $RepoRoot $relative
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            continue
        }
        $target = Join-Path $Destination $relative
        $targetDir = Split-Path -Parent $target
        if (-not [string]::IsNullOrWhiteSpace($targetDir)) {
            New-Item -ItemType Directory -Force -Path $targetDir | Out-Null
        }
        Copy-Item -LiteralPath $source -Destination $target
    }
}

function Invoke-Gitleaks {
    param(
        [string]$Gitleaks,
        [string[]]$Arguments,
        [string]$FailureMessage
    )

    & $Gitleaks @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw $FailureMessage
    }
}

$repoRoot = Resolve-RepoPath
$gitleaks = Get-Gitleaks -Version $GitleaksVersion
& $gitleaks version

$scanRoot = Join-Path $env:TEMP "transcription-studio-committable-secret-scan"
Copy-CommittableTree -RepoRoot $repoRoot -Destination $scanRoot

Invoke-Gitleaks `
    -Gitleaks $gitleaks `
    -Arguments @("detect", "--source", $scanRoot, "--no-git", "--redact", "--verbose") `
    -FailureMessage "Gitleaks found a secret in files that would be committed."

if (-not $SkipHistory) {
    Invoke-Gitleaks `
        -Gitleaks $gitleaks `
        -Arguments @("detect", "--source", $repoRoot, "--redact", "--verbose") `
        -FailureMessage "Gitleaks found a secret in git history."
}

Write-Host "Secret scan completed with no findings."
