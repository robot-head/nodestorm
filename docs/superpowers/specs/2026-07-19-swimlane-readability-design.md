# Swimlane readability

Approved 2026-07-19.

## Goal

Keep swimlane titles readable and make overlapping cards discoverable without
adding persistent ordering state or new controls.

## Scope

Reserve a 36px title strip at the top of each labeled swimlane. Automatic lane
layout places cards below that strip while retaining the existing 20px bottom
padding and spacing between lanes. The label itself remains above cards so a
pinned card at a user-defined position cannot erase it.

Give canvas cards explicit stacking levels: normal cards use level 1, hovered
cards rise temporarily to level 2, and the selected card stays at level 3. Lane
labels use level 4. Existing keyboard selection remains an escape hatch for a
card that is fully covered.

No document schema, persistence, context-menu action, or manual z-order control
is added.

## Components

- `src/layout.rs` owns the lane title clearance and includes it in lane height
  and automatic card placement.
- `assets/main.css` owns the normal, hover, selected, and lane-label stacking
  levels plus the label's compact background treatment.
- Existing selection and hover markup in `src/ui/node_card.rs` is reused without
  new component state.

## Interaction

Hovering any exposed part of a card brings the whole card forward for quick
reading. Clicking it selects it, keeping it above neighboring cards after the
pointer leaves. Selecting another card transfers that persistent foreground
position. Background click or Escape clears selection and returns cards to
their normal ordering.

## Verification

Add a layout regression test asserting that automatically placed cards begin
below the swimlane title strip. Add stylesheet contract assertions for the
normal, hovered, and selected stacking levels. Follow test-driven development:
run the new assertions first and confirm they fail, apply the minimal layout
and CSS changes, then run the focused tests and the full project checks.
