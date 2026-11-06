# Drives the real Studio app (demo task, isolated profile, unreachable Babel, silent audio)
# without touching the desktop input: PostMessage keys/clicks/wheel, PrintWindow captures.
#
#   app.ps1 -Out <dir> -Mode shots [-Exe studio-app.exe]
#       Captures the window at three exactly reachable timeline states (playhead 0):
#         zoom1    the demo's opening view (zoom 1, viewport from 0 s)
#         zoom10   Ctrl+wheel over the ruler until zoom clamps at 10 (2000 px/s), from 0 s
#         zoom005  Ctrl+wheel until zoom clamps at 0.05 (10 px/s), from 0 s
#       -> <Out>/app/<state>.png and <Out>/app/states.json
#   app.ps1 -Out <dir> -Mode cpu -Exe <exe> -Label <name> [-Runs 3] [-Seconds 30]
#       Plays the demo for -Seconds at zoom 1 and at zoom 10 and records the process CPU
#       time -> <Out>/cpu/<label>.jsonl (one line per run; % of one core).
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [ValidateSet('shots', 'cpu')][string]$Mode = 'shots',
    [string]$Exe = "$PSScriptRoot/../../target/release/studio-app.exe",
    [string]$Label = 'current',
    [int]$Runs = 3,
    [int]$Seconds = 30,
    [string]$NodeModules = '',
    # Lane height in CSS px of the build under test (125 before the Babel lane heights).
    [double]$LaneCss = 150
)
$ErrorActionPreference = 'Stop'
if ($env:CARGO_TARGET_DIR -and -not $PSBoundParameters.ContainsKey('Exe')) {
    $Exe = "$env:CARGO_TARGET_DIR/release/studio-app.exe"
}
$Exe = (Resolve-Path $Exe).Path

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern bool GetKeyboardState(byte[] s);
  [DllImport("user32.dll")] public static extern bool SetKeyboardState(byte[] s);
}
"@
[W]::SetProcessDPIAware() | Out-Null

function Log($message) { Write-Host "[$(Get-Date -Format o)] $message" }

function Start-Studio($home_) {
    if (Test-Path $home_) { Remove-Item -Recurse -Force $home_ }
    New-Item -ItemType Directory -Force -Path $home_ | Out-Null
    $env:APPDATA = $home_ -replace '/', '\'
    $env:STUDIO_PROVIDER_DIR = "$home_/providers"
    $env:STUDIO_SUBACCOUNT_RELAY_SETTINGS = "$home_/relay.json"
    $env:STUDIO_BABEL_SERVICE_BASE_URL = 'http://127.0.0.1:9'
    $env:STUDIO_BABEL_CLERK_URL = 'http://127.0.0.1:9'
    $env:STUDIO_REVIEW_BACKEND_BASE_URL = 'http://127.0.0.1:9'
    $env:STUDIO_AUDIO_SILENT = '1'
    $env:STUDIO_OPEN_DEMO_ON_START = '1'
    $proc = Start-Process $Exe -PassThru -RedirectStandardError "$home_/stderr.log" -RedirectStandardOutput "$home_/stdout.log"
    for ($i = 0; $i -lt 120 -and $proc.MainWindowHandle -eq [IntPtr]::Zero; $i++) {
        Start-Sleep -Milliseconds 250; $proc.Refresh()
    }
    if ($proc.MainWindowHandle -eq [IntPtr]::Zero) { throw 'studio window did not appear' }
    $h = $proc.MainWindowHandle
    # Fixed outer size so layouts repeat (device px).
    [W]::SetWindowPos($h, [IntPtr]::Zero, 40, 40, 1700, 1300, 0x0004) | Out-Null
    Start-Sleep -Seconds 12   # demo audio decode + display signal
    @{ proc = $proc; h = $h }
}

