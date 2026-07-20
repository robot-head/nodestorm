# Integrated Terminal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run launched agent sessions inside Nodestorm in a bottom terminal panel (Ferroterm WebGL + Rust PTY over a token-gated loopback WebSocket), restorable by clicking the agent's name anywhere in the UI.

**Architecture:** A `TerminalManager` in Rust owns PTYs (portable-pty: ConPTY on Windows, openpty elsewhere), scrollback rings, and statuses. The embedded axum server gains a `/terminal/{id}/ws` WebSocket route gated by a per-run random token, plus `/terminal/assets/*` serving the vendored Ferroterm ES module + WASM from embedded bytes. The Dioxus UI hosts one Ferroterm instance (WebGL renderer, Canvas2D fallback) per agent tab in a collapsible bottom dock; topbar chips and existing agent-name attributions focus tabs. The launcher's integrated path calls `TerminalManager::spawn` with the same `CommandSpec` it builds today; the system-terminal path stays.

**Tech Stack:** Rust 2024 / Dioxus 0.7 desktop (WebView2), axum 0.8 (`ws` feature), portable-pty 0.9, tokio, vendored `ferroterm` npm package (ES module + WASM, WebGL renderer — https://datanoisetv.github.io/ferroterm/), tokio-tungstenite (dev).

**Spec:** `docs/superpowers/specs/2026-07-20-integrated-terminal-design.md`

## Global Constraints

- Rust `edition = "2024"`, `rust-version = "1.97.1"`.
- The webview loads no remote code and the app makes no external network fetches: Ferroterm is vendored under `assets/ferroterm/`, embedded with `include_bytes!`, and served only from the loopback listener at `/terminal/assets/*` (correct MIME types; `Access-Control-Allow-Origin: *` so the webview origin can import the module and fetch the WASM).
- The server binds `127.0.0.1` only. The terminal WebSocket requires the per-run token on every upgrade.
- Process spawning uses executable-plus-argument arrays end to end; never build a shell string.
- Terminal ids are the launcher agent identity `{agent-id}-{session-slug}` (e.g. `claude-cache-redesign`): lowercase alphanumerics, `-`, `/` from the slug — safe to embed in URLs and JS string literals without escaping.
- Nothing about terminal output is persisted.
- Tests use `assert2::assert!` and `yare::parameterized` like the rest of the repo. PTY tests must pass on both Windows (`cmd`) and POSIX (`sh`) — every spawn in a test picks the command with `cfg!(windows)`.
- Verification gates (same as CI): `cargo test --all-targets --locked`, `cargo fmt --all -- --check`, `cargo clippy --all-targets --locked -- -D warnings`.
- Comment style: `//!` module docs; sparse inline comments only for non-obvious constraints.

## File Structure

- Create: `src/terminal.rs` — PTY lifecycle, scrollback, statuses (`TerminalManager`).
- Create: `src/server/terminal_ws.rs` — WebSocket route, token check, byte pump; later the Ferroterm asset routes.
- Create: `src/ui/terminal_panel.rs` — bottom dock, tabs, Ferroterm mount, close confirm.
- Create: `assets/ferroterm/` — vendored Ferroterm ES module + WASM (exact names from the npm tarball), `mount.js`, `README.md`.
- Modify: `src/lib.rs`, `src/main.rs`, `src/server/mod.rs`, `src/ui/mod.rs`, `src/ui/app.rs`, `src/ui/agent_launcher.rs`, `src/ui/topbar.rs`, `src/ui/activity.rs`, `src/ui/node_card.rs`, `src/ui/choice_panel.rs`, `assets/main.css`, `Cargo.toml`, `tests/mcp_roundtrip.rs`.

---

### Task 1: TerminalManager (PTY core)

**Files:**
- Modify: `Cargo.toml` (add `portable-pty`)
- Modify: `src/lib.rs` (add `pub mod terminal;`)
- Create: `src/terminal.rs`

**Interfaces:**
- Consumes: `crate::agent_launcher::CommandSpec` (`program: String`, `args: Vec<String>`, `current_dir: Option<String>`).
- Produces (used by Tasks 2–6):
  - `TerminalManager::new() -> Arc<TerminalManager>`
  - `fn token(&self) -> &str`
  - `fn spawn(self: &Arc<Self>, id: &str, spec: &CommandSpec) -> anyhow::Result<()>`
  - `fn attach(&self, id: &str) -> Option<Attached>` where `pub struct Attached { pub replay: Vec<u8>, pub output: tokio::sync::broadcast::Receiver<Vec<u8>> }`
  - `fn write(&self, id: &str, bytes: &[u8]) -> anyhow::Result<()>`
  - `fn resize(&self, id: &str, cols: u16, rows: u16) -> anyhow::Result<()>`
  - `fn kill(&self, id: &str)` — terminate child, keep the entry.
  - `fn close(&self, id: &str)` — kill and remove the entry.
  - `fn kill_all(&self)` — kill every running child (app shutdown).
  - `fn list(&self) -> Vec<TerminalInfo>` where `pub struct TerminalInfo { pub id: String, pub status: TerminalStatus }`, `pub enum TerminalStatus { Running, Exited(u32) }` (both `Debug, Clone, PartialEq, Eq`; `TerminalStatus` also `Copy`).
  - `fn running_count(&self) -> usize`
  - `fn subscribe(&self) -> tokio::sync::watch::Receiver<u64>` — generation bumps on spawn/exit/close.

- [ ] **Step 1: Add the dependency**

```powershell
cargo add portable-pty@0.9
```

Expected: `Cargo.toml` gains `portable-pty = "0.9"` under `[dependencies]`; `cargo build` succeeds.

- [ ] **Step 2: Write failing unit tests for the pure parts**

Create `src/terminal.rs` with module doc, the types above as stubs (`todo!()` bodies are fine for methods not under test yet), the pure ring helper, and this test module. Add `pub mod terminal;` to `src/lib.rs` (keep the module list alphabetical).

```rust
//! Integrated terminal PTYs: spawn agent commands, buffer output, and expose
//! attach/write/resize/kill for the WebSocket bridge and the UI.
//!
//! Blocking PTY I/O lives on dedicated reader threads; everything else is a
//! mutex-guarded map so both the axum runtime and the UI thread can call in.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{broadcast, watch};

use crate::agent_launcher::CommandSpec;

/// Scrollback cap per terminal. Replay after this shows a truncated head,
/// which is fine for a live agent session.
pub const SCROLLBACK_LIMIT: usize = 1024 * 1024;

/// Cap enforcement: keep the newest bytes, drop the oldest.
fn push_ring(ring: &mut Vec<u8>, chunk: &[u8], cap: usize) {
    ring.extend_from_slice(chunk);
    if ring.len() > cap {
        ring.drain(..ring.len() - cap);
    }
}
```

Tests (bottom of the file):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_newest_bytes_up_to_cap() {
        let mut ring = Vec::new();
        push_ring(&mut ring, b"abcd", 6);
        push_ring(&mut ring, b"efgh", 6);
        assert2::assert!((ring) == (b"cdefgh".to_vec()));
        push_ring(&mut ring, b"", 6);
        assert2::assert!((ring) == (b"cdefgh".to_vec()));
        push_ring(&mut ring, b"0123456789", 6);
        assert2::assert!((ring) == (b"456789".to_vec()));
    }

    #[test]
    fn token_is_stable_long_and_url_safe() {
        let manager = TerminalManager::new();
        let token = manager.token().to_owned();
        assert2::assert!(token.len() >= 32);
        assert2::assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
        assert2::assert!((manager.token()) == (token));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib terminal -- --nocapture`
Expected: FAIL (compile error on missing `TerminalManager` methods, or `todo!()` panic).

- [ ] **Step 4: Implement the manager**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalStatus {
    Running,
    Exited(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalInfo {
    pub id: String,
    pub status: TerminalStatus,
}

/// A subscriber's view: everything so far plus the live stream. Snapshot and
/// subscription happen under one lock so no byte is missed or duplicated.
pub struct Attached {
    pub replay: Vec<u8>,
    pub output: broadcast::Receiver<Vec<u8>>,
}

struct Entry {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    ring: Vec<u8>,
    tx: broadcast::Sender<Vec<u8>>,
    status: TerminalStatus,
}

pub struct TerminalManager {
    token: String,
    inner: Mutex<BTreeMap<String, Entry>>,
    generation: watch::Sender<u64>,
}

impl TerminalManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            inner: Mutex::new(BTreeMap::new()),
            generation: watch::Sender::new(0),
        })
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    fn bump(&self) {
        self.generation.send_modify(|g| *g += 1);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Entry>> {
        self.inner.lock().expect("terminal map mutex poisoned")
    }

    pub fn spawn(self: &Arc<Self>, id: &str, spec: &CommandSpec) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.lock().contains_key(id),
            "terminal `{id}` already exists"
        );
        let pair = native_pty_system().openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut builder = CommandBuilder::new(&spec.program);
        builder.args(&spec.args);
        if let Some(dir) = &spec.current_dir {
            builder.cwd(dir);
        }
        #[cfg(not(target_os = "windows"))]
        builder.env("TERM", "xterm-256color");
        let mut child = pair.slave.spawn_command(builder)?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let writer = pair.master.take_writer()?;
        let mut reader = pair.master.try_clone_reader()?;
        self.lock().insert(
            id.to_owned(),
            Entry {
                master: pair.master,
                writer,
                killer,
                ring: Vec::new(),
                tx: broadcast::channel(1024).0,
                status: TerminalStatus::Running,
            },
        );
        let manager = self.clone();
        let id = id.to_owned();
        std::thread::Builder::new()
            .name(format!("pty-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut map = manager.lock();
                    if let Some(entry) = map.get_mut(&id) {
                        push_ring(&mut entry.ring, &buf[..n], SCROLLBACK_LIMIT);
                        // Receivers may lag or be absent; both are fine.
                        let _ = entry.tx.send(buf[..n].to_vec());
                    } else {
                        break;
                    }
                }
                let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
                if let Some(entry) = manager.lock().get_mut(&id) {
                    entry.status = TerminalStatus::Exited(code);
                }
                manager.bump();
            })
            .expect("spawning the pty reader thread");
        self.bump();
        Ok(())
    }

    pub fn attach(&self, id: &str) -> Option<Attached> {
        let map = self.lock();
        let entry = map.get(id)?;
        Some(Attached {
            replay: entry.ring.clone(),
            output: entry.tx.subscribe(),
        })
    }

    pub fn write(&self, id: &str, bytes: &[u8]) -> anyhow::Result<()> {
        let mut map = self.lock();
        let entry = map
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("no terminal `{id}`"))?;
        entry.writer.write_all(bytes)?;
        entry.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> anyhow::Result<()> {
        let map = self.lock();
        let entry = map
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("no terminal `{id}`"))?;
        entry.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn kill(&self, id: &str) {
        if let Some(entry) = self.lock().get_mut(id) {
            let _ = entry.killer.kill();
        }
    }

    pub fn close(&self, id: &str) {
        self.kill(id);
        // Dropping the entry drops `tx`, which ends every attached pump.
        self.lock().remove(id);
        self.bump();
    }

    pub fn kill_all(&self) {
        let mut map = self.lock();
        for entry in map.values_mut() {
            if entry.status == TerminalStatus::Running {
                let _ = entry.killer.kill();
            }
        }
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        self.lock()
            .iter()
            .map(|(id, entry)| TerminalInfo {
                id: id.clone(),
                status: entry.status,
            })
            .collect()
    }

    pub fn running_count(&self) -> usize {
        self.lock()
            .values()
            .filter(|entry| entry.status == TerminalStatus::Running)
            .count()
    }

    pub fn status(&self, id: &str) -> Option<TerminalStatus> {
        self.lock().get(id).map(|entry| entry.status)
    }
}
```

Note: the reader thread holds `child` (for `wait()`); the map keeps a `clone_killer()` handle so `kill` never blocks on the reader.

- [ ] **Step 5: Run unit tests**

Run: `cargo test --lib terminal`
Expected: both tests PASS.

- [ ] **Step 6: Write failing PTY integration tests**

Append to the test module in `src/terminal.rs`. Shared helpers first:

```rust
    use std::time::{Duration, Instant};

    /// `cmd /c <line>` on Windows, `sh -c <line>` elsewhere.
    fn shell_spec(line: &str, cwd: Option<&str>) -> CommandSpec {
        let (program, flag) = if cfg!(windows) {
            ("cmd", "/c")
        } else {
            ("sh", "-c")
        };
        CommandSpec {
            program: program.into(),
            args: vec![flag.into(), line.into()],
            current_dir: cwd.map(str::to_owned),
        }
    }

    /// An interactive shell that reads commands from the PTY.
    fn interactive_shell_spec() -> CommandSpec {
        CommandSpec {
            program: if cfg!(windows) { "cmd" } else { "sh" }.into(),
            args: vec![],
            current_dir: None,
        }
    }

    fn wait_for(mut pred: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !pred() {
            assert2::assert!(Instant::now() < deadline, "timed out waiting");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn exited(manager: &TerminalManager, id: &str) -> bool {
        matches!(manager.status(id), Some(TerminalStatus::Exited(_)))
    }

    #[test]
    fn spawn_streams_output_and_reports_exit_code() {
        let manager = TerminalManager::new();
        let mut generation = manager.subscribe();
        manager
            .spawn("t-echo", &shell_spec("echo pty-hello&& exit 3", None))
            .unwrap();
        wait_for(|| exited(&manager, "t-echo"));

        let replay = manager.attach("t-echo").unwrap().replay;
        let text = String::from_utf8_lossy(&replay);
        assert2::assert!(text.contains("pty-hello"), "replay: {text}");
        assert2::assert!((manager.status("t-echo")) == (Some(TerminalStatus::Exited(3))));
        assert2::assert!(generation.has_changed().unwrap());
        assert2::assert!((manager.running_count()) == (0));
    }

    #[test]
    fn spawn_respects_current_dir() {
        let dir = std::env::temp_dir().join(format!("nodestorm-pty-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = TerminalManager::new();
        let line = if cfg!(windows) { "cd" } else { "pwd" };
        manager
            .spawn("t-cwd", &shell_spec(line, Some(dir.to_str().unwrap())))
            .unwrap();
        wait_for(|| exited(&manager, "t-cwd"));

        let replay = manager.attach("t-cwd").unwrap().replay;
        let text = String::from_utf8_lossy(&replay);
        let marker = dir.file_name().unwrap().to_str().unwrap().to_owned();
        assert2::assert!(text.contains(&marker), "replay: {text}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn write_reaches_the_child_and_close_removes_the_entry() {
        let manager = TerminalManager::new();
        manager
            .spawn("t-shell", &interactive_shell_spec())
            .unwrap();
        assert2::assert!((manager.status("t-shell")) == (Some(TerminalStatus::Running)));
        manager.write("t-shell", b"exit\r\n").unwrap();
        wait_for(|| exited(&manager, "t-shell"));

        manager.close("t-shell");
        assert2::assert!(manager.attach("t-shell").is_none());
        assert2::assert!(manager.list().is_empty());
    }

    #[test]
    fn kill_terminates_a_running_child() {
        let manager = TerminalManager::new();
        manager
            .spawn("t-kill", &interactive_shell_spec())
            .unwrap();
        assert2::assert!((manager.status("t-kill")) == (Some(TerminalStatus::Running)));
        manager.kill("t-kill");
        wait_for(|| exited(&manager, "t-kill"));
        // Entry survives a kill: scrollback stays readable until close().
        assert2::assert!(manager.attach("t-kill").is_some());
    }

    #[test]
    fn duplicate_ids_and_unknown_ids_error() {
        let manager = TerminalManager::new();
        manager
            .spawn("t-dup", &interactive_shell_spec())
            .unwrap();
        assert2::assert!(manager.spawn("t-dup", &interactive_shell_spec()).is_err());
        assert2::assert!(manager.write("missing", b"x").is_err());
        assert2::assert!(manager.resize("missing", 80, 24).is_err());
        manager.close("t-dup");
    }
```

- [ ] **Step 7: Run the integration tests**

Run: `cargo test --lib terminal -- --test-threads=4`
Expected: all PASS. If `spawn_streams_output_and_reports_exit_code` sees no EOF on Windows, the ConPTY reader is not unblocking on child exit — debug there, do not lengthen timeouts past 20s.

- [ ] **Step 8: Gate and commit**

Run: `cargo fmt --all && cargo clippy --all-targets --locked -- -D warnings && cargo test --lib terminal`
Expected: clean.

```powershell
git add Cargo.toml Cargo.lock src/lib.rs src/terminal.rs
git commit -m "Add TerminalManager: PTY spawn, scrollback, status over portable-pty"
```

---

### Task 2: WebSocket route and server wiring

**Files:**
- Modify: `Cargo.toml` (axum `ws` feature; dev-deps `tokio-tungstenite`, `futures-util`)
- Create: `src/server/terminal_ws.rs`
- Modify: `src/server/mod.rs` (declare module; `serve`/`serve_with_manager` gain a `terminals` parameter; merge routes)
- Modify: `src/main.rs` (create the manager, pass to `serve`)
- Modify: `tests/mcp_roundtrip.rs:53` (pass a manager to `serve_with_manager`)

**Interfaces:**
- Consumes: `TerminalManager` (`token()`, `attach`, `write`, `resize`) from Task 1.
- Produces:
  - `pub(super) fn routes(manager: Arc<TerminalManager>) -> axum::Router` in `terminal_ws.rs`.
  - `server::serve(listener, sessions, terminals: Arc<TerminalManager>, shutdown)` and `serve_with_manager(listener, sessions, terminals, shutdown, manager)` — note the new third parameter.
  - Wire protocol (client ⇄ server): server sends Binary frames of raw PTY output (first frame is the scrollback replay, possibly empty); client sends Binary frames of raw input bytes and Text frames `{"resize":{"cols":N,"rows":N}}`.
  - `main.rs` owns `let terminals = nodestorm::terminal::TerminalManager::new();` — Tasks 3–6 receive this same `Arc`.

- [ ] **Step 1: Add dependencies**

```powershell
cargo add axum@0.8 --features ws
cargo add --dev tokio-tungstenite futures-util
```

Expected: `axum = { version = "0.8", features = ["ws"] }` in `[dependencies]`; both dev-deps added.

- [ ] **Step 2: Write the failing tests**

Create `src/server/terminal_ws.rs`:

```rust
//! Token-gated WebSocket bridge between a [`TerminalManager`] PTY and the
//! Ferroterm instance in the webview.
//!
//! Protocol: server→client Binary frames are raw PTY output (the first frame
//! replays scrollback); client→server Binary frames are raw input bytes and
//! Text frames carry `{"resize":{"cols":N,"rows":N}}`.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::terminal::{Attached, TerminalManager};

#[derive(serde::Deserialize)]
struct TokenQuery {
    #[serde(default)]
    token: String,
}

#[derive(serde::Deserialize)]
struct ControlFrame {
    resize: ResizeFrame,
}

#[derive(serde::Deserialize)]
struct ResizeFrame {
    cols: u16,
    rows: u16,
}

fn parse_resize(text: &str) -> Option<(u16, u16)> {
    let frame: ControlFrame = serde_json::from_str(text).ok()?;
    Some((frame.resize.cols, frame.resize.rows))
}

pub(super) fn routes(manager: Arc<TerminalManager>) -> axum::Router {
    axum::Router::new()
        .route("/terminal/{id}/ws", get(terminal_ws))
        .with_state(manager)
}

async fn terminal_ws(
    Path(id): Path<String>,
    Query(query): Query<TokenQuery>,
    State(manager): State<Arc<TerminalManager>>,
    ws: WebSocketUpgrade,
) -> Response {
    if query.token != manager.token() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(attached) = manager.attach(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ws.on_upgrade(move |socket| pump(socket, manager, id, attached))
}

async fn pump(mut socket: WebSocket, manager: Arc<TerminalManager>, id: String, attached: Attached) {
    let Attached { replay, mut output } = attached;
    if socket.send(Message::Binary(replay.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                Ok(bytes) => {
                    if socket.send(Message::Binary(bytes.into())).await.is_err() {
                        return;
                    }
                }
                // ponytail: lagged frames are dropped; the 1 MiB ring plus a
                // 1024-chunk channel makes this a reconnect-and-replay rarity.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if manager.write(&id, &bytes).is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Some((cols, rows)) = parse_resize(&text) {
                        let _ = manager.resize(&id, cols, rows);
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}
```

Test module in the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_launcher::CommandSpec;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite;
    use yare::parameterized;

    #[parameterized(
        valid = { r#"{"resize":{"cols":120,"rows":40}}"#, Some((120, 40)) },
        not_json = { "resize 120 40", None },
        missing_rows = { r#"{"resize":{"cols":120}}"#, None },
        wrong_shape = { r#"{"cols":120,"rows":40}"#, None },
    )]
    fn resize_frames_parse_strictly(text: &str, expected: Option<(u16, u16)>) {
        assert2::assert!(parse_resize(text) == expected);
    }

    async fn serve_terminal_routes(manager: Arc<TerminalManager>) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, routes(manager)).into_future());
        format!("ws://{addr}")
    }

    #[tokio::test]
    async fn websocket_replays_and_streams_pty_output() {
        let manager = TerminalManager::new();
        let (program, flag) = if cfg!(windows) { ("cmd", "/c") } else { ("sh", "-c") };
        manager
            .spawn(
                "ws-echo",
                &CommandSpec {
                    program: program.into(),
                    args: vec![flag.into(), "echo ws-pty-hello".into()],
                    current_dir: None,
                },
            )
            .unwrap();
        let base = serve_terminal_routes(manager.clone()).await;
        let url = format!("{base}/terminal/ws-echo/ws?token={}", manager.token());
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        while !String::from_utf8_lossy(&seen).contains("ws-pty-hello") {
            let frame = tokio::time::timeout_at(deadline, ws.next())
                .await
                .expect("pty output within 20s")
                .expect("socket open")
                .unwrap();
            if let tungstenite::Message::Binary(bytes) = frame {
                seen.extend_from_slice(&bytes);
            }
        }
        // Resize and input frames are accepted without dropping the socket.
        ws.send(tungstenite::Message::Text(
            r#"{"resize":{"cols":100,"rows":30}}"#.into(),
        ))
        .await
        .unwrap();
        ws.send(tungstenite::Message::Binary(b"\r\n".to_vec().into()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn wrong_token_and_unknown_terminal_are_rejected() {
        let manager = TerminalManager::new();
        let base = serve_terminal_routes(manager.clone()).await;

        let bad_token = format!("{base}/terminal/ws-echo/ws?token=wrong");
        let error = tokio_tungstenite::connect_async(bad_token).await.unwrap_err();
        let tungstenite::Error::Http(response) = error else {
            panic!("expected an http rejection, got {error:?}");
        };
        assert2::assert!((response.status()) == (403));

        let unknown = format!("{base}/terminal/missing/ws?token={}", manager.token());
        let error = tokio_tungstenite::connect_async(unknown).await.unwrap_err();
        let tungstenite::Error::Http(response) = error else {
            panic!("expected an http rejection, got {error:?}");
        };
        assert2::assert!((response.status()) == (404));
    }
}
```

- [ ] **Step 3: Run the new tests to verify current failure**

Run: `cargo test terminal_ws`
Expected: FAIL to compile until `mod terminal_ws;` is declared and signatures settle. That's the point — fix in the next step.

- [ ] **Step 4: Wire the module and thread the manager through**

In `src/server/mod.rs`:
- Add `mod terminal_ws;` under `pub mod tools;`.
- Add `use crate::terminal::TerminalManager;` and change signatures:

```rust
pub async fn serve(
    listener: TcpListener,
    sessions: Arc<Sessions>,
    terminals: Arc<TerminalManager>,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    serve_with_manager(
        listener,
        sessions,
        terminals,
        shutdown,
        Arc::new(LocalSessionManager::default()),
    )
    .await
}
```

`serve_with_manager` gains the same `terminals: Arc<TerminalManager>` third parameter, and its router line becomes:

```rust
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            transport_connections,
            track_transport_disconnect,
        ))
        .merge(terminal_ws::routes(terminals));
