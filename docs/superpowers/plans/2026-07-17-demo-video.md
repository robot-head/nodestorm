# Demo Video + Mermaid Architecture Diagram Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A paced, subtitled 8-segment demo (GIFs embedded in README + full MP4 in docs/demo/) recorded via window-scoped automation, plus the README ASCII diagram converted to mermaid architecture-beta.

**Architecture:** A dot-sourceable UIA library extracted from the E2E script powers a new recorder (`scripts/record-demo.ps1`) that drives the app with posted messages while a background runspace captures `PrintWindow PW_RENDERFULLCONTENT` frames at 10fps; a scripted agent (`examples/demo_agent.rs`) plays the MCP side. Captions are recorded as a timeline while driving and burned in with ffmpeg `drawtext`. Per-size launches (new `--window-size` flag) avoid the WebView2 occluded-resize freeze entirely.

**Tech Stack:** PowerShell 5.1+ (UIA, GDI), Rust/rmcp (agent example), ffmpeg (winget Gyan.FFmpeg), mermaid architecture-beta.

**Spec:** `docs/superpowers/specs/2026-07-17-demo-video-design.md`

## Global Constraints

- Machine in active human use: window-targeted automation ONLY (UIA + PostMessage + PrintWindow on the app's own windows). The recorder must never contain SendInput, SetForegroundWindow, cursor movement, or full-screen capture. App windows go to HWND_BOTTOM right after launch.
- Never resize the app window after launch — every segment size is a fresh launch with `--window-size` (WebView2 occluded-resize freeze, docs/webview2-occluded-resize.md).
- Recorder isolation: port **4801**, sessions dir `%TEMP%\nodestorm-demo-sessions`, prefs `%TEMP%\nodestorm-demo-prefs.json` (never the user's data or port 4747; never verify-windows' 4799 scratch either).
- Pacing: dwell 1.5–3 s after each visual payoff; no caption displayed < 2.5 s.
- Outputs: `docs/demo/01…08-*.gif` each ≤ 4 MB (degrade 10→8 fps, then 800→720 px, in that order), `docs/demo/nodestorm-demo.mp4` (H.264 yuv420p) + `nodestorm-demo.srt` + `poster.png`; total `docs/demo/` ≤ 35 MB.
- After every task: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` clean. Full E2E (`powershell -File scripts\verify-windows.ps1`) green in Tasks 1 and 6.
- Never edit the `[data-theme]` CSS blocks; no app-code changes beyond `examples/demo_agent.rs` and the `--window-size` flag.
- Branch: `claude/ui-redesign-button-overflow-f1beb5` (continues PR #9).

---

### Task 1: Extract `scripts/uia-lib.ps1` (E2E-gated refactor)

**Files:**
- Create: `scripts/uia-lib.ps1`
- Modify: `scripts/verify-windows.ps1`

**Interfaces:**
- Produces (unchanged signatures, now dot-sourceable): `Log`, `Fail`, `Get-AppWindow`, `Find-Element`, `Wait-Element`, `Wait-ElementGone`, `Get-RenderWidget`, `Click-Element`, `Click-Point`, `Save-WindowPng`, `Wait-Tcp`, the WM_CHAR typing helper, the `[NodestormVerify.Native]` type, and script-scope state `$script:AppWindow`.

- [ ] **Step 1: Create the library.** Move, verbatim, from `verify-windows.ps1` into a new `scripts/uia-lib.ps1`: the three `Add-Type -AssemblyName` lines, the whole `Add-Type -Namespace NodestormVerify …` member block plus the `SetProcessDPIAware` call (lines 44–59), `Log`, `Fail`, `$script:AppWindow = $null`, and every helper function through the input/capture section (`Get-AppWindow`, `Find-Element`, `Wait-Element`, `Wait-ElementGone`, `Get-RenderWidget`, `Click-Element`, `Click-Point`, `Save-WindowPng` with its occlusion-fallback logic, `Wait-Tcp`, and the WM_CHAR-posting typing helper — identify it by its `0x0102` PostMessageW calls). Top of the new file gets:

```powershell
# uia-lib.ps1 - shared window-targeted UIA/automation helpers for nodestorm
# scripts (verify-windows.ps1, record-demo.ps1). Everything is PostMessage/
# PrintWindow against the app's own windows: no cursor, no foreground, no
# full-screen capture. Dot-source this file; it defines functions and the
# NodestormVerify.Native type in the caller's scope.
if (-not ('NodestormVerify.Native' -as [type])) {
```

wrapping ONLY the `Add-Type -Namespace` block (so double dot-sourcing can't fail on a duplicate type), closing `}` right after it; the `Add-Type -AssemblyName` lines are idempotent and stay unwrapped.

- [ ] **Step 2: Dot-source from the E2E script.** In `verify-windows.ps1`, where the moved code was, leave exactly:

```powershell
. (Join-Path $PSScriptRoot 'uia-lib.ps1')
```

(after the `$PrefsFile` assignment, before the first remaining function). Delete nothing else; every remaining line stays byte-identical.

- [ ] **Step 3: Parse check.**

Run: `powershell -NoProfile -Command "[void][System.Management.Automation.Language.Parser]::ParseFile('scripts/verify-windows.ps1',[ref]$null,[ref]$e); $e.Count; [void][System.Management.Automation.Language.Parser]::ParseFile('scripts/uia-lib.ps1',[ref]$null,[ref]$e2); $e2.Count"`
Expected: `0` twice.

- [ ] **Step 4: E2E gate.**

Run: `powershell -File scripts\verify-windows.ps1` (full mode)
Expected: PASS, exit 0 — identical behavior to before the refactor. Also run `-DemoShot` → PASS.

- [ ] **Step 5: Commit.**

```powershell
git add scripts/uia-lib.ps1 scripts/verify-windows.ps1
git commit -m "refactor(e2e): extract shared UIA helpers into scripts/uia-lib.ps1"
```

---

### Task 2: `--window-size` flag

**Files:**
- Modify: `src/cli.rs` (field + parser + tests), `src/ui/mod.rs:131` (launch size), `README.md` (CLI table)

**Interfaces:**
- Produces: `Cli.window_size: Option<(f64, f64)>` (logical px); default behavior identical when absent.

- [ ] **Step 1: Failing tests.** In `src/cli.rs` `mod tests`, add:

```rust
    #[test]
    fn window_size_parses_and_validates() {
        let cli = Cli::parse_from(["nodestorm", "--window-size", "760x840"]);
        assert_eq!(cli.window_size, Some((760.0, 840.0)));
        assert_eq!(Cli::parse_from(["nodestorm"]).window_size, None);
        assert!(Cli::try_parse_from(["nodestorm", "--window-size", "760"]).is_err());
        assert!(Cli::try_parse_from(["nodestorm", "--window-size", "10x840"]).is_err());
        assert!(Cli::try_parse_from(["nodestorm", "--window-size", "axb"]).is_err());
    }
```

- [ ] **Step 2: Run to verify failure.** `cargo test window_size` → FAIL (no field `window_size`).

- [ ] **Step 3: Implement.** In the `Cli` struct after `pub headless: bool,`:

```rust
    /// Initial window size in logical pixels, `WIDTHxHEIGHT` (default
    /// 1280x840). Lets demo recordings and E2E runs launch at a target
    /// size instead of resizing a running window.
    #[arg(long, value_name = "WxH", value_parser = parse_window_size)]
    pub window_size: Option<(f64, f64)>,
```

and below the struct:

```rust
/// Parse `760x840` into logical (width, height); both in 200..=10000.
fn parse_window_size(s: &str) -> Result<(f64, f64), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("expected WIDTHxHEIGHT, e.g. 760x840, got '{s}'"))?;
    let dim = |v: &str, name: &str| -> Result<f64, String> {
        let n: f64 = v
            .trim()
            .parse()
            .map_err(|_| format!("{name} is not a number in '{s}'"))?;
        if (200.0..=10_000.0).contains(&n) {
            Ok(n)
        } else {
            Err(format!("{name} out of range 200..=10000 in '{s}'"))
        }
    };
    Ok((dim(w, "width")?, dim(h, "height")?))
}
```

In `src/ui/mod.rs`, replace the fixed size line:

```rust
        .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(1280.0, 840.0))
