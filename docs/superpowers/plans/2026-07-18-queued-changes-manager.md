# Queued Changes Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the queued-count status segment into a right-side staging panel where any undelivered action can be inspected, edited, or removed safely.

**Architecture:** Persist a snapshot of the document immediately before the current undelivered batch. When an item is removed, rebuild the document from that snapshot by replaying the remaining events, moving actions whose targets no longer exist into an explicit blocked queue. A focused Dioxus panel renders pending and blocked entries while the existing node/choice UI performs replacements.

**Tech Stack:** Rust 2024, Dioxus 0.7 desktop, serde session persistence, Cargo tests.

## Global Constraints

- Keep delivered events immutable and preserve the current exactly-once delivery cursor semantics.
- Persist queue-rebuild state with `#[serde(default)]` so saved sessions remain readable.
- Do not add dependencies.
- Preserve the document revision monotonicity when rebuilding queued state.
- The panel must be hideable and mutually exclusive with Timeline and the selected-node panel.

---

### Task 1: Persist sufficient queued-event data and replay it safely

**Files:**
- Modify: `src/model.rs:497-551`
- Modify: `src/store.rs:25-104, 240-410, 768-918, 1640-1780`

**Interfaces:**
- Produces `QueuedChange { event: DecisionEvent, blocked_reason: Option<String> }` and `QueueEditTarget { node_id: Option<NodeId>, choice_id: Option<ChoiceId> }`.
- Produces `Store::queued_changes() -> Vec<QueuedChange>`, `Store::remove_queued_change(seq: u64) -> Result<QueueEditTarget, StoreError>`, and `Store::remove_blocked_change(seq: u64) -> Result<QueueEditTarget, StoreError>`.
- `remove_queued_change` only accepts events at or after `delivery_cursor`; it never changes delivered log entries.

- [ ] **Step 1: Write failing store tests for arbitrary removal and dependency failure**

Add these tests to `src/store.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn removing_an_earlier_queued_change_replays_later_changes() {
    let store = demo_store();
    let node = store.add_user_node("Widget".into(), NodeKind::Component, None).unwrap();
    store.edit_node(&node, "Renamed".into(), NodeKind::Service, "edited".into()).unwrap();

    store.remove_queued_change(1).unwrap();

    assert!(store.snapshot_doc().node(&node).is_none());
    let changes = store.queued_changes();
    assert_eq!(changes.len(), 1);
    assert!(changes[0].blocked_reason.as_deref().is_some_and(|reason| reason.contains("node")));
    assert!(store.peek_undelivered().is_empty(), "blocked events do not send");
}

#[test]
fn removing_a_choice_pick_reopens_that_choice_and_keeps_a_note() {
    let store = demo_store();
    pick_first_choice(&store);
    store.add_note(&NodeId::from("sync-engine"), "Need migration notes".into()).unwrap();

    let target = store.remove_queued_change(1).unwrap();

    assert_eq!(target.node_id, Some(NodeId::from("sync-engine")));
    assert_eq!(target.choice_id, Some(ChoiceId::from("conflict-resolution")));
    let choice = store.snapshot_doc().node(&NodeId::from("sync-engine")).unwrap()
        .choice(&ChoiceId::from("conflict-resolution")).unwrap();
    assert_eq!(choice.status, ChoiceStatus::Open);
    assert_eq!(store.peek_undelivered().len(), 1);
}
```

- [ ] **Step 2: Run the new tests and verify they fail**

Run: `cargo test store::tests::removing_ --lib`

Expected: compilation fails because `queued_changes` and `remove_queued_change` do not exist.

- [ ] **Step 3: Add lossless event fields needed for replay**

Replace the two lossy event payloads in `src/model.rs` so replay has the original model data:

```rust
NoteAdded { node_id: NodeId, note: Note },
NodeAdded { node: Node },
```

Update `Store::add_note` to construct its `Note` once, clone it into the document and event, and update `Store::add_user_node` to clone its new `Node` into `NodeAdded`. Update every model/store/export test pattern to match the new fields. `describe_event` must use `note.text` and `node.label`.

