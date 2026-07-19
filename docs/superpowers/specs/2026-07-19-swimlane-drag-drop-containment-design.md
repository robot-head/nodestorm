# Swimlane drag-drop containment

Approved 2026-07-19.

## Goal

Let users create swimlanes, drag cards between them (or out of them entirely),
and always see which lane a card belongs to. Today a lane exists only while some
card carries its name, dragging a card never changes its lane, and a dragged
card floats outside every band with no visible membership.

## Root cause

- A swimlane is derived purely from `node.lane: Option<String>`. There is no way
  to declare an empty lane.
- Dragging a card calls `store.set_position` (pins it) and nothing else. It never
  touches `node.lane`.
- Pinned cards are excluded from lane bands (`src/layout.rs` skips
  `position.is_some()` nodes when sizing and placing bands), so a dragged card
  sits outside the strip with no containment cue. Sibling `group` outlines already
  grow to enclose their (pinned) members; swimlanes do not. That inconsistency is
  the bug.

## Storage: declared lanes as view-state

Add `SessionState.declared_lanes: Vec<String>`, a per-session, user-only,
`#[serde(default)]` list — an exact mirror of the existing `collapsed_groups`
view-state field. It persists per session, is invisible to agents, and rides the
existing `SessionState` serialization with no migration.

Node membership stays in the doc (`node.lane`). An empty lane is a name present in
`declared_lanes` with no member nodes.

Render order for bands:

1. Declared lanes, in `declared_lanes` order.
2. Lanes referenced only by `node.lane` (e.g. agent-set) not already declared, in
   node first-appearance order.
3. The unlabeled default lane (nodes with `lane = None`) — draws no band.

Rejected alternative: a `SessionDoc.lanes` field. It turns an empty canvas
scaffold into graph data, forcing propose-merge preservation and export changes
for no user benefit. The `collapsed_groups` precedent settles the choice.

## Layout: bands emit for every declared lane and grow to contain pins

*(Revised 2026-07-19 after live use: the original two-rect band — a grown drawn
rect over a stable base hit-strip — made the drawn band chase the dragged card,
let grown bands overlap their neighbors, and left the drop zone a thin strip at
the top of what the user saw. Single rect now; stability comes from releasing
the card's lane at drag start instead.)*

`place_laned` in `src/layout.rs`:

- Accepts the explicit ordered lane list above instead of deriving lane order from
  node first-appearance.
- Emits a `LaneBand` for every declared lane, including empty ones, with a minimum
  band height (`LANE_MIN_H`) so an empty band is a comfortable drop zone.
- Stacks bands top-to-bottom in one pass: each labeled band grows **downward and
  sideways** (never upward) to enclose its pinned members, and the next band
  starts below the grown bottom plus `LANE_SEP`. Bands therefore never overlap
  and always keep a minimum gap; auto-placed cards in later lanes move down with
  their band.

`LaneBand` has a single `rect`: it is drawn, hit-tested, and highlighted as one
rectangle, so the visible band is exactly the drop zone. Stability mid-drag comes
from the drag interaction (below), not from a second rect.

Mid-drag the dragged card belongs to no lane (released at drag start), so no
band's geometry depends on the cursor: every band holds still while the card is
in flight. On drop the next layout grows the receiving band to enclose the card,
pushing lower bands down as needed.

## Drop = re-parent by geometry

On the first actual movement of a `DragNode` gesture (`src/ui/canvas.rs`), the
card is released from its lane (`store.set_lane(id, None)`): bands hold still
for the whole drag, and dragging out is the default — membership is re-earned
by where the card lands. If the card was the lane's last member and the lane
was undeclared (agent-set), `set_lane` rescues it into `declared_lanes` so the
band survives the drag instead of vanishing; deleting a lane stays an explicit
`×` click.

In `Canvas.onmouseup`, when the ending gesture is a `DragNode` that actually
moved:

- Test the dropped card's center point (plane coords) against each band rect in
  `layout.lanes`.
- Center inside a band -> `store.set_lane(id, Some(label))`.
- Center inside no band -> `store.set_lane(id, None)` (the "drop outside" case).

The card keeps its pinned drop position (Model B). The drop path mutates
`node.lane` *without* a fresh checkpoint — exactly like `set_position` — so the
drag's start-of-drag checkpoint captures both the old position and old lane, and a
single undo reverts the whole drag (undo restores the doc, and `node.lane` lives in
the doc). Lane normalization (trim, empty -> None) mirrors `edit_node`.

`add_lane` is a pure view-state operation (it only appends to `declared_lanes`,
touching no doc data) and is **not** undoable — `Snapshot` captures only the doc,
matching `collapsed_groups`/`toggle_group_collapsed`.

`rename_lane` and `delete_lane` also rewrite member `node.lane` (a doc field), so a
misclick on a populated lane's `×` would otherwise irreversibly strip membership
from every member card. They therefore **checkpoint** (`push_undo`) before
mutating: one Ctrl+Z restores `node.lane` on the members, and the lane reappears as
a band. Undo does not restore `declared_lanes` (it is view-state, outside the
snapshot); for a *declared* lane this leaves the renamed-to name as an empty band
after undo — a minor, accepted wart, since restoring membership is the goal.

## Drag feedback

While a `DragNode` gesture is live, compute the band under the dragged card's
center and mark it with a `drop-target` state. `assets/main.css` adds
`.swimlane.drop-target` (brighter fill/outline) so the destination lane is obvious
mid-drag. The band element stays `pointer-events: none`; the highlight is driven by
geometry, not DOM hover.

## Add / rename / delete lane

- Add: a `+ Swimlane` button in `canvas-controls` calls `store.add_lane()`, which
  appends a deduped default name (`"New lane"`, `"New lane 2"`, ...) to
  `declared_lanes`.
- Rename: the lane label (rendered in `src/ui/canvas.rs`) becomes an inline text
  input. Commit renames the `declared_lanes` entry and rewrites `node.lane` on
  every member node so membership follows the rename.
- Delete: a `×` control on the label calls `store.delete_lane(label)`, removing it
  from `declared_lanes` and clearing `node.lane` on members (cards fall to the
  default lane). Add-without-delete would strand empty lanes, so delete is in
  scope.

The label and its controls re-enable `pointer-events: auto` over the otherwise
non-interactive band.

## Verification (TDD)

Write and confirm-failing before implementing:

- Layout: a declared empty lane produces a band; a pinned card whose `lane` is set
  is fully enclosed by that lane's band rect.
- Layout: band render order is declared-then-referenced-then-default.
- Store: `set_lane` sets and clears membership; a checkpoint taken before it, then
  `set_lane`, then one undo reverts the lane change (drag semantics, since
  `node.lane` is in the doc). `add_lane` dedups names; `delete_lane` clears member
  `node.lane` and removes the declared entry; `rename_lane` rewrites member
  `node.lane` and the declared entry. `delete_lane`/`rename_lane` checkpoint, so
  one undo restores member membership.
- Layout: `lane_at` returns the lane whose band rect contains a point, and
  `None` for a point outside every band (the drop-outside case).
- CSS contract assertion for `.swimlane.drop-target`.

Then run the focused tests and the full project checks.

## Out of scope

Lane reordering, drag-to-reorder lanes, per-lane color, and agent-visible lane
declarations. Add when asked.