```

(The `.merge` sits after `.layer` so the disconnect middleware stays scoped to `/mcp`.)

In `src/main.rs`, create the manager right after `Sessions::open_from_cli` and pass it to `serve`; add a kill-all backstop after the UI returns (before `sessions.save_all()`):

```rust
    let terminals = nodestorm::terminal::TerminalManager::new();
```

```rust
                    runtime.block_on(nodestorm::server::serve(
                        listener,
                        sessions,
                        terminals,
                        shutdown_rx,
                    ))
```

(clone `terminals` into the server thread closure the same way `sessions` is cloned), and after the `if cli.headless { ... } else { ... }` block:

```rust
    // The UI (or ctrl-c) is done: no PTY child outlives the app.
    terminals.kill_all();
```

Update the two existing callers the compiler flags:
- `src/server/mod.rs` test `serve_waits_for_shutdown`: pass `crate::terminal::TerminalManager::new()`.
- `tests/mcp_roundtrip.rs:53`: pass `nodestorm::terminal::TerminalManager::new()`.

- [ ] **Step 5: Run the tests**

Run: `cargo test terminal_ws && cargo test --test mcp_roundtrip && cargo test --lib server`
Expected: all PASS.

- [ ] **Step 6: Gate and commit**

Run: `cargo fmt --all && cargo clippy --all-targets --locked -- -D warnings && cargo test --all-targets --locked`
Expected: clean.

```powershell
git add Cargo.toml Cargo.lock src/server/mod.rs src/server/terminal_ws.rs src/main.rs tests/mcp_roundtrip.rs
git commit -m "Serve token-gated terminal WebSockets from the embedded server"
```

---

### Task 3: Vendored Ferroterm and the terminal dock UI

**Files:**
- Create: `assets/ferroterm/<esm-entry>.js`, `assets/ferroterm/<wasm-file>.wasm` (exact names from the npm tarball; possibly a `.d.ts` kept for reference), `assets/ferroterm/mount.js`, `assets/ferroterm/README.md`
- Create: `src/ui/terminal_panel.rs`
- Modify: `src/server/terminal_ws.rs` (add `/terminal/assets/{file}` static routes)
- Modify: `src/ui/mod.rs` (module decl, context types, `focus_terminal`, `launch` signature)
- Modify: `src/ui/app.rs` (contexts, terminals signal, dock mount)
- Modify: `src/main.rs` (pass `terminals` to `ui::launch`)
- Modify: `assets/main.css` (dock styles)

**Interfaces:**
- Consumes: `TerminalManager` (`list`, `subscribe`, `token`, `close`, `status`), `TerminalInfo`, `TerminalStatus`; `cli.port`; wire protocol and `routes()` from Task 2.
- Produces (used by Tasks 4–6):
  - `ui::launch(sessions, terminals: Arc<TerminalManager>, cli)` — new middle parameter.
  - In `src/ui/mod.rs`:
    - `#[derive(Clone, Copy)] pub(crate) struct Terminals(pub Signal<Vec<crate::terminal::TerminalInfo>>);`
    - `#[derive(Clone, Copy)] pub(crate) struct TerminalPanel { pub open: Signal<bool>, pub focused: Signal<Option<String>>, pub confirm_close: Signal<Option<String>>, pub quit_confirm: Signal<bool> }`
    - `pub(crate) fn focus_terminal(panel: &TerminalPanel, id: &str)` — opens the dock and selects the tab.
    - `pub(crate) fn terminal_for(terminals: &[crate::terminal::TerminalInfo], name: &str) -> bool` — true when an open tab has exactly that id.
  - `terminal_panel::TerminalDock` component (no props; reads contexts).
  - DOM contract: each tab body div has `id="term-{terminal-id}"`; `window.__nsTerms[id].dispose()` tears a tab down.

