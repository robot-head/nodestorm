//! Shared session state between the UI (main thread / Dioxus runtime) and the
//! MCP server (dedicated tokio runtime thread).
//!
//! Concurrency model: one `std::sync::Mutex` around all state (critical
//! sections are tiny and never held across `.await`) plus a `tokio::sync::watch`
//! channel carrying the latest revision for change notification. Decisions are
//! delivered to the agent **exactly once** via an append-only log and a
//! delivery cursor advanced under the mutex; see [`Store::await_flush`].

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::watch;

use crate::model::{
    ActivityEntry, ActivityOrigin, ChoiceId, ChoiceStatus, DecisionEvent, DecisionKind, GraphOp,
    Node, NodeId, Note, NoteId, OptionId, Point, SessionDoc,
};

const ACTIVITY_CAP: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unknown node `{0}`")]
    UnknownNode(NodeId),
    #[error("unknown choice `{choice}` on node `{node}`")]
    UnknownChoice { node: NodeId, choice: ChoiceId },
    #[error("unknown option `{option}` in choice `{choice}` on node `{node}`")]
    UnknownOption {
        node: NodeId,
        choice: ChoiceId,
        option: OptionId,
    },
    #[error("document rejected: {}", errors.join("; "))]
    Invalid { errors: Vec<String> },
}

/// Full session state guarded by the store mutex.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionState {
    pub doc: SessionDoc,
    /// Append-only within a session; `seq` is the 1-based index.
    pub decision_log: Vec<DecisionEvent>,
    /// `decision_log[..delivery_cursor]` has been handed to the agent.
    pub delivery_cursor: usize,
    /// Bumped by "Send to agent" (or autoflush when the last choice closes).
    pub flush_seq: u64,
    /// The last `flush_seq` actually delivered to an `await_decisions` call.
    pub delivered_flush_seq: u64,
    pub activity: Vec<ActivityEntry>,
    /// Live `await_decisions` calls (transient; not persisted).
    #[serde(skip)]
    pub waiting_agents: usize,
}

/// Lightweight snapshot for the top bar / panels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UiMeta {
    pub waiting_agents: usize,
    pub undelivered: usize,
    pub open_choices: usize,
    pub activity: Vec<ActivityEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlushOutcome {
    /// The batch of decision events since the previous delivery.
    Delivered(Vec<DecisionEvent>),
    /// No flush within the timeout. `preview` is non-authoritative: the same
    /// events will be re-sent in the eventual `Delivered` response.
    TimedOut { preview: Vec<DecisionEvent> },
    /// The store is shutting down.
    Shutdown,
}

/// Result of an agent graph mutation.
#[derive(Debug, Clone, Default)]
pub struct UpdateSummary {
    pub revision: u64,
    pub node_count: usize,
    pub open_choice_count: usize,
    pub warnings: Vec<String>,
}

pub struct Store {
    state: Mutex<SessionState>,
    revision_tx: watch::Sender<u64>,
}

impl Store {
    pub fn new(state: SessionState) -> Arc<Self> {
        let (revision_tx, _) = watch::channel(state.doc.revision);
        Arc::new(Self {
            state: Mutex::new(state),
            revision_tx,
        })
    }

    pub fn with_doc(doc: SessionDoc) -> Arc<Self> {
        Self::new(SessionState {
            doc,
            ..SessionState::default()
        })
    }

