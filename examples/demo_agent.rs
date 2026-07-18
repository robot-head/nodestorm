//! Scripted agent for the README demo recording (driven by
//! scripts/record-demo.ps1): proposes a realtime-notes architecture with a
//! `sync` group and two rippling choices, reacts once to the first
//! delivered decisions, then keeps awaiting until the recorder kills it.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:4801/mcp".to_owned());
    eprintln!("demo_agent connecting to {url}…");
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url),
    );
    let client = ClientInfo::default().serve(transport).await?;

    let graph = json!({
        "title": "Realtime collaboration for the notes app",
        "announce": "Proposed a realtime sync design — two decisions need you.",
        "focus": "sync-engine",
        "nodes": [
            {"id": "web", "label": "Web Client", "kind": "ui", "status": "existing",
             "description": "React SPA for editing notes"},
            {"id": "api", "label": "Notes API", "kind": "service", "status": "existing",
             "description": "CRUD for notes, folders, and sharing"},
            {"id": "auth", "label": "Auth Service", "kind": "service", "status": "existing",
             "description": "Sessions, tokens, permissions"},
            {"id": "sync-engine", "label": "Sync Engine", "kind": "component", "status": "proposed",
             "group": "sync",
             "description": "Merges concurrent edits from multiple clients",
             "choices": [{
                "id": "conflict-strategy",
                "prompt": "Conflict resolution strategy",
                "rationale": "Concurrent edits must merge without losing keystrokes; the strategy shapes storage and the client protocol.",
                "options": [
                    {"id": "crdt", "label": "CRDT document model",
                     "summary": "Notes become CRDTs; merges are automatic and offline-friendly.",
                     "pros": ["No central lock", "Offline edits merge cleanly"],
                     "cons": ["Document format migration", "Larger stored docs"],
                     "recommended": true,
                     "affects": ["storage", "presence"]},
                    {"id": "ot", "label": "Operational transforms",
                     "summary": "Server-ordered transforms, Google-Docs style.",
                     "pros": ["Compact history", "Well-trodden path"],
                     "cons": ["Server is a serialization bottleneck", "Tricky edge cases"],
                     "affects": ["ws"]}
                ]
             }]},
            {"id": "presence", "label": "Presence Service", "kind": "service", "status": "proposed",
             "group": "sync",
             "description": "Who is online, cursors, and typing indicators"},
            {"id": "ws", "label": "WebSocket Gateway", "kind": "service", "status": "proposed",
             "group": "sync",
             "description": "Long-lived connections pushing edits to clients"},
            {"id": "storage", "label": "Notes Store", "kind": "data_store", "status": "existing",
             "description": "Primary storage for notes and users",
             "choices": [{
                "id": "history-storage",
                "prompt": "Edit history storage",
                "options": [
                    {"id": "event-log", "label": "Append-only event log",
                     "summary": "Every edit is an event; state is a fold.",
                     "pros": ["Perfect audit trail", "Time travel for free"],
                     "cons": ["Compaction needed", "Bigger storage bill"],
                     "recommended": true,
                     "affects": ["sync-engine", "search"]},
                    {"id": "snapshots", "label": "Periodic snapshots",
                     "summary": "Store full documents every N edits.",
                     "pros": ["Simple reads", "Small working set"],
                     "cons": ["History granularity lost"]}
                ]
             }]},
            {"id": "search", "label": "Search Index", "kind": "component", "status": "existing",
             "description": "Full-text search over notes"},
            {"id": "jobs", "label": "Job Queue", "kind": "queue", "status": "existing",
             "description": "Background work: indexing, emails"},
            {"id": "mail", "label": "Email Provider", "kind": "external", "status": "existing",
             "description": "Transactional mail (share invites)"}
        ],
        "edges": [
            {"from": "web", "to": "api", "kind": "data_flow", "status": "existing"},
            {"from": "web", "to": "ws", "kind": "data_flow", "status": "proposed"},
            {"from": "ws", "to": "sync-engine", "kind": "data_flow", "status": "proposed"},
            {"from": "sync-engine", "to": "storage", "kind": "depends_on", "status": "proposed"},
            {"from": "presence", "to": "ws", "kind": "data_flow", "status": "proposed"},
            {"from": "api", "to": "auth", "kind": "depends_on", "status": "existing"},
            {"from": "api", "to": "storage", "kind": "depends_on", "status": "existing"},
            {"from": "storage", "to": "search", "kind": "data_flow", "status": "existing"},
            {"from": "api", "to": "jobs", "kind": "data_flow", "status": "existing"},
            {"from": "jobs", "to": "mail", "kind": "data_flow", "status": "existing"}
        ]
    });
    client
        .call_tool(
            CallToolRequestParams::new("propose_graph")
                .with_arguments(graph.as_object().cloned().unwrap()),
        )
        .await?;
    eprintln!("graph proposed; awaiting decisions…");

    let mut reacted = false;
    loop {
        let result = client
            .call_tool(
                CallToolRequestParams::new("await_decisions").with_arguments(
                    json!({"timeout_seconds": 240})
                        .as_object()
                        .cloned()
                        .unwrap(),
                ),
            )
            .await?;
        let text = result.content[0]
            .as_text()
            .map(|t| t.text.clone())
            .unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text)?;
        if v["status"] == "delivered" && !reacted {
            reacted = true;
            client
                .call_tool(
                    CallToolRequestParams::new("update_graph").with_arguments(
                        json!({
                            "announce": "Applied your decisions — CRDTs it is; storage feels it.",
                            "ops": [
                                {"op": "set_status", "id": "sync-engine", "status": "modified"},
                                {"op": "set_status", "id": "storage", "status": "affected"}
                            ]
                        })
                        .as_object()
                        .cloned()
                        .unwrap(),
                    ),
                )
                .await?;
            eprintln!("reacted to delivery; continuing to await…");
        }
    }
}
