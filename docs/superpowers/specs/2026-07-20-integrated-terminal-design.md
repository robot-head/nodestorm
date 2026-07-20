# Integrated Terminal Design

**Date:** 2026-07-20

**Status:** Approved

## Goal

Run launched agent sessions inside Nodestorm in an integrated terminal panel
instead of the system terminal. The user can minimize the panel and bring any
agent's terminal back into focus by clicking that agent's name anywhere in the
UI. The integrated terminal is the default launch target; the existing
system-terminal launch remains available as an option.

Out of scope for this version: a general-purpose (non-agent) shell, detached
agents that survive app exit, a resizable panel divider, and persistence of
terminal output.

## Requirements

- Local and SSH launches both run in the integrated terminal. The PTY runs
  the same command the system-terminal path builds today, so SSH host-key and
  authentication prompts appear inside the panel.
- Multiple agents run concurrently, one terminal tab per agent.
- Closing a tab or quitting the app kills the contained agent process, after
  a confirmation when the agent is still running.
- Every rendered agent name that matches an open terminal tab (running or
  exited, not yet closed) is a click target
  that restores and focuses that terminal. Persistent topbar chips guarantee a
  visible click target even when the panel is collapsed and the agent has not
  yet produced any attributed activity.

## Architecture

Approach: Ferroterm (a Rust→WASM VT100/xterm terminal emulator,
<https://datanoisetv.github.io/ferroterm/>) in the webview using its WebGL
renderer, plus a Rust PTY, bridged over a local WebSocket — the same
architecture VS Code uses, with Ferroterm in place of xterm.js. Rejected
alternatives: xterm.js (works, but Ferroterm's WebGL renderer and ~70 KB
footprint were preferred), a pure-Rust terminal renderer in Dioxus DOM
(enormous rendering/IME/perf lift), and pushing PTY bytes through the Dioxus
`eval` bridge (no backpressure, stutters under TUI redraw load).

### TerminalManager (`src/terminal.rs`)

Owned by app state alongside `Sessions`. Holds a map of terminal id to:

- the PTY pair, spawned with `portable-pty` (ConPTY on Windows, `openpty`
  elsewhere),
- the child process handle,
- a scrollback ring buffer of about 1 MiB,
- status: running or exited,
- a broadcast channel of output chunks.

API: `spawn(CommandSpec, cwd) -> TerminalId`, `write`, `resize`, `kill`,
`subscribe`, `list`. `spawn` maps the launcher's existing `CommandSpec`
argv directly onto portable-pty's `CommandBuilder`; no shell interpretation
is added on any platform. SSH launches spawn the same `ssh -t ...` spec the
external path uses today.

### WebSocket route

The embedded axum server (already serving MCP on loopback) gains
`GET /terminal/{id}/ws` using axum's `ws` feature:

- On upgrade, require a per-run 128-bit random token; reject otherwise.
- On connect, replay the scrollback buffer, then stream live output.
- Client-to-server traffic: raw input bytes, plus small JSON control frames
  for resize (columns and rows).

### Terminal panel (`src/ui/terminal_panel.rs`)

Bottom dock rendered above the canvas. One Ferroterm instance (WebGL
renderer, Canvas2D fallback) per tab, mounted with a one-time eval; each
instance opens its own WebSocket with the token. Ferroterm is an ES-module
graph plus a WASM binary, so it cannot be inlined as a script tag: the
author-deployed build (Ferroterm is not published to npm) is vendored under
`assets/ferroterm/` with recorded SHA-256 hashes, embedded into the binary
with `include_bytes!`, and served by the embedded loopback server at
`/terminal/assets/*` with correct MIME types and CORS headers for the
webview origin. No CDN or external network fetch at runtime.

### Launcher integration

The launcher dialog gains a "Run in" choice: Integrated terminal (default) or
System terminal. Validation and git preparation are unchanged. On the
integrated path, the final step calls `TerminalManager::spawn` instead of
`open_terminal`; the system-terminal path keeps today's code.

## User Experience

- The panel is hidden until the first integrated launch, then expands to a
  fixed 40% height with the new tab focused. A chevron collapses it to
  nothing.
- Tab bar: agent display name (agent kind plus session slug, matching the
  launcher's agent identity), a status dot (green running, gray exited), and
  a close button.
- Topbar chips: one per open terminal, showing name and status dot. Clicking
  a chip expands the panel and selects that tab. The chip disappears when the
  tab is closed.
- Agent-name attributions in the activity feed, node cards, choice panel,
  questions panel, and queued changes become clickable when the name matches
  an open terminal tab; hover shows a pointer and underline. Names without a
  matching terminal render as today, since agents may connect from outside
  Nodestorm.
- Closing a tab whose agent is still running asks for confirmation, then
  kills the PTY. Quitting the app with any running agent asks once.
- An exited agent's tab and scrollback stay readable until the user closes
  the tab.

## Lifecycle and Errors

- Child exit flips the tab and chip to exited; output remains visible.
- PTY spawn failure keeps the launcher dialog open with the captured error;
  the prepared branch or worktree is retained, matching the existing
  no-silent-cleanup policy.
- WebSocket drop shows a reconnecting notice in the terminal; the client
  auto-reconnects and the scrollback replay restores the view.
- Nothing about terminal output is persisted; scrollback lives in memory
  only, consistent with the launcher's no-transcript policy.

## Security

- The WebSocket route is shell-grade access, so upgrades require the per-run
  random token, which is generated at startup and embedded only in the
  webview. The server continues to bind loopback only.
- PTY spawning uses executable-plus-argument arrays end to end; existing
  POSIX escaping for SSH is unchanged.
- Ferroterm is vendored and pinned; the webview loads no remote code. Its
  JS/WASM assets are served only on the loopback listener; they contain no
  secrets, so the token gates only the WebSocket.
- Kill-on-close only; Nodestorm never leaves orphaned agent processes by
  design, and never deletes branches or worktrees.

## Dependencies

- `portable-pty` (new crate dependency).
- axum `ws` feature (axum already present).
- Token generation via the existing `uuid` dependency (two v4 UUIDs or
  equivalent 128-bit randomness).
- `tokio-tungstenite` as a dev-dependency for WebSocket integration tests.
- Ferroterm (ES modules + WASM, MIT) vendored under `assets/ferroterm/` from
  the author's deployed build, with SHA-256 hashes recorded (not on npm).

## Testing

Unit tests:

- WebSocket upgrade rejected without or with a wrong token.
- Scrollback ring buffer: append, wrap, replay order.
- Resize control-frame parsing.
- `CommandSpec` to `CommandBuilder` mapping preserves program, argv, and cwd.
- Manager status transitions on kill and on child exit.

Integration tests (temporary processes, real PTY):

- Spawn `cmd /c echo` on Windows and `sh -c echo` elsewhere through the PTY
  and read the output through the WebSocket route.
- Kill terminates the child and reports exited.

UI verification (manual):

- Launch Claude Code integrated; the panel opens with the agent TUI.
- Collapse the panel; click the topbar chip and an activity-feed agent name;
  both restore and focus the tab.
- The System terminal launcher option still opens the external terminal.
- Full run of existing Rust tests, Clippy, and release gates.

## Completion Criteria

1. Launching any supported agent locally opens its interactive TUI in a new
   integrated terminal tab by default.
2. An SSH launch runs in the panel with working authentication prompts and
   the reverse MCP tunnel.
3. Multiple agents run concurrently in separate tabs.
4. Collapsing the panel and clicking the agent's topbar chip or any rendered
   agent-name attribution restores and focuses the right tab.
5. Closing tabs or the app confirms before killing running agents; the
   system-terminal option still works.