    fn lock(&self) -> MutexGuard<'_, SessionState> {
        self.state.lock().expect("store mutex poisoned")
    }

    pub fn read<R>(&self, f: impl FnOnce(&SessionState) -> R) -> R {
        f(&self.lock())
    }

    /// Run a mutation, bump the doc revision, and notify subscribers after
    /// the lock is released.
    pub fn mutate<R>(&self, f: impl FnOnce(&mut SessionState) -> R) -> R {
        let (result, revision) = {
            let mut s = self.lock();
            let result = f(&mut s);
            s.doc.revision += 1;
            (result, s.doc.revision)
        };
        self.revision_tx.send_replace(revision);
        result
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision_tx.subscribe()
    }

    pub fn snapshot_doc(&self) -> SessionDoc {
        self.read(|s| s.doc.clone())
    }

    pub fn snapshot_state(&self) -> SessionState {
        self.read(Clone::clone)
    }

    pub fn snapshot_meta(&self) -> UiMeta {
        self.read(|s| UiMeta {
            waiting_agents: s.waiting_agents,
            undelivered: s.decision_log.len() - s.delivery_cursor,
            open_choices: s.doc.open_choice_count(),
            activity: s.activity.clone(),
        })
    }

    // ---------- user-facing mutations (called from the UI) ----------

    pub fn set_position(&self, node: &NodeId, position: Point) {
        self.mutate(|s| {
            if let Some(n) = s.doc.node_mut(node) {
                n.position = Some(position);
            }
        });
    }

    /// Record the user picking an option. Re-picking is allowed; the event log
    /// preserves the history and the doc reflects the latest selection.
    pub fn select_option(
        &self,
        node: &NodeId,
        choice: &ChoiceId,
        option: &OptionId,
        considered: Vec<OptionId>,
    ) -> Result<(), StoreError> {
        self.mutate(|s| {
            let n = s
                .doc
                .node_mut(node)
                .ok_or_else(|| StoreError::UnknownNode(node.clone()))?;
            let label = n.label.clone();
            let c = n
                .choices
                .iter_mut()
                .find(|c| &c.id == choice)
                .ok_or_else(|| StoreError::UnknownChoice {
                    node: node.clone(),
                    choice: choice.clone(),
                })?;
            let opt_label = c
                .options
                .iter()
                .find(|o| &o.id == option)
                .map(|o| o.label.clone())
                .ok_or_else(|| StoreError::UnknownOption {
                    node: node.clone(),
                    choice: choice.clone(),
                    option: option.clone(),
                })?;
            c.selected = Some(option.clone());
            c.status = ChoiceStatus::Decided;
            push_event(
                s,
                DecisionKind::OptionSelected {
                    node_id: node.clone(),
                    choice_id: choice.clone(),
                    option_id: option.clone(),
                    considered,
                },
            );
            push_activity(
                s,
                ActivityOrigin::User,
                format!("picked \u{201c}{opt_label}\u{201d} for {label}"),
            );
            autoflush(s);
            Ok(())
        })
    }

    pub fn dismiss_choice(
        &self,
        node: &NodeId,
        choice: &ChoiceId,
        reason: Option<String>,
    ) -> Result<(), StoreError> {
        self.mutate(|s| {
            let n = s
                .doc
                .node_mut(node)
                .ok_or_else(|| StoreError::UnknownNode(node.clone()))?;
            let label = n.label.clone();
            let c = n
                .choices
                .iter_mut()
                .find(|c| &c.id == choice)
                .ok_or_else(|| StoreError::UnknownChoice {
                    node: node.clone(),
                    choice: choice.clone(),
                })?;
            c.status = ChoiceStatus::Dismissed;
            push_event(
                s,
                DecisionKind::ChoiceDismissed {
                    node_id: node.clone(),
                    choice_id: choice.clone(),
                    reason,
                },
            );
            push_activity(
                s,
                ActivityOrigin::User,
                format!("dismissed a choice on {label}"),
            );
            autoflush(s);
            Ok(())
        })
    }

    pub fn add_note(&self, node: &NodeId, text: String) -> Result<(), StoreError> {
        self.mutate(|s| {
            let n = s
                .doc
                .node_mut(node)
                .ok_or_else(|| StoreError::UnknownNode(node.clone()))?;
            let label = n.label.clone();
            n.notes.push(Note {
                id: NoteId::new(uuid::Uuid::new_v4().to_string()),
                text: text.clone(),
                created_at: Utc::now(),
            });
            push_event(
                s,
                DecisionKind::NoteAdded {
                    node_id: node.clone(),
                    text,
                },
            );
            push_activity(s, ActivityOrigin::User, format!("added a note to {label}"));
            Ok(())
        })
    }

    /// Post an agent-authored message to the activity feed.
    pub fn announce(&self, message: String) {
        self.mutate(|s| push_activity(s, ActivityOrigin::Agent, message));
    }

    /// Record a completed UI export in the activity feed — the feed entry is
    /// the user's receipt for where the record landed.
    pub fn record_export(&self, path: &std::path::Path) {
        self.mutate(|s| {
            push_activity(
                s,
                ActivityOrigin::User,
                format!("exported decision record to {}", path.display()),
            );
        });
    }

    /// Surface a failed UI export in the activity feed.
    pub fn record_export_failed(&self, err: &str) {
        self.mutate(|s| push_activity(s, ActivityOrigin::User, format!("export failed: {err}")));
    }

    /// "Send to agent": flush everything undelivered (an empty flush is a
    /// valid "reviewed, proceed" signal).
    pub fn request_flush(&self, comment: Option<String>) {
        self.mutate(|s| {
            if comment.as_deref().is_some_and(|c| !c.trim().is_empty()) {
                push_event(
                    s,
                    DecisionKind::FlushRequested {
                        comment: comment.map(|c| c.trim().to_owned()),
                    },
                );
            }
            s.flush_seq += 1;
            push_activity(
                s,
                ActivityOrigin::User,
                "sent decisions to the agent".into(),
            );
        });
    }

    // ---------- agent-facing mutations (called from MCP tools) ----------

    /// Replace the document, preserving user-owned state (positions, notes,
    /// decided choices) for nodes whose ids survive.
    pub fn apply_propose(&self, incoming: SessionDoc) -> Result<UpdateSummary, StoreError> {
        let validation = incoming.validate();
        if !validation.is_ok() {
            return Err(StoreError::Invalid {
                errors: validation.errors,
            });
        }
        Ok(self.mutate(|s| {
            let old_revision = s.doc.revision;
            let previous = std::mem::take(&mut s.doc);
            s.doc = incoming;
            s.doc.version = SessionDoc::VERSION;
            s.doc.revision = old_revision;
            for node in &mut s.doc.nodes {
                if let Some(prev) = previous.node(&node.id) {
                    let mut merged = prev.clone();
                    merged.merge_from_agent(std::mem::replace(node, placeholder_node()));
                    *node = merged;
                }
            }
            let title = if s.doc.title.is_empty() {
                "a graph".to_owned()
            } else {
                format!("\u{201c}{}\u{201d}", s.doc.title)
            };
            push_activity(s, ActivityOrigin::Agent, format!("proposed {title}"));
            summary(s, validation.warnings.clone())
        }))
    }

    /// Apply a batch of ops atomically: all-or-nothing against validation.
    pub fn apply_update(&self, ops: Vec<GraphOp>) -> Result<UpdateSummary, StoreError> {
        // Stage on a clone so a failed op or failed validation commits nothing.
        let staged = self.read(|s| s.doc.clone());
        let mut doc = staged;
        let mut announces: Vec<String> = Vec::new();
        for op in ops {
            apply_op(&mut doc, op, &mut announces)?;
        }
        let validation = doc.validate();
        if !validation.is_ok() {
            return Err(StoreError::Invalid {
                errors: validation.errors,
            });
        }
        Ok(self.mutate(|s| {
            doc.revision = s.doc.revision;
            s.doc = doc;
            if announces.is_empty() {
                push_activity(s, ActivityOrigin::Agent, "updated the graph".into());
            }
            for msg in announces {
                push_activity(s, ActivityOrigin::Agent, msg);
            }
            summary(s, validation.warnings.clone())
        }))
    }

    pub fn clear_session(&self) -> UpdateSummary {
        self.mutate(|s| {
            let revision = s.doc.revision;
            *s = SessionState::default();
            s.doc.revision = revision;
            push_activity(s, ActivityOrigin::System, "session cleared".into());
            summary(s, vec![])
        })
    }

    // ---------- delivery ----------

    pub fn peek_undelivered(&self) -> Vec<DecisionEvent> {
        self.read(|s| s.decision_log[s.delivery_cursor..].to_vec())
    }

    /// Atomically take the undelivered batch if a flush is pending.
    fn try_deliver(&self) -> Option<Vec<DecisionEvent>> {
        let taken = {
            let mut s = self.lock();
            try_deliver_locked(&mut s)
        };
        if taken.is_some() {
            // Repaint (undelivered count, pill) — bump revision via mutate.
            self.mutate(|_| {});
        }
        taken
    }

    /// Block until the user flushes decisions, the timeout elapses, or the
    /// store shuts down. Must run on a tokio runtime (uses `tokio::time`).
    ///
    /// Exactly-once: when several calls race one flush, one gets
    /// `Delivered` and the rest run to their own deadlines.
    pub async fn await_flush(self: &Arc<Self>, timeout: Duration) -> FlushOutcome {
        let mut rev = self.subscribe();
        let _guard = WaitGuard::enter(self.clone());
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            if let Some(batch) = self.try_deliver() {
                return FlushOutcome::Delivered(batch);
            }
            tokio::select! {
                changed = rev.changed() => {
                    if changed.is_err() {
                        return FlushOutcome::Shutdown;
                    }
                }
                () = &mut deadline => {
                    // Final re-check closes the click-vs-timeout race: a flush
                    // that landed before this point is delivered, not dropped.
                    if let Some(batch) = self.try_deliver() {
                        return FlushOutcome::Delivered(batch);
                    }
                    return FlushOutcome::TimedOut {
                        preview: self.peek_undelivered(),
                    };
                }
            }
        }
    }
}