- [ ] **Step 1: Vendor Ferroterm (pinned, no CDN at runtime)**

> Downloads the `ferroterm` npm package tarball (MIT, ~65-73 KB gzip; user-directed switch from xterm.js, download approved at dispatch).

```powershell
cd $env:TEMP
npm pack ferroterm
tar -xf (Get-ChildItem ferroterm-*.tgz | Select-Object -First 1).Name
```

Inspect `package/package.json` (`exports` / `module` / `main` / `files`) and the `package/` tree to identify (a) the ES-module entry `.js` and any chunk files it imports, (b) the `.wasm` binary, (c) the `.d.ts` (`ferroterm.d.ts` per upstream docs). Copy those into `assets/ferroterm/` in the repo, preserving the tarball's file names, and note the exact version from `package.json`. Read the `.d.ts` now — it is the authority for the API used in Step 2 (constructor options `cols, rows, scrollback, fontSize, renderer ('webgl'|'canvas'), autoFit, wasmUrl, theme`; methods `write`, `onData`, `fit`; whether a dispose/destroy method and a resize event exist). If the packaged module hardcodes a relative WASM fetch path instead of honoring a `wasmUrl` option, report DONE_WITH_CONCERNS with what the `.d.ts` actually exposes.

Create `assets/ferroterm/README.md`:

