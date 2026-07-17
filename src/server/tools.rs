//! The six nodestorm MCP tools.
//!
//! All rmcp contact stays in this module (and `server::mod`): the store knows
//! nothing about MCP, which keeps it unit-testable and shields the rest of
//! the crate from SDK churn.

use std::sync::Arc;
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
use crate::store::{FlushOutcome, Store, StoreError, UpdateSummary};

/// Progress heartbeat cadence while an `await_decisions` call blocks. Keeps
/// Claude Code's HTTP idle-abort (default 5 min) from killing the call.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(25);
/// Default and ceiling for `await_decisions.timeout_seconds`. The default
/// stays under Claude Code's 5-minute idle abort so the call returns cleanly
/// even when no progress token was sent; the skill tells agents to re-call.
const DEFAULT_AWAIT_SECS: u64 = 240;
const MAX_AWAIT_SECS: u64 = 3600;

#[derive(Clone)]
pub struct NodestormServer {
    sessions: Arc<crate::sessions::Sessions>,
}

impl NodestormServer {
    pub fn new(sessions: Arc<crate::sessions::Sessions>) -> Self {
        Self { sessions }
    }

    /// Route a tool call to its session's store: `None` → the active
    /// session; unknown names error listing what exists.
    fn store_for(&self, session: &Option<String>) -> Result<Arc<Store>, ErrorData> {
        self.sessions
            .resolve(session.as_deref())
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
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateGraphParams {
    /// Ops apply in order, atomically: if any fails validation nothing commits.
    pub ops: Vec<GraphOp>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AwaitDecisionsParams {
    /// How long to block waiting for the user (seconds). On `"timeout"`,
    /// simply call this tool again — decisions are never lost.
    #[serde(default = "default_await_secs")]
    pub timeout_seconds: u64,
}

fn default_await_secs() -> u64 {
    DEFAULT_AWAIT_SECS
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportParams {
    /// What to return: the full Markdown decision record (default), or just
    /// the Mermaid `flowchart` block body.
    #[serde(default)]
    pub format: ExportFormat,
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
    revision: u64,
    node_count: usize,
    open_choice_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

impl From<UpdateSummary> for GraphSummary {
    fn from(s: UpdateSummary) -> Self {
        Self {
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
        };
        let summary = self
            .store_for(&None)?
            .apply_propose(doc)
            .map_err(store_err)?;
        if let Some(msg) = p.announce {
            self.store_for(&None)?.announce(msg);
        }
        json_result(GraphSummary::from(summary))
    }

    #[tool(
        description = "Patch the current graph with an ordered list of ops (applied atomically): \
                       upsert_node, remove_node, upsert_edge, remove_edge, add_choice, \
                       resolve_choice, set_status, set_focus, set_title, announce. After the user \
                       decides something, use this to apply the ripple: mark impacted nodes \
                       status=affected, open follow-up choices on them, and announce a summary."
    )]
    async fn update_graph(
        &self,
        Parameters(p): Parameters<UpdateGraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let summary = self
            .store_for(&None)?
            .apply_update(p.ops)
            .map_err(store_err)?;
        json_result(GraphSummary::from(summary))
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
    ) -> Result<CallToolResult, ErrorData> {
        let timeout = Duration::from_secs(p.timeout_seconds.clamp(5, MAX_AWAIT_SECS));

        // Best-effort heartbeat so long waits survive client idle-abort.
        let cancel = CancellationToken::new();
        if let Some(token) = meta.get_progress_token() {
            let store = self.store_for(&None)?.clone();
            let cancel = cancel.clone();
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
                        () = cancel.cancelled() => break,
                        () = tokio::time::sleep(HEARTBEAT_EVERY) => {
                            elapsed += HEARTBEAT_EVERY.as_secs();
                        }
                    }
                }
            });
        }

        let outcome = self.store_for(&None)?.await_flush(timeout).await;
        cancel.cancel();

        let (open, revision) = self
            .store_for(&None)?
            .read(|s| (s.doc.open_choice_count(), s.doc.revision));
        match outcome {
            FlushOutcome::Delivered(decisions) => json_result(AwaitResult::Delivered {
                decisions,
                open_choice_count: open,
                revision,
            }),
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
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (doc, undelivered, log_len, waiting) = self.store_for(&None)?.read(|s| {
            (
                s.doc.clone(),
                s.decision_log[s.delivery_cursor..].to_vec(),
                s.decision_log.len(),
                s.waiting_agents > 0,
            )
        });
        json_result(StateResult {
            doc,
            undelivered_decisions: undelivered,
            decision_log_len: log_len,
            agent_waiting: waiting,
        })
    }

    #[tool(description = "Wipe the canvas and decision log to start a fresh brainstorm.")]
    async fn clear_session(
        &self,
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let summary = self.store_for(&None)?.clear_session();
        json_result(GraphSummary::from(summary))
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
        let text = self.store_for(&None)?.read(|s| match p.format {
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
             export_markdown and save the decision record into the user's repo."
                .into(),
        );
        info
    }
}
