//! MCP server (streamable HTTP) hosting the nodestorm tools.
//!
//! Runs on its own tokio runtime thread; the UI thread never blocks on it.

pub mod tools;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::{self, Next};
use axum::response::Response;
use rmcp::transport::common::http_header::HEADER_SESSION_ID;
use rmcp::transport::streamable_http_server::session::store::{
    SessionState as TransportSessionState, SessionStore, SessionStoreError,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::net::TcpListener;
use tokio::sync::{RwLock, watch};
use tokio_util::sync::CancellationToken;

use crate::sessions::Sessions;
use crate::store::ConnectionId;

#[derive(Default)]
struct SessionRestoreStore(RwLock<BTreeMap<String, TransportSessionState>>);

#[async_trait::async_trait]
impl SessionStore for SessionRestoreStore {
    async fn load(
        &self,
        session_id: &str,
    ) -> Result<Option<TransportSessionState>, SessionStoreError> {
        Ok(self.0.read().await.get(session_id).cloned())
    }

    async fn store(
        &self,
        session_id: &str,
        state: &TransportSessionState,
    ) -> Result<(), SessionStoreError> {
        self.0
            .write()
            .await
            .insert(session_id.to_owned(), state.clone());
        Ok(())
    }

    async fn delete(&self, session_id: &str) -> Result<(), SessionStoreError> {
        self.0.write().await.remove(session_id);
        Ok(())
    }
}

struct TransportConnection {
    id: ConnectionId,
    cancel: CancellationToken,
}

pub(super) struct TransportConnections {
    sessions: Arc<Sessions>,
    connections: Mutex<BTreeMap<String, TransportConnection>>,
}

impl TransportConnections {
    fn new(sessions: Arc<Sessions>) -> Self {
        Self {
            sessions,
            connections: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn connect(&self, session_id: String, id: ConnectionId, cancel: CancellationToken) {
        self.connections
            .lock()
            .expect("transport connections mutex poisoned")
            .insert(session_id, TransportConnection { id, cancel });
    }

    fn disconnect(&self, session_id: &str) {
        let connection = self
            .connections
            .lock()
            .expect("transport connections mutex poisoned")
            .remove(session_id);
        if let Some(connection) = connection {
            connection.cancel.cancel();
            self.sessions.disconnect_client(connection.id);
        }
    }
}

async fn track_transport_disconnect(
    State(connections): State<Arc<TransportConnections>>,
    request: Request,
    next: Next,
) -> Response {
    let session_id = (request.method() == Method::DELETE)
        .then(|| {
            request
                .headers()
                .get(HEADER_SESSION_ID)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        })
        .flatten();
    let response = next.run(request).await;
    if response.status().is_success()
        && let Some(session_id) = session_id
    {
        connections.disconnect(&session_id);
    }
    response
}

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
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    serve_with_manager(
        listener,
        sessions,
        shutdown,
        Arc::new(LocalSessionManager::default()),
    )
    .await
}

pub async fn serve_with_manager(
    listener: TcpListener,
    sessions: Arc<Sessions>,
    mut shutdown: watch::Receiver<bool>,
    manager: Arc<LocalSessionManager>,
) -> anyhow::Result<()> {
    let transport_connections = Arc::new(TransportConnections::new(sessions.clone()));
    let mut config = StreamableHttpServerConfig::default();
    config.session_store = Some(Arc::new(SessionRestoreStore::default()));
    let service: StreamableHttpService<tools::NodestormServer, LocalSessionManager> =
        StreamableHttpService::new(
            {
                let transport_connections = transport_connections.clone();
                move || {
                    Ok(tools::NodestormServer::new(
                        sessions.clone(),
                        transport_connections.clone(),
                    ))
                }
            },
            manager,
            config,
        );
    let router =
        axum::Router::new()
            .nest_service("/mcp", service)
            .layer(middleware::from_fn_with_state(
                transport_connections,
                track_transport_disconnect,
            ));
    let addr = listener.local_addr()?;
    tracing::info!("MCP server ready at http://{addr}/mcp");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|stop| *stop).await;
        })
        .await
        .context("mcp server crashed")
}
