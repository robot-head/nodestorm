# Connection Popover Readability

**Status:** approved in-session

## Problem

Each Claude MCP connection is rendered as its own two-column grid. The status
column uses intrinsic `auto` sizing, so a long scoped status such as
`Receiving · session · agent` consumes nearly the entire popover width and
collapses the client name into one-character lines.

## Design

Keep the existing responsive popover width and connection data unchanged. Give
each connection row explicit grid areas:

- first line: client name on the left and version on the right;
- second line: connection status spanning the full row, left-aligned.

Long client, session, and agent names may wrap, but only after receiving the
row's full usable width. The layout must remain within the existing viewport
width and height limits.

## Scope

This is a CSS-only presentation fix in `assets/main.css`. It does not change
Dioxus markup, connection state, status labels, routing, or reconnect behavior.
No dependency is added.

## Verification

Extend the stylesheet contract test to require the explicit client, version,
and full-width status grid areas. Run the focused theme tests and the full Rust
test suite, then visually confirm that a long Receiving label no longer
collapses `claude-code` into a vertical column.