/// RAII guard for the "agent is waiting" indicator; drop-safe against client
/// aborts because the future dropping runs `Drop`.
struct WaitGuard {
    store: Arc<Store>,
}

impl WaitGuard {
    fn enter(store: Arc<Store>) -> Self {
        store.mutate(|s| s.waiting_agents += 1);
        Self { store }
    }
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        self.store
            .mutate(|s| s.waiting_agents = s.waiting_agents.saturating_sub(1));
    }
}

fn try_deliver_locked(s: &mut SessionState) -> Option<Vec<DecisionEvent>> {
    if s.flush_seq > s.delivered_flush_seq {
        let batch = s.decision_log[s.delivery_cursor..].to_vec();
        s.delivery_cursor = s.decision_log.len();
        s.delivered_flush_seq = s.flush_seq;
        Some(batch)
    } else {
        None
    }
}

fn push_event(s: &mut SessionState, kind: DecisionKind) {
    let seq = s.decision_log.len() as u64 + 1;
    s.decision_log.push(DecisionEvent {
        seq,
        at: Utc::now(),
        kind,
    });
}

fn push_activity(s: &mut SessionState, origin: ActivityOrigin, text: String) {
    s.activity.push(ActivityEntry {
        at: Utc::now(),
        origin,
        text,
    });
    if s.activity.len() > ACTIVITY_CAP {
        let excess = s.activity.len() - ACTIVITY_CAP;
        s.activity.drain(..excess);
    }
}

