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

`place_laned` in `src/layout.rs`:

- Accepts the explicit ordered lane list above instead of deriving lane order from
  node first-appearance.
- Emits a `LaneBand` for every declared lane, including empty ones, with a minimum
  band height so an empty band is a visible drop zone.
- After pinned cards are placed, unions each pinned card's rect into its lane's
  band rect (the same bounding math `group_outline_rects` uses in
  `src/ui/canvas.rs`), so a dropped card is visibly enclosed by its band.

## Drop = re-parent by geometry

In `Canvas.onmouseup` (`src/ui/canvas.rs`), when the ending gesture is a
`DragNode` that actually moved:

- Test the dropped card's center point (plane coords) against each band rect in
  `layout.lanes`.
- Center inside a band -> `store.set_lane(id, Some(label))`.
- Center inside no band -> `store.set_lane(id, None)` (the "drop outside" case).

The card keeps its pinned drop position (Model B). The drop path mutates
`node.lane` *without* a fresh checkpoint — exactly like `set_position` — so the
drag's start-of-drag checkpoint captures both the old position and old lane, and a
single undo reverts the whole drag. Lane normalization (trim, empty -> None)
mirrors `edit_node`. The standalone lane operations below (`add_lane`,
`rename_lane`, `delete_lane`) each push their own undo entry since they are not
part of a drag.

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
- Store: the drop-path lane mutation sets and clears membership; a checkpoint
  taken before it, then the mutation, then one undo reverts the lane change (drag
  semantics). `add_lane` dedups names; `delete_lane` clears member `node.lane`;
  `rename_lane` rewrites member `node.lane`.
- CSS contract assertion for `.swimlane.drop-target`.

Then run the focused tests and the full project checks.

## Out of scope

Lane reordering, drag-to-reorder lanes, per-lane color, and agent-visible lane
declarations. Add when asked.