```

with:

```rust
        .with_inner_size({
            let (w, h) = cli.window_size.unwrap_or((1280.0, 840.0));
            dioxus::desktop::tao::dpi::LogicalSize::new(w, h)
        })
```

(`cli` is in scope in `launch`; the closure-free block avoids a borrow issue since `window_size` is `Copy` inside `Option`.)

- [ ] **Step 4: Tests pass.** `cargo test` → green; `cargo clippy --all-targets -- -D warnings`; `cargo fmt`.

- [ ] **Step 5: README CLI table row.** After the `--headless` row:

```markdown
| `--window-size <WxH>` | initial window size in logical px (default `1280x840`) |
```

- [ ] **Step 6: Commit.**

```powershell
git add src/cli.rs src/ui/mod.rs README.md
git commit -m "feat(cli): --window-size flag for launch-time window sizing"
```

---

### Task 3: `examples/demo_agent.rs`

**Files:**
- Create: `examples/demo_agent.rs`
- Reference (read, don't modify): `examples/drive.rs`, `src/server/tools.rs`

**Interfaces:**
- Consumes: MCP tools `propose_graph`, `await_decisions`, `update_graph` on `http://127.0.0.1:4801/mcp` (URL argv-overridable like drive.rs).
- Produces: node labels/ids later clicked by name in Task 5 — **exactly**: `Web Client`(web), `Notes API`(api), `Auth Service`(auth), `Sync Engine`(sync-engine), `Presence Service`(presence), `WebSocket Gateway`(ws), `Notes Store`(storage), `Search Index`(search), `Job Queue`(jobs), `Email Provider`(mail); group `sync` on sync-engine/presence/ws; choice prompts `Conflict resolution strategy` (on sync-engine; options `CRDT document model` ★, affects storage+presence / `Operational transforms`, affects ws) and `Edit history storage` (on storage; options `Append-only event log` ★, affects sync-engine+search / `Periodic snapshots`).