function Shot($app, $path) {
    $r = New-Object W+RECT
    [W]::GetWindowRect($app.h, [ref]$r) | Out-Null
    $bmp = New-Object System.Drawing.Bitmap ($r.Right - $r.Left), ($r.Bottom - $r.Top)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [W]::PrintWindow($app.h, $hdc, 2) | Out-Null
    $g.ReleaseHdc($hdc)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

# Window-pixel (screenshot) coordinates -> client coordinates.
function Client($app, [int]$x, [int]$y) {
    $wr = New-Object W+RECT
    [W]::GetWindowRect($app.h, [ref]$wr) | Out-Null
    $o = New-Object W+POINT
    [W]::ClientToScreen($app.h, [ref]$o) | Out-Null
    @(($x - ($o.X - $wr.Left)), ($y - ($o.Y - $wr.Top)), ($wr.Left + $x), ($wr.Top + $y))
}

function Set-Ctrl($app, [bool]$down) {
    $tmp = 0
    $ui = [W]::GetWindowThreadProcessId($app.h, [ref]$tmp)
    $me = [W]::GetCurrentThreadId()
    if ($down) { [W]::AttachThreadInput($me, $ui, $true) | Out-Null }
    $state = New-Object byte[] 256
    [W]::GetKeyboardState($state) | Out-Null
    $v = if ($down) { 0x80 } else { 0 }
    $state[0x11] = $v; $state[0xA2] = $v
    [W]::SetKeyboardState($state) | Out-Null
    if (-not $down) { [W]::AttachThreadInput($me, $ui, $false) | Out-Null }
}

function Click($app, [int]$x, [int]$y) {
    $c = Client $app $x $y
    $l = [IntPtr](($c[1] -shl 16) -bor $c[0])
    [W]::PostMessage($app.h, 0x0200, [IntPtr]0, $l) | Out-Null; Start-Sleep -Milliseconds 100
    [W]::PostMessage($app.h, 0x0201, [IntPtr]1, $l) | Out-Null; Start-Sleep -Milliseconds 100
    [W]::PostMessage($app.h, 0x0202, [IntPtr]0, $l) | Out-Null; Start-Sleep -Milliseconds 300
}

function Key($app, [int]$vk) {
    [W]::PostMessage($app.h, 0x0100, [IntPtr]$vk, [IntPtr]1) | Out-Null; Start-Sleep -Milliseconds 80
    [W]::PostMessage($app.h, 0x0101, [IntPtr]$vk, [IntPtr]([int64]0xC0000001)) | Out-Null
    Start-Sleep -Milliseconds 300
}

# Ctrl+wheel over (x, y) until the zoom clamps (`notches` > 0 zooms in).
function Zoom-Clamp($app, [int]$x, [int]$y, [int]$notches) {
    $c = Client $app $x $y
    [W]::PostMessage($app.h, 0x0200, [IntPtr]0, [IntPtr](($c[1] -shl 16) -bor $c[0])) | Out-Null
    Start-Sleep -Milliseconds 100
    Set-Ctrl $app $true
    $delta = if ($notches -gt 0) { 600 } else { -600 }
    $w = [IntPtr]([int64]((($delta -band 0xFFFF) -shl 16) -bor 0x0008))
    $l = [IntPtr](($c[3] -shl 16) -bor ($c[2] -band 0xFFFF))
    for ($i = 0; $i -lt [math]::Abs($notches); $i++) {
        [W]::PostMessage($app.h, 0x020A, $w, $l) | Out-Null; Start-Sleep -Milliseconds 40
    }
    Start-Sleep -Milliseconds 300
    Set-Ctrl $app $false
    Start-Sleep -Milliseconds 700
}

# The ruler row sits just above the first lane. In the opening view the first lane (Speaker 1)
# is silent: its first #707070 (unplayed waveform) row is the centre line, one px above the
# lane's middle, and the 22 CSS px ruler row ends one border above the lane.
function Find-Ruler($path, $dpr) {
    $bmp = New-Object System.Drawing.Bitmap $path
    try {
        $x = [int]($bmp.Width * 0.6)
        for ($y = 0; $y -lt $bmp.Height; $y++) {
            $p = $bmp.GetPixel($x, $y)
            if ($p.R -eq 112 -and $p.G -eq 112 -and $p.B -eq 112) {
                $half = [math]::Round($LaneCss * $dpr) / 2
                return @($x, [int]($y + 1 - $half - 12 * $dpr))
            }
        }
        throw 'no waveform pixels found'
    } finally { $bmp.Dispose() }
}

if ($Mode -eq 'shots') {
    $dir = "$Out/app"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $app = Start-Studio "$dir/home"
    try {
        $dpr = [W]::GetDpiForWindow($app.h) / 96.0
        Shot $app "$dir/zoom1.png"
        Log "zoom1 captured (dpr $dpr)"
        $r = Find-Ruler "$dir/zoom1.png" $dpr
        # Ctrl+wheel over the ruler row (no lane hover) zooms around the playhead (0 s).
        $states = @(@{ id = 'zoom1'; zoom = 1.0 })
        Zoom-Clamp $app $r[0] $r[1] 60
        Shot $app "$dir/zoom10.png"; Log 'zoom10 captured'
        $states += @{ id = 'zoom10'; zoom = 10.0 }
        Zoom-Clamp $app $r[0] $r[1] -80
        Shot $app "$dir/zoom005.png"; Log 'zoom005 captured'
        $states += @{ id = 'zoom005'; zoom = 0.05 }
        @{ dpr = $dpr; playhead = 0.0; start = 0.0; wheel_at = $r; states = $states } |
            ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 "$dir/states.json"
    } finally {
        Stop-Process -Id $app.proc.Id -Force -ErrorAction SilentlyContinue
    }
} else {
    $dir = "$Out/cpu"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $results = "$dir/$Label.jsonl"
    foreach ($zoom in @('zoom1', 'zoom10')) {
        for ($run = 1; $run -le $Runs; $run++) {
            $app = Start-Studio "$dir/home-$Label"
            try {
                $dpr = [W]::GetDpiForWindow($app.h) / 96.0
                Shot $app "$dir/$Label-$zoom-$run-a.png"
                # The rows above the lanes are laid out the same in every build, so the ruler
                # found in the first measured build (whose waveform is #707070) is reused for
                # builds that draw it in other colours.
                $rulerFile = "$dir/ruler.json"
                if (Test-Path $rulerFile) {
                    $r = Get-Content $rulerFile | ConvertFrom-Json
                } else {
                    $r = Find-Ruler "$dir/$Label-$zoom-$run-a.png" $dpr
                    ConvertTo-Json @($r) | Set-Content -Encoding utf8 $rulerFile
                }
                if ($zoom -eq 'zoom10') { Zoom-Clamp $app $r[0] $r[1] 60 }
                Click $app $r[0] $r[1]     # focus the timeline (the ruler has no click action)
                Start-Sleep -Seconds 2
                $app.proc.Refresh()
                $cpu0 = $app.proc.TotalProcessorTime.TotalMilliseconds
                $t0 = Get-Date
                Key $app 0x20             # Space: play
                Start-Sleep -Seconds $Seconds
                $app.proc.Refresh()
                $cpu1 = $app.proc.TotalProcessorTime.TotalMilliseconds
                $wall = ((Get-Date) - $t0).TotalMilliseconds
                Shot $app "$dir/$Label-$zoom-$run-b.png"
                Key $app 0x20             # pause
                $line = @{ label = $Label; zoom = $zoom; run = $run; seconds = $wall / 1000;
                    cpu_ms = $cpu1 - $cpu0; cpu_percent_of_core = 100 * ($cpu1 - $cpu0) / $wall;
                    exe = $Exe } | ConvertTo-Json -Compress
                Add-Content -Encoding utf8 $results $line
                Log $line
            } finally {
                Stop-Process -Id $app.proc.Id -Force -ErrorAction SilentlyContinue
                Start-Sleep -Seconds 2
            }
        }
    }
}
