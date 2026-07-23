# Swimlane Drag-Drop Containment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users create empty swimlanes, drag cards between lanes or out of them, and always see which lane a card belongs to.

**Architecture:** Swimlane membership stays as `node.lane` in the doc. A new per-session view-state list `SessionState.declared_lanes` (mirroring `collapsed_groups`) holds user-created lanes so empty ones can exist. Layout emits a band for every declared/referenced lane, grows the drawn band to enclose pinned members, and keeps a stable base "hit" strip for drop targeting. Dropping a card sets its lane by geometry; a `+ Swimlane` button, inline label rename, and `×` delete manage the registry.

**Tech Stack:** Rust, Dioxus 0.7 (desktop/WebView2), inline `#[cfg(test)]` unit tests, CSS in `assets/main.css` with contract tests in `src/theme.rs`.

## Global Constraints

- Rust edition/toolchain already pinned by the repo; do not bump.
- All checks must pass: `cargo test --all-targets --locked`, `cargo fmt --all -- --check`, `cargo clippy --all-targets --locked -- -D warnings`.
- No new dependencies.
- `SessionDoc` schema is NOT changed. Lanes-as-registry is view-state only (`SessionState`), invisible to agents, never in the doc.
- Follow existing patterns: `set_position`/`toggle_group_collapsed` for view mutations, `compute_collapsed` for layout view-args, `assert_block_contains` for CSS contracts.
- `CARD_WIDTH = 260.0`, `LANE_TITLE_H = 36.0`, `LANE_CARD_PAD = 20.0` (existing constants in `src/layout.rs`).

---

## File Structure

- `src/store.rs` — add `declared_lanes` to `SessionState` + `UiMeta` + `snapshot_meta`; add `set_lane`, `add_lane`, `rename_lane`, `delete_lane`, and free helpers `unique_lane_name`, `dedupe_in_place`.
- `src/layout.rs` — add `hit: Rect` to `LaneBand`; add `compute_view(doc, collapsed, declared)`; change `compute_inner`/`place_laned` to take the declared-lane list; add pure `lane_at` helper.
- `src/ui/app.rs` — the layout `Memo` passes `meta.declared_lanes` to `compute_view`.
- `src/ui/canvas.rs` — on drop, re-parent via `lane_at`+`set_lane`; drag-target highlight signal; band render adds `drop-target` class, inline-editable label, `×` delete; `+ Swimlane` control.
- `assets/main.css` — `.swimlane.drop-target`, label `pointer-events`, `.swimlane-label input`, `.lane-delete`, `.add-lane-btn`.
- `src/theme.rs` — CSS contract assertions for the new selectors.

---

## Task 1: Store lane registry + membership API

**Files:**
- Modify: `src/store.rs` (`SessionState` ~line 94-139, `UiMeta` ~line 142-151, `snapshot_meta` ~line 238-248, new methods near `set_position` ~line 299, free helpers near `push_undo` ~line 1343)
- Test: `src/store.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `SessionState.declared_lanes: Vec<String>`
  - `UiMeta.declared_lanes: Vec<String>`
  - `Store::set_lane(&self, node: &NodeId, lane: Option<String>)` — no checkpoint, trims, empty→None.
  - `Store::add_lane(&self) -> String` — appends a unique default name, returns it.
  - `Store::rename_lane(&self, old: &str, new: &str)` — rewrites declared entry + member `node.lane`; blank/no-op ignored.
  - `Store::delete_lane(&self, label: &str)` — removes declared entry + clears member `node.lane`.
- Consumes: existing `mutate`, `NodeId`, `Point` test helpers `demo_store()`, `Store::with_doc`, `doc`.

- [ ] **Step 1: Write failing tests**

Add to `src/store.rs` `#[cfg(test)] mod tests` (use existing `demo_store()`; node ids `redis`, `sync-engine` exist in the demo doc):

