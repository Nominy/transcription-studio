[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Component,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'dist')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$componentPath = (Resolve-Path -LiteralPath $Component).Path
$manifestPath = Join-Path $PSScriptRoot 'manifest.json'
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
if ($manifest.id -ne 'example.hello-panel' -or $manifest.runtime -ne 'wasm') { throw 'Unexpected manifest identity/runtime' }
$bytes = [IO.File]::ReadAllBytes($componentPath)
if ($bytes.Length -gt 16777216 -or $bytes.Length -lt 8) { throw 'Component must be between 8 bytes and 16 MiB' }
$header = ($bytes[0..7] | ForEach-Object { $_.ToString('X2') }) -join ''
if ($header -ne '0061736D0D000100') { throw 'Expected a WebAssembly component, not a core module' }
$null = New-Item -ItemType Directory -Force -Path $OutputDirectory
$destination = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) "$($manifest.id)-$($manifest.version).exaudio"
if (Test-Path -LiteralPath $destination) { throw "Release artifact already exists: $destination (immutable releases must not be overwritten)" }
$temp = "$destination.tmp-$([Guid]::NewGuid().ToString('N'))"
try {
    # Update (not Create) mode: .NET Framework forbids editing entry metadata in Create mode.
    $archive = [IO.Compression.ZipFile]::Open($temp, [IO.Compression.ZipArchiveMode]::Update)
    try {
        $entries = @(
            @{ Source = $manifestPath; Name = 'manifest.json' },
            @{ Source = $componentPath; Name = 'module.wasm' },
            @{ Source = (Join-Path $PSScriptRoot 'LICENSE'); Name = 'assets/LICENSE' }
        )
        foreach ($item in $entries) {
            $entry = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $item.Source, $item.Name, [IO.Compression.CompressionLevel]::Optimal)
            $entry.LastWriteTime = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
        }
    } finally { $archive.Dispose() }
    [IO.File]::Move($temp, $destination)
} finally { if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp } }
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash.ToLowerInvariant()
[ordered]@{ path = $destination; id = $manifest.id; version = $manifest.version; size = (Get-Item -LiteralPath $destination).Length; sha256 = $hash } | ConvertTo-Json
