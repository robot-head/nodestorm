# Connection Popover Readability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep long Claude MCP connection statuses readable without collapsing client names into vertical text.

**Architecture:** Preserve the existing Dioxus markup and responsive popover bounds. Define explicit CSS grid areas so client and version share the first line while status spans the second line.

**Tech Stack:** Rust, Dioxus RSX, CSS, Rust unit tests

## Global Constraints

- Do not change Dioxus markup, connection state, status labels, routing, or reconnect behavior.
- Do not add dependencies.
- Keep the existing popover viewport width and height limits.

---

### Task 1: Stack Connection Identity Above Status

**Files:**
- Modify: `src/theme.rs:293-306`
- Modify: `assets/main.css:1334-1362`

**Interfaces:**
- Consumes: existing `.connection-row`, `.connection-client`, `.connection-meta`, and `.connection-state` markup classes
- Produces: an explicit `client` / `meta` / `state` CSS grid layout with status spanning the full row

- [x] **Step 1: Write the failing stylesheet contract test**

Extend `claude_connections_send_receipt_and_error_toast_are_accessible` in
`src/theme.rs` with these assertions:

```rust
assert_block_contains(
    ".connection-row",
    "grid-template-areas: \"client meta\" \"state state\"",
);
assert_block_contains(".connection-client", "grid-area: client");
assert_block_contains(".connection-meta", "grid-area: meta");
assert_block_contains(".connection-state", "grid-area: state");
assert_block_contains(".connection-state", "text-align: left");
```

- [x] **Step 2: Run the focused test and verify it fails**

Run:

```bash
cargo test theme::tests::claude_connections_send_receipt_and_error_toast_are_accessible -- --exact
```

Expected: FAIL because `.connection-row` does not define named grid areas.

- [x] **Step 3: Implement the minimal CSS layout**

Update the existing rules in `assets/main.css`:

```css
.connection-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  grid-template-areas: "client meta" "state state";
  gap: 3px 12px;
  padding: 8px 10px;
}

.connection-client {
  grid-area: client;
  overflow-wrap: anywhere;
}

.connection-meta {
  grid-area: meta;
  color: var(--text-dim);
  font-size: 11px;
}

.connection-state {
  grid-area: state;
  color: var(--badge-decided);
  font-size: 11px;
  text-align: left;
}
```

- [x] **Step 4: Run focused and full verification**

Run:

```bash
cargo test theme::tests::claude_connections_send_receipt_and_error_toast_are_accessible -- --exact
cargo test
git diff --check
```

Expected: the focused test and full suite PASS; `git diff --check` reports no errors.

Visually open the connection popover with a long Receiving session/agent label.
Confirm `claude-code` reads normally on the first line, its version is aligned to
the right, and the full status wraps beneath them without exceeding the popover.

- [x] **Step 5: Commit the fix**

```bash
git add src/theme.rs assets/main.css docs/superpowers/plans/2026-07-20-connection-popover-readability.md
git commit -m "fix: keep connection rows readable"
```
