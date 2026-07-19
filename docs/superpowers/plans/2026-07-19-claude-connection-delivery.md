# Claude Connection Status and Targeted Delivery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show every incoming Claude MCP session, route Send only to the matching waiting session, recover interrupted delivery after reconnect, and display deterministic Send receipts and error toasts.

**Architecture:** Keep queue ownership and exactly-once delivery in `Store`, adding transient connection-scoped waiters and fixed-boundary receipts around the existing persisted cursors. Keep the global MCP connection registry in `Sessions`, with a separate watch channel bridged into Dioxus. `NodestormServer` owns a clone-safe connection lease; the UI renders registry snapshots plus recoverable receipt targets.

**Tech Stack:** Rust 2024, Tokio watch channels, rmcp 2.2 streamable HTTP, Dioxus 0.7 desktop, existing CSS/theme contract tests.

## Global Constraints

- Preserve named-agent ownership: each named agent receives owned events plus unclaimed events; one anonymous agent receives the complete batch.
- Reject missing or ambiguous explicit-Send targets without consuming queued decisions.
- Keep interrupted delivery recoverable by named agent id or a sole anonymous claimant on the same brainstorm.
- Drive Send state from receipt events, never timers.
- Do not add dependencies, change the persisted document schema, expose internal connection ids through MCP, or add a recipient picker.
- Clear an optional comment only after Send is accepted.

## File map

- `src/store.rs`: transient waiter/receipt model, targeted flush validation, fixed batch boundaries, reconnect claims, Send/toast metadata, and unit tests.
- `src/sessions.rs`: global MCP connection registry, merged live/reconnecting snapshots, connection lease ids, independent watch channel, and registry tests.
- `src/server/tools.rs`: clone-safe MCP connection lease and `await_decisions` lifecycle integration.
- `tests/mcp_roundtrip.rs`: two-client routing and reconnect end-to-end coverage.
- `src/ui/app.rs`: connection-watch bridge and accessible toast mount.
- `src/ui/topbar.rs`: connection indicator/popover and shared receipt-driven Send behavior.
- `assets/main.css`: connection, receipt, and toast presentation.
- `src/theme.rs`: source/CSS accessibility and layout contracts.

---

### Task 1: Connection-scoped store delivery and recoverable receipts

**Files:**
- Modify: `src/store.rs`

**Interfaces:**
- Produces: `ConnectionId`, `Awaiter`, `SendStatus`, `ToastLevel`, `UiToast`, `ReconnectTarget`, and the extended `UiMeta` consumed by later tasks.
- Produces: `Store::request_flush(Option<String>) -> Result<(), StoreError>` and `Store::await_flush(Duration, Awaiter) -> Result<FlushOutcome, StoreError>`.
- Preserves: persisted `flush_seq`, `delivered_flush_seq`, `agent_cursors`, and `agent_flush` as restart recovery state.

- [ ] **Step 1: Add failing tests for deterministic target validation**

Add these test helpers near the existing delivery tests:

```rust
fn awaiter(id: u64, agent: Option<&str>) -> Awaiter {
    Awaiter {
        connection_id: ConnectionId(id),
        client_label: format!("Claude {id}"),
        agent: agent.map(str::to_owned),
    }
}

async fn wait_until(store: &Arc<Store>, count: usize) {
    for _ in 0..50 {
        if store.snapshot_meta().waiting_agents == count {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("expected {count} waiters, got {}", store.snapshot_meta().waiting_agents);
}
```

Add three `#[tokio::test]` cases:

```rust
#[tokio::test]
async fn explicit_send_rejects_no_waiter_without_consuming_queue() {
    let store = demo_store();
    pick_first_choice(&store);
    let before = store.peek_undelivered();

    let err = store.request_flush(None).unwrap_err();

    assert!(matches!(err, StoreError::NoWaitingClient));
    assert_eq!(store.peek_undelivered(), before);
    assert_eq!(store.snapshot_meta().send_status, SendStatus::Failed);
    assert!(store.snapshot_meta().toast.unwrap().message.contains("waiting"));
}

#[tokio::test]
async fn explicit_send_rejects_duplicate_agent_claims() {
    let store = demo_store();
    pick_first_choice(&store);
    let a = tokio::spawn({
        let store = store.clone();
        async move { store.await_flush(Duration::from_secs(30), awaiter(1, Some("alpha"))).await }
    });
    let b = tokio::spawn({
        let store = store.clone();
        async move { store.await_flush(Duration::from_secs(30), awaiter(2, Some("alpha"))).await }
    });
    wait_until(&store, 2).await;

    let err = store.request_flush(None).unwrap_err();

    assert!(matches!(err, StoreError::AmbiguousWaitingClients(_)));
    assert_eq!(store.read(|s| s.delivery_cursor), 0);
    a.abort();
    b.abort();
}

#[tokio::test]
async fn explicit_send_rejects_mixed_named_and_anonymous_waiters() {
    let store = demo_store();
    pick_first_choice(&store);
    let named = tokio::spawn({
        let store = store.clone();
        async move { store.await_flush(Duration::from_secs(30), awaiter(1, Some("alpha"))).await }
    });
    let anonymous = tokio::spawn({
        let store = store.clone();
        async move { store.await_flush(Duration::from_secs(30), awaiter(2, None)).await }
    });
    wait_until(&store, 2).await;

    assert!(matches!(
        store.request_flush(None).unwrap_err(),
        StoreError::AmbiguousWaitingClients(_)
    ));
    named.abort();
    anonymous.abort();
}
```

