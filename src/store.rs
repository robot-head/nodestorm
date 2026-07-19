//! Shared session state between the UI (main thread / Dioxus runtime) and the
//! MCP server (dedicated tokio runtime thread).
//!
//! Concurrency model: one `std::sync::Mutex` around all state (critical
//! sections are tiny and never held across `.await`) plus a `tokio::sync::watch`
//! channel carrying the latest revision for change notification. Decisions are
//! delivered to the agent **exactly once** via an append-only log and a
//! delivery cursor advanced under the mutex; see [`Store::await_flush`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::watch;

use crate::model::{
    ActivityEntry, ActivityOrigin, Annotation, AnnotationId, AnnotationKind, ChoiceId,
    ChoiceStatus, DecisionEvent, DecisionKind, Edge, EdgeKind, ElementStatus, GraphOp, Node,
    NodeId, NodeKind, Note, NoteId, OptionId, Origin, Point, QuestionId, SessionDoc,
};

const ACTIVITY_CAP: usize = 200;
/// Undo/redo depth. Snapshots are whole docs, but docs are small (≤ a few
/// hundred nodes), so 50 is cheap.
const UNDO_CAP: usize = 50;

/// One undoable step: the doc as it was *before* a user mutation, plus the
/// undelivered decision-log tail at that moment (redo has to put events
/// back, so a length alone is not enough). Always the tail only — flushes
/// clear the stacks, so `delivery_cursor` is stable across the window.
#[derive(Debug, Clone)]
pub struct Snapshot {
    label: String,
    doc: SessionDoc,
    log_tail: Vec<DecisionEvent>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueuedChange {
    /// Stable row identity. Pending changes derive this from their current
    /// sequence; blocked changes keep a UUID because replay can renumber the
    /// live tail.
    #[serde(default)]
    pub id: String,
    pub event: DecisionEvent,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    /// Set only while the queue is displayed: persisted sessions from before
    /// queue replay and agent graph mutations have no safe replay baseline.
    #[serde(skip)]
    pub interaction_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueueEditTarget {
    pub node_id: Option<NodeId>,
    pub choice_id: Option<ChoiceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaiter {
    pub connection_id: ConnectionId,
    pub client_label: String,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendStatus {
    #[default]
    Idle,
    Sending,
    Sent,
    Reconnecting,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiToast {
    pub level: ToastLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectTarget {
    pub connection_id: ConnectionId,
    pub client_label: String,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RecipientKey {
    Agent(String),
    Anonymous(ConnectionId),
}

#[derive(Debug, Clone)]
struct Waiter {
    awaiter: Awaiter,
    recipient: RecipientKey,
}

#[derive(Debug, Clone)]
struct ReceiptTarget {
    key: RecipientKey,
    connection_id: Option<ConnectionId>,
    last_connection_id: ConnectionId,
    client_label: String,
    delivered: bool,
}

#[derive(Debug, Clone)]
struct SendReceipt {
    flush_seq: u64,
    end_cursor: usize,
    doc_at_send: SessionDoc,
    targets: Vec<ReceiptTarget>,
    /// A no-waiter autoflush remains independently claimable by named agents
    /// under the persisted `agent_flush`/`agent_cursors` contract.
    claimable: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unknown node `{0}`")]
    UnknownNode(NodeId),
    #[error("unknown choice `{choice}` on node `{node}`")]
    UnknownChoice { node: NodeId, choice: ChoiceId },
    #[error("unknown question `{0}`")]
    UnknownQuestion(QuestionId),
    #[error("unknown annotation `{0}`")]
    UnknownAnnotation(AnnotationId),
    #[error("unknown option `{option}` in choice `{choice}` on node `{node}`")]
    UnknownOption {
        node: NodeId,
        choice: ChoiceId,
        option: OptionId,
    },
    #[error("choice `{choice}` on node `{node}` is locked until its dependencies are decided")]
    ChoiceLocked { node: NodeId, choice: ChoiceId },
    #[error("document rejected: {}", errors.join("; "))]
    Invalid { errors: Vec<String> },
    #[error("edge {0} -> {1} of that kind already exists")]
    DuplicateEdge(NodeId, NodeId),
    #[error("an edge cannot connect a node to itself")]
    SelfLoop,
    #[error("no edge {0} -> {1} of that kind")]
    UnknownEdge(NodeId, NodeId),
    #[error("queued change `{0}` is unavailable")]
    UnknownQueuedChange(String),
    #[error("queued changes cannot be replayed because their baseline is unavailable")]
    MissingQueuedBaseline,
    #[error("this queued batch is currently being delivered")]
    QueuedBatchDelivering,
    #[error("no Claude session is waiting on this brainstorm")]
    NoWaitingClient,
    #[error("cannot choose a Claude recipient: {0}")]
    AmbiguousWaitingClients(String),
    #[error("this Claude session already has an active await_decisions request")]
    ConnectionAlreadyWaiting,
}

/// Full session state guarded by the store mutex.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionState {
    pub doc: SessionDoc,
    /// Append-only within a session; `seq` is the 1-based index.
    pub decision_log: Vec<DecisionEvent>,
    /// `decision_log[..delivery_cursor]` has been handed to the agent.
    pub delivery_cursor: usize,
    /// Document state before the undelivered log tail, used to replay queued
    /// edits when one is removed.
    #[serde(default)]
    pub pending_base: Option<SessionDoc>,
    /// Queued events that can no longer be replayed after an earlier edit was
    /// removed. Kept for the user to inspect or dismiss.
    #[serde(default)]
    pub blocked_changes: Vec<QueuedChange>,
    /// Why the undelivered tail cannot be safely replayed. Persisted so an
    /// autosave/restart does not make an unavailable queue look editable.
    #[serde(default)]
    pub queue_edit_error: Option<String>,
    /// Bumped by "Send to agent" (or autoflush when the last choice closes).
    pub flush_seq: u64,
    /// The last `flush_seq` actually delivered to an `await_decisions` call.
    pub delivered_flush_seq: u64,
    /// Multi-agent delivery: per-named-agent position in `decision_log` already
    /// handed to that agent. The default (unnamed) agent uses
    /// `delivery_cursor`; these run alongside it without disturbing it.
    #[serde(default)]
    pub agent_cursors: std::collections::HashMap<String, usize>,
    /// Per-named-agent last `flush_seq` delivered (the per-agent exactly-once
    /// gate, mirroring `delivered_flush_seq` for the default agent).
    #[serde(default)]
    pub agent_flush: std::collections::HashMap<String, u64>,
    pub activity: Vec<ActivityEntry>,
    /// Groups the user collapsed on the canvas. View state: persisted per
    /// session, never part of the doc, invisible to agents.
    #[serde(default)]
    pub collapsed_groups: Vec<String>,
    /// Undo/redo stacks (transient; cleared by any flush or agent turn —
    /// the undo window is "since the agent last spoke").
    #[serde(skip)]
    pub undo: Vec<Snapshot>,
    #[serde(skip)]
    pub redo: Vec<Snapshot>,
    /// Live `await_decisions` calls (transient; not persisted).
    #[serde(skip)]
    pub waiting_agents: usize,
    #[serde(skip)]
    waiters: BTreeMap<ConnectionId, Waiter>,
    #[serde(skip)]
    send_receipt: Option<SendReceipt>,
    #[serde(skip)]
    send_status: SendStatus,
    #[serde(skip)]
    toast: Option<UiToast>,
}

/// Lightweight snapshot for the top bar / panels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UiMeta {
    pub waiting_agents: usize,
    pub undelivered: usize,
    pub open_choices: usize,
    pub activity: Vec<ActivityEntry>,
    pub collapsed_groups: Vec<String>,
    pub undo_available: bool,
    pub redo_available: bool,
    pub send_status: SendStatus,
    pub toast: Option<UiToast>,
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

#[derive(Debug)]
pub struct Store {
    state: Mutex<SessionState>,
    revision_tx: watch::Sender<u64>,
    connection_tx: OnceLock<watch::Sender<u64>>,
}

impl Store {
    pub fn new(mut state: SessionState) -> Arc<Self> {
        normalize_blocked_change_ids(&mut state);
        if state.decision_log.len() > state.delivery_cursor
            && state.pending_base.is_none()
            && state.queue_edit_error.is_none()
        {
            state.queue_edit_error = Some(
                "This queue was saved before queue editing was available; send it before making changes."
                    .into(),
            );
        }
        if state.send_receipt.is_none()
            && (state.flush_seq > state.delivered_flush_seq
                || state
                    .doc
                    .nodes
                    .iter()
                    .filter_map(|node| node.agent.as_ref())
                    .any(|agent| {
                        state.agent_flush.get(agent).copied().unwrap_or(0) < state.flush_seq
                    }))
        {
            state.send_receipt = Some(SendReceipt {
                flush_seq: state.flush_seq,
                end_cursor: state.decision_log.len(),
                doc_at_send: state.doc.clone(),
                targets: vec![],
                claimable: true,
            });
        }
        let (revision_tx, _) = watch::channel(state.doc.revision);
        Arc::new(Self {
            state: Mutex::new(state),
            revision_tx,
            connection_tx: OnceLock::new(),
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

    pub(crate) fn set_connection_notifier(&self, notifier: watch::Sender<u64>) {
        assert!(
            self.connection_tx.set(notifier).is_ok(),
            "store already belongs to a session manager"
        );
    }

    fn notify_connection_projection(&self) {
        if let Some(notifier) = self.connection_tx.get() {
            notifier.send_modify(|generation| *generation += 1);
        }
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
            collapsed_groups: s.collapsed_groups.clone(),
            undo_available: !s.undo.is_empty(),
            redo_available: !s.redo.is_empty(),
            send_status: s.send_status,
            toast: s.toast.clone(),
        })
    }

    pub fn reconnecting_targets(&self) -> Vec<ReconnectTarget> {
        self.read(|s| {
            s.send_receipt
                .iter()
                .flat_map(|receipt| &receipt.targets)
                .filter(|target| !target.delivered && target.connection_id.is_none())
                .map(|target| ReconnectTarget {
                    connection_id: target.last_connection_id,
                    client_label: target.client_label.clone(),
                    agent: match &target.key {
                        RecipientKey::Agent(agent) => Some(agent.clone()),
                        RecipientKey::Anonymous(_) => None,
                    },
                })
                .collect()
        })
    }

    pub fn dismiss_toast(&self) {
        self.mutate(|s| s.toast = None);
    }

    /// Undo the most recent user step (edits and undelivered decisions
    /// only — flushes and agent turns clear the stacks). Returns whether
    /// anything happened.
    pub fn undo(&self) -> bool {
        self.mutate(|s| {
            let Some(snap) = s.undo.pop() else {
                return false;
            };
            let current = Snapshot {
                label: snap.label.clone(),
                doc: s.doc.clone(),
                log_tail: s.decision_log[s.delivery_cursor..].to_vec(),
            };
            s.redo.push(current);
            restore(s, snap, "undid");
            true
        })
    }

    /// Redo the most recently undone step.
    pub fn redo(&self) -> bool {
        self.mutate(|s| {
            let Some(snap) = s.redo.pop() else {
                return false;
            };
            let current = Snapshot {
                label: snap.label.clone(),
                doc: s.doc.clone(),
                log_tail: s.decision_log[s.delivery_cursor..].to_vec(),
            };
            s.undo.push(current);
            restore(s, snap, "redid");
            true
        })
    }

    /// One undo entry per drag: called at drag START by the UI;
    /// `set_position` itself never checkpoints.
    pub fn checkpoint_position(&self, node: &NodeId) {
        self.mutate(|s| {
            if let Some(n) = s.doc.node(node) {
                let label = format!("moved \u{201c}{}\u{201d}", n.label);
                push_undo(s, &label);
            }
        });
    }

    // ---------- user-facing mutations (called from the UI) ----------

    pub fn set_position(&self, node: &NodeId, position: Point) {
        self.mutate(|s| {
            if let Some(n) = s.doc.node_mut(node) {
                n.position = Some(position);
            }
        });
    }

    /// Collapse/expand a group on the canvas. View state only: no decision
    /// event, invisible to agents, persisted with the session.
    pub fn toggle_group_collapsed(&self, group: &str) {
        self.mutate(|s| {
            if let Some(i) = s.collapsed_groups.iter().position(|g| g == group) {
                s.collapsed_groups.remove(i);
            } else {
                s.collapsed_groups.push(group.to_owned());
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
            // Validate + gather labels immutably so the undo checkpoint only
            // lands once success is guaranteed.
            let (label, opt_label) = {
                let n = s
                    .doc
                    .node(node)
                    .ok_or_else(|| StoreError::UnknownNode(node.clone()))?;
                let c = n.choice(choice).ok_or_else(|| StoreError::UnknownChoice {
                    node: node.clone(),
                    choice: choice.clone(),
                })?;
                let o = c.options.iter().find(|o| &o.id == option).ok_or_else(|| {
                    StoreError::UnknownOption {
                        node: node.clone(),
                        choice: choice.clone(),
                        option: option.clone(),
                    }
                })?;
                if s.doc.is_choice_locked(c) {
                    return Err(StoreError::ChoiceLocked {
                        node: node.clone(),
                        choice: choice.clone(),
                    });
                }
                (n.label.clone(), o.label.clone())
            };
            push_undo(
                s,
                &format!("picked \u{201c}{opt_label}\u{201d} for {label}"),
            );
            let c = s
                .doc
                .node_mut(node)
                .expect("validated above")
                .choices
                .iter_mut()
                .find(|c| &c.id == choice)
                .expect("validated above");
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
            let label = {
                let n = s
                    .doc
                    .node(node)
                    .ok_or_else(|| StoreError::UnknownNode(node.clone()))?;
                let c = n.choice(choice).ok_or_else(|| StoreError::UnknownChoice {
                    node: node.clone(),
                    choice: choice.clone(),
                })?;
                if s.doc.is_choice_locked(c) {
                    return Err(StoreError::ChoiceLocked {
                        node: node.clone(),
                        choice: choice.clone(),
                    });
                }
                n.label.clone()
            };
            push_undo(s, &format!("dismissed a choice on {label}"));
            let c = s
                .doc
                .node_mut(node)
                .expect("validated above")
                .choices
                .iter_mut()
                .find(|c| &c.id == choice)
                .expect("validated above");
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
            let label = s
                .doc
                .node(node)
                .map(|n| n.label.clone())
                .ok_or_else(|| StoreError::UnknownNode(node.clone()))?;
            push_undo(s, &format!("added a note to {label}"));
            let note = Note {
                id: NoteId::new(uuid::Uuid::new_v4().to_string()),
                text,
                created_at: Utc::now(),
            };
            let n = s.doc.node_mut(node).expect("validated above");
            n.notes.push(note.clone());
            push_event(
                s,
                DecisionKind::NoteAdded {
                    node_id: node.clone(),
                    note,
                },
            );
            push_activity(s, ActivityOrigin::User, format!("added a note to {label}"));
            Ok(())
        })
    }

    /// Record the user's free-text answer to an agent question. Re-answering
    /// is allowed; the doc keeps the latest answer and the log preserves the
    /// history. Rides along with the next Send (does not autoflush).
    pub fn answer_question(&self, id: &QuestionId, text: String) -> Result<(), StoreError> {
        self.mutate(|s| {
            let prompt = s
                .doc
                .question(id)
                .map(|q| q.prompt.clone())
                .ok_or_else(|| StoreError::UnknownQuestion(id.clone()))?;
            push_undo(s, &format!("answered \u{201c}{prompt}\u{201d}"));
            let q = s
                .doc
                .questions
                .iter_mut()
                .find(|q| &q.id == id)
                .expect("validated above");
            q.answer = Some(text.clone());
            q.answered_at = Some(Utc::now());
            push_event(
                s,
                DecisionKind::QuestionAnswered {
                    question_id: id.clone(),
                    answer: text,
                },
            );
            push_activity(
                s,
                ActivityOrigin::User,
                format!("answered \u{201c}{prompt}\u{201d}"),
            );
            Ok(())
        })
    }

    // ---------- freehand annotations (called from the UI) ----------

    /// Draw a user-owned annotation (sticky note, arrow, or highlight region).
    /// Returns its generated id. Rides along with the next Send.
    pub fn add_annotation(
        &self,
        kind: AnnotationKind,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        text: String,
    ) -> AnnotationId {
        self.mutate(|s| {
            let id = AnnotationId::new(uuid::Uuid::new_v4().to_string());
            let annotation = Annotation {
                id: id.clone(),
                kind,
                x,
                y,
                w,
                h,
                text,
                origin: Origin::User,
            };
            push_undo(s, "added an annotation");
            s.doc.annotations.push(annotation.clone());
            push_event(s, DecisionKind::AnnotationAdded { annotation });
            push_activity(s, ActivityOrigin::User, "added an annotation".into());
            id
        })
    }

    /// Move or re-word an existing annotation.
    pub fn edit_annotation(
        &self,
        id: &AnnotationId,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        text: String,
    ) -> Result<(), StoreError> {
        self.mutate(|s| {
            if !s.doc.annotations.iter().any(|a| &a.id == id) {
                return Err(StoreError::UnknownAnnotation(id.clone()));
            }
            push_undo(s, "edited an annotation");
            let a = s
                .doc
                .annotations
                .iter_mut()
                .find(|a| &a.id == id)
                .expect("validated above");
            a.x = x;
            a.y = y;
            a.w = w;
            a.h = h;
            a.text = text;
            let annotation = a.clone();
            push_event(s, DecisionKind::AnnotationEdited { annotation });
            push_activity(s, ActivityOrigin::User, "edited an annotation".into());
            Ok(())
        })
    }

    /// Erase an annotation.
    pub fn delete_annotation(&self, id: &AnnotationId) -> Result<(), StoreError> {
        self.mutate(|s| {
            if !s.doc.annotations.iter().any(|a| &a.id == id) {
                return Err(StoreError::UnknownAnnotation(id.clone()));
            }
            push_undo(s, "deleted an annotation");
            s.doc.annotations.retain(|a| &a.id != id);
            push_event(
                s,
                DecisionKind::AnnotationDeleted {
                    annotation_id: id.clone(),
                },
            );
            push_activity(s, ActivityOrigin::User, "deleted an annotation".into());
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
            push_undo(s, &format!("added component \u{201c}{label}\u{201d}"));
            let node = Node {
                id: id.clone(),
                label: label.clone(),
                kind,
                description: String::new(),
                status: ElementStatus::Proposed,
                build: None,
                group: None,
                lane: None,
                choices: vec![],
                notes: vec![],
                agent: None,
                position,
                origin: Origin::User,
            };
            s.doc.nodes.push(node.clone());
            push_event(s, DecisionKind::NodeAdded { node });
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
        lane: Option<String>,
    ) -> Result<(), StoreError> {
        self.mutate(|s| {
            if s.doc.node(id).is_none() {
                return Err(StoreError::UnknownNode(id.clone()));
            }
            let lane = lane.filter(|l| !l.trim().is_empty());
            push_undo(s, &format!("edited \u{201c}{label}\u{201d}"));
            let n = s.doc.node_mut(id).expect("validated above");
            n.label = label.clone();
            n.kind = kind;
            n.description = description.clone();
            n.lane = lane.clone();
            push_event(
                s,
                DecisionKind::NodeEdited {
                    node_id: id.clone(),
                    label: label.clone(),
                    node_kind: kind,
                    description,
                    lane,
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
                    push_undo(s, &format!("deleted \u{201c}{label}\u{201d}"));
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
                    if s.doc.node(id).expect("checked above").status == ElementStatus::Removed {
                        return Ok(());
                    }
                    push_undo(
                        s,
                        &format!("asked the agent to remove \u{201c}{label}\u{201d}"),
                    );
                    let n = s.doc.node_mut(id).expect("existence checked above");
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
            push_undo(s, &format!("connected {from} \u{2192} {to}"));
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
            if !s.doc.edges.iter().any(|e| e.key() == (from, to, kind)) {
                return Err(StoreError::UnknownEdge(from.clone(), to.clone()));
            }
            push_undo(s, &format!("removed the edge {from} \u{2192} {to}"));
            s.doc.edges.retain(|e| e.key() != (from, to, kind));
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

    /// Generic user-action receipt for the activity feed (clipboard copies…).
    pub fn record_user_action(&self, text: String) {
        self.mutate(|s| push_activity(s, ActivityOrigin::User, text));
    }

    /// "Send to agent": flush everything undelivered (an empty flush is a
    /// valid "reviewed, proceed" signal).
    pub fn request_flush(&self, comment: Option<String>) -> Result<(), StoreError> {
        self.mutate(|s| {
            let targets = match validated_targets(s) {
                Ok(targets) => targets,
                Err(err) => {
                    s.send_status = SendStatus::Failed;
                    s.toast = Some(UiToast {
                        level: ToastLevel::Error,
                        message: err.to_string(),
                    });
                    return Err(err);
                }
            };
            if comment.as_deref().is_some_and(|c| !c.trim().is_empty()) {
                push_event(
                    s,
                    DecisionKind::FlushRequested {
                        comment: comment.map(|c| c.trim().to_owned()),
                    },
                );
            }
            s.flush_seq += 1;
            s.send_receipt = Some(SendReceipt {
                flush_seq: s.flush_seq,
                end_cursor: s.decision_log.len(),
                doc_at_send: s.doc.clone(),
                targets,
                claimable: false,
            });
            s.send_status = SendStatus::Sending;
            s.toast = None;
            clear_undo(s);
            Ok(())
        })
    }

    // ---------- agent-facing mutations (called from MCP tools) ----------

    /// Replace the document, preserving user-owned state (positions, notes,
    /// decided choices) for nodes whose ids survive, and preserving whole
    /// user-authored nodes/edges the proposal did not mention.
    pub fn apply_propose(&self, incoming: SessionDoc) -> Result<UpdateSummary, StoreError> {
        self.apply_propose_as(incoming, None)
    }

    /// Like [`Self::apply_propose`], attributing the proposed nodes to `agent`
    /// (a multi-agent session id). `None` clears attribution (single-agent).
    pub fn apply_propose_as(
        &self,
        mut incoming: SessionDoc,
        agent: Option<String>,
    ) -> Result<UpdateSummary, StoreError> {
        let validation = incoming.validate();
        if !validation.is_ok() {
            return Err(StoreError::Invalid {
                errors: validation.errors,
            });
        }
        // Everything arriving over MCP is agent-authored, whatever it claims;
        // attribution is forced from the caller's id, never trusted off the wire.
        for node in &mut incoming.nodes {
            node.origin = Origin::Agent;
            node.agent = agent.clone();
        }
        for edge in &mut incoming.edges {
            edge.origin = Origin::Agent;
        }
        Ok(self.mutate(|s| {
            clear_undo(s); // an agent turn invalidates the user's undo window
            invalidate_pending_replay(s);
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
            // Questions (and the user's answers) are part of the running
            // dialogue, not the graph a propose replaces: carry forward any
            // the new proposal did not itself restate by id.
            for prev in &previous.questions {
                if !s.doc.questions.iter().any(|q| q.id == prev.id) {
                    s.doc.questions.push(prev.clone());
                }
            }
            // Freehand annotations are a user margin layer, not graph
            // structure: they always survive a propose.
            for prev in &previous.annotations {
                if !s.doc.annotations.iter().any(|a| a.id == prev.id) {
                    s.doc.annotations.push(prev.clone());
                }
            }
            // A re-propose that drops or reopens a decided choice flags its
            // decided dependents for review (a propose carries no per-op
            // re-scopes, so nothing is exempt).
            flag_reopened_dependents(&previous, &mut s.doc, &std::collections::HashSet::new());
            let title = if s.doc.title.is_empty() {
                "a graph".to_owned()
            } else {
                format!("\u{201c}{}\u{201d}", s.doc.title)
            };
            push_activity_as(
                s,
                ActivityOrigin::Agent,
                agent.clone(),
                format!("proposed {title}"),
            );
            summary(s, warnings)
        }))
    }

    /// Apply a batch of ops atomically: all-or-nothing against validation.
    pub fn apply_update(&self, ops: Vec<GraphOp>) -> Result<UpdateSummary, StoreError> {
        self.apply_update_as(ops, None)
    }

    /// Like [`Self::apply_update`], attributing upserted nodes and the feed
    /// entry to `agent` (a multi-agent session id).
    pub fn apply_update_as(
        &self,
        ops: Vec<GraphOp>,
        agent: Option<String>,
    ) -> Result<UpdateSummary, StoreError> {
        let mutates_document = ops.iter().any(|op| !matches!(op, GraphOp::Announce { .. }));
        // Choices this batch explicitly (re-)scopes — exempt from the reopened
        // dependent flagging below so a same-turn re-scope keeps its cleared flag.
        let mut rescoped: std::collections::HashSet<(NodeId, ChoiceId)> =
            std::collections::HashSet::new();
        for op in &ops {
            match op {
                GraphOp::AddChoice { node_id, choice } => {
                    rescoped.insert((node_id.clone(), choice.id.clone()));
                }
                GraphOp::ResolveChoice {
                    node_id, choice_id, ..
                } => {
                    rescoped.insert((node_id.clone(), choice_id.clone()));
                }
                GraphOp::UpsertNode { node } => {
                    for c in &node.choices {
                        rescoped.insert((node.id.clone(), c.id.clone()));
                    }
                }
                _ => {}
            }
        }
        // Stage on a clone so a failed op or failed validation commits nothing.
        let old = self.read(|s| s.doc.clone());
        let mut doc = old.clone();
        let mut announces: Vec<String> = Vec::new();
        for op in ops {
            apply_op(&mut doc, op, &agent, &mut announces)?;
        }
        let validation = doc.validate();
        if !validation.is_ok() {
            return Err(StoreError::Invalid {
                errors: validation.errors,
            });
        }
        // Reopening a parent flags decided dependents for the agent to revisit.
        flag_reopened_dependents(&old, &mut doc, &rescoped);
        Ok(self.mutate(|s| {
            clear_undo(s); // an agent turn invalidates the user's undo window
            if mutates_document {
                invalidate_pending_replay(s);
            }
            doc.revision = s.doc.revision;
            s.doc = doc;
            if announces.is_empty() {
                push_activity_as(
                    s,
                    ActivityOrigin::Agent,
                    agent.clone(),
                    "updated the graph".into(),
                );
            }
            for msg in announces {
                push_activity_as(s, ActivityOrigin::Agent, agent.clone(), msg);
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

    /// The current undelivered tail followed by changes blocked during a
    /// replay. Blocked changes are never eligible for delivery.
    pub fn queued_changes(&self) -> Vec<QueuedChange> {
        self.read(|s| {
            s.decision_log[s.delivery_cursor..]
                .iter()
                .cloned()
                .map(|event| QueuedChange {
                    id: format!("pending:{}", event.seq),
                    interaction_error: s
                        .send_receipt
                        .as_ref()
                        .filter(|receipt| {
                            receipt.targets.iter().any(|target| !target.delivered)
                                && event.seq <= receipt.end_cursor as u64
                        })
                        .map(|_| "This batch is currently being delivered.".into())
                        .or_else(|| s.queue_edit_error.clone()),
                    event,
                    blocked_reason: None,
                })
                .chain(s.blocked_changes.iter().cloned().map(|mut change| {
                    change.interaction_error = None;
                    change
                }))
                .collect()
        })
    }

    /// Remove one undelivered change, then reconstruct the visible document
    /// from the pending baseline and the remaining replayable events.
    pub fn remove_queued_change(&self, seq: u64) -> Result<QueueEditTarget, StoreError> {
        self.mutate(|s| {
            if seq <= s.delivery_cursor as u64 {
                return Err(StoreError::UnknownQueuedChange(seq.to_string()));
            }
            let active_receipt = s
                .send_receipt
                .as_ref()
                .filter(|receipt| receipt.targets.iter().any(|target| !target.delivered));
            if active_receipt.is_some_and(|receipt| seq <= receipt.end_cursor as u64) {
                return Err(StoreError::QueuedBatchDelivering);
            }
            let replay_start = active_receipt
                .map(|receipt| receipt.end_cursor)
                .unwrap_or(s.delivery_cursor);
            let tail_index = s.decision_log[replay_start..]
                .iter()
                .position(|event| event.seq == seq)
                .ok_or_else(|| StoreError::UnknownQueuedChange(seq.to_string()))?;
            let target = queue_edit_target(&s.decision_log[replay_start + tail_index]);
            let base = match active_receipt {
                Some(receipt) => receipt.doc_at_send.clone(),
                None => s
                    .pending_base
                    .take()
                    .ok_or(StoreError::MissingQueuedBaseline)?,
            };
            let mut survivors = s.decision_log[replay_start..].to_vec();
            survivors.remove(tail_index);

            let revision = s.doc.revision;
            let mut replayed_doc = base.clone();
            let mut replayed_tail = Vec::with_capacity(survivors.len());
            for event in survivors {
                match replay_event(&mut replayed_doc, &event) {
                    Ok(()) => replayed_tail.push(event),
                    Err(reason) => s.blocked_changes.push(blocked_change(event, reason)),
                }
            }
            // Positions are canvas-only state: they do not generate decision
            // events, so rebase them from the live document after replay.
            for node in &mut replayed_doc.nodes {
                if let Some(current) = s.doc.node(&node.id) {
                    node.position = current.position;
                }
            }
            replayed_doc.revision = revision;
            s.doc = replayed_doc;
            s.decision_log.truncate(replay_start);
            for (offset, event) in replayed_tail.iter_mut().enumerate() {
                event.seq = replay_start as u64 + offset as u64 + 1;
            }
            s.decision_log.extend(replayed_tail);
            s.pending_base = (!s.decision_log[replay_start..].is_empty()).then_some(base);
            if active_receipt.is_none() {
                s.flush_seq = s.delivered_flush_seq;
                if s.send_receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.claimable)
                {
                    s.send_receipt = None;
                }
            }
            clear_undo(s);
            push_activity(
                s,
                ActivityOrigin::User,
                format!("removed queued change {seq}"),
            );
            Ok(target)
        })
    }

    /// Dismiss a replay-blocked change without altering the document or the
    /// append-only delivered portion of the decision log.
    pub fn remove_blocked_change(&self, id: &str) -> Result<QueueEditTarget, StoreError> {
        self.mutate(|s| {
            let index = s
                .blocked_changes
                .iter()
                .position(|change| change.id == id)
                .ok_or_else(|| StoreError::UnknownQueuedChange(id.to_owned()))?;
            let change = s.blocked_changes.remove(index);
            let target = queue_edit_target(&change.event);
            push_activity(
                s,
                ActivityOrigin::User,
                format!("removed blocked queued change {id}"),
            );
            Ok(target)
        })
    }

    pub fn peek_undelivered(&self) -> Vec<DecisionEvent> {
        self.read(|s| s.decision_log[s.delivery_cursor..].to_vec())
    }

    /// The undelivered tail an agent would receive: the whole tail for the
    /// default (unnamed) agent, or the events targeting a named agent.
    fn peek_undelivered_for(&self, agent: &Option<String>) -> Vec<DecisionEvent> {
        self.read(|s| match agent {
            None => s.decision_log[s.delivery_cursor..].to_vec(),
            Some(a) => {
                let cursor = s
                    .agent_cursors
                    .get(a)
                    .copied()
                    .unwrap_or(0)
                    .min(s.decision_log.len());
                s.decision_log[cursor..]
                    .iter()
                    .filter(|e| addressed_to(e, a))
                    .cloned()
                    .collect()
            }
        })
    }

    /// Atomically take the fixed receipt slice assigned to this connection.
    fn try_deliver(&self, connection_id: ConnectionId) -> Option<Vec<DecisionEvent>> {
        let taken = {
            let mut s = self.lock();
            try_deliver_locked(&mut s, connection_id)
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
    /// Exactly-once delivery is scoped to the connection targets captured by
    /// the send receipt. A disconnected target can be rebound on reconnect.
    pub async fn await_flush(
        self: &Arc<Self>,
        timeout: Duration,
        awaiter: Awaiter,
    ) -> Result<FlushOutcome, StoreError> {
        let mut rev = self.subscribe();
        let connection_id = awaiter.connection_id;
        let agent = awaiter.agent.clone();
        let _guard = WaitGuard::enter(self.clone(), awaiter)?;
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            if let Some(batch) = self.try_deliver(connection_id) {
                return Ok(FlushOutcome::Delivered(batch));
            }
            tokio::select! {
                changed = rev.changed() => {
                    if changed.is_err() {
                        return Ok(FlushOutcome::Shutdown);
                    }
                }
                () = &mut deadline => {
                    // Final re-check closes the click-vs-timeout race: a flush
                    // that landed before this point is delivered, not dropped.
                    if let Some(batch) = self.try_deliver(connection_id) {
                        return Ok(FlushOutcome::Delivered(batch));
                    }
                    return Ok(FlushOutcome::TimedOut {
                        preview: self.peek_undelivered_for(&agent),
                    });
                }
            }
        }
    }
}

/// RAII guard for the "agent is waiting" indicator; drop-safe against client
/// aborts because the future dropping runs `Drop`.
struct WaitGuard {
    store: Arc<Store>,
    connection_id: ConnectionId,
}

impl WaitGuard {
    fn enter(store: Arc<Store>, awaiter: Awaiter) -> Result<Self, StoreError> {
        let connection_id = awaiter.connection_id;
        let (rebound, revision) = {
            let mut state = store.lock();
            let rebound = register_waiter(&mut state, awaiter)?;
            state.doc.revision += 1;
            (rebound, state.doc.revision)
        };
        store.revision_tx.send_replace(revision);
        if rebound {
            store.notify_connection_projection();
        }
        Ok(Self {
            store,
            connection_id,
        })
    }
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        let orphaned = self.store.mutate(|s| {
            let removed = s.waiters.remove(&self.connection_id);
            s.waiting_agents = s.waiters.len();
            let Some(target) = s.send_receipt.as_mut().and_then(|receipt| {
                receipt.targets.iter_mut().find(|target| {
                    !target.delivered && target.connection_id == Some(self.connection_id)
                })
            }) else {
                return false;
            };
            target.last_connection_id = self.connection_id;
            target.connection_id = None;
            s.send_status = SendStatus::Reconnecting;
            let label = removed
                .as_ref()
                .map(|waiter| waiter.awaiter.client_label.as_str())
                .unwrap_or(target.client_label.as_str());
            let recipient = match &target.key {
                RecipientKey::Agent(agent) => format!("{label} ({agent})"),
                RecipientKey::Anonymous(_) => label.to_owned(),
            };
            s.toast = Some(UiToast {
                level: ToastLevel::Warning,
                message: format!("{recipient} disconnected; waiting to reconnect"),
            });
            true
        });
        if orphaned {
            self.store.notify_connection_projection();
        }
    }
}

fn register_waiter(s: &mut SessionState, awaiter: Awaiter) -> Result<bool, StoreError> {
    if s.waiters.contains_key(&awaiter.connection_id) {
        return Err(StoreError::ConnectionAlreadyWaiting);
    }
    if matches!(s.send_status, SendStatus::Sent | SendStatus::Failed) {
        s.send_status = SendStatus::Idle;
    }
    let connection_id = awaiter.connection_id;
    let recipient = match &awaiter.agent {
        Some(agent) => RecipientKey::Agent(agent.clone()),
        None => RecipientKey::Anonymous(connection_id),
    };
    let mut rebound = false;

    if let Some(receipt) = s.send_receipt.as_mut() {
        let target = match &recipient {
            RecipientKey::Agent(agent) => receipt.targets.iter_mut().find(|target| {
                !target.delivered
                    && target.connection_id.is_none()
                    && target.key == RecipientKey::Agent(agent.clone())
            }),
            RecipientKey::Anonymous(_) => {
                let orphan_count = receipt
                    .targets
                    .iter()
                    .filter(|target| {
                        !target.delivered
                            && target.connection_id.is_none()
                            && matches!(target.key, RecipientKey::Anonymous(_))
                    })
                    .count();
                let other_anonymous_waiter = s
                    .waiters
                    .values()
                    .any(|waiter| matches!(waiter.recipient, RecipientKey::Anonymous(_)));
                (orphan_count == 1 && !other_anonymous_waiter)
                    .then(|| {
                        receipt.targets.iter_mut().find(|target| {
                            !target.delivered
                                && target.connection_id.is_none()
                                && matches!(target.key, RecipientKey::Anonymous(_))
                        })
                    })
                    .flatten()
            }
        };
        if let Some(target) = target {
            target.connection_id = Some(connection_id);
            target.last_connection_id = connection_id;
            target.client_label = awaiter.client_label.clone();
            s.send_status = SendStatus::Sending;
            rebound = true;
        }
    }

    s.waiters.insert(
        connection_id,
        Waiter {
            awaiter: awaiter.clone(),
            recipient: recipient.clone(),
        },
    );
    s.waiting_agents = s.waiters.len();

    if let Some(receipt) = s.send_receipt.as_mut().filter(|receipt| receipt.claimable) {
        let pending = match &recipient {
            RecipientKey::Agent(agent) => {
                s.agent_flush.get(agent).copied().unwrap_or(0) < receipt.flush_seq
            }
            RecipientKey::Anonymous(_) => s.delivered_flush_seq < receipt.flush_seq,
        };
        let already_targeted = receipt.targets.iter().any(|target| target.key == recipient);
        if pending && !already_targeted {
            receipt.targets.push(ReceiptTarget {
                key: recipient,
                connection_id: Some(connection_id),
                last_connection_id: connection_id,
                client_label: awaiter.client_label,
                delivered: false,
            });
            s.send_status = SendStatus::Sending;
        }
        return Ok(rebound);
    }

    let unfinished_receipt = s
        .send_receipt
        .as_ref()
        .is_some_and(|receipt| receipt.targets.iter().any(|target| !target.delivered));
    if !unfinished_receipt && s.flush_seq > s.delivered_flush_seq {
        s.send_receipt = Some(SendReceipt {
            flush_seq: s.flush_seq,
            end_cursor: s.decision_log.len(),
            doc_at_send: s.doc.clone(),
            targets: vec![ReceiptTarget {
                key: recipient,
                connection_id: Some(connection_id),
                last_connection_id: connection_id,
                client_label: awaiter.client_label,
                delivered: false,
            }],
            claimable: true,
        });
        s.send_status = SendStatus::Sending;
    }
    Ok(rebound)
}

fn validated_targets(s: &SessionState) -> Result<Vec<ReceiptTarget>, StoreError> {
    if s.waiters.is_empty() {
        return Err(StoreError::NoWaitingClient);
    }
    let anonymous: Vec<&Waiter> = s
        .waiters
        .values()
        .filter(|waiter| matches!(waiter.recipient, RecipientKey::Anonymous(_)))
        .collect();
    let mut named: BTreeMap<&str, Vec<&Waiter>> = BTreeMap::new();
    for waiter in s.waiters.values() {
        if let RecipientKey::Agent(agent) = &waiter.recipient {
            named.entry(agent).or_default().push(waiter);
        }
    }

    if !named.is_empty() && !anonymous.is_empty() {
        return Err(StoreError::AmbiguousWaitingClients(
            "named and anonymous sessions are waiting together".into(),
        ));
    }
    if anonymous.len() > 1 {
        return Err(StoreError::AmbiguousWaitingClients(
            "multiple anonymous sessions are waiting".into(),
        ));
    }
    if let Some((agent, _)) = named.iter().find(|(_, waiters)| waiters.len() > 1) {
        return Err(StoreError::AmbiguousWaitingClients(format!(
            "multiple sessions claim agent {agent}"
        )));
    }

    let target = |waiter: &Waiter| ReceiptTarget {
        key: waiter.recipient.clone(),
        connection_id: Some(waiter.awaiter.connection_id),
        last_connection_id: waiter.awaiter.connection_id,
        client_label: waiter.awaiter.client_label.clone(),
        delivered: false,
    };
    if let Some(waiter) = anonymous.first() {
        return Ok(vec![target(waiter)]);
    }
    Ok(named
        .into_values()
        .map(|waiters| target(waiters[0]))
        .collect())
}

fn try_deliver_locked(
    s: &mut SessionState,
    connection_id: ConnectionId,
) -> Option<Vec<DecisionEvent>> {
    let receipt = s.send_receipt.as_mut()?;
    let target_index = receipt
        .targets
        .iter()
        .position(|target| !target.delivered && target.connection_id == Some(connection_id))?;
    let target = &receipt.targets[target_index];
    let end_cursor = receipt.end_cursor.min(s.decision_log.len());
    let batch = match &target.key {
        RecipientKey::Agent(agent) => {
            let cursor = s
                .agent_cursors
                .get(agent)
                .copied()
                .unwrap_or(0)
                .min(end_cursor);
            let batch = s.decision_log[cursor..end_cursor]
                .iter()
                .filter(|event| addressed_to(event, agent))
                .cloned()
                .collect();
            s.agent_cursors.insert(agent.clone(), end_cursor);
            s.agent_flush.insert(agent.clone(), receipt.flush_seq);
            batch
        }
        RecipientKey::Anonymous(_) => s.decision_log[s.delivery_cursor..end_cursor].to_vec(),
    };
    receipt.targets[target_index].delivered = true;
    if receipt.targets.iter().all(|target| target.delivered) {
        let actionable = s.decision_log.len() > receipt.end_cursor
            || s.waiters.keys().any(|connection_id| {
                !receipt
                    .targets
                    .iter()
                    .any(|target| target.connection_id == Some(*connection_id))
            });
        s.delivery_cursor = receipt.end_cursor;
        s.delivered_flush_seq = receipt.flush_seq;
        s.pending_base =
            (s.decision_log.len() > receipt.end_cursor).then(|| receipt.doc_at_send.clone());
        s.queue_edit_error = None;
        s.send_status = if actionable {
            SendStatus::Idle
        } else {
            SendStatus::Sent
        };
        push_activity(s, ActivityOrigin::User, "sent decisions to Claude".into());
    }
    Some(batch)
}

/// Which agent a decision is addressed to: the agent that authored the
/// node/choice/question it concerns (resolved against the live doc), or `None`
/// for unclaimed decisions (user elements, annotations, flush) that every
/// agent should hear about.
fn event_target(doc: &SessionDoc, kind: &DecisionKind) -> Option<String> {
    let node_agent = |id: &NodeId| doc.node(id).and_then(|n| n.agent.clone());
    match kind {
        DecisionKind::OptionSelected { node_id, .. }
        | DecisionKind::ChoiceDismissed { node_id, .. }
        | DecisionKind::NoteAdded { node_id, .. }
        | DecisionKind::NodeEdited { node_id, .. }
        | DecisionKind::NodeDeleted { node_id }
        | DecisionKind::RemovalRequested { node_id } => node_agent(node_id),
        DecisionKind::NodeAdded { node } => node.agent.clone().or_else(|| node_agent(&node.id)),
        DecisionKind::EdgeAdded { from, .. } | DecisionKind::EdgeDeleted { from, .. } => {
            node_agent(from)
        }
        DecisionKind::QuestionAnswered { question_id, .. } => doc
            .question(question_id)
            .and_then(|q| q.node_id.as_ref())
            .and_then(node_agent),
        DecisionKind::AnnotationAdded { .. }
        | DecisionKind::AnnotationEdited { .. }
        | DecisionKind::AnnotationDeleted { .. }
        | DecisionKind::FlushRequested { .. } => None,
    }
}

/// Whether an event should reach `agent`: addressed to it, or unclaimed. Uses
/// the target captured at event creation, not the current (mutable) doc.
fn addressed_to(event: &DecisionEvent, agent: &str) -> bool {
    match &event.target_agent {
        Some(target) => target == agent,
        None => true,
    }
}

fn push_event(s: &mut SessionState, kind: DecisionKind) {
    if matches!(s.send_status, SendStatus::Sent | SendStatus::Failed) {
        s.send_status = SendStatus::Idle;
        if s.send_receipt.as_ref().is_some_and(|receipt| {
            !receipt.claimable
                && !receipt.targets.is_empty()
                && receipt.targets.iter().all(|target| target.delivered)
        }) {
            s.send_receipt = None;
        }
    }
    if s.decision_log.len() == s.delivery_cursor && s.pending_base.is_none() {
        s.pending_base = Some(s.doc.clone());
        s.queue_edit_error = None;
    }
    // Capture the routing target now, against the doc as it stands, so a later
    // re-authoring or removal of the node cannot misroute this decision.
    let target_agent = event_target(&s.doc, &kind);
    let seq = s.decision_log.len() as u64 + 1;
    s.decision_log.push(DecisionEvent {
        seq,
        at: Utc::now(),
        target_agent,
        kind,
    });
}

/// Record the pre-mutation state as an undoable step (called at the top of
/// every user mutation once its validation has passed). A new action
/// invalidates any redo history.
fn push_undo(s: &mut SessionState, label: &str) {
    if s.decision_log.len() == s.delivery_cursor {
        s.pending_base = Some(s.doc.clone());
        s.queue_edit_error = None;
    }
    s.undo.push(Snapshot {
        label: label.to_owned(),
        doc: s.doc.clone(),
        log_tail: s.decision_log[s.delivery_cursor..].to_vec(),
    });
    if s.undo.len() > UNDO_CAP {
        let excess = s.undo.len() - UNDO_CAP;
        s.undo.drain(..excess);
    }
    s.redo.clear();
}

fn blocked_change(event: DecisionEvent, reason: String) -> QueuedChange {
    QueuedChange {
        id: format!("blocked:{}", uuid::Uuid::new_v4()),
        event,
        blocked_reason: Some(reason),
        interaction_error: None,
    }
}

fn normalize_blocked_change_ids(s: &mut SessionState) {
    for change in &mut s.blocked_changes {
        if change.id.is_empty() {
            change.id = format!("blocked:{}", uuid::Uuid::new_v4());
        }
        change.interaction_error = None;
    }
}

fn invalidate_pending_replay(s: &mut SessionState) {
    if s.decision_log.len() > s.delivery_cursor {
        s.pending_base = None;
        s.queue_edit_error = Some(
            "Queued changes cannot be edited after an agent graph update; send them before making changes."
                .into(),
        );
    }
}

fn queue_edit_target(event: &DecisionEvent) -> QueueEditTarget {
    match &event.kind {
        DecisionKind::OptionSelected {
            node_id, choice_id, ..
        }
        | DecisionKind::ChoiceDismissed {
            node_id, choice_id, ..
        } => QueueEditTarget {
            node_id: Some(node_id.clone()),
            choice_id: Some(choice_id.clone()),
        },
        DecisionKind::NoteAdded { node_id, .. }
        | DecisionKind::NodeEdited { node_id, .. }
        | DecisionKind::NodeDeleted { node_id }
        | DecisionKind::RemovalRequested { node_id } => QueueEditTarget {
            node_id: Some(node_id.clone()),
            choice_id: None,
        },
        // A question answer targets whatever node the question hangs off, but
        // the event alone does not carry it — the queue row still identifies
        // it by text, so no node/choice anchor is needed here.
        DecisionKind::QuestionAnswered { .. }
        | DecisionKind::AnnotationAdded { .. }
        | DecisionKind::AnnotationEdited { .. }
        | DecisionKind::AnnotationDeleted { .. } => QueueEditTarget {
            node_id: None,
            choice_id: None,
        },
        DecisionKind::NodeAdded { node } => QueueEditTarget {
            node_id: Some(node.id.clone()),
            choice_id: None,
        },
        DecisionKind::EdgeAdded { from, .. } | DecisionKind::EdgeDeleted { from, .. } => {
            QueueEditTarget {
                node_id: Some(from.clone()),
                choice_id: None,
            }
        }
        DecisionKind::FlushRequested { .. } => QueueEditTarget {
            node_id: None,
            choice_id: None,
        },
    }
}

fn replay_event(doc: &mut SessionDoc, event: &DecisionEvent) -> Result<(), String> {
    match &event.kind {
        DecisionKind::OptionSelected {
            node_id,
            choice_id,
            option_id,
            ..
        } => {
            let choice = doc
                .node_mut(node_id)
                .ok_or_else(|| format!("node {node_id} no longer exists"))?
                .choices
                .iter_mut()
                .find(|choice| choice.id == *choice_id)
                .ok_or_else(|| format!("choice {choice_id} no longer exists"))?;
            if !choice.options.iter().any(|option| option.id == *option_id) {
                return Err(format!("option {option_id} no longer exists"));
            }
            choice.selected = Some(option_id.clone());
            choice.status = ChoiceStatus::Decided;
        }
        DecisionKind::ChoiceDismissed {
            node_id, choice_id, ..
        } => {
            let choice = doc
                .node_mut(node_id)
                .ok_or_else(|| format!("node {node_id} no longer exists"))?
                .choices
                .iter_mut()
                .find(|choice| choice.id == *choice_id)
                .ok_or_else(|| format!("choice {choice_id} no longer exists"))?;
            choice.selected = None;
            choice.status = ChoiceStatus::Dismissed;
        }
        DecisionKind::NoteAdded { node_id, note } => {
            doc.node_mut(node_id)
                .ok_or_else(|| format!("node {node_id} no longer exists"))?
                .notes
                .push(note.clone());
        }
        DecisionKind::QuestionAnswered {
            question_id,
            answer,
        } => {
            let q = doc
                .questions
                .iter_mut()
                .find(|q| &q.id == question_id)
                .ok_or_else(|| format!("question {question_id} no longer exists"))?;
            q.answer = Some(answer.clone());
            q.answered_at = Some(event.at);
        }
        DecisionKind::AnnotationAdded { annotation } => {
            if doc.annotations.iter().any(|a| a.id == annotation.id) {
                return Err(format!("annotation {} already exists", annotation.id));
            }
            doc.annotations.push(annotation.clone());
        }
        DecisionKind::AnnotationEdited { annotation } => {
            let existing = doc
                .annotations
                .iter_mut()
                .find(|a| a.id == annotation.id)
                .ok_or_else(|| format!("annotation {} no longer exists", annotation.id))?;
            *existing = annotation.clone();
        }
        DecisionKind::AnnotationDeleted { annotation_id } => {
            if !doc.annotations.iter().any(|a| &a.id == annotation_id) {
                return Err(format!("annotation {annotation_id} no longer exists"));
            }
            doc.annotations.retain(|a| &a.id != annotation_id);
        }
        DecisionKind::FlushRequested { .. } => {}
        DecisionKind::NodeAdded { node } => {
            if doc.node(&node.id).is_some() {
                return Err(format!("node {} already exists", node.id));
            }
            doc.nodes.push(node.clone());
        }
        DecisionKind::NodeEdited {
            node_id,
            label,
            node_kind,
            description,
            lane,
        } => {
            let node = doc
                .node_mut(node_id)
                .ok_or_else(|| format!("node {node_id} no longer exists"))?;
            node.label = label.clone();
            node.kind = *node_kind;
            node.description = description.clone();
            node.lane = lane.clone();
        }
        DecisionKind::NodeDeleted { node_id } => {
            if doc.node(node_id).is_none() {
                return Err(format!("node {node_id} no longer exists"));
            }
            doc.nodes.retain(|node| node.id != *node_id);
            doc.edges
                .retain(|edge| edge.from != *node_id && edge.to != *node_id);
        }
        DecisionKind::RemovalRequested { node_id } => {
            doc.node_mut(node_id)
                .ok_or_else(|| format!("node {node_id} no longer exists"))?
                .status = ElementStatus::Removed;
        }
        DecisionKind::EdgeAdded {
            from,
            to,
            edge_kind,
        } => {
            if doc.node(from).is_none() || doc.node(to).is_none() {
                return Err(format!("edge {from} → {to} has a missing endpoint"));
            }
            if doc
                .edges
                .iter()
                .any(|edge| edge.key() == (from, to, *edge_kind))
            {
                return Err(format!("edge {from} → {to} already exists"));
            }
            doc.edges.push(Edge {
                from: from.clone(),
                to: to.clone(),
                kind: *edge_kind,
                label: None,
                status: ElementStatus::Proposed,
                origin: Origin::User,
            });
        }
        DecisionKind::EdgeDeleted {
            from,
            to,
            edge_kind,
        } => {
            if !doc
                .edges
                .iter()
                .any(|edge| edge.key() == (from, to, *edge_kind))
            {
                return Err(format!("edge {from} → {to} no longer exists"));
            }
            doc.edges
                .retain(|edge| edge.key() != (from, to, *edge_kind));
        }
    }
    Ok(())
}

/// Apply a snapshot: doc restored (revision stays monotonic — the watch
/// channel and agents both assume it never goes backward), the undelivered
/// decision-log tail replaced wholesale, and a feed receipt.
fn restore(s: &mut SessionState, snap: Snapshot, verb: &str) {
    let revision = s.doc.revision;
    s.doc = snap.doc;
    s.doc.revision = revision;
    s.decision_log.truncate(s.delivery_cursor);
    s.decision_log.extend(snap.log_tail);
    push_activity(s, ActivityOrigin::User, format!("{verb}: {}", snap.label));
}

/// Wipe both undo stacks — used when history becomes non-undoable (a flush
/// delivered decisions, or the agent mutated the graph).
fn clear_undo(s: &mut SessionState) {
    s.undo.clear();
    s.redo.clear();
}

fn push_activity(s: &mut SessionState, origin: ActivityOrigin, text: String) {
    push_activity_as(s, origin, None, text);
}

/// Like [`push_activity`], attributing the entry to a named agent for the feed
/// color/badge (multi-agent sessions).
fn push_activity_as(
    s: &mut SessionState,
    origin: ActivityOrigin,
    agent: Option<String>,
    text: String,
) {
    s.activity.push(ActivityEntry {
        at: Utc::now(),
        origin,
        text,
        agent,
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
        if s.send_receipt
            .as_ref()
            .is_some_and(|receipt| receipt.targets.iter().any(|target| !target.delivered))
        {
            return;
        }
        match validated_targets(s) {
            Ok(targets) => {
                s.flush_seq += 1;
                s.send_receipt = Some(SendReceipt {
                    flush_seq: s.flush_seq,
                    end_cursor: s.decision_log.len(),
                    doc_at_send: s.doc.clone(),
                    targets,
                    claimable: false,
                });
                s.send_status = SendStatus::Sending;
                s.toast = None;
                clear_undo(s);
                push_activity(
                    s,
                    ActivityOrigin::System,
                    "all choices decided — sending to the agent".into(),
                );
            }
            Err(StoreError::NoWaitingClient) => {
                s.flush_seq += 1;
                s.send_receipt = Some(SendReceipt {
                    flush_seq: s.flush_seq,
                    end_cursor: s.decision_log.len(),
                    doc_at_send: s.doc.clone(),
                    targets: vec![],
                    claimable: true,
                });
                clear_undo(s);
                push_activity(
                    s,
                    ActivityOrigin::System,
                    "all choices decided — sending to the agent".into(),
                );
            }
            Err(err) => {
                s.send_status = SendStatus::Failed;
                s.toast = Some(UiToast {
                    level: ToastLevel::Error,
                    message: err.to_string(),
                });
            }
        }
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
/// (Also used by the UI for suggested export filenames.)
pub(crate) fn slugify(label: &str) -> String {
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
        build: None,
        group: None,
        lane: None,
        choices: vec![],
        notes: vec![],
        agent: None,
        position: None,
        origin: Default::default(),
    }
}

fn apply_op(
    doc: &mut SessionDoc,
    op: GraphOp,
    agent: &Option<String>,
    announces: &mut Vec<String>,
) -> Result<(), StoreError> {
    match op {
        GraphOp::UpsertNode { mut node } => {
            // Agent-supplied content is agent-authored; upserting a user
            // node's id adopts it (user-owned fields survive via the merge).
            // Attribution is forced from the caller's id, never off the wire.
            node.origin = Origin::Agent;
            node.agent = agent.clone();
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
        GraphOp::Ask { mut question } => {
            // The answer is user-owned: an agent cannot pre-answer a question.
            // A re-ask (same id) refreshes the wording but keeps the user's
            // existing reply.
            if let Some(existing) = doc.questions.iter_mut().find(|q| q.id == question.id) {
                let (answer, answered_at) = (existing.answer.take(), existing.answered_at.take());
                *existing = question;
                existing.answer = answer;
                existing.answered_at = answered_at;
            } else {
                question.answer = None;
                question.answered_at = None;
                doc.questions.push(question);
            }
        }
        GraphOp::RemoveQuestion { id } => {
            doc.questions.retain(|q| q.id != id);
        }
        GraphOp::SetStatus { id, status } => {
            let node = doc.node_mut(&id).ok_or(StoreError::UnknownNode(id))?;
            node.status = status;
        }
        GraphOp::SetBuild { id, build } => {
            let node = doc.node_mut(&id).ok_or(StoreError::UnknownNode(id))?;
            node.build = build;
        }
        GraphOp::SetLane { id, lane } => {
            let node = doc.node_mut(&id).ok_or(StoreError::UnknownNode(id))?;
            node.lane = lane.filter(|l| !l.trim().is_empty());
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

/// When an agent turn reopens (or removes) a choice that other *decided*
/// choices depended on, flag those dependents `needs_review` so the agent
/// re-scopes them. Compares the pre-turn doc with the post-turn doc.
///
/// `rescoped` holds the `(node, choice)` pairs this same batch explicitly
/// added/upserted — those are skipped, so an agent that reopens a parent and
/// re-scopes the dependent (with `needs_review: false`) in one `update_graph`
/// call keeps the cleared flag.
fn flag_reopened_dependents(
    old: &SessionDoc,
    new: &mut SessionDoc,
    rescoped: &std::collections::HashSet<(NodeId, ChoiceId)>,
) {
    let mut reopened: std::collections::HashSet<(NodeId, ChoiceId)> =
        std::collections::HashSet::new();
    for node in &old.nodes {
        for choice in &node.choices {
            if choice.status != ChoiceStatus::Decided {
                continue;
            }
            let still_decided = new
                .node(&node.id)
                .and_then(|n| n.choice(&choice.id))
                .is_some_and(|c| c.status == ChoiceStatus::Decided);
            if !still_decided {
                reopened.insert((node.id.clone(), choice.id.clone()));
            }
        }
    }
    if reopened.is_empty() {
        return;
    }
    for node in &mut new.nodes {
        for choice in &mut node.choices {
            if rescoped.contains(&(node.id.clone(), choice.id.clone())) {
                continue; // the agent already re-scoped this dependent this turn
            }
            if choice.status == ChoiceStatus::Decided
                && choice
                    .depends_on
                    .iter()
                    .any(|dep| reopened.contains(&(dep.node.clone(), dep.choice.clone())))
            {
                choice.needs_review = true;
            }
        }
    }
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
            build: None,
            group: None,
            lane: None,
            choices: vec![],
            notes: vec![],
            agent: None,
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
                DecisionKind::NodeAdded { node }
                    if node.id == id2 && node.label == "My Cache" && node.kind == NodeKind::DataStore
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
                None,
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
                    String::new(),
                    None,
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
    fn undo_redo_round_trip_user_edit() {
        use crate::model::NodeKind;
        let store = demo_store();
        let id = store
            .add_user_node("Widget".into(), NodeKind::Component, None)
            .unwrap();
        let meta = store.snapshot_meta();
        assert!(meta.undo_available);
        assert!(!meta.redo_available);

        assert!(store.undo());
        assert!(store.snapshot_doc().node(&id).is_none(), "node gone");
        assert_eq!(store.read(|s| s.decision_log.len()), 0, "event gone");
        let meta = store.snapshot_meta();
        assert!(!meta.undo_available);
        assert!(meta.redo_available);
        assert!(
            meta.activity.iter().any(|a| a.text.contains("undid")),
            "receipt"
        );

        assert!(store.redo());
        assert!(store.snapshot_doc().node(&id).is_some(), "node back");
        assert_eq!(store.read(|s| s.decision_log.len()), 1, "event back");
    }

    #[test]
    fn undo_restores_decision_and_log_monotonic_revision() {
        let store = demo_store();
        pick_first_choice(&store); // demo has two open choices — no autoflush
        let rev_after_pick = store.snapshot_doc().revision;
        assert_eq!(store.read(|s| s.decision_log.len()), 1);

        assert!(store.undo());
        let doc = store.snapshot_doc();
        let choice = &doc.node(&NodeId::from("sync-engine")).unwrap().choices[0];
        assert_eq!(choice.status, ChoiceStatus::Open, "choice open again");
        assert!(choice.selected.is_none());
        assert_eq!(store.read(|s| s.decision_log.len()), 0);
        assert!(
            doc.revision > rev_after_pick,
            "revision stays monotonic: {} vs {rev_after_pick}",
            doc.revision
        );
    }

    #[tokio::test]
    async fn flush_clears_stacks() {
        use crate::model::NodeKind;
        let store = demo_store();
        store
            .add_user_node("Widget".into(), NodeKind::Component, None)
            .unwrap();
        assert!(store.snapshot_meta().undo_available);
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(100, None))
                    .await
            }
        });
        wait_until(&store, 1).await;
        store.request_flush(None).unwrap();
        let meta = store.snapshot_meta();
        assert!(!meta.undo_available, "delivered decisions are facts");
        assert!(!meta.redo_available);
        assert!(!store.undo(), "nothing to undo after a flush");
        waiting.await.unwrap().unwrap();
    }

    #[test]
    fn autoflush_clears_stacks() {
        let store = demo_store();
        pick_first_choice(&store);
        store
            .select_option(
                &NodeId::from("ws-gateway"),
                &ChoiceId::from("ws-deployment"),
                &OptionId::from("dedicated"),
                vec![],
            )
            .unwrap(); // last open choice → autoflush
        assert!(store.read(|s| s.flush_seq) > 0, "autoflush fired");
        let meta = store.snapshot_meta();
        assert!(!meta.undo_available);
        assert!(!meta.redo_available);
    }

    #[test]
    fn agent_mutations_clear_stacks() {
        use crate::model::NodeKind;
        let store = demo_store();
        store
            .add_user_node("Widget".into(), NodeKind::Component, None)
            .unwrap();
        store
            .apply_update(vec![GraphOp::SetTitle {
                title: "new title".into(),
            }])
            .unwrap();
        assert!(
            !store.snapshot_meta().undo_available,
            "agent turn invalidates the undo window"
        );

        store
            .add_user_node("Widget 2".into(), NodeKind::Component, None)
            .unwrap();
        store.apply_propose(demo_doc()).unwrap();
        assert!(!store.snapshot_meta().undo_available);
    }

    #[test]
    fn drag_checkpoints_once() {
        let store = demo_store();
        store.checkpoint_position(&NodeId::from("web-ui"));
        for i in 0..3 {
            store.set_position(
                &NodeId::from("web-ui"),
                Point {
                    x: f64::from(i) * 10.0,
                    y: 5.0,
                },
            );
        }
        assert_eq!(store.read(|s| s.undo.len()), 1, "one entry per drag");
        assert!(store.undo());
        assert!(
            store
                .snapshot_doc()
                .node(&NodeId::from("web-ui"))
                .unwrap()
                .position
                .is_none(),
            "back to auto-layout"
        );
    }

    #[test]
    fn undo_cap_holds() {
        let store = demo_store();
        for i in 0..60 {
            store
                .add_note(&NodeId::from("web-ui"), format!("note {i}"))
                .unwrap();
        }
        assert_eq!(store.read(|s| s.undo.len()), 50, "capped");
    }

    #[test]
    fn toggle_group_collapsed_round_trips_and_never_events() {
        let store = demo_store();
        let log_before = store.read(|s| s.decision_log.len());
        store.toggle_group_collapsed("Platform");
        assert_eq!(
            store.snapshot_meta().collapsed_groups,
            vec!["Platform".to_owned()]
        );
        store.toggle_group_collapsed("Platform");
        assert!(store.snapshot_meta().collapsed_groups.is_empty());
        assert_eq!(
            store.read(|s| s.decision_log.len()),
            log_before,
            "view state emits no decision events"
        );
    }

    #[test]
    fn legacy_persisted_note_and_node_events_deserialize() {
        let state: SessionState = serde_json::from_str(
            r#"{
                "doc": {"title": "Legacy session"},
                "decision_log": [
                    {
                        "seq": 1,
                        "at": "2026-07-01T12:00:00Z",
                        "kind": "note_added",
                        "node_id": "legacy-node",
                        "text": "Keep this context"
                    },
                    {
                        "seq": 2,
                        "at": "2026-07-01T12:01:00Z",
                        "kind": "node_added",
                        "node_id": "legacy-service",
                        "label": "Legacy Service",
                        "node_kind": "service"
                    }
                ],
                "delivery_cursor": 0,
                "flush_seq": 0,
                "delivered_flush_seq": 0,
                "activity": []
            }"#,
        )
        .expect("legacy persisted session deserializes");

        match &state.decision_log[0].kind {
            DecisionKind::NoteAdded { node_id, note } => {
                assert_eq!(node_id, &NodeId::from("legacy-node"));
                assert_eq!(note.id, NoteId::from("legacy-note-1"));
                assert_eq!(note.text, "Keep this context");
                assert_eq!(note.created_at.to_rfc3339(), "2026-07-01T12:00:00+00:00");
            }
            event => panic!("expected migrated note event, got {event:?}"),
        }
        match &state.decision_log[1].kind {
            DecisionKind::NodeAdded { node } => {
                assert_eq!(node.id, NodeId::from("legacy-service"));
                assert_eq!(node.label, "Legacy Service");
                assert_eq!(node.kind, NodeKind::Service);
                assert_eq!(node.origin, Origin::User);
            }
            event => panic!("expected migrated node event, got {event:?}"),
        }
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

        store.record_user_action("copied the diagram to the clipboard".into());
        let meta = store.snapshot_meta();
        let entry = meta.activity.last().unwrap();
        assert_eq!(entry.origin, ActivityOrigin::User);
        assert!(entry.text.contains("clipboard"), "text: {}", entry.text);
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

    fn awaiter(id: u64, agent: Option<&str>) -> Awaiter {
        Awaiter {
            connection_id: ConnectionId(id),
            client_label: format!("Claude {id}"),
            agent: agent.map(str::to_owned),
        }
    }

    async fn wait_until(store: &Arc<Store>, count: usize) {
        for _ in 0..50 {
            if store.snapshot_meta().waiting_agents == count {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "expected {count} waiters, got {}",
            store.snapshot_meta().waiting_agents
        );
    }

    #[tokio::test]
    async fn explicit_send_rejects_no_waiter_without_consuming_queue() {
        let store = demo_store();
        pick_first_choice(&store);
        let before = store.peek_undelivered();

        let err = store.request_flush(None).unwrap_err();

        assert!(matches!(err, StoreError::NoWaitingClient));
        assert_eq!(store.peek_undelivered(), before);
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Failed);
        assert!(
            store
                .snapshot_meta()
                .toast
                .unwrap()
                .message
                .contains("waiting")
        );
        let guard = WaitGuard::enter(store.clone(), awaiter(99, None)).unwrap();
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Idle);
        assert!(store.snapshot_meta().toast.is_some());
        store.dismiss_toast();
        assert!(store.snapshot_meta().toast.is_none());
        drop(guard);
    }

    #[tokio::test]
    async fn explicit_send_rejects_duplicate_agent_claims() {
        let store = demo_store();
        pick_first_choice(&store);
        let a = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(1, Some("alpha")))
                    .await
            }
        });
        let b = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(2, Some("alpha")))
                    .await
            }
        });
        wait_until(&store, 2).await;

        let err = store.request_flush(None).unwrap_err();

        assert!(matches!(err, StoreError::AmbiguousWaitingClients(_)));
        assert_eq!(store.read(|s| s.delivery_cursor), 0);
        a.abort();
        b.abort();
    }

    #[tokio::test]
    async fn second_await_on_same_connection_is_rejected_without_replacing_first() {
        let store = demo_store();
        pick_first_choice(&store);
        let first = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(3, Some("alpha")))
                    .await
            }
        });
        wait_until(&store, 1).await;

        let error = tokio::time::timeout(
            Duration::from_millis(50),
            store.await_flush(Duration::from_secs(30), awaiter(3, Some("beta"))),
        )
        .await
        .expect("a duplicate await is rejected immediately")
        .expect_err("a connection may own only one active await");

        assert!(matches!(error, StoreError::ConnectionAlreadyWaiting));
        assert_eq!(store.snapshot_meta().waiting_agents, 1);
        store.request_flush(None).expect("original waiter remains");
        assert!(matches!(
            first.await.expect("first task").expect("first result"),
            FlushOutcome::Delivered(batch) if batch.len() == 1
        ));
        assert_eq!(store.read(|state| state.delivery_cursor), 1);
    }

    #[tokio::test]
    async fn explicit_send_rejects_mixed_named_and_anonymous_waiters() {
        let store = demo_store();
        pick_first_choice(&store);
        let named = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(1, Some("alpha")))
                    .await
            }
        });
        let anonymous = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(2, None))
                    .await
            }
        });
        wait_until(&store, 2).await;

        assert!(matches!(
            store.request_flush(None).unwrap_err(),
            StoreError::AmbiguousWaitingClients(_)
        ));
        named.abort();
        anonymous.abort();
    }

