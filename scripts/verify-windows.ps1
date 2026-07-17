# verify-windows.ps1 - automated GUI verification of nodestorm on Windows.
#
# Windows counterpart of the README's Xvfb/scrot recipe, plus real interaction:
# it launches the app, locates elements through UI Automation (WebView2 exposes
# the DOM as a UIA tree), clicks them by posting WM_LBUTTON* messages to the
# WebView2 render widget, and captures window screenshots via PrintWindow.
# Everything is window-targeted: no cursor movement, no foreground stealing -
# safe to run while a human uses the desktop, and the window itself is pushed
# to the bottom of the z-order right after launch.
#
# Default (full) mode drives the complete human-in-the-loop protocol against
# the `drive` example agent:
#   app up -> drive proposes webhook graph -> script "user" clicks through both
#   open choices -> autoflush delivers decisions -> drive prints them and exits
# and fails unless the drive client actually received the decisions.
#
#   powershell -File scripts\verify-windows.ps1            # full E2E
#   powershell -File scripts\verify-windows.ps1 -DemoShot  # render smoke test
#
# Artifacts (screenshots, logs) land in target\verify\.

#Requires -Version 5.1
[CmdletBinding()]
param(
    [int]$Port = 4799,
    [switch]$DemoShot,      # only launch --demo, verify render, screenshot
    [switch]$NoBuild,       # skip cargo build (use existing target\debug)
    [switch]$KeepOpen,      # leave the app running afterwards
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot 'target\verify' }
$SessionFile = Join-Path $env:TEMP "nodestorm-verify-session-$Port.json"
$ExportFile = $SessionFile -replace '\.json$', '.export.md'

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -Namespace NodestormVerify -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
[DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EnumProc cb, IntPtr lp);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
[DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
public delegate bool EnumProc(IntPtr h, IntPtr lp);
public struct RECT { public int Left, Top, Right, Bottom; }
'@
[void][NodestormVerify.Native]::SetProcessDPIAware()

function Log([string]$msg) {
    Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $msg)
}

function Fail([string]$msg) {
    Write-Host "FAIL: $msg" -ForegroundColor Red
    exit 1
}

# ---------- UIA helpers ----------

$script:AppWindow = $null   # cached UIA element for the app's top-level window

function Get-AppWindow([int]$ProcessId, [int]$TimeoutSec = 30) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcessId)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
        if ($win) { return $win }
        Start-Sleep -Milliseconds 300
    }
    return $null
}

function Find-Element([string]$Name) {
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    try {
        return $script:AppWindow.FindFirst(
            [System.Windows.Automation.TreeScope]::Descendants, $cond)
    } catch {
        return $null
    }
}

function Wait-Element([string]$Name, [int]$TimeoutSec = 15) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $el = Find-Element $Name
        if ($el) { return $el }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

function Wait-ElementGone([string]$Name, [int]$TimeoutSec = 15) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        if (-not (Find-Element $Name)) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

# ---------- input + capture helpers ----------

function Get-RenderWidget([IntPtr]$TopHwnd) {
    # The WebView2 child window that receives mouse input.
    $script:rwHwnd = [IntPtr]::Zero
    $cb = [NodestormVerify.Native+EnumProc]{ param($h, $lp)
        $c = New-Object System.Text.StringBuilder 256
        [void][NodestormVerify.Native]::GetClassName($h, $c, 256)
        if ($c.ToString() -eq 'Chrome_RenderWidgetHostHWND') {
            $script:rwHwnd = $h
            return $false
        }
        $true
    }
    [void][NodestormVerify.Native]::EnumChildWindows($TopHwnd, $cb, [IntPtr]::Zero)
    return $script:rwHwnd
}

function Click-Element([IntPtr]$TopHwnd, [string]$Name) {
    # Click = WM_MOUSEMOVE + WM_LBUTTONDOWN/UP posted to the render widget at
    # the element's center (client coordinates). Coordinates travel inside the
    # messages, so a human moving the real mouse cannot deflect the click.
    $el = Find-Element $Name
    if (-not $el) { Fail "element to click not found: '$Name'" }
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
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0201, [IntPtr]1, $lp)      # WM_LBUTTONDOWN
    Start-Sleep -Milliseconds 50
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0202, [IntPtr]::Zero, $lp) # WM_LBUTTONUP
    Log "clicked '$Name' (client $cx,$cy)"
}