- [ ] **Step 2: Run the target-validation tests and confirm RED**

Run:

```bash
cargo test store::tests::explicit_send_rejects_ -- --nocapture
```

Expected: compilation fails because `Awaiter`, connection-aware `await_flush`, `SendStatus`, and the new errors do not exist.

- [ ] **Step 3: Add failing tests for routing, fixed boundaries, and recovery**

Adapt the existing `named_agents_receive_their_own_plus_unclaimed_decisions` setup so alpha and beta await with different connection ids. Start both futures before calling `request_flush(None)`, then assert each delivered slice contains its owned decision and the shared annotation, but not the other agent's decision. Assert `send_status == SendStatus::Sent` only after both futures complete.

Add these focused cases using that same two-agent setup:

```rust
#[tokio::test]
async fn receipt_excludes_edits_created_after_send() {
    let store = demo_store();
    pick_first_choice(&store);
    let waiting = tokio::spawn({
        let store = store.clone();
        async move { store.await_flush(Duration::from_secs(30), awaiter(1, None)).await }
    });
    wait_until(&store, 1).await;
    store.request_flush(None).unwrap();
    store.add_annotation(AnnotationKind::Note, 1.0, 2.0, 0.0, 0.0, "later".into());

    let FlushOutcome::Delivered(batch) = waiting.await.unwrap().unwrap() else {
        panic!("expected delivery");
    };
    assert_eq!(batch.len(), 1, "post-send annotation stays out of the receipt");
    assert_eq!(store.peek_undelivered().len(), 1, "later edit remains queued");
}

#[tokio::test]
async fn named_agent_reconnect_claims_orphaned_receipt() {
    let store = demo_store();
    pick_first_choice(&store);
    let first = tokio::spawn({
        let store = store.clone();
        async move { store.await_flush(Duration::from_secs(30), awaiter(1, Some("alpha"))).await }
    });
    wait_until(&store, 1).await;
    store.request_flush(None).unwrap();
    first.abort();
    let _ = first.await;
    assert_eq!(store.snapshot_meta().send_status, SendStatus::Reconnecting);

    let recovered = store
        .await_flush(Duration::from_secs(1), awaiter(2, Some("alpha")))
        .await
        .unwrap();
    assert!(matches!(recovered, FlushOutcome::Delivered(_)));
    assert_eq!(store.snapshot_meta().send_status, SendStatus::Sent);
}

#[tokio::test]
async fn sole_anonymous_reconnect_claims_orphaned_receipt() {
    let store = demo_store();
    pick_first_choice(&store);
    let first = tokio::spawn({
        let store = store.clone();
        async move { store.await_flush(Duration::from_secs(30), awaiter(10, None)).await }
    });
    wait_until(&store, 1).await;
    store.request_flush(None).unwrap();
    first.abort();
    let _ = first.await;

    let recovered = store
        .await_flush(Duration::from_secs(1), awaiter(11, None))
        .await
        .unwrap();
    assert!(matches!(recovered, FlushOutcome::Delivered(_)));
}
```

These current-thread Tokio tests abort immediately after synchronous `request_flush` returns, before yielding back to the waiting task. This deterministically exercises receipt orphaning without production delays or test hooks.

- [ ] **Step 4: Run the routing and recovery tests and confirm RED**

Run each exact test as a separate Cargo invocation:

```bash
cargo test store::tests::receipt_excludes_edits_created_after_send -- --exact --nocapture
cargo test store::tests::named_agent_reconnect_claims_orphaned_receipt -- --exact --nocapture
cargo test store::tests::sole_anonymous_reconnect_claims_orphaned_receipt -- --exact --nocapture
```

Expected: FAIL because delivery is still agent-string/global-cursor based and has no fixed receipt boundary or reconnect target.