```markdown
# Vendored Ferroterm

- Source: `ferroterm` npm package, version <VERSION> (fill in), MIT.
  https://datanoisetv.github.io/ferroterm/
- Files: <list the copied .js/.wasm/.d.ts names> — served at runtime from the
  embedded loopback server under /terminal/assets/, never from a CDN.
- Update by re-vendoring a newer pinned version.
- `mount.js` is ours: the per-tab glue template (see `src/ui/terminal_panel.rs`).
```

- [ ] **Step 1b: Serve the assets from the embedded server**

In `src/server/terminal_ws.rs`, embed the vendored files and extend `routes()` (substitute the real file names everywhere `<esm-entry>`/`<wasm-file>` appear):

```rust
const FERROTERM_JS: &[u8] = include_bytes!("../../assets/ferroterm/<esm-entry>.js");
const FERROTERM_WASM: &[u8] = include_bytes!("../../assets/ferroterm/<wasm-file>.wasm");

async fn terminal_asset(Path(file): Path<String>) -> Response {
    let (bytes, mime): (&[u8], &str) = match file.as_str() {
        "<esm-entry>.js" => (FERROTERM_JS, "text/javascript"),
        "<wasm-file>.wasm" => (FERROTERM_WASM, "application/wasm"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [
            (axum::http::header::CONTENT_TYPE, mime),
            // Module imports and WASM fetches from the webview's custom-scheme
            // origin are CORS-gated; the assets are public code, no secrets.
            (axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        bytes,
    )
        .into_response()
}
```

Add `.route("/terminal/assets/{file}", get(terminal_asset))` to the router in `routes()`. (If the entry imports additional chunk files, add them to the match with `text/javascript`.) Tests in the same file's test module, reusing the `serve_terminal_routes` helper (reqwest is already a dev-dependency):

```rust
    #[tokio::test]
    async fn ferroterm_assets_are_served_with_mime_and_cors() {
        let manager = TerminalManager::new();
        let base = serve_terminal_routes(manager).await;
        let http = base.replace("ws://", "http://");

        let js = reqwest::get(format!("{http}/terminal/assets/<esm-entry>.js"))
            .await
            .unwrap();
        assert2::assert!((js.status().as_u16()) == (200));
        assert2::assert!(
            (js.headers()["content-type"].to_str().unwrap()) == ("text/javascript")
        );
        assert2::assert!(
            (js.headers()["access-control-allow-origin"].to_str().unwrap()) == ("*")
        );

        let wasm = reqwest::get(format!("{http}/terminal/assets/<wasm-file>.wasm"))
            .await
            .unwrap();
        assert2::assert!((wasm.status().as_u16()) == (200));
        assert2::assert!(
            (wasm.headers()["content-type"].to_str().unwrap()) == ("application/wasm")
        );

        let missing = reqwest::get(format!("{http}/terminal/assets/evil.js"))
            .await
            .unwrap();
        assert2::assert!((missing.status().as_u16()) == (404));
    }
```