function Click-Point([IntPtr]$TopHwnd, [double]$ScreenX, [double]$ScreenY) {
    # Click at absolute screen coordinates (for elements UIA can't name,
    # e.g. text inputs) — same window-targeted WM_LBUTTON* posting.
    $rw = Get-RenderWidget $TopHwnd
    if ($rw -eq [IntPtr]::Zero) { Fail 'WebView2 render widget window not found' }
    $rwRect = New-Object NodestormVerify.Native+RECT
    [void][NodestormVerify.Native]::GetWindowRect($rw, [ref]$rwRect)
    $cx = [int]$ScreenX - $rwRect.Left
    $cy = [int]$ScreenY - $rwRect.Top
    $lp = [IntPtr](($cy -shl 16) -bor ($cx -band 0xFFFF))
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0200, [IntPtr]::Zero, $lp)
    Start-Sleep -Milliseconds 50
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0201, [IntPtr]1, $lp)
    Start-Sleep -Milliseconds 50
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0202, [IntPtr]::Zero, $lp)
    Log "clicked point (client $cx,$cy)"
}

function Send-Key([IntPtr]$TopHwnd, [int]$VirtualKey) {
    # WM_KEYDOWN/WM_KEYUP posted to the render widget — never the real
    # keyboard, so a human typing elsewhere is unaffected.
    $rw = Get-RenderWidget $TopHwnd
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0100, [IntPtr]$VirtualKey, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 30
    [void][NodestormVerify.Native]::PostMessageW($rw, 0x0101, [IntPtr]$VirtualKey, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 30
}

function Type-Text([IntPtr]$TopHwnd, [string]$Text) {
    # One WM_CHAR per character to the focused element in the render widget.
    $rw = Get-RenderWidget $TopHwnd
    foreach ($ch in $Text.ToCharArray()) {
        [void][NodestormVerify.Native]::PostMessageW($rw, 0x0102, [IntPtr][int]$ch, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 15
    }
    Log "typed '$Text'"
}

function Save-WindowPng([IntPtr]$TopHwnd, [string]$Path) {
    $r = New-Object NodestormVerify.Native+RECT
    [void][NodestormVerify.Native]::GetWindowRect($TopHwnd, [ref]$r)
    $w = $r.Right - $r.Left
    $h = $r.Bottom - $r.Top
    if ($w -le 0 -or $h -le 0) { Fail 'window has empty rect; cannot capture' }

    function Capture {
        $bmp = New-Object System.Drawing.Bitmap $w, $h
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $g.GetHdc()
        # PW_RENDERFULLCONTENT (2): forces the DirectComposition/WebView2
        # content to render even when the window is occluded.
        [void][NodestormVerify.Native]::PrintWindow($TopHwnd, $hdc, 2)
        $g.ReleaseHdc($hdc)
        $g.Dispose()
        return $bmp
    }

    $bmp = Capture
    # Occlusion fallback: if the frame is uniform (all sample pixels equal the
    # corner pixel), raise the window without activating it and recapture.
    $p0 = $bmp.GetPixel(5, 5)
    $uniform = $true
    foreach ($pt in @(@(($w / 2), ($h / 2)), @(($w / 3), ($h / 3)), @(($w - 10), ($h - 10)), @(($w / 2), 60))) {
        if ($bmp.GetPixel([int]$pt[0], [int]$pt[1]) -ne $p0) { $uniform = $false; break }
    }
    if ($uniform) {
        Log 'capture looked blank; raising window (no activate) and retrying'
        # SWP_NOMOVE|NOSIZE|NOACTIVATE = 0x0013, insert after HWND_TOP (0)
        [void][NodestormVerify.Native]::SetWindowPos($TopHwnd, [IntPtr]::Zero, 0, 0, 0, 0, 0x0013)
        Start-Sleep -Milliseconds 500
        $bmp.Dispose()
        $bmp = Capture
    }
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Log "captured $Path"
}

function Wait-Tcp([int]$TcpPort, [int]$TimeoutSec = 60) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $client = New-Object System.Net.Sockets.TcpClient
        try {
            $client.Connect('127.0.0.1', $TcpPort)
            $client.Close()
            return $true
        } catch {
            $client.Close()
            Start-Sleep -Milliseconds 300
        }
    }
    return $false
}

# ---------- run ----------

New-Item -ItemType Directory -Force $OutDir | Out-Null
Remove-Item $SessionFile -Force -ErrorAction SilentlyContinue
Remove-Item $ExportFile -Force -ErrorAction SilentlyContinue