- [ ] **Step 5: Implement the transient delivery model**

Add these public UI/server-facing types above `SessionState`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaiter {
    pub connection_id: ConnectionId,
    pub client_label: String,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendStatus {
    #[default]
    Idle,
    Sending,
    Sent,
    Reconnecting,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiToast {
    pub level: ToastLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectTarget {
    pub connection_id: ConnectionId,
    pub client_label: String,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RecipientKey {
    Agent(String),
    Anonymous(ConnectionId),
}

#[derive(Debug, Clone)]
struct Waiter {
    awaiter: Awaiter,
    recipient: RecipientKey,
}

#[derive(Debug, Clone)]
struct ReceiptTarget {
    key: RecipientKey,
    connection_id: Option<ConnectionId>,
    last_connection_id: ConnectionId,
    client_label: String,
    delivered: bool,
}

#[derive(Debug, Clone)]
struct SendReceipt {
    flush_seq: u64,
    end_cursor: usize,
    doc_at_send: SessionDoc,
    targets: Vec<ReceiptTarget>,
}
```

Add `#[serde(skip)] waiters: BTreeMap<ConnectionId, Waiter>`, `#[serde(skip)] send_receipt: Option<SendReceipt>`, `#[serde(skip)] send_status: SendStatus`, and `#[serde(skip)] toast: Option<UiToast>` to `SessionState`. Extend `UiMeta` with `send_status` and `toast`. Add `Store::reconnecting_targets() -> Vec<ReconnectTarget>`, populated from unfinished receipt targets whose current `connection_id` is `None`; each target retains the previous connection id separately so `Sessions` can merge it with the disconnected registry row.

Add these errors:

```rust
#[error("no Claude session is waiting on this brainstorm")]
NoWaitingClient,
#[error("cannot choose a Claude recipient: {0}")]
AmbiguousWaitingClients(String),
```

Implement `validated_targets(&SessionState) -> Result<Vec<ReceiptTarget>, StoreError>` with these exact rules:

1. Empty waiter map returns `NoWaitingClient`.
2. Named and anonymous waiters together return `AmbiguousWaitingClients("named and anonymous sessions are waiting together".into())`.
3. More than one anonymous waiter returns `AmbiguousWaitingClients("multiple anonymous sessions are waiting".into())`.
4. Group named waiters by agent id; any group with more than one connection returns an error naming that agent.
5. Otherwise return one target per distinct named agent, or the sole anonymous target.

Change explicit Send to validate before appending the optional comment:

```rust
pub fn request_flush(&self, comment: Option<String>) -> Result<(), StoreError> {
    self.mutate(|s| {
        let targets = match validated_targets(s) {
            Ok(targets) => targets,
            Err(err) => {
                s.send_status = SendStatus::Failed;
                s.toast = Some(UiToast { level: ToastLevel::Error, message: err.to_string() });
                return Err(err);
            }
        };
        if comment.as_deref().is_some_and(|c| !c.trim().is_empty()) {
            push_event(s, DecisionKind::FlushRequested {
                comment: comment.map(|c| c.trim().to_owned()),
            });
        }
        s.flush_seq += 1;
        s.send_receipt = Some(SendReceipt {
            flush_seq: s.flush_seq,
            end_cursor: s.decision_log.len(),
            doc_at_send: s.doc.clone(),
            targets,
        });
        s.send_status = SendStatus::Sending;
        s.toast = None;
        clear_undo(s);
        Ok(())
    })
}
```

Change `await_flush` to accept `Awaiter`, register it in a connection-aware `WaitGuard`, and return `Result<FlushOutcome, StoreError>`. Registration rebinds an orphaned `Agent(id)` target to a new connection with the same agent id. It rebinds an anonymous target only when there is one anonymous orphan and no other anonymous waiter. Rebinding updates both connection-id fields to the new connection. The guard removes the waiter on drop; if it owned an incomplete receipt target, copy the live id into `last_connection_id`, clear `connection_id`, set `Reconnecting`, and emit a warning toast that names the client/agent without exposing the internal id.

Replace `try_deliver(&Option<String>)` with `try_deliver(ConnectionId)`. It may consume only the receipt target bound to that connection. Slice only through `end_cursor`, apply `addressed_to` for named targets, update the matching persisted agent cursor/flush immediately, and mark the target delivered. When all targets are delivered:

```rust
s.delivery_cursor = receipt.end_cursor;
s.delivered_flush_seq = receipt.flush_seq;
s.pending_base = (s.decision_log.len() > receipt.end_cursor)
    .then(|| receipt.doc_at_send.clone());
s.queue_edit_error = None;
s.send_status = SendStatus::Sent;
push_activity(s, ActivityOrigin::User, "sent decisions to Claude".into());
```

Keep the completed receipt long enough for the UI to show `Sent`; clear completed/failed feedback from `push_event` when new queued work arrives. When a new waiter registers after a completed or failed receipt, reset the button state to `Idle` while leaving an undismissed toast visible. Add `Store::dismiss_toast()` to clear only the toast. Prevent `remove_queued_change` and comment editing from touching events at or before an unfinished receipt's `end_cursor`; return an interaction error explaining that the batch is currently being delivered.

Update `autoflush` to create the same targeted receipt when `validated_targets` succeeds. If no waiter exists, retain the existing persisted `flush_seq` claimable path. For ambiguity, keep the decisions queued, set Failed/error toast, and do not increment the flush.

- [ ] **Step 6: Make the store suite GREEN**

Update existing delivery tests to create `Awaiter` values and unwrap the new `Result`. Tests that called `request_flush` without a waiter must first spawn the intended waiter, except tests specifically covering persisted/autoflush claimable delivery.

Run:

```bash
cargo test store::tests -- --nocapture
```

Expected: all store tests pass, including target validation, fixed boundary, partial completion, future-drop cleanup, and reconnect recovery.

- [ ] **Step 7: Commit the store behavior**

```bash
git add src/store.rs
git commit -m "feat(store): target Claude delivery receipts"
```

---

### Task 2: Global MCP connection registry and clone-safe lifecycle

**Files:**
- Modify: `src/sessions.rs`
- Modify: `src/server/tools.rs`

**Interfaces:**
- Consumes: `ConnectionId` and `Awaiter` from Task 1.
- Produces: `ConnectionInfo`, `ConnectionState`, registry snapshot/watch methods, and server lifecycle updates used by the UI and MCP tests.

- [ ] **Step 1: Add failing registry tests**

Add `use std::time::Duration;` to the existing test module and add these tests in `src/sessions.rs`:

```rust
#[test]
fn connection_registry_reports_metadata_and_notifies_independently() {
    let sessions = Sessions::single(Store::new(SessionState::default()), tmp_root("connections"));
    let mut changes = sessions.subscribe_connections();
    let before = *changes.borrow_and_update();
    let id = sessions.next_connection_id();

    sessions.connect_client(id, "claude-code".into(), "1.2.3".into());
    sessions.set_connection_waiting(id, "default".into(), Some("alpha".into()));

    assert!(*changes.borrow_and_update() > before);
    assert_eq!(sessions.connections(), vec![ConnectionInfo {
        id,
        client_name: "claude-code".into(),
        version: "1.2.3".into(),
        state: ConnectionState::Waiting {
            session: "default".into(),
            agent: Some("alpha".into()),
        },
    }]);
}

#[test]
fn disconnect_removes_live_connection_without_bumping_session_generation() {
    let sessions = Sessions::single(Store::new(SessionState::default()), tmp_root("disconnect"));
    let id = sessions.next_connection_id();
    let mut generation = sessions.subscribe_generation();
    let before = *generation.borrow_and_update();
    sessions.connect_client(id, "claude-code".into(), "1".into());

    sessions.disconnect_client(id);

    assert!(sessions.connections().is_empty());
    assert_eq!(*generation.borrow_and_update(), before);
}

#[tokio::test]
async fn orphaned_receipt_keeps_a_reconnecting_connection_row() {
    let store = Store::new(SessionState::default());
    let sessions = Sessions::single(store.clone(), tmp_root("reconnecting-row"));
    let id = sessions.next_connection_id();
    sessions.connect_client(id, "claude-code".into(), "1".into());
    let waiting = tokio::spawn({
        let store = store.clone();
        async move {
            store.await_flush(
                Duration::from_secs(30),
                Awaiter {
                    connection_id: id,
                    client_label: "claude-code 1".into(),
                    agent: Some("alpha".into()),
                },
            ).await
        }
    });
    for _ in 0..50 {
        if store.snapshot_meta().waiting_agents == 1 { break; }
        tokio::task::yield_now().await;
    }
    store.request_flush(None).unwrap();
    waiting.abort();
    let _ = waiting.await;
    sessions.disconnect_client(id);

    assert!(matches!(
        sessions.connections()[0].state,
        ConnectionState::Reconnecting { ref session, ref agent }
            if session == "default" && agent.as_deref() == Some("alpha")
    ));
}
```

- [ ] **Step 2: Run registry tests and confirm RED**

Run:

```bash
cargo test sessions::tests::connection_registry_ -- --nocapture
cargo test sessions::tests::disconnect_removes_ -- --nocapture
cargo test sessions::tests::orphaned_receipt_keeps_a_reconnecting_connection_row -- --exact --nocapture
```

Expected: compilation fails because the registry API and types do not exist.

- [ ] **Step 3: Implement the registry in `Sessions`**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Waiting { session: String, agent: Option<String> },
    Receiving { session: String, agent: Option<String> },
    Reconnecting { session: String, agent: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub id: ConnectionId,
    pub client_name: String,
    pub version: String,
    pub state: ConnectionState,
}
```

Extend `Sessions` with a private registry entry containing `ConnectionInfo` plus a live/disconnected bit, `connections: Mutex<BTreeMap<ConnectionId, RegistryEntry>>`, `connection_generation: watch::Sender<u64>`, and `next_connection: AtomicU64`, initialized by both constructors. Implement:

```rust
pub fn next_connection_id(&self) -> ConnectionId;
pub fn connect_client(&self, id: ConnectionId, client_name: String, version: String);
pub fn set_connection_waiting(&self, id: ConnectionId, session: String, agent: Option<String>);
pub fn set_connection_receiving(&self, id: ConnectionId, session: String, agent: Option<String>);
pub fn set_connection_connected(&self, id: ConnectionId);
pub fn disconnect_client(&self, id: ConnectionId);
pub fn connection(&self, id: ConnectionId) -> Option<ConnectionInfo>;
pub fn connections(&self) -> Vec<ConnectionInfo>;
pub fn subscribe_connections(&self) -> watch::Receiver<u64>;
```

Every mutation bumps only `connection_generation`. `disconnect_client` marks the entry disconnected instead of deleting its metadata. `connections()` returns every live entry, then checks every store's `reconnecting_targets()` and includes a disconnected entry as `ConnectionState::Reconnecting` only while its previous connection id owns an orphaned receipt. A normal disconnected entry is omitted, so clone-safe double cleanup remains harmless and reconnect completion automatically drops the stale row on the next connection-state bump.

- [ ] **Step 4: Add failing MCP lifecycle coverage**

In `tests/mcp_roundtrip.rs`, after creating a real client, assert the registry has one connection with the client's handshake name/version and `Connected`. Start `await_decisions`, wait until its state is `Waiting { session: "default", agent: Some("alpha") }`, cancel the client, and assert the live registry becomes empty.

Run:

```bash
cargo test --test mcp_roundtrip connection_lifecycle_is_visible -- --exact --nocapture
```

Expected: FAIL because `NodestormServer` does not register handshake or await lifecycle state.

- [ ] **Step 5: Implement the clone-safe server lease**

Change `NodestormServer` to hold an `Arc<ConnectionLease>`:

```rust
struct ConnectionLease {
    id: ConnectionId,
    sessions: Arc<Sessions>,
    initialized: std::sync::atomic::AtomicBool,
}

impl Drop for ConnectionLease {
    fn drop(&mut self) {
        if self.initialized.load(std::sync::atomic::Ordering::Acquire) {
            self.sessions.disconnect_client(self.id);
        }
    }
}

#[derive(Clone)]
pub struct NodestormServer {
    sessions: Arc<Sessions>,
    connection: Arc<ConnectionLease>,
}
```

`NodestormServer::new` reserves one id. Implement `ServerHandler::on_initialized` and read the already-negotiated handshake through `context.peer.peer_info()`:

```rust
async fn on_initialized(&self, context: rmcp::service::NotificationContext<RoleServer>) {
    let Some(info) = context.peer.peer_info() else { return };
    if !self.connection.initialized.swap(true, std::sync::atomic::Ordering::AcqRel) {
        self.sessions.connect_client(
            self.connection.id,
            info.client_info.name.clone(),
            info.client_info.version.clone(),
        );
    }
}
```

Around `await_decisions`, resolve the canonical session before changing state, fetch the registered client label, set `Waiting`, and use an RAII state guard that restores `Connected` on every return/cancellation path. Pass this `Awaiter` to the store:

```rust
let awaiter = Awaiter {
    connection_id: self.connection.id,
    client_label: format!("{} {}", info.client_name, info.version),
    agent: p.agent.clone(),
};
let outcome = session_store.await_flush(timeout, awaiter).await.map_err(store_err)?;
```

Set registry state to `Receiving` for a delivered outcome before building `AwaitResult`; timeout and error paths return to `Connected` through the guard.

- [ ] **Step 6: Make registry and lifecycle tests GREEN**

Run:

```bash
cargo test sessions::tests -- --nocapture
cargo test --test mcp_roundtrip connection_lifecycle_is_visible -- --exact --nocapture
```

Expected: both commands pass and client cancellation leaves no live registry row or stuck store waiter.

- [ ] **Step 7: Commit connection lifecycle support**

```bash
git add src/sessions.rs src/server/tools.rs tests/mcp_roundtrip.rs
git commit -m "feat(mcp): track incoming Claude sessions"
```

---

### Task 3: Real two-client routing and reconnect recovery

**Files:**
- Modify: `tests/mcp_roundtrip.rs`

**Interfaces:**
- Consumes: targeted store awaits from Task 1 and connection lifecycle from Task 2.
- Produces: end-to-end proof that two transport sessions cannot consume each other's response and a reestablished session can recover.

- [ ] **Step 1: Replace the single-transport multi-agent test with two real clients**

Extract:

```rust
async fn connect_client(port: u16, name: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ClientInfo> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp")),
    );
    rmcp::model::ClientInfo::default()
        .serve(transport)
        .await
        .unwrap_or_else(|err| panic!("{name} handshake failed: {err}"))
}
```

Use two independently connected clients. Alpha proposes its node through client A; beta upserts its node through client B. Spawn both `await_decisions` calls concurrently, poll `store.snapshot_meta().waiting_agents == 2`, make the two user decisions without closing the last choice until both awaits are registered, and explicitly Send. Assert alpha sees only alpha-owned plus unclaimed events and beta sees only beta-owned plus unclaimed events. Assert the registry showed two distinct live rows during the wait.

- [ ] **Step 2: Run the two-client test and confirm RED if any connection routing is incomplete**

Run:

```bash
cargo test --test mcp_roundtrip multi_agent_awaits_route_per_connection -- --exact --nocapture
```

Expected before the full implementation: FAIL if either client can win the other client's batch or receipt completion is reported before both targets consume.

- [ ] **Step 3: Add reconnect recovery round trip**

Create `reestablished_agent_recovers_pending_delivery`: connect alpha, propose an alpha-owned choice, begin its await, wait for registration, cancel the first client, then make the final user decision so the existing no-waiter autoflush records a claimable persisted flush. Connect a fresh client and call `await_decisions` with `agent: "alpha"` on the same named brainstorm. Assert the fresh client receives the original decision exactly once and the persisted agent cursor advances. The Task 1 unit test remains the deterministic proof for a transport dropping after an explicit receipt has already targeted it.

- [ ] **Step 4: Run reconnect test and make GREEN**

Run:

```bash
cargo test --test mcp_roundtrip reestablished_agent_recovers_pending_delivery -- --exact --nocapture
```

Expected: PASS; cancelling the old transport does not advance the pending target's cursor, and the new transport with the same agent id consumes it.

- [ ] **Step 5: Run the complete MCP integration target**

```bash
cargo test --test mcp_roundtrip -- --nocapture
```

Expected: every MCP round-trip test passes, including timeouts, named sessions, two-agent ownership, cancellation, and recovery.

- [ ] **Step 6: Commit end-to-end routing coverage**

```bash
git add src/store.rs src/server/tools.rs tests/mcp_roundtrip.rs
git commit -m "test(mcp): verify targeted reconnect delivery"
```

---

### Task 4: Connection indicator, deterministic Send button, and error toast

**Files:**
- Modify: `src/ui/app.rs`
- Modify: `src/ui/topbar.rs`
- Modify: `assets/main.css`
- Modify: `src/theme.rs`

**Interfaces:**
- Consumes: `Signal<Vec<ConnectionInfo>>`, `UiMeta::send_status`, and `UiMeta::toast`.
- Produces: `send_label(SendStatus) -> &'static str` and `connection_state_label(&ConnectionState) -> String` for deterministic rendering/tests.

- [ ] **Step 1: Add failing pure UI and source-contract tests**

In `src/ui/topbar.rs`, add tests for the pure labels:

```rust
#[test]
fn send_labels_are_receipt_driven() {
    assert_eq!(send_label(SendStatus::Idle), "Send");
    assert_eq!(send_label(SendStatus::Sending), "Sending...");
    assert_eq!(send_label(SendStatus::Sent), "Sent");
    assert_eq!(send_label(SendStatus::Reconnecting), "Reconnecting...");
    assert_eq!(send_label(SendStatus::Failed), "Failed - Retry");
}

#[test]
fn connection_labels_name_state_session_and_agent() {
    assert_eq!(connection_state_label(&ConnectionState::Connected), "Connected");
    assert_eq!(
        connection_state_label(&ConnectionState::Waiting {
            session: "plan".into(),
            agent: Some("alpha".into()),
        }),
        "Waiting · plan · alpha"
    );
    assert_eq!(
        connection_state_label(&ConnectionState::Reconnecting {
            session: "plan".into(),
            agent: None,
        }),
        "Reconnecting · plan"
    );
}
```

In `src/theme.rs`, add:

```rust
#[test]
fn claude_connections_send_receipt_and_error_toast_are_accessible() {
    assert!(TOPBAR_SOURCE.contains(r#"aria_label: \"Claude MCP connections\""#));
    assert!(TOPBAR_SOURCE.contains(r#"class: \"connection-row\""#));
    assert!(APP_SOURCE.contains(r#"role: \"alert\""#));
    assert!(APP_SOURCE.contains(r#"store.dismiss_toast()"#));
    assert_block_contains(".connection-pop", "max-height: calc(100vh - 64px)");
    assert_block_contains(".connection-row", "display: grid");
    assert_block_contains(".delivery-toast", "position: fixed");
    assert_block_contains(".delivery-toast", "z-index: 30");
    assert_block_contains(".delivery-toast-error", "color: var(--status-removed)");
}
```

- [ ] **Step 2: Run UI contract tests and confirm RED**

Run:

```bash
cargo test ui::topbar::tests::send_labels_are_receipt_driven -- --exact
cargo test ui::topbar::tests::connection_labels_name_state_session_and_agent -- --exact
cargo test theme::tests::claude_connections_send_receipt_and_error_toast_are_accessible -- --exact
```

Expected: compilation/assertion failure because the helpers, markup, and CSS do not exist.

- [ ] **Step 3: Bridge connection changes without resetting canvas state**

In `App`, initialize:

```rust
let mut connections = use_signal(|| sessions.connections());
use_future({
    let sessions = sessions.clone();
    move || {
        let sessions = sessions.clone();
        async move {
            let mut changes = sessions.subscribe_connections();
            while changes.changed().await.is_ok() {
                connections.set(sessions.connections());
            }
        }
    }
});
```

Pass `connections` to `TopBar`. Do not add this channel to the existing session-generation `select!` because that branch resets selection/search/panels.

After `.main`, render the latest toast:

```rust
if let Some(toast) = meta.read().toast.clone() {
    div {
        class: match toast.level {
            ToastLevel::Warning => "delivery-toast delivery-toast-warning",
            ToastLevel::Error => "delivery-toast delivery-toast-error",
        },
        role: "alert",
        span { "{toast.message}" }
        button {
            aria_label: "Dismiss error",
            onclick: {
                let store = sessions.active_store();
                move |_| store.dismiss_toast()
            },
            "×"
        }
    }
}
```

- [ ] **Step 4: Render the connection indicator and share Send behavior**

Extend `TopBar` with `connections: Signal<Vec<ConnectionInfo>>`. Add:

```rust
fn send_label(status: SendStatus) -> &'static str {
    match status {
        SendStatus::Idle => "Send",
        SendStatus::Sending => "Sending...",
        SendStatus::Sent => "Sent",
        SendStatus::Reconnecting => "Reconnecting...",
        SendStatus::Failed => "Failed - Retry",
    }
}

fn connection_state_label(state: &ConnectionState) -> String {
    let scoped = |label: &str, session: &str, agent: &Option<String>| match agent {
        Some(agent) => format!("{label} · {session} · {agent}"),
        None => format!("{label} · {session}"),
    };
    match state {
        ConnectionState::Connected => "Connected".into(),
        ConnectionState::Waiting { session, agent } => scoped("Waiting", session, agent),
        ConnectionState::Receiving { session, agent } => scoped("Receiving", session, agent),
        ConnectionState::Reconnecting { session, agent } => {
            scoped("Reconnecting", session, agent)
        }
    }
}
```

Create one helper used by both send buttons. It trims the comment, calls `store.request_flush`, clears the comment and closes the compose popover only on `Ok(())`, and leaves both untouched on `Err` because the store already publishes the toast:

```rust
fn submit_send(
    store: &Arc<Store>,
    mut comment: Signal<String>,
    mut compose_open: Signal<bool>,
) {
    let text = comment.read().trim().to_owned();
    if store.request_flush((!text.is_empty()).then_some(text)).is_ok() {
        comment.set(String::new());
        compose_open.set(false);
    }
}
```

Each Dioxus handler clones the store and calls `submit_send(&store, comment, compose_open)`. Button text is `send_label(m.send_status)`. Disable only while `Sending` or `Reconnecting`; allow Idle/Failed when queued work or a waiter exists, and leave Sent disabled until new work/waiter resets it.

Add an `export-menu` connection pod beside Send. Its button always renders, shows a green dot/count when connections are live and a dim zero/offline state otherwise, and has `aria_label: "Claude MCP connections"`. Its dropdown renders every merged `ConnectionInfo` from `Sessions::connections()` with client/version plus state/session/agent; reconnecting rows already arrive in that snapshot and require no UI-side identity merge.

- [ ] **Step 5: Add minimal themed CSS**

Add flat selector blocks compatible with `theme.rs`'s exact-selector helper:

```css
.connection-pod { position: relative; }
.connection-toggle { min-width: 38px; justify-content: center; }
.connection-dot { color: var(--text-dim); }
.connection-dot.live { color: var(--badge-decided); }
.connection-pop { right: 0; min-width: min(280px, calc(100vw - 32px)); max-width: calc(100vw - 32px); max-height: calc(100vh - 64px); overflow-y: auto; }
.connection-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 3px 12px; padding: 8px 10px; }
.connection-client { overflow-wrap: anywhere; }
.connection-meta { color: var(--text-dim); font-size: 11px; }
.connection-state { color: var(--badge-decided); font-size: 11px; text-align: right; }
.connection-state.reconnecting { color: var(--badge-open); }
.btn-send.sent { background: var(--badge-decided); border-color: var(--badge-decided); }
.btn-send.failed { background: var(--status-removed); border-color: var(--status-removed); }
.delivery-toast { position: fixed; right: 18px; bottom: 18px; z-index: 30; display: flex; align-items: start; gap: 12px; max-width: min(440px, calc(100vw - 36px)); padding: 12px 14px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-panel); box-shadow: 0 12px 32px color-mix(in srgb, var(--shadow) 40%, transparent); }
.delivery-toast-warning { color: var(--badge-open); }
.delivery-toast-error { color: var(--status-removed); }
.delivery-toast button { margin-left: auto; border: 0; background: transparent; color: inherit; cursor: pointer; font: inherit; }
```

Add `.connection-count { display: none; }` inside the existing `@container topbar (max-width: 600px)` block. Keep the connection dot and toast dismiss button visible.

- [ ] **Step 6: Make focused UI tests GREEN**

Run:

```bash
cargo test ui::topbar::tests::send_labels_are_receipt_driven -- --exact
cargo test ui::topbar::tests::connection_labels_name_state_session_and_agent -- --exact
cargo test theme::tests::claude_connections_send_receipt_and_error_toast_are_accessible -- --exact
cargo test theme::tests -- --nocapture
```

Expected: all focused UI/source/CSS contracts pass.

- [ ] **Step 7: Commit the UI**

```bash
git add src/ui/app.rs src/ui/topbar.rs assets/main.css src/theme.rs
git commit -m "feat(ui): show Claude delivery status"
```

---

### Task 5: Full verification and documentation alignment

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: completed behavior from Tasks 1-4.
- Produces: user-facing explanation and fresh repository-wide verification evidence.

- [ ] **Step 1: Document the connection and Send semantics**

In the named-session/MCP workflow section, add one paragraph stating that the topbar Claude indicator lists live MCP transports and their waiting brainstorm/agent; Send targets only the unambiguous matching waiters; interrupted named-agent delivery resumes when the same agent id reconnects; errors leave decisions queued and appear in a toast.

- [ ] **Step 2: Format and inspect the complete diff**

Run:

```bash
cargo fmt --all
git diff --check
git status --short
git diff --stat
```

Expected: no formatting/whitespace errors; only the files in this plan are modified.

- [ ] **Step 3: Run all Rust tests**

```bash
cargo test --locked
```

Expected: exit 0 with unit and MCP integration tests all passing.

- [ ] **Step 4: Run Clippy and format verification**

```bash
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

Expected: both commands exit 0 with no warnings or formatting diffs.

- [ ] **Step 5: Run package/release contract checks**

```bash
npm --prefix plugins/nodestorm ci --ignore-scripts
node --test tests/*.mjs
node scripts/validate-release.mjs
node scripts/validate-npm-pack.mjs
```

Expected: all Node test files and validation scripts exit 0.

- [ ] **Step 6: Re-read requirements against evidence**

Confirm from tests and diff:

- every live MCP connection appears in the indicator;
- only the correct waiting agent connection receives its slice;
- duplicate/anonymous ambiguity consumes nothing;
- dropped delivery becomes Reconnecting and the same agent can reclaim it;
- Send reaches Sent or Failed/Reconnecting from receipt events, not time;
- concrete errors appear in a dismissible `role="alert"` toast;
- rejected Send retains the optional comment and queued decisions.

- [ ] **Step 7: Commit documentation and any verification-only fixes**

```bash
git add README.md
git commit -m "docs: explain targeted Claude delivery"
```
