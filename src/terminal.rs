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

/// ConPTY's win32-input-mode startup handshake: a Device Status Report query
/// asking the terminal for the cursor position. The child stays blocked in
/// startup until something answers it, so we do — see the reader thread in
/// `spawn`. Windows-only: ConPTY is the only platform that blocks on this at
/// startup.
#[cfg(windows)]
const CPR_QUERY: &[u8] = b"\x1b[6n";
#[cfg(windows)]
const CPR_REPLY: &[u8] = b"\x1b[1;1R";

/// Cap enforcement: keep the newest bytes, drop the oldest.
fn push_ring(ring: &mut Vec<u8>, chunk: &[u8], cap: usize) {
    ring.extend_from_slice(chunk);
    if ring.len() > cap {
        ring.drain(..ring.len() - cap);
    }
}

/// A subscriber's view: everything so far plus the live stream. Snapshot and
/// subscription happen under one lock so no byte is missed or duplicated.
pub struct Attached {
    pub replay: Vec<u8>,
    pub output: broadcast::Receiver<Vec<u8>>,
}

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

struct Entry {
    // `None` once the child has exited: on Windows, closing the pseudoconsole
    // (dropping this) is what unblocks the reader thread's pending read, so
    // it can't wait around for `close()`.
    master: Option<Box<dyn MasterPty + Send>>,
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
                master: Some(pair.master),
                writer,
                killer,
                ring: Vec::new(),
                tx: broadcast::channel(1024).0,
                status: TerminalStatus::Running,
            },
        );

        // Reader thread: pumps PTY output into the ring/broadcast until EOF.
        let manager = self.clone();
        let reader_id = id.to_owned();
        std::thread::Builder::new()
            .name(format!("pty-read-{reader_id}"))
            .spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut map = manager.lock();
                    if let Some(entry) = map.get_mut(&reader_id) {
                        push_ring(&mut entry.ring, &buf[..n], SCROLLBACK_LIMIT);
                        // Receivers may lag or be absent; both are fine.
                        let _ = entry.tx.send(buf[..n].to_vec());
                        // ConPTY's win32-input-mode handshake asks the
                        // terminal for its cursor position right after spawn
                        // and blocks the child until it gets an answer, so
                        // we answer on its behalf — but only while no client
                        // is attached yet. Once Ferroterm attaches it
                        // answers CPR itself; replying here too would race
                        // it and send a spurious extra `ESC[1;1R` into a
                        // TUI's mid-session cursor probe.
                        // ponytail: scans only within one read() chunk; a
                        // query split across two reads would be missed.
                        // Upgrade to a cross-chunk scan if that's observed.
                        #[cfg(windows)]
                        if entry.tx.receiver_count() == 0
                            && buf[..n].windows(CPR_QUERY.len()).any(|w| w == CPR_QUERY)
                        {
                            let _ = entry.writer.write_all(CPR_REPLY);
                            let _ = entry.writer.flush();
                        }
                    } else {
                        break;
                    }
                }
            })
            .expect("spawning the pty reader thread");

        // Waiter thread: blocks for the exit code, then drops the master.
        // ConPTY does not EOF the output pipe just because the child process
        // exited; closing the pseudoconsole is what unblocks the reader
        // thread above. Unix already EOFs on exit, so this is a no-op there.
        let manager = self.clone();
        let waiter_id = id.to_owned();
        std::thread::Builder::new()
            .name(format!("pty-wait-{waiter_id}"))
            .spawn(move || {
                let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
                let closed_master = {
                    let mut map = manager.lock();
                    map.get_mut(&waiter_id).and_then(|entry| {
                        entry.status = TerminalStatus::Exited(code);
                        entry.master.take()
                    })
                };
                drop(closed_master); // outside the lock: may block briefly on Windows
                manager.bump();
            })
            .expect("spawning the pty waiter thread");
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
        let master = entry
            .master
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("terminal `{id}` has exited"))?;
        master.resize(PtySize {
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
        let generation = manager.subscribe();
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
        manager.spawn("t-shell", &interactive_shell_spec()).unwrap();
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
        manager.spawn("t-kill", &interactive_shell_spec()).unwrap();
        assert2::assert!((manager.status("t-kill")) == (Some(TerminalStatus::Running)));
        manager.kill("t-kill");
        wait_for(|| exited(&manager, "t-kill"));
        // Entry survives a kill: scrollback stays readable until close().
        assert2::assert!(manager.attach("t-kill").is_some());
    }

    #[test]
    fn duplicate_ids_and_unknown_ids_error() {
        let manager = TerminalManager::new();
        manager.spawn("t-dup", &interactive_shell_spec()).unwrap();
        assert2::assert!(manager.spawn("t-dup", &interactive_shell_spec()).is_err());
        assert2::assert!(manager.write("missing", b"x").is_err());
        assert2::assert!(manager.resize("missing", 80, 24).is_err());
        manager.close("t-dup");
    }
}