- [ ] **Step 4: Add pending-baseline and blocked-queue persistence**

Add these types beside `Snapshot` in `src/store.rs`:

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueuedChange {
    pub event: DecisionEvent,
    #[serde(default)]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueueEditTarget {
    pub node_id: Option<NodeId>,
    pub choice_id: Option<ChoiceId>,
}
```

Add `#[serde(default)] pub pending_base: Option<SessionDoc>` and `#[serde(default)] pub blocked_changes: Vec<QueuedChange>` to `SessionState`. In `push_undo`, set `pending_base` to the pre-action document whenever the undelivered tail is empty. In `push_event`, set it to `s.doc.clone()` only for a first no-document action such as `FlushRequested`. Clear `pending_base` after a successful `try_deliver_locked`, and clear both fields in `clear_session` through the existing default replacement.

- [ ] **Step 5: Implement deterministic replay and removal**

Add `replay_event(doc: &mut SessionDoc, event: &DecisionEvent) -> Result<(), String>` in `src/store.rs`. Match every `DecisionKind` exactly:

```rust
DecisionKind::OptionSelected { node_id, choice_id, option_id, .. } => {
    let choice = doc.node_mut(node_id)
        .ok_or_else(|| format!("node {node_id} no longer exists"))?
        .choices.iter_mut().find(|choice| choice.id == *choice_id)
        .ok_or_else(|| format!("choice {choice_id} no longer exists"))?;
    if !choice.options.iter().any(|option| option.id == *option_id) {
        return Err(format!("option {option_id} no longer exists"));
    }
    choice.selected = Some(option_id.clone());
    choice.status = ChoiceStatus::Decided;
}
DecisionKind::ChoiceDismissed { node_id, choice_id, .. } => {
    let choice = doc.node_mut(node_id)
        .ok_or_else(|| format!("node {node_id} no longer exists"))?
        .choices.iter_mut().find(|choice| choice.id == *choice_id)
        .ok_or_else(|| format!("choice {choice_id} no longer exists"))?;
    choice.selected = None;
    choice.status = ChoiceStatus::Dismissed;
}
DecisionKind::NoteAdded { node_id, note } => {
    doc.node_mut(node_id)
        .ok_or_else(|| format!("node {node_id} no longer exists"))?
        .notes.push(note.clone());
}
DecisionKind::FlushRequested { .. } => {}
DecisionKind::NodeAdded { node } => {
    if doc.node(&node.id).is_some() { return Err(format!("node {} already exists", node.id)); }
    doc.nodes.push(node.clone());
}
DecisionKind::NodeEdited { node_id, label, node_kind, description } => {
    let node = doc.node_mut(node_id).ok_or_else(|| format!("node {node_id} no longer exists"))?;
    node.label = label.clone();
    node.kind = *node_kind;
    node.description = description.clone();
}
DecisionKind::NodeDeleted { node_id } => {
    if doc.node(node_id).is_none() { return Err(format!("node {node_id} no longer exists")); }
    doc.nodes.retain(|node| node.id != *node_id);
    doc.edges.retain(|edge| edge.from != *node_id && edge.to != *node_id);
}
DecisionKind::RemovalRequested { node_id } => {
    doc.node_mut(node_id).ok_or_else(|| format!("node {node_id} no longer exists"))?.status = ElementStatus::Removed;
}
DecisionKind::EdgeAdded { from, to, edge_kind } => {
    if doc.node(from).is_none() || doc.node(to).is_none() { return Err(format!("edge {from} → {to} has a missing endpoint")); }
    if doc.edges.iter().any(|edge| edge.key() == (from, to, *edge_kind)) { return Err(format!("edge {from} → {to} already exists")); }
    doc.edges.push(Edge { from: from.clone(), to: to.clone(), kind: *edge_kind, label: None, status: ElementStatus::Proposed, origin: Origin::User });
}
DecisionKind::EdgeDeleted { from, to, edge_kind } => {
    if !doc.edges.iter().any(|edge| edge.key() == (from, to, *edge_kind)) { return Err(format!("edge {from} → {to} no longer exists")); }
    doc.edges.retain(|edge| edge.key() != (from, to, *edge_kind));
}
```