- [ ] **Step 1: Write the example.** Model on `examples/drive.rs` (same imports, transport setup, call/await loop). Body:

```rust
//! Scripted agent for the README demo recording (driven by
//! scripts/record-demo.ps1): proposes a realtime-notes architecture with a
//! `sync` group and two rippling choices, reacts once to the first
//! delivered decisions, then keeps awaiting until the recorder kills it.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:4801/mcp".to_owned());
    eprintln!("demo_agent connecting to {url}…");
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url),
    );
    let client = ClientInfo::default().serve(transport).await?;

    let graph = json!({
        "title": "Realtime collaboration for the notes app",
        "announce": "Proposed a realtime sync design — two decisions need you.",
        "focus": "sync-engine",
        "nodes": [
            {"id": "web", "label": "Web Client", "kind": "ui", "status": "existing",
             "description": "React SPA for editing notes"},
            {"id": "api", "label": "Notes API", "kind": "service", "status": "existing",
             "description": "CRUD for notes, folders, and sharing"},
            {"id": "auth", "label": "Auth Service", "kind": "service", "status": "existing",
             "description": "Sessions, tokens, permissions"},
            {"id": "sync-engine", "label": "Sync Engine", "kind": "component", "status": "proposed",
             "group": "sync",
             "description": "Merges concurrent edits from multiple clients",
             "choices": [{
                "id": "conflict-strategy",
                "prompt": "Conflict resolution strategy",
                "rationale": "Concurrent edits must merge without losing keystrokes; the strategy shapes storage and the client protocol.",
                "options": [
                    {"id": "crdt", "label": "CRDT document model",
                     "summary": "Notes become CRDTs; merges are automatic and offline-friendly.",
                     "pros": ["No central lock", "Offline edits merge cleanly"],
                     "cons": ["Document format migration", "Larger stored docs"],
                     "recommended": true,
                     "affects": ["storage", "presence"]},
                    {"id": "ot", "label": "Operational transforms",
                     "summary": "Server-ordered transforms, Google-Docs style.",
                     "pros": ["Compact history", "Well-trodden path"],
                     "cons": ["Server is a serialization bottleneck", "Tricky edge cases"],
                     "affects": ["ws"]}
                ]
             }]},
            {"id": "presence", "label": "Presence Service", "kind": "service", "status": "proposed",
             "group": "sync",
             "description": "Who is online, cursors, and typing indicators"},
            {"id": "ws", "label": "WebSocket Gateway", "kind": "service", "status": "proposed",
             "group": "sync",
             "description": "Long-lived connections pushing edits to clients"},
            {"id": "storage", "label": "Notes Store", "kind": "data_store", "status": "existing",
             "description": "Primary storage for notes and users",
             "choices": [{
                "id": "history-storage",
                "prompt": "Edit history storage",
                "options": [
                    {"id": "event-log", "label": "Append-only event log",
                     "summary": "Every edit is an event; state is a fold.",
                     "pros": ["Perfect audit trail", "Time travel for free"],
                     "cons": ["Compaction needed", "Bigger storage bill"],
                     "recommended": true,
                     "affects": ["sync-engine", "search"]},
                    {"id": "snapshots", "label": "Periodic snapshots",
                     "summary": "Store full documents every N edits.",
                     "pros": ["Simple reads", "Small working set"],
                     "cons": ["History granularity lost"]}
                ]
             }]},
            {"id": "search", "label": "Search Index", "kind": "component", "status": "existing",
             "description": "Full-text search over notes"},
            {"id": "jobs", "label": "Job Queue", "kind": "queue", "status": "existing",
             "description": "Background work: indexing, emails"},
            {"id": "mail", "label": "Email Provider", "kind": "external", "status": "existing",
             "description": "Transactional mail (share invites)"}
        ],
        "edges": [
            {"from": "web", "to": "api", "kind": "data_flow", "status": "existing"},
            {"from": "web", "to": "ws", "kind": "data_flow", "status": "proposed"},
            {"from": "ws", "to": "sync-engine", "kind": "data_flow", "status": "proposed"},
            {"from": "sync-engine", "to": "storage", "kind": "depends_on", "status": "proposed"},
            {"from": "presence", "to": "ws", "kind": "data_flow", "status": "proposed"},
            {"from": "api", "to": "auth", "kind": "depends_on", "status": "existing"},
            {"from": "api", "to": "storage", "kind": "depends_on", "status": "existing"},
            {"from": "storage", "to": "search", "kind": "data_flow", "status": "existing"},
            {"from": "api", "to": "jobs", "kind": "data_flow", "status": "existing"},
            {"from": "jobs", "to": "mail", "kind": "data_flow", "status": "existing"}
        ]
    });
    client
        .call_tool(
            CallToolRequestParams::new("propose_graph")
                .with_arguments(graph.as_object().cloned().unwrap()),
        )
        .await?;
    eprintln!("graph proposed; awaiting decisions…");

    let mut reacted = false;
    loop {
        let result = client
            .call_tool(
                CallToolRequestParams::new("await_decisions").with_arguments(
                    json!({"timeout_seconds": 240}).as_object().cloned().unwrap(),
                ),
            )
            .await?;
        let text = result.content[0]
            .as_text()
            .map(|t| t.text.clone())
            .unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text)?;
        if v["status"] == "delivered" && !reacted {
            reacted = true;
            client
                .call_tool(
                    CallToolRequestParams::new("update_graph").with_arguments(
                        json!({
                            "announce": "Applied your decisions — CRDTs it is; storage feels it.",
                            "ops": [
                                {"op": "set_status", "id": "sync-engine", "status": "modified"},
                                {"op": "set_status", "id": "storage", "status": "affected"}
                            ]
                        })
                        .as_object()
                        .cloned()
                        .unwrap(),
                    ),
                )
                .await?;
            eprintln!("reacted to delivery; continuing to await…");
        }
    }
}
```

