//! The nodestorm MCP tools.
//!
//! All rmcp contact stays in this module (and `server::mod`): the store knows
//! nothing about MCP, which keeps it unit-testable and shields the rest of
//! the crate from SDK churn.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Meta, ProgressNotificationParam, ServerCapabilities, ServerInfo,
};
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::model::{DecisionEvent, Edge, GraphOp, Node, NodeId, SessionDoc};
use crate::store::{Awaiter, ConnectionId, FlushOutcome, Store, StoreError, UpdateSummary};

/// Progress heartbeat cadence while an `await_decisions` call blocks. Keeps
/// Claude Code's HTTP idle-abort (default 5 min) from killing the call.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(25);
/// Default and ceiling for `await_decisions.timeout_seconds`. The default
/// stays under Claude Code's 5-minute idle abort so the call returns cleanly
/// even when no progress token was sent; the skill tells agents to re-call.
const DEFAULT_AWAIT_SECS: u64 = 240;
const MAX_AWAIT_SECS: u64 = 3600;

struct ConnectionLease {
    id: ConnectionId,
    sessions: Arc<crate::sessions::Sessions>,
    initialized: AtomicBool,
    awaiting: AtomicBool,
    transport_cancel: CancellationToken,
}

impl Drop for ConnectionLease {
    fn drop(&mut self) {
        if self.initialized.load(Ordering::Acquire) {
            self.sessions.disconnect_client(self.id);
        }
    }
}

struct ConnectionStateGuard {
    id: ConnectionId,
    sessions: Arc<crate::sessions::Sessions>,
}

struct ActiveAwaitGuard {
    connection: Arc<ConnectionLease>,
}

impl ActiveAwaitGuard {
    fn enter(connection: Arc<ConnectionLease>) -> Result<Self, ErrorData> {
        connection
            .awaiting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| store_err(StoreError::ConnectionAlreadyWaiting))?;
        Ok(Self { connection })
    }
}

impl Drop for ActiveAwaitGuard {
    fn drop(&mut self) {
        self.connection.awaiting.store(false, Ordering::Release);
    }
}

impl Drop for ConnectionStateGuard {
    fn drop(&mut self) {
        self.sessions.set_connection_connected(self.id);
    }
}

#[derive(Clone)]
pub struct NodestormServer {
    sessions: Arc<crate::sessions::Sessions>,
    transport_connections: Arc<super::TransportConnections>,
    connection: Arc<ConnectionLease>,
}

impl NodestormServer {
    pub(super) fn new(
        sessions: Arc<crate::sessions::Sessions>,
        transport_connections: Arc<super::TransportConnections>,
    ) -> Self {
        let id = sessions.next_connection_id();
        Self {
            sessions: sessions.clone(),
            transport_connections,
            connection: Arc::new(ConnectionLease {
                id,
                sessions,
                initialized: AtomicBool::new(false),
                awaiting: AtomicBool::new(false),
                transport_cancel: CancellationToken::new(),
            }),
        }
    }

    /// Route a tool call to its session's store: `None` → the active
    /// session; unknown names error listing what exists.
    fn store_for(&self, session: &Option<String>) -> Result<Arc<Store>, ErrorData> {
        self.sessions
            .resolve(session.as_deref())
            .map_err(|msg| ErrorData::invalid_params(msg, None))
    }

    /// Like [`Self::store_for`], plus the canonical session slug for results.
    fn session_and_store(
        &self,
        session: &Option<String>,
    ) -> Result<(String, Arc<Store>), ErrorData> {
        self.sessions
            .resolve_named(session.as_deref())
            .map_err(|msg| ErrorData::invalid_params(msg, None))
    }
}