    #[tokio::test]
    async fn receipt_excludes_edits_created_after_send() {
        let store = demo_store();
        pick_first_choice(&store);
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(1, None))
                    .await
            }
        });
        wait_until(&store, 1).await;
        store.request_flush(None).unwrap();
        store.add_annotation(AnnotationKind::Note, 1.0, 2.0, 0.0, 0.0, "later".into());

        let FlushOutcome::Delivered(batch) = waiting.await.unwrap().unwrap() else {
            panic!("expected delivery");
        };
        assert_eq!(
            batch.len(),
            1,
            "post-send annotation stays out of the receipt"
        );
        assert_eq!(
            store.peek_undelivered().len(),
            1,
            "later edit remains queued"
        );
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Idle);
    }

    #[test]
    fn removing_post_send_edit_preserves_active_receipt_claim() {
        let store = demo_store();
        pick_first_choice(&store);
        let guard = WaitGuard::enter(store.clone(), awaiter(20, None)).unwrap();
        store.request_flush(None).unwrap();
        store.add_annotation(AnnotationKind::Note, 1.0, 2.0, 0.0, 0.0, "later".into());

        store.remove_queued_change(2).unwrap();

        assert_eq!(store.read(|s| (s.flush_seq, s.delivered_flush_seq)), (1, 0));
        assert!(store.try_deliver(ConnectionId(20)).is_some());
        assert_eq!(store.read(|s| (s.flush_seq, s.delivered_flush_seq)), (1, 1));
        drop(guard);
    }

    #[test]
    fn autoflush_preserves_unfinished_explicit_receipt() {
        let store = demo_store();
        pick_first_choice(&store);
        let alpha = WaitGuard::enter(store.clone(), awaiter(21, Some("alpha"))).unwrap();
        store.request_flush(None).unwrap();
        let beta = WaitGuard::enter(store.clone(), awaiter(22, Some("beta"))).unwrap();

        store
            .dismiss_choice(
                &NodeId::from("ws-gateway"),
                &ChoiceId::from("ws-deployment"),
                None,
            )
            .unwrap();

        let batch = store
            .try_deliver(ConnectionId(21))
            .expect("original target receives the active receipt");
        assert_eq!(batch.len(), 1, "final choice stays out of active receipt");
        assert!(matches!(batch[0].kind, DecisionKind::OptionSelected { .. }));
        assert!(
            store.try_deliver(ConnectionId(22)).is_none(),
            "a later waiter is not added to the active receipt"
        );
        assert_eq!(store.read(|s| (s.flush_seq, s.delivery_cursor)), (1, 1));
        assert!(matches!(
            store.peek_undelivered().as_slice(),
            [DecisionEvent {
                kind: DecisionKind::ChoiceDismissed { .. },
                ..
            }]
        ));

        drop(alpha);
        drop(beta);
    }

    #[test]
    fn receipt_completion_is_idle_for_later_waiter() {
        let store = demo_store();
        pick_first_choice(&store);
        let alpha = WaitGuard::enter(store.clone(), awaiter(23, Some("alpha"))).unwrap();
        store.request_flush(None).unwrap();
        let beta = WaitGuard::enter(store.clone(), awaiter(24, Some("beta"))).unwrap();

        assert!(store.try_deliver(ConnectionId(23)).is_some());

        assert_eq!(store.peek_undelivered().len(), 0);
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Idle);
        drop(alpha);
        drop(beta);
    }

    #[tokio::test]
    async fn named_agent_reconnect_claims_orphaned_receipt() {
        let store = demo_store();
        pick_first_choice(&store);
        let first = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(1, Some("alpha")))
                    .await
            }
        });
        wait_until(&store, 1).await;
        store.request_flush(None).unwrap();
        first.abort();
        let _ = first.await;
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Reconnecting);
        assert_eq!(
            store.reconnecting_targets(),
            vec![ReconnectTarget {
                connection_id: ConnectionId(1),
                client_label: "Claude 1".into(),
                agent: Some("alpha".into()),
            }]
        );

        let recovered = store
            .await_flush(Duration::from_secs(1), awaiter(2, Some("alpha")))
            .await
            .unwrap();
        assert!(matches!(recovered, FlushOutcome::Delivered(_)));
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Sent);
    }

    #[tokio::test]
    async fn sole_anonymous_reconnect_claims_orphaned_receipt() {
        let store = demo_store();
        pick_first_choice(&store);
        let first = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(10, None))
                    .await
            }
        });
        wait_until(&store, 1).await;
        store.request_flush(None).unwrap();
        first.abort();
        let _ = first.await;

        let recovered = store
            .await_flush(Duration::from_secs(1), awaiter(11, None))
            .await
            .unwrap();
        assert!(matches!(recovered, FlushOutcome::Delivered(_)));
    }

    #[test]
    fn removing_an_earlier_queued_change_replays_later_changes() {
        let store = demo_store();
        let node = store
            .add_user_node("Widget".into(), NodeKind::Component, None)
            .unwrap();
        store
            .edit_node(
                &node,
                "Renamed".into(),
                NodeKind::Service,
                "edited".into(),
                None,
            )
            .unwrap();

        store.remove_queued_change(1).unwrap();

        assert!(store.snapshot_doc().node(&node).is_none());
        let changes = store.queued_changes();
        assert_eq!(changes.len(), 1);
        assert!(
            changes[0]
                .blocked_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("node"))
        );
        assert!(
            store.peek_undelivered().is_empty(),
            "blocked events do not send"
        );
    }

    #[test]
    fn removing_a_choice_pick_reopens_that_choice_and_keeps_a_note() {
        let store = demo_store();
        pick_first_choice(&store);
        store
            .add_note(&NodeId::from("sync-engine"), "Need migration notes".into())
            .unwrap();

        let target = store.remove_queued_change(1).unwrap();

        assert_eq!(target.node_id, Some(NodeId::from("sync-engine")));
        assert_eq!(
            target.choice_id,
            Some(ChoiceId::from("conflict-resolution"))
        );
        let doc = store.snapshot_doc();
        let choice = doc
            .node(&NodeId::from("sync-engine"))
            .unwrap()
            .choice(&ChoiceId::from("conflict-resolution"))
            .unwrap();
        assert_eq!(choice.status, ChoiceStatus::Open);
        assert_eq!(store.peek_undelivered().len(), 1);
    }

    #[test]
    fn agent_update_marks_pending_changes_unavailable_for_replay() {
        let store = demo_store();
        pick_first_choice(&store);
        store
            .apply_update(vec![GraphOp::SetTitle {
                title: "Agent title".into(),
            }])
            .unwrap();

        let change = store.queued_changes().pop().unwrap();

        assert!(
            change
                .interaction_error
                .as_deref()
                .is_some_and(|reason| reason.contains("agent graph update"))
        );
        assert!(matches!(
            store.remove_queued_change(change.event.seq),
            Err(StoreError::MissingQueuedBaseline)
        ));
        assert_eq!(store.snapshot_doc().title, "Agent title");
    }

    #[test]
    fn legacy_pending_changes_explain_that_they_cannot_be_replayed() {
        let store = Store::new(SessionState {
            doc: demo_doc(),
            decision_log: vec![DecisionEvent {
                seq: 1,
                at: Utc::now(),
                target_agent: None,
                kind: DecisionKind::FlushRequested {
                    comment: Some("legacy queue".into()),
                },
            }],
            ..SessionState::default()
        });

        let change = store.queued_changes().pop().unwrap();

        assert!(
            change
                .interaction_error
                .as_deref()
                .is_some_and(|reason| reason.contains("saved before queue editing"))
        );
        assert!(matches!(
            store.remove_queued_change(change.event.seq),
            Err(StoreError::MissingQueuedBaseline)
        ));
    }

    #[test]
    fn blocked_changes_have_stable_unique_ids() {
        let store = demo_store();
        let first = store
            .add_user_node("First".into(), NodeKind::Component, None)
            .unwrap();
        store
            .edit_node(
                &first,
                "First edited".into(),
                NodeKind::Component,
                String::new(),
                None,
            )
            .unwrap();
        store.remove_queued_change(1).unwrap();

        let second = store
            .add_user_node("Second".into(), NodeKind::Component, None)
            .unwrap();
        store
            .edit_node(
                &second,
                "Second edited".into(),
                NodeKind::Component,
                String::new(),
                None,
            )
            .unwrap();
        store.remove_queued_change(1).unwrap();

        let blocked: Vec<_> = store
            .queued_changes()
            .into_iter()
            .filter(|change| change.blocked_reason.is_some())
            .collect();
        assert_eq!(blocked.len(), 2);
        assert_ne!(blocked[0].id, blocked[1].id);

        store.remove_blocked_change(&blocked[1].id).unwrap();
        let remaining = store.queued_changes();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, blocked[0].id);
    }

    #[tokio::test]
    async fn removed_queue_items_are_never_delivered() {
        let store = demo_store();
        pick_first_choice(&store);
        store
            .add_note(&NodeId::from("sync-engine"), "keep cache local".into())
            .unwrap();
        store.remove_queued_change(1).unwrap();
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(101, None))
                    .await
            }
        });
        wait_until(&store, 1).await;
        store.request_flush(None).unwrap();
        let FlushOutcome::Delivered(delivered) = waiting.await.unwrap().unwrap() else {
            panic!("expected delivery");
        };

        assert_eq!(delivered.len(), 1);
        assert!(matches!(delivered[0].kind, DecisionKind::NoteAdded { .. }));
    }

    #[tokio::test]
    async fn removing_after_autoflush_cancels_delivery_until_a_new_send() {
        let store = demo_store();
        pick_first_choice(&store);
        store
            .dismiss_choice(
                &NodeId::from("ws-gateway"),
                &ChoiceId::from("ws-deployment"),
                None,
            )
            .unwrap(); // last open choice → autoflush

        store.remove_queued_change(1).unwrap();

        assert!(store.read(|s| s.flush_seq == s.delivered_flush_seq));
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(102, None))
                    .await
            }
        });
        wait_until(&store, 1).await;
        store.request_flush(None).unwrap();
        let FlushOutcome::Delivered(delivered) = waiting.await.unwrap().unwrap() else {
            panic!("expected delivery");
        };
        assert_eq!(delivered.len(), 1);
        assert!(matches!(
            delivered[0].kind,
            DecisionKind::ChoiceDismissed { .. }
        ));
    }

    #[tokio::test]
    async fn removing_a_queued_comment_is_blocked_while_delivering() {
        let store = demo_store();
        let guard = WaitGuard::enter(store.clone(), awaiter(103, None)).unwrap();
        store.request_flush(Some("hold for review".into())).unwrap();

        assert!(matches!(
            store.remove_queued_change(1),
            Err(StoreError::QueuedBatchDelivering)
        ));
        drop(guard);
    }

    #[test]
    fn removing_a_queued_change_preserves_a_later_position_change() {
        let store = demo_store();
        pick_first_choice(&store);
        let position = Point { x: 240.0, y: 120.0 };
        store.set_position(&NodeId::from("redis"), position);

        store.remove_queued_change(1).unwrap();

        assert_eq!(
            store
                .snapshot_doc()
                .node(&NodeId::from("redis"))
                .unwrap()
                .position,
            Some(position)
        );
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

    #[tokio::test]
    async fn flush_delivers_exactly_once() {
        let store = demo_store();
        pick_first_choice(&store);
        let guard = WaitGuard::enter(store.clone(), awaiter(104, None)).unwrap();
        store.request_flush(None).unwrap();
        let first = store.try_deliver(ConnectionId(104)).expect("pending flush");
        assert_eq!(first.len(), 1);
        assert!(store.try_deliver(ConnectionId(104)).is_none());
        drop(guard);
    }

    #[tokio::test]
    async fn autoflush_fires_when_last_choice_closes() {
        let store = demo_store();
        pick_first_choice(&store);
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(105, None))
                    .await
            }
        });
        wait_until(&store, 1).await;
        store
            .dismiss_choice(
                &NodeId::from("ws-gateway"),
                &ChoiceId::from("ws-deployment"),
                None,
            )
            .unwrap();
        let FlushOutcome::Delivered(batch) = waiting.await.unwrap().unwrap() else {
            panic!("expected delivery");
        };
        assert_eq!(batch.len(), 2);
    }

    #[tokio::test]
    async fn empty_flush_still_delivers() {
        let store = demo_store();
        let guard = WaitGuard::enter(store.clone(), awaiter(106, None)).unwrap();
        store
            .request_flush(Some("looks good, proceed".into()))
            .unwrap();
        let batch = store.try_deliver(ConnectionId(106)).expect("flush pending");
        assert_eq!(batch.len(), 1, "comment rides as an event");
        assert!(matches!(batch[0].kind, DecisionKind::FlushRequested { .. }));
        drop(guard);
    }

    #[tokio::test(start_paused = true)]
    async fn multiple_anonymous_awaits_reject_send() {
        let store = demo_store();
        pick_first_choice(&store);

        let a = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(5), awaiter(107, None))
                    .await
            }
        });
        let b = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(5), awaiter(108, None))
                    .await
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(store.snapshot_meta().waiting_agents, 2);
        assert!(matches!(
            store.request_flush(None),
            Err(StoreError::AmbiguousWaitingClients(_))
        ));

        let (ra, rb) = tokio::join!(a, b);
        let outcomes = [ra.unwrap().unwrap(), rb.unwrap().unwrap()];
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| matches!(o, FlushOutcome::TimedOut { .. }))
                .count(),
            2
        );
        assert_eq!(store.snapshot_meta().waiting_agents, 0, "guards released");
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_preview_does_not_consume() {
        let store = demo_store();
        pick_first_choice(&store);
        let outcome = store
            .await_flush(Duration::from_secs(1), awaiter(109, None))
            .await
            .unwrap();
        match outcome {
            FlushOutcome::TimedOut { preview } => assert_eq!(preview.len(), 1),
            other => panic!("expected timeout, got {other:?}"),
        }
        // The decision was NOT consumed: a later flush delivers it.
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(1), awaiter(110, None))
                    .await
            }
        });
        tokio::task::yield_now().await;
        store.request_flush(None).unwrap();
        let outcome = waiting.await.unwrap().unwrap();
        match outcome {
            FlushOutcome::Delivered(batch) => assert_eq!(batch.len(), 1),
            other => panic!("expected delivery, got {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn flush_after_timeout_is_returned_by_next_call_instantly() {
        let store = demo_store();
        pick_first_choice(&store);
        let FlushOutcome::TimedOut { .. } = store
            .await_flush(Duration::from_millis(10), awaiter(111, None))
            .await
            .unwrap()
        else {
            panic!("expected timeout");
        };
        let waiting = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(600), awaiter(112, None))
                    .await
            }
        });
        tokio::task::yield_now().await;
        store.request_flush(None).unwrap();
        let outcome = tokio::time::timeout(Duration::from_millis(1), waiting)
            .await
            .expect("await_flush should return immediately")
            .unwrap()
            .unwrap();
        assert!(matches!(outcome, FlushOutcome::Delivered(b) if b.len() == 1));
    }

    #[tokio::test(start_paused = true)]
    async fn wait_guard_releases_on_future_drop() {
        let store = demo_store();
        let fut = {
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(60), awaiter(113, None))
                    .await
            }
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

    #[test]
    fn lane_set_by_user_edit_and_agent_op() {
        let store = demo_store();
        // User override through the edit form emits a node_edited with the lane.
        store
            .edit_node(
                &NodeId::from("sync-engine"),
                "Sync Engine".into(),
                NodeKind::Component,
                "desc".into(),
                Some("realtime".into()),
            )
            .expect("edit");
        assert_eq!(
            store
                .snapshot_doc()
                .node(&NodeId::from("sync-engine"))
                .unwrap()
                .lane
                .as_deref(),
            Some("realtime")
        );
        assert!(matches!(
            store.peek_undelivered().last().unwrap().kind,
            DecisionKind::NodeEdited { lane: Some(_), .. }
        ));
        // Agent assigns a lane; a blank lane clears it.
        store
            .apply_update(vec![GraphOp::SetLane {
                id: NodeId::from("web-ui"),
                lane: Some("client".into()),
            }])
            .expect("set_lane");
        assert_eq!(
            store
                .snapshot_doc()
                .node(&NodeId::from("web-ui"))
                .unwrap()
                .lane
                .as_deref(),
            Some("client")
        );
        store
            .apply_update(vec![GraphOp::SetLane {
                id: NodeId::from("web-ui"),
                lane: Some("   ".into()),
            }])
            .expect("clear lane");
        assert_eq!(
            store
                .snapshot_doc()
                .node(&NodeId::from("web-ui"))
                .unwrap()
                .lane,
            None
        );
    }

    #[test]
    fn locked_choice_rejects_selection_until_parent_resolves() {
        // demo: ws-deployment depends_on conflict-resolution; both start open.
        let store = demo_store();
        let ws = NodeId::from("ws-gateway");
        let dep = ChoiceId::from("ws-deployment");
        let err = store
            .select_option(&ws, &dep, &OptionId::from("dedicated"), vec![])
            .unwrap_err();
        assert!(matches!(err, StoreError::ChoiceLocked { .. }), "{err}");
        // Dismissing is also blocked while locked.
        assert!(matches!(
            store.dismiss_choice(&ws, &dep, None).unwrap_err(),
            StoreError::ChoiceLocked { .. }
        ));
        // Decide the parent; the dependent then unlocks.
        store
            .select_option(
                &NodeId::from("sync-engine"),
                &ChoiceId::from("conflict-resolution"),
                &OptionId::from("crdt"),
                vec![],
            )
            .expect("decide parent");
        store
            .select_option(&ws, &dep, &OptionId::from("dedicated"), vec![])
            .expect("dependent now decidable");
    }

    #[test]
    fn reopening_a_parent_flags_decided_dependents() {
        use crate::model::{Choice, ChoiceOption, ChoiceRef, ChoiceStatus};
        let opt = |id: &str| ChoiceOption {
            id: id.into(),
            label: id.to_owned(),
            summary: String::new(),
            pros: vec![],
            cons: vec![],
            recommended: false,
            affects: vec![],
        };
        let mk = |id: &str, deps: Vec<ChoiceRef>| Choice {
            id: id.into(),
            prompt: format!("{id}?"),
            rationale: None,
            options: vec![opt("x"), opt("y")],
            selected: Some(OptionId::from("x")),
            status: ChoiceStatus::Decided,
            depends_on: deps,
            needs_review: false,
            reopen: false,
        };
        let mut n = user_node("n");
        n.origin = crate::model::Origin::Agent;
        n.choices = vec![
            mk("a", vec![]),
            mk(
                "b",
                vec![ChoiceRef {
                    node: NodeId::from("n"),
                    choice: ChoiceId::from("a"),
                }],
            ),
        ];
        let store = Store::with_doc(SessionDoc {
            nodes: vec![n],
            ..Default::default()
        });
        // Agent reopens the parent choice `a`.
        let mut reopened = mk("a", vec![]);
        reopened.reopen = true;
        store
            .apply_update(vec![GraphOp::AddChoice {
                node_id: NodeId::from("n"),
                choice: reopened,
            }])
            .expect("reopen");
        let n = store
            .snapshot_doc()
            .node(&NodeId::from("n"))
            .cloned()
            .unwrap();
        assert_eq!(
            n.choice(&ChoiceId::from("a")).unwrap().status,
            ChoiceStatus::Open,
            "parent reopened"
        );
        assert!(
            n.choice(&ChoiceId::from("b")).unwrap().needs_review,
            "decided dependent flagged for review"
        );
    }

    #[test]
    fn same_batch_rescope_exempts_dependent_from_review_flag() {
        use crate::model::{Choice, ChoiceOption, ChoiceRef, ChoiceStatus};
        let opt = |id: &str| ChoiceOption {
            id: id.into(),
            label: id.to_owned(),
            summary: String::new(),
            pros: vec![],
            cons: vec![],
            recommended: false,
            affects: vec![],
        };
        let mk = |id: &str, deps: Vec<ChoiceRef>| Choice {
            id: id.into(),
            prompt: format!("{id}?"),
            rationale: None,
            options: vec![opt("x"), opt("y")],
            selected: Some(OptionId::from("x")),
            status: ChoiceStatus::Decided,
            depends_on: deps,
            needs_review: false,
            reopen: false,
        };
        let mut n = user_node("n");
        n.origin = crate::model::Origin::Agent;
        n.choices = vec![
            mk("a", vec![]),
            mk(
                "b",
                vec![ChoiceRef {
                    node: NodeId::from("n"),
                    choice: ChoiceId::from("a"),
                }],
            ),
        ];
        let store = Store::with_doc(SessionDoc {
            nodes: vec![n],
            ..Default::default()
        });
        // In ONE update the agent reopens parent `a` AND re-decides dependent
        // `b` — so `b` must not be re-flagged: it was addressed this turn.
        let mut reopened = mk("a", vec![]);
        reopened.reopen = true;
        store
            .apply_update(vec![
                GraphOp::AddChoice {
                    node_id: NodeId::from("n"),
                    choice: reopened,
                },
                GraphOp::ResolveChoice {
                    node_id: NodeId::from("n"),
                    choice_id: ChoiceId::from("b"),
                    selected: Some(OptionId::from("y")),
                    dismiss: false,
                },
            ])
            .expect("reopen + rescope");
        let n = store
            .snapshot_doc()
            .node(&NodeId::from("n"))
            .cloned()
            .unwrap();
        assert_eq!(
            n.choice(&ChoiceId::from("a")).unwrap().status,
            ChoiceStatus::Open
        );
        assert!(
            !n.choice(&ChoiceId::from("b")).unwrap().needs_review,
            "a same-turn re-scope keeps the cleared flag"
        );
    }

    #[test]
    fn ask_ignores_agent_supplied_answer_on_new_question() {
        let store = demo_store();
        let q1 = crate::model::QuestionId::from("q1");
        store
            .apply_update(vec![GraphOp::Ask {
                question: crate::model::Question {
                    id: q1.clone(),
                    prompt: "?".into(),
                    node_id: None,
                    rationale: None,
                    answer: Some("agent pre-answer".into()),
                    answered_at: Some(Utc::now()),
                },
            }])
            .expect("ask");
        let q = store.snapshot_doc().question(&q1).cloned().unwrap();
        assert!(q.answer.is_none(), "agent cannot pre-answer a question");
        assert!(q.answered_at.is_none());
        // The user answers; a re-ask keeps that reply.
        store.answer_question(&q1, "real answer".into()).unwrap();
        store
            .apply_update(vec![GraphOp::Ask {
                question: crate::model::Question {
                    id: q1.clone(),
                    prompt: "reworded?".into(),
                    node_id: None,
                    rationale: None,
                    answer: None,
                    answered_at: None,
                },
            }])
            .expect("re-ask");
        let q = store.snapshot_doc().question(&q1).cloned().unwrap();
        assert_eq!(q.prompt, "reworded?");
        assert_eq!(q.answer.as_deref(), Some("real answer"));
    }

    #[test]
    fn set_build_advances_and_survives_unrelated_upsert() {
        use crate::model::BuildStatus;
        let store = demo_store();
        let id = NodeId::from("sync-engine");
        store
            .apply_update(vec![GraphOp::SetBuild {
                id: id.clone(),
                build: Some(BuildStatus::Building),
            }])
            .expect("set_build");
        assert_eq!(
            store.snapshot_doc().node(&id).unwrap().build,
            Some(BuildStatus::Building)
        );
        // An unrelated upsert (relabel, no build restated) preserves progress.
        let mut relabel = store.snapshot_doc().node(&id).cloned().unwrap();
        relabel.label = "Sync Engine v2".into();
        relabel.build = None;
        store
            .apply_update(vec![GraphOp::UpsertNode { node: relabel }])
            .expect("upsert");
        let n = store.snapshot_doc().node(&id).cloned().unwrap();
        assert_eq!(n.label, "Sync Engine v2");
        assert_eq!(n.build, Some(BuildStatus::Building), "progress preserved");
        // Clearing works via an explicit null.
        store
            .apply_update(vec![GraphOp::SetBuild {
                id: id.clone(),
                build: None,
            }])
            .expect("clear");
        assert_eq!(store.snapshot_doc().node(&id).unwrap().build, None);
    }

    #[test]
    fn annotations_add_edit_delete_and_survive_propose() {
        let store = demo_store();
        let id = store.add_annotation(AnnotationKind::Note, 10.0, 20.0, 0.0, 0.0, "hi".into());
        assert_eq!(store.snapshot_doc().annotations.len(), 1);
        assert!(matches!(
            store.peek_undelivered().last().unwrap().kind,
            DecisionKind::AnnotationAdded { .. }
        ));
        // Edit moves/re-words it.
        store
            .edit_annotation(&id, 30.0, 40.0, 0.0, 0.0, "updated".into())
            .expect("edit");
        let a = store.snapshot_doc().annotations[0].clone();
        assert_eq!((a.x, a.y), (30.0, 40.0));
        assert_eq!(a.text, "updated");
        // A propose (which carries no annotations) keeps the user's margin layer.
        let mut fresh = demo_doc();
        fresh.annotations.clear();
        store.apply_propose(fresh).expect("propose");
        assert_eq!(
            store.snapshot_doc().annotations.len(),
            1,
            "annotation survived propose"
        );
        // Delete removes it.
        store.delete_annotation(&id).expect("delete");
        assert!(store.snapshot_doc().annotations.is_empty());
        assert!(matches!(
            store.delete_annotation(&id).unwrap_err(),
            StoreError::UnknownAnnotation(_)
        ));
    }

    #[test]
    fn propose_and_upsert_attribute_nodes_to_the_agent() {
        let store = Store::new(SessionState::default());
        store
            .apply_propose_as(demo_doc(), Some("alpha".into()))
            .expect("propose");
        assert!(
            store
                .snapshot_doc()
                .nodes
                .iter()
                .all(|n| n.agent.as_deref() == Some("alpha")),
            "all proposed nodes attributed to alpha"
        );
        // A different agent's upsert re-attributes that node; a wire-supplied
        // agent value is ignored (forced from the caller's id).
        let mut n = store.snapshot_doc().nodes[0].clone();
        n.agent = Some("spoofed".into());
        let id = n.id.clone();
        store
            .apply_update_as(vec![GraphOp::UpsertNode { node: n }], Some("beta".into()))
            .expect("upsert");
        assert_eq!(
            store.snapshot_doc().node(&id).unwrap().agent.as_deref(),
            Some("beta")
        );
        // The feed entries are attributed to their author.
        assert!(
            store
                .snapshot_meta()
                .activity
                .iter()
                .any(|a| a.agent.as_deref() == Some("alpha"))
        );
    }

    #[tokio::test]
    async fn decision_routes_to_original_author_even_after_reownership() {
        use crate::model::{Choice, ChoiceOption, ChoiceStatus, Origin};
        let opt = |id: &str| ChoiceOption {
            id: id.into(),
            label: id.to_owned(),
            summary: String::new(),
            pros: vec![],
            cons: vec![],
            recommended: false,
            affects: vec![],
        };
        let mut a = user_node("a");
        a.origin = Origin::Agent;
        a.agent = Some("alpha".into());
        a.choices = vec![Choice {
            id: "ca".into(),
            prompt: "?".into(),
            rationale: None,
            options: vec![opt("x"), opt("y")],
            selected: None,
            status: ChoiceStatus::Open,
            depends_on: vec![],
            needs_review: false,
            reopen: false,
        }];
        let store = Store::with_doc(SessionDoc {
            nodes: vec![a],
            ..Default::default()
        });
        // The user decides (autoflush); the routing target is captured as alpha.
        store
            .select_option(
                &NodeId::from("a"),
                &ChoiceId::from("ca"),
                &OptionId::from("x"),
                vec![],
            )
            .unwrap();
        // Before anyone consumes the flush, beta re-authors the node.
        let mut relabel = store
            .snapshot_doc()
            .node(&NodeId::from("a"))
            .cloned()
            .unwrap();
        relabel.agent = None; // wire value ignored; forced to beta below
        store
            .apply_update_as(
                vec![GraphOp::UpsertNode { node: relabel }],
                Some("beta".into()),
            )
            .unwrap();
        assert_eq!(
            store
                .snapshot_doc()
                .node(&NodeId::from("a"))
                .unwrap()
                .agent
                .as_deref(),
            Some("beta"),
            "node is now owned by beta"
        );
        // alpha still receives the decision it originally owned…
        let FlushOutcome::Delivered(a_batch) = store
            .await_flush(Duration::from_secs(1), Some("alpha".into()))
            .await
        else {
            panic!("alpha not delivered");
        };
        assert_eq!(a_batch.len(), 1, "original author still gets the decision");
        // …and beta does not.
        let FlushOutcome::Delivered(b_batch) = store
            .await_flush(Duration::from_secs(1), Some("beta".into()))
            .await
        else {
            panic!("beta not delivered");
        };
        assert!(
            b_batch.is_empty(),
            "new owner does not steal it: {b_batch:?}"
        );
    }

    #[tokio::test]
    async fn named_agents_receive_their_own_plus_unclaimed_decisions() {
        use crate::model::{Choice, ChoiceOption, ChoiceStatus, Origin};
        let opt = |id: &str| ChoiceOption {
            id: id.into(),
            label: id.to_owned(),
            summary: String::new(),
            pros: vec![],
            cons: vec![],
            recommended: false,
            affects: vec![],
        };
        let mk = |id: &str| Choice {
            id: id.into(),
            prompt: format!("{id}?"),
            rationale: None,
            options: vec![opt("x"), opt("y")],
            selected: None,
            status: ChoiceStatus::Open,
            depends_on: vec![],
            needs_review: false,
            reopen: false,
        };
        let mut a = user_node("a");
        a.origin = Origin::Agent;
        a.agent = Some("alpha".into());
        a.choices = vec![mk("ca")];
        let mut b = user_node("b");
        b.origin = Origin::Agent;
        b.agent = Some("beta".into());
        b.choices = vec![mk("cb")];
        let mut gate = user_node("gate");
        gate.choices = vec![mk("cg")];
        let store = Store::with_doc(SessionDoc {
            nodes: vec![a, b, gate],
            ..Default::default()
        });

        // An unclaimed annotation plus a decision on each agent's node. The
        // user-owned gate remains open so this batch waits for explicit Send.
        store.add_annotation(AnnotationKind::Note, 0.0, 0.0, 0.0, 0.0, "shared".into());
        store
            .select_option(
                &NodeId::from("a"),
                &ChoiceId::from("ca"),
                &OptionId::from("x"),
                vec![],
            )
            .unwrap();
        store
            .select_option(
                &NodeId::from("b"),
                &ChoiceId::from("cb"),
                &OptionId::from("y"),
                vec![],
            )
            .unwrap();

        // Both agents await the SAME session; each gets its own slice plus the
        // unclaimed annotation — concurrent awaits are legal.
        let alpha = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(1, Some("alpha")))
                    .await
            }
        });
        let beta = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(Duration::from_secs(30), awaiter(2, Some("beta")))
                    .await
            }
        });
        wait_until(&store, 2).await;
        store.request_flush(None).unwrap();
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Sending);
        let (a_outcome, b_outcome) = tokio::join!(alpha, beta);
        let FlushOutcome::Delivered(a_batch) = a_outcome.unwrap().unwrap() else {
            panic!("alpha not delivered");
        };
        let FlushOutcome::Delivered(b_batch) = b_outcome.unwrap().unwrap() else {
            panic!("beta not delivered");
        };
        let picked_nodes = |batch: &[DecisionEvent]| {
            batch
                .iter()
                .filter_map(|e| match &e.kind {
                    DecisionKind::OptionSelected { node_id, .. } => {
                        Some(node_id.as_str().to_owned())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(picked_nodes(&a_batch), vec!["a"], "alpha only sees a");
        assert_eq!(picked_nodes(&b_batch), vec!["b"], "beta only sees b");
        let has_anno = |batch: &[DecisionEvent]| {
            batch
                .iter()
                .any(|e| matches!(e.kind, DecisionKind::AnnotationAdded { .. }))
        };
        assert!(
            has_anno(&a_batch) && has_anno(&b_batch),
            "unclaimed reaches everyone"
        );
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Sent);
        assert!(store.try_deliver(ConnectionId(1)).is_none());
    }

    #[test]
    fn send_status_stays_sending_until_every_target_finishes() {
        let store = demo_store();
        pick_first_choice(&store);
        let alpha = WaitGuard::enter(store.clone(), awaiter(201, Some("alpha"))).unwrap();
        let beta = WaitGuard::enter(store.clone(), awaiter(202, Some("beta"))).unwrap();
        store.request_flush(None).unwrap();

        assert!(store.try_deliver(ConnectionId(201)).is_some());
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Sending);
        assert!(store.try_deliver(ConnectionId(202)).is_some());
        assert_eq!(store.snapshot_meta().send_status, SendStatus::Sent);

        drop(alpha);
        drop(beta);
    }

    fn ask(store: &Arc<Store>, id: &str, node: Option<&str>) {
        store
            .apply_update(vec![GraphOp::Ask {
                question: crate::model::Question {
                    id: crate::model::QuestionId::from(id),
                    prompt: format!("prompt for {id}"),
                    node_id: node.map(NodeId::from),
                    rationale: None,
                    answer: None,
                    answered_at: None,
                },
            }])
            .expect("ask");
    }

    #[test]
    fn answer_question_updates_doc_log_and_undo() {
        let store = demo_store();
        ask(&store, "deploy", Some("sync-engine"));
        store
            .answer_question(&crate::model::QuestionId::from("deploy"), "staging".into())
            .expect("answer");
        let s = store.snapshot_state();
        let q = s
            .doc
            .question(&crate::model::QuestionId::from("deploy"))
            .unwrap();
        assert_eq!(q.answer.as_deref(), Some("staging"));
        assert!(q.answered_at.is_some());
        assert!(matches!(
            s.decision_log.last().unwrap().kind,
            DecisionKind::QuestionAnswered { .. }
        ));
        // Answering does not autoflush — it rides with the next Send.
        assert_eq!(s.delivery_cursor, 0);
        assert!(store.snapshot_meta().undo_available);
        assert!(store.undo());
        assert!(
            store
                .snapshot_doc()
                .question(&crate::model::QuestionId::from("deploy"))
                .unwrap()
                .answer
                .is_none()
        );
    }

    #[test]
    fn answer_unknown_question_errors() {
        let store = demo_store();
        let err = store
            .answer_question(&crate::model::QuestionId::from("nope"), "x".into())
            .unwrap_err();
        assert!(matches!(err, StoreError::UnknownQuestion(_)));
    }

    #[test]
    fn ask_upsert_preserves_the_users_answer() {
        let store = demo_store();
        ask(&store, "deploy", None);
        store
            .answer_question(&crate::model::QuestionId::from("deploy"), "staging".into())
            .expect("answer");
        // The agent re-asks (re-words) the same question id.
        store
            .apply_update(vec![GraphOp::Ask {
                question: crate::model::Question {
                    id: crate::model::QuestionId::from("deploy"),
                    prompt: "Which environment ships first, really?".into(),
                    node_id: Some(NodeId::from("sync-engine")),
                    rationale: Some("clarifying".into()),
                    answer: None,
                    answered_at: None,
                },
            }])
            .expect("re-ask");
        let q = store
            .snapshot_doc()
            .question(&crate::model::QuestionId::from("deploy"))
            .cloned()
            .unwrap();
        assert_eq!(q.prompt, "Which environment ships first, really?");
        assert_eq!(q.node_id, Some(NodeId::from("sync-engine")));
        // The user's answer survived the re-ask.
        assert_eq!(q.answer.as_deref(), Some("staging"));
    }

    #[test]
    fn propose_carries_questions_forward() {
        let store = demo_store();
        ask(&store, "deploy", None);
        store
            .answer_question(&crate::model::QuestionId::from("deploy"), "prod".into())
            .expect("answer");
        // A fresh proposal (no questions of its own) must not drop the dialogue.
        let mut fresh = demo_doc();
        fresh.questions.clear();
        store.apply_propose(fresh).expect("propose");
        let q = store
            .snapshot_doc()
            .question(&crate::model::QuestionId::from("deploy"))
            .cloned()
            .unwrap();
        assert_eq!(q.answer.as_deref(), Some("prod"));
    }

    #[test]
    fn removing_an_earlier_queued_change_replays_a_later_question_answer() {
        let store = demo_store();
        ask(&store, "deploy", Some("sync-engine"));
        // Two queued user actions: a note, then a question answer.
        store
            .add_note(&NodeId::from("sync-engine"), "a note".into())
            .expect("note");
        store
            .answer_question(&crate::model::QuestionId::from("deploy"), "staging".into())
            .expect("answer");
        let note_seq = store
            .peek_undelivered()
            .iter()
            .find(|e| matches!(e.kind, DecisionKind::NoteAdded { .. }))
            .unwrap()
            .seq;
        store.remove_queued_change(note_seq).expect("remove note");
        // The answer replayed onto the rebuilt doc.
        let q = store
            .snapshot_doc()
            .question(&crate::model::QuestionId::from("deploy"))
            .cloned()
            .unwrap();
        assert_eq!(q.answer.as_deref(), Some("staging"));
        assert!(store.read(|s| s.blocked_changes.is_empty()));
    }
}
