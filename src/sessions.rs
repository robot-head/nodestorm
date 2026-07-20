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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;

use crate::store::{ConnectionId, SessionState, Store, slugify};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Waiting {
        session: String,
        agent: Option<String>,
    },
    Receiving {
        session: String,
        agent: Option<String>,
    },
    Reconnecting {
        session: String,
        agent: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub id: ConnectionId,
    pub client_name: String,
    pub version: String,
    pub state: ConnectionState,
}

fn client_label(client_name: &str, version: &str) -> String {
    match version.trim() {
        "" => client_name.to_owned(),
        version => format!("{client_name} {version}"),
    }
}

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

struct RegistryEntry {
    info: ConnectionInfo,
    live: bool,
}

pub struct Sessions {
    inner: Mutex<Inner>,
    dir: PathBuf,
    /// Bumped whenever the session list or the active session changes; the
    /// UI re-subscribes its store bridge on it.
    generation: watch::Sender<u64>,
    connections: Mutex<BTreeMap<ConnectionId, RegistryEntry>>,
    connection_generation: watch::Sender<u64>,
    next_connection: AtomicU64,
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
        let (connection_generation, _) = watch::channel(0);
        let sessions = Arc::new(Self {
            inner: Mutex::new(Inner { map, active }),
            dir,
            generation,
            connections: Mutex::new(BTreeMap::new()),
            connection_generation,
            next_connection: AtomicU64::new(1),
            runtime: Mutex::new(None),
        });
        sessions.bind_connection_notifiers();
        Ok(sessions)
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
        let (connection_generation, _) = watch::channel(0);
        let sessions = Arc::new(Self {
            inner: Mutex::new(Inner {
                map,
                active: "default".into(),
            }),
            dir,
            generation,
            connections: Mutex::new(BTreeMap::new()),
            connection_generation,
            next_connection: AtomicU64::new(1),
            runtime: Mutex::new(None),
        });
        sessions.bind_connection_notifiers();
        sessions
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

    fn bump_connections(&self) {
        self.connection_generation.send_modify(|g| *g += 1);
    }

    fn bind_store_connection_notifier(&self, store: &Arc<Store>) {
        store.set_connection_notifier(self.connection_generation.clone());
    }

    fn bind_connection_notifiers(&self) {
        for entry in self.lock().map.values() {
            self.bind_store_connection_notifier(&entry.store);
        }
    }

    pub fn next_connection_id(&self) -> ConnectionId {
        ConnectionId(self.next_connection.fetch_add(1, Ordering::Relaxed))
    }

    pub fn connect_client(&self, id: ConnectionId, client_name: String, version: String) {
        let info = ConnectionInfo {
            id,
            client_name,
            version,
            state: ConnectionState::Connected,
        };
        let label = client_label(&info.client_name, &info.version);
        self.connections
            .lock()
            .expect("connections mutex poisoned")
            .insert(id, RegistryEntry { info, live: true });
        tracing::info!("{label} connected");
        self.bump_connections();
    }

    fn set_connection_state(&self, id: ConnectionId, state: ConnectionState) {
        let changed = self
            .connections
            .lock()
            .expect("connections mutex poisoned")
            .get_mut(&id)
            .is_some_and(|entry| {
                entry.info.state = state;
                true
            });
        if changed {
            self.bump_connections();
        }
    }

    pub fn set_connection_waiting(&self, id: ConnectionId, session: String, agent: Option<String>) {
        self.set_connection_state(id, ConnectionState::Waiting { session, agent });
    }

    pub fn set_connection_receiving(
        &self,
        id: ConnectionId,
        session: String,
        agent: Option<String>,
    ) {
        self.set_connection_state(id, ConnectionState::Receiving { session, agent });
    }

    pub fn set_connection_connected(&self, id: ConnectionId) {
        self.set_connection_state(id, ConnectionState::Connected);
    }

