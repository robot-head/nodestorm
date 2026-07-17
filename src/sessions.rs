//! Concurrent named sessions: one [`Store`] per session, exactly one
//! *active* session for the UI, files at `<dir>/<name>.json`.
//!
//! The store itself is untouched — every session keeps its own delivery
//! machinery, so an agent can block on `await_decisions` for session A
//! while the user works in session B. Only the user switches the active
//! session; agents address sessions by name over MCP (names are slugified
//! on every lookup, so "My Thing" and `my-thing` are the same session).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;

use crate::store::{SessionState, Store, slugify};

/// One row of [`Sessions::list`] — what the switcher and `list_sessions`
/// tool show per session.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SessionInfo {
    pub name: String,
    pub active: bool,
    pub node_count: usize,
    pub open_choices: usize,
    pub agent_waiting: bool,
    pub undelivered: usize,
}

struct Entry {
    store: Arc<Store>,
    path: PathBuf,
}

struct Inner {
    map: BTreeMap<String, Entry>,
    active: String,
}

pub struct Sessions {
    inner: Mutex<Inner>,
    dir: PathBuf,
    /// Bumped whenever the session list or the active session changes; the
    /// UI re-subscribes its store bridge on it.
    generation: watch::Sender<u64>,
    /// Set by [`Sessions::spawn_autosaves`]; later `create()` calls use it
    /// to give new sessions their own autosave task.
    runtime: Mutex<Option<tokio::runtime::Handle>>,
}

