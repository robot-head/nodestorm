# Right-Panel Decision Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the open decisions of a session reachable — list them all in one
panel and step through them — instead of requiring the user to find the node
that carries each one.

**Architecture:** Apply the existing `QuestionsPanel` pattern to choices. The
per-item renderer (`ChoiceBlock`) is shared between the node panel and a new
session-wide panel, exactly as `QuestionBlock` already is. Navigation is a
cursor over a pure, document-ordered list of actionable choices; moving it
sets `selected` and `ZoomTarget`, reusing the canvas's existing centering
effect. No new zoom, scroll, or layout machinery.

**Tech Stack:** Rust, Dioxus, CSS, Rust unit tests, PowerShell UIA E2E.

## Global Constraints

- Do not change delivery semantics: the decision queue, exactly-once delivery,
  and last-open-choice autoflush behave exactly as before.
- Locked choices (unmet `depends_on`) are listed but never navigated to — the
  cursor must never park on something that cannot be decided.
- Reuse `ChoiceBlock`; do not write a second choice renderer.
- Compose CSS from existing theme tokens so all twelve palettes and both
  modes follow with no per-theme additions.

---

### Task 1: Share the choice renderer

**Files:**
- Modify: `src/ui/choice_panel.rs`

**Interfaces:**
- Produces: `pub(crate) ChoiceBlock`, `pub(crate) type ConsideredTrails`.

- [x] **Step 1: Widen the considered-trail key**

`considered` was keyed by `ChoiceId` alone. Choice ids are only unique
*within* a node — `ChoiceRef { node, choice }` is the codebase's own evidence
— and the collision was unreachable only because `ChoicePanel` is keyed on
`node.id` and remounts per node. A session-wide panel renders many nodes'
choices against one map, which makes it reachable. Key it by
`(NodeId, ChoiceId)` via the new `ConsideredTrails` alias.

- [x] **Step 2: Add the owner affordance**

Add a `selected: Option<Signal<Option<NodeId>>>` prop, mirroring
`QuestionBlock`'s `show_attachment`. `Some` renders a `.choice-owner`
click-through line naming the owning node; `None` (the node panel, where the
owner is the panel header) renders nothing.

### Task 2: The decisions panel and its navigation arithmetic

**Files:**
- Create: `src/ui/decisions_panel.rs`
- Modify: `src/ui/mod.rs`

**Interfaces:**
- Produces: `DecisionsPanel`, `actionable_decisions`, `step_decision`,
  `resume_after`, and the `DecisionNav { open, cursor }` context newtype.

- [x] **Step 1: Write the pure helpers and their tests first**

Keep the arithmetic off the component so it is testable without a WebView,
following the file-local pure-function convention already used for
`connection_display` / `visible_group`:

- `actionable_decisions(doc)` — open, unlocked, in document order.
- `step_decision(refs, current, step)` — wrapping prev/next.
- `resume_after(doc, refs, current)` — where the cursor lands once its target
  stops being actionable: the first actionable choice *after* it in document
  order, so deciding walks forward instead of snapping back to the top.
- `bucket(doc, choice)` — which of the four sections a choice belongs to.

Run: `cargo test decisions_panel`

- [x] **Step 2: Render the panel**

Four sections — Open, Waiting on dependencies, Decided, Skipped — each a list
of `ChoiceBlock { selected: Some(..) }`. Empty sections omitted. Header
carries the ‹ / `i of N` / › cluster with `aria_label`s, since the UIA E2E
locates controls by name.

- [x] **Step 3: Auto-advance**

A `use_effect` over `doc`: when the cursor's target leaves the actionable
list, move to `resume_after`. One effect covers the user deciding, the user
skipping, *and* the agent resolving a choice from its side — no callback prop
threaded through `ChoiceBlock`. It only fires when the target has genuinely
left the list, so it never fights the user.

### Task 3: Wire it into the app

**Files:**
- Modify: `src/ui/app.rs`, `src/ui/topbar.rs`, `src/ui/canvas.rs`,
  `src/ui/more_menu.rs`

- [x] **Step 1: Panel slot ordering**

Branch `DecisionsPanel` *before* `ChoicePanel` in the `App` chain — the
opposite of every other panel. Stepping through decisions sets the selection,
so if selection kept winning the slot the panel would close itself on the
first ›. Every other panel keeps today's precedence. Reset `open` and `cursor`
on session switch alongside the sibling panel flags.

- [x] **Step 2: Make the count actionable**

Turn the `seg seg-open` `span` into a `button`, matching `seg-questions` /
`seg-queued`. No new CSS: `.status-chip button.seg` already resets native
button chrome. Because the new panel outranks selection, every sibling
entry point — questions, queued, timeline, session compare, and record
compare — must now also close it.

- [x] **Step 3: Keyboard**

`]` / `[` step decisions and `d` toggles the panel, added to the canvas
`onkeydown` in its existing single-character style. `Tab` cycles *nodes*;
these cycle *decisions*.

### Task 4: Styling, contracts, and E2E

**Files:**
- Modify: `assets/main.css`, `src/theme.rs`, `scripts/verify-windows.ps1`

- [x] **Step 1: CSS**

A `decisions panel` block beside the agent-questions block:
`.decisions-section`, `.panel-nav` / `.panel-nav-count`, `.choice-owner`.

- [x] **Step 2: Source-coupling contracts**

Two `src/theme.rs` tests, mirroring the queue precedents:
`opening_decisions_closes_every_other_right_panel` and
`the_decisions_panel_outranks_node_selection` — the latter because that
ordering is load-bearing and silent to break.

- [x] **Step 3: E2E**

Extend the UIA script: open the panel from the count, assert it lists the
remaining open choice, press ›, assert the counter reads `1 of 1`, and decide
from the panel. Close it before the v0.3 editing checks, which drive the node
panel.

Run: `cargo test --all-targets --locked`, `cargo clippy --all-targets --locked -- -D warnings`