    pub fn disconnect_client(&self, id: ConnectionId) -> Option<ConnectionInfo> {
        let disconnected = self
            .connections
            .lock()
            .expect("connections mutex poisoned")
            .get_mut(&id)
            .and_then(|entry| {
                if !entry.live {
                    return None;
                }
                entry.live = false;
                Some(entry.info.clone())
            });
        if let Some(info) = &disconnected {
            let label = client_label(&info.client_name, &info.version);
            tracing::info!("{label} disconnected");
            self.bump_connections();
        }
        disconnected
    }

    pub fn connection(&self, id: ConnectionId) -> Option<ConnectionInfo> {
        self.connections
            .lock()
            .expect("connections mutex poisoned")
            .get(&id)
            .map(|entry| entry.info.clone())
    }

    pub fn connections(&self) -> Vec<ConnectionInfo> {
        let entries = self.connections.lock().expect("connections mutex poisoned");
        let mut result: Vec<_> = entries
            .values()
            .filter(|entry| entry.live)
            .map(|entry| entry.info.clone())
            .collect();
        let stores: Vec<_> = self
            .lock()
            .map
            .iter()
            .map(|(name, entry)| (name.clone(), entry.store.clone()))
            .collect();
        for (session, store) in stores {
            for target in store.reconnecting_targets() {
                if let Some(entry) = entries
                    .get(&target.connection_id)
                    .filter(|entry| !entry.live)
                {
                    let mut info = entry.info.clone();
                    info.state = ConnectionState::Reconnecting {
                        session: session.clone(),
                        agent: target.agent,
                    };
                    result.push(info);
                }
            }
        }
        result
    }