/// Autoflush: deciding the last open choice sends pending events without an
/// explicit "Send to agent" click.
fn autoflush(s: &mut SessionState) {
    if s.doc.open_choice_count() == 0 && s.delivery_cursor < s.decision_log.len() {
        s.flush_seq += 1;
        push_activity(
            s,
            ActivityOrigin::System,
            "all choices decided — sending to the agent".into(),
        );
    }
}

fn summary(s: &SessionState, warnings: Vec<String>) -> UpdateSummary {
    UpdateSummary {
        revision: s.doc.revision,
        node_count: s.doc.nodes.len(),
        open_choice_count: s.doc.open_choice_count(),
        warnings,
    }
}

fn placeholder_node() -> Node {
    Node {
        id: NodeId::new(""),
        label: String::new(),
        kind: Default::default(),
        description: String::new(),
        status: Default::default(),
        group: None,
        choices: vec![],
        notes: vec![],
        position: None,
    }
}

fn apply_op(
    doc: &mut SessionDoc,
    op: GraphOp,
    announces: &mut Vec<String>,
) -> Result<(), StoreError> {
    match op {
        GraphOp::UpsertNode { mut node } => {
            if let Some(existing) = doc.node_mut(&node.id) {
                existing.merge_from_agent(node);
            } else {
                for c in &mut node.choices {
                    c.reopen = false;
                }
                doc.nodes.push(node);
            }
        }
        GraphOp::RemoveNode { id } => {
            let before = doc.nodes.len();
            doc.nodes.retain(|n| n.id != id);
            if doc.nodes.len() == before {
                return Err(StoreError::UnknownNode(id));
            }
            doc.edges.retain(|e| e.from != id && e.to != id);
            if doc.focus.as_ref() == Some(&id) {
                doc.focus = None;
            }
        }
        GraphOp::UpsertEdge { edge } => {
            if let Some(existing) = doc.edges.iter_mut().find(|e| e.key() == edge.key()) {
                *existing = edge;
            } else {
                doc.edges.push(edge);
            }
        }
        GraphOp::RemoveEdge { from, to, kind } => {
            doc.edges
                .retain(|e| !(e.from == from && e.to == to && kind.is_none_or(|k| e.kind == k)));
        }
        GraphOp::AddChoice { node_id, choice } => {
            let node = doc
                .node_mut(&node_id)
                .ok_or(StoreError::UnknownNode(node_id))?;
            if let Some(existing) = node.choices.iter_mut().find(|c| c.id == choice.id) {
                let is_decided = existing.status == ChoiceStatus::Decided;
                if !is_decided || choice.reopen {
                    *existing = consume_reopen(choice);
                }
            } else {
                node.choices.push(consume_reopen(choice));
            }
        }
        GraphOp::ResolveChoice {
            node_id,
            choice_id,
            selected,
            dismiss,
        } => {
            let node = doc
                .node_mut(&node_id)
                .ok_or_else(|| StoreError::UnknownNode(node_id.clone()))?;
            let c = node.choices.iter_mut().find(|c| c.id == choice_id).ok_or(
                StoreError::UnknownChoice {
                    node: node_id,
                    choice: choice_id,
                },
            )?;
            if dismiss {
                c.status = ChoiceStatus::Dismissed;
            } else {
                if selected.is_some() {
                    c.selected = selected;
                }
                c.status = ChoiceStatus::Decided;
            }
        }
        GraphOp::SetStatus { id, status } => {
            let node = doc.node_mut(&id).ok_or(StoreError::UnknownNode(id))?;
            node.status = status;
        }
        GraphOp::SetFocus { id } => {
            doc.focus = id;
        }
        GraphOp::SetTitle { title } => {
            doc.title = title;
        }
        GraphOp::Announce { message } => {
            announces.push(message);
        }
    }
    Ok(())
}