- [ ] **Step 2: Reconcile the `update_graph` op schema.** Read `src/server/tools.rs` and check the exact JSON shape of the op list (field names for a status-set op; whether it is `{"op": "set_status", "id": …, "status": …}` or another spelling). Adjust ONLY the two op objects to match the real schema — everything else stays. If `set_status` ops don't exist, use the schema's node-upsert op re-sending the two nodes with the new `status` (copy their JSON from the proposal above, changed status only).

- [ ] **Step 3: Compile + lint.** `cargo build --examples` → success; `cargo clippy --all-targets -- -D warnings`; `cargo fmt`. (`clippy --all-targets` covers examples — the infinite loop needs no `#[allow]`: it returns `anyhow::Result` via `?` exits only.)

- [ ] **Step 4: Runtime smoke.** `cargo run -- --headless --port 4801 --sessions-dir "$env:TEMP\nodestorm-demo-sessions" --prefs "$env:TEMP\nodestorm-demo-prefs.json"` in the background, then `cargo run --example demo_agent` for ~5 s: its stderr must print `graph proposed; awaiting decisions…` with no error. Kill both, delete the scratch session dir.

- [ ] **Step 5: Commit.**

```powershell
git add examples/demo_agent.rs
git commit -m "feat(examples): demo_agent — scripted notes-app graph for the README demo"
```