- [ ] **Step 2: Write the mount glue template**

Create `assets/ferroterm/mount.js`. `__ID__`, `__PORT__`, `__TOKEN__` are replaced by Rust before eval (terminal ids are slug-safe so plain string replacement is sound); the two vendored file names are hardcoded — substitute the real names from Step 1. Adjust the Ferroterm calls to what `ferroterm.d.ts` actually declares (this template assumes `Ferroterm.create`, `write`, `onData`, `fit`, `cols`/`rows`; verify each, and use the real dispose/destroy method if one exists).

```javascript
(function () {
  const id = "__ID__";
  const host = document.getElementById("term-" + id);
  if (!host || host.dataset.mounted) { return; }
  host.dataset.mounted = "1";
  const base = "http://127.0.0.1:__PORT__/terminal/assets/";
  import(base + "<esm-entry>.js")
    .then(async function (mod) {
      const Ferroterm = mod.Ferroterm || mod.default;
      const options = {
        scrollback: 5000,
        fontSize: 13,
        autoFit: true,
        wasmUrl: base + "<wasm-file>.wasm",
      };
      let term;
      try {
        term = await Ferroterm.create(host, { renderer: "webgl", ...options });
      } catch (_e) {
        // WebGL unavailable in this WebView2 session: fall back to Canvas2D.
        term = await Ferroterm.create(host, { renderer: "canvas", ...options });
      }
      let ws = null;
      let closed = false;
      function sendResize() {
        if (ws && ws.readyState === 1) {
          ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }));
        }
      }
      function connect() {
        if (closed || !document.getElementById("term-" + id)) { return; }
        ws = new WebSocket("ws://127.0.0.1:__PORT__/terminal/" + id + "/ws?token=__TOKEN__");
        ws.binaryType = "arraybuffer";
        ws.onopen = sendResize;
        ws.onmessage = function (event) {
          if (typeof event.data !== "string") { term.write(new Uint8Array(event.data)); }
        };
        ws.onclose = function () {
          if (!closed) {
            term.write(new TextEncoder().encode("\r\n\x1b[90m[disconnected - reconnecting]\x1b[0m\r\n"));
            setTimeout(connect, 1000);
          }
        };
      }
      term.onData(function (data) {
        if (ws && ws.readyState === 1) {
          ws.send(typeof data === "string" ? new TextEncoder().encode(data) : data);
        }
      });
      new ResizeObserver(function () {
        // Hidden tabs (display:none) must not fit to a 0x0 box.
        if (host.offsetParent !== null) {
          term.fit();
          sendResize();
        }
      }).observe(host);
      connect();
      window.__nsTerms = window.__nsTerms || {};
      window.__nsTerms[id] = {
        dispose: function () {
          closed = true;
          try { if (ws) { ws.close(); } } catch (_e) {}
          if (typeof term.dispose === "function") { term.dispose(); }
          else { host.replaceChildren(); }
          delete window.__nsTerms[id];
        },
      };
    })
    .catch(function (err) {
      host.textContent = "terminal failed to load: " + err;
    });
})();
```

Note: on reconnect the server replays the scrollback ring into the same live terminal; if Ferroterm exposes a reset/clear method in the `.d.ts`, call it in `ws.onopen` before the replay arrives so replayed bytes don't duplicate the visible screen.

- [ ] **Step 3: Write failing unit tests for the UI helpers**

In `src/ui/mod.rs`, add to the existing `tests` module:

```rust
    #[test]
    fn terminal_lookup_matches_exact_ids_only() {
        let terminals = vec![
            crate::terminal::TerminalInfo {
                id: "claude-cache-redesign".into(),
                status: crate::terminal::TerminalStatus::Running,
            },
            crate::terminal::TerminalInfo {
                id: "pi-api-v2".into(),
                status: crate::terminal::TerminalStatus::Exited(0),
            },
        ];
        assert2::assert!(terminal_for(&terminals, "claude-cache-redesign"));
        // Exited-but-open tabs still count; unknown and partial names do not.
        assert2::assert!(terminal_for(&terminals, "pi-api-v2"));
        assert2::assert!(!terminal_for(&terminals, "claude"));
        assert2::assert!(!terminal_for(&terminals, "codex-cache-redesign"));
    }

    #[test]
    fn mount_template_placeholders_are_all_known() {
        let js = include_str!("../../assets/ferroterm/mount.js");
        for placeholder in ["__ID__", "__PORT__", "__TOKEN__"] {
            assert2::assert!(js.contains(placeholder));
        }
        // No stray `__UPPERCASE` template markers beyond the three we
        // substitute (`__nsTerms` and friends are legitimate identifiers).
        let substituted = js
            .replace("__ID__", "x")
            .replace("__PORT__", "1")
            .replace("__TOKEN__", "t");
        let stray = substituted
            .as_bytes()
            .windows(3)
            .any(|w| w[0] == b'_' && w[1] == b'_' && w[2].is_ascii_uppercase());
        assert2::assert!(!stray);
        // The vendored file names must be substituted, not left as templates.
        assert2::assert!(!substituted.contains("<esm-entry>"));
        assert2::assert!(!substituted.contains("<wasm-file>"));
    }
```

Run: `cargo test --lib ui -- terminal_lookup mount_template`
Expected: FAIL (missing `terminal_for`, missing file until Step 2 is committed — the file exists now, so just `terminal_for`).

- [ ] **Step 4: Add the context types and helpers**

In `src/ui/mod.rs` (near the other context newtypes):

```rust
/// Open terminal tabs, refreshed from the [`crate::terminal::TerminalManager`]
/// generation watch.
#[derive(Clone, Copy)]
pub(crate) struct Terminals(pub Signal<Vec<crate::terminal::TerminalInfo>>);

/// Dock state: visibility, the focused tab, the pending tab-close
/// confirmation, and the pending quit confirmation.
#[derive(Clone, Copy)]
pub(crate) struct TerminalPanel {
    pub open: Signal<bool>,
    pub focused: Signal<Option<String>>,
    pub confirm_close: Signal<Option<String>>,
    pub quit_confirm: Signal<bool>,
}

/// Expand the dock and select `id` — the single entry point every
/// agent-name click target uses.
pub(crate) fn focus_terminal(panel: &TerminalPanel, id: &str) {
    let mut open = panel.open;
    let mut focused = panel.focused;
    open.set(true);
    focused.set(Some(id.to_owned()));
}

/// Whether an open terminal tab (running or exited) has exactly this id.
pub(crate) fn terminal_for(terminals: &[crate::terminal::TerminalInfo], name: &str) -> bool {
    terminals.iter().any(|t| t.id == name)
}
```

Add `mod terminal_panel;` to the module list. Change `launch`:

```rust
pub fn launch(
    sessions: Arc<crate::sessions::Sessions>,
    terminals: Arc<crate::terminal::TerminalManager>,
    cli: Cli,
) {
```

and add `.with_context(terminals)` beside the existing `.with_context(sessions)`. In `src/main.rs` pass `terminals.clone()` as the new middle argument.

- [ ] **Step 5: Build the dock component**

Create `src/ui/terminal_panel.rs`:

