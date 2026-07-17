//! Minimal agent simulator: pushes a graph to a running nodestorm and blocks
//! on await_decisions, printing what comes back. Useful for demos and manual
//! end-to-end verification:
//!
//! ```sh
//! nodestorm &            # window opens
//! cargo run --example drive
//! # …click options and "Send to agent" in the window…
//! ```

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:4747/mcp".to_owned());
    eprintln!("connecting to {url}…");
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url),
    );
    let client = ClientInfo::default().serve(transport).await?;

    let args = json!({
        "title": "Add a webhook subsystem",
        "announce": "Proposed a webhook delivery design — two decisions need you.",
        "focus": "dispatcher",
        "nodes": [
            {"id": "api", "label": "API", "kind": "service", "status": "existing",
             "description": "Existing public REST API"},
            {"id": "events", "label": "Event Bus", "kind": "queue", "status": "existing",
             "description": "Internal domain events"},
            {"id": "dispatcher", "label": "Webhook Dispatcher", "kind": "service", "status": "proposed",
             "description": "Consumes events, delivers HTTP callbacks to subscribers",
             "choices": [{
                "id": "delivery-guarantee",
                "prompt": "What delivery guarantee should webhooks have?",
                "rationale": "Retries and ordering shape the dispatcher's storage needs and the subscriber contract.",
                "options": [
                    {"id": "at-least-once", "label": "At-least-once with retries",
                     "summary": "Persist deliveries, retry with backoff for 24h.",
                     "pros": ["Subscribers eventually get everything", "Industry standard"],
                     "cons": ["Subscribers must dedupe", "Needs a delivery store"],
                     "recommended": true,
                     "affects": ["delivery-store", "api"]},
                    {"id": "best-effort", "label": "Best-effort fire-and-forget",
                     "summary": "One attempt, drop on failure.",
                     "pros": ["No storage needed", "Trivial"],
                     "cons": ["Silent data loss for subscribers"],
                     "affects": ["api"]}
                ]
             }]},
            {"id": "delivery-store", "label": "Delivery Store", "kind": "data_store", "status": "proposed",
             "description": "Pending/failed deliveries with retry state",
             "choices": [{
                "id": "store-tech",
                "prompt": "Where should pending deliveries live?",
                "options": [
                    {"id": "postgres", "label": "Existing PostgreSQL",
                     "summary": "A deliveries table with SKIP LOCKED workers.",
                     "pros": ["No new infra", "Transactional with domain data"],
                     "cons": ["Polling load on the primary"],
                     "recommended": true, "affects": ["dispatcher"]},
                    {"id": "redis-streams", "label": "Redis Streams",
                     "summary": "Consumer groups for delivery workers.",
                     "pros": ["Built for queues", "Low latency"],
                     "cons": ["New operational surface", "Persistence tuning required"],
                     "affects": ["dispatcher"]}
                ]
             }]}
        ],
        "edges": [
            {"from": "api", "to": "events", "kind": "data_flow", "status": "existing"},
            {"from": "events", "to": "dispatcher", "kind": "data_flow", "status": "proposed"},
            {"from": "dispatcher", "to": "delivery-store", "kind": "depends_on", "status": "proposed"}
        ]
    });

    let result = client
        .call_tool(
            CallToolRequestParams::new("propose_graph")
                .with_arguments(args.as_object().cloned().unwrap()),
        )
        .await?;
    println!(
        "propose_graph → {}",
        result.content[0]
            .as_text()
            .map(|t| t.text.as_str())
            .unwrap_or("?")
    );

    eprintln!("waiting for your decisions in the nodestorm window…");
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
        if v["status"] == "delivered" {
            println!("delivered → {}", serde_json::to_string_pretty(&v)?);
            break;
        }
        eprintln!("timeout — re-calling await_decisions…");
    }

    client.cancel().await?;
    Ok(())
}