---

### Task 4: ffmpeg + `scripts/record-demo.ps1` core, proven on segment 1

**Files:**
- Create: `scripts/record-demo.ps1`
- Output (not committed yet): `target\demo\01-propose\frames\*.png`, `target\demo\01-propose.gif`

**Interfaces:**
- Consumes: `scripts/uia-lib.ps1` helpers (Task 1), `--window-size` (Task 2), `examples/demo_agent.rs` (Task 3).
- Produces for Task 5: functions `Start-DemoApp([double]$W,[double]$H)`, `Stop-DemoApp`, `Start-Capture([string]$SegDir)`, `Stop-Capture`, `Add-Caption([string]$Text,[double]$MinSeconds=2.5)`, `Invoke-Dwell([double]$Seconds)`, `Convert-SegmentGif([string]$SegDir,[string]$OutGif)`, plus `$script:Ffmpeg`.

- [ ] **Step 1: Install ffmpeg (user-approved system install).**

```powershell
winget install --id Gyan.FFmpeg -e --accept-source-agreements --accept-package-agreements
```

Expected: success (or "already installed"). Resolve for the current session:

```powershell
$script:Ffmpeg = (Get-Command ffmpeg -ErrorAction SilentlyContinue).Source
if (-not $script:Ffmpeg) {
    $script:Ffmpeg = (Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Gyan.FFmpeg*" -Recurse -Filter ffmpeg.exe | Select-Object -First 1).FullName
}
& $script:Ffmpeg -version | Select-Object -First 1
```

- [ ] **Step 2: Write the recorder skeleton.** `scripts/record-demo.ps1`:

```powershell
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
        '{"theme":"nodestorm","mode":"dark"}' | Set-Content -Encoding utf8NoBOM $PrefsFile
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
    New-Item -ItemType Directory -Force (Join-Path $SegDir 'frames') | Out-Null
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
    $t = ((Get-Date) - $script:CaptureStart).TotalSeconds
    if ($script:Captions.Count -gt 0) {
        $prev = $script:Captions[$script:Captions.Count - 1]
        $prev.end = [Math]::Max($prev.start + $prev.min, $t)   # close previous caption
    }
    $script:Captions.Add(@{ start = $t; end = 0; min = $MinSeconds; text = $Text })
}

function Invoke-Dwell([double]$Seconds) { Start-Sleep -Milliseconds (1000 * $Seconds) }

function Save-Captions([string]$SegDir) {
    $t = ((Get-Date) - $script:CaptureStart).TotalSeconds
    if ($script:Captions.Count -gt 0) {
        $last = $script:Captions[$script:Captions.Count - 1]
        $last.end = [Math]::Max($last.start + $last.min, $t)
    }
    $script:Captions | ConvertTo-Json | Set-Content (Join-Path $SegDir 'captions.json')
}

function Convert-SegmentGif([string]$SegDir, [string]$OutGif, [int]$Fps = 10, [int]$Width = 800) {
    $caps = Get-Content (Join-Path $SegDir 'captions.json') | ConvertFrom-Json
    $font = 'C\:/Windows/Fonts/segoeui.ttf'
    $draw = foreach ($c in $caps) {
        $txt = ($c.text -replace "'", "\\'" -replace ':', '\:')
        "drawtext=fontfile='$font':text='$txt':fontsize=22:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=10:x=(w-text_w)/2:y=h-th-16:enable='between(t,{0},{1})'" -f
            [Math]::Round($c.start, 2), [Math]::Round($c.end, 2)
    }
    $vf = "fps=$Fps,scale=${Width}:-1:flags=lanczos," + ($draw -join ',') +
        ",split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer"
    & $script:Ffmpeg -y -framerate 10 -i (Join-Path $SegDir 'frames\frame_%05d.png') `
        -vf $vf -loop 0 $OutGif
    if ($LASTEXITCODE -ne 0) { Fail "ffmpeg gif failed for $OutGif" }
    Log ("{0}: {1:N1} MB" -f (Split-Path -Leaf $OutGif), ((Get-Item $OutGif).Length / 1MB))
}
```

(Adapt `$script:Ffmpeg` resolution from Step 1 into the top of the script. If `Save-WindowPng`'s signature in uia-lib differs — e.g. it takes the top HWND and derives sizes internally — call it exactly as `verify-windows.ps1` does; the runspace dot-sources the lib so the same call works.)

- [ ] **Step 3: Segment 1 action list.** Append:

```powershell
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
```

and a `main` dispatch at the bottom:

```powershell
New-Item -ItemType Directory -Force $WorkDir | Out-Null
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
if (-not $Segment) { Stop-DemoApp }
```

- [ ] **Step 4: Prove segment 1.**

Run: `powershell -File scripts\record-demo.ps1 -Segment 1` (then `Stop-DemoApp` happens; adjust dispatch so a single-segment run also stops the app).
Expected: `target\demo\01-propose.gif` exists, loops, ≤4 MB, captions legible and paced (≥2.5 s each). **View the GIF frames with the Read tool** (or the gif in a viewer-less spot-check: read 3 spread frame PNGs) and confirm captions + content.

- [ ] **Step 5: Commit (script only — no media yet).**

```powershell
git add scripts/record-demo.ps1
git commit -m "feat(demo): recorder core — capture runspace, caption timeline, gif assembly"
```

---

### Task 5: Segments 2–8 + full assembly (MP4, SRT, poster)

**Files:**
- Modify: `scripts/record-demo.ps1` (segment functions + assembly + `-Publish`)
- Output (committed in Task 6): `docs/demo/01…08-*.gif`, `nodestorm-demo.mp4`, `nodestorm-demo.srt`, `poster.png`

**Interfaces:**
- Consumes: Task 4's functions; demo_agent node labels/choice prompts (Task 3's exact strings).
- Produces: the 8 final media files.

- [ ] **Step 1: Segment functions 2–8.** Same pattern as `Invoke-Segment1`; the UIA action vocabulary (all from uia-lib): `Click-Element $script:Hwnd '<name>'`, typing via the WM_CHAR helper, `Wait-Element`. Segment scripts (captions abridged here are the EXACT strings to use):

- **2 (decide, same app):** `Click-Element` `Sync Engine` → wait `Conflict resolution strategy` → captions: `Every open choice is pinned to its component` / hover `CRDT document model` via a posted WM_MOUSEMOVE at its rect center (`Click-Point`-style move without button messages — add a `Move-To-Element` helper in record-demo.ps1 that posts only 0x0200) → caption `Hover an option - the components it would ripple into light up` (dwell 3) → click `CRDT document model` → caption `Picked - with the trade-offs recorded` → close panel `✕`, click `Notes Store`, click `Append-only event log` → type into `optional message to the agent…` (focus by click, then WM_CHAR) the text `prefer CRDTs - offline first` → caption `Add a note for the agent and send your decisions` → click `Send to agent` → wait for agent reaction (`Wait-Element` on the `MODIFIED` status text or the announce toast in the activity feed; use `Wait-Element 'Applied your decisions — CRDTs it is; storage feels it.' 20` if exposed, else the `modified` tag on Sync Engine) → caption `The agent wakes with your decisions and updates the graph` (dwell 3).
- **3 (edit, same app):** click `+ Component` → caption `The canvas is yours too - add your own components`; select the new card `New component`; type rename `Metrics Collector` via the panel's edit form (click the label input first — the E2E rename flow in verify-windows.ps1 is the reference for exact clicks); caption `Rename, connect, delete - your edits flow back to the agent`; panel `Connect →` then click `Notes API`; click `Presence Service` then panel `Delete` (agent node → removal_requested); caption `Deleting an agent's component asks the agent politely`; click the `↶ Undo` pod; caption `Undo covers every edit until decisions are delivered` (dwell 2.5).
- **4 (navigate, same app):** click the search box, type `notes`; caption `Search highlights matches...`; post Enter (WM_KEYDOWN 0x0D to the widget) twice with 1.2 s between — caption `...and Enter zoom-cycles through them`; Esc to clear; caption `The minimap pans big graphs - groups collapse into one card`; click the `sync` group pill on `Sync Engine` → cluster card appears (wait `3 components`); dwell 2; click `⊞ expand`; dwell 1.5.
- **5 (sessions, same app):** open the session menu (click the `sess-pod` by its current name — `Wait-Element` the row `Create`), type `experiment` into `new session…`, click `Create`; caption `Sessions are parallel brainstorms - agents can wait on one while you work in another`; switch back via the menu (click the original session row name); click `Compare` on the experiment row; caption `Compare shows how two sessions drifted`; close `✕`; click `Timeline`; caption `The timeline is the session's full decision log`; dwell 2.5; click `Timeline` again.
- **6 (export, same app):** click `More` → `Export ▾` → caption `Export writes a Markdown decision record - pros, cons, and the trail`; click `Export`; wait for the activity-feed receipt (`Wait-Element` on text starting `exported` — check the actual receipt string in src/store.rs `record_export`); dwell 3.
- **7 (themes, same app):** click `More` → `Theme ▾`; caption `Twelve palettes, light and dark, live-switching`; click `Light`; dwell 1.5; click `Gruvbox` (menu closes); dwell 1.5; `More` → `Theme ▾` → click `Catppuccin`; dwell 2; back to dark default: `More` → `Theme ▾` → `Dark` → `Nodestorm` (restores for consistency; prefs file is scratch anyway).
- **8 (responsive):** `Stop-DemoApp`; `Start-DemoApp 760 840` (data persists — same scratch session shows the graph); caption `Narrow window? The bar folds - message the agent from the compose pod`; click `Message to agent` (✎) → popover; type `ship it`; click `Send with message`; dwell 2; `Stop-DemoApp`; `Start-DemoApp 520 840`; caption `Even tiny windows keep every control reachable via More`; click `More`; wait `↶ Undo`; dwell 2.5; click `More`; `Stop-DemoApp`. Frames from both launches append into ONE segment dir (continue the frame counter — pass a start index to `Start-Capture`, or use two frame dirs concatenated at assembly; pick the two-dir + ffmpeg concat approach and stop/start capture around the relaunch).

