# Queued Changes Manager Design

## Goal

Make the `N queued` status control an actionable staging area. Users can
inspect, change, or remove every undelivered action before it is sent to the
agent.

## User experience

- Clicking the queued segment opens a hideable right-side **Queued changes**
  panel. Clicking the segment again or the panel close control hides it.
- The panel lists all undelivered events in their delivery order. Each row
  contains a concise description, timestamp, and target where applicable.
- Supported items include decisions and skipped choices, notes, node and edge
  additions, edits, deletion/removal requests, and explicit send requests.
- **Edit** removes the selected queued action, restores the state before it,
  selects or focuses its original target, and lets the user submit a
  replacement through the existing UI.
- **Remove** removes the action without replacement and restores the resulting
  canvas/document state.
- Previously delivered events remain immutable and are shown only in the
  existing Timeline.

## State and replay

The store will retain enough pending-action history to restore the document
state immediately before any queued action. Removing or editing an earlier
item restores that checkpoint and replays its later queued actions in order.
This keeps the visible document and the outgoing decision batch identical.

If a later action cannot be replayed because its target no longer exists or a
dependency changed, it remains visible in the queue as blocked with the reason
and is not sent. The user can edit or remove the blocked item; no action is
silently discarded.

## UI integration

`App` will own the open/closed state beside the existing Timeline state, and
the top bar will receive a signal for toggling it. The new panel uses the same
right-side panel layout and close affordance as Timeline. Opening it clears a
selected node, as Timeline already does, so only one right panel occupies the
slot at once.

## Verification

Store tests will establish that removing a queued item restores its previous
state, replays independent later items, and marks invalidated later items as
blocked. UI-focused tests will cover toggling the panel and exposing queued
item metadata and actions. The complete Rust test suite will run before the
feature is considered done.
