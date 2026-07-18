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

# ---------- frame-resolution normalization (segment 8 fix) ----------
# Segment 8 relaunches the app at two different window widths (760px then
# 520px), so its two capture runs produce two different PNG resolutions.
# ffmpeg's image2 demuxer reconfigures (tears down and recreates) its whole
# -vf filter graph the instant it sees a frame whose size differs from the
# previous one. That reconfiguration cascades to every filter in the chain,
# including palettegen/paletteuse (Convert-SegmentGif's gif path) - and
# palettegen only flushes its accumulated palette at its OWN instance's
# end-of-stream. When the graph is rebuilt mid-stream, the pre-boundary
# instance never reaches EOF, so every frame it already buffered is
# silently dropped from the encoded gif (confirmed: an encode of the raw
# mixed-resolution merged directory produced a gif whose frame count and
# duration exactly matched an encode of the post-boundary run ALONE - the
# pre-boundary run's frames vanished with no ffmpeg error). Padding
# in-graph (an ffmpeg `pad` filter ahead of the rest of the chain) does
# NOT help: the reconfiguration is triggered upstream, by the buffer
# source itself, before any filter runs. libx264 (Publish-Demo's mp4
# path) tolerates the same mid-stream resolution change without dropping
# frames - only the gif's palette-based encode is affected - but the only
# fix that removes the resolution change (and therefore the
# reconfiguration) entirely is to materialize every frame at one fixed
# size on disk before ffmpeg ever opens the sequence.
Add-Type -AssemblyName System.Drawing

function Get-ImageSize([string]$Path) {
    $img = [System.Drawing.Image]::FromFile($Path)
    try { return @{ W = $img.Width; H = $img.Height } } finally { $img.Dispose() }
}