if (-not $NoBuild) {
    Log 'cargo build (nodestorm + drive example)...'
    Push-Location $RepoRoot
    try {
        & cargo build --bin nodestorm --example drive
        if ($LASTEXITCODE -ne 0) { Fail "cargo build exited $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

$exe = Join-Path $RepoRoot 'target\debug\nodestorm.exe'
$driveExe = Join-Path $RepoRoot 'target\debug\examples\drive.exe'
if (-not (Test-Path $exe)) { Fail "$exe not built" }
if (-not $DemoShot -and -not (Test-Path $driveExe)) { Fail "$driveExe not built" }

$appArgs = @('--port', $Port, '--session', $SessionFile)
if ($DemoShot) { $appArgs += '--demo' }
$appLog = Join-Path $OutDir 'nodestorm.log'
Log "launching nodestorm on port $Port..."
$app = Start-Process -FilePath $exe -ArgumentList $appArgs -PassThru `
    -WindowStyle Hidden -RedirectStandardOutput $appLog `
    -RedirectStandardError (Join-Path $OutDir 'nodestorm.err.log')
$drive = $null

try {
    if (-not (Wait-Tcp $Port)) { Fail "MCP port $Port never opened (see $appLog)" }
    Log 'MCP port is up'

    $script:AppWindow = Get-AppWindow $app.Id
    if (-not $script:AppWindow) { Fail 'app window did not appear in UIA' }
    $hwnd = [IntPtr]$script:AppWindow.Current.NativeWindowHandle
    # Stay out of the way of a human using the desktop: bottom of z-order,
    # no activation. All automation below is window-targeted anyway.
    # HWND_BOTTOM = 1; SWP_NOMOVE|NOSIZE|NOACTIVATE = 0x0013
    [void][NodestormVerify.Native]::SetWindowPos($hwnd, [IntPtr]1, 0, 0, 0, 0, 0x0013)

    if ($DemoShot) {
        foreach ($name in @('Sync Engine', 'Send to agent', '2 open decisions')) {
            if (-not (Wait-Element $name 15)) { Fail "demo element missing from UIA tree: '$name'" }
        }
        Log 'demo graph is rendered and exposed via UIA'
        Save-WindowPng $hwnd (Join-Path $OutDir 'demo.png')
        Write-Host 'PASS: demo render verified' -ForegroundColor Green
        exit 0
    }

    # Full E2E: drive proposes a graph, we decide both choices, autoflush
    # delivers, drive prints the decisions and exits.
    $driveOut = Join-Path $OutDir 'drive.out.log'
    $driveErr = Join-Path $OutDir 'drive.err.log'
    Log 'starting drive example (agent simulator)...'
    $drive = Start-Process -FilePath $driveExe -ArgumentList "http://127.0.0.1:$Port/mcp" `
        -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $driveOut -RedirectStandardError $driveErr

    if (-not (Wait-Element 'Webhook Dispatcher' 30)) {
        Fail "drive's proposed graph never appeared (see $driveErr)"
    }
    Log 'proposed graph is on the canvas'
    Save-WindowPng $hwnd (Join-Path $OutDir '01-proposed.png')

    Click-Element $hwnd 'Webhook Dispatcher'
    if (-not (Wait-Element 'At-least-once with retries' 10)) { Fail 'choice panel did not open' }
    Click-Element $hwnd 'At-least-once with retries'
    if (-not (Wait-Element '1 to send' 10)) { Fail "decision was not recorded ('1 to send' pill missing)" }
    Log 'first decision recorded'

    # The choice panel overlays the right edge of the canvas and can cover the
    # next card (UIA still reports the covered card's rect, but a click there
    # lands on the panel). Close it before selecting another node.
    Click-Element $hwnd ([string][char]0x2715)   # panel close button
    if (-not (Wait-ElementGone 'At-least-once with retries' 10)) { Fail 'choice panel did not close' }

    Click-Element $hwnd 'Delivery Store'
    if (-not (Wait-Element 'Existing PostgreSQL' 10)) { Fail 'second choice panel did not open' }
    Click-Element $hwnd 'Existing PostgreSQL'
    # Last open choice decided -> autoflush -> await_decisions delivers.
    Log 'second decision made; waiting for drive to receive the delivery...'

    if (-not $drive.WaitForExit(60000)) { Fail "drive did not exit within 60s (see $driveOut)" }
    Start-Sleep -Milliseconds 700   # let the delivered/flushed state repaint
    Save-WindowPng $hwnd (Join-Path $OutDir '02-decided.png')

    $out = Get-Content $driveOut -Raw
    if ($out -notmatch '"status": "delivered"') { Fail "drive never saw a delivery:`n$out" }
    foreach ($expect in @('"option_id": "at-least-once"', '"option_id": "postgres"')) {
        if ($out -notlike "*$expect*") { Fail "delivered decisions missing $expect :`n$out" }
    }
    Log 'drive received both decisions (at-least-once, postgres)'

    # ---- v0.3 user editing through the real controls ----

    # Add a user component; the panel opens on it.
    Click-Element $hwnd '+ Component'
    if (-not (Wait-Element 'New component' 10)) { Fail 'user component did not appear' }

    # Rename it via the panel's Edit form: expand, focus the label input
    # (just below the Edit summary — inputs have no reliable UIA name),
    # clear, type, save.
    Click-Element $hwnd 'Edit'
    Start-Sleep -Milliseconds 300
    $editEl = Find-Element 'Edit'
    if (-not $editEl) { Fail 'Edit summary not found after expanding' }
    $er = $editEl.Current.BoundingRectangle
    Click-Point $hwnd ($er.X + $er.Width / 2) ($er.Y + $er.Height + 18)
    Send-Key $hwnd 0x23   # VK_END
    for ($i = 0; $i -lt 20; $i++) { Send-Key $hwnd 0x08 }   # Backspace × 20
    Type-Text $hwnd 'Rate Limiter'
    Click-Element $hwnd 'Save'
    if (-not (Wait-Element 'Rate Limiter' 10)) { Fail 'rename did not land on the card' }
    Log 'user component added and renamed'

    # Connect it to an agent node via the panel's Connect flow.
    if (-not (Wait-Element ('Connect ' + [char]0x2192) 5)) { Fail 'Connect button missing' }
    Click-Element $hwnd ('Connect ' + [char]0x2192)
    Click-Element $hwnd 'Webhook Dispatcher'
    $connRow = 'Rate Limiter ' + [char]0x2014 + 'depends on' + [char]0x2192 + ' Webhook Dispatcher'
    if (-not (Wait-Element $connRow 10)) { Fail "connection row missing: '$connRow'" }
    Log 'user edge drawn via Connect flow'

    # Soft-delete an agent-authored node: card gets status "removed".
    Click-Element $hwnd ([string][char]0x2715)   # close the open panel first
    Start-Sleep -Milliseconds 300
    Click-Element $hwnd 'Delivery Store'
    if (-not (Wait-Element 'Delete' 10)) { Fail 'panel Delete button missing' }
    Click-Element $hwnd 'Delete'
    if (-not (Wait-Element 'removed' 10)) { Fail 'agent node was not marked removed' }
    Log 'agent node soft-removed (removal_requested queued)'
    Save-WindowPng $hwnd (Join-Path $OutDir '03-edited.png')

    # Export via the menu: the record must include the user edits.
    Click-Element $hwnd ('Export ' + [char]0x25BE)
    if (-not (Wait-Element 'Copy Markdown' 5)) { Fail 'export menu did not open' }
    Click-Element $hwnd 'Export'
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline -and -not (Test-Path $ExportFile)) { Start-Sleep -Milliseconds 250 }
    if (-not (Test-Path $ExportFile)) { Fail "export file did not appear: $ExportFile" }
    $record = Get-Content $ExportFile -Raw
    foreach ($needle in @('```mermaid', '# Add a webhook subsystem', 'At-least-once with retries',
            'Rate Limiter', 'Delivery Store')) {
        if ($record -notlike "*$needle*") { Fail "export record missing '$needle':`n$record" }
    }
    Log "Export menu wrote the decision record ($ExportFile)"
    Write-Host 'PASS: decisions + user editing verified through the real GUI' -ForegroundColor Green
    Write-Host "artifacts: $OutDir"
    exit 0
} finally {
    if ($drive -and -not $drive.HasExited) { Stop-Process -Id $drive.Id -Force -ErrorAction SilentlyContinue }
    if (-not $KeepOpen -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item $SessionFile -Force -ErrorAction SilentlyContinue
    Remove-Item $ExportFile -Force -ErrorAction SilentlyContinue
}
