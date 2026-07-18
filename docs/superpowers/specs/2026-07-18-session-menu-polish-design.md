# Session menu polish design

**Date:** 2026-07-18  
**Status:** approved

## Problem

The session dropdown combines the active brainstorm title, session switching,
two always-visible form rows, and destructive actions in one flat list. In the
common single-session state, the active session name is repeated below the
title, the empty form controls dominate the menu, and the archive and delete
actions are not visually separated from routine actions.

## Chosen direction

Keep the dropdown as the fast session switcher, but disclose management
controls only when requested. This preserves its quick, lightweight behavior
while giving creation and renaming a focused, intentional surface.

This was chosen over:

1. Restyling the two permanent forms, which would improve their appearance but
   keep the menu crowded.
2. Moving session management to a modal, which is clearer but unnecessarily
   interrupts a frequent workflow.

## Menu structure

1. **Current session header.** Show the `Brainstorm` eyebrow and full document
   title. Do not repeat the active session slug as a selectable row when it is
   the only session.
2. **Sessions.** Show switchable saved sessions. Preserve their status badges
   and the Compare action for inactive sessions.
3. **Manage session disclosure.** A single row opens management controls in
   place:
   - a labeled, full-width `Rename current session` input and a clear Rename
     button;
   - a labeled, full-width `Create new session` input and Create button.
   Inputs use the existing semantic tokens, generous horizontal padding,
   visible border, rounded rectangle geometry, and the existing focus ring.
   The two forms are separated from each other and from the session list by
   restrained spacing and dividers.
4. **Danger zone.** Archive and Delete sit in a separated footer. Archive is a
   low-emphasis destructive action; Delete uses a two-step inline confirmation
   state (`Delete session` then `Confirm delete`) to avoid accidental permanent
   removal. Cancel returns to the normal footer.
5. **Archived sessions.** Keep the existing restore list after the danger-zone
   controls, preserving its clear Archived label.

## Behavior and accessibility

- Opening the menu shows only the current-session header, session rows,
  `Manage session`, danger-zone actions, and any archived sessions. Empty
  fields are not visible by default.
- Opening Manage focuses neither input automatically; it must not disrupt a
  user who only opened the menu to switch sessions.
- Closing the menu clears unfinished drafts and any pending delete confirmation.
- Buttons retain descriptive native titles where useful. The delete
  confirmation text names the active session so the irreversible target is
  clear.
- Existing viewport bounds and vertical scrolling rules remain unchanged.
- No session data model, persistence format, MCP tool behavior, or theme token
  is changed.

## Testing

- Add component-source contract tests for the management disclosure and the
  delete confirmation state.
- Extend stylesheet contract tests for the management form, danger zone, and
  full-width input layout.
- Run `cargo fmt --check`, `cargo test`, and
  `cargo clippy --all-targets -- -D warnings`.
- Manually verify a one-session menu, multiple-session menu, create, rename,
  archive, delete-confirm/cancel, restore, and a narrow viewport.