```rust
#[test]
fn set_lane_sets_and_clears_membership() {
    let store = demo_store();
    store.set_lane(&NodeId::from("redis"), Some("  data  ".into()));
    assert_eq!(
        store.snapshot_doc().node(&NodeId::from("redis")).unwrap().lane.as_deref(),
        Some("data"),
        "lane is trimmed and set"
    );
    store.set_lane(&NodeId::from("redis"), Some("   ".into()));
    assert_eq!(
        store.snapshot_doc().node(&NodeId::from("redis")).unwrap().lane,
        None,
        "blank lane clears membership"
    );
}

#[test]
fn set_lane_folds_into_a_drag_checkpoint() {
    let store = demo_store();
    store.checkpoint_position(&NodeId::from("redis")); // drag start
    store.set_position(&NodeId::from("redis"), Point { x: 5.0, y: 5.0 });
    store.set_lane(&NodeId::from("redis"), Some("data".into()));
    assert!(store.undo(), "one undo available for the drag");
    let redis = store.snapshot_doc().node(&NodeId::from("redis")).unwrap().clone();
    assert_eq!(redis.lane, None, "undo reverts the lane change");
}

#[test]
fn add_lane_appends_unique_names() {
    let store = demo_store();
    assert_eq!(store.add_lane(), "New lane");
    assert_eq!(store.add_lane(), "New lane 2");
    assert_eq!(store.add_lane(), "New lane 3");
    assert_eq!(
        store.snapshot_meta().declared_lanes,
        vec!["New lane", "New lane 2", "New lane 3"]
    );
}

#[test]
fn rename_lane_rewrites_registry_and_members() {
    let store = demo_store();
    store.set_lane(&NodeId::from("redis"), Some("data".into()));
    store.add_lane(); // "New lane"
    store.rename_lane("New lane", "data"); // collides → merges
    store.rename_lane("data", "backend");
    assert_eq!(store.snapshot_meta().declared_lanes, vec!["backend"]);
    assert_eq!(
        store.snapshot_doc().node(&NodeId::from("redis")).unwrap().lane.as_deref(),
        Some("backend")
    );
    store.rename_lane("backend", "  "); // blank ignored
    assert_eq!(store.snapshot_meta().declared_lanes, vec!["backend"]);
}

#[test]
fn delete_lane_removes_registry_and_clears_members() {
    let store = demo_store();
    store.set_lane(&NodeId::from("redis"), Some("data".into()));
    store.add_lane();
    store.rename_lane("New lane", "data");
    store.delete_lane("data");
    assert!(store.snapshot_meta().declared_lanes.is_empty());
    assert_eq!(
        store.snapshot_doc().node(&NodeId::from("redis")).unwrap().lane,
        None
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib store:: -- set_lane add_lane rename_lane delete_lane`
Expected: FAIL — `no method named set_lane` / `no field declared_lanes`.

- [ ] **Step 3: Add the `declared_lanes` field + UiMeta wiring**

In `SessionState` (after `collapsed_groups`, ~line 129):

```rust
    /// User-declared swimlanes, in display order. View state: persisted per
    /// session, never part of the doc, invisible to agents — mirrors
    /// `collapsed_groups`. A lane may exist here with no member nodes (an
    /// empty lane awaiting cards).
    #[serde(default)]
    pub declared_lanes: Vec<String>,
```

In `UiMeta` (after `collapsed_groups`, ~line 148):

```rust
    pub declared_lanes: Vec<String>,
```

In `snapshot_meta` (after `collapsed_groups: s.collapsed_groups.clone(),`, ~line 244):

```rust
            declared_lanes: s.declared_lanes.clone(),
```

- [ ] **Step 4: Add the store methods**

Insert after `set_position` (~line 305):

