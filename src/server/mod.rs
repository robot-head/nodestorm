//! MCP server (streamable HTTP) hosting the nodestorm tools.
//!
//! Runs on its own tokio runtime thread; the UI thread never blocks on it.

pub mod tools;

use std::sync::Arc;

use anyhow::Context;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::sessions::Sessions;

/// Bind the MCP port, failing with an actionable message when it is taken.
/// Called from inside the server runtime (`Runtime::block_on`) so the
/// listener belongs to that runtime's reactor.
pub async fn bind(port: u16) -> anyhow::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| {
            format!(
                "cannot listen on 127.0.0.1:{port} — is another nodestorm running? \
                 Pass --port to use a different one"
            )
        })
}

/// Serve MCP on `listener` until `shutdown` flips to `true` (or the sender
/// drops, meaning the UI side is gone).
pub async fn serve(
    listener: TcpListener,
    sessions: Arc<Sessions>,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    // Stateless transport: don't mint or require an `Mcp-Session-Id`. rmcp's
    // default (stateful) keeps per-connection sessions in an in-memory manager
    // that idle-reaps them, so any request on a lapsed session gets a
    // `404 Session not found` the client can't recover from (Claude Code drops
    // the whole server). Our tools carry no transport-session state — they
    // route by the `session` param and persist to disk in `Sessions` — so each
    // request is self-contained. `json_response` stays false so a long
    // `await_decisions` can still stream progress heartbeats over its response.
    let service: StreamableHttpService<tools::NodestormServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(tools::NodestormServer::new(sessions.clone())),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_stateful_mode(false),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let addr = listener.local_addr()?;
    tracing::info!("MCP server ready at http://{addr}/mcp");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|stop| *stop).await;
        })
        .await
        .context("mcp server crashed")
}
