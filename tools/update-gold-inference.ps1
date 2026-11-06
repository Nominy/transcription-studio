[CmdletBinding()]
param(
    # Clean Gold checkout at the commit to publish (babel/drafting/gold-drafting-extension, or a
    # worktree of it placed beside it so its `file:` dependencies resolve).
    [Parameter(Mandatory = $true)]
    [string]$GoldDir,
    # Run `npm run build:backend` before packaging.
    [switch]$Build,
    # SSH host serving https://reviewgen.ovh/gold-inference/ from $RemoteRoot.
    [string]$Remote = "ethernetservers",
    [string]$RemoteRoot = "/var/www/gold-inference",
    [string]$BaseUrl = "https://reviewgen.ovh/gold-inference"
)

# Publishes Gold's unpacked host inference build (`npm run build:backend` output) for Studio:
# zips it, uploads it to an immutable versioned URL, verifies the public download and rewrites
# extensions/gold/gold-inference.json, the pin LocalBackend downloads, verifies and
# installs on first use.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments)
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
    return -join ($bytes | ForEach-Object { $_.ToString("x2") })
}

# Chromium extension id: the first 128 bits of SHA-256(public key DER), one letter a-p per nibble.
function Get-ExtensionId {
    param([string]$Base64Key)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $hash = $sha256.ComputeHash([System.Convert]::FromBase64String($Base64Key))
    } finally {
        $sha256.Dispose()
    }
    $id = foreach ($byte in $hash[0..15]) {
        [char]([int][char]'a' + ($byte -shr 4))
        [char]([int][char]'a' + ($byte -band 15))
    }
    return -join $id
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$goldRoot = (Resolve-Path -LiteralPath $GoldDir).Path

# Content changes against HEAD (stat-only differences from line-ending conversion don't count).
function Get-GoldChanges {
    $changed = @(& git -C $goldRoot diff --name-only HEAD)
    $untracked = @(& git -C $goldRoot ls-files --others --exclude-standard)
    return @($changed + $untracked | Where-Object { $_ })
}

$dirty = @(Get-GoldChanges)
if ($dirty.Count -gt 0) {
    throw "Gold checkout has uncommitted changes; publish only committed builds (use a clean worktree):`n$($dirty -join "`n")"
}
$commit = (& git -C $goldRoot rev-parse --short=12 HEAD).Trim()

if ($Build) {
    Push-Location -LiteralPath $goldRoot
    try {
        # npm.cmd: the npm.ps1 shim fails under StrictMode in Windows PowerShell.
        Invoke-Checked -Command "npm.cmd" -Arguments @("run", "build:backend")
    } finally {
        Pop-Location
    }
    # The build regenerates tracked identity files (source hashes, model sha256) from the checked
    # out bytes; a difference means Gold committed them stale. The artifact is still reproducible
    # from this commit's checkout, but Gold should commit the regenerated files.
    $changed = @(Get-GoldChanges)
    if ($changed.Count -gt 0) {
        Write-Warning "Building regenerated tracked Gold files that differ from commit ${commit}:`n$($changed -join "`n")"
    }
}

$source = Join-Path $goldRoot ".artifacts/inference-backend/extension"
foreach ($required in @("manifest.json", "inference.html", "inference.js")) {
    if (-not (Test-Path -LiteralPath (Join-Path $source $required) -PathType Leaf)) {
        throw "Gold host build is incomplete ($required missing) at $source. Run with -Build."
    }
}
$manifest = Get-Content -Raw -LiteralPath (Join-Path $source "manifest.json") | ConvertFrom-Json
if (-not ($manifest.PSObject.Properties.Name -contains "key")) {
    throw "Gold host manifest has no fixed key; the extension id (and its model cache) would depend on the install path."
}
$version = [string]$manifest.version
$release = "$version-$commit"

$zip = Join-Path ([System.IO.Path]::GetTempPath()) "gold-inference-$release.zip"
if (Test-Path -LiteralPath $zip) {
    Remove-Item -LiteralPath $zip -Force
}
$entries = @(Get-ChildItem -LiteralPath $source | ForEach-Object { $_.Name })
# bsdtar writes portable `/`-separated entry names.
Invoke-Checked -Command (Join-Path $env:SystemRoot "System32/tar.exe") -Arguments (@("-a", "-c", "-f", $zip, "-C", $source) + $entries)

$sha256 = Get-Sha256Hex -Path $zip
$bytes = (Get-Item -LiteralPath $zip).Length
$remoteDir = "$RemoteRoot/$release"
$remoteFile = "$remoteDir/gold-inference.zip"
$url = "$BaseUrl/$release/gold-inference.zip"

# Published releases are immutable: an existing upload must be byte-identical.
$existing = "$(& ssh $Remote "test -f '$remoteFile' && sha256sum '$remoteFile' | cut -d' ' -f1 || true")".Trim()
if ($existing -and $existing -ne $sha256) {
    throw "$url already exists with a different SHA-256 ($existing); bump the Gold commit instead of overwriting."
}
if (-not $existing) {
    Invoke-Checked -Command "ssh" -Arguments @($Remote, "mkdir -p '$remoteDir'")
    Invoke-Checked -Command "scp" -Arguments @("-q", $zip, "${Remote}:$remoteFile.part")
    Invoke-Checked -Command "ssh" -Arguments @($Remote, "chmod 644 '$remoteFile.part' && mv '$remoteFile.part' '$remoteFile'")
}

$check = Join-Path ([System.IO.Path]::GetTempPath()) "gold-inference-$release.check.zip"
Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $check
$publicSha256 = Get-Sha256Hex -Path $check
Remove-Item -LiteralPath $check -Force
if ($publicSha256 -ne $sha256) {
    throw "$url serves SHA-256 $publicSha256, expected $sha256."
}

$pin = [ordered]@{
    version = $version
    commit = $commit
    url = $url
    sha256 = $sha256
    bytes = $bytes
    extensionId = Get-ExtensionId -Base64Key ([string]$manifest.key)
}
# One `"key": value` per line (Windows PowerShell's ConvertTo-Json pads with odd spacing).
$fields = foreach ($entry in $pin.GetEnumerator()) {
    '  "{0}": {1}' -f $entry.Key, (ConvertTo-Json -Compress -InputObject $entry.Value)
}
[System.IO.File]::WriteAllText(
    (Join-Path $repoRoot "extensions/gold/gold-inference.json"),
    "{`n" + ($fields -join ",`n") + "`n}`n",
    [System.Text.UTF8Encoding]::new($false)
)
Remove-Item -LiteralPath $zip -Force
Write-Host "Published Gold $release ($bytes bytes, sha256 $sha256, extension $($pin.extensionId)) at $url and pinned it in extensions/gold/gold-inference.json."