```rust
//! Bottom terminal dock: one Ferroterm tab per launched agent.
//!
//! Tab bodies stay mounted (hidden tabs use display:none) so terminal state and
//! the WebSocket survive tab switches and dock collapse; the fit addon
//! re-measures when a tab becomes visible again.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::terminal::{TerminalManager, TerminalStatus};

const MOUNT_TEMPLATE: &str = include_str!("../../assets/ferroterm/mount.js");

fn mount_js(id: &str, port: u16, token: &str) -> String {
    MOUNT_TEMPLATE
        .replace("__ID__", id)
        .replace("__PORT__", &port.to_string())
        .replace("__TOKEN__", token)
}

fn dispose_js(id: &str) -> String {
    format!("if (window.__nsTerms && window.__nsTerms[\"{id}\"]) {{ window.__nsTerms[\"{id}\"].dispose(); }}")
}

#[component]
pub fn TerminalDock() -> Element {
    let cli = use_context::<crate::cli::Cli>();
    let manager = use_context::<Arc<TerminalManager>>();
    let terminals = use_context::<super::Terminals>().0;
    let panel = use_context::<super::TerminalPanel>();
    let mut open = panel.open;
    let mut focused = panel.focused;
    let mut confirm_close = panel.confirm_close;

    let list = terminals.read().clone();
    if list.is_empty() {
        return rsx! {};
    }
    // A stale focus (closed tab) falls back to the first tab.
    let focused_id = focused
        .read()
        .clone()
        .filter(|id| list.iter().any(|t| &t.id == id))
        .unwrap_or_else(|| list[0].id.clone());

    rsx! {
        div { class: if open() { "term-dock" } else { "term-dock collapsed" },
            div { class: "term-tabs",
                for info in list.iter().cloned() {
                    div {
                        key: "{info.id}",
                        class: if info.id == focused_id { "term-tab active" } else { "term-tab" },
                        onclick: {
                            let id = info.id.clone();
                            move |_| focused.set(Some(id.clone()))
                        },
                        span {
                            class: match info.status {
                                TerminalStatus::Running => "term-dot running",
                                TerminalStatus::Exited(_) => "term-dot exited",
                            },
                            title: match info.status {
                                TerminalStatus::Running => "running".to_owned(),
                                TerminalStatus::Exited(code) => format!("exited ({code})"),
                            },
                            "●"
                        }
                        span {
                            class: "term-tab-name",
                            style: "color: {super::agent_color(&info.id)};",
                            "{info.id}"
                        }
                        button {
                            class: "term-tab-close",
                            aria_label: "Close terminal {info.id}",
                            onclick: {
                                let id = info.id.clone();
                                let status = info.status;
                                let manager = manager.clone();
                                move |event: MouseEvent| {
                                    event.stop_propagation();
                                    if status == TerminalStatus::Running {
                                        confirm_close.set(Some(id.clone()));
                                    } else {
                                        close_tab(&manager, &id, &mut focused);
                                    }
                                }
                            },
                            "×"
                        }
                    }
                }
                button {
                    class: "term-collapse",
                    aria_label: if open() { "Collapse terminal panel" } else { "Expand terminal panel" },
                    onclick: move |_| open.toggle(),
                    if open() { "▾" } else { "▴" }
                }
            }
            div { class: "term-body",
                for info in list.iter() {
                    div {
                        key: "{info.id}",
                        id: "term-{info.id}",
                        class: "term-host",
                        style: if info.id == focused_id { "" } else { "display: none;" },
                        onmounted: {
                            let js = mount_js(&info.id, cli.port, manager.token());
                            move |_| {
                                document::eval(&js);
                            }
                        },
                    }
                }
            }
            if let Some(id) = confirm_close() {
                div { class: "term-confirm-overlay",
                    div { class: "term-confirm", role: "alertdialog",
                        p { "The agent in “{id}” is still running. Stop it and close the tab?" }
                        div { class: "term-confirm-actions",
                            button {
                                class: "btn",
                                onclick: move |_| confirm_close.set(None),
                                "Cancel"
                            }
                            button {
                                class: "btn btn-primary",
                                onclick: {
                                    let manager = manager.clone();
                                    move |_| {
                                        close_tab(&manager, &id, &mut focused);
                                        confirm_close.set(None);
                                    }
                                },
                                "Stop agent"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn close_tab(manager: &Arc<TerminalManager>, id: &str, focused: &mut Signal<Option<String>>) {
    document::eval(&dispose_js(id));
    manager.close(id);
    if focused.read().as_deref() == Some(id) {
        focused.set(None);
    }
}
```

Adjust closure captures until borrows check (`id` clones per closure); keep the structure.

- [ ] **Step 6: Wire the dock into the app**

In `src/ui/app.rs`:
- No global script/style includes are needed: `mount.js` dynamically imports the Ferroterm module from the loopback server the first time a tab mounts.
- Provide contexts near the other providers:

```rust
    let terminal_manager = use_context::<Arc<crate::terminal::TerminalManager>>();
    let mut terminals =
        use_context_provider(|| super::Terminals(Signal::new(terminal_manager.list()))).0;
    use_context_provider(|| super::TerminalPanel {
        open: Signal::new(false),
        focused: Signal::new(None),
        confirm_close: Signal::new(None),
        quit_confirm: Signal::new(false),
    });
```

- Refresh loop beside the connections `use_future` (same shape):

```rust
    use_future({
        let manager = terminal_manager.clone();
        move || {
            let manager = manager.clone();
            async move {
                let mut changes = manager.subscribe();
                terminals.set(manager.list());
                while changes.changed().await.is_ok() {
                    terminals.set(manager.list());
                }
            }
        }
    });
```

- Mount the dock inside the `.app` div, directly after the `div { class: "main", ... }` block closes:

```rust
            super::terminal_panel::TerminalDock {}
```

(`terminal_panel` needs `pub` visibility on `TerminalDock` only — module stays private like its siblings; use `super::terminal_panel::TerminalDock` via a `use` in app.rs matching the existing import style: `use super::terminal_panel::TerminalDock;` then `TerminalDock {}`.)

- [ ] **Step 7: Dock styles**

Append to `assets/main.css` (after the activity feed section; reuse the existing variables):

```css
/* ---------- terminal dock ---------- */

.term-dock {
  display: flex;
  flex-direction: column;
  height: 40vh;
  min-height: 0;
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
}

.term-dock.collapsed {
  display: none;
}

.term-tabs {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 2px 6px;
  border-bottom: 1px solid var(--border);
  font-family: var(--font-mono);
  font-size: 12px;
}

.term-tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  border-radius: 6px 6px 0 0;
  cursor: pointer;
  color: var(--text-dim);
}

.term-tab.active {
  background: var(--bg-card);
  color: var(--text);
}

.term-dot.running { color: var(--volt); }
.term-dot.exited { color: var(--text-dim); }

.term-tab-close {
  background: none;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
  padding: 0 2px;
}

.term-tab-close:hover { color: var(--text); }

.term-collapse {
  margin-left: auto;
  background: none;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
}

.term-body {
  flex: 1;
  min-height: 0;
  position: relative;
}

.term-host {
  position: absolute;
  inset: 0;
  padding: 4px 0 0 6px;
  /* ponytail: fixed Ferroterm default (dark) theme; themed terminals need a
     setTheme call on palette switch. */
  background: #000;
}

.term-confirm-overlay {
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  background: rgba(0, 0, 0, 0.45);
  z-index: 60;
}

.term-confirm {
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 16px;
  max-width: 420px;
  box-shadow: var(--shadow);
}

.term-confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}

.agent-clickable {
  cursor: pointer;
  text-decoration: underline dotted;
}
```

- [ ] **Step 8: Run tests and build**

Run: `cargo test --lib ui && cargo test terminal_ws && cargo build --locked`
Expected: the Step 3 and Step 1b tests PASS; the app compiles.

- [ ] **Step 9: Gate and commit**

Run: `cargo fmt --all && cargo clippy --all-targets --locked -- -D warnings && cargo test --all-targets --locked`
Expected: clean. (Manual visual verification happens at the end of Task 4, when a launch can populate the dock.)

```powershell
git add assets/ferroterm assets/main.css src/server/terminal_ws.rs src/ui src/main.rs
git commit -m "Add the terminal dock UI with vendored Ferroterm"
```

---

### Task 4: Integrated launch path

**Files:**
- Modify: `src/ui/agent_launcher.rs`

**Interfaces:**
- Consumes: `TerminalManager::spawn`; `focus_terminal`, `TerminalPanel` from Task 3; existing launcher functions.
- Produces: `LaunchOutcome::Started { terminal: Option<String> }` (terminal id when integrated); draft field `integrated: bool` defaulting to `true`; retry re-runs whichever run target failed.

- [ ] **Step 1: Write failing tests**

In the `tests` module of `src/ui/agent_launcher.rs`:

```rust
    #[test]
    fn draft_defaults_to_integrated_terminal() {
        assert2::assert!(LaunchDraft::default().integrated);
    }

    #[test]
    fn integrated_launch_spawns_a_terminal_and_reports_its_id() {
        let sessions = crate::sessions::Sessions::open(
            tmp_path("integrated-sessions"),
            None,
        )
        .unwrap();
        let terminals = crate::terminal::TerminalManager::new();
        // A repo-free request exercises only the terminal branch: SSH targets
        // skip local git preparation, and `ssh` exists on dev machines and CI.
        let request = LaunchRequest {
            session_name: "Integrated Test".into(),
            task: "do things".into(),
            agent: AgentKind::Claude,
            target: LaunchTarget::Ssh { alias: "nodestorm-test-invalid-host".into() },
            repository: "/srv/repo".into(),
            branch: "nodestorm/integrated-test".into(),
            git_mode: GitMode::NewWorktree { path: "/srv/repo-worktrees/x".into() },
            mcp_port: 4747,
        };

        let outcome = perform_launch(
            request,
            sessions,
            false,
            Some(terminals.clone()),
        );

        let LaunchOutcome::Started { terminal: Some(id) } = outcome else {
            panic!("expected an integrated start");
        };
        assert2::assert!((id) == ("claude-integrated-test"));
        assert2::assert!(terminals.status(&id).is_some());
        terminals.close(&id);
        std::fs::remove_dir_all(tmp_path("integrated-sessions")).ok();
    }
```

(`Sessions::open` signature: copy the call shape used in `src/server/mod.rs` tests — `Sessions::open(root, None)`. The ssh child fails to resolve the host and exits inside the PTY; that is fine — spawn succeeded, which is what `Started` means.)

Run: `cargo test --lib agent_launcher -- integrated`
Expected: FAIL to compile (`integrated` field, `Started` shape, `perform_launch` arity).

- [ ] **Step 2: Implement**

In `src/ui/agent_launcher.rs`:

1. `LaunchDraft` gains `integrated: bool`, default `true`.
2. `LaunchOutcome::Started` becomes `Started { terminal: Option<String> }`.
3. `perform_launch` gains a fourth parameter `terminals: Option<Arc<crate::terminal::TerminalManager>>` (`Some` = integrated). Replace the tail:

```rust
    match terminals {
        Some(manager) => {
            let terminal_id = format!("{}-{slug}", request.agent.id());
            match manager.spawn(&terminal_id, &command) {
                Ok(()) => LaunchOutcome::Started {
                    terminal: Some(terminal_id),
                },
                Err(err) => LaunchOutcome::TerminalFailed {
                    message: err.to_string(),
                    retained,
                    command,
                    terminal: Some(terminal_id),
                },
            }
        }
        None => match open_terminal(&command) {
            Ok(()) => LaunchOutcome::Started { terminal: None },
            Err(err) => LaunchOutcome::TerminalFailed {
                message: err.to_string(),
                retained,
                command,
                terminal: None,
            },
        },
    }
```

(`LaunchOutcome::TerminalFailed` gains the `terminal: Option<String>` field — step 4 below consumes it for retry.)

The terminal id `{agent-id}-{slug}` is exactly the agent identity `compose_prompt` instructs the agent to use in MCP calls — that equality is what makes attribution names clickable later.

4. `start_attempt`: add parameters `terminals: Option<Arc<...>>` and `panel: super::TerminalPanel`; thread `terminals` into `perform_launch`; in the `Started { terminal }` arm, before `signals.open.set(false)`:

```rust
                if let Some(id) = terminal {
                    super::focus_terminal(&panel, &id);
                }
```

5. `AgentLauncher` component: read the contexts —

```rust
    let terminal_manager = use_context::<Arc<crate::terminal::TerminalManager>>();
    let panel = use_context::<super::TerminalPanel>();
```

pass `draft.read().integrated.then(|| terminal_manager.clone())` and `panel` to `start_attempt`.

Retry must reuse the same run target. Change the retry signal to carry the terminal id (`Some` = integrated):

```rust
    let retry: Signal<Option<(CommandSpec, String, Option<String>)>> = use_signal(|| None);
```

`TerminalFailed` handling in `start_attempt` stores that triple — it needs the terminal id, so add it to the outcome: `TerminalFailed { message, retained, command, terminal: Option<String> }` (the integrated branch fills `Some(terminal_id)`, the system branch `None`), and the `start_attempt` arm does `signals.retry.set(Some((command, repo, terminal)))`.

The "Retry terminal" onclick body becomes:

```rust
                                let (command, repo, terminal) = retry.read().clone().expect("retry command");
                                let cli = cli.clone();
                                let manager = terminal_manager.clone();
                                running.set(true);
                                error.set(None);
                                spawn(async move {
                                    let attempt = tokio::task::spawn_blocking({
                                        let terminal = terminal.clone();
                                        move || match &terminal {
                                            Some(id) => manager.spawn(id, &command).map_err(|e| e.to_string()),
                                            None => open_terminal(&command).map_err(|e| e.to_string()),
                                        }
                                    })
                                    .await;
                                    running.set(false);
                                    match attempt {
                                        Ok(Ok(())) => {
                                            if let Some(id) = &terminal {
                                                super::focus_terminal(&panel, id);
                                            }
                                            remember_repository(&mut prefs.write(), &cli, &repo);
                                            open.set(false);
                                        }
                                        Ok(Err(message)) => error.set(Some(message)),
                                        Err(err) => error.set(Some(format!("launcher worker failed: {err}"))),
                                    }
                                });
```

6. Dialog UI: add a "Run in" fieldset after the "Target" fieldset, same markup pattern:

```rust
                    fieldset { class: "agent-launch-field agent-launch-options",
                        legend { "Run in" }
                        label {
                            input {
                                r#type: "radio",
                                name: "agent-run-in",
                                checked: draft.read().integrated,
                                oninput: move |_| draft.write().integrated = true,
                            }
                            "Integrated terminal"
                        }
                        label {
                            input {
                                r#type: "radio",
                                name: "agent-run-in",
                                checked: !draft.read().integrated,
                                oninput: move |_| draft.write().integrated = false,
                            }
                            "System terminal"
                        }
                    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib agent_launcher`
Expected: all PASS (old tests updated for the new `Started` shape where they match on it).

- [ ] **Step 4: Manual smoke test (first end-to-end run)**

Run: `cargo run` — then in the app: Start agent → fill a real local repo path, session name, task → keep "Integrated terminal" → Create branch & start agent.
Expected: the dock expands with a `claude-…` tab showing Claude Code's TUI; typing works; collapse hides the dock. Also verify "System terminal" still opens an external console. Record any deviation before proceeding.

- [ ] **Step 5: Gate and commit**

Run: `cargo fmt --all && cargo clippy --all-targets --locked -- -D warnings && cargo test --all-targets --locked`
Expected: clean.

```powershell
git add src/ui/agent_launcher.rs
git commit -m "Launch agents in the integrated terminal by default"
```

---

### Task 5: Topbar chips and clickable agent names

**Files:**
- Modify: `src/ui/topbar.rs`, `src/ui/activity.rs`, `src/ui/node_card.rs`, `src/ui/choice_panel.rs`
- Modify: `assets/main.css` (chip styles)

**Interfaces:**
- Consumes: `Terminals`, `TerminalPanel`, `focus_terminal`, `terminal_for`, `agent_color` from `src/ui/mod.rs`.
- Produces: no new APIs. Behavior: every rendered agent name whose text equals an open terminal id gets class `agent-clickable`, `title: "Focus terminal"`, and an onclick that calls `focus_terminal` (with `stop_propagation` inside node cards). The queued-changes and questions panels render no agent names today — no change there.

- [ ] **Step 1: Topbar chips**

In `src/ui/topbar.rs`, inside the `TopBar` component read the contexts:

```rust
    let terminals = use_context::<super::Terminals>().0;
    let panel = use_context::<super::TerminalPanel>();
```

Locate the topbar's root rsx and insert a chips container right after the fused status chip element (grep for the connection-state chip markup; it renders `connection_state_label`). Markup:

