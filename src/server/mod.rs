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

use crate::store::Store;

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
    store: Arc<Store>,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let service: StreamableHttpService<tools::NodestormServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(tools::NodestormServer::new(store.clone())),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
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