```rust
    /// Re-parent a node into a swimlane (or clear it with `None`). Called on
    /// drag-drop; like `set_position` it does not checkpoint — the drag's
    /// start-of-drag checkpoint already captured the old lane, so a single
    /// undo reverts the whole drag. Blank names normalize to `None`.
    pub fn set_lane(&self, node: &NodeId, lane: Option<String>) {
        let lane = lane.and_then(|l| {
            let t = l.trim();
            (!t.is_empty()).then(|| t.to_owned())
        });
        self.mutate(|s| {
            if let Some(n) = s.doc.node_mut(node) {
                n.lane = lane;
            }
        });
    }

    /// Append a new empty swimlane with a unique default name and return it.
    /// View state only (like `toggle_group_collapsed`): no decision event, no
    /// undo entry, invisible to agents.
    pub fn add_lane(&self) -> String {
        self.mutate(|s| {
            let name = unique_lane_name(&s.declared_lanes);
            s.declared_lanes.push(name.clone());
            name
        })
    }

    /// Rename a swimlane: rewrite the declared entry and `lane` on every
    /// member node so membership follows. A blank or unchanged name is a
    /// no-op; a name that collides with an existing lane merges into it.
    pub fn rename_lane(&self, old: &str, new: &str) {
        let new = new.trim().to_owned();
        if new.is_empty() || new == old {
            return;
        }
        self.mutate(|s| {
            for slot in s.declared_lanes.iter_mut() {
                if slot == old {
                    *slot = new.clone();
                }
            }
            dedupe_in_place(&mut s.declared_lanes);
            for n in s.doc.nodes.iter_mut() {
                if n.lane.as_deref() == Some(old) {
                    n.lane = Some(new.clone());
                }
            }
        });
    }

    /// Delete a swimlane: remove the declared entry and clear `lane` on its
    /// members (cards fall back to the default lane). View state + doc, but
    /// no undo entry — consistent with the view-state model.
    pub fn delete_lane(&self, label: &str) {
        self.mutate(|s| {
            s.declared_lanes.retain(|l| l != label);
            for n in s.doc.nodes.iter_mut() {
                if n.lane.as_deref() == Some(label) {
                    n.lane = None;
                }
            }
        });
    }
```

Add free helpers near `push_undo` (~line 1343):

```rust
/// First free name in the `New lane`, `New lane 2`, … series.
fn unique_lane_name(existing: &[String]) -> String {
    let base = "New lane";
    if !existing.iter().any(|l| l == base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|c| !existing.iter().any(|l| l == c))
        .expect("an infinite range always yields a free name")
}

/// Drop later duplicates, keeping first-appearance order.
fn dedupe_in_place(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|x| seen.insert(x.clone()));
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib store::`
Expected: PASS (new tests + existing store tests).

- [ ] **Step 6: Commit**

```bash
git add src/store.rs
git commit -m "feat(store): swimlane registry + drag-drop membership API"
```

---

## Task 2: Layout — declared lanes, grown bands, stable hit-strip, lane_at

**Files:**
- Modify: `src/layout.rs` (`LaneBand` ~line 72-75, `compute`/`compute_collapsed` ~line 189-197, `compute_inner` ~line 311-368, `place_laned` ~line 574-694, new `compute_view` + `lane_at`)
- Modify: `src/ui/app.rs` (layout `Memo` ~line 61-65)
- Test: `src/layout.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `SessionState.declared_lanes` (via `meta.declared_lanes` in app).
- Produces:
  - `LaneBand { label: String, rect: Rect, hit: Rect }` — `rect` drawn (grown), `hit` stable base strip.
  - `layout::compute_view(doc: &SessionDoc, collapsed: &BTreeSet<String>, declared: &[String]) -> Layout`
  - `layout::lane_at(lanes: &[LaneBand], x: f64, y: f64) -> Option<String>`
  - `compute(doc)` and `compute_collapsed(doc, collapsed)` remain as back-compat wrappers passing `declared = &[]`.

- [ ] **Step 1: Write failing tests**

Add to `src/layout.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn declared_empty_lane_gets_a_band_in_declared_order() {
    // No node references "review"; it exists only in the declared list.
    let mut d = doc(&["a", "b"], &[("a", "b")]);
    d.node_mut(&NodeId::from("a")).unwrap().lane = Some("build".into());
    let layout = compute_view(
        &d,
        &std::collections::BTreeSet::new(),
        &["review".to_owned(), "build".to_owned()],
    );
    let labels: Vec<&str> = layout.lanes.iter().map(|l| l.label.as_str()).collect();
    // Declared order wins; the empty "review" lane still renders a band.
    assert_eq!(labels, vec!["review", "build"]);
    let review = layout.lanes.iter().find(|l| l.label == "review").unwrap();
    assert!(review.rect.h > 0.0, "empty declared lane has a visible band");
}

#[test]
fn referenced_only_lane_follows_declared_lanes() {
    let mut d = doc(&["a", "b"], &[("a", "b")]);
    d.node_mut(&NodeId::from("a")).unwrap().lane = Some("adhoc".into()); // not declared
    let layout = compute_view(
        &d,
        &std::collections::BTreeSet::new(),
        &["build".to_owned()], // declared but unreferenced
    );
    let labels: Vec<&str> = layout.lanes.iter().map(|l| l.label.as_str()).collect();
    assert_eq!(labels, vec!["build", "adhoc"], "declared before referenced-only");
}