impl Sessions {
    /// Load every session in `dir` (creating it, migrating a legacy sibling
    /// `session.json` into `default.json`, and guaranteeing at least one
    /// session). `pinned` loads/creates that exact file as a session named
    /// after its stem and makes it active (the `--session` contract).
    pub fn open(dir: PathBuf, pinned: Option<PathBuf>) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&dir)?;

        // Legacy migration: a v0.3-era single session.json next to the dir.
        let has_sessions = std::fs::read_dir(&dir)?
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "json"));
        if !has_sessions && let Some(parent) = dir.parent() {
            let legacy = parent.join("session.json");
            if legacy.exists() {
                std::fs::rename(&legacy, dir.join("default.json"))?;
                tracing::info!("migrated legacy session.json to sessions/default.json");
            }
        }

        let mut map = BTreeMap::new();
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "json")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                let name = slugify(stem);
                let store = match crate::persist::load(&path) {
                    Some(state) => Store::new(state),
                    None => Store::new(SessionState::default()),
                };
                map.insert(name, Entry { store, path });
            }
        }

        let mut active = if map.contains_key("default") {
            "default".to_owned()
        } else {
            map.keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "default".into())
        };
        if map.is_empty() {
            map.insert(
                "default".to_owned(),
                Entry {
                    store: Store::new(SessionState::default()),
                    path: dir.join("default.json"),
                },
            );
        }

        if let Some(path) = pinned {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(slugify)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "pinned".into());
            let store = match crate::persist::load(&path) {
                Some(state) => Store::new(state),
                None => Store::new(SessionState::default()),
            };
            map.insert(name.clone(), Entry { store, path });
            active = name;
        }

        let (generation, _) = watch::channel(0);
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner { map, active }),
            dir,
            generation,
            runtime: Mutex::new(None),
        }))
    }

    /// Convenience for `main`: directory and pin from the CLI.
    pub fn open_from_cli(cli: &crate::cli::Cli) -> anyhow::Result<Arc<Self>> {
        Sessions::open(cli.sessions_dir()?, cli.session.clone())
    }

    /// A manager wrapping one pre-built store as `default` — the test and
    /// embedding harness path (no files are touched until save/create).
    pub fn single(store: Arc<Store>, dir: PathBuf) -> Arc<Self> {
        let mut map = BTreeMap::new();
        map.insert(
            "default".to_owned(),
            Entry {
                store,
                path: dir.join("default.json"),
            },
        );
        let (generation, _) = watch::channel(0);
        Arc::new(Self {
            inner: Mutex::new(Inner {
                map,
                active: "default".into(),
            }),
            dir,
            generation,
            runtime: Mutex::new(None),
        })
    }

    /// The active session's backing file (export paths derive from it).
    pub fn active_path(&self) -> PathBuf {
        let inner = self.lock();
        inner.map[&inner.active].path.clone()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("sessions mutex poisoned")
    }

    fn bump(&self) {
        self.generation.send_modify(|g| *g += 1);
    }

    /// Create an empty session; the returned name is the slug (deduped with
    /// `-2`, `-3`… on collision). Saves the file immediately.
    pub fn create(&self, name: &str) -> anyhow::Result<String> {
        let (slug, store, path) = {
            let mut inner = self.lock();
            let base = slugify(name);
            let mut slug = base.clone();
            let mut n = 2;
            while inner.map.contains_key(&slug) {
                slug = format!("{base}-{n}");
                n += 1;
            }
            let store = Store::new(SessionState::default());
            let path = self.dir.join(format!("{slug}.json"));
            inner.map.insert(
                slug.clone(),
                Entry {
                    store: store.clone(),
                    path: path.clone(),
                },
            );
            (slug, store, path)
        };
        crate::persist::save(&path, &store.snapshot_state())?;
        if let Some(handle) = self.runtime.lock().expect("runtime lock").clone() {
            handle.spawn(crate::persist::autosave_task(store, path));
        }
        self.bump();
        Ok(slug)
    }

    /// `None` → the active session; `Some(name)` → that session (slugified
    /// lookup), or an error naming the available sessions.
    pub fn resolve(&self, name: Option<&str>) -> Result<Arc<Store>, String> {
        self.resolve_named(name).map(|(_, store)| store)
    }

    /// Like [`Sessions::resolve`], but also returns the canonical slug of
    /// the session that was resolved (for tool results).
    pub fn resolve_named(&self, name: Option<&str>) -> Result<(String, Arc<Store>), String> {
        let inner = self.lock();
        match name {
            None => Ok((inner.active.clone(), inner.map[&inner.active].store.clone())),
            Some(n) => {
                let slug = slugify(n);
                inner
                    .map
                    .get(&slug)
                    .map(|e| (slug, e.store.clone()))
                    .ok_or_else(|| {
                        let available = inner.map.keys().cloned().collect::<Vec<_>>().join(", ");
                        format!("unknown session `{n}`; available: {available}")
                    })
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<Store>> {
        self.lock().map.get(&slugify(name)).map(|e| e.store.clone())
    }

    /// The propose_graph auto-create path: existing session or a fresh one
    /// under that name. Returns the canonical slug.
    pub fn get_or_create(&self, name: &str) -> anyhow::Result<(String, Arc<Store>)> {
        let slug = slugify(name);
        if let Some(store) = self.get(&slug) {
            return Ok((slug, store));
        }
        let created = self.create(&slug)?;
        let store = self.get(&created).expect("just created");
        Ok((created, store))
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let inner = self.lock();
        inner
            .map
            .iter()
            .map(|(name, entry)| {
                entry.store.read(|s| SessionInfo {
                    name: name.clone(),
                    active: *name == inner.active,
                    node_count: s.doc.nodes.len(),
                    open_choices: s.doc.open_choice_count(),
                    agent_waiting: s.waiting_agents > 0,
                    undelivered: s.decision_log.len() - s.delivery_cursor,
                })
            })
            .collect()
    }

    pub fn active_name(&self) -> String {
        self.lock().active.clone()
    }

    pub fn active_store(&self) -> Arc<Store> {
        let inner = self.lock();
        inner.map[&inner.active].store.clone()
    }

    /// User-only: make another session the one the window shows.
    pub fn switch(&self, name: &str) -> anyhow::Result<()> {
        {
            let mut inner = self.lock();
            let slug = slugify(name);
            if !inner.map.contains_key(&slug) {
                anyhow::bail!("unknown session `{name}`");
            }
            inner.active = slug;
        }
        self.bump();
        Ok(())
    }

    /// Save, then move the session's file into `<dir>/archive/` and drop it
    /// from the live list. The last session can't be archived; archiving
    /// the active one switches to a survivor first.
    pub fn archive(&self, name: &str) -> anyhow::Result<()> {
        let entry = {
            let mut inner = self.lock();
            let slug = slugify(name);
            if !inner.map.contains_key(&slug) {
                anyhow::bail!("unknown session `{name}`");
            }
            if inner.map.len() == 1 {
                anyhow::bail!("cannot archive the last session");
            }
            if inner.active == slug {
                let survivor = inner
                    .map
                    .keys()
                    .find(|k| **k != slug)
                    .cloned()
                    .expect("len > 1");
                inner.active = survivor;
            }
            let entry = inner.map.remove(&slug).expect("checked above");
            (slug, entry)
        };
        let (slug, entry) = entry;
        crate::persist::save(&entry.path, &entry.store.snapshot_state())?;
        let archive_dir = self.dir.join("archive");
        std::fs::create_dir_all(&archive_dir)?;
        std::fs::rename(&entry.path, archive_dir.join(format!("{slug}.json")))?;
        self.bump();
        Ok(())
    }

    /// Final saves for every live session (shutdown path).
    pub fn save_all(&self) {
        let entries: Vec<(Arc<Store>, PathBuf)> = self
            .lock()
            .map
            .values()
            .map(|e| (e.store.clone(), e.path.clone()))
            .collect();
        for (store, path) in entries {
            if let Err(err) = crate::persist::save(&path, &store.snapshot_state()) {
                tracing::warn!(%err, path = %path.display(), "session save failed");
            }
        }
    }

    /// Give every current session an autosave task and remember the handle
    /// so later `create()` calls spawn one too.
    pub fn spawn_autosaves(&self, handle: &tokio::runtime::Handle) {
        *self.runtime.lock().expect("runtime lock") = Some(handle.clone());
        let entries: Vec<(Arc<Store>, PathBuf)> = self
            .lock()
            .map
            .values()
            .map(|e| (e.store.clone(), e.path.clone()))
            .collect();
        for (store, path) in entries {
            handle.spawn(crate::persist::autosave_task(store, path));
        }
    }

    pub fn subscribe_generation(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_doc;
    use crate::model::NodeId;
    use std::path::PathBuf;

    fn tmp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nodestorm-sess-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn open_creates_default_when_empty() {
        let root = tmp_root("empty");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        assert_eq!(sessions.active_name(), "default");
        let list = sessions.list();
        assert_eq!(list.len(), 1);
        assert!(list[0].active);
        assert_eq!(list[0].name, "default");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_migrates_legacy_session_json() {
        let root = tmp_root("migrate");
        let store = crate::store::Store::with_doc(demo_doc());
        crate::persist::save(&root.join("session.json"), &store.snapshot_state()).unwrap();

        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        assert_eq!(sessions.active_name(), "default");
        let doc = sessions.active_store().snapshot_doc();
        assert!(
            doc.node(&NodeId::from("sync-engine")).is_some(),
            "legacy content migrated into default"
        );
        assert!(
            !root.join("session.json").exists(),
            "legacy file moved, not copied"
        );
        assert!(root.join("sessions").join("default.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_pins_explicit_file() {
        let root = tmp_root("pin");
        let pinned = root.join("nodestorm-verify-session-4799.json");
        let sessions = Sessions::open(root.join("sessions"), Some(pinned.clone())).unwrap();
        assert_eq!(sessions.active_name(), "nodestorm-verify-session-4799");
        // Saving lands in the pinned file, not the sessions dir.
        sessions.save_all();
        assert!(pinned.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn create_slugs_and_dedups() {
        let root = tmp_root("create");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        assert_eq!(sessions.create("My Thing").unwrap(), "my-thing");
        assert_eq!(sessions.create("My Thing").unwrap(), "my-thing-2");
        assert!(root.join("sessions").join("my-thing.json").exists());
        assert_eq!(sessions.list().len(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn switch_changes_active_and_bumps_generation() {
        let root = tmp_root("switch");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        let mut generation = sessions.subscribe_generation();
        let before = *generation.borrow_and_update();
        sessions.switch("alpha").unwrap();
        assert_eq!(sessions.active_name(), "alpha");
        assert!(
            *generation.borrow_and_update() > before,
            "generation bumped"
        );
        assert!(sessions.switch("ghost").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_none_is_active_and_unknown_lists_available() {
        let root = tmp_root("resolve");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        let active = sessions.resolve(None).unwrap();
        assert!(Arc::ptr_eq(&active, &sessions.active_store()));
        // Lookups slugify, so display names find their slug.
        assert!(sessions.resolve(Some("Alpha")).is_ok());
        let err = sessions.resolve(Some("ghost")).unwrap_err();
        assert!(err.contains("unknown session"), "{err}");
        assert!(err.contains("available"), "{err}");
        assert!(err.contains("alpha"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_moves_file_and_never_last() {
        let root = tmp_root("archive");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        sessions.switch("alpha").unwrap();

        // Archiving the ACTIVE session switches to a survivor first.
        sessions.archive("alpha").unwrap();
        assert_eq!(sessions.active_name(), "default");
        assert!(
            root.join("sessions")
                .join("archive")
                .join("alpha.json")
                .exists(),
            "archived file moved into archive/"
        );
        assert!(sessions.get("alpha").is_none());

        // The last remaining session cannot be archived.
        assert!(sessions.archive("default").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_reports_summaries() {
        let root = tmp_root("list");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("demo").unwrap();
        sessions
            .get("demo")
            .unwrap()
            .apply_propose(demo_doc())
            .unwrap();
        let info = sessions
            .list()
            .into_iter()
            .find(|i| i.name == "demo")
            .unwrap();
        assert_eq!(info.node_count, 11);
        assert_eq!(info.open_choices, 2);
        assert!(!info.active);
        assert!(!info.agent_waiting);
        let _ = std::fs::remove_dir_all(&root);
    }
}