    pub fn subscribe_connections(&self) -> watch::Receiver<u64> {
        self.connection_generation.subscribe()
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
            self.bind_store_connection_notifier(&store);
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

    /// Rename a session: slugified and deduped like `create`. The backing
    /// file is renamed and `active` follows. The `Arc<Store>` is unchanged,
    /// so an in-flight `await_decisions` keeps working; only lookups by the
    /// old name start failing (with the usual available-sessions error).
    pub fn rename(&self, old: &str, new: &str) -> anyhow::Result<String> {
        let (old_slug, slug, old_path, new_path, reconnecting_projection_changed) = {
            let mut inner = self.lock();
            let old_slug = slugify(old);
            if !inner.map.contains_key(&old_slug) {
                anyhow::bail!("unknown session `{old}`");
            }
            let base = slugify(new);
            if base.is_empty() {
                anyhow::bail!("the new name is empty after slugifying");
            }
            let mut slug = base.clone();
            let mut n = 2;
            while slug != old_slug && inner.map.contains_key(&slug) {
                slug = format!("{base}-{n}");
                n += 1;
            }
            let mut entry = inner.map.remove(&old_slug).expect("checked above");
            let reconnecting_projection_changed =
                old_slug != slug && !entry.store.reconnecting_targets().is_empty();
            let old_path = entry.path.clone();
            let new_path = self.dir.join(format!("{slug}.json"));
            entry.path = new_path.clone();
            crate::persist::save(&old_path, &entry.store.snapshot_state())?;
            inner.map.insert(slug.clone(), entry);
            if inner.active == old_slug {
                inner.active = slug.clone();
            }
            (
                old_slug,
                slug,
                old_path,
                new_path,
                reconnecting_projection_changed,
            )
        };
        if old_path != new_path {
            std::fs::rename(&old_path, &new_path)?;
        }
        let live_projection_changed = if old_slug == slug {
            false
        } else {
            let mut entries = self.connections.lock().expect("connections mutex poisoned");
            let mut changed = false;
            for entry in entries.values_mut().filter(|entry| entry.live) {
                match &mut entry.info.state {
                    ConnectionState::Waiting { session, .. }
                    | ConnectionState::Receiving { session, .. }
                        if *session == old_slug =>
                    {
                        *session = slug.clone();
                        changed = true;
                    }
                    _ => {}
                }
            }
            changed
        };
        self.bump();
        if reconnecting_projection_changed || live_projection_changed {
            self.bump_connections();
        }
        Ok(slug)
    }

    /// Hard-delete a session: gone from the list AND from disk (archive is
    /// the reversible sibling). Same guardrails as archive: never the last
    /// session; deleting the active one switches to a survivor first.
    pub fn delete(&self, name: &str) -> anyhow::Result<()> {
        let entry = {
            let mut inner = self.lock();
            let slug = slugify(name);
            if !inner.map.contains_key(&slug) {
                anyhow::bail!("unknown session `{name}`");
            }
            if inner.map.len() == 1 {
                anyhow::bail!("cannot delete the last session");
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
            inner.map.remove(&slug).expect("checked above")
        };
        if entry.path.exists() {
            std::fs::remove_file(&entry.path)?;
        }
        let connection_projection_changed = !entry.store.reconnecting_targets().is_empty();
        self.bump();
        if connection_projection_changed {
            self.bump_connections();
        }
        Ok(())
    }

    /// Sorted stems of `archive/*.json`.
    pub fn list_archived(&self) -> Vec<String> {
        let archive_dir = self.dir.join("archive");
        let mut names: Vec<String> = std::fs::read_dir(&archive_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension().is_some_and(|x| x == "json") {
                    path.file_stem().and_then(|s| s.to_str()).map(str::to_owned)
                } else {
                    None
                }
            })
            .collect();
        names.sort();
        names
    }

    /// Bring an archived session back to the live list (name deduped
    /// against live sessions). Returns the canonical slug.
    pub fn unarchive(&self, name: &str) -> anyhow::Result<String> {
        let base = slugify(name);
        let archived = self.dir.join("archive").join(format!("{base}.json"));
        if !archived.exists() {
            anyhow::bail!("no archived session `{name}`");
        }
        let slug = {
            let inner = self.lock();
            let mut slug = base.clone();
            let mut n = 2;
            while inner.map.contains_key(&slug) {
                slug = format!("{base}-{n}");
                n += 1;
            }
            slug
        };
        let path = self.dir.join(format!("{slug}.json"));
        std::fs::rename(&archived, &path)?;
        let store = match crate::persist::load(&path) {
            Some(state) => Store::new(state),
            None => Store::new(SessionState::default()),
        };
        self.bind_store_connection_notifier(&store);
        {
            let mut inner = self.lock();
            inner.map.insert(
                slug.clone(),
                Entry {
                    store: store.clone(),
                    path: path.clone(),
                },
            );
        }
        if let Some(handle) = self.runtime.lock().expect("runtime lock").clone() {
            handle.spawn(crate::persist::autosave_task(store, path));
        }
        self.bump();
        Ok(slug)
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
        let connection_projection_changed = !entry.store.reconnecting_targets().is_empty();
        self.bump();
        if connection_projection_changed {
            self.bump_connections();
        }
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
    use crate::store::Awaiter;
    use std::path::PathBuf;
    use std::time::Duration;

    fn tmp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nodestorm-sess-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn client_label_omits_an_empty_version() {
        assert_eq!(
            client_label("claude-code", "2.1.215"),
            "claude-code 2.1.215"
        );
        assert_eq!(client_label("claude-code", ""), "claude-code");
        assert_eq!(client_label("claude-code", "   "), "claude-code");
    }

    #[test]
    fn disconnect_reports_only_the_first_live_transition() {
        let root = tmp_root("disconnect-transition");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        let id = sessions.next_connection_id();
        sessions.connect_client(id, "claude-code".into(), "2.1.215".into());

        assert_eq!(
            sessions.disconnect_client(id).map(|info| info.client_name),
            Some("claude-code".into())
        );
        assert!(sessions.disconnect_client(id).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_creates_default_when_empty() {
        let root = tmp_root("empty");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        assert2::assert!((sessions.active_name()) == ("default"));
        let list = sessions.list();
        assert2::assert!((list.len()) == (1));
        assert2::assert!(list[0].active);
        assert2::assert!((list[0].name) == ("default"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_migrates_legacy_session_json() {
        let root = tmp_root("migrate");
        let store = crate::store::Store::with_doc(demo_doc());
        crate::persist::save(&root.join("session.json"), &store.snapshot_state()).unwrap();
        std::fs::create_dir(root.join("sessions")).unwrap();
        std::fs::write(root.join("sessions").join("README.txt"), "not a session").unwrap();

        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        assert2::assert!((sessions.active_name()) == ("default"));
        let doc = sessions.active_store().snapshot_doc();
        assert2::assert!(
            doc.node(&NodeId::from("sync-engine")).is_some(),
            "legacy content migrated into default"
        );
        assert2::assert!(
            !root.join("session.json").exists(),
            "legacy file moved, not copied"
        );
        assert2::assert!(root.join("sessions").join("default.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_pins_explicit_file() {
        let root = tmp_root("pin");
        let pinned = root.join("nodestorm-verify-session-4799.json");
        let sessions = Sessions::open(root.join("sessions"), Some(pinned.clone())).unwrap();
        assert2::assert!((sessions.active_name()) == ("nodestorm-verify-session-4799"));
        assert2::assert!((sessions.active_path()) == (pinned));
        // Saving lands in the pinned file, not the sessions dir.
        sessions.save_all();
        assert2::assert!(pinned.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn create_slugs_and_dedups() {
        let root = tmp_root("create");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        assert2::assert!((sessions.create("My Thing").unwrap()) == ("my-thing"));
        assert2::assert!((sessions.create("My Thing").unwrap()) == ("my-thing-2"));
        assert2::assert!((sessions.create("My Thing").unwrap()) == ("my-thing-3"));
        assert2::assert!(root.join("sessions").join("my-thing.json").exists());
        assert2::assert!((sessions.list().len()) == (4));
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
        assert2::assert!((sessions.active_name()) == ("alpha"));
        assert2::assert!(
            *generation.borrow_and_update() > before,
            "generation bumped"
        );
        assert2::assert!(sessions.switch("ghost").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_none_is_active_and_unknown_lists_available() {
        let root = tmp_root("resolve");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        let active = sessions.resolve(None).unwrap();
        assert2::assert!(Arc::ptr_eq(&active, &sessions.active_store()));
        // Lookups slugify, so display names find their slug.
        assert2::assert!(sessions.resolve(Some("Alpha")).is_ok());
        let err = sessions.resolve(Some("ghost")).unwrap_err();
        assert2::assert!(err.contains("unknown session"), "{err}");
        assert2::assert!(err.contains("available"), "{err}");
        assert2::assert!(err.contains("alpha"), "{err}");
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
        assert2::assert!((sessions.active_name()) == ("default"));
        assert2::assert!(
            root.join("sessions")
                .join("archive")
                .join("alpha.json")
                .exists(),
            "archived file moved into archive/"
        );
        assert2::assert!(sessions.get("alpha").is_none());

        // The last remaining session cannot be archived.
        assert2::assert!(sessions.archive("default").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_rekeys_file_and_active() {
        let root = tmp_root("rename");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        sessions.switch("alpha").unwrap();
        let store_before = sessions.get("alpha").unwrap();
        let mut generation = sessions.subscribe_generation();
        let before = *generation.borrow_and_update();

        let slug = sessions.rename("alpha", "Big Plan").unwrap();
        assert2::assert!((slug) == ("big-plan"));
        assert2::assert!((sessions.active_name()) == ("big-plan"), "active follows");
        assert2::assert!(sessions.get("alpha").is_none());
        let store_after = sessions.get("big-plan").unwrap();
        assert2::assert!(
            Arc::ptr_eq(&store_before, &store_after),
            "same store, new name — in-flight awaits keep working"
        );
        assert2::assert!(root.join("sessions").join("big-plan.json").exists());
        assert2::assert!(!root.join("sessions").join("alpha.json").exists());
        assert2::assert!(*generation.borrow_and_update() > before);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_dedups_against_existing() {
        let root = tmp_root("rename-dedup");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        sessions.create("alpha").unwrap();
        sessions.create("beta").unwrap();
        let slug = sessions.rename("beta", "alpha").unwrap();
        assert2::assert!((slug) == ("alpha-3"), "collides with both live alpha names");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_updates_matching_live_connection_rows_once() {
        let root = tmp_root("rename-live-connections");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        sessions.create("beta").unwrap();

        let waiting = sessions.next_connection_id();
        sessions.connect_client(waiting, "claude-code".into(), "1".into());
        sessions.set_connection_waiting(waiting, "alpha".into(), Some("one".into()));
        let receiving = sessions.next_connection_id();
        sessions.connect_client(receiving, "claude-code".into(), "1".into());
        sessions.set_connection_receiving(receiving, "alpha".into(), Some("two".into()));
        let other = sessions.next_connection_id();
        sessions.connect_client(other, "claude-code".into(), "1".into());
        sessions.set_connection_waiting(other, "beta".into(), Some("three".into()));
        let connected = sessions.next_connection_id();
        sessions.connect_client(connected, "claude-code".into(), "1".into());

        let mut changes = sessions.subscribe_connections();
        let before = *changes.borrow_and_update();
        assert2::assert!((sessions.rename("alpha", "Big Plan").unwrap()) == ("big-plan"));

        assert2::assert!(changes.has_changed().unwrap());
        assert2::assert!((*changes.borrow_and_update()) == (before + 1));
        assert2::assert!(matches!(
            sessions.connection(waiting).unwrap().state,
            ConnectionState::Waiting { session, agent }
                if session == "big-plan" && agent.as_deref() == Some("one")
        ));
        assert2::assert!(matches!(
            sessions.connection(receiving).unwrap().state,
            ConnectionState::Receiving { session, agent }
                if session == "big-plan" && agent.as_deref() == Some("two")
        ));
        assert2::assert!(matches!(
            sessions.connection(other).unwrap().state,
            ConnectionState::Waiting { session, agent }
                if session == "beta" && agent.as_deref() == Some("three")
        ));
        assert2::assert!(
            (sessions.connection(connected).unwrap().state) == (ConnectionState::Connected)
        );

        let before_noop = *changes.borrow_and_update();
        assert2::assert!((sessions.rename("big-plan", "Big Plan").unwrap()) == ("big-plan"));
        assert2::assert!(!changes.has_changed().unwrap());
        assert2::assert!((*changes.borrow_and_update()) == (before_noop));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_removes_file_and_never_last() {
        let root = tmp_root("delete");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        sessions.switch("alpha").unwrap();

        sessions.delete("alpha").unwrap();
        assert2::assert!(
            (sessions.active_name()) == ("default"),
            "active switched first"
        );
        assert2::assert!(sessions.get("alpha").is_none());
        assert2::assert!(
            !root.join("sessions").join("alpha.json").exists(),
            "file deleted, not archived"
        );
        assert2::assert!(
            !root
                .join("sessions")
                .join("archive")
                .join("alpha.json")
                .exists()
        );
        assert2::assert!(sessions.delete("default").is_err(), "never the last");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unarchive_round_trip() {
        let root = tmp_root("unarchive");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("alpha").unwrap();
        sessions
            .get("alpha")
            .unwrap()
            .apply_propose(demo_doc())
            .unwrap();
        sessions.archive("alpha").unwrap();
        assert2::assert!((sessions.list_archived()) == (vec!["alpha".to_owned()]));

        let slug = sessions.unarchive("alpha").unwrap();
        assert2::assert!((slug) == ("alpha"));
        assert2::assert!(sessions.list_archived().is_empty());
        let doc = sessions.get("alpha").unwrap().snapshot_doc();
        assert2::assert!(
            doc.node(&NodeId::from("sync-engine")).is_some(),
            "content survived the round trip"
        );

        // Unarchiving over a live name collision dedups.
        sessions.archive("alpha").unwrap();
        sessions.create("alpha").unwrap();
        sessions.create("alpha").unwrap();
        let slug = sessions.unarchive("alpha").unwrap();
        assert2::assert!((slug) == ("alpha-3"));
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
        assert2::assert!(
            info == SessionInfo {
                name: "demo".into(),
                active: false,
                node_count: 11,
                open_choices: 2,
                agent_waiting: false,
                undelivered: 0,
            }
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn connection_registry_reports_metadata_and_notifies_independently() {
        let sessions =
            Sessions::single(Store::new(SessionState::default()), tmp_root("connections"));
        let mut changes = sessions.subscribe_connections();
        let before = *changes.borrow_and_update();
        let id = sessions.next_connection_id();

        sessions.connect_client(id, "claude-code".into(), "1.2.3".into());
        sessions.set_connection_waiting(id, "default".into(), Some("alpha".into()));

        assert2::assert!(*changes.borrow_and_update() > before);
        assert2::assert!(
            (sessions.connections())
                == (vec![ConnectionInfo {
                    id,
                    client_name: "claude-code".into(),
                    version: "1.2.3".into(),
                    state: ConnectionState::Waiting {
                        session: "default".into(),
                        agent: Some("alpha".into()),
                    },
                }])
        );

        sessions.set_connection_connected(id);
        assert2::assert!((sessions.connection(id).unwrap().state) == (ConnectionState::Connected));
    }

    #[test]
    fn disconnect_removes_live_connection_without_bumping_session_generation() {
        let sessions =
            Sessions::single(Store::new(SessionState::default()), tmp_root("disconnect"));
        let id = sessions.next_connection_id();
        let mut generation = sessions.subscribe_generation();
        let before = *generation.borrow_and_update();
        sessions.connect_client(id, "claude-code".into(), "1".into());

        sessions.disconnect_client(id);

        assert2::assert!(sessions.connections().is_empty());
        assert2::assert!((*generation.borrow_and_update()) == (before));
    }

    #[tokio::test]
    async fn orphaned_receipt_keeps_a_reconnecting_connection_row() {
        let store = Store::new(SessionState::default());
        let sessions = Sessions::single(store.clone(), tmp_root("reconnecting-row"));
        let id = sessions.next_connection_id();
        sessions.connect_client(id, "claude-code".into(), "1".into());
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(
                        Duration::from_secs(30),
                        Awaiter {
                            connection_id: id,
                            client_label: "claude-code 1".into(),
                            agent: Some("alpha".into()),
                        },
                    )
                    .await
            }
        });
        for _ in 0..50 {
            if store.snapshot_meta().waiting_agents == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        store.request_flush(None).unwrap();
        waiting.abort();
        let _ = waiting.await;
        sessions.disconnect_client(id);

        assert2::assert!(matches!(
            sessions.connections()[0].state,
            ConnectionState::Reconnecting { ref session, ref agent }
                if session == "default" && agent.as_deref() == Some("alpha")
        ));
    }

    #[tokio::test]
    async fn reconnecting_projection_notifies_on_orphan_and_rebind() {
        let root = tmp_root("reconnecting-events");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("plan").unwrap();
        let store = sessions.get("plan").unwrap();
        let first_id = sessions.next_connection_id();
        sessions.connect_client(first_id, "claude-code".into(), "1".into());
        let mut changes = sessions.subscribe_connections();
        changes.borrow_and_update();

        let first = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(
                        Duration::from_secs(30),
                        Awaiter {
                            connection_id: first_id,
                            client_label: "claude-code 1".into(),
                            agent: Some("alpha".into()),
                        },
                    )
                    .await
            }
        });
        let mut revisions = store.subscribe();
        while store.snapshot_meta().waiting_agents != 1 {
            revisions.changed().await.unwrap();
        }
        store.request_flush(None).unwrap();
        sessions.disconnect_client(first_id);
        changes.borrow_and_update();
        assert2::assert!(sessions.connections().is_empty());
        let before_orphan = *changes.borrow_and_update();

        first.abort();
        let _ = first.await;
        tokio::time::timeout(Duration::from_secs(1), changes.changed())
            .await
            .expect("orphan creation publishes a connection change")
            .unwrap();
        assert2::assert!((*changes.borrow_and_update()) == (before_orphan + 1));
        assert2::assert!(matches!(
            sessions.connections().as_slice(),
            [ConnectionInfo {
                id,
                state: ConnectionState::Reconnecting { session, agent },
                ..
            }] if *id == first_id
                && session == "plan"
                && agent.as_deref() == Some("alpha")
        ));

        let second_id = sessions.next_connection_id();
        sessions.connect_client(second_id, "claude-code".into(), "2".into());
        changes.borrow_and_update();
        let recovered = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(
                        Duration::from_secs(30),
                        Awaiter {
                            connection_id: second_id,
                            client_label: "claude-code 2".into(),
                            agent: Some("alpha".into()),
                        },
                    )
                    .await
            }
        });

        tokio::time::timeout(Duration::from_secs(1), changes.changed())
            .await
            .expect("reconnect rebind publishes a connection change")
            .unwrap();
        assert2::assert!(
            (sessions.connections())
                == (vec![ConnectionInfo {
                    id: second_id,
                    client_name: "claude-code".into(),
                    version: "2".into(),
                    state: ConnectionState::Connected,
                }])
        );
        assert2::assert!(matches!(
            recovered.await.unwrap().unwrap(),
            crate::store::FlushOutcome::Delivered(_)
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn unarchived_store_notifies_when_a_receipt_is_orphaned() {
        let root = tmp_root("unarchived-reconnecting-events");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("plan").unwrap();
        sessions.archive("plan").unwrap();
        sessions.unarchive("plan").unwrap();
        let store = sessions.get("plan").unwrap();
        let id = sessions.next_connection_id();
        sessions.connect_client(id, "claude-code".into(), "1".into());
        let mut changes = sessions.subscribe_connections();
        changes.borrow_and_update();

        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(
                        Duration::from_secs(30),
                        Awaiter {
                            connection_id: id,
                            client_label: "claude-code 1".into(),
                            agent: Some("alpha".into()),
                        },
                    )
                    .await
            }
        });
        let mut revisions = store.subscribe();
        while store.snapshot_meta().waiting_agents != 1 {
            revisions.changed().await.unwrap();
        }
        store.request_flush(None).unwrap();
        sessions.disconnect_client(id);
        changes.borrow_and_update();
        waiting.abort();
        let _ = waiting.await;

        tokio::time::timeout(Duration::from_secs(1), changes.changed())
            .await
            .expect("an unarchived store publishes orphan projection changes")
            .unwrap();
        assert2::assert!(matches!(
            sessions.connections().as_slice(),
            [ConnectionInfo {
                state: ConnectionState::Reconnecting { session, .. },
                ..
            }] if session == "plan"
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    async fn orphan_named_receipt(sessions: &Arc<Sessions>, session: &str) {
        let store = sessions.get(session).unwrap();
        let id = sessions.next_connection_id();
        sessions.connect_client(id, "claude-code".into(), "1".into());
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(
                        Duration::from_secs(30),
                        Awaiter {
                            connection_id: id,
                            client_label: "claude-code 1".into(),
                            agent: Some("alpha".into()),
                        },
                    )
                    .await
            }
        });
        let mut revisions = store.subscribe();
        while store.snapshot_meta().waiting_agents != 1 {
            revisions.changed().await.unwrap();
        }
        store.request_flush(None).unwrap();
        sessions.disconnect_client(id);
        waiting.abort();
        let _ = waiting.await;
        assert2::assert!(matches!(
            sessions.connections().as_slice(),
            [ConnectionInfo {
                state: ConnectionState::Reconnecting {
                    session: projected,
                    ..
                },
                ..
            }] if projected == session
        ));
    }

    #[tokio::test]
    async fn reconnecting_projection_notifies_on_rename_archive_and_not_unarchive() {
        let root = tmp_root("reconnecting-session-lifecycle");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("plan").unwrap();
        orphan_named_receipt(&sessions, "plan").await;
        let mut changes = sessions.subscribe_connections();
        changes.borrow_and_update();

        assert2::assert!((sessions.rename("plan", "roadmap").unwrap()) == ("roadmap"));
        tokio::time::timeout(Duration::from_secs(1), changes.changed())
            .await
            .expect("rename publishes reconnect projection change")
            .unwrap();
        assert2::assert!(matches!(
            sessions.connections().as_slice(),
            [ConnectionInfo {
                state: ConnectionState::Reconnecting { session, .. },
                ..
            }] if session == "roadmap"
        ));

        changes.borrow_and_update();
        assert2::assert!((sessions.rename("roadmap", "roadmap").unwrap()) == ("roadmap"));
        assert2::assert!(!changes.has_changed().unwrap(), "no-op rename stays quiet");

        sessions.archive("roadmap").unwrap();
        tokio::time::timeout(Duration::from_secs(1), changes.changed())
            .await
            .expect("archive publishes reconnect projection change")
            .unwrap();
        assert2::assert!(sessions.connections().is_empty());

        changes.borrow_and_update();
        sessions.unarchive("roadmap").unwrap();
        assert2::assert!(
            !changes.has_changed().unwrap(),
            "unarchive has no transient reconnect projection to publish"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn reconnecting_projection_notifies_when_session_is_deleted() {
        let root = tmp_root("reconnecting-session-delete");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.create("plan").unwrap();
        orphan_named_receipt(&sessions, "plan").await;
        let mut changes = sessions.subscribe_connections();
        changes.borrow_and_update();

        sessions.delete("plan").unwrap();

        tokio::time::timeout(Duration::from_secs(1), changes.changed())
            .await
            .expect("delete publishes reconnect projection change")
            .unwrap();
        assert2::assert!(sessions.connections().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[should_panic(expected = "store already belongs to a session manager")]
    fn store_rejects_a_second_connection_notifier_owner() {
        let store = Store::new(SessionState::default());
        let _first = Sessions::single(store.clone(), tmp_root("store-owner-1"));
        let _second = Sessions::single(store, tmp_root("store-owner-2"));
    }

    #[tokio::test(start_paused = true)]
    async fn list_reports_waiting_and_only_undelivered_events() {
        let root = tmp_root("list-live");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        let store = sessions.active_store();
        store
            .add_user_node("first".into(), crate::model::NodeKind::Component, None)
            .unwrap();
        let first_store = store.clone();
        let first_waiter = tokio::spawn(async move {
            first_store
                .await_flush(
                    std::time::Duration::from_secs(60),
                    Awaiter {
                        connection_id: crate::store::ConnectionId(901),
                        client_label: "first test client".into(),
                        agent: None,
                    },
                )
                .await
        });
        while store.snapshot_meta().waiting_agents != 1 {
            tokio::task::yield_now().await;
        }
        store.request_flush(None).unwrap();
        assert2::assert!(matches!(
            first_waiter.await.unwrap().unwrap(),
            crate::store::FlushOutcome::Delivered(_),
        ));
        store
            .add_user_node("second".into(), crate::model::NodeKind::Component, None)
            .unwrap();
        let wait_store = store.clone();
        let waiter = tokio::spawn(async move {
            wait_store
                .await_flush(
                    std::time::Duration::from_secs(60),
                    Awaiter {
                        connection_id: crate::store::ConnectionId(902),
                        client_label: "second test client".into(),
                        agent: None,
                    },
                )
                .await
        });
        while store.snapshot_meta().waiting_agents != 1 {
            tokio::task::yield_now().await;
        }

        let info = sessions.list().remove(0);
        assert2::assert!(info.agent_waiting);
        assert2::assert!((info.undelivered) == (1));
        waiter.abort();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn spawn_autosaves_remembers_the_runtime() {
        let root = tmp_root("autosave-runtime");
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        sessions.spawn_autosaves(&tokio::runtime::Handle::current());
        assert2::assert!(sessions.runtime.lock().unwrap().is_some());
        let _ = std::fs::remove_dir_all(root);
    }
}