#[test]
fn pinned_card_is_enclosed_by_its_grown_band() {
    let mut d = doc(&["a", "b"], &[("a", "b")]);
    d.node_mut(&NodeId::from("a")).unwrap().lane = Some("build".into());
    d.node_mut(&NodeId::from("b")).unwrap().lane = Some("build".into());
    // Pin b far below where the auto strip would sit.
    d.node_mut(&NodeId::from("b")).unwrap().position = Some(Point { x: 40.0, y: 400.0 });
    let layout = compute_view(&d, &std::collections::BTreeSet::new(), &["build".to_owned()]);
    let band = layout.lanes.iter().find(|l| l.label == "build").unwrap();
    let rb = layout.rects[&NodeId::from("b")];
    assert!(
        band.rect.y <= rb.y && band.rect.y + band.rect.h >= rb.y + rb.h,
        "grown band {:?} must enclose pinned card {:?}",
        band.rect, rb
    );
    // The hit strip stays put (does not chase the pinned card down to y=400).
    assert!(band.hit.y + band.hit.h < 300.0, "hit strip is the stable base band");
}

#[test]
fn lane_at_hits_bands_and_misses_the_gap() {
    let mut d = doc(&["a", "b"], &[("a", "b")]);
    d.node_mut(&NodeId::from("a")).unwrap().lane = Some("build".into());
    let layout = compute_view(&d, &std::collections::BTreeSet::new(), &["build".to_owned()]);
    let band = layout.lanes.iter().find(|l| l.label == "build").unwrap();
    let cx = band.hit.x + band.hit.w / 2.0;
    let cy = band.hit.y + band.hit.h / 2.0;
    assert_eq!(lane_at(&layout.lanes, cx, cy).as_deref(), Some("build"));
    assert_eq!(lane_at(&layout.lanes, cx, band.hit.y - 5000.0), None, "far away = no lane");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib layout:: -- declared_empty referenced_only pinned_card lane_at`
Expected: FAIL — `cannot find function compute_view` / `no field hit`.

- [ ] **Step 3: Extend `LaneBand`**

Replace the `LaneBand` struct (~line 72-75):

```rust
pub struct LaneBand {
    pub label: String,
    /// The drawn band: the base strip grown to enclose the lane's pinned
    /// members, so a dropped card is visibly contained.
    pub rect: Rect,
    /// The stable base strip (independent of pinned cards). Drop hit-testing
    /// and the drag-target highlight use this so the target does not chase a
    /// card being dragged.
    pub hit: Rect,
}
```

- [ ] **Step 4: Thread the declared list through the layout entry points**

Replace `compute` / `compute_collapsed` header (~line 189-200) so `compute_collapsed` delegates to a new `compute_view`, and `compute` passes empty view-state:

```rust
pub fn compute(doc: &SessionDoc) -> Layout {
    compute_view(doc, &BTreeSet::new(), &[])
}

/// Layout with named groups collapsed. See [`compute_view`].
pub fn compute_collapsed(doc: &SessionDoc, collapsed: &BTreeSet<String>) -> Layout {
    compute_view(doc, collapsed, &[])
}

/// Layout with collapsed groups and user-declared swimlanes. `declared` lists
/// lanes the user created (including empty ones) in display order; lanes only
/// referenced by `node.lane` are appended after, in first-appearance order.
pub fn compute_view(
    doc: &SessionDoc,
    collapsed: &BTreeSet<String>,
    declared: &[String],
) -> Layout {
    if collapsed.is_empty() {
        return compute_inner(doc, &[], declared);
    }
```

Then in the existing `compute_collapsed` body that followed (the group-collapsing logic through the final `compute_inner(&syn_doc, &counts)` call at ~line 294), keep it verbatim but:
- it is now the tail of `compute_view` (the `if collapsed.is_empty()` early-return above replaces the old `return compute_inner(doc, &[]);` at ~line 199),
- change the final call `compute_inner(&syn_doc, &counts)` to `compute_inner(&syn_doc, &counts, declared)` so a collapsed view still shows swimlanes.

There are exactly two `compute_inner` call sites — the early return (now in `compute_view`) and the `&syn_doc` call — and **both pass `declared`**, not `&[]`. Update `compute_inner`'s signature (~line 311) and its `place`/`place_laned` branch (~line 343-347):

```rust
fn compute_inner(doc: &SessionDoc, counts: &[usize], declared: &[String]) -> Layout {
```

```rust
    let (rects, lanes) = if declared.is_empty() && !doc.nodes.iter().any(|n| n.lane.is_some()) {
        (place(doc, &ranks, &order), Vec::new())
    } else {
        place_laned(doc, &ranks, &order, declared)
    };
```

- [ ] **Step 5: Rewrite lane ordering + emit empty bands + grow rect in `place_laned`**

Change `place_laned` signature (~line 574):

```rust
fn place_laned(
    doc: &SessionDoc,
    _ranks: &[usize],
    order: &[Vec<usize>],
    declared: &[String],
) -> (HashMap<NodeId, Rect>, Vec<LaneBand>) {
```

Replace the lane-order construction block (~line 584-593) so declared lanes come first, then referenced-only, then the default `""` last:

```rust
    // Lane order: declared lanes first (in the user's order), then lanes only
    // referenced by nodes (first appearance), then the unlabeled default last.
    let mut lane_order: Vec<String> = Vec::new();
    let mut lane_index: HashMap<String, usize> = HashMap::new();
    let mut push_lane = |order: &mut Vec<String>, index: &mut HashMap<String, usize>, key: String| {
        if !index.contains_key(&key) {
            index.insert(key.clone(), order.len());
            order.push(key);
        }
    };
    for label in declared {
        push_lane(&mut lane_order, &mut lane_index, label.clone());
    }
    for i in 0..doc.nodes.len() {
        let key = lane_key(i);
        if !key.is_empty() {
            push_lane(&mut lane_order, &mut lane_index, key);
        }
    }
    // The default lane is appended only if some node has no lane.
    if doc.nodes.iter().any(|n| n.lane.is_none()) {
        push_lane(&mut lane_order, &mut lane_index, String::new());
    }
```

The `top_pad`, `lane_stack_h`, `lane_y`, `band_h`, per-rank cursor, and pinned-rect placement blocks (~line 596-676) stay as-is — they already key off `lane_index[&lane_key(i)]` and skip `position.is_some()` nodes. An empty declared lane naturally yields `lane_stack_h = 0` and `band_h = LANE_TITLE_H + LANE_CARD_PAD` (56px), a visible band.

Replace the final `LaneBand` emission block (~line 678-692) to compute both the base strip and a grown rect:

```rust
    let mut lanes: Vec<LaneBand> = Vec::new();
    for (li, label) in lane_order.iter().enumerate() {
        if label.is_empty() {
            continue; // the default lane draws no band
        }
        let base = Rect {
            x: -LANE_HPAD,
            y: lane_y[li],
            w: graph_w + 2.0 * LANE_HPAD,
            h: band_h[li],
        };
        // Grow the drawn band to enclose this lane's pinned members.
        let mut grown = base;
        for (i, node) in doc.nodes.iter().enumerate() {
            if node.position.is_some() && lane_key(i) == *label {
                grown = union_rect(grown, rects[&node.id]);
            }
        }
        lanes.push(LaneBand {
            label: label.clone(),
            rect: grown,
            hit: base,
        });
    }
    (rects, lanes)
```

(`union_rect` already exists in this file — it is used for `bounds`.)

- [ ] **Step 6: Add the `lane_at` helper**

Add near `place_laned` (module-public):

```rust
/// The lane whose stable hit-strip contains `(x, y)` in plane coords, or
/// `None` if the point is outside every band (the drop-outside case).
pub fn lane_at(lanes: &[LaneBand], x: f64, y: f64) -> Option<String> {
    lanes
        .iter()
        .find(|l| {
            x >= l.hit.x && x < l.hit.x + l.hit.w && y >= l.hit.y && y < l.hit.y + l.hit.h
        })
        .map(|l| l.label.clone())
}
```

- [ ] **Step 7: Wire the app's layout Memo to pass declared lanes**

In `src/ui/app.rs`, replace the layout `Memo` (~line 61-65):

```rust
    let layout: Memo<Layout> = use_memo(move || {
        let m = meta.read();
        let collapsed: std::collections::BTreeSet<String> =
            m.collapsed_groups.iter().cloned().collect();
        layout::compute_view(&doc.read(), &collapsed, &m.declared_lanes)
    });
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test --lib layout::`
Expected: PASS. Also confirm existing `swimlanes_confine_cards_to_stacked_bands` still passes (it uses `compute` → declared `&[]`, referenced order `frontend`, `backend`).

- [ ] **Step 9: Build the whole crate (catch call-site breakage)**

Run: `cargo build --all-targets --locked`
Expected: PASS. If any `compute_inner(...)` call errors on arity, add the `&[]` third arg.

- [ ] **Step 10: Commit**

```bash
git add src/layout.rs src/ui/app.rs
git commit -m "feat(layout): declared lanes, grown bands, stable hit-strip, lane_at"
```

---

## Task 3: Canvas drop re-parenting + drag-target highlight

**Files:**
- Modify: `src/ui/canvas.rs` (signals ~line 110-124; `onmousemove` DragNode branch ~line 443-456; `onmouseup` ~line 461-506; `onmouseleave` ~line 508-514; band render ~line 565-572)
- Modify: `assets/main.css` (swimlane section ~line 1815-1844)
- Test: `src/theme.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `store.set_lane`, `layout::lane_at`, `LaneBand.hit`, `crate::layout::CARD_WIDTH`.
- Produces: `.swimlane.drop-target` styling; a `drop_lane: Signal<Option<String>>` local to `Canvas`.

- [ ] **Step 1: Write failing CSS contract test**

Add to `src/theme.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn dragged_card_highlights_its_target_lane() {
    // The drop-target band is visibly distinct via the accent color.
    assert_block_contains(".swimlane.drop-target", "var(--accent)");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib theme:: -- dragged_card_highlights`
Expected: FAIL — `block_for_in` panics: selector `.swimlane.drop-target` not found.

- [ ] **Step 3: Add the drop-target CSS**

In `assets/main.css`, after the `.swimlane:nth-child(even)` rule (~line 1828):

```css
.swimlane.drop-target {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  border-color: var(--accent);
}
```

- [ ] **Step 4: Add the `drop_lane` signal**

In `Canvas`, next to the other `use_signal` declarations (~line 124):

```rust
    // The lane the currently dragged card would drop into (drag feedback).
    let mut drop_lane: Signal<Option<String>> = use_signal(|| None);
```

- [ ] **Step 5: Set the highlight during a node drag**

In the `onmousemove` `Some(Gesture::DragNode { .. })` branch, after `store.set_position(id, Point { x: nx, y: ny });` (~line 448), add:

```rust
                                let l = layout.read();
                                let (cx, cy) =
                                    (nx + crate::layout::CARD_WIDTH / 2.0, ny + l.rects.get(id).map_or(40.0, |r| r.h) / 2.0);
                                drop_lane.set(crate::layout::lane_at(&l.lanes, cx, cy));
```

- [ ] **Step 6: Re-parent on drop and clear the highlight**

In `onmouseup`, replace the terminal cleanup (the block that currently does `ghost_to.set(None); gesture.set(GestureState::default());` ~line 504-505) with:

```rust
                // A node drag that actually moved re-parents by geometry:
                // dropped inside a band → that lane; outside every band → none.
                if let Some(Gesture::DragNode { moved: true, .. }) = state.gesture
                    && let Some(id) = &state.node
                {
                    let l = layout.read();
                    let target = l.rects.get(id).map(|r| {
                        crate::layout::lane_at(&l.lanes, r.x + r.w / 2.0, r.y + r.h / 2.0)
                    });
                    if let Some(target) = target {
                        store.set_lane(id, target);
                    }
                }
                ghost_to.set(None);
                drop_lane.set(None);
                gesture.set(GestureState::default());
```

(Keep the earlier Pan/connect handling in `onmouseup` unchanged; only the final cleanup lines change. `store` is already cloned into this handler.)

Also clear the highlight in `onmouseleave` (~line 508-514), add:

```rust
                drop_lane.set(None);
```

- [ ] **Step 7: Apply the highlight class in the band render**

Replace the swimlane render loop (~line 565-572):

```rust
                for lane in l.lanes.iter() {
                    div {
                        key: "lane-{lane.label}",
                        class: if drop_lane().as_deref() == Some(lane.label.as_str()) {
                            "swimlane drop-target"
                        } else {
                            "swimlane"
                        },
                        style: "left: {lane.rect.x}px; top: {lane.rect.y}px; width: {lane.rect.w}px; height: {lane.rect.h}px;",
                        span { class: "swimlane-label", "{lane.label}" }
                    }
                }
```

- [ ] **Step 8: Run the CSS test + build**

Run: `cargo test --lib theme:: -- dragged_card_highlights && cargo build --all-targets --locked`
Expected: PASS + clean build.

- [ ] **Step 9: Commit**

```bash
git add src/ui/canvas.rs assets/main.css src/theme.rs
git commit -m "feat(canvas): drop-to-reparent swimlanes with drag-target highlight"
```

---

## Task 4: Lane management UI — add, rename, delete

**Files:**
- Modify: `src/ui/canvas.rs` (edit state ~line 124; band label render ~line 565-572; `canvas-controls` ~line 768-804)
- Modify: `assets/main.css` (swimlane label section ~line 1830-1844; controls section)
- Test: `src/theme.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `store.add_lane`, `store.rename_lane`, `store.delete_lane`.
- Produces: `.swimlane-label { pointer-events: auto }`, `.lane-delete`, `.add-lane-btn` styling; `editing_lane: Signal<Option<String>>` local to `Canvas`.

- [ ] **Step 1: Write failing CSS contract test**

Add to `src/theme.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn swimlane_labels_are_interactive() {
    // The label sits over an otherwise non-interactive band; its controls
    // must receive pointer events.
    assert_block_contains(".swimlane-label", "pointer-events: auto");
    assert!(!block_for(".add-lane-btn").trim().is_empty(), "add-lane control is styled");
    assert!(!block_for(".lane-delete").trim().is_empty(), "delete control is styled");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib theme:: -- swimlane_labels_are_interactive`
Expected: FAIL — `.swimlane-label` lacks `pointer-events: auto` / `.add-lane-btn` block not found.

- [ ] **Step 3: Add the editing signal**

In `Canvas`, next to `drop_lane` (~line 124):

```rust
    // The lane whose label is being renamed inline (its current name).
    let mut editing_lane: Signal<Option<String>> = use_signal(|| None);
```

- [ ] **Step 4: Render an editable label with a delete control**

Replace the band render loop from Task 3 Step 7 so the label supports rename + delete. `store` is already available in `Canvas` (`let store = use_store();` at top):

```rust
                for lane in l.lanes.iter() {
                    div {
                        key: "lane-{lane.label}",
                        class: if drop_lane().as_deref() == Some(lane.label.as_str()) {
                            "swimlane drop-target"
                        } else {
                            "swimlane"
                        },
                        style: "left: {lane.rect.x}px; top: {lane.rect.y}px; width: {lane.rect.w}px; height: {lane.rect.h}px;",
                        div { class: "swimlane-label",
                            onmousedown: move |ev| ev.stop_propagation(),
                            if editing_lane().as_deref() == Some(lane.label.as_str()) {
                                input {
                                    class: "lane-name-edit",
                                    value: "{lane.label}",
                                    autofocus: true,
                                    onmounted: move |ev| { let _ = ev.data().set_focus(true); },
                                    onkeydown: {
                                        let old = lane.label.clone();
                                        let store = store.clone();
                                        move |ev: KeyboardEvent| {
                                            if ev.key() == Key::Enter {
                                                let new = ev.data().value();
                                                store.rename_lane(&old, &new);
                                                editing_lane.set(None);
                                            } else if ev.key() == Key::Escape {
                                                editing_lane.set(None);
                                            }
                                        }
                                    },
                                    onblur: {
                                        let old = lane.label.clone();
                                        let store = store.clone();
                                        move |ev: FocusEvent| {
                                            store.rename_lane(&old, &ev.data().value());
                                            editing_lane.set(None);
                                        }
                                    },
                                }
                            } else {
                                span {
                                    class: "lane-name",
                                    ondoubleclick: {
                                        let label = lane.label.clone();
                                        move |_| editing_lane.set(Some(label.clone()))
                                    },
                                    "{lane.label}"
                                }
                                button {
                                    class: "lane-delete",
                                    title: "Delete swimlane",
                                    onclick: {
                                        let label = lane.label.clone();
                                        let store = store.clone();
                                        move |_| store.delete_lane(&label)
                                    },
                                    "×"
                                }
                            }
                        }
                    }
                }
```

Note: the `input`'s `value` binds the current label; on commit `rename_lane` is a no-op when unchanged. Confirm `FocusEvent`/`Key` are in scope (`dioxus::prelude::*` already imported; add `use dioxus::events::FocusEvent;` if the build reports it missing).

- [ ] **Step 5: Add the `+ Swimlane` button**

In the `canvas-controls` div, after the `⤢ fit` button (~line 804):

```rust
                button {
                    class: "ctl-btn add-lane-btn",
                    title: "Add a swimlane",
                    onclick: {
                        let store = store.clone();
                        move |_| {
                            let name = store.add_lane();
                            editing_lane.set(Some(name));
                        }
                    },
                    "+ swimlane"
                }
```

- [ ] **Step 6: Add the label/control CSS**

In `assets/main.css`, update `.swimlane-label` (~line 1830) to be interactive and a flex row, and add control styles:

```css
.swimlane-label {
  position: absolute;
  z-index: 4;
  top: 8px;
  left: 12px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 6px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--bg) 88%, transparent);
  font-family: var(--font-display);
  font-size: 14px;
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--text-dim);
  text-transform: uppercase;
  pointer-events: auto;
}

.lane-name-edit {
  font: inherit;
  text-transform: uppercase;
  color: var(--text);
  background: var(--bg-card);
  border: 1px solid var(--accent);
  border-radius: 4px;
  padding: 0 4px;
  width: 10ch;
}

.lane-delete {
  border: none;
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  font-size: 15px;
  line-height: 1;
  padding: 0 2px;
  opacity: 0.6;
}

.lane-delete:hover {
  opacity: 1;
  color: var(--danger, #e5484d);
}

.add-lane-btn {
  white-space: nowrap;
}
```

- [ ] **Step 7: Run the CSS test + build**

Run: `cargo test --lib theme:: -- swimlane_labels_are_interactive && cargo build --all-targets --locked`
Expected: PASS + clean build.

- [ ] **Step 8: Commit**

```bash
git add src/ui/canvas.rs assets/main.css src/theme.rs
git commit -m "feat(canvas): add/rename/delete swimlanes from the canvas"
```

---

## Task 5: Full verification pass

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Then: `cargo fmt --all -- --check`
Expected: PASS.

- [ ] **Step 2: Full test suite**

Run: `cargo test --all-targets --locked`
Expected: PASS.

- [ ] **Step 3: Clippy (deny warnings)**

Run: `cargo clippy --all-targets --locked -- -D warnings`
Expected: PASS. Fix any lint inline (e.g. a needless `clone`) and re-run.

- [ ] **Step 4: Manual smoke (if a desktop run is available)**

Launch the app, then verify: `+ swimlane` creates a named, empty band that starts in rename mode; typing + Enter renames it; dragging a card into the band sets its lane and the band grows to wrap it; the target band highlights mid-drag; dragging the card out to empty space clears its lane; `×` deletes the band and its cards fall out.

- [ ] **Step 5: Commit any fixups**

```bash
git add -A
git commit -m "chore: fmt + clippy fixups for swimlane drag-drop"
```

---

## Self-Review Notes

- **Spec coverage:** storage (Task 1 field + Task 2 threading), grown-band-contains-pins (Task 2), stable hit-strip + `lane_at` (Task 2/3), drop re-parent incl. drop-outside=None (Task 3), drag feedback (Task 3), add/rename/delete (Task 4), all four verification categories (Tasks 1-4 tests + Task 5). Out-of-scope items (reorder, color, agent-visible lanes) are not implemented — intentional.
- **Undo:** `set_lane` folds into the drag checkpoint (tested); registry ops are view-state and not undoable, matching `collapsed_groups`.
- **Type consistency:** `compute_view(doc, collapsed, declared)`, `lane_at(lanes, x, y)`, `LaneBand { label, rect, hit }`, `set_lane`/`add_lane`/`rename_lane`/`delete_lane` names are used identically across tasks.
- **`ponytail:` ceiling:** mid-drag, a card's old lane band stretches toward the cursor (drawn `rect` grows) until drop. Accepted as live feedback; if it reads as noise, exclude the actively-dragged node from band growth by threading its id into `place_laned`.