Implement `remove_queued_change` inside `Store::mutate`: take `pending_base`, remove the requested tail event by sequence, replay the survivors in order, retain successful replayed events as the new tail, push failures into `blocked_changes` with the returned reason, renumber tail sequences from `delivery_cursor + 1`, preserve the previous document revision, clear undo/redo, and add an activity receipt. Return the node and optional choice derived from the removed event. Implement `remove_blocked_change` as a direct removal from `blocked_changes` that returns the same target. `queued_changes` returns the undelivered tail as `QueuedChange { blocked_reason: None }` followed by `blocked_changes`.

- [ ] **Step 6: Run focused store tests and the library suite**

Run: `cargo test store::tests::removing_ --lib && cargo test --lib`

Expected: PASS; no existing persistence, delivery, undo, or export test regresses.

- [ ] **Step 7: Commit the replay layer**

```bash
git add src/model.rs src/store.rs src/export.rs
git commit -m "feat(queue): support queued change removal"
```

### Task 2: Render a dedicated, hideable queue panel

**Files:**
- Create: `src/ui/queued_changes.rs`
- Modify: `src/ui/mod.rs:7-17`
- Modify: `src/ui/app.rs:13-116`
- Modify: `src/ui/topbar.rs:10-305`
- Modify: `assets/main.css:372-430, 1030-1053`

**Interfaces:**
- Consumes `Store::queued_changes` and `Store::remove_queued_change` from Task 1.
- Produces `QueuedChangesPanel { on_close: EventHandler<()>, selected: Signal<Option<NodeId>> }`.
- `TopBar` consumes `queued_changes_open: Signal<bool>` and toggles it from the queue segment.

- [ ] **Step 1: Write failing pure display tests for queue rows**

In the new `src/ui/queued_changes.rs`, expose a small testable formatter:

```rust
fn queued_change_label(doc: &SessionDoc, change: &QueuedChange) -> String {
    change.blocked_reason.as_ref().map_or_else(
        || crate::export::describe_event(doc, &change.event),
        |reason| format!("{} — blocked: {reason}", crate::export::describe_event(doc, &change.event)),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn blocked_changes_explain_why_they_will_not_send() {
        let doc = SessionDoc::default();
        let change = QueuedChange {
            event: DecisionEvent {
                seq: 1,
                at: Utc::now(),
                kind: DecisionKind::NodeEdited {
                    node_id: NodeId::from("widget"),
                    label: "Widget".into(),
                    node_kind: NodeKind::Component,
                    description: String::new(),
                },
            },
            blocked_reason: Some("node widget no longer exists".into()),
        };
        assert_eq!(queued_change_label(&doc, &change), "edited “Widget” — blocked: node widget no longer exists");
    }
}
```

Write the test with a concrete `DecisionKind::NodeEdited` event and assert both the node label and `blocked: node widget no longer exists` appear.

- [ ] **Step 2: Run the formatter test and verify it fails**

Run: `cargo test queued_changes::tests::blocked_changes_explain --lib`

Expected: compilation fails because the module and formatter do not exist.

- [ ] **Step 3: Add the queue panel component**

Create `QueuedChangesPanel` following `Timeline`'s `aside { class: "panel timeline" }` structure. Read `doc` and `meta` so updates are live, call `store.queued_changes()`, and render:

```rust
aside { class: "panel queued-changes",
    div { class: "panel-head",
        h2 { "Queued changes" }
        button { class: "ctl-btn", title: "Close", onclick: move |_| on_close.call(()), "✕" }
    }
    if changes.is_empty() { p { class: "panel-desc", "Nothing is queued for the agent." } }
    for change in changes {
        div { class: if change.blocked_reason.is_some() { "queue-row queue-blocked" } else { "queue-row" },
            span { class: "timeline-time", "{change.event.at.format(\"%H:%M\")}" }
            span { class: "timeline-text", "{queued_change_label(&doc.read(), &change)}" }
            button { class: "ctl-btn", title: "Edit queued change", "Edit" }
            button { class: "ctl-btn", title: "Remove queued change", "✕" }
        }
    }
}
```