- [ ] **Step 2: Assembly + publish.** Add after the segment dispatch:

```powershell
function Publish-Demo {
    $docs = Join-Path $RepoRoot 'docs\demo'
    New-Item -ItemType Directory -Force $docs | Out-Null
    $names = '01-propose','02-decide','03-edit','04-navigate','05-sessions','06-export','07-themes','08-responsive'
    foreach ($n in $names) { Copy-Item (Join-Path $WorkDir "$n.gif") (Join-Path $docs "$n.gif") -Force }
    # MP4: per-segment captioned mp4s, then concat.
    $listFile = Join-Path $WorkDir 'concat.txt'
    Remove-Item $listFile -ErrorAction SilentlyContinue
    foreach ($n in $names) {
        $seg = Join-Path $WorkDir $n
        $mp4 = Join-Path $WorkDir "$n.mp4"
        $caps = Get-Content (Join-Path $seg 'captions.json') | ConvertFrom-Json
        $font = 'C\:/Windows/Fonts/segoeui.ttf'
        $draw = (foreach ($c in $caps) {
            $txt = ($c.text -replace "'", "\\'" -replace ':', '\:')
            "drawtext=fontfile='$font':text='$txt':fontsize=28:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=12:x=(w-text_w)/2:y=h-th-24:enable='between(t,{0},{1})'" -f
                [Math]::Round($c.start, 2), [Math]::Round($c.end, 2)
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
}

function Write-DemoSrt([string[]]$Names, [string]$OutSrt) {
    $offset = 0.0; $idx = 1; $lines = @()
    foreach ($n in $Names) {
        $seg = Join-Path $WorkDir $n
        $caps = Get-Content (Join-Path $seg 'captions.json') | ConvertFrom-Json
        $frameCount = (Get-ChildItem (Join-Path $seg 'frames') -Filter 'frame_*.png').Count
        foreach ($c in $caps) {
            $fmt = { param($s) ([TimeSpan]::FromSeconds($s)).ToString('hh\:mm\:ss\,fff') }
            $lines += $idx; $idx++
            $lines += ('{0} --> {1}' -f (& $fmt ($offset + $c.start)), (& $fmt ($offset + $c.end)))
            $lines += $c.text; $lines += ''
        }
        $offset += $frameCount / 10.0
    }
    $lines | Set-Content -Encoding utf8NoBOM $OutSrt
}
```

