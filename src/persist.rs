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

/// Atomic write: temp file in the same directory, then rename.
pub fn save(path: &Path, state: &SessionState) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(state)?;
    std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
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

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nodestorm-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn save_load_round_trip() {
        let path = tmp_path("roundtrip.json");
        let store = Store::with_doc(demo_doc());
        store.request_flush(Some("hello".into()));
        let state = store.snapshot_state();
        save(&path, &state).unwrap();
        let loaded = load(&path).expect("loads");
        assert_eq!(loaded.doc, state.doc);
        assert_eq!(loaded.decision_log.len(), state.decision_log.len());
        assert_eq!(loaded.flush_seq, state.flush_seq);
        assert_eq!(loaded.waiting_agents, 0, "transient field not persisted");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_file_is_none() {
        assert!(load(Path::new("/definitely/not/here.json")).is_none());
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let path = tmp_path("corrupt.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(load(&path).is_none());
        assert!(!path.exists(), "corrupt file moved away");
        let backup = path.with_extension("json.corrupt");
        assert!(backup.exists());
        std::fs::remove_file(backup).ok();
    }
}