function Copy-FramePadded([string]$Src, [string]$Dst, [int]$CanvasW, [int]$CanvasH) {
    $srcImg = [System.Drawing.Image]::FromFile($Src)
    $sameSize = ($srcImg.Width -eq $CanvasW -and $srcImg.Height -eq $CanvasH)
    if ($sameSize) {
        $srcImg.Dispose()
        Copy-Item $Src $Dst -Force
        return
    }
    try {
        $canvas = New-Object System.Drawing.Bitmap $CanvasW, $CanvasH
        try {
            $g = [System.Drawing.Graphics]::FromImage($canvas)
            try {
                $g.Clear([System.Drawing.Color]::Black)
                $x = [int](($CanvasW - $srcImg.Width) / 2)
                $y = [int](($CanvasH - $srcImg.Height) / 2)
                $g.DrawImage($srcImg, $x, $y, $srcImg.Width, $srcImg.Height)
            } finally { $g.Dispose() }
            $canvas.Save($Dst, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $canvas.Dispose() }
    } finally {
        $srcImg.Dispose()
    }
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

function Read-Captions([string]$SegDir) {
    # Get-Content | ConvertFrom-Json piped straight into an @(...) wrapper
    # misbehaves on this host's Windows PowerShell 5.1: for a multi-element
    # JSON array it silently collapses to a single merged object (each
    # property becomes an array of every element's values) instead of an
    # array of objects. Assign first, then wrap the variable - reliable on
    # both 5.1 and 7+.
    $caps = Get-Content (Join-Path $SegDir 'captions.json') | ConvertFrom-Json
    return @($caps)
}

function Convert-SegmentGif([string]$SegDir, [string]$OutGif, [int]$Fps = 10, [int]$Width = 800) {
    $caps = Read-Captions $SegDir
    $font = 'C\:/Windows/Fonts/segoeui.ttf'
    # Caption text goes through per-caption textfiles rather than inline
    # text='...': no filtergraph escaping to get wrong (apostrophes, colons,
    # commas, whatever future captions hold).
    $i = 0
    $draw = foreach ($c in $caps) {
        $capFile = Join-Path $SegDir "cap_$i.txt"
        Write-Utf8NoBom $capFile $c.text
        $capPath = ($capFile -replace '\\', '/') -replace ':', '\:'
        # expansion=none: a bare % in a caption would otherwise be read as a
        # strftime/eval directive and silently blank the caption.
        # gte(t,a)*lt(t,b) (not between(t,a,b)): between() is a closed
        # interval, so on a frame-aligned boundary both the outgoing and
        # incoming caption's enable expression can evaluate true for the
        # same frame and double-render; gte*lt is half-open, so exactly one
        # caption is active at any sampled t.
        "drawtext=fontfile='$font':textfile='$capPath':expansion=none:fontsize=22:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=10:x=(w-text_w)/2:y=h-th-16:enable='gte(t,{0})*lt(t,{1})'" -f
            [Math]::Round($c.start, 2), [Math]::Round($c.end, 2)
        $i++
    }
    $capDraw = $draw -join ','

    function Invoke-GifPass([int]$PassFps, [int]$PassWidth) {
        $vf = "fps=$PassFps,scale=${PassWidth}:-1:flags=lanczos,$capDraw" +
            ",split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer"
        & $script:Ffmpeg -y -framerate 10 -i (Join-Path $SegDir 'frames\frame_%05d.png') `
            -vf $vf -loop 0 $OutGif
        if ($LASTEXITCODE -ne 0) { Fail "ffmpeg gif failed for $OutGif" }
    }

    Invoke-GifPass $Fps $Width
    # Budget: each gif <=4MB. Degrade fps/width per the brief's fallback
    # ladder rather than failing the whole run over one oversized segment.
    $ladder = @(@(8, 720), @(8, 640), @(6, 560))
    $step = 0
    while (((Get-Item $OutGif).Length / 1MB) -gt 4 -and $step -lt $ladder.Count) {
        $f, $w = $ladder[$step]
        Log ("{0}: {1:N1} MB over budget, retrying at {2}fps/{3}px" -f (Split-Path -Leaf $OutGif), ((Get-Item $OutGif).Length / 1MB), $f, $w)
        Invoke-GifPass $f $w
        $step++
    }
    Log ("{0}: {1:N1} MB" -f (Split-Path -Leaf $OutGif), ((Get-Item $OutGif).Length / 1MB))
}

# ---------- extra helpers for segments 2-8 ----------

function Move-To-Element([IntPtr]$TopHwnd, [string]$Name) {
    # Hover only: a posted WM_MOUSEMOVE with no button messages, so the
    # ripple/hover preview lights up without picking anything. Same
    # window-targeted technique as Click-Element, just the first half of it.
    $el = Find-Element $Name
    if (-not $el) { Fail "element to hover not found: '$Name'" }
    $r = $el.Current.BoundingRectangle
    $rw = Get-RenderWidget $TopHwnd
    if ($rw -eq [IntPtr]::Zero) { Fail 'WebView2 render widget window not found' }
    $rwRect = New-Object NodestormVerify.Native+RECT
    [void][NodestormVerify.Native]::GetWindowRect($rw, [ref]$rwRect)
    $cx = [int]($r.X + $r.Width / 2) - $rwRect.Left
    $cy = [int]($r.Y + $r.Height / 2) - $rwRect.Top
    $lp = [IntPtr](($cy -shl 16) -bor ($cx -band 0xFFFF))
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0200, [IntPtr]::Zero, $lp) # WM_MOUSEMOVE
    Log "hovered '$Name' (client $cx,$cy)"
}

function Find-ElementContains([string]$Needle) {
    # Find-Element/Click-Element/Wait-Element all match the UIA Name
    # exactly. Some receipts we need to wait on embed a dynamic path (the
    # export decision record's filename), so this does a substring scan of
    # every descendant instead - read-only UIA enumeration, no input.
    try {
        $all = $script:AppWindow.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition)
        foreach ($el in $all) {
            if ($el.Current.Name -and $el.Current.Name.Contains($Needle)) { return $el }
        }
    } catch {
        return $null
    }
    return $null
}

function Wait-ElementContains([string]$Needle, [int]$TimeoutSec = 15) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $el = Find-ElementContains $Needle
        if ($el) { return $el }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

function RightClick-Element([IntPtr]$TopHwnd, [string]$Name) {
    # Same window-targeted technique as Click-Element, but WM_RBUTTONDOWN/UP
    # to open the app's own right-click context menu (context_menu.rs).
    # Used instead of clicking a node's small in-card group pill directly:
    # all three cards in the "sync" group carry an identically-named "sync"
    # pill, and on this host, after a pan/zoom reset, two of the three
    # same-named elements' UIA BoundingRectangle come back stale/garbage
    # (off-screen-negative X), confirmed by dumping all three rects plus
    # before/after screenshots - clicking the resolved point hit nothing.
    # 'Sync Engine' itself (a unique name) is never affected, so
    # right-clicking it and picking the menu's "Collapse group" (also
    # unique) sidesteps the whole same-name ambiguity.
    $el = Find-Element $Name
    if (-not $el) { Fail "element to right-click not found: '$Name'" }
    $r = $el.Current.BoundingRectangle
    $rw = Get-RenderWidget $TopHwnd
    if ($rw -eq [IntPtr]::Zero) { Fail 'WebView2 render widget window not found' }
    $rwRect = New-Object NodestormVerify.Native+RECT
    [void][NodestormVerify.Native]::GetWindowRect($rw, [ref]$rwRect)
    $cx = [int]($r.X + $r.Width / 2) - $rwRect.Left
    $cy = [int]($r.Y + $r.Height / 2) - $rwRect.Top
    $lp = [IntPtr](($cy -shl 16) -bor ($cx -band 0xFFFF))
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0200, [IntPtr]::Zero, $lp) # WM_MOUSEMOVE
    Start-Sleep -Milliseconds 50
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0204, [IntPtr]2, $lp)      # WM_RBUTTONDOWN
    Start-Sleep -Milliseconds 50
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0205, [IntPtr]::Zero, $lp) # WM_RBUTTONUP
    Log "right-clicked '$Name' (client $cx,$cy)"
}

function Click-ElementContains([IntPtr]$TopHwnd, [string]$Needle) {
    # Click-Element's Find-Element match is exact; use this instead when the
    # target's Name can carry a dynamic suffix (a session row's agent-
    # waiting/open-choices pill text appended after its name).
    $el = Find-ElementContains $Needle
    if (-not $el) { Fail "element containing '$Needle' not found" }
    $r = $el.Current.BoundingRectangle
    Click-Point $TopHwnd ($r.X + $r.Width / 2) ($r.Y + $r.Height / 2)
    Log "clicked element containing '$Needle'"
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

function Invoke-Segment2 {
    # Decide both open choices, add a note, Send - same app/session as
    # segment 1 (the graph and decisions must accumulate).
    $seg = Join-Path $WorkDir '02-decide'
    Start-Capture $seg
    Click-Element $script:Hwnd 'Sync Engine'
    if (-not (Wait-Element 'Conflict resolution strategy' 10)) { Fail 'choice panel did not open on Sync Engine' }
    Add-Caption 'Every open choice is pinned to its component' 3
    Move-To-Element $script:Hwnd 'CRDT document model'
    Add-Caption 'Hover an option - the components it would ripple into light up' 3
    Invoke-Dwell 3
    Click-Element $script:Hwnd 'CRDT document model'
    Add-Caption 'Picked - with the trade-offs recorded' 2.5
    # Let the picked option + its trade-offs sit on screen before the panel
    # closes - without this the close (and everything after it) happens
    # instantly, so viewers never actually see the state this caption
    # describes.
    Invoke-Dwell 2
    Click-Element $script:Hwnd ([string][char]0x2715)   # close panel
    Click-Element $script:Hwnd 'Notes Store'
    if (-not (Wait-Element 'Edit history storage' 10)) { Fail 'choice panel did not open on Notes Store' }
    Click-Element $script:Hwnd 'Append-only event log'
    Click-Element $script:Hwnd ('optional message to the agent' + [char]0x2026)
    Type-Text $script:Hwnd 'prefer CRDTs - offline first'
    Add-Caption 'Add a note for the agent and send your decisions' 3
    # Show the typed note before Send whisks it away.
    Invoke-Dwell 2
    Click-Element $script:Hwnd 'Send to agent'
    # The agent reacts once per delivery: Sync Engine -> modified, Notes
    # Store -> affected. The status tag renders lowercase but CSS
    # text-transform:uppercase changes the UIA-exposed name (see
    # verify-windows.ps1), so wait on the uppercase tag.
    if (-not (Wait-Element 'MODIFIED' 20)) { Fail 'agent did not react with a modified Sync Engine' }
    Add-Caption 'The agent wakes with your decisions and updates the graph' 3
    Invoke-Dwell 3
    Save-Captions $seg
    Stop-Capture
}

function Invoke-Segment3 {
    # Add a user component, rename it, connect it, soft-delete an
    # agent-authored node, then undo - same app/session as segments 1-2.
    $seg = Join-Path $WorkDir '03-edit'
    Start-Capture $seg
    Click-Element $script:Hwnd '+ Component'
    Add-Caption 'The canvas is yours too - add your own components' 3
    if (-not (Wait-Element 'New component' 10)) { Fail 'user component did not appear' }
    # Let the freshly-added default-named card sit on screen for a beat -
    # without this the rename form opens over it instantly, so viewers
    # never see the "you just added a component" state this caption
    # describes before it's replaced by the rename UI.
    Invoke-Dwell 2
    if (-not (Wait-Element 'Edit' 10)) { Fail 'panel did not open with an Edit button' }
    # Rename via the panel's Edit form - the exact click/backspace/type
    # sequence proven in verify-windows.ps1's user-editing flow.
    Click-Element $script:Hwnd 'Edit'
    Start-Sleep -Milliseconds 300
    $editEl = Find-Element 'Edit'
    if (-not $editEl) { Fail 'Edit button vanished after opening the form' }
    $er = $editEl.Current.BoundingRectangle
    Click-Point $script:Hwnd ($er.X + $er.Width / 2) ($er.Y + $er.Height + 18)
    Send-Key $script:Hwnd 0x23   # VK_END
    for ($i = 0; $i -lt 20; $i++) { Send-Key $script:Hwnd 0x08 }   # Backspace x 20
    Type-Text $script:Hwnd 'Metrics Collector'
    Click-Element $script:Hwnd 'Save'
    if (-not (Wait-Element 'Metrics Collector' 10)) { Fail 'rename did not land on the card' }
    Add-Caption 'Rename, connect, delete - your edits flow back to the agent' 3
    if (-not (Wait-Element ('Connect ' + [char]0x2192) 5)) { Fail 'Connect button missing' }
    Click-Element $script:Hwnd ('Connect ' + [char]0x2192)
    Click-Element $script:Hwnd 'Notes API'
    $connRow = 'Metrics Collector ' + [char]0x2014 + 'depends on' + [char]0x2192 + ' Notes API'
    if (-not (Wait-Element $connRow 10)) { Fail "connection row missing: '$connRow'" }
    # The open panel can overlay the next card's clickable area - close it
    # first (same guard as verify-windows.ps1).
    Click-Element $script:Hwnd ([string][char]0x2715)
    Start-Sleep -Milliseconds 300
    Click-Element $script:Hwnd 'Presence Service'
    if (-not (Wait-Element 'Delete' 10)) { Fail 'panel Delete button missing on Presence Service' }
    Click-Element $script:Hwnd 'Delete'
    if (-not (Wait-Element 'REMOVED' 10)) { Fail 'agent node was not marked removed' }
    Add-Caption "Deleting an agent's component asks the agent politely" 3
    # Show the REMOVED tag before Undo clears it right back out.
    Invoke-Dwell 2
    Click-Element $script:Hwnd ([string][char]0x21B6 + ' Undo')
    if (-not (Wait-ElementGone 'REMOVED' 10)) { Fail 'undo did not clear the removal' }
    Add-Caption 'Undo covers every edit until decisions are delivered' 2.5
    Invoke-Dwell 2.5
    Save-Captions $seg
    Stop-Capture
}

function Invoke-Segment4 {
    # Minimap group collapse/expand, then search + zoom-cycle - same
    # app/session. Collapse/expand is done FIRST, before the view has been
    # panned anywhere: the pan/zoom-cycle later in this segment leaves the
    # viewport (and the accuracy of every element's UIA BoundingRectangle -
    # see below) in a state that's no longer safe to click against, so nothing
    # after it needs precise coordinates. The brief orders the beats
    # search-then-collapse; the four captions/actions are the same, just
    # reordered for reliability (confirmed both readings tell the same
    # story: search/zoom, minimap collapse/expand).
    $seg = Join-Path $WorkDir '04-navigate'
    Start-Capture $seg
    Add-Caption 'The minimap pans big graphs - groups collapse into one card' 3
    # Collapse via right-click -> "Collapse group" (context_menu.rs), not by
    # clicking the small in-card group pill directly: all three cards in the
    # "sync" group carry an identically-named "sync" pill, and on this host
    # two of the three same-named elements' UIA BoundingRectangle come back
    # garbage (confirmed by dumping all three rects + screenshots). Right-
    # clicking 'Sync Engine' (a unique name) and picking "Collapse group"
    # (also unique) sidesteps that entirely.
    RightClick-Element $script:Hwnd 'Sync Engine'
    if (-not (Wait-Element 'Collapse group' 5)) { Fail 'context menu did not open on Sync Engine' }
    Click-Element $script:Hwnd 'Collapse group'
    if (-not (Wait-Element '3 components' 10)) { Fail 'sync group did not collapse into a cluster card' }
    Invoke-Dwell 2
    Click-Element $script:Hwnd ([string][char]0x229E + ' expand')
    if (-not (Wait-ElementGone '3 components' 10)) { Fail 'sync cluster card did not expand back into its components' }
    Invoke-Dwell 1.5
    Click-Element $script:Hwnd ('search components' + [char]0x2026)
    Type-Text $script:Hwnd 'notes'
    Add-Caption 'Search highlights matches...' 3
    # Show the highlighted-but-not-yet-zoomed matches before Enter pans
    # the view - otherwise the zoom fires instantly and viewers never see
    # the plain highlight state this caption describes.
    Invoke-Dwell 1.5
    Send-Key $script:Hwnd 0x0D   # Enter
    Invoke-Dwell 1.2
    Send-Key $script:Hwnd 0x0D   # Enter again: cycles to the next match
    Add-Caption '...and Enter zoom-cycles through them' 3
    Invoke-Dwell 1.2
    Send-Key $script:Hwnd 0x1B   # Esc clears the search
    Save-Captions $seg
    Stop-Capture
}

function Invoke-Segment5 {
    # Create a session, switch back, compare, timeline - same app/session.
    $seg = Join-Path $WorkDir '05-sessions'
    Start-Capture $seg
    Click-Element $script:Hwnd ('default ' + [char]0x25BE)
    if (-not (Wait-Element 'Create' 5)) { Fail 'sessions menu did not open' }
    Click-Element $script:Hwnd ('new session' + [char]0x2026)
    Type-Text $script:Hwnd 'experiment'
    Click-Element $script:Hwnd 'Create'
    if (-not (Wait-Element 'Waiting for an agent to connect.' 10)) {
        Fail 'created session did not become the active empty canvas'
    }
    Add-Caption 'Sessions are parallel brainstorms - agents can wait on one while you work in another' 3.5
    # Let the new empty "waiting for an agent" canvas sit on screen before
    # the switcher menu opens over it.
    Invoke-Dwell 1.5
    if (-not (Wait-Element ('experiment ' + [char]0x25BE) 10)) {
        Fail 'switcher label did not update to the new active session'
    }
    Click-Element $script:Hwnd ('experiment ' + [char]0x25BE)
    # The 'default' row may carry an agent-waiting pill (the demo agent
    # never exits, unlike verify-windows.ps1's drive example) - its exact
    # accessible Name can be "default ●" rather than plain "default",
    # so match by substring and click its resolved rect rather than by
    # exact name.
    $defaultRow = Wait-ElementContains 'default' 5
    if (-not $defaultRow) { Fail 'original session missing from the list' }
    Click-ElementContains $script:Hwnd 'default'
    if (-not (Wait-Element 'Sync Engine' 10)) { Fail 'original graph did not return after switching back' }
    Click-Element $script:Hwnd ('default ' + [char]0x25BE)
    if (-not (Wait-Element 'Compare' 5)) { Fail 'Compare button missing for the experiment row' }
    Click-Element $script:Hwnd 'Compare'
    if (-not (Wait-Element 'Session diff' 10)) { Fail 'diff panel did not open' }
    Add-Caption 'Compare shows how two sessions drifted' 3
    # The diff panel must actually be visible while this caption is on
    # screen - without this dwell the close (and the Timeline open right
    # after it) happens instantly, so viewers see the Timeline panel
    # instead of the diff this caption describes.
    Invoke-Dwell 2
    Click-Element $script:Hwnd ([string][char]0x2715)
    Click-Element $script:Hwnd 'Timeline'
    if (-not (Wait-Element 'Session timeline' 5)) { Fail 'timeline panel did not open' }
    Add-Caption "The timeline is the session's full decision log" 2.5
    Invoke-Dwell 2.5
    Click-Element $script:Hwnd 'Timeline'
    Save-Captions $seg
    Stop-Capture
}

function Invoke-Segment6 {
    # Export via More -> Export - same app/session.
    $seg = Join-Path $WorkDir '06-export'
    Start-Capture $seg
    Click-Element $script:Hwnd 'More'
    if (-not (Wait-Element ('Export ' + [char]0x25BE) 5)) { Fail 'More menu did not open' }
    Click-Element $script:Hwnd ('Export ' + [char]0x25BE)
    if (-not (Wait-Element 'Copy Markdown' 5)) { Fail 'export accordion did not open' }
    Add-Caption 'Export writes a Markdown decision record - pros, cons, and the trail' 3
    Click-Element $script:Hwnd 'Export'
    # src/store.rs record_export's activity-feed text is "exported decision
    # record to {path}" - the path is dynamic, so match by substring.
    if (-not (Wait-ElementContains 'exported decision record to' 10)) {
        Fail 'export receipt did not appear in the activity feed'
    }
    Invoke-Dwell 3
    Save-Captions $seg
    Stop-Capture
}

function Invoke-Segment7 {
    # Live theme/mode switching via More -> Theme - same app/session.
    $seg = Join-Path $WorkDir '07-themes'
    Start-Capture $seg
    Click-Element $script:Hwnd 'More'
    if (-not (Wait-Element ('Theme ' + [char]0x25BE) 5)) { Fail 'More menu did not open' }
    Click-Element $script:Hwnd ('Theme ' + [char]0x25BE)
    if (-not (Wait-Element 'Gruvbox' 5)) { Fail 'theme accordion did not open' }
    Add-Caption 'Twelve palettes, light and dark, live-switching' 3
    # Show the open palette menu before the first mode click repaints the
    # whole UI - without this the switch happens instantly and viewers
    # never see the menu of palettes this caption introduces.
    Invoke-Dwell 1.5
    Click-Element $script:Hwnd 'Light'   # mode click: menu stays open
    Invoke-Dwell 1.5
    Click-Element $script:Hwnd 'Gruvbox'   # family click: menu closes
    if (-not (Wait-ElementGone 'Gruvbox' 10)) { Fail 'theme menu did not close after picking Gruvbox' }
    Invoke-Dwell 1.5
    Click-Element $script:Hwnd 'More'
    if (-not (Wait-Element ('Theme ' + [char]0x25BE) 5)) { Fail 'More menu did not reopen' }
    Click-Element $script:Hwnd ('Theme ' + [char]0x25BE)
    if (-not (Wait-Element 'Catppuccin' 5)) { Fail 'theme accordion did not reopen' }
    Click-Element $script:Hwnd 'Catppuccin'
    if (-not (Wait-ElementGone 'Catppuccin' 10)) { Fail 'theme menu did not close after picking Catppuccin' }
    Invoke-Dwell 2
    # Restore the dark/nodestorm default so the recorder leaves the app in
    # its canonical look (the prefs file is scratch either way).
    Click-Element $script:Hwnd 'More'
    if (-not (Wait-Element ('Theme ' + [char]0x25BE) 5)) { Fail 'More menu did not reopen (restore)' }
    Click-Element $script:Hwnd ('Theme ' + [char]0x25BE)
    if (-not (Wait-Element 'Dark' 5)) { Fail 'theme accordion did not reopen (restore)' }
    Click-Element $script:Hwnd 'Dark'   # mode click: menu stays open
    Click-Element $script:Hwnd 'Nodestorm'   # family click: menu closes
    if (-not (Wait-ElementGone 'Nodestorm' 10)) { Fail 'theme menu did not close after restoring Nodestorm' }
    Save-Captions $seg
    Stop-Capture
}

function Invoke-Segment8 {
    # Two fresh launches (760x840, then 520x840): a running window must
    # never be resized (WebView2 occluded-resize freeze -
    # docs/webview2-occluded-resize.md), so stop/relaunch instead. Data
    # persists (same SessionsDir, no -FreshData): the graph from segments
    # 1-7 is still there. Each launch gets its own capture run under
    # 08-responsive\run1 / run2; frames and captions are merged afterwards
    # into 08-responsive\frames + captions.json so Convert-SegmentGif and
    # Publish-Demo/Write-DemoSrt need no segment-8-specific handling at all.
    $seg = Join-Path $WorkDir '08-responsive'
    $dir1 = Join-Path $seg 'run1'
    $dir2 = Join-Path $seg 'run2'
    New-Item -ItemType Directory -Force $dir1 | Out-Null
    New-Item -ItemType Directory -Force $dir2 | Out-Null

    Stop-DemoApp   # end the segments 1-7 instance before relaunching narrower
    Start-DemoApp 760 840
    Start-Capture $dir1
    Add-Caption 'Narrow window? The bar folds - message the agent from the compose pod' 3
    if (-not (Wait-Element 'Message to agent' 10)) { Fail 'compose pod missing at 760px' }
    Click-Element $script:Hwnd 'Message to agent'
    Start-Sleep -Milliseconds 300
    Click-Element $script:Hwnd ('optional message to the agent' + [char]0x2026)
    Type-Text $script:Hwnd 'ship it'
    Click-Element $script:Hwnd 'Send with message'
    Invoke-Dwell 2
    Save-Captions $dir1
    Stop-Capture
    Stop-DemoApp

    Start-DemoApp 520 840
    Start-Capture $dir2
    Add-Caption 'Even tiny windows keep every control reachable via More' 3
    if (-not (Wait-Element 'More' 10)) { Fail 'More pod missing at 520px' }
    Click-Element $script:Hwnd 'More'
    if (-not (Wait-Element ([string][char]0x21B6 + ' Undo') 5)) { Fail 'Undo row missing from More menu at 520px' }
    Invoke-Dwell 2.5
    Click-Element $script:Hwnd 'More'
    Save-Captions $dir2
    Stop-Capture
    Stop-DemoApp

    # Merge: dir1's frames keep their numbers; dir2's continue the count.
    # The two runs are two different window widths (760px/520px), so their
    # captured PNGs are two different pixel resolutions - pad every frame
    # onto one common canvas (the max width/height of the two runs) as a
    # real file on disk before the merge, not as an ffmpeg -vf filter (see
    # Copy-FramePadded for why an in-graph pad doesn't work here).
    $merged = Join-Path $seg 'frames'
    Remove-Item -Recurse -Force $merged -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $merged | Out-Null
    $dir1Frames = Get-ChildItem (Join-Path $dir1 'frames') -Filter 'frame_*.png' | Sort-Object Name
    $dir2Frames = Get-ChildItem (Join-Path $dir2 'frames') -Filter 'frame_*.png' | Sort-Object Name
    $size1 = Get-ImageSize $dir1Frames[0].FullName
    $size2 = Get-ImageSize $dir2Frames[0].FullName
    $canvasW = [Math]::Max($size1.W, $size2.W)
    $canvasH = [Math]::Max($size1.H, $size2.H)
    foreach ($f in $dir1Frames) { Copy-FramePadded $f.FullName (Join-Path $merged $f.Name) $canvasW $canvasH }
    $n = $dir1Frames.Count
    $i = $n
    foreach ($f in $dir2Frames) {
        Copy-FramePadded $f.FullName (Join-Path $merged ("frame_{0:D5}.png" -f $i)) $canvasW $canvasH
        $i++
    }

    # Merge captions.json, offsetting run2's timestamps by run1's duration
    # (frame count / 10fps - same arithmetic Write-DemoSrt uses).
    $caps1 = Read-Captions $dir1
    $caps2 = Read-Captions $dir2
    $offset = $n / 10.0
    $mergedCaps = @()
    foreach ($c in $caps1) { $mergedCaps += $c }
    foreach ($c in $caps2) {
        $mergedCaps += [pscustomobject]@{
            start = $c.start + $offset
            end   = $c.end + $offset
            min   = $c.min
            text  = $c.text
        }
    }
    Write-Utf8NoBom (Join-Path $seg 'captions.json') ($mergedCaps | ConvertTo-Json)
}

function Publish-Demo {
    $docs = Join-Path $RepoRoot 'docs\demo'
    New-Item -ItemType Directory -Force $docs | Out-Null
    $names = '01-propose', '02-decide', '03-edit', '04-navigate', '05-sessions', '06-export', '07-themes', '08-responsive'
    foreach ($n in $names) { Copy-Item (Join-Path $WorkDir "$n.gif") (Join-Path $docs "$n.gif") -Force }
    # MP4: per-segment captioned mp4s, then concat.
    $listFile = Join-Path $WorkDir 'concat.txt'
    Remove-Item $listFile -ErrorAction SilentlyContinue
    foreach ($n in $names) {
        $seg = Join-Path $WorkDir $n
        $mp4 = Join-Path $WorkDir "$n.mp4"
        $caps = Read-Captions $seg
        $font = 'C\:/Windows/Fonts/segoeui.ttf'
        # textfile='...', not inline text='...': the brief's original inline
        # form (text='$txt' with `-replace "'", "\\'"`) mis-escapes a literal
        # apostrophe for ffmpeg's filtergraph single-quoted value syntax
        # (closing/escaping/reopening a quote needs `'\''`, not `\'`) and
        # ffmpeg fails to parse the filter the first time a caption contains
        # one (segment 3's "agent's component"). Per-caption textfiles (same
        # fix Convert-SegmentGif already uses for the gifs) sidestep the
        # escaping question entirely.
        # $(...), not (...): a bare `foreach` statement can't be wrapped in
        # plain parens as an expression, only in a $() subexpression.
        $i = 0
        $draw = $(foreach ($c in $caps) {
                $capFile = Join-Path $seg "cap_$i.txt"
                Write-Utf8NoBom $capFile $c.text
                $capPath = ($capFile -replace '\\', '/') -replace ':', '\:'
                # Same residual fixes as Convert-SegmentGif: expansion=none
                # guards a bare '%' in a caption, and gte*lt (not the closed
                # between()) keeps caption enable-windows from double-
                # rendering on a frame-aligned boundary.
                "drawtext=fontfile='$font':textfile='$capPath':expansion=none:fontsize=28:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=12:x=(w-text_w)/2:y=h-th-24:enable='gte(t,{0})*lt(t,{1})'" -f
                [Math]::Round($c.start, 2), [Math]::Round($c.end, 2)
                $i++
            }) -join ','
        & $script:Ffmpeg -y -framerate 10 -i (Join-Path $seg 'frames\frame_%05d.png') `
            -vf "$draw,scale=1160:-2" -c:v libx264 -pix_fmt yuv420p -crf 23 $mp4
        if ($LASTEXITCODE -ne 0) { Fail "ffmpeg mp4 failed for $n" }
        Add-Content $listFile ("file '{0}'" -f ($mp4 -replace '\\', '/'))
    }
    & $script:Ffmpeg -y -f concat -safe 0 -i $listFile -c copy (Join-Path $docs 'nodestorm-demo.mp4')
    if ($LASTEXITCODE -ne 0) { Fail 'ffmpeg concat failed' }
    Write-DemoSrt $names (Join-Path $docs 'nodestorm-demo.srt')
    # Poster: a frame 4s into segment 2 (panel + ripple on screen).
    & $script:Ffmpeg -y -ss 4 -i (Join-Path $WorkDir '02-decide.mp4') -frames:v 1 (Join-Path $docs 'poster.png')
    if ($LASTEXITCODE -ne 0) { Fail 'ffmpeg poster extraction failed' }
    $total = (Get-ChildItem $docs | Measure-Object Length -Sum).Sum / 1MB
    Log ("docs\demo total: {0:N1} MB" -f $total)
}

function Write-DemoSrt([string[]]$Names, [string]$OutSrt) {
    $offset = 0.0; $idx = 1; $lines = @()
    foreach ($n in $Names) {
        $seg = Join-Path $WorkDir $n
        $caps = Read-Captions $seg
        $frameCount = (Get-ChildItem (Join-Path $seg 'frames') -Filter 'frame_*.png').Count
        foreach ($c in $caps) {
            $fmt = { param($s) ([TimeSpan]::FromSeconds($s)).ToString('hh\:mm\:ss\,fff') }
            $lines += $idx; $idx++
            $lines += ('{0} --> {1}' -f (& $fmt ($offset + $c.start)), (& $fmt ($offset + $c.end)))
            $lines += $c.text; $lines += ''
        }
        $offset += $frameCount / 10.0
    }
    Write-Utf8NoBom $OutSrt (($lines -join "`r`n") + "`r`n")
}

New-Item -ItemType Directory -Force $WorkDir | Out-Null

# Segment number -> (invoke function, gif base name). Segments 2-7 reuse the
# app + agent segment 1 started (the graph and session state accumulate:
# decisions in 2, edits in 3, the search/minimap state in 4, etc.) -
# Invoke-Segment1 is the only one that (re)launches the app, and Invoke-
# Segment8 manages its own two fresh relaunches internally.

# A plain hashtable, NOT [ordered]@{} - OrderedDictionary's indexer treats an
# Int32 argument as a 0-based positional index rather than a key lookup, so
# $SegmentTable[1] on an [ordered] table would silently return the entry for
# key 2. A plain Hashtable indexes Int32 keys correctly.
$SegmentTable = @{
    1 = @{ Fn = 'Invoke-Segment1'; Name = '01-propose' }
    2 = @{ Fn = 'Invoke-Segment2'; Name = '02-decide' }
    3 = @{ Fn = 'Invoke-Segment3'; Name = '03-edit' }
    4 = @{ Fn = 'Invoke-Segment4'; Name = '04-navigate' }
    5 = @{ Fn = 'Invoke-Segment5'; Name = '05-sessions' }
    6 = @{ Fn = 'Invoke-Segment6'; Name = '06-export' }
    7 = @{ Fn = 'Invoke-Segment7'; Name = '07-themes' }
    8 = @{ Fn = 'Invoke-Segment8'; Name = '08-responsive' }
}

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
    $requested = if ($all) { 1..8 } else { $Segment | Sort-Object -Unique }
    # Segments 2-8 need the prior segments' app/session state to exist (the
    # graph isn't proposed, decisions aren't made, etc. otherwise) - a
    # `-Segment N` run for N > 1 replays every segment up to N for real
    # (capture included), then only gif-converts the ones actually
    # requested. This documents/implements the "replay prerequisites"
    # option the brief called out, rather than requiring a full run for
    # anything past segment 1.
    $lastRequested = ($requested | Measure-Object -Maximum).Maximum
    foreach ($n in 1..$lastRequested) {
        & $SegmentTable[$n].Fn
        if ($requested -contains $n) {
            $base = $SegmentTable[$n].Name
            Convert-SegmentGif (Join-Path $WorkDir $base) (Join-Path $WorkDir "$base.gif")
        }
    }
    if ($Publish) { Publish-Demo }
} finally {
    Stop-DemoApp
}
