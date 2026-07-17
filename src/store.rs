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
    ActivityEntry, ActivityOrigin, ChoiceId, ChoiceStatus, DecisionEvent, DecisionKind, Edge,
    EdgeKind, ElementStatus, GraphOp, Node, NodeId, NodeKind, Note, NoteId, OptionId, Origin,
    Point, SessionDoc,
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
    #[error("edge {0} -> {1} of that kind already exists")]
    DuplicateEdge(NodeId, NodeId),
    #[error("an edge cannot connect a node to itself")]
    SelfLoop,
    #[error("no edge {0} -> {1} of that kind")]
    UnknownEdge(NodeId, NodeId),
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

    // ---------- user graph editing (called from the UI) ----------
    //
    // Each mutation records a decision event that rides along with the next
    // Send/flush — editing never autoflushes (unlike deciding the last open
    // choice), so the user batches edits with their decisions.

    /// Create a user-authored node. The id is a slug of the label, suffixed
    /// `-2`, `-3`… on collision. `position: None` lets auto-layout place it.
    pub fn add_user_node(
        &self,
        label: String,
        kind: NodeKind,
        position: Option<Point>,
    ) -> Result<NodeId, StoreError> {
        self.mutate(|s| {
            let base = slugify(&label);
            let mut candidate = base.clone();
            let mut n = 2;
            while s.doc.node(&NodeId::new(candidate.clone())).is_some() {
                candidate = format!("{base}-{n}");
                n += 1;
            }
            let id = NodeId::new(candidate);
            s.doc.nodes.push(Node {
                id: id.clone(),
                label: label.clone(),
                kind,
                description: String::new(),
                status: ElementStatus::Proposed,
                group: None,
                choices: vec![],
                notes: vec![],
                position,
                origin: Origin::User,
            });
            push_event(
                s,
                DecisionKind::NodeAdded {
                    node_id: id.clone(),
                    label: label.clone(),
                    node_kind: kind,
                },
            );
            push_activity(
                s,
                ActivityOrigin::User,
                format!("added component \u{201c}{label}\u{201d}"),
            );
            Ok(id)
        })
    }

    /// Edit a card's content. Allowed on any node; on agent-authored nodes
    /// the `node_edited` event tells the agent to treat the new content as
    /// canonical (its next upsert should carry it forward).
    pub fn edit_node(
        &self,
        id: &NodeId,
        label: String,
        kind: NodeKind,
        description: String,
    ) -> Result<(), StoreError> {
        self.mutate(|s| {
            let n = s
                .doc
                .node_mut(id)
                .ok_or_else(|| StoreError::UnknownNode(id.clone()))?;
            n.label = label.clone();
            n.kind = kind;
            n.description = description.clone();
            push_event(
                s,
                DecisionKind::NodeEdited {
                    node_id: id.clone(),
                    label: label.clone(),
                    node_kind: kind,
                    description,
                },
            );
            push_activity(
                s,
                ActivityOrigin::User,
                format!("edited \u{201c}{label}\u{201d}"),
            );
            Ok(())
        })
    }

    /// Delete a node. User-authored nodes hard-delete (with their incident
    /// edges) and emit `node_deleted`; agent-authored nodes are only marked
    /// `removed` and emit `removal_requested` — the agent applies the real
    /// removal via `update_graph` (or pushes back). Idempotent on
    /// already-marked agent nodes.
    pub fn delete_node(&self, id: &NodeId) -> Result<(), StoreError> {
        self.mutate(|s| {
            let node = s
                .doc
                .node(id)
                .ok_or_else(|| StoreError::UnknownNode(id.clone()))?;
            let label = node.label.clone();
            match node.origin {
                Origin::User => {
                    s.doc.nodes.retain(|n| &n.id != id);
                    s.doc.edges.retain(|e| &e.from != id && &e.to != id);
                    push_event(
                        s,
                        DecisionKind::NodeDeleted {
                            node_id: id.clone(),
                        },
                    );
                    push_activity(
                        s,
                        ActivityOrigin::User,
                        format!("deleted \u{201c}{label}\u{201d}"),
                    );
                }
                Origin::Agent => {
                    let n = s.doc.node_mut(id).expect("existence checked above");
                    if n.status == ElementStatus::Removed {
                        return Ok(());
                    }
                    n.status = ElementStatus::Removed;
                    push_event(
                        s,
                        DecisionKind::RemovalRequested {
                            node_id: id.clone(),
                        },
                    );
                    push_activity(
                        s,
                        ActivityOrigin::User,
                        format!("asked the agent to remove \u{201c}{label}\u{201d}"),
                    );
                }
            }
            Ok(())
        })
    }

    /// Draw a user edge. Rejects self-loops, dangling endpoints, and
    /// duplicate `(from, to, kind)` keys.
    pub fn add_user_edge(
        &self,
        from: &NodeId,
        to: &NodeId,
        kind: EdgeKind,
    ) -> Result<(), StoreError> {
        self.mutate(|s| {
            if from == to {
                return Err(StoreError::SelfLoop);
            }
            if s.doc.node(from).is_none() {
                return Err(StoreError::UnknownNode(from.clone()));
            }
            if s.doc.node(to).is_none() {
                return Err(StoreError::UnknownNode(to.clone()));
            }
            if s.doc.edges.iter().any(|e| e.key() == (from, to, kind)) {
                return Err(StoreError::DuplicateEdge(from.clone(), to.clone()));
            }
            s.doc.edges.push(Edge {
                from: from.clone(),
                to: to.clone(),
                kind,
                label: None,
                status: ElementStatus::Proposed,
                origin: Origin::User,
            });
            push_event(
                s,
                DecisionKind::EdgeAdded {
                    from: from.clone(),
                    to: to.clone(),
                    edge_kind: kind,
                },
            );
            push_activity(
                s,
                ActivityOrigin::User,
                format!("connected {from} \u{2192} {to}"),
            );
            Ok(())
        })
    }

    /// Delete an edge of any origin — edges always hard-delete (they carry
    /// no choices or notes; the agent re-adds if it disagrees).
    pub fn delete_edge(
        &self,
        from: &NodeId,
        to: &NodeId,
        kind: EdgeKind,
    ) -> Result<(), StoreError> {
        self.mutate(|s| {
            let before = s.doc.edges.len();
            s.doc.edges.retain(|e| e.key() != (from, to, kind));
            if s.doc.edges.len() == before {
                return Err(StoreError::UnknownEdge(from.clone(), to.clone()));
            }
            push_event(
                s,
                DecisionKind::EdgeDeleted {
                    from: from.clone(),
                    to: to.clone(),
                    edge_kind: kind,
                },
            );
            push_activity(
                s,
                ActivityOrigin::User,
                format!("removed the edge {from} \u{2192} {to}"),
            );
            Ok(())
        })
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
    /// decided choices) for nodes whose ids survive, and preserving whole
    /// user-authored nodes/edges the proposal did not mention.
    pub fn apply_propose(&self, mut incoming: SessionDoc) -> Result<UpdateSummary, StoreError> {
        let validation = incoming.validate();
        if !validation.is_ok() {
            return Err(StoreError::Invalid {
                errors: validation.errors,
            });
        }
        // Everything arriving over MCP is agent-authored, whatever it claims.
        for node in &mut incoming.nodes {
            node.origin = Origin::Agent;
        }
        for edge in &mut incoming.edges {
            edge.origin = Origin::Agent;
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
            // Preserve user-authored elements the proposal did not mention.
            // (A user node whose id IS mentioned was adopted by the merge
            // above.) User edges only survive while both endpoints exist.
            let mut warnings = validation.warnings.clone();
            for prev in &previous.nodes {
                if prev.origin == Origin::User && s.doc.node(&prev.id).is_none() {
                    s.doc.nodes.push(prev.clone());
                }
            }
            for prev in &previous.edges {
                if prev.origin == Origin::User && !s.doc.edges.iter().any(|e| e.key() == prev.key())
                {
                    if s.doc.node(&prev.from).is_some() && s.doc.node(&prev.to).is_some() {
                        s.doc.edges.push(prev.clone());
                    } else {
                        warnings.push(format!(
                            "user edge {} -> {} dropped: endpoint no longer exists",
                            prev.from, prev.to
                        ));
                    }
                }
            }
            let title = if s.doc.title.is_empty() {
                "a graph".to_owned()
            } else {
                format!("\u{201c}{}\u{201d}", s.doc.title)
            };
            push_activity(s, ActivityOrigin::Agent, format!("proposed {title}"));
            summary(s, warnings)
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

/// Slug for a user-created node id: lowercase ASCII alphanumeric runs
/// joined by `-`; anything else separates. Empty input → `"component"`.
fn slugify(label: &str) -> String {
    let mut out = String::new();
    let mut pending_sep = false;
    for c in label.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('-');
            }
            pending_sep = false;
            out.push(c);
        } else {
            pending_sep = true;
        }
    }
    if out.is_empty() {
        "component".into()
    } else {
        out
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
        origin: Default::default(),
    }
}

