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
# Isolated named-sessions dir: without this, created sessions land in the
# user's real data dir and dedup renames break re-runs.
$SessionsDir = Join-Path $env:TEMP "nodestorm-verify-sessions-$Port"
# Isolated preferences file: the theme step must not touch the user's real
# preferences.json.
$PrefsFile = Join-Path $env:TEMP "nodestorm-verify-prefs-$Port.json"

. (Join-Path $PSScriptRoot 'uia-lib.ps1')

# ---------- run ----------

New-Item -ItemType Directory -Force $OutDir | Out-Null
Remove-Item $SessionFile -Force -ErrorAction SilentlyContinue
Remove-Item $ExportFile -Force -ErrorAction SilentlyContinue
Remove-Item $SessionsDir -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $PrefsFile -Force -ErrorAction SilentlyContinue

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

$appArgs = @('--port', $Port, '--session', $SessionFile, '--sessions-dir', $SessionsDir, '--prefs', $PrefsFile)
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

    # Rename it via the panel's Edit form: open the form, focus the label
    # input (just below the actions row — inputs have no reliable UIA name),
    # clear, type, save.
    if (-not (Wait-Element 'Edit' 10)) { Fail 'panel did not open with an Edit button' }
    Click-Element $hwnd 'Edit'
    Start-Sleep -Milliseconds 300
    $editEl = Find-Element 'Edit'
    if (-not $editEl) { Fail 'Edit button vanished after opening the form' }
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
    # The status tag renders lowercase but CSS text-transform: uppercase
    # changes the UIA-exposed name.
    if (-not (Wait-Element 'REMOVED' 10)) { Fail 'agent node was not marked removed' }
    Log 'agent node soft-removed (removal_requested queued)'

    # v0.6: undo restores the node's status; redo re-marks it.
    Click-Element $hwnd ([string][char]0x21B6 + ' Undo')
    if (-not (Wait-ElementGone 'REMOVED' 10)) { Fail 'undo did not clear the removal' }
    Click-Element $hwnd ([string][char]0x21B7 + ' Redo')
    if (-not (Wait-Element 'REMOVED' 10)) { Fail 'redo did not re-mark the removal' }
    Log 'undo/redo round-trip on the soft-delete'
    Save-WindowPng $hwnd (Join-Path $OutDir '03-edited.png')

    # ---- v0.4 sessions + timeline through the real controls ----

    # Create a second session, land in its empty state, switch back, and the
    # original graph must return intact.
    $SessionStem = [IO.Path]::GetFileNameWithoutExtension($SessionFile)
    Click-Element $hwnd ($SessionStem + ' ' + [char]0x25BE)
    if (-not (Wait-Element 'Create' 5)) { Fail 'sessions menu did not open' }
    Click-Element $hwnd ('new session' + [char]0x2026)
    Type-Text $hwnd 'scratch'
    Click-Element $hwnd 'Create'
    if (-not (Wait-Element 'Waiting for an agent to connect.' 10)) {
        Fail 'created session did not become the active empty canvas'
    }
    Log 'created and switched to session "scratch"'
    if (-not (Wait-Element ('scratch ' + [char]0x25BE) 10)) {
        Fail 'switcher label did not update to the new active session'
    }
    Click-Element $hwnd ('scratch ' + [char]0x25BE)
    if (-not (Wait-Element $SessionStem 5)) { Fail 'original session missing from the list' }
    Click-Element $hwnd $SessionStem
    if (-not (Wait-Element 'Webhook Dispatcher' 10)) { Fail 'original graph did not return after switching back' }
    Log 'sessions: create/switch round-trip OK'

    # v0.5: rename the active session and back (label follows both ways).
    Click-Element $hwnd ($SessionStem + ' ' + [char]0x25BE)
    if (-not (Wait-Element 'Rename' 5)) { Fail 'manage section missing' }
    Click-Element $hwnd ('rename to' + [char]0x2026)
    Type-Text $hwnd 'renamed-e2e'
    Click-Element $hwnd 'Rename'
    if (-not (Wait-Element ('renamed-e2e ' + [char]0x25BE) 10)) { Fail 'rename did not update the switcher' }
    Click-Element $hwnd ('renamed-e2e ' + [char]0x25BE)
    if (-not (Wait-Element 'Rename' 5)) { Fail 'manage section missing after rename' }
    Click-Element $hwnd ('rename to' + [char]0x2026)
    Type-Text $hwnd $SessionStem
    Click-Element $hwnd 'Rename'
    if (-not (Wait-Element ($SessionStem + ' ' + [char]0x25BE) 10)) { Fail 'rename back failed' }
    Log 'session renamed and renamed back'

    # v0.5: Compare opens the diff panel against the scratch session.
    Click-Element $hwnd ($SessionStem + ' ' + [char]0x25BE)
    if (-not (Wait-Element 'Compare' 5)) { Fail 'Compare button missing' }
    Click-Element $hwnd 'Compare'
    if (-not (Wait-Element 'Session diff' 10)) { Fail 'diff panel did not open' }
    Save-WindowPng $hwnd (Join-Path $OutDir '05-manage-diff.png')
    Click-Element $hwnd ([string][char]0x2715)
    Log 'session diff panel verified'

    # Timeline: the session-log panel opens and is captured.
    Click-Element $hwnd 'Timeline'
    if (-not (Wait-Element 'Session timeline' 5)) { Fail 'timeline panel did not open' }
    Save-WindowPng $hwnd (Join-Path $OutDir '04-sessions-timeline.png')
    Click-Element $hwnd 'Timeline'

    # Export via the menu: the record must include the user edits.
    Click-Element $hwnd 'More'
    if (-not (Wait-Element ('Export ' + [char]0x25BE) 5)) { Fail 'More menu did not open' }
    Click-Element $hwnd ('Export ' + [char]0x25BE)
    if (-not (Wait-Element 'Copy Markdown' 5)) { Fail 'export menu did not open' }
    Click-Element $hwnd 'Export'
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline -and -not (Test-Path $ExportFile)) { Start-Sleep -Milliseconds 250 }
    if (-not (Test-Path $ExportFile)) { Fail "export file did not appear: $ExportFile" }
    $record = Get-Content $ExportFile -Raw
    foreach ($needle in @('```mermaid', '# Add a webhook subsystem', 'At-least-once with retries',
            'Rate Limiter', 'Delivery Store', '## Session log')) {
        if ($record -notlike "*$needle*") { Fail "export record missing '$needle':`n$record" }
    }
    Log "Export menu wrote the decision record ($ExportFile)"

    # ---- v0.7 theming through the real controls ----

    # Open the picker; a non-active family row's UIA name is its plain
    # display name (the active row flattens to "<Name> ✓").
    Click-Element $hwnd 'More'
    if (-not (Wait-Element ('Theme ' + [char]0x25BE) 5)) { Fail 'More menu did not open' }
    Click-Element $hwnd ('Theme ' + [char]0x25BE)
    if (-not (Wait-Element 'Gruvbox' 5)) { Fail 'theme menu did not open' }
    # Mode clicks keep the menu open; family clicks close it.
    Click-Element $hwnd 'Light'
    Click-Element $hwnd 'Gruvbox'
    if (-not (Wait-ElementGone 'Gruvbox' 10)) { Fail 'theme menu did not close after picking a palette' }

    # The preference file must record the choice (written on every click).
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline -and -not (Test-Path $PrefsFile)) { Start-Sleep -Milliseconds 250 }
    if (-not (Test-Path $PrefsFile)) { Fail "preferences file did not appear: $PrefsFile" }
    $prefs = Get-Content $PrefsFile -Raw | ConvertFrom-Json
    if ($prefs.theme -ne 'gruvbox' -or $prefs.mode -ne 'light') {
        Fail "preferences not persisted (theme=$($prefs.theme), mode=$($prefs.mode))"
    }
    Log 'theme choice persisted (gruvbox, light)'

    # Visual hard-assert: gruvbox-light content is unambiguously bright
    # (the dark UI averages ~0.1 on the same grid).
    Start-Sleep -Milliseconds 500
    $themeShot = Join-Path $OutDir '06-theme-gruvbox-light.png'
    Save-WindowPng $hwnd $themeShot
    $bmp = New-Object System.Drawing.Bitmap $themeShot
    $sum = 0.0; $n = 0
    for ($x = 50; $x -lt $bmp.Width - 50; $x += 20) {
        for ($y = 60; $y -lt $bmp.Height - 50; $y += 20) {
            $sum += $bmp.GetPixel($x, $y).GetBrightness(); $n++
        }
    }
    $bmp.Dispose()
    $avg = $sum / $n
    if ($avg -le 0.5) { Fail ("content did not switch to a light palette (mean brightness {0:N2})" -f $avg) }
    Log ("gruvbox-light content verified (mean brightness {0:N2})" -f $avg)

    # ---- v0.8: at a narrow window nothing in the topbar overflows ----
    # 760 physical px: hits the <=780px (logical) compose breakpoint at 100%
    # DPI and deeper folds on scaled displays; either way Send must fit.
    [void][NodestormVerify.Native]::SetWindowPos($hwnd, [IntPtr]1, 0, 0, 760, 840, 0x0012)
    Start-Sleep -Milliseconds 800
    $send = Wait-Element 'Send to agent' 5
    if (-not $send) { Fail 'Send to agent missing from UIA at 760px' }
    $sr = $send.Current.BoundingRectangle
    $wr = $script:AppWindow.Current.BoundingRectangle
    if ($sr.Right -gt $wr.Right) {
        Fail "Send button overflows the window at 760px (send.Right=$($sr.Right) window.Right=$($wr.Right))"
    }
    if (-not (Wait-Element 'Message to agent' 5)) { Fail 'compose pod did not appear at 760px' }
    Save-WindowPng $hwnd (Join-Path $OutDir '07-narrow-760.png')
    Log 'narrow-window topbar fit verified (760px)'

    # At <=560px (logical) the Undo/Redo pods fold into the More menu.
    [void][NodestormVerify.Native]::SetWindowPos($hwnd, [IntPtr]1, 0, 0, 520, 840, 0x0012)
    Start-Sleep -Milliseconds 800
    # An occluded (bottom z-order) WebView2 commits at most one out-of-band
    # resize per app lifetime: this second one shrinks the native windows but
    # the page keeps the 760px layout, so UIA rect centers can lie beyond the
    # shrunken render widget, where posted clicks are dropped unseen. Clamp
    # the click x into the widget - the More pod straddles the stale edge, so
    # the clamped point still lands on it (and is a no-op on fresh layouts).
    $more = Wait-Element 'More' 5
    if (-not $more) { Fail 'More pod missing from UIA at 520px' }
    $mr = $more.Current.BoundingRectangle
    $rwh = Get-RenderWidget $hwnd
    if ($rwh -eq [IntPtr]::Zero) { Fail 'WebView2 render widget window not found' }
    $rwr = New-Object NodestormVerify.Native+RECT
    [void][NodestormVerify.Native]::GetWindowRect($rwh, [ref]$rwr)
    $mx = [Math]::Min($mr.X + $mr.Width / 2, $rwr.Right - 12)
    $my = $mr.Y + $mr.Height / 2
    Click-Point $hwnd $mx $my
    if (-not (Wait-Element ([string][char]0x21B6 + ' Undo') 5)) { Fail 'Undo row missing from More menu at 520px' }
    Save-WindowPng $hwnd (Join-Path $OutDir '08-narrow-520-more.png')
    Click-Point $hwnd $mx $my
    Log 'narrow-window More-menu undo/redo fallback verified (520px)'

    [void][NodestormVerify.Native]::SetWindowPos($hwnd, [IntPtr]1, 0, 0, 1280, 840, 0x0012)
    Start-Sleep -Milliseconds 400

    # Native title bar follows the mode: after clicking Light the DWM
    # immersive-dark flag must be off. (Pixel checks are unreliable here —
    # accent-on-title-bars paints active bars the accent color either way.)
    $dark = 0
    $hr = [NodestormVerify.Native]::DwmGetWindowAttribute($hwnd, 20, [ref]$dark, 4)
    if ($hr -ne 0) { Fail "DwmGetWindowAttribute failed (hr=$hr)" }
    if ($dark -ne 0) { Fail 'native title bar still dark after switching the mode to Light' }
    Log 'native title bar switched to light with the mode'

    Write-Host 'PASS: decisions + user editing verified through the real GUI' -ForegroundColor Green
    Write-Host "artifacts: $OutDir"
    exit 0
} finally {
    if ($drive -and -not $drive.HasExited) { Stop-Process -Id $drive.Id -Force -ErrorAction SilentlyContinue }
    if (-not $KeepOpen -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item $SessionFile -Force -ErrorAction SilentlyContinue
    Remove-Item $ExportFile -Force -ErrorAction SilentlyContinue
    Remove-Item $SessionsDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $PrefsFile -Force -ErrorAction SilentlyContinue
}