// ---------- params ----------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposeGraphParams {
    /// Short session title shown in the top bar.
    pub title: String,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Node the canvas should center on.
    #[serde(default)]
    pub focus: Option<NodeId>,
    /// Message for the user's activity feed, e.g. what you just proposed.
    #[serde(default)]
    pub announce: Option<String>,
    /// Named session to propose into (created if missing). Omit for the
    /// session the user is looking at.
    #[serde(default)]
    pub session: Option<String>,
    /// Your agent id in a multi-agent session. Nodes you propose are attributed
    /// to you (color/badge) and the user's decisions on them route back to you.
    /// Omit in single-agent sessions.
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateGraphParams {
    /// Ops apply in order, atomically: if any fails validation nothing commits.
    pub ops: Vec<GraphOp>,
    /// Named session to patch. Omit for the active one.
    #[serde(default)]
    pub session: Option<String>,
    /// Your agent id in a multi-agent session (attributes upserted nodes).
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AwaitDecisionsParams {
    /// How long to block waiting for the user (seconds). On `"timeout"`,
    /// simply call this tool again — decisions are never lost.
    #[serde(default = "default_await_secs")]
    pub timeout_seconds: u64,
    /// Named session to wait on. Different Claude connections can wait on
    /// different sessions concurrently. Omit for the active one.
    #[serde(default)]
    pub session: Option<String>,
    /// Your agent id in a multi-agent session. You receive only decisions
    /// addressed to you (on nodes you authored) plus unclaimed ones, and
    /// several agents can wait on the same session at once. Omit to receive
    /// every decision (single-agent).
    #[serde(default)]
    pub agent: Option<String>,
}

fn default_await_secs() -> u64 {
    DEFAULT_AWAIT_SECS
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionOnlyParams {
    /// Named session. Omit for the active one.
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffParams {
    /// Baseline session name.
    pub a: String,
    /// Session compared against the baseline.
    pub b: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffRecordParams {
    /// Path to a decision-record `.md` file previously written by
    /// `export_markdown` (it carries a hidden snapshot used as the baseline).
    pub path: String,
    /// Session to compare against the record. Omit for the active one.
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportParams {
    /// What to return: the full Markdown decision record (default), or just
    /// the Mermaid `flowchart` block body.
    #[serde(default)]
    pub format: ExportFormat,
    /// Named session to export. Omit for the active one.
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[default]
    Markdown,
    Mermaid,
}

// ---------- results ----------

#[derive(Debug, Serialize)]
struct GraphSummary {
    /// Canonical slug of the session this mutation landed in.
    session: String,
    revision: u64,
    node_count: usize,
    open_choice_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

impl GraphSummary {
    fn new(session: String, s: UpdateSummary) -> Self {
        Self {
            session,
            revision: s.revision,
            node_count: s.node_count,
            open_choice_count: s.open_choice_count,
            warnings: s.warnings,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AwaitResult {
    /// The user sent their decisions. Act on these.
    Delivered {
        decisions: Vec<DecisionEvent>,
        open_choice_count: usize,
        revision: u64,
    },
    /// Nobody clicked "Send to agent" in time. `decisions_so_far` is a
    /// preview only — the same events WILL be re-sent by the next call.
    Timeout {
        decisions_so_far: Vec<DecisionEvent>,
        open_choice_count: usize,
        revision: u64,
        hint: &'static str,
    },
}

#[derive(Debug, Serialize)]
struct StateResult {
    /// Canonical slug of the session this state describes.
    session: String,
    doc: SessionDoc,
    undelivered_decisions: Vec<DecisionEvent>,
    decision_log_len: usize,
    agent_waiting: bool,
}

fn store_err(err: StoreError) -> ErrorData {
    ErrorData::invalid_params(err.to_string(), None)
}

fn json_result<S: Serialize>(value: S) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::json(value)?]))
}

async fn await_with_cancellation<F>(
    await_flush: F,
    request_cancel: &CancellationToken,
    transport_cancel: &CancellationToken,
) -> Result<FlushOutcome, ErrorData>
where
    F: Future<Output = Result<FlushOutcome, StoreError>>,
{
    tokio::select! {
        biased;
        () = request_cancel.cancelled() => {
            Err(ErrorData::internal_error("request cancelled", None))
        }
        () = transport_cancel.cancelled() => {
            Err(ErrorData::internal_error("transport disconnected", None))
        }
        outcome = await_flush => outcome.map_err(store_err),
    }
}

// ---------- tools ----------

#[tool_router]
impl NodestormServer {
    #[tool(
        description = "Start or replace the architecture graph on the user's canvas. Nodes are \
                       components (id = stable slug); attach open choices to the node they belong \
                       to, each option carrying pros/cons and the node ids it `affects`. User \
                       positions, notes, and already-decided choices survive re-proposes by node \
                       id. Prefer update_graph for incremental changes."
    )]
    async fn propose_graph(
        &self,
        Parameters(p): Parameters<ProposeGraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let doc = SessionDoc {
            version: SessionDoc::VERSION,
            title: p.title,
            revision: 0,
            focus: p.focus,
            nodes: p.nodes,
            edges: p.edges,
            questions: vec![],
            annotations: vec![],
        };
        // A named session is created on the spot — agents can spin up a
        // parallel brainstorm without a separate call.
        let (name, store) = match &p.session {
            Some(n) => self
                .sessions
                .get_or_create(n)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
            None => (self.sessions.active_name(), self.sessions.active_store()),
        };
        let summary = store.apply_propose_as(doc, p.agent).map_err(store_err)?;
        if let Some(msg) = p.announce {
            store.announce(msg);
        }
        json_result(GraphSummary::new(name, summary))
    }

    #[tool(
        description = "Patch the current graph with an ordered list of ops (applied atomically): \
                       upsert_node, remove_node, upsert_edge, remove_edge, add_choice, \
                       resolve_choice, ask (attach a free-form question for the user to answer in \
                       text, optionally about a node), remove_question, set_status, \
                       set_build (advance a node's implementation lifecycle: planned → building → \
                       built → verified, or null to clear), \
                       set_lane (assign a node to a labeled swimlane, or null to clear), \
                       set_focus, \
                       set_title, announce. After the user decides something, use this to apply \
                       the ripple: mark impacted nodes status=affected, open follow-up choices on \
                       them, and announce a summary. Use ask for open questions that need prose, \
                       not a pick — answers come back as question_answered decisions. As you \
                       implement, use set_build so the canvas becomes a live progress board. A \
                       choice may declare depends_on other choices ({node, choice} refs): the UI \
                       locks a dependent until its parents are decided, and reopening a parent \
                       flags decided dependents for review — dependency cycles are rejected."
    )]
    async fn update_graph(
        &self,
        Parameters(p): Parameters<UpdateGraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (name, store) = self.session_and_store(&p.session)?;
        let summary = store.apply_update_as(p.ops, p.agent).map_err(store_err)?;
        json_result(GraphSummary::new(name, summary))
    }

    #[tool(
        description = "Block until the user clicks 'Send to agent' (or every open choice is \
                       decided), then return their decisions: selected options (with the \
                       `considered` exploration trail), dismissed choices, notes, and an optional \
                       comment. On status=timeout call this tool again immediately — decisions \
                       are queued and never lost. Returns at most timeout_seconds (default 240)."
    )]
    async fn await_decisions(
        &self,
        Parameters(p): Parameters<AwaitDecisionsParams>,
        meta: Meta,
        client: Peer<RoleServer>,
        request_cancel: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        let timeout = Duration::from_secs(p.timeout_seconds.clamp(5, MAX_AWAIT_SECS));

        let (session, session_store) = self.session_and_store(&p.session)?;
        let info = self
            .sessions
            .connection(self.connection.id)
            .ok_or_else(|| ErrorData::internal_error("MCP client is not initialized", None))?;
        let _await_guard = ActiveAwaitGuard::enter(self.connection.clone())?;
        self.sessions
            .set_connection_waiting(self.connection.id, session.clone(), p.agent.clone());
        let state_guard = ConnectionStateGuard {
            id: self.connection.id,
            sessions: self.sessions.clone(),
        };

        // Best-effort heartbeat so long waits survive client idle-abort.
        let heartbeat_cancel = CancellationToken::new();
        if let Some(token) = meta.get_progress_token() {
            let store = session_store.clone();
            let heartbeat_cancel = heartbeat_cancel.clone();
            tokio::spawn(async move {
                let mut elapsed = 0u64;
                loop {
                    let open = store.snapshot_meta().open_choices;
                    let note = ProgressNotificationParam::new(token.clone(), elapsed as f64)
                        .with_message(format!("waiting for user decisions ({open} open)"));
                    if client.notify_progress(note).await.is_err() {
                        break;
                    }
                    tokio::select! {
                        () = heartbeat_cancel.cancelled() => break,
                        () = tokio::time::sleep(HEARTBEAT_EVERY) => {
                            elapsed += HEARTBEAT_EVERY.as_secs();
                        }
                    }
                }
            });
        }

        let await_flush = session_store.await_flush(
            timeout,
            Awaiter {
                connection_id: self.connection.id,
                client_label: format!("{} {}", info.client_name, info.version),
                agent: p.agent.clone(),
            },
        );
        let outcome = await_with_cancellation(
            await_flush,
            &request_cancel,
            &self.connection.transport_cancel,
        )
        .await;
        heartbeat_cancel.cancel();
        let outcome = outcome?;

        let (open, revision) = session_store.read(|s| (s.doc.open_choice_count(), s.doc.revision));
        match outcome {
            FlushOutcome::Delivered(decisions) => {
                drop(state_guard);
                self.sessions.set_connection_receiving(
                    self.connection.id,
                    session,
                    p.agent.clone(),
                );
                json_result(AwaitResult::Delivered {
                    decisions,
                    open_choice_count: open,
                    revision,
                })
            }
            FlushOutcome::TimedOut { preview } => json_result(AwaitResult::Timeout {
                decisions_so_far: preview,
                open_choice_count: open,
                revision,
                hint: "call await_decisions again; the user has not clicked Send yet",
            }),
            FlushOutcome::Shutdown => Err(ErrorData::internal_error(
                "nodestorm is shutting down",
                None,
            )),
        }
    }

    #[tool(
        description = "Read the full current graph plus any undelivered decisions without \
                       blocking. Use to resync after a transport error or before resuming a \
                       session."
    )]
    async fn get_state(
        &self,
        Parameters(p): Parameters<SessionOnlyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (name, store) = self.session_and_store(&p.session)?;
        let (doc, undelivered, log_len, waiting) = store.read(|s| {
            (
                s.doc.clone(),
                s.decision_log[s.delivery_cursor..].to_vec(),
                s.decision_log.len(),
                s.waiting_agents > 0,
            )
        });
        json_result(StateResult {
            session: name,
            doc,
            undelivered_decisions: undelivered,
            decision_log_len: log_len,
            agent_waiting: waiting,
        })
    }

    #[tool(description = "Wipe a session's canvas and decision log to start a fresh brainstorm.")]
    async fn clear_session(
        &self,
        Parameters(p): Parameters<SessionOnlyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (name, store) = self.session_and_store(&p.session)?;
        let summary = store.clear_session();
        json_result(GraphSummary::new(name, summary))
    }

    #[tool(
        description = "Compare two named sessions structurally — components added/removed/changed \
                       (field-level), edges added/removed, and decision drift (newly decided, \
                       decided differently, reopened, dismissed) — as plain Markdown. Useful \
                       before re-proposing into an older brainstorm."
    )]
    async fn diff_sessions(
        &self,
        Parameters(p): Parameters<DiffParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (a_name, a_store) = self.session_and_store(&Some(p.a))?;
        let (b_name, b_store) = self.session_and_store(&Some(p.b))?;
        let a_doc = a_store.snapshot_doc();
        let b_doc = b_store.snapshot_doc();
        Ok(CallToolResult::success(vec![ContentBlock::text(
            crate::diff::diff_docs(&a_name, &a_doc, &b_name, &b_doc),
        )]))
    }

    #[tool(
        description = "Compare a session against a previously exported decision-record `.md` file \
                       (written by export_markdown, which embeds a hidden snapshot). Reports how \
                       the current graph has drifted from what was recorded — components, edges, \
                       and decisions added/removed/changed — as Markdown. Useful to see what has \
                       changed since a record was committed to the repo."
    )]
    async fn diff_record(
        &self,
        Parameters(p): Parameters<DiffRecordParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (name, store) = self.session_and_store(&p.session)?;
        let text = std::fs::read_to_string(&p.path)
            .map_err(|e| ErrorData::invalid_params(format!("cannot read {}: {e}", p.path), None))?;
        let doc = store.snapshot_doc();
        let record_name = std::path::Path::new(&p.path)
            .file_name()
            .map_or_else(|| p.path.clone(), |n| n.to_string_lossy().into_owned());
        let diff = crate::diff::diff_doc_vs_record(&record_name, &text, &name, &doc)
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(diff)]))
    }

    #[tool(
        description = "List the named brainstorm sessions: name, whether it is the one the user \
                       is looking at (active), node/open-choice counts, and whether an agent is \
                       currently waiting in it. Address any tool at a session via its `session` \
                       param; only the user switches which session is active on screen."
    )]
    async fn list_sessions(
        &self,
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        json_result(serde_json::json!({
            "sessions": self.sessions.list(),
            "active": self.sessions.active_name(),
        }))
    }

    #[tool(
        description = "Export the current brainstorm as a Markdown decision record with an \
                       embedded Mermaid architecture diagram: decisions with pros/cons and the \
                       user's considered trail, dismissed choices, open questions, notes, and a \
                       component inventory. Returns plain Markdown — write it into the user's \
                       repo (e.g. docs/decisions/). Pass format: \"mermaid\" for just the \
                       diagram block."
    )]
    async fn export_markdown(
        &self,
        Parameters(p): Parameters<ExportParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let text = self.store_for(&p.session)?.read(|s| match p.format {
            ExportFormat::Markdown => {
                crate::export::render_markdown(&s.doc, &s.decision_log, chrono::Utc::now())
            }
            ExportFormat::Mermaid => crate::export::render_mermaid(&s.doc),
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }
}

#[tool_handler]
impl ServerHandler for NodestormServer {
    async fn on_initialized(&self, context: rmcp::service::NotificationContext<RoleServer>) {
        let Some(info) = context.peer.peer_info() else {
            return;
        };
        if !self.connection.initialized.swap(true, Ordering::AcqRel) {
            self.sessions.connect_client(
                self.connection.id,
                info.client_info.name.clone(),
                info.client_info.version.clone(),
            );
            if let Some(session_id) = context
                .extensions
                .get::<axum::http::request::Parts>()
                .and_then(|parts| parts.headers.get(super::HEADER_SESSION_ID))
                .and_then(|value| value.to_str().ok())
            {
                self.transport_connections.connect(
                    session_id.to_owned(),
                    self.connection.id,
                    self.connection.transport_cancel.clone(),
                );
            }
        }
    }

    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info.name = "nodestorm".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info.instructions = Some(
            "nodestorm renders your architecture proposals as a live node graph the user \
             can see. Loop: (1) propose_graph with components, edges, and open choices \
             attached to the nodes they belong to; (2) await_decisions — the user picks \
             options, writes notes, and clicks Send; on status=timeout just call it again; \
             (3) apply the ripple with update_graph (mark affected nodes, open follow-up \
             choices, announce a summary) and repeat. Keep discussing in the terminal; the \
             canvas is a companion, not a replacement. When the brainstorm winds down, call \
             export_markdown and save the decision record into the user's repo. Named \
             sessions: every tool takes an optional `session`; propose_graph creates missing \
             ones, list_sessions shows what exists, and only the user switches which session \
             is on screen — say which session you touched."
                .into(),
        );
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AnnotationKind;
    use crate::store::{SendStatus, SessionState};

    #[tokio::test]
    async fn ready_cancellation_does_not_consume_a_ready_receipt() {
        let store = Store::new(SessionState::default());
        store.add_annotation(AnnotationKind::Note, 0.0, 0.0, 0.0, 0.0, "ready".into());
        let first = tokio::spawn({
            let store = store.clone();
            async move {
                store
                    .await_flush(
                        Duration::from_secs(30),
                        Awaiter {
                            connection_id: ConnectionId(1),
                            client_label: "Claude alpha".into(),
                            agent: Some("alpha".into()),
                        },
                    )
                    .await
            }
        });
        while store.snapshot_meta().waiting_agents != 1 {
            tokio::task::yield_now().await;
        }
        store.request_flush(None).expect("create active receipt");
        first.abort();
        let _ = first.await;
        assert2::assert!((store.snapshot_meta().send_status) == (SendStatus::Reconnecting));

        let request_cancel = CancellationToken::new();
        request_cancel.cancel();
        let transport_cancel = CancellationToken::new();
        let result = await_with_cancellation(
            store.await_flush(
                Duration::from_secs(30),
                Awaiter {
                    connection_id: ConnectionId(2),
                    client_label: "Claude alpha reconnected".into(),
                    agent: Some("alpha".into()),
                },
            ),
            &request_cancel,
            &transport_cancel,
        )
        .await;

        assert2::assert!(
            result
                .expect_err("ready cancellation wins")
                .to_string()
                .contains("request cancelled")
        );
        assert2::assert!((store.read(|state| state.delivery_cursor)) == (0));
        assert2::assert!((store.snapshot_meta().waiting_agents) == (0));
        assert2::assert!((store.snapshot_meta().send_status) == (SendStatus::Reconnecting));
    }

    use rmcp::ServiceExt;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::CallToolRequestParams;

    struct TestDir(std::path::PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn server() -> (TestDir, NodestormServer) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nodestorm-tools-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let sessions = crate::sessions::Sessions::open(dir.join("sessions"), None).unwrap();
        let transport_connections =
            Arc::new(super::super::TransportConnections::new(sessions.clone()));
        let dir = TestDir(dir);
        (dir, NodestormServer::new(sessions, transport_connections))
    }

    fn node(id: &str) -> Node {
        serde_json::from_value(serde_json::json!({"id": id, "label": id})).unwrap()
    }

    fn result_text(result: &CallToolResult) -> String {
        serde_json::to_string(result).unwrap()
    }

    #[test]
    fn parameter_defaults_and_result_helpers_are_exact() {
        let params: AwaitDecisionsParams = serde_json::from_value(serde_json::json!({})).unwrap();
        assert2::assert!((params.timeout_seconds) == (240));
        assert2::assert!((default_await_secs()) == (240));

        let result = json_result(serde_json::json!({"answer": 42})).unwrap();
        assert2::assert!(result_text(&result).contains("answer"));
        assert2::assert!(result_text(&result).contains("42"));

        let error = store_err(StoreError::UnknownNode(NodeId::from("missing")));
        assert2::assert!(error.message.contains("missing"));
    }

    #[test]
    fn lifecycle_guards_restore_connection_state_on_drop() {
        let (_dir, server) = server();
        let id = server.connection.id;
        server
            .sessions
            .connect_client(id, "client".into(), "1".into());
        server
            .sessions
            .set_connection_waiting(id, "default".into(), Some("alpha".into()));
        {
            let _guard = ConnectionStateGuard {
                id,
                sessions: server.sessions.clone(),
            };
        }
        assert2::assert!(
            (server.sessions.connection(id).unwrap().state)
                == (crate::sessions::ConnectionState::Connected)
        );

        let first = ActiveAwaitGuard::enter(server.connection.clone()).unwrap();
        assert2::assert!(ActiveAwaitGuard::enter(server.connection.clone()).is_err());
        drop(first);
        assert2::assert!(ActiveAwaitGuard::enter(server.connection.clone()).is_ok());
    }

    #[test]
    fn initialized_connection_lease_disconnects_on_drop() {
        let (_dir, server) = server();
        let id = server.connection.id;
        server
            .sessions
            .connect_client(id, "client".into(), "1".into());
        server.connection.initialized.store(true, Ordering::Release);
        let sessions = server.sessions.clone();
        drop(server);
        assert2::assert!(sessions.connections().is_empty());
    }

    #[tokio::test]
    async fn graph_state_list_export_and_clear_tools_have_observable_results() {
        let (_dir, server) = server();
        let proposed = server
            .propose_graph(Parameters(ProposeGraphParams {
                title: "Original".into(),
                nodes: vec![node("a")],
                edges: vec![],
                focus: Some(NodeId::from("a")),
                announce: Some("hello".into()),
                session: None,
                agent: Some("agent-a".into()),
            }))
            .await
            .unwrap();
        assert2::assert!(result_text(&proposed).contains("node_count"));
        assert2::assert!((server.sessions.active_store().snapshot_doc().title) == ("Original"));

        let updated = server
            .update_graph(Parameters(UpdateGraphParams {
                ops: vec![GraphOp::SetTitle {
                    title: "Updated".into(),
                }],
                session: None,
                agent: None,
            }))
            .await
            .unwrap();
        assert2::assert!(result_text(&updated).contains("revision"));
        assert2::assert!((server.sessions.active_store().snapshot_doc().title) == ("Updated"));

        let store = server.sessions.active_store();
        let idle_state = server
            .get_state(Parameters(SessionOnlyParams { session: None }))
            .await
            .unwrap();
        assert2::assert!(
            idle_state.content[0]
                .as_text()
                .unwrap()
                .text
                .contains("\"agent_waiting\":false")
        );
        let waiter_store = store.clone();
        let waiter = tokio::spawn(async move {
            waiter_store
                .await_flush(
                    Duration::from_secs(60),
                    Awaiter {
                        connection_id: ConnectionId(900),
                        client_label: "test client".into(),
                        agent: None,
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert2::assert!((store.snapshot_meta().waiting_agents) == (1));
        let state = server
            .get_state(Parameters(SessionOnlyParams { session: None }))
            .await
            .unwrap();
        let state_text = result_text(&state);
        assert2::assert!(state_text.contains("Updated"));
        assert2::assert!(
            state.content[0]
                .as_text()
                .unwrap()
                .text
                .contains("\"agent_waiting\":true")
        );
        waiter.abort();

        let listed = server
            .list_sessions(Parameters(EmptyParams {}))
            .await
            .unwrap();
        assert2::assert!(result_text(&listed).contains("default"));

        let exported = server
            .export_markdown(Parameters(ExportParams {
                format: ExportFormat::Markdown,
                session: None,
            }))
            .await
            .unwrap();
        assert2::assert!(result_text(&exported).contains("Updated"));

        let cleared = server
            .clear_session(Parameters(SessionOnlyParams { session: None }))
            .await
            .unwrap();
        assert2::assert!(result_text(&cleared).contains("node_count"));
        assert2::assert!(
            server
                .sessions
                .active_store()
                .snapshot_doc()
                .nodes
                .is_empty()
        );
    }

    #[tokio::test]
    async fn diff_tools_return_content_and_report_bad_records() {
        let (_dir, server) = server();
        server.sessions.create("other").unwrap();
        server
            .sessions
            .active_store()
            .apply_update(vec![GraphOp::SetTitle { title: "A".into() }])
            .unwrap();
        server
            .sessions
            .get("other")
            .unwrap()
            .apply_update(vec![GraphOp::SetTitle { title: "B".into() }])
            .unwrap();

        let diff = server
            .diff_sessions(Parameters(DiffParams {
                a: "default".into(),
                b: "other".into(),
            }))
            .await
            .unwrap();
        assert2::assert!(!diff.content.is_empty());
        assert2::assert!(result_text(&diff).contains("default"));
        assert2::assert!(
            server
                .diff_record(Parameters(DiffRecordParams {
                    path: "/definitely/missing/nodestorm-record.md".into(),
                    session: None,
                }))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn await_decisions_is_exercised_through_the_tool_router() {
        let (_dir, server) = server();
        let store = server.sessions.active_store();
        let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
        let server_task = tokio::spawn(async move {
            server
                .serve(server_transport)
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let client = ().serve(client_transport).await.unwrap();

        let call = tokio::spawn(async move {
            client
                .call_tool(
                    CallToolRequestParams::new("await_decisions").with_arguments(
                        serde_json::json!({"timeout_seconds": 5})
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                )
                .await
        });
        while store.snapshot_meta().waiting_agents != 1 {
            tokio::task::yield_now().await;
        }
        store.request_flush(None).unwrap();
        let result = call.await.unwrap().unwrap();

        let text = result_text(&result);
        assert2::assert!(text.contains("delivered"));
        assert2::assert!(text.contains("decisions"));
        server_task.abort();
    }

    #[test]
    fn server_info_identifies_nodestorm_and_enables_tools() {
        let (_dir, server) = server();
        let info = server.get_info();
        assert2::assert!((info.server_info.name) == ("nodestorm"));
        assert2::assert!((info.server_info.version) == (env!("CARGO_PKG_VERSION")));
        assert2::assert!(info.capabilities.tools.is_some());
        assert2::assert!(info.instructions.unwrap().contains("await_decisions"));
    }
}
