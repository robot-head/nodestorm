//! Session persistence: XDG default path, atomic saves, debounced autosave.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use crate::store::{SessionState, Store};

const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(750);

/// `~/.local/share/nodestorm/session.json` (per-platform equivalent).
pub fn default_session_path() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "nodestorm", "nodestorm")
        .context("cannot determine a data directory for this platform")?;
    Ok(dirs.data_dir().join("session.json"))
}

/// Load a saved session. Missing file → `None`. A corrupt file is moved
/// aside (never silently clobbered) and also yields `None`.
pub fn load(path: &Path) -> Option<SessionState> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "cannot read session file");
            return None;
        }
    };
    match serde_json::from_slice::<SessionState>(&bytes) {
        Ok(state) => {
            tracing::info!(path = %path.display(), nodes = state.doc.nodes.len(), "session restored");
            Some(state)
        }
        Err(err) => {
            let backup = path.with_extension("json.corrupt");
            tracing::warn!(%err, backup = %backup.display(), "session file unreadable — moving aside");
            let _ = std::fs::rename(path, &backup);
            None
        }
    }
}

/// Atomic write primitive shared by [`save`], [`save_export`], and the
/// preferences file: temp file (`<name>.tmp`) in the same directory, then
/// rename over the target.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = {
        let mut name = path.as_os_str().to_owned();
        name.push(".tmp");
        PathBuf::from(name)
    };
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// Atomic session save.
pub fn save(path: &Path, state: &SessionState) -> anyhow::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(state)?)
}

/// Where a UI-triggered export lands: the session path with its extension
/// swapped, e.g. `session.json` → `session.export.md`.
pub fn export_path(session_path: &Path) -> PathBuf {
    session_path.with_extension("export.md")
}

/// Where a mermaid-only export lands: `session.json` → `session.mermaid.md`.
pub fn mermaid_export_path(session_path: &Path) -> PathBuf {
    session_path.with_extension("mermaid.md")
}

/// Atomically write an exported Markdown record; overwriting an earlier
/// export is intentional and idempotent.
pub fn save_export(path: &Path, markdown: &str) -> anyhow::Result<()> {
    write_atomic(path, markdown.as_bytes())
}

/// Autosave loop: waits for a revision change, debounces, saves. Runs on the
/// server runtime until the store's watch channel closes.
pub async fn autosave_task(store: Arc<Store>, path: PathBuf) {
    let mut rev = store.subscribe();
    loop {
        if rev.changed().await.is_err() {
            break;
        }
        // Debounce: absorb the burst, then save once.
        loop {
            tokio::select! {
                changed = rev.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                () = tokio::time::sleep(AUTOSAVE_DEBOUNCE) => break,
            }
        }
        let state = store.snapshot_state();
        if let Err(err) = save(&path, &state) {
            tracing::warn!(%err, "autosave failed");
        } else {
            tracing::debug!(path = %path.display(), revision = state.doc.revision, "autosaved");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_doc;
    use yare::parameterized;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nodestorm-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn default_path_ends_in_session_json() {
        assert2::assert!(
            (default_session_path().unwrap().file_name().unwrap()) == ("session.json")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn autosave_task_writes_after_the_debounce() {
        let path = tmp_path("autosave.json");
        let store = Store::with_doc(demo_doc());
        let task = tokio::spawn(autosave_task(store.clone(), path.clone()));
        tokio::task::yield_now().await;

        store.announce("save me".into());
        tokio::task::yield_now().await;
        tokio::time::advance(AUTOSAVE_DEBOUNCE + Duration::from_millis(1)).await;
        tokio::task::yield_now().await;

        assert2::assert!(path.exists());
        task.abort();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn save_load_round_trip() {
        let path = tmp_path("roundtrip.json");
        let mut state = SessionState::default();
        state.doc = demo_doc();
        state.flush_seq = 1;
        let store = Store::new(state);
        let state = store.snapshot_state();
        save(&path, &state).unwrap();
        let loaded = load(&path).expect("loads");
        assert2::assert!((loaded.doc) == (state.doc));
        assert2::assert!((loaded.decision_log.len()) == (state.decision_log.len()));
        assert2::assert!((loaded.flush_seq) == (state.flush_seq));
        assert2::assert!(
            (loaded.waiting_agents) == (0),
            "transient field not persisted"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_file_is_none() {
        assert2::assert!(load(Path::new("/definitely/not/here.json")).is_none());
    }

    #[test]
    fn mermaid_export_path_maps() {
        assert2::assert!(
            (mermaid_export_path(Path::new("/data/session.json")))
                == (PathBuf::from("/data/session.mermaid.md"))
        );
    }

    #[parameterized(
        session_json = { "/data/session.json", "/data/session.export.md" },
        extensionless_override = { "custom-session", "custom-session.export.md" },
    )]
    fn export_path_maps_session_json(input: &str, expected: &str) {
        assert2::assert!(export_path(Path::new(input)) == PathBuf::from(expected));
    }

    #[test]
    fn save_export_round_trip() {
        let path = tmp_path("record.export.md");
        save_export(&path, "# hello\n").unwrap();
        assert2::assert!((std::fs::read_to_string(&path).unwrap()) == ("# hello\n"));
        // Re-export overwrites in place.
        save_export(&path, "# again\n").unwrap();
        assert2::assert!((std::fs::read_to_string(&path).unwrap()) == ("# again\n"));
        // No temp residue next to the record.
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        assert2::assert!(!PathBuf::from(tmp).exists());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let path = tmp_path("corrupt.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert2::assert!(load(&path).is_none());
        assert2::assert!(!path.exists(), "corrupt file moved away");
        let backup = path.with_extension("json.corrupt");
        assert2::assert!(backup.exists());
        std::fs::remove_file(backup).ok();
    }
}