/// A reopening upsert resets the choice to `Open`; the flag never persists.
fn consume_reopen(mut c: crate::model::Choice) -> crate::model::Choice {
    if c.reopen {
        c.reopen = false;
        c.status = ChoiceStatus::Open;
        c.selected = None;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_doc;

    fn demo_store() -> Arc<Store> {
        Store::with_doc(demo_doc())
    }

    #[test]
    fn record_export_lands_in_activity() {
        let store = demo_store();
        store.record_export(std::path::Path::new("some/dir/session.export.md"));
        let meta = store.snapshot_meta();
        let entry = meta.activity.last().expect("an activity entry");
        assert_eq!(entry.origin, ActivityOrigin::User);
        assert!(
            entry.text.contains("session.export.md"),
            "text: {}",
            entry.text
        );

        store.record_export_failed("disk full");
        let meta = store.snapshot_meta();
        let entry = meta.activity.last().unwrap();
        assert!(
            entry.text.contains("export failed: disk full"),
            "text: {}",
            entry.text
        );
    }

    fn pick_first_choice(store: &Arc<Store>) {
        store
            .select_option(
                &NodeId::from("sync-engine"),
                &ChoiceId::from("conflict-resolution"),
                &OptionId::from("crdt"),
                vec![OptionId::from("ot"), OptionId::from("crdt")],
            )
            .unwrap();
    }

    #[test]
    fn select_option_updates_doc_and_log() {
        let store = demo_store();
        pick_first_choice(&store);
        let doc = store.snapshot_doc();
        let c = doc
            .node(&NodeId::from("sync-engine"))
            .unwrap()
            .choice(&ChoiceId::from("conflict-resolution"))
            .unwrap();
        assert_eq!(c.status, ChoiceStatus::Decided);
        assert_eq!(c.selected, Some(OptionId::from("crdt")));
        assert_eq!(store.peek_undelivered().len(), 1);
        assert_eq!(store.snapshot_meta().open_choices, 1);
    }

    #[test]
    fn flush_delivers_exactly_once() {
        let store = demo_store();
        pick_first_choice(&store);
        store.request_flush(None);
        let first = store.try_deliver().expect("pending flush");
        assert_eq!(first.len(), 1);
        assert!(store.try_deliver().is_none(), "second take gets nothing");
    }

    #[test]
    fn autoflush_fires_when_last_choice_closes() {
        let store = demo_store();
        pick_first_choice(&store);
        assert!(store.try_deliver().is_none(), "one choice still open");
        store
            .dismiss_choice(
                &NodeId::from("ws-gateway"),
                &ChoiceId::from("ws-deployment"),
                None,
            )
            .unwrap();
        let batch = store.try_deliver().expect("autoflush fired");
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn empty_flush_still_delivers() {
        let store = demo_store();
        store.request_flush(Some("looks good, proceed".into()));
        let batch = store.try_deliver().expect("flush pending");
        assert_eq!(batch.len(), 1, "comment rides as an event");
        assert!(matches!(batch[0].kind, DecisionKind::FlushRequested { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn racing_awaits_deliver_to_exactly_one() {
        let store = demo_store();
        pick_first_choice(&store);

        let a = tokio::spawn({
            let store = store.clone();
            async move { store.await_flush(Duration::from_secs(5)).await }
        });
        let b = tokio::spawn({
            let store = store.clone();
            async move { store.await_flush(Duration::from_secs(5)).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(store.snapshot_meta().waiting_agents, 2);
        store.request_flush(None);

        let (ra, rb) = tokio::join!(a, b);
        let outcomes = [ra.unwrap(), rb.unwrap()];
        let delivered = outcomes
            .iter()
            .filter(|o| matches!(o, FlushOutcome::Delivered(_)))
            .count();
        let timed_out = outcomes
            .iter()
            .filter(|o| matches!(o, FlushOutcome::TimedOut { .. }))
            .count();
        assert_eq!((delivered, timed_out), (1, 1));
        assert_eq!(store.snapshot_meta().waiting_agents, 0, "guards released");
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_preview_does_not_consume() {
        let store = demo_store();
        pick_first_choice(&store);
        let outcome = store.await_flush(Duration::from_secs(1)).await;
        match outcome {
            FlushOutcome::TimedOut { preview } => assert_eq!(preview.len(), 1),
            other => panic!("expected timeout, got {other:?}"),
        }
        // The decision was NOT consumed: a later flush delivers it.
        store.request_flush(None);
        let outcome = store.await_flush(Duration::from_secs(1)).await;
        match outcome {
            FlushOutcome::Delivered(batch) => assert_eq!(batch.len(), 1),
            other => panic!("expected delivery, got {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn flush_after_timeout_is_returned_by_next_call_instantly() {
        let store = demo_store();
        pick_first_choice(&store);
        let FlushOutcome::TimedOut { .. } = store.await_flush(Duration::from_millis(10)).await
        else {
            panic!("expected timeout");
        };
        store.request_flush(None);
        // Next call must return without waiting: deliver on entry.
        let outcome = tokio::time::timeout(
            Duration::from_millis(1),
            store.await_flush(Duration::from_secs(600)),
        )
        .await
        .expect("await_flush should return immediately");
        assert!(matches!(outcome, FlushOutcome::Delivered(b) if b.len() == 1));
    }

    #[tokio::test(start_paused = true)]
    async fn wait_guard_releases_on_future_drop() {
        let store = demo_store();
        let fut = {
            let store = store.clone();
            async move { store.await_flush(Duration::from_secs(60)).await }
        };
        let handle = tokio::spawn(fut);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(store.snapshot_meta().waiting_agents, 1);
        handle.abort(); // simulate the MCP client vanishing mid-await
        let _ = handle.await;
        assert_eq!(store.snapshot_meta().waiting_agents, 0);
    }

    #[test]
    fn apply_propose_preserves_user_state() {
        let store = demo_store();
        store.set_position(&NodeId::from("redis"), Point { x: 9.0, y: 9.0 });
        store
            .add_note(&NodeId::from("redis"), "keep redis small".into())
            .unwrap();
        pick_first_choice(&store);

        // Agent re-proposes the same doc (fresh choices, no positions).
        store.apply_propose(demo_doc()).unwrap();
        let doc = store.snapshot_doc();
        let redis = doc.node(&NodeId::from("redis")).unwrap();
        assert_eq!(redis.position, Some(Point { x: 9.0, y: 9.0 }));
        assert_eq!(redis.notes.len(), 1);
        let sync = doc.node(&NodeId::from("sync-engine")).unwrap();
        let c = sync.choice(&ChoiceId::from("conflict-resolution")).unwrap();
        assert_eq!(c.status, ChoiceStatus::Decided, "decision survives propose");
        assert_eq!(c.selected, Some(OptionId::from("crdt")));
    }

    #[test]
    fn apply_propose_rejects_invalid_doc() {
        let store = demo_store();
        let mut bad = demo_doc();
        bad.nodes.push(bad.nodes[0].clone()); // duplicate id
        let err = store.apply_propose(bad).unwrap_err();
        assert!(matches!(err, StoreError::Invalid { .. }));
        // Store unchanged.
        assert_eq!(store.snapshot_doc().nodes.len(), demo_doc().nodes.len());
    }

    #[test]
    fn apply_update_is_atomic() {
        let store = demo_store();
        let before = store.snapshot_doc();
        let ops = vec![
            GraphOp::SetTitle {
                title: "changed".into(),
            },
            GraphOp::RemoveNode {
                id: NodeId::from("nope-not-here"),
            },
        ];
        assert!(store.apply_update(ops).is_err());
        let after = store.snapshot_doc();
        assert_eq!(before.title, after.title, "first op rolled back");
    }

    #[test]
    fn apply_update_add_choice_respects_decided() {
        let store = demo_store();
        pick_first_choice(&store);
        // Agent re-adds the same choice without reopen: decision stands.
        let fresh = demo_doc()
            .node(&NodeId::from("sync-engine"))
            .unwrap()
            .choice(&ChoiceId::from("conflict-resolution"))
            .unwrap()
            .clone();
        store
            .apply_update(vec![GraphOp::AddChoice {
                node_id: NodeId::from("sync-engine"),
                choice: fresh.clone(),
            }])
            .unwrap();
        let doc = store.snapshot_doc();
        let c = doc
            .node(&NodeId::from("sync-engine"))
            .unwrap()
            .choice(&ChoiceId::from("conflict-resolution"))
            .unwrap();
        assert_eq!(c.status, ChoiceStatus::Decided);

        // With reopen: the choice opens again.
        let mut reopened = fresh;
        reopened.reopen = true;
        store
            .apply_update(vec![GraphOp::AddChoice {
                node_id: NodeId::from("sync-engine"),
                choice: reopened,
            }])
            .unwrap();
        let doc = store.snapshot_doc();
        let c = doc
            .node(&NodeId::from("sync-engine"))
            .unwrap()
            .choice(&ChoiceId::from("conflict-resolution"))
            .unwrap();
        assert_eq!(c.status, ChoiceStatus::Open);
        assert_eq!(c.selected, None);
    }

    #[test]
    fn clear_session_resets_but_keeps_revision_monotonic() {
        let store = demo_store();
        pick_first_choice(&store);
        let rev_before = store.snapshot_doc().revision;
        store.clear_session();
        let s = store.snapshot_state();
        assert!(s.doc.nodes.is_empty());
        assert!(s.decision_log.is_empty());
        assert!(s.doc.revision > rev_before);
    }
}
