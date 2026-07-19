//! End-to-end MCP integration: a real rmcp client talks to the real
//! streamable-HTTP server over loopback, with a simulated user clicking in
//! the store.

use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, ClientRequest, Implementation, Request,
};
use rmcp::service::PeerRequestOptions;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

use nodestorm::model::{ChoiceId, NodeId, NodeKind, OptionId, QuestionId};
use nodestorm::sessions::{ConnectionState, Sessions};
use nodestorm::store::{Awaiter, ConnectionId, SendStatus, SessionState, Store};

async fn start_server(store: Arc<Store>) -> (u16, tokio::sync::watch::Sender<bool>, Arc<Sessions>) {
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral");
    let port = listener.local_addr().expect("addr").port();
    let dir = std::env::temp_dir().join(format!("nodestorm-mcp-{}-{port}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("session dir");
    let sessions = Sessions::single(store, dir);
    tokio::spawn({
        let sessions = sessions.clone();
        async move {
            let _ = nodestorm::server::serve(listener, sessions, shutdown_rx).await;
        }
    });
    (port, shutdown_tx, sessions)
}

async fn connect_client(
    port: u16,
    name: &str,
) -> rmcp::service::RunningService<rmcp::RoleClient, ClientInfo> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    ClientInfo::default()
        .serve(transport)
        .await
        .unwrap_or_else(|err| panic!("{name} handshake failed: {err}"))
}

fn tool_json(result: &rmcp::model::CallToolResult) -> Value {
    let text = result.content[0].as_text().expect("text content");
    serde_json::from_str(&text.text).expect("tool result is json")
}

fn propose_args() -> Value {
    json!({
        "title": "test graph",
        "nodes": [
            {"id": "api", "label": "API", "kind": "service", "status": "existing"},
            {
                "id": "cache", "label": "Cache", "kind": "data_store", "status": "proposed",
                "choices": [{
                    "id": "engine",
                    "prompt": "Which cache engine?",
                    "options": [
                        {"id": "redis", "label": "Redis", "recommended": true, "affects": ["api"]},
                        {"id": "memcached", "label": "Memcached"}
                    ]
                }]
            }
        ],
        "edges": [{"from": "api", "to": "cache", "kind": "depends_on"}]
    })
}

#[tokio::test]
async fn connection_lifecycle_is_visible() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, sessions) = start_server(store.clone()).await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    let client = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("claude-code", "1.2.3"),
    )
    .serve(transport)
    .await
    .expect("mcp handshake");

    let connections = sessions.connections();
    assert_eq!(connections.len(), 1);
    assert_eq!(connections[0].client_name, "claude-code");
    assert_eq!(connections[0].version, "1.2.3");
    assert_eq!(connections[0].state, ConnectionState::Connected);

    let awaiting = client
        .send_cancellable_request(
            ClientRequest::CallToolRequest(Request::new(
                CallToolRequestParams::new("await_decisions").with_arguments(
                    json!({"timeout_seconds": 30, "agent": "alpha"})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )),
            PeerRequestOptions::no_options(),
        )
        .await
        .expect("start await_decisions");
    for _ in 0..100 {
        if matches!(
            sessions.connections().first().map(|info| &info.state),
            Some(ConnectionState::Waiting { session, agent })
                if session == "default" && agent.as_deref() == Some("alpha")
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(matches!(
        sessions.connections().first().map(|info| &info.state),
        Some(ConnectionState::Waiting { session, agent })
            if session == "default" && agent.as_deref() == Some("alpha")
    ));

    awaiting
        .cancel(Some("test disconnect".into()))
        .await
        .expect("cancel await_decisions");
    client.cancel().await.expect("client shutdown");
    for _ in 0..100 {
        if sessions.connections().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        sessions.connections().is_empty(),
        "remaining connections: {:?}",
        sessions.connections()
    );
    assert_eq!(store.snapshot_meta().waiting_agents, 0);
}

#[tokio::test]
async fn transport_shutdown_releases_waiter_for_reconnect() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, sessions) = start_server(store.clone()).await;
    let uri = format!("http://127.0.0.1:{port}/mcp");
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri.clone()),
    );
    let client = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("claude-code", "1.2.3"),
    )
    .serve(transport)
    .await
    .expect("first handshake");
    let awaiting = client
        .send_cancellable_request(
            ClientRequest::CallToolRequest(Request::new(
                CallToolRequestParams::new("await_decisions").with_arguments(
                    json!({"timeout_seconds": 30, "agent": "alpha"})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )),
            PeerRequestOptions::no_options(),
        )
        .await
        .expect("start first await_decisions");
    let mut changes = sessions.subscribe_connections();
    tokio::time::timeout(Duration::from_secs(1), async {
        while store.snapshot_meta().waiting_agents != 1 {
            changes.changed().await.expect("connection watch open");
        }
    })
    .await
    .expect("first client becomes waiting");

    client.cancel().await.expect("transport shutdown");
    drop(awaiting);
    tokio::time::timeout(Duration::from_secs(1), async {
        while store.snapshot_meta().waiting_agents != 0 || !sessions.connections().is_empty() {
            changes.changed().await.expect("connection watch open");
        }
    })
    .await
    .expect("transport shutdown releases waiter and connection");

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri),
    );
    let reconnected = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("claude-code", "1.2.3"),
    )
    .serve(transport)
    .await
    .expect("reconnect handshake");
    let peer = reconnected.peer().clone();
    let delivery = tokio::spawn(async move {
        peer.call_tool(
            CallToolRequestParams::new("await_decisions").with_arguments(
                json!({"timeout_seconds": 10, "agent": "alpha"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while store.snapshot_meta().waiting_agents != 1 {
            changes.changed().await.expect("connection watch open");
        }
    })
    .await
    .expect("reconnected client becomes waiting");
    store
        .request_flush(None)
        .expect("send to reconnected client");
    let result = delivery
        .await
        .expect("delivery task")
        .expect("await_decisions response");
    assert_eq!(tool_json(&result)["status"], "delivered");
    reconnected.cancel().await.expect("reconnected shutdown");
}

#[tokio::test]
async fn full_decision_roundtrip() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, _sessions) = start_server(store.clone()).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    let client = ClientInfo::default()
        .serve(transport)
        .await
        .expect("mcp handshake");

    // Tool discovery: every tool is advertised.
    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in [
        "propose_graph",
        "update_graph",
        "await_decisions",
        "get_state",
        "clear_session",
        "export_markdown",
        "list_sessions",
        "diff_sessions",
        "diff_record",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}: {names:?}"
        );
    }

    // propose_graph
    let result = client
        .call_tool(
            CallToolRequestParams::new("propose_graph")
                .with_arguments(propose_args().as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("propose_graph");
    let summary = tool_json(&result);
    assert_eq!(summary["node_count"], 2);
    assert_eq!(summary["open_choice_count"], 1);

    // Simulated user: after 300ms, add a component, ask to remove an
    // agent-authored one, then pick Redis. The edits come first — deciding
    // the last open choice autoflushes, delivering all three events at once.
    tokio::spawn({
        let store = store.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            store
                .add_user_node("Rate Limiter".into(), NodeKind::Component, None)
                .expect("add user node");
            store
                .delete_node(&NodeId::from("api"))
                .expect("soft-remove agent node");
            store
                .select_option(
                    &NodeId::from("cache"),
                    &ChoiceId::from("engine"),
                    &OptionId::from("redis"),
                    vec![OptionId::from("memcached"), OptionId::from("redis")],
                )
                .expect("select");
        }
    });

    // await_decisions blocks until the simulated click, then delivers.
    let result = client
        .call_tool(
            CallToolRequestParams::new("await_decisions").with_arguments(
                json!({"timeout_seconds": 10})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("await_decisions");
    let outcome = tool_json(&result);
    assert_eq!(outcome["status"], "delivered", "{outcome:#}");
    let decisions = outcome["decisions"].as_array().expect("decisions array");
    assert_eq!(decisions.len(), 3, "edits ride along: {decisions:#?}");
    assert_eq!(decisions[0]["kind"], "node_added");
    assert_eq!(decisions[0]["node"]["id"], "rate-limiter");
    assert_eq!(decisions[0]["node"]["label"], "Rate Limiter");
    assert_eq!(decisions[1]["kind"], "removal_requested");
    assert_eq!(decisions[1]["node_id"], "api");
    assert_eq!(decisions[2]["kind"], "option_selected");
    assert_eq!(decisions[2]["option_id"], "redis");
    assert_eq!(
        decisions[2]["considered"],
        json!(["memcached", "redis"]),
        "exploration trail rides along"
    );
    assert_eq!(outcome["open_choice_count"], 0);

    // Ripple: agent marks api affected and opens a follow-up choice there.
    let result = client
        .call_tool(CallToolRequestParams::new("update_graph").with_arguments(json!({
                "ops": [
                    {"op": "set_status", "id": "api", "status": "affected"},
                    {"op": "add_choice", "node_id": "api", "choice": {
                        "id": "invalidation",
                        "prompt": "Cache invalidation strategy?",
                        "options": [
                            {"id": "ttl", "label": "TTL only", "recommended": true},
                            {"id": "events", "label": "Event-driven"}
                        ]
                    }},
                    {"op": "announce", "message": "Applied Redis decision; cache invalidation is next."}
                ]
            })
            .as_object().cloned().unwrap_or_default()))
        .await
        .expect("update_graph");
    let summary = tool_json(&result);
    assert_eq!(summary["open_choice_count"], 1);

    // get_state reflects everything.
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_state")
                .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("get_state");
    let state = tool_json(&result);
    assert_eq!(state["doc"]["nodes"][0]["status"], "affected");
    assert_eq!(state["undelivered_decisions"].as_array().unwrap().len(), 0);
    assert_eq!(state["decision_log_len"], 3);
    assert_eq!(
        state["doc"]["nodes"][2]["origin"], "user",
        "user node visible in state: {:#}",
        state["doc"]["nodes"]
    );

    // export_markdown returns the decision record as plain Markdown (not
    // JSON): the Redis decision with its exploration trail, and the follow-up
    // choice under Open questions.
    let result = client
        .call_tool(
            CallToolRequestParams::new("export_markdown")
                .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("export_markdown");
    let record = &result.content[0]
        .as_text()
        .expect("plain text content")
        .text;
    assert!(record.starts_with("# test graph\n"), "got: {record}");
    assert!(
        record.contains("```mermaid\nflowchart LR\n"),
        "in: {record}"
    );
    assert!(
        record.contains("**Decision: Redis ★ agent-recommended**"),
        "in: {record}"
    );
    assert!(
        record.contains("after first exploring Memcached"),
        "trail, in: {record}"
    );
    let decisions_at = record.find("## Decisions").expect("decisions section");
    let open_at = record.find("## Open questions").expect("open section");
    assert!(decisions_at < open_at, "section order, in: {record}");
    assert!(
        record[open_at..].contains("Cache invalidation strategy?"),
        "in: {record}"
    );
    assert!(
        record.contains("Rate Limiter"),
        "user node in the record, in: {record}"
    );

    // format: "mermaid" returns just the diagram block body.
    let result = client
        .call_tool(
            CallToolRequestParams::new("export_markdown").with_arguments(
                json!({"format": "mermaid"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("export_markdown mermaid");
    let mermaid = &result.content[0].as_text().expect("text").text;
    assert!(mermaid.starts_with("flowchart LR\n"), "got: {mermaid}");
    assert!(!mermaid.contains("# "), "no markdown headings: {mermaid}");

    client.cancel().await.expect("client shutdown");
}

#[tokio::test]
async fn agent_question_answered_roundtrip() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, _sessions) = start_server(store.clone()).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    let client = ClientInfo::default()
        .serve(transport)
        .await
        .expect("mcp handshake");

    // propose a one-node graph, then attach a free-form question to it.
    client
        .call_tool(
            CallToolRequestParams::new("propose_graph").with_arguments(
                json!({
                    "title": "q graph",
                    "nodes": [{"id": "api", "label": "API", "kind": "service"}]
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            ),
        )
        .await
        .expect("propose_graph");
    client
        .call_tool(
            CallToolRequestParams::new("update_graph").with_arguments(
                json!({
                    "ops": [{"op": "ask", "question": {
                        "id": "deploy-target",
                        "prompt": "Which environment ships first?",
                        "node_id": "api"
                    }}]
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            ),
        )
        .await
        .expect("ask");

    // Simulated user answers in text, then clicks Send (answers do not autoflush).
    tokio::spawn({
        let store = store.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            store
                .answer_question(&QuestionId::from("deploy-target"), "staging first".into())
                .expect("answer");
            store.request_flush(None).expect("send answer");
        }
    });

    let result = client
        .call_tool(
            CallToolRequestParams::new("await_decisions").with_arguments(
                json!({"timeout_seconds": 10})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("await_decisions");
    let outcome = tool_json(&result);
    assert_eq!(outcome["status"], "delivered", "{outcome:#}");
    let decisions = outcome["decisions"].as_array().expect("decisions array");
    assert_eq!(decisions[0]["kind"], "question_answered");
    assert_eq!(decisions[0]["question_id"], "deploy-target");
    assert_eq!(decisions[0]["answer"], "staging first");

    // The answer is durable in the doc and exported alongside decisions.
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_state")
                .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("get_state");
    let state = tool_json(&result);
    assert_eq!(state["doc"]["questions"][0]["answer"], "staging first");

    client.cancel().await.expect("client shutdown");
}

#[tokio::test]
async fn multi_agent_awaits_route_per_connection() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, sessions) = start_server(store.clone()).await;
    let alpha_client = connect_client(port, "alpha").await;
    let beta_client = connect_client(port, "beta").await;

    alpha_client
        .call_tool(CallToolRequestParams::new("propose_graph").with_arguments(
            json!({
                "title": "multi", "agent": "alpha",
                "nodes": [{"id": "a", "label": "A", "choices": [
                    {"id": "ca", "prompt": "A?", "options": [{"id": "x", "label": "X"}, {"id": "y", "label": "Y"}]},
                    {"id": "later", "prompt": "Later?", "options": [{"id": "yes", "label": "Yes"}, {"id": "no", "label": "No"}]}
                ]}]
            })
            .as_object().cloned().unwrap_or_default(),
        ))
        .await
        .expect("propose alpha");
    beta_client
        .call_tool(CallToolRequestParams::new("update_graph").with_arguments(
            json!({
                "agent": "beta",
                "ops": [{"op": "upsert_node", "node": {"id": "b", "label": "B", "choices": [
                    {"id": "cb", "prompt": "B?", "options": [{"id": "p", "label": "P"}, {"id": "q", "label": "Q"}]}
                ]}}]
            })
            .as_object().cloned().unwrap_or_default(),
        ))
        .await
        .expect("update beta");

    let alpha_peer = alpha_client.peer().clone();
    let mut connection_changes = sessions.subscribe_connections();
    let alpha_await = tokio::spawn(async move {
        alpha_peer
            .call_tool(
                CallToolRequestParams::new("await_decisions").with_arguments(
                    json!({"timeout_seconds": 10, "agent": "alpha"})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await
    });
    let beta_peer = beta_client.peer().clone();
    let beta_await = tokio::spawn(async move {
        beta_peer
            .call_tool(
                CallToolRequestParams::new("await_decisions").with_arguments(
                    json!({"timeout_seconds": 10, "agent": "beta"})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while store.snapshot_meta().waiting_agents != 2 {
            connection_changes
                .changed()
                .await
                .expect("connection watch open");
        }
    })
    .await
    .expect("both clients become waiting");

    let waiting_connections = sessions.connections();
    assert_eq!(waiting_connections.len(), 2, "{waiting_connections:#?}");
    assert_ne!(waiting_connections[0].id, waiting_connections[1].id);
    assert!(waiting_connections.iter().any(|connection| matches!(
        &connection.state,
        ConnectionState::Waiting { session, agent }
            if session == "default" && agent.as_deref() == Some("alpha")
    )));
    assert!(waiting_connections.iter().any(|connection| matches!(
        &connection.state,
        ConnectionState::Waiting { session, agent }
            if session == "default" && agent.as_deref() == Some("beta")
    )));

    store
        .select_option(
            &NodeId::from("a"),
            &ChoiceId::from("ca"),
            &OptionId::from("x"),
            vec![],
        )
        .expect("select a");
    store
        .select_option(
            &NodeId::from("b"),
            &ChoiceId::from("cb"),
            &OptionId::from("q"),
            vec![],
        )
        .expect("select b");
    store
        .request_flush(Some("ready".into()))
        .expect("send to both clients");

    let alpha_result = alpha_await
        .await
        .expect("alpha delivery task")
        .expect("alpha await");
    let beta_result = beta_await
        .await
        .expect("beta delivery task")
        .expect("beta await");
    let alpha = tool_json(&alpha_result);
    let beta = tool_json(&beta_result);
    assert_eq!(alpha["status"], "delivered", "{alpha:#}");
    let a_dec = alpha["decisions"].as_array().unwrap();
    assert_eq!(
        a_dec.len(),
        2,
        "alpha sees owned plus unclaimed: {a_dec:#?}"
    );
    assert_eq!(a_dec[0]["kind"], "option_selected");
    assert_eq!(a_dec[0]["node_id"], "a");
    assert_eq!(a_dec[1]["kind"], "flush_requested");
    assert_eq!(a_dec[1]["comment"], "ready");

    assert_eq!(beta["status"], "delivered", "{beta:#}");
    let b_dec = beta["decisions"].as_array().unwrap();
    assert_eq!(b_dec.len(), 2, "beta sees owned plus unclaimed: {b_dec:#?}");
    assert_eq!(b_dec[0]["kind"], "option_selected");
    assert_eq!(b_dec[0]["node_id"], "b");
    assert_eq!(b_dec[1]["kind"], "flush_requested");
    assert_eq!(b_dec[1]["comment"], "ready");

    alpha_client.cancel().await.expect("alpha shutdown");
    beta_client.cancel().await.expect("beta shutdown");
}

#[tokio::test]
async fn explicit_send_stays_pending_until_every_target_consumes() {
    let store = Store::new(SessionState::default());
    let alpha = store.await_flush(
        Duration::from_secs(10),
        Awaiter {
            connection_id: ConnectionId(1),
            client_label: "alpha client".into(),
            agent: Some("alpha".into()),
        },
    );
    tokio::pin!(alpha);
    poll_fn(|context| match alpha.as_mut().poll(context) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(outcome) => panic!("alpha await completed before Send: {outcome:?}"),
    })
    .await;
    let beta = store.await_flush(
        Duration::from_secs(10),
        Awaiter {
            connection_id: ConnectionId(2),
            client_label: "beta client".into(),
            agent: Some("beta".into()),
        },
    );
    tokio::pin!(beta);
    poll_fn(|context| match beta.as_mut().poll(context) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(outcome) => panic!("beta await completed before Send: {outcome:?}"),
    })
    .await;
    assert_eq!(store.snapshot_meta().waiting_agents, 2);

    store.request_flush(None).expect("send to both targets");
    alpha.await.expect("alpha consumes receipt");
    assert_eq!(
        store.snapshot_meta().send_status,
        SendStatus::Sending,
        "Send must not report Sent while beta has not consumed"
    );

    beta.await.expect("beta consumes receipt");
    assert_eq!(store.snapshot_meta().send_status, SendStatus::Sent);
}

#[tokio::test]
async fn one_connection_rejects_a_second_concurrent_await() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, sessions) = start_server(store.clone()).await;
    let client = connect_client(port, "single connection").await;
    client
        .call_tool(
            CallToolRequestParams::new("propose_graph").with_arguments(
                json!({
                    "session": "other", "title": "other",
                    "nodes": [{"id": "other", "label": "Other"}]
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            ),
        )
        .await
        .expect("create other session");
    let other = sessions.get("other").expect("other session");

    let first_peer = client.peer().clone();
    let first = tokio::spawn(async move {
        first_peer
            .call_tool(
                CallToolRequestParams::new("await_decisions").with_arguments(
                    json!({"timeout_seconds": 10, "agent": "alpha"})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await
    });
    let mut changes = sessions.subscribe_connections();
    tokio::time::timeout(Duration::from_secs(1), async {
        while store.snapshot_meta().waiting_agents != 1 {
            changes.changed().await.expect("connection watch open");
        }
    })
    .await
    .expect("first await becomes active");

    let error = tokio::time::timeout(
        Duration::from_secs(1),
        client.call_tool(
            CallToolRequestParams::new("await_decisions").with_arguments(
                json!({"timeout_seconds": 10, "session": "other", "agent": "beta"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        ),
    )
    .await
    .expect("second await is rejected immediately")
    .expect_err("one transport cannot own two awaits");

    assert!(
        error.to_string().contains("active await_decisions"),
        "concrete duplicate error: {error}"
    );
    assert_eq!(store.snapshot_meta().waiting_agents, 1);
    assert_eq!(other.snapshot_meta().waiting_agents, 0);
    assert!(matches!(
        sessions.connections().first().map(|connection| &connection.state),
        Some(ConnectionState::Waiting { session, agent })
            if session == "default" && agent.as_deref() == Some("alpha")
    ));

    store
        .request_flush(None)
        .expect("deliver to original await");
    let result = first
        .await
        .expect("first await task")
        .expect("first await remains valid");
    assert_eq!(tool_json(&result)["status"], "delivered");
    client.cancel().await.expect("client shutdown");
}

#[tokio::test]
async fn reestablished_agent_recovers_pending_delivery() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, sessions) = start_server(store).await;
    let first_client = connect_client(port, "initial alpha").await;

    first_client
        .call_tool(CallToolRequestParams::new("propose_graph").with_arguments(
            json!({
                "session": "recovery", "title": "recovery", "agent": "alpha",
                "nodes": [{"id": "a", "label": "A", "choices": [
                    {"id": "ca", "prompt": "A?", "options": [{"id": "x", "label": "X"}, {"id": "y", "label": "Y"}]}
                ]}]
            })
            .as_object().cloned().unwrap_or_default(),
        ))
        .await
        .expect("propose recovery graph");
    let recovery = sessions.get("recovery").expect("recovery session");

    let mut connection_changes = sessions.subscribe_connections();
    let first_peer = first_client.peer().clone();
    let first_await = tokio::spawn(async move {
        first_peer
            .call_tool(
                CallToolRequestParams::new("await_decisions").with_arguments(
                    json!({"timeout_seconds": 30, "session": "recovery", "agent": "alpha"})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while recovery.snapshot_meta().waiting_agents != 1 {
            connection_changes
                .changed()
                .await
                .expect("connection watch open");
        }
    })
    .await
    .expect("initial alpha becomes waiting");

    first_client.cancel().await.expect("initial alpha shutdown");
    let first_error = first_await
        .await
        .expect("initial alpha await task")
        .expect_err("disconnected await must fail");
    let first_error = first_error.to_string();
    assert!(
        first_error.contains("transport")
            || first_error.contains("cancel")
            || first_error.contains("closed"),
        "unexpected disconnect error: {first_error}"
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while recovery.snapshot_meta().waiting_agents != 0 || !sessions.connections().is_empty() {
            connection_changes
                .changed()
                .await
                .expect("connection watch open");
        }
    })
    .await
    .expect("initial alpha disconnects cleanly");

    recovery
        .select_option(
            &NodeId::from("a"),
            &ChoiceId::from("ca"),
            &OptionId::from("x"),
            vec![],
        )
        .expect("select while disconnected");
    assert_eq!(
        recovery.read(|state| state.agent_cursors.get("alpha").copied()),
        None,
        "disconnect must not advance alpha's cursor"
    );

    let reconnected = connect_client(port, "reestablished alpha").await;
    let result = reconnected
        .call_tool(
            CallToolRequestParams::new("await_decisions").with_arguments(
                json!({"timeout_seconds": 10, "session": "recovery", "agent": "alpha"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("reestablished alpha await");
    let outcome = tool_json(&result);
    assert_eq!(outcome["status"], "delivered", "{outcome:#}");
    let decisions = outcome["decisions"].as_array().expect("decisions array");
    assert_eq!(decisions.len(), 1, "original decision delivered once");
    assert_eq!(decisions[0]["kind"], "option_selected");
    assert_eq!(decisions[0]["node_id"], "a");
    assert_eq!(decisions[0]["choice_id"], "ca");
    assert_eq!(decisions[0]["option_id"], "x");
    let delivered_position = recovery.read(|state| {
        let position = (
            state.agent_cursors.get("alpha").copied(),
            state.agent_flush.get("alpha").copied(),
        );
        assert_eq!(position.0, Some(state.decision_log.len()));
        assert_eq!(position.1, Some(state.flush_seq));
        position
    });

    reconnected.cancel().await.expect("reestablished shutdown");

    let fresh_client = connect_client(port, "fresh alpha").await;
    let result = fresh_client
        .call_tool(
            CallToolRequestParams::new("await_decisions").with_arguments(
                json!({"timeout_seconds": 5, "session": "recovery", "agent": "alpha"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("fresh alpha await");
    let outcome = tool_json(&result);
    assert_eq!(outcome["status"], "timeout", "{outcome:#}");
    assert_eq!(outcome["decisions_so_far"], json!([]));
    assert_eq!(
        recovery.read(|state| (
            state.agent_cursors.get("alpha").copied(),
            state.agent_flush.get("alpha").copied(),
        )),
        delivered_position,
        "empty retry must not advance alpha's delivery position"
    );

    fresh_client.cancel().await.expect("fresh alpha shutdown");
}

#[tokio::test]
async fn diff_against_exported_record_file() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, _sessions) = start_server(store.clone()).await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    let client = ClientInfo::default()
        .serve(transport)
        .await
        .expect("mcp handshake");

    client
        .call_tool(CallToolRequestParams::new("propose_graph").with_arguments(
            json!({"title": "rec", "nodes": [{"id": "api", "label": "API", "kind": "service"}]})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ))
        .await
        .expect("propose");

    // Export the record to a temp file.
    let result = client
        .call_tool(
            CallToolRequestParams::new("export_markdown")
                .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("export");
    let record = result.content[0].as_text().expect("text").text.clone();
    let path = std::env::temp_dir().join(format!("nodestorm-rec-{}.md", std::process::id()));
    std::fs::write(&path, &record).expect("write record");

    // Unchanged session vs its own record → no differences.
    let result = client
        .call_tool(
            CallToolRequestParams::new("diff_record").with_arguments(
                json!({"path": path.to_string_lossy()})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("diff_record");
    let diff = &result.content[0].as_text().expect("text").text;
    assert!(diff.contains("_No differences._"), "{diff}");

    // Add a component, then diff again → drift is reported.
    client
        .call_tool(CallToolRequestParams::new("update_graph").with_arguments(
            json!({"ops": [{"op": "upsert_node", "node": {"id": "cache", "label": "Cache", "kind": "data_store"}}]})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ))
        .await
        .expect("update");
    let result = client
        .call_tool(
            CallToolRequestParams::new("diff_record").with_arguments(
                json!({"path": path.to_string_lossy()})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("diff_record 2");
    let diff = &result.content[0].as_text().expect("text").text;
    assert!(diff.contains("+ added: **Cache**"), "{diff}");

    let _ = std::fs::remove_file(&path);
    client.cancel().await.expect("client shutdown");
}

#[tokio::test]
async fn await_decisions_times_out_without_losing_anything() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, _sessions) = start_server(store.clone()).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    let client = ClientInfo::default()
        .serve(transport)
        .await
        .expect("mcp handshake");

    client
        .call_tool(
            CallToolRequestParams::new("propose_graph")
                .with_arguments(propose_args().as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("propose_graph");

    // A second open choice first, so picking the first one cannot autoflush.
    store
        .apply_update(vec![nodestorm::model::GraphOp::AddChoice {
            node_id: NodeId::from("api"),
            choice: serde_json::from_value(json!({
                "id": "second",
                "prompt": "keep one open",
                "options": [{"id": "a", "label": "A"}]
            }))
            .unwrap(),
        }])
        .expect("keep one open choice");
    // User picks but never clicks Send (one choice still open → no autoflush).
    store
        .select_option(
            &NodeId::from("cache"),
            &ChoiceId::from("engine"),
            &OptionId::from("memcached"),
            vec![],
        )
        .expect("select");

    let result = client
        .call_tool(
            CallToolRequestParams::new("await_decisions").with_arguments(
                json!({"timeout_seconds": 5})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("await_decisions");
    let outcome = tool_json(&result);
    assert_eq!(outcome["status"], "timeout", "{outcome:#}");
    assert_eq!(
        outcome["decisions_so_far"].as_array().unwrap().len(),
        1,
        "preview shows the un-sent decision"
    );

    // The user clicks Send after the next call has registered its waiter.
    tokio::spawn({
        let store = store.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            store.request_flush(None).expect("send queued decision");
        }
    });
    let result = client
        .call_tool(
            CallToolRequestParams::new("await_decisions").with_arguments(
                json!({"timeout_seconds": 10})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("await_decisions 2");
    let outcome = tool_json(&result);
    assert_eq!(outcome["status"], "delivered");
    assert_eq!(outcome["decisions"].as_array().unwrap().len(), 1);

    client.cancel().await.expect("client shutdown");
}

/// Regression guard for the stateless-transport fix (`stateful_mode: false`).
/// The server must neither mint nor require an `Mcp-Session-Id`, so a client
/// that never tracks a session — or lost the one it had — can still call tools
/// instead of getting a `404 Session not found`. That 404 is the failure mode
/// that made Claude Code's HTTP MCP connection unrecoverable and forced the
/// manual curl bypass.
#[tokio::test]
async fn stateless_tools_call_without_session_id() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, _sessions) = start_server(store).await;
    let url = format!("http://127.0.0.1:{port}/mcp");
    let http = reqwest::Client::new();
    let accept = "application/json, text/event-stream";

    // Handshake — a stateless server should not demand a prior session.
    let init = http
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", accept)
        .body(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"stateless-probe","version":"1"}}}"#,
        )
        .send()
        .await
        .expect("initialize");
    assert!(
        init.status().is_success(),
        "initialize status {}",
        init.status()
    );
    // Proper client courtesy; a notification needs no session id either.
    let _ = http
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", accept)
        .body(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .send()
        .await;

    // The crux: a tools/call carrying NO `Mcp-Session-Id` must return 200, not
    // 404. Under the old stateful default this 404'd.
    let resp = http
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", accept)
        .body(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#,
        )
        .send()
        .await
        .expect("tools/call");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "tools/call without a session id must not 404 in stateless mode"
    );
    let body = resp.text().await.expect("body");
    // The tool ran and returned a non-error result (envelope is unescaped;
    // the inner payload is a JSON-stringified `sessions` listing).
    assert!(
        body.contains(r#""isError":false"#) && body.contains("sessions"),
        "tool actually ran, got: {body}"
    );
}

#[tokio::test]
async fn invalid_propose_returns_actionable_error() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, _sessions) = start_server(store.clone()).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    let client = ClientInfo::default()
        .serve(transport)
        .await
        .expect("mcp handshake");

    let err = client
        .call_tool(
            CallToolRequestParams::new("propose_graph").with_arguments(
                json!({
                    "title": "bad",
                    "nodes": [{"id": "a", "label": "A"}],
                    "edges": [{"from": "a", "to": "ghost"}]
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            ),
        )
        .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("unknown node `ghost`"),
        "error should name the bad edge target: {msg}"
    );

    client.cancel().await.expect("client shutdown");
}

#[tokio::test]
async fn sessions_route_and_await_concurrently() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown, sessions) = start_server(store.clone()).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    let client = ClientInfo::default()
        .serve(transport)
        .await
        .expect("mcp handshake");

    // propose_graph with a session name auto-creates that session.
    let mut alpha_args = propose_args();
    alpha_args["session"] = json!("alpha");
    let result = client
        .call_tool(
            CallToolRequestParams::new("propose_graph")
                .with_arguments(alpha_args.as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("propose alpha");
    let summary = tool_json(&result);
    assert_eq!(summary["session"], "alpha", "{summary:#}");

    let mut beta_args = propose_args();
    beta_args["title"] = json!("beta graph");
    beta_args["session"] = json!("beta");
    client
        .call_tool(
            CallToolRequestParams::new("propose_graph")
                .with_arguments(beta_args.as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("propose beta");

    // list_sessions sees all three, with per-session summaries.
    let result = client
        .call_tool(
            CallToolRequestParams::new("list_sessions")
                .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
        )
        .await
        .expect("list_sessions");
    let listing = tool_json(&result);
    let names: Vec<&str> = listing["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"default"), "{listing:#}");
    assert!(names.contains(&"alpha"), "{listing:#}");
    assert!(names.contains(&"beta"), "{listing:#}");
    let alpha_info = listing["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "alpha")
        .unwrap();
    assert_eq!(alpha_info["node_count"], 2);
    assert_eq!(listing["active"], "default");

    // Two agents block on two different sessions at once; each delivery
    // stays inside its own session.
    tokio::spawn({
        let sessions = sessions.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            sessions
                .get("alpha")
                .unwrap()
                .select_option(
                    &NodeId::from("cache"),
                    &ChoiceId::from("engine"),
                    &OptionId::from("redis"),
                    vec![OptionId::from("memcached"), OptionId::from("redis")],
                )
                .expect("alpha select");
            tokio::time::sleep(Duration::from_millis(300)).await;
            sessions
                .get("beta")
                .unwrap()
                .select_option(
                    &NodeId::from("cache"),
                    &ChoiceId::from("engine"),
                    &OptionId::from("redis"),
                    vec![OptionId::from("redis")],
                )
                .expect("beta select");
        }
    });

    let beta_client = connect_client(port, "beta session").await;
    let alpha_await = client.call_tool(
        CallToolRequestParams::new("await_decisions").with_arguments(
            json!({"timeout_seconds": 10, "session": "alpha"})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ),
    );
    let beta_await = beta_client.call_tool(
        CallToolRequestParams::new("await_decisions").with_arguments(
            json!({"timeout_seconds": 10, "session": "beta"})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ),
    );
    let (alpha_out, beta_out) = tokio::join!(alpha_await, beta_await);
    let alpha_out = tool_json(&alpha_out.expect("alpha await"));
    let beta_out = tool_json(&beta_out.expect("beta await"));
    assert_eq!(alpha_out["status"], "delivered", "{alpha_out:#}");
    assert_eq!(beta_out["status"], "delivered", "{beta_out:#}");
    let alpha_decisions = alpha_out["decisions"].as_array().unwrap();
    let beta_decisions = beta_out["decisions"].as_array().unwrap();
    assert_eq!(alpha_decisions.len(), 1, "no cross-session leakage");
    assert_eq!(beta_decisions.len(), 1, "no cross-session leakage");
    assert_eq!(
        alpha_decisions[0]["considered"],
        json!(["memcached", "redis"])
    );
    assert_eq!(beta_decisions[0]["considered"], json!(["redis"]));

    // get_state is per-session and names the session it describes.
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_state").with_arguments(
                json!({"session": "beta"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("get_state beta");
    let state = tool_json(&result);
    assert_eq!(state["session"], "beta");
    assert_eq!(state["doc"]["title"], "beta graph");

    // Unknown sessions error and name what exists.
    let err = client
        .call_tool(
            CallToolRequestParams::new("update_graph").with_arguments(
                json!({"ops": [], "session": "ghost"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await;
    let msg = format!("{err:?}");
    assert!(msg.contains("unknown session"), "{msg}");
    assert!(msg.contains("available"), "{msg}");

    // diff_sessions renders the structured comparison as plain Markdown.
    let result = client
        .call_tool(
            CallToolRequestParams::new("diff_sessions").with_arguments(
                json!({"a": "alpha", "b": "beta"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await
        .expect("diff_sessions");
    let text = &result.content[0].as_text().expect("plain text").text;
    assert!(text.starts_with("# Diff: alpha → beta"), "got: {text}");
    // alpha decided redis (delivered earlier); beta decided redis too, so
    // the graphs differ only if titles/decisions drifted — assert the
    // header and that unknown names still error.
    let err = client
        .call_tool(
            CallToolRequestParams::new("diff_sessions").with_arguments(
                json!({"a": "alpha", "b": "ghost"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await;
    let msg = format!("{err:?}");
    assert!(msg.contains("unknown session"), "{msg}");

    client.cancel().await.expect("client shutdown");
    beta_client.cancel().await.expect("beta client shutdown");
}
