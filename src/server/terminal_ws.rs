//! Token-gated WebSocket bridge between a [`TerminalManager`] PTY and the
//! Ferroterm instance in the webview.
//!
//! Protocol: server→client Binary frames are raw PTY output (the first frame
//! replays scrollback); client→server Binary frames are raw input bytes and
//! Text frames carry `{"resize":{"cols":N,"rows":N}}`.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::terminal::{Attached, TerminalManager};

#[derive(serde::Deserialize)]
struct TokenQuery {
    #[serde(default)]
    token: String,
}

#[derive(serde::Deserialize)]
struct ControlFrame {
    resize: ResizeFrame,
}

#[derive(serde::Deserialize)]
struct ResizeFrame {
    cols: u16,
    rows: u16,
}

fn parse_resize(text: &str) -> Option<(u16, u16)> {
    let frame: ControlFrame = serde_json::from_str(text).ok()?;
    Some((frame.resize.cols, frame.resize.rows))
}

pub(super) fn routes(manager: Arc<TerminalManager>) -> axum::Router {
    axum::Router::new()
        .route("/terminal/{id}/ws", get(terminal_ws))
        .with_state(manager)
}

async fn terminal_ws(
    Path(id): Path<String>,
    Query(query): Query<TokenQuery>,
    State(manager): State<Arc<TerminalManager>>,
    ws: WebSocketUpgrade,
) -> Response {
    if query.token != manager.token() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(attached) = manager.attach(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ws.on_upgrade(move |socket| pump(socket, manager, id, attached))
}

async fn pump(
    mut socket: WebSocket,
    manager: Arc<TerminalManager>,
    id: String,
    attached: Attached,
) {
    let Attached { replay, mut output } = attached;
    if socket.send(Message::Binary(replay.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                Ok(bytes) => {
                    if socket.send(Message::Binary(bytes.into())).await.is_err() {
                        return;
                    }
                }
                // ponytail: lagged frames are dropped; the 1 MiB ring plus a
                // 1024-chunk channel makes this a reconnect-and-replay rarity.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if manager.write(&id, &bytes).is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Some((cols, rows)) = parse_resize(&text) {
                        let _ = manager.resize(&id, cols, rows);
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_launcher::CommandSpec;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite;
    use yare::parameterized;

    #[parameterized(
        valid = { r#"{"resize":{"cols":120,"rows":40}}"#, Some((120, 40)) },
        not_json = { "resize 120 40", None },
        missing_rows = { r#"{"resize":{"cols":120}}"#, None },
        wrong_shape = { r#"{"cols":120,"rows":40}"#, None },
    )]
    fn resize_frames_parse_strictly(text: &str, expected: Option<(u16, u16)>) {
        assert2::assert!(parse_resize(text) == expected);
    }

    async fn serve_terminal_routes(manager: Arc<TerminalManager>) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, routes(manager)).into_future());
        format!("ws://{addr}")
    }

    #[tokio::test]
    async fn websocket_replays_and_streams_pty_output() {
        let manager = TerminalManager::new();
        let (program, flag) = if cfg!(windows) {
            ("cmd", "/c")
        } else {
            ("sh", "-c")
        };
        manager
            .spawn(
                "ws-echo",
                &CommandSpec {
                    program: program.into(),
                    args: vec![flag.into(), "echo ws-pty-hello".into()],
                    current_dir: None,
                },
            )
            .unwrap();
        let base = serve_terminal_routes(manager.clone()).await;
        let url = format!("{base}/terminal/ws-echo/ws?token={}", manager.token());
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        while !String::from_utf8_lossy(&seen).contains("ws-pty-hello") {
            let frame = tokio::time::timeout_at(deadline, ws.next())
                .await
                .expect("pty output within 20s")
                .expect("socket open")
                .unwrap();
            if let tungstenite::Message::Binary(bytes) = frame {
                seen.extend_from_slice(&bytes);
            }
        }
        // Resize and input frames are accepted without dropping the socket.
        ws.send(tungstenite::Message::Text(
            r#"{"resize":{"cols":100,"rows":30}}"#.into(),
        ))
        .await
        .unwrap();
        ws.send(tungstenite::Message::Binary(b"\r\n".to_vec().into()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn wrong_token_and_unknown_terminal_are_rejected() {
        let manager = TerminalManager::new();
        let base = serve_terminal_routes(manager.clone()).await;

        let bad_token = format!("{base}/terminal/ws-echo/ws?token=wrong");
        let error = tokio_tungstenite::connect_async(bad_token)
            .await
            .unwrap_err();
        let tungstenite::Error::Http(response) = error else {
            panic!("expected an http rejection, got {error:?}");
        };
        assert2::assert!((response.status()) == (403));

        let unknown = format!("{base}/terminal/missing/ws?token={}", manager.token());
        let error = tokio_tungstenite::connect_async(unknown).await.unwrap_err();
        let tungstenite::Error::Http(response) = error else {
            panic!("expected an http rejection, got {error:?}");
        };
        assert2::assert!((response.status()) == (404));
    }
}
