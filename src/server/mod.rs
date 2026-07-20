//! MCP server (streamable HTTP) hosting the nodestorm tools.
//!
//! Runs on its own tokio runtime thread; the UI thread never blocks on it.

pub mod tools;

mod terminal_ws;

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
use crate::terminal::TerminalManager;

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
    let method = request.method().clone();
    let session_id = request.headers().get(HEADER_SESSION_ID).cloned();
    let response = next.run(request).await;
    if let Some(session_id) = disconnect_session_id(&method, response.status(), session_id.as_ref())
    {
        connections.disconnect(&session_id);
    }
    response
}

fn disconnect_session_id(
    method: &Method,
    status: axum::http::StatusCode,
    session_id: Option<&axum::http::HeaderValue>,
) -> Option<String> {
    (method == Method::DELETE && status.is_success())
        .then(|| session_id?.to_str().ok().map(str::to_owned))
        .flatten()
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
    terminals: Arc<TerminalManager>,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    serve_with_manager(
        listener,
        sessions,
        terminals,
        shutdown,
        Arc::new(LocalSessionManager::default()),
    )
    .await
}

pub async fn serve_with_manager(
    listener: TcpListener,
    sessions: Arc<Sessions>,
    terminals: Arc<TerminalManager>,
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
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            transport_connections,
            track_transport_disconnect,
        ))
        .merge(terminal_ws::routes(terminals));
    let addr = listener.local_addr()?;
    tracing::info!("MCP server ready at http://{addr}/mcp");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|stop| *stop).await;
        })
        .await
        .context("mcp server crashed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{SessionState, Store};

    fn test_sessions(name: &str) -> Arc<Sessions> {
        Sessions::single(
            Store::new(SessionState::default()),
            std::env::temp_dir().join(format!("nodestorm-server-{name}")),
        )
    }

    #[tokio::test]
    async fn restore_store_round_trips_and_deletes_state() {
        let store = SessionRestoreStore::default();
        let state = TransportSessionState::new(rmcp::model::InitializeRequestParams::new(
            rmcp::model::ClientCapabilities::default(),
            rmcp::model::Implementation::default(),
        ));

        assert2::assert!(store.load("session").await.unwrap().is_none());
        store.store("session", &state).await.unwrap();
        assert2::assert!(
            (store
                .load("session")
                .await
                .unwrap()
                .unwrap()
                .initialize_params)
                == (state.initialize_params)
        );
        store.delete("session").await.unwrap();
        assert2::assert!(store.load("session").await.unwrap().is_none());
    }

    #[test]
    fn transport_connections_register_and_disconnect_the_client() {
        let sessions = test_sessions("connections");
        let id = sessions.next_connection_id();
        sessions.connect_client(id, "client".into(), "1".into());
        let connections = TransportConnections::new(sessions.clone());
        let cancel = CancellationToken::new();

        connections.connect("transport".into(), id, cancel.clone());
        assert2::assert!((connections.connections.lock().unwrap()["transport"].id) == (id));
        connections.disconnect("transport");

        assert2::assert!(cancel.is_cancelled());
        assert2::assert!(connections.connections.lock().unwrap().is_empty());
        assert2::assert!(sessions.connections().is_empty());
    }

    #[test]
    fn only_successful_delete_requests_disconnect_valid_session_ids() {
        use axum::http::{HeaderValue, StatusCode};

        let id = HeaderValue::from_static("transport");
        assert2::assert!(
            (disconnect_session_id(&Method::DELETE, StatusCode::NO_CONTENT, Some(&id)).as_deref())
                == (Some("transport"))
        );
        assert2::assert!(
            (disconnect_session_id(&Method::POST, StatusCode::NO_CONTENT, Some(&id))) == (None)
        );
        assert2::assert!(
            (disconnect_session_id(&Method::DELETE, StatusCode::BAD_REQUEST, Some(&id))) == (None)
        );
        assert2::assert!(
            (disconnect_session_id(&Method::DELETE, StatusCode::NO_CONTENT, None)) == (None)
        );
    }

    #[tokio::test]
    async fn serve_waits_for_shutdown() {
        let root =
            std::env::temp_dir().join(format!("nodestorm-server-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sessions = Sessions::open(root.join("sessions"), None).unwrap();
        let listener = bind(0).await.unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(serve(
            listener,
            sessions,
            crate::terminal::TerminalManager::new(),
            shutdown_rx,
        ));
        tokio::task::yield_now().await;

        assert2::assert!(!task.is_finished());
        shutdown_tx.send(true).unwrap();
        task.await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}