Segment 8's two frame dirs: give `Publish-Demo`/`Convert-SegmentGif` for that segment a pre-step that renumbers dir2's frames to continue dir1's count into a merged `frames\` dir (a simple copy loop) — captions were recorded on a single clock only if capture wasn't restarted; since it WAS restarted, record segment 8 as two caption files and merge with the dir1 duration as offset (same arithmetic as `Write-DemoSrt`).

- [ ] **Step 3: Full recording run.**

Run: `powershell -File scripts\record-demo.ps1` then `powershell -File scripts\record-demo.ps1 -Publish` (or fold `-Publish` into the default full run).
Expected: 8 GIFs in `docs\demo\` each ≤4 MB (if any exceeds: re-run `Convert-SegmentGif` for it with `-Fps 8`, then `-Width 720`), MP4 + SRT + poster present, total ≤35 MB (`(Get-ChildItem docs\demo | Measure-Object Length -Sum).Sum / 1MB`). **Review every GIF** by reading 3–4 spread frames each: captions correct and legible, actions visible, pacing calm. Re-record any segment that reads rushed (`-Segment N`).

- [ ] **Step 4: Verification sweep.** `cargo fmt` / `clippy -D warnings` / `cargo test` (unchanged code, still must be green), plus `powershell -File scripts\verify-windows.ps1 -DemoShot` (the recorder must not have broken the shared lib).

- [ ] **Step 5: Commit (script + media).**

```powershell
git add scripts/record-demo.ps1 docs/demo
git commit -m "feat(demo): segments 2-8, mp4/srt/poster assembly; recorded assets"
```

---

### Task 6: README — embeds + mermaid architecture-beta + final gates

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: `docs/demo/*` (Task 5), spec §5's exact mermaid block.

- [ ] **Step 1: Poster + MP4 link.** After the intro paragraph (before `## How it works`):

```markdown
[![100-second tour of nodestorm](docs/demo/poster.png)](docs/demo/nodestorm-demo.mp4)

*Click for the full 100-second tour (MP4 + [subtitles](docs/demo/nodestorm-demo.srt)); GIF highlights are inline below.*
```

- [ ] **Step 2: Replace the ASCII diagram.** Delete the fenced code block containing the box-drawing diagram in `## How it works` and insert the spec §5 mermaid block verbatim (architecture-beta: `agent(server)` ↔ group `app` containing `mcp(internet)` / `store(database)` / `canvas(cloud)`, `<-->` edges), followed by the legend line: ``*`propose_graph` / `update_graph` flow left→right; `await_decisions` blocks until your decisions flow back with **Send ϟ** (loopback HTTP on 127.0.0.1:4747).*``

- [ ] **Step 3: Embed the GIFs** in the matching sections, each as `![<alt>](docs/demo/NN-name.gif)`: 01 after the mermaid legend (end of "How it works"), 02 same section after the decision bullets, 03 in "Edit the graph yourself", 04 after the big-graphs/search paragraph, 05 in "Sessions", 06 near the Export bullets in "Timeline"/export area, 07 in "Theming", and 08 at the END of "Theming" with alt text `The top bar folds gracefully at narrow widths`. Every path must satisfy `git ls-files docs/demo` after Task 5's commit.

- [ ] **Step 4: Gates.** `cargo fmt` / `clippy` / `cargo test` green; `powershell -File scripts\verify-windows.ps1` full E2E green (README-only change — this is the branch's final safety net); link check: every `docs/demo/...` path in README exists in `git ls-files`; etiquette audit per the spec: `Select-String -Pattern 'SendInput|SetForegroundWindow|CopyFromScreen' scripts\*.ps1` → zero matches.

- [ ] **Step 5: Commit + push.**

```powershell
git add README.md
git commit -m "docs: demo GIFs + full-tour MP4 embedded; architecture as mermaid architecture-beta"
git push
```

- [ ] **Step 6: Rendering verification (controller step).** Load the pushed README on the PR branch in a real browser (`https://github.com/robot-head/nodestorm/blob/claude/ui-redesign-button-overflow-f1beb5/README.md`): the mermaid architecture-beta diagram must render (not show raw code or an error box) and the GIFs must animate. If mermaid fails to render, replace with the spec's `flowchart LR` fallback (same nodes + legend) and re-push.