For either row action, call `remove_queued_change` for a pending event and `remove_blocked_change` for a blocked event. When Edit succeeds, clear the queue panel, assign its returned `node_id` to `selected`, and leave the existing ChoicePanel or node edit workflow as the replacement surface. For note events this opens the node's note composer; for added/deleted edges it opens the source node's Connect/Delete controls; and for a queued comment the existing top-bar message composer remains the replacement surface. A blocked event uses the same Edit action, so it can always be removed and replaced rather than becoming a dead-end row.

- [ ] **Step 4: Wire open state and a real queue button**

In `App`, create `let mut queued_changes_open: Signal<bool> = use_signal(|| false);`, pass it to `TopBar`, and render `QueuedChangesPanel` after the Timeline branch. Use mutually exclusive branches so a selected node, Timeline, diff, or queue panel cannot coexist in the right slot. In `TopBar`, replace the queued `span` with:

```rust
button {
    class: if queued_changes_open() { "seg seg-queued btn-armed" } else { "seg seg-queued" },
    aria_label: "{m.undelivered} queued changes",
    title: "Review, edit, or remove queued changes",
    onclick: move |_| queued_changes_open.toggle(),
    "{m.undelivered}"
    span { class: "seg-word", " queued" }
}
```

The handler must clear `selected` and set `timeline_open` false before opening, matching the existing Timeline behavior.

- [ ] **Step 5: Add minimal panel styles and run UI-adjacent tests**

Add `.queue-row` as a three-column grid (time, content, actions), `.queue-blocked` with the existing removed-status color/background, and `.queue-row .ctl-btn` sizing that does not overflow the 360px panel. Reuse existing `timeline-*`, `panel-*`, and `ctl-btn` styles rather than adding a modal or new dependency.

Run: `cargo test queued_changes --lib && cargo fmt --check`

Expected: PASS and no formatting changes needed.

- [ ] **Step 6: Commit the panel**

```bash
git add src/ui/queued_changes.rs src/ui/mod.rs src/ui/app.rs src/ui/topbar.rs assets/main.css
git commit -m "feat(queue): add queued changes panel"
```

### Task 3: Verify end-to-end queue and delivery behavior

**Files:**
- Modify: `src/store.rs:1640-1780`
- Modify: `src/ui/queued_changes.rs:1-160`

**Interfaces:**
- Consumes the queue manager and panel from Tasks 1–2.
- Produces regression coverage for pending, blocked, and delivered queue items.

- [ ] **Step 1: Add a failing delivery regression test**

Add this to `src/store.rs` tests:

```rust
#[test]
fn removed_queue_items_are_never_delivered() {
    let store = demo_store();
    pick_first_choice(&store);
    store.add_note(&NodeId::from("sync-engine"), "keep cache local".into()).unwrap();
    store.remove_queued_change(1).unwrap();
    store.request_flush(None);

    let delivered = store.try_deliver().unwrap();

    assert_eq!(delivered.len(), 1);
    assert!(matches!(delivered[0].kind, DecisionKind::NoteAdded { .. }));
}
```

- [ ] **Step 2: Run it and verify the expected initial failure**

Run: `cargo test store::tests::removed_queue_items_are_never_delivered --lib`

Expected: FAIL before Task 1 removal/replay implementation is present; PASS after the full feature exists.

- [ ] **Step 3: Run the complete verification set**

Run:

```bash
cargo fmt --check
cargo test --lib
cargo test
```

Expected: all commands exit 0. If an existing session fixture serializes `NoteAdded` or `NodeAdded`, update it to the new payload shape rather than weakening serialization coverage.

- [ ] **Step 4: Inspect the final diff**

Run: `git diff HEAD~2..HEAD --check && git status --short`

Expected: no whitespace errors and no untracked/generated artifacts.

- [ ] **Step 5: Commit final regression coverage**

```bash
git add src/store.rs src/ui/queued_changes.rs
git commit -m "test(queue): cover queued delivery"
```