```rust
                div { class: "term-chips",
                    for info in terminals.read().iter().cloned() {
                        button {
                            key: "{info.id}",
                            class: "term-chip",
                            title: match info.status {
                                crate::terminal::TerminalStatus::Running => "Focus terminal (running)",
                                crate::terminal::TerminalStatus::Exited(_) => "Focus terminal (exited)",
                            },
                            onclick: {
                                let id = info.id.clone();
                                move |_| super::focus_terminal(&panel, &id)
                            },
                            span {
                                class: match info.status {
                                    crate::terminal::TerminalStatus::Running => "term-dot running",
                                    crate::terminal::TerminalStatus::Exited(_) => "term-dot exited",
                                },
                                "●"
                            }
                            span { style: "color: {super::agent_color(&info.id)};", "{info.id}" }
                        }
                    }
                }
```

Chip styles appended to `assets/main.css` after the dock section:

```css
.term-chips {
  display: flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-mono);
  font-size: 11px;
}

.term-chip {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--bg-card);
  color: var(--text-dim);
  cursor: pointer;
}

.term-chip:hover { background: var(--bg-card-hover); color: var(--text); }
```

- [ ] **Step 2: Clickable attributions**

Same pattern in three files; read the two contexts at the top of the component, then extend the existing agent-name span.

`src/ui/activity.rs` — inside `ActivityFeed` add the context reads; the agent span (currently lines 49–55) becomes:

```rust
                    if let Some(agent) = &entry.agent {
                        {
                            let clickable = super::terminal_for(&terminals.read(), agent);
                            let id = agent.clone();
                            rsx! {
                                span {
                                    class: if clickable { "activity-agent agent-clickable" } else { "activity-agent" },
                                    style: "color: {super::agent_color(agent)};",
                                    title: if clickable { "Focus terminal" } else { "" },
                                    onclick: move |_| {
                                        if clickable {
                                            super::focus_terminal(&panel, &id);
                                        }
                                    },
                                    "{agent}"
                                }
                            }
                        }
                    }
```

`src/ui/node_card.rs` — the `node-agent` span (lines 185–191) gets the same treatment, plus `event.stop_propagation()` in the onclick before focusing (a card click selects the node; the chip click must not).

`src/ui/choice_panel.rs` — the `node-agent` span (lines 159–169): same as activity (no propagation concern).

Note: both components live under `Canvas`/`App` providers, so `use_context` resolves; if a component is also used outside those providers (it is not today), `try_use_context` would be the fallback — do not add it speculatively.

- [ ] **Step 3: Build and manually verify**

Run: `cargo run` — launch an integrated agent, collapse the dock, then:
- Click the topbar chip → dock expands on the right tab.
- Once the agent produces activity or proposes nodes, click its name in the feed and on a node card → dock focuses; card click still selects the node when clicking outside the name.
Expected: all three paths restore the terminal.

- [ ] **Step 4: Gate and commit**

Run: `cargo fmt --all && cargo clippy --all-targets --locked -- -D warnings && cargo test --all-targets --locked`
Expected: clean.

```powershell
git add src/ui/topbar.rs src/ui/activity.rs src/ui/node_card.rs src/ui/choice_panel.rs assets/main.css
git commit -m "Focus agent terminals from topbar chips and name attributions"
```

---

### Task 6: Quit confirmation

**Files:**
- Modify: `src/ui/app.rs`

**Interfaces:**
- Consumes: `TerminalPanel.quit_confirm`, `TerminalManager::{running_count, kill_all}`, dioxus desktop `use_wry_event_handler`, `WindowCloseBehaviour`, `DesktopService::{set_close_behavior, close}`. (The `main.rs` kill-all backstop landed in Task 2; tab-close confirmation landed in Task 3.)
- Produces: closing the window with running agents shows an in-app confirm; confirming kills all PTYs and closes; declining keeps the app open.

- [ ] **Step 1: Implement the close interception**

In `src/ui/app.rs`, after the terminal contexts from Task 3:

```rust
    let desktop = dioxus::desktop::use_window();
    let panel = use_context::<super::TerminalPanel>();
    let mut quit_confirm = panel.quit_confirm;

    // While agents run, a close request hides the window (tao cannot veto a
    // close); the handler below flips it back visible with the confirm open.
    use_effect({
        let desktop = desktop.clone();
        let manager = terminal_manager.clone();
        move || {
            let _ = terminals.read(); // re-run on terminal changes
            let behaviour = if manager.running_count() > 0 {
                dioxus::desktop::WindowCloseBehaviour::WindowHides
            } else {
                dioxus::desktop::WindowCloseBehaviour::WindowCloses
            };
            desktop.set_close_behavior(behaviour);
        }
    });
    dioxus::desktop::use_wry_event_handler({
        let manager = terminal_manager.clone();
        move |event, _| {
            use dioxus::desktop::tao::event::{Event, WindowEvent};
            if let Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } = event
                && manager.running_count() > 0
            {
                quit_confirm.set(true);
            }
        }
    });
    use_effect({
        let desktop = desktop.clone();
        move || {
            if quit_confirm() {
                desktop.window.set_visible(true);
            }
        }
    });
```

Dialog, next to the launcher mount inside the `.app` div (reuse the Task 3 confirm classes):

```rust
            if quit_confirm() {
                div { class: "term-confirm-overlay",
                    div { class: "term-confirm", role: "alertdialog",
                        p {
                            {
                                let n = use_context::<Arc<crate::terminal::TerminalManager>>().running_count();
                                format!("{n} agent{} still running. Quit and stop {}?",
                                    if n == 1 { " is" } else { "s are" },
                                    if n == 1 { "it" } else { "them" })
                            }
                        }
                        div { class: "term-confirm-actions",
                            button {
                                class: "btn",
                                onclick: move |_| quit_confirm.set(false),
                                "Keep running"
                            }
                            button {
                                class: "btn btn-primary",
                                onclick: {
                                    let desktop = desktop.clone();
                                    let manager = terminal_manager.clone();
                                    move |_| {
                                        manager.kill_all();
                                        desktop.set_close_behavior(
                                            dioxus::desktop::WindowCloseBehaviour::WindowCloses,
                                        );
                                        desktop.close();
                                    }
                                },
                                "Quit and stop agents"
                            }
                        }
                    }
                }
            }
```

(Adapt captures to what the borrow checker requires; `terminal_manager` is the `Arc` read in Task 3's wiring. A brief window-hide flicker before the confirm reappears is expected and acceptable — tao cannot veto a close request.)

- [ ] **Step 2: Manual verification**

Run: `cargo run` — launch an integrated agent, click the window ×.
Expected: window stays (brief flicker allowed), confirm dialog shows; "Keep running" keeps everything alive; × again then "Quit and stop agents" exits and the agent process is gone (check Task Manager / `ps`). With no agents running, × closes immediately.

- [ ] **Step 3: Gate and commit**

Run: `cargo fmt --all && cargo clippy --all-targets --locked -- -D warnings && cargo test --all-targets --locked`
Expected: clean.

```powershell
git add src/ui/app.rs
git commit -m "Confirm before quitting with running integrated agents"
```

---

### Task 7: Full verification

**Files:** none (verification only; fix regressions where found).

- [ ] **Step 1: Full gates**

Run each; expected: exit 0.

```powershell
cargo build --all-targets --locked
cargo test --all-targets --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
node --test tests/plugin_contract.mjs tests/host_adapters.mjs tests/installers.mjs tests/release_gates.mjs
```

- [ ] **Step 2: Manual completion checklist (spec's completion criteria)**

With `cargo run`:

1. Integrated launch of Claude Code opens its TUI in a new dock tab (done in Task 4 — re-verify).
2. If an SSH alias is available: SSH launch runs in the panel, auth prompts appear, agent starts. If no SSH host is available, note that this criterion was not manually exercised.
3. Launch a second agent → two tabs run concurrently, switching preserves both.
4. Collapse the dock → topbar chip click and activity-feed name click both restore the right tab.
5. Close a running tab → confirm → child process gone; system-terminal launch option still works; app quit confirm works.

- [ ] **Step 3: Commit any fixes; report**

Report results against the five criteria, including anything not exercised (e.g. no SSH host available) — do not claim untested paths as verified.
