//! End-to-end MCP integration: a real rmcp client talks to the real
//! streamable-HTTP server over loopback, with a simulated user clicking in
//! the store.

use std::sync::Arc;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

use nodestorm::model::{ChoiceId, NodeId, OptionId};
use nodestorm::store::{SessionState, Store};

async fn start_server(store: Arc<Store>) -> (u16, tokio::sync::watch::Sender<bool>) {
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        let _ = nodestorm::server::serve(listener, store, shutdown_rx).await;
    });
    (port, shutdown_tx)
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
async fn full_decision_roundtrip() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown) = start_server(store.clone()).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    let client = ClientInfo::default()
        .serve(transport)
        .await
        .expect("mcp handshake");

    // Tool discovery: all six tools are advertised.
    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in [
        "propose_graph",
        "update_graph",
        "await_decisions",
        "get_state",
        "clear_session",
        "export_markdown",
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

    // Simulated user: after 300ms, pick Redis (autoflush fires — last open
    // choice decided).
    tokio::spawn({
        let store = store.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
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
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["kind"], "option_selected");
    assert_eq!(decisions[0]["option_id"], "redis");
    assert_eq!(
        decisions[0]["considered"],
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
    assert_eq!(state["decision_log_len"], 1);

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

    client.cancel().await.expect("client shutdown");
}

#[tokio::test]
async fn await_decisions_times_out_without_losing_anything() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown) = start_server(store.clone()).await;

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

    // The user clicks Send afterwards: the next call gets it instantly.
    store.request_flush(None);
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

#[tokio::test]
async fn invalid_propose_returns_actionable_error() {
    let store = Store::new(SessionState::default());
    let (port, _shutdown) = start_server(store.clone()).await;

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
