# Swimlane Readability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep swimlane titles clear of cards and let hovered or selected cards rise above overlapping neighbors.

**Architecture:** Extend the existing pure lane-placement calculation with a labeled-lane-only 36px header inset while preserving the default lane's current 20px padding. Reuse the existing `.node-card:hover` and `.node-card.selected` states for CSS-only card ordering, and let the lane label participate in the canvas stacking context above cards.

**Tech Stack:** Rust layout logic and unit tests; Dioxus' existing class markup; CSS stacking; Cargo test, fmt, and clippy.

## Global Constraints

- Labeled swimlanes reserve exactly 36px above automatically placed cards.
- Preserve the existing 20px bottom padding and the default unlabeled lane's 20px top padding.
- Normal cards use z-index 1, hovered cards use 2, selected cards use 3, and lane labels use 4.
- Pinned cards retain their user-defined positions.
- Add no document schema, persistence, context-menu action, manual z-order control, component state, or dependency.
- Follow test-driven development: every production change follows a focused test that was observed failing for the expected reason.

---

### Task 1: Raise hovered and selected cards

**Files:**
- Modify: `src/theme.rs:193-206`
- Modify: `assets/main.css:1879-1909`

**Interfaces:**
- Consumes: existing `.node-card`, `.node-card:hover`, and `.node-card.selected` selectors emitted by `NodeCard`.
- Produces: CSS stacking contract `normal = 1`, `hover = 2`, `selected = 3`; no Rust runtime interface.

- [ ] **Step 1: Write the failing stylesheet contract test**

Add this test after `long_content_surfaces_have_overflow_contracts` in `src/theme.rs`:

```rust
#[test]
fn canvas_cards_raise_on_hover_and_selection() {
    assert_block_contains(".node-card", "z-index: 1");
    assert_block_contains(".node-card:hover", "z-index: 2");
    assert_block_contains(".node-card.selected", "z-index: 3");
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test theme::tests::canvas_cards_raise_on_hover_and_selection -- --exact
```

Expected: FAIL at `.node-card must contain 'z-index: 1'` because cards do not yet define stacking levels.

- [ ] **Step 3: Add the minimal stacking declarations**

Add `z-index: 1` immediately after `position: absolute` in `.node-card`:

```css
z-index: 1;
```

Add `z-index: 2` as the first declaration in `.node-card:hover`:

```css
z-index: 2;
```

Add `z-index: 3` as the first declaration in `.node-card.selected`:

```css
z-index: 3;
```

The selected block follows the hover block and has equal specificity, so a selected card remains at level 3 while hovered.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
cargo test theme::tests::canvas_cards_raise_on_hover_and_selection -- --exact
```

Expected: PASS.

- [ ] **Step 5: Commit the card stacking fix**

```bash
git add src/theme.rs assets/main.css
git commit -m "fix(canvas): raise active cards"
```

---

### Task 2: Reserve and foreground the swimlane title

**Files:**
- Modify: `src/layout.rs:565-678,1045-1074`
- Modify: `src/theme.rs:193-216`
- Modify: `assets/main.css:1595-1618`

**Interfaces:**
- Consumes: `place_laned(doc: &SessionDoc, _ranks: &[usize], order: &[Vec<usize>]) -> (HashMap<NodeId, Rect>, Vec<LaneBand>)` and existing `.swimlane`/`.swimlane-label` markup.
- Produces: module constants `LANE_CARD_PAD: f64 = 20.0` and `LANE_TITLE_H: f64 = 36.0`; labeled lanes place automatic cards at or below `band.y + LANE_TITLE_H`; label CSS level 4.

- [ ] **Step 1: Write the failing lane-clearance assertion**

In `swimlanes_confine_cards_to_stacked_bands` in `src/layout.rs`, add this assertion inside the existing `for id in ids` loop immediately after `let r = ...`:

```rust
assert!(
    r.y >= band.y + 36.0,
    "{id} overlaps the {label} title strip"
);
```

- [ ] **Step 2: Write the failing label-layer stylesheet contract test**

Add this test after the card stacking test in `src/theme.rs`:

```rust
#[test]
fn swimlane_label_stays_above_cards() {
    assert_block_contains(".swimlane", "z-index: auto");
    assert_block_contains(".swimlane-label", "z-index: 4");
    assert_block_contains(
        ".swimlane-label",
        "background: color-mix(in srgb, var(--bg) 88%, transparent)",
    );
}
```

- [ ] **Step 3: Run both focused tests and verify RED**

Run:

```bash
cargo test layout::tests::swimlanes_confine_cards_to_stacked_bands -- --exact
cargo test theme::tests::swimlane_label_stays_above_cards -- --exact
```

Expected: the layout test FAILS with `a overlaps the frontend title strip`; the theme test FAILS because `.swimlane` contains `z-index: 0`, not `z-index: auto`.

- [ ] **Step 4: Reserve title space only for labeled lanes**

Add these module constants immediately above the `place_laned` documentation comment:

```rust
const LANE_CARD_PAD: f64 = 20.0;
const LANE_TITLE_H: f64 = 36.0;
```

Delete this local declaration from `place_laned`:

```rust
const LANE_VPAD: f64 = 20.0;
```

After `let num_ranks = order.len();`, insert the per-lane top padding calculation:

```rust
let top_pad: Vec<f64> = lane_order
    .iter()
    .map(|label| {
        if label.is_empty() {
            LANE_CARD_PAD
        } else {
            LANE_TITLE_H
        }
    })
    .collect();
```

Replace the band-height assignment:

```rust
band_h[li] = lane_stack_h[li] + top_pad[li] + LANE_CARD_PAD;
```

Replace the cursor initialization before the rank-node placement loop:

```rust
let mut cursor: Vec<f64> = lane_y
    .iter()
    .zip(&top_pad)
    .map(|(y, pad)| y + pad)
    .collect();
```

- [ ] **Step 5: Put the label above cards without raising the lane background**

In `.swimlane`, replace the existing stacking declaration exactly:

```css
z-index: auto;
```

In `.swimlane-label`, add these declarations immediately after `position: absolute`:

```css
z-index: 4;
padding: 1px 6px;
border-radius: 6px;
background: color-mix(in srgb, var(--bg) 88%, transparent);
```

`z-index: auto` prevents the lane container from trapping its label in a level-0 stacking context; only the compact label rises above cards.

- [ ] **Step 6: Run the focused tests and verify GREEN**

Run:

```bash
cargo test layout::tests::swimlanes_confine_cards_to_stacked_bands -- --exact
cargo test theme::tests::swimlane_label_stays_above_cards -- --exact
```

Expected: both PASS.

- [ ] **Step 7: Run formatting and the complete verification gates**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

Expected: every command exits 0; all Rust tests pass; clippy reports no warnings; the diff has no whitespace errors.

- [ ] **Step 8: Commit the lane-title fix**

```bash
git add src/layout.rs src/theme.rs assets/main.css
git commit -m "fix(canvas): keep lane titles clear"
```
