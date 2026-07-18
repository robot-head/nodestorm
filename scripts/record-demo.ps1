# record-demo.ps1 - records the README demo segments of nodestorm.
# Window-scoped only (see uia-lib.ps1): safe while a human uses the desktop.
#   powershell -File scripts\record-demo.ps1                # all segments
#   powershell -File scripts\record-demo.ps1 -Segment 3     # one segment
# Frames + working files in target\demo\; final assets in docs\demo\ via -Publish.
#Requires -Version 5.1
[CmdletBinding()]
param(
    [int[]]$Segment,        # default: all
    [switch]$NoBuild,
    [switch]$Publish        # copy finished gifs/mp4/srt/poster into docs\demo\
)
$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'uia-lib.ps1')

$Port = 4801
$WorkDir = Join-Path $RepoRoot 'target\demo'
$SessionsDir = Join-Path $env:TEMP 'nodestorm-demo-sessions'
$PrefsFile = Join-Path $env:TEMP 'nodestorm-demo-prefs.json'
$Exe = Join-Path $RepoRoot 'target\debug\nodestorm.exe'
$AgentExe = Join-Path $RepoRoot 'target\debug\examples\demo_agent.exe'

# Resolve ffmpeg for this session (installed system-wide via winget, but not
# necessarily on PATH yet without a shell restart).
$script:Ffmpeg = (Get-Command ffmpeg -ErrorAction SilentlyContinue).Source
if (-not $script:Ffmpeg) {
    $script:Ffmpeg = (Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Gyan.FFmpeg*" -Recurse -Filter ffmpeg.exe -ErrorAction SilentlyContinue | Select-Object -First 1).FullName
}
if (-not $script:Ffmpeg) { Fail 'ffmpeg not found (winget install Gyan.FFmpeg first)' }

# BOM-less UTF-8 writer: Set-Content -Encoding utf8NoBOM is PS7+ only and
# this host runs Windows PowerShell 5.1, so go through .NET.
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding $false
function Write-Utf8NoBom([string]$Path, [string]$Content) {
    [System.IO.File]::WriteAllText($Path, $Content, $script:Utf8NoBom)
}

$script:App = $null
$script:AgentProc = $null
$script:Hwnd = [IntPtr]::Zero
$script:CaptureJob = $null
$script:CaptureStart = $null
$script:Captions = [System.Collections.Generic.List[object]]::new()

function Start-DemoApp([double]$W = 1160, [double]$H = 800, [switch]$FreshData) {
    if ($FreshData) {
        Remove-Item -Recurse -Force $SessionsDir -ErrorAction SilentlyContinue
        # Seed dark mode explicitly: the recording machine's OS is in light
        # mode, and the spec wants the dark default on camera.
        # (Set-Content -Encoding utf8NoBOM is PS7+ only; this host runs
        # Windows PowerShell 5.1, so write BOM-less UTF-8 via .NET instead.)
        $utf8NoBom = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText($PrefsFile, '{"theme":"nodestorm","mode":"dark"}', $utf8NoBom)
    }
    $script:App = Start-Process -FilePath $Exe -PassThru -WindowStyle Hidden `
        -ArgumentList '--port', $Port, '--sessions-dir', $SessionsDir, `
        '--prefs', $PrefsFile, '--window-size', ("{0}x{1}" -f $W, $H)
    if (-not (Wait-Tcp $Port)) { Fail "MCP port $Port never opened" }
    $script:AppWindow = Get-AppWindow $script:App.Id
    if (-not $script:AppWindow) { Fail 'app window not found in UIA' }
    $script:Hwnd = [IntPtr]$script:AppWindow.Current.NativeWindowHandle
    # Out of the user's way: bottom of z-order, never activated.
    [void][NodestormVerify.Native]::SetWindowPos($script:Hwnd, [IntPtr]1, 40, 40, 0, 0, 0x0013)
}

function Stop-DemoApp {
    foreach ($p in @($script:AgentProc, $script:App)) {
        if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    }
    $script:App = $null; $script:AgentProc = $null
}

function Start-Agent {
    $script:AgentProc = Start-Process -FilePath $AgentExe -PassThru -WindowStyle Hidden `
        -ArgumentList "http://127.0.0.1:$Port/mcp" `
        -RedirectStandardError (Join-Path $WorkDir 'agent.err.log') `
        -RedirectStandardOutput (Join-Path $WorkDir 'agent.out.log')
}

function Start-Capture([string]$SegDir) {
    # Wipe any frames from a previous run of this segment first: capture
    # always restarts numbering at frame_00000, so without this a shorter
    # re-run would leave stale trailing frames from a longer prior run and
    # corrupt the tail of the gif.
    $framesDir = Join-Path $SegDir 'frames'
    Remove-Item -Recurse -Force $framesDir -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $framesDir | Out-Null
    $script:Captions.Clear()
    $script:CaptureStart = Get-Date
    # 10fps PrintWindow loop in a background runspace: window-scoped capture
    # that works while the app is occluded at the bottom of the z-order.
    $ps = [powershell]::Create()
    [void]$ps.AddScript({
        param($Hwnd, $FramesDir, $LibPath)
        . $LibPath
        $i = 0
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        while ($true) {
            $target = [TimeSpan]::FromMilliseconds(100 * $i)
            $sleep = $target - $sw.Elapsed
            if ($sleep -gt [TimeSpan]::Zero) { Start-Sleep -Milliseconds $sleep.TotalMilliseconds }
            Save-WindowPng ([IntPtr]$Hwnd) (Join-Path $FramesDir ("frame_{0:D5}.png" -f $i))
            $i++
        }
    }).AddArgument([int64]$script:Hwnd).AddArgument((Join-Path $SegDir 'frames')).AddArgument((Join-Path $PSScriptRoot 'uia-lib.ps1'))
    $script:CaptureJob = @{ PS = $ps; Handle = $ps.BeginInvoke() }
}

function Stop-Capture {
    $script:CaptureJob.PS.Stop()
    $script:CaptureJob.PS.Dispose()
    $script:CaptureJob = $null
}

function Add-Caption([string]$Text, [double]$MinSeconds = 2.5) {
    if ($script:Captions.Count -gt 0) {
        $prev = $script:Captions[$script:Captions.Count - 1]
        # Pacing: the previous caption gets its full minimum on screen
        # before the next one starts — sleep out any remainder. This keeps
        # the drawtext enable-windows strictly non-overlapping
        # (prev.end == next.start), so two captions never render at once.
        $due = $prev.start + $prev.min
        $now = ((Get-Date) - $script:CaptureStart).TotalSeconds
        if ($now -lt $due) { Start-Sleep -Milliseconds (1000 * ($due - $now)) }
        $prev.end = ((Get-Date) - $script:CaptureStart).TotalSeconds
    }
    $t = ((Get-Date) - $script:CaptureStart).TotalSeconds
    $script:Captions.Add(@{ start = $t; end = 0; min = $MinSeconds; text = $Text })
}

function Invoke-Dwell([double]$Seconds) { Start-Sleep -Milliseconds (1000 * $Seconds) }

function Save-Captions([string]$SegDir) {
    $t = ((Get-Date) - $script:CaptureStart).TotalSeconds
    if ($script:Captions.Count -gt 0) {
        $last = $script:Captions[$script:Captions.Count - 1]
        $last.end = [Math]::Max($last.start + $last.min, $t)
    }
    Write-Utf8NoBom (Join-Path $SegDir 'captions.json') ($script:Captions | ConvertTo-Json)
}

function Convert-SegmentGif([string]$SegDir, [string]$OutGif, [int]$Fps = 10, [int]$Width = 800) {
    $caps = Get-Content (Join-Path $SegDir 'captions.json') | ConvertFrom-Json
    $font = 'C\:/Windows/Fonts/segoeui.ttf'
    # Caption text goes through per-caption textfiles rather than inline
    # text='...': no filtergraph escaping to get wrong (apostrophes, colons,
    # commas, whatever future captions hold).
    $i = 0
    $draw = foreach ($c in $caps) {
        $capFile = Join-Path $SegDir "cap_$i.txt"
        Write-Utf8NoBom $capFile $c.text
        $capPath = ($capFile -replace '\\', '/') -replace ':', '\:'
        "drawtext=fontfile='$font':textfile='$capPath':fontsize=22:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=10:x=(w-text_w)/2:y=h-th-16:enable='between(t,{0},{1})'" -f
            [Math]::Round($c.start, 2), [Math]::Round($c.end, 2)
        $i++
    }
    $vf = "fps=$Fps,scale=${Width}:-1:flags=lanczos," + ($draw -join ',') +
        ",split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer"
    & $script:Ffmpeg -y -framerate 10 -i (Join-Path $SegDir 'frames\frame_%05d.png') `
        -vf $vf -loop 0 $OutGif
    if ($LASTEXITCODE -ne 0) { Fail "ffmpeg gif failed for $OutGif" }
    Log ("{0}: {1:N1} MB" -f (Split-Path -Leaf $OutGif), ((Get-Item $OutGif).Length / 1MB))
}

function Invoke-Segment1 {
    $seg = Join-Path $WorkDir '01-propose'
    Start-DemoApp -FreshData
    Start-Capture $seg
    Add-Caption 'nodestorm: a live canvas your agent draws on while you decide' 3
    Invoke-Dwell 2.5
    Add-Caption 'Claude Code (any MCP agent) proposes an architecture...' 3
    Start-Agent
    if (-not (Wait-Element 'Sync Engine' 20)) { Fail 'proposed graph did not render' }
    Invoke-Dwell 1.5
    Add-Caption 'Components become cards, dependencies become edges' 3
    Invoke-Dwell 3
    Add-Caption 'Status rails: existing, proposed - and the agent is now waiting on YOU' 3.5
    Invoke-Dwell 3.5
    Save-Captions $seg
    Stop-Capture
    # App intentionally left running for segment 2 when running all segments.
}

New-Item -ItemType Directory -Force $WorkDir | Out-Null
# finally runs even when Fail's `exit 1` fires inside try, so a mid-segment
# timeout can't orphan nodestorm.exe/demo_agent.exe holding port 4801.
try {
    if (-not $NoBuild) {
        Push-Location $RepoRoot
        cargo build --bins --examples
        if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
        Pop-Location
    }
    $all = -not $Segment
    if ($all -or $Segment -contains 1) { Invoke-Segment1 }
    # segments 2..8 appended in the next task
    if ($all -or $Segment -contains 1) {
        Convert-SegmentGif (Join-Path $WorkDir '01-propose') (Join-Path $WorkDir '01-propose.gif')
    }
} finally {
    Stop-DemoApp
}
