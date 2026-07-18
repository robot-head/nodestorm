# uia-lib.ps1 - shared window-targeted UIA/automation helpers for nodestorm
# scripts (verify-windows.ps1, record-demo.ps1). Everything is PostMessage/
# PrintWindow against the app's own windows: no cursor, no foreground, no
# full-screen capture. Dot-source this file; it defines functions and the
# NodestormVerify.Native type in the caller's scope.
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
if (-not ('NodestormVerify.Native' -as [type])) {
Add-Type -Namespace NodestormVerify -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
[DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EnumProc cb, IntPtr lp);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
[DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
[DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out int val, int size);
public delegate bool EnumProc(IntPtr h, IntPtr lp);
public struct RECT { public int Left, Top, Right, Bottom; }
'@
}
[void][NodestormVerify.Native]::SetProcessDPIAware()

function Log([string]$msg) {
    Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $msg)
}

function Fail([string]$msg) {
    Write-Host "FAIL: $msg" -ForegroundColor Red
    exit 1
}

$script:AppWindow = $null   # cached UIA element for the app's top-level window

function Get-AppWindow([int]$ProcessId, [int]$TimeoutSec = 30) {
    # The process owns two top-level windows: the real one ("nodestorm") and
    # tao's hidden "Tao Thread Event Target". Enumeration order is not
    # guaranteed, so pick by name instead of taking the first match.
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcessId)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        foreach ($win in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)) {
            if ($win.Current.Name -eq 'nodestorm') { return $win }
        }
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