fn apply_op(
    doc: &mut SessionDoc,
    op: GraphOp,
    announces: &mut Vec<String>,
) -> Result<(), StoreError> {
    match op {
        GraphOp::UpsertNode { mut node } => {
            // Agent-supplied content is agent-authored; upserting a user
            // node's id adopts it (user-owned fields survive via the merge).
            node.origin = Origin::Agent;
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
        GraphOp::UpsertEdge { mut edge } => {
            edge.origin = Origin::Agent;
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

    fn user_node(id: &str) -> Node {
        Node {
            id: NodeId::from(id),
            label: id.to_owned(),
            kind: crate::model::NodeKind::Component,
            description: String::new(),
            status: crate::model::ElementStatus::Proposed,
            group: None,
            choices: vec![],
            notes: vec![],
            position: None,
            origin: crate::model::Origin::User,
        }
    }

    fn user_edge(from: &str, to: &str) -> crate::model::Edge {
        crate::model::Edge {
            from: NodeId::from(from),
            to: NodeId::from(to),
            kind: crate::model::EdgeKind::DependsOn,
            label: None,
            status: crate::model::ElementStatus::Proposed,
            origin: crate::model::Origin::User,
        }
    }

    #[test]
    fn propose_preserves_unmentioned_user_elements() {
        use crate::model::Origin;
        let mut doc = demo_doc();
        doc.nodes.push(user_node("my-cache"));
        doc.edges.push(user_edge("my-cache", "postgres"));
        let store = Store::with_doc(doc);

        let mut incoming = demo_doc();
        incoming.title = "second round".into();
        // An agent claiming user origin must be normalized to agent.
        incoming.nodes[0].origin = Origin::User;
        store.apply_propose(incoming).unwrap();

        let doc = store.snapshot_doc();
        let cache = doc
            .node(&NodeId::from("my-cache"))
            .expect("user node survives a propose that omits it");
        assert_eq!(cache.origin, Origin::User);
        assert!(
            doc.edges
                .iter()
                .any(|e| e.from.as_str() == "my-cache" && e.origin == Origin::User),
            "user edge survives"
        );
        assert_eq!(
            doc.node(&NodeId::from("web-ui")).unwrap().origin,
            Origin::Agent,
            "claimed user origin is normalized"
        );
    }

    #[test]
    fn propose_drops_user_edge_with_dead_endpoint_and_warns() {
        let mut doc = demo_doc();
        doc.nodes.push(user_node("widget"));
        doc.edges.push(user_edge("widget", "job-queue"));
        let store = Store::with_doc(doc);

        let mut incoming = demo_doc();
        incoming.nodes.retain(|n| n.id.as_str() != "job-queue");
        incoming
            .edges
            .retain(|e| e.from.as_str() != "job-queue" && e.to.as_str() != "job-queue");
        let summary = store.apply_propose(incoming).unwrap();

        let doc = store.snapshot_doc();
        assert!(doc.node(&NodeId::from("widget")).is_some(), "node survives");
        assert!(
            !doc.edges.iter().any(|e| e.from.as_str() == "widget"),
            "user edge with vanished endpoint is dropped"
        );
        assert!(
            summary.warnings.iter().any(|w| w.contains("user edge")),
            "warnings: {:?}",
            summary.warnings
        );
    }

    #[test]
    fn slugify_cases() {
        assert_eq!(slugify("My Cache!"), "my-cache");
        assert_eq!(slugify("  "), "component");
        assert_eq!(slugify("Ünïcode Näme"), "n-code-n-me");
        assert_eq!(slugify("API Gateway 2"), "api-gateway-2");
    }

    #[test]
    fn add_user_node_slugs_and_dedups() {
        use crate::model::{ElementStatus, NodeKind, Origin, Point};
        let store = demo_store();
        let flush_before = store.read(|s| s.flush_seq);

        let id = store
            .add_user_node(
                "My Cache".into(),
                NodeKind::DataStore,
                Some(Point { x: 10.0, y: 20.0 }),
            )
            .unwrap();
        assert_eq!(id.as_str(), "my-cache");
        let id2 = store
            .add_user_node("My Cache".into(), NodeKind::DataStore, None)
            .unwrap();
        assert_eq!(id2.as_str(), "my-cache-2", "id collision suffixed");

        let doc = store.snapshot_doc();
        let n = doc.node(&id).unwrap();
        assert_eq!(n.origin, Origin::User);
        assert_eq!(n.status, ElementStatus::Proposed);
        assert_eq!(n.kind, NodeKind::DataStore);
        assert_eq!(n.label, "My Cache");
        assert_eq!(n.position, Some(Point { x: 10.0, y: 20.0 }));

        let log = store.read(|s| s.decision_log.clone());
        assert!(
            matches!(
                &log.last().unwrap().kind,
                DecisionKind::NodeAdded { node_id, label, node_kind }
                    if node_id == &id2 && label == "My Cache" && *node_kind == NodeKind::DataStore
            ),
            "last event: {:?}",
            log.last()
        );
        assert_eq!(
            store.read(|s| s.flush_seq),
            flush_before,
            "editing never autoflushes"
        );
        assert!(
            store
                .snapshot_meta()
                .activity
                .last()
                .unwrap()
                .text
                .contains("My Cache"),
            "activity entry present"
        );
    }

    #[test]
    fn edit_node_updates_and_events() {
        use crate::model::NodeKind;
        let store = demo_store();
        store
            .edit_node(
                &NodeId::from("redis"),
                "Redis Cluster".into(),
                NodeKind::DataStore,
                "now clustered".into(),
            )
            .unwrap();
        let doc = store.snapshot_doc();
        let n = doc.node(&NodeId::from("redis")).unwrap();
        assert_eq!(n.label, "Redis Cluster");
        assert_eq!(n.description, "now clustered");

        let log = store.read(|s| s.decision_log.clone());
        assert!(matches!(
            &log.last().unwrap().kind,
            DecisionKind::NodeEdited { node_id, label, description, .. }
                if node_id.as_str() == "redis" && label == "Redis Cluster" && description == "now clustered"
        ));
        assert!(
            store
                .edit_node(
                    &NodeId::from("ghost"),
                    "x".into(),
                    NodeKind::Component,
                    String::new()
                )
                .is_err()
        );
    }

    #[test]
    fn delete_node_matrix() {
        use crate::model::ElementStatus;
        let mut doc = demo_doc();
        doc.nodes.push(user_node("mine"));
        doc.edges.push(user_edge("mine", "postgres"));
        let store = Store::with_doc(doc);

        // User-origin: hard delete, incident edges included.
        store.delete_node(&NodeId::from("mine")).unwrap();
        let doc = store.snapshot_doc();
        assert!(doc.node(&NodeId::from("mine")).is_none());
        assert!(
            !doc.edges
                .iter()
                .any(|e| e.from.as_str() == "mine" || e.to.as_str() == "mine")
        );
        let log = store.read(|s| s.decision_log.clone());
        assert!(matches!(
            &log.last().unwrap().kind,
            DecisionKind::NodeDeleted { node_id } if node_id.as_str() == "mine"
        ));

        // Agent-origin: soft — marked removed, node stays.
        store.delete_node(&NodeId::from("redis")).unwrap();
        let doc = store.snapshot_doc();
        assert_eq!(
            doc.node(&NodeId::from("redis")).unwrap().status,
            ElementStatus::Removed
        );
        let log = store.read(|s| s.decision_log.clone());
        assert!(matches!(
            &log.last().unwrap().kind,
            DecisionKind::RemovalRequested { node_id } if node_id.as_str() == "redis"
        ));

        // Idempotent: a second delete of an already-Removed agent node is a no-op.
        let log_len = store.read(|s| s.decision_log.len());
        store.delete_node(&NodeId::from("redis")).unwrap();
        assert_eq!(store.read(|s| s.decision_log.len()), log_len);

        assert!(store.delete_node(&NodeId::from("ghost")).is_err());
    }

    #[test]
    fn add_user_edge_validates() {
        use crate::model::{EdgeKind, ElementStatus, Origin};
        let store = demo_store();
        store
            .add_user_edge(
                &NodeId::from("web-ui"),
                &NodeId::from("postgres"),
                EdgeKind::DataFlow,
            )
            .unwrap();
        let doc = store.snapshot_doc();
        let e = doc
            .edges
            .iter()
            .find(|e| e.from.as_str() == "web-ui" && e.to.as_str() == "postgres")
            .unwrap();
        assert_eq!(e.origin, Origin::User);
        assert_eq!(e.status, ElementStatus::Proposed);
        let log = store.read(|s| s.decision_log.clone());
        assert!(matches!(
            &log.last().unwrap().kind,
            DecisionKind::EdgeAdded { from, to, edge_kind }
                if from.as_str() == "web-ui" && to.as_str() == "postgres"
                    && *edge_kind == EdgeKind::DataFlow
        ));

        // Duplicate of the edge we just added, and of a demo edge.
        assert!(
            store
                .add_user_edge(
                    &NodeId::from("web-ui"),
                    &NodeId::from("postgres"),
                    EdgeKind::DataFlow
                )
                .is_err()
        );
        assert!(
            store
                .add_user_edge(
                    &NodeId::from("web-ui"),
                    &NodeId::from("api-gateway"),
                    EdgeKind::DataFlow
                )
                .is_err()
        );
        // Self-loop and dangling endpoint.
        assert!(
            store
                .add_user_edge(
                    &NodeId::from("web-ui"),
                    &NodeId::from("web-ui"),
                    EdgeKind::DependsOn
                )
                .is_err()
        );
        assert!(
            store
                .add_user_edge(
                    &NodeId::from("web-ui"),
                    &NodeId::from("ghost"),
                    EdgeKind::DependsOn
                )
                .is_err()
        );
    }

    #[test]
    fn delete_edge_events() {
        use crate::model::EdgeKind;
        let store = demo_store();
        store
            .delete_edge(
                &NodeId::from("web-ui"),
                &NodeId::from("api-gateway"),
                EdgeKind::DataFlow,
            )
            .unwrap();
        let doc = store.snapshot_doc();
        assert!(
            !doc.edges
                .iter()
                .any(|e| e.from.as_str() == "web-ui" && e.to.as_str() == "api-gateway")
        );
        let log = store.read(|s| s.decision_log.clone());
        assert!(matches!(
            &log.last().unwrap().kind,
            DecisionKind::EdgeDeleted { from, .. } if from.as_str() == "web-ui"
        ));
        assert!(
            store
                .delete_edge(
                    &NodeId::from("web-ui"),
                    &NodeId::from("api-gateway"),
                    EdgeKind::DataFlow
                )
                .is_err(),
            "already gone"
        );
    }

    #[test]
    fn agent_upsert_adopts_user_node() {
        use crate::model::{Origin, Point};
        let mut doc = demo_doc();
        let mut mine = user_node("adoptee");
        mine.notes.push(Note {
            id: NoteId::from("n1"),
            text: "keep me".into(),
            created_at: Utc::now(),
        });
        mine.position = Some(Point { x: 5.0, y: 6.0 });
        doc.nodes.push(mine);
        let store = Store::with_doc(doc);

        let mut incoming = user_node("adoptee"); // claims User — normalized away
        incoming.label = "Adoptee (enriched)".into();
        store
            .apply_update(vec![GraphOp::UpsertNode { node: incoming }])
            .unwrap();

        let doc = store.snapshot_doc();
        let n = doc.node(&NodeId::from("adoptee")).unwrap();
        assert_eq!(n.origin, Origin::Agent, "agent upsert adopts the node");
        assert_eq!(n.label, "Adoptee (enriched)");
        assert_eq!(n.notes.len(), 1, "user note survives adoption");
        assert_eq!(n.position, Some(Point { x: 5.0, y: 6.0 }));
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
