# Claude Delivery Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve the MCP `Receiving` state and deliver queued work to named agents that were absent during Send.

**Architecture:** Keep the existing connection-state guard and per-agent delivery cursors. Reorder the successful state transition so the guard cannot overwrite it, and use the named recipient's `agent_flush` value when deciding whether a newly registered waiter has pending work.

**Tech Stack:** Rust, Tokio, rmcp, Cargo tests

## Global Constraints

- Do not change the MCP tool request or response schema.
- Do not change the persisted session document schema.
- Keep anonymous delivery governed by the global flush cursor.
- Add no dependencies.

---

### Task 1: Preserve Receiving after delivery

**Files:**
- Modify: `tests/mcp_roundtrip.rs`
- Modify: `src/server/tools.rs`

**Interfaces:**
- Consumes: `ConnectionStateGuard`, `Sessions::set_connection_receiving`
- Produces: a delivered `await_decisions` call leaves its live row in `ConnectionState::Receiving`

- [ ] **Step 1: Write the failing integration assertion**

After the multi-agent delivery results are received, assert that both live
connections remain in `Receiving` with their session and agent identity.

```rust
let receiving = sessions.connections();
assert!(receiving.iter().any(|connection| matches!(
    &connection.state,
    ConnectionState::Receiving { session, agent }
        if session == "default" && agent.as_deref() == Some("alpha")
)));
assert!(receiving.iter().any(|connection| matches!(
    &connection.state,
    ConnectionState::Receiving { session, agent }
        if session == "default" && agent.as_deref() == Some("beta")
)));
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test --locked --test mcp_roundtrip multi_agent_awaits_route_per_connection -- --exact`

Expected: FAIL because the guard resets both rows to `Connected`.

- [ ] **Step 3: Apply the minimal lifecycle fix**

Name the guard `state_guard`, explicitly drop it in the delivered branch, and
only then set the connection to `Receiving`.

```rust
let state_guard = ConnectionStateGuard { /* existing fields */ };

FlushOutcome::Delivered(decisions) => {
    drop(state_guard);
    self.sessions.set_connection_receiving(
        self.connection.id,
        session,
        p.agent.clone(),
    );
    // existing result construction
}
```

- [ ] **Step 4: Run the focused test and verify GREEN**

Run: `cargo test --locked --test mcp_roundtrip multi_agent_awaits_route_per_connection -- --exact`

Expected: PASS.

### Task 2: Recover work for an absent named agent

**Files:**
- Modify: `src/store.rs`

**Interfaces:**
- Consumes: `RecipientKey`, `SessionState::agent_flush`, `SessionState::flush_seq`
- Produces: named waiter registration creates a claimable receipt whenever that agent's flush position is behind

- [ ] **Step 1: Write the failing store test**

Create alpha- and gamma-owned choices, register only alpha, send both queued
events, let alpha consume, then register gamma and assert gamma receives only
its addressed event.

```rust
let gamma = store
    .await_flush(Duration::from_secs(1), awaiter(2, Some("gamma")))
    .await
    .expect("gamma await");
let FlushOutcome::Delivered(events) = gamma else {
    panic!("gamma did not receive its pending event");
};
assert_eq!(events.len(), 1);
assert_eq!(events[0].target_agent.as_deref(), Some("gamma"));
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test --locked store::tests::absent_named_agent_claims_its_pending_flush -- --exact`

Expected: FAIL with a timeout because the global flush was already marked delivered.

- [ ] **Step 3: Use the recipient-specific pending cursor**

Replace the final global-only waiter check with the same recipient-specific
pending rule already used for claimable receipts.

```rust
let pending = match &recipient {
    RecipientKey::Agent(agent) => {
        s.agent_flush.get(agent).copied().unwrap_or(0) < s.flush_seq
    }
    RecipientKey::Anonymous(_) => s.delivered_flush_seq < s.flush_seq,
};
if !unfinished_receipt && pending {
    // existing claimable receipt construction
}
```

- [ ] **Step 4: Run the focused test and verify GREEN**

Run: `cargo test --locked store::tests::absent_named_agent_claims_its_pending_flush -- --exact`

Expected: PASS.

### Task 3: Verify and publish

**Files:**
- Verify all modified files

**Interfaces:**
- Consumes: completed fixes from Tasks 1 and 2
- Produces: a follow-up pull request targeting `main`

- [ ] **Step 1: Run complete verification**

```bash
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: every command exits zero.

- [ ] **Step 2: Review the final diff**

Run: `git diff --check && git diff --stat && git status -sb`

Expected: only the two review fixes, their regression tests, and these design
artifacts are changed.

- [ ] **Step 3: Commit, push, and open the follow-up PR**

```bash
git add src/server/tools.rs src/store.rs tests/mcp_roundtrip.rs
git commit -m "fix(mcp): preserve targeted delivery state"
git push -u origin codex/fix-claude-delivery-review
gh pr create --base main --head codex/fix-claude-delivery-review \
  --title "Fix Claude delivery review findings" \
  --body-file /tmp/nodestorm-followup-pr.md
```

Expected: GitHub returns the new follow-up PR URL.
