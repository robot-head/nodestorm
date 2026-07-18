# nodestorm

**A visual brainstorming canvas for agentic AI planning.** When Claude Code
(or any MCP agent) designs a system with you, nodestorm turns the wall of
text into a live architecture graph: components as cards, dependencies as
edges, and every implementation choice pinned to the node it belongs to.
Hover an option and the nodes it would ripple into light up. Pick your
options, drop notes, hit **Send to agent** — the agent, blocked on
`await_decisions`, wakes up with your decisions (including which options you
hesitated over) and updates the graph with the fallout.

Built in Rust with [Dioxus](https://dioxuslabs.com) (desktop WebView UI) and
[rmcp](https://github.com/modelcontextprotocol/rust-sdk) (MCP over streamable
HTTP).

## How it works

```
┌────────────┐  MCP (streamable HTTP, 127.0.0.1:4747)  ┌───────────────┐
│ Claude Code │ ──── propose_graph / update_graph ────► │   nodestorm    │
│  (agent)    │ ◄─── await_decisions (blocks…) ──────── │  desktop app   │
└────────────┘        decisions, notes, comments        └──────┬────────┘
                                                               │ you click
                                                               ▼
                                                      cards · choices · ripple
```

- **Agent is the author**: it proposes nodes, edges, and open choices.
- **You are the decider**: pick options (pros/cons, ★ recommendation),
  drag cards (they stay pinned), attach notes, dismiss choices.
- **Exactly-once delivery**: decisions queue until you click *Send to agent*
  (or the last open choice is decided); a timed-out agent re-calls and gets
  them — nothing is ever lost.
- **You can edit too**: add, rename, connect, and delete components right on
  the canvas; your changes flow back to the agent as decision events, and
  your components survive agent re-proposes.
- **Everything leaves a record**: the agent pulls a Markdown decision record
  (with an embedded Mermaid diagram) over `export_markdown` and writes it
  into your repo — or use **⋯ More → Export ▾** (write next to the session
  file, Save As…, copy Markdown/Mermaid to the clipboard, or a mermaid-only
  file).

## Install

System packages (Debian/Ubuntu):

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev
```

On Windows nothing extra is needed — the UI uses the WebView2 runtime that
ships with Windows 10/11.

Build and run:

```sh
cargo install --path .   # or: cargo run --release
nodestorm                # opens the window, MCP server on 127.0.0.1:4747
```

## Connect Claude Code

```sh
claude mcp add --transport http nodestorm http://127.0.0.1:4747/mcp
```

or per-project via `.mcp.json` (the bigger `timeout` lets `await_decisions`
block comfortably):

```json
{
  "mcpServers": {
    "nodestorm": {
      "type": "http",
      "url": "http://127.0.0.1:4747/mcp",
      "timeout": 600000
    }
  }
}
```

Then install the skill so brainstorming/plan flows drive the canvas well:

```sh
mkdir -p ~/.claude/skills
cp -r skills/nodestorm ~/.claude/skills/
```

Ask Claude Code to *"design X and use nodestorm for the choices"* — or let
the skill trigger on its own during brainstorming.

## Try it without an agent

```sh
nodestorm --demo                 # a built-in sample brainstorm
cargo run --example drive        # simulates an agent against a running app
```

## Edit the graph yourself

The canvas is a shared whiteboard, not just the agent's. Everything you
author is origin-tracked and **survives agent re-proposes** until the agent
adopts it (enriching your node via upsert makes it theirs to carry forward).
Every edit flows back as a decision event with your next Send.

- **Add**: the **+ Component** button, double-click empty canvas, or
  right-click → *Add component here* (`n` works too).
- **Edit**: select a card → the panel's *Edit* form (label, kind,
  description).
- **Connect**: drag the ◉ handle from one card onto another, or the panel's
  *Connect →* button, then click the target.
- **Delete**: panel *Delete*, right-click, or `Del`. Your components delete
  immediately (with their edges); agent components are only marked
  `removed` — the agent gets a `removal_requested` event and applies it (or
  pushes back). Edges always delete immediately.
- **Undo/Redo** (topbar buttons — inside **⋯ More** on very narrow windows — or `Ctrl+Z`/`Ctrl+Y`) covers every edit
  and every not-yet-sent decision. Honest boundaries: once decisions are
  delivered to the agent, or the agent mutates the graph, the undo history
  clears — you can't unsend facts or silently clobber agent work.

Finding your way around big graphs: the **search box** highlights matches
(Enter cycles + zooms, Esc clears, `/` focuses it) and the **minimap**
(bottom-right) pans on click/drag. Past ~100 components: cards and edges
outside the viewport aren't rendered at all (always-on culling), and any
`group` can be **collapsed into one cluster card** — click a card's group
pill, or right-click → *Collapse group*. Edges into a collapsed cluster
merge into one thick `×N` bundle; expand with the cluster's ⊞ button or a
double-click. Long edges (spanning several columns) route through shared
horizontal channels, one lane per edge, instead of criss-crossing the
gutters. Try it: `nodestorm --demo-big 300`.

## Sessions

Brainstorms are **named sessions** (files under `sessions/` in the data
dir; a v0.3 `session.json` migrates automatically). The **session menu** in
the top bar lists them with open-decision and ●-agent-waiting badges —
click to switch, type a name + **Create** for a new one, **Archive
current** to move its file to `sessions/archive/`. Every session has its
own store, so **an agent can sit in `await_decisions` on one session while
you work in another**: every MCP tool takes an optional `session` name
(omitted = the session on screen), `propose_graph` auto-creates missing
names, and `list_sessions` shows what exists. Only you switch what's on
screen.

The menu's Manage block also **renames** the active session (the file
follows; a waiting agent is unaffected), **deletes** it permanently, and
**unarchives** anything in `sessions/archive/`. Every other session's row
has a **Compare** button: a side panel shows how it differs from the
active one — components added/removed/changed, edges, and decision drift —
the same summary agents get from `diff_sessions`.

## Timeline

The **Timeline** button opens the session log: every pick, dismissal,
note, edit, and Send-comment in order, timestamped. The same log lands in
exported records as a `## Session log` section — including the comments
you typed into the Send box.

| Key | Action |
| --- | --- |
| arrows | move selection to the nearest card in that direction |
| `Tab` / `Shift-Tab` | cycle selection in document order |
| `Enter` | open the panel for the focus node when nothing is selected |
| `Del` | delete the selection |
| `+` / `-` / `0` | zoom in / out / fit |
| `n` | new component at the view center |
| `Ctrl+Z` / `Ctrl+Y` | undo / redo your edits and undelivered decisions |
| `Esc` | cancel connect → close menu → clear search → deselect |
| double-click card | zoom to it |
| double-click background | new component there |

## Theming

**⋯ More → Theme ▾** in the top bar picks a color palette and mode. Twelve
palette families, each with a dark **and** a light variant: Nodestorm (the
default), Solarized, Gruvbox, Catppuccin (Mocha/Latte), Nord, Dracula,
Tokyo Night, One, GitHub, Everforest, Rosé Pine, and Monokai. Each row
shows live swatches of that palette in the current mode.

The mode row switches **Auto / Light / Dark**. Auto (the default) follows
the system setting — on Windows, *Settings → Personalization → Colors →
"Choose your default app mode"* — and tracks changes live, no restart
needed. The native title bar follows the chosen mode too. The choice is
global (all sessions) and persists in `preferences.json` in the data dir;
it never touches session files, undo history, agents, or exported records
(export colors stay fixed so records read the same everywhere).

## CLI

| Flag | Meaning |
| --- | --- |
| `--port <N>` | MCP port (default 4747, loopback only) |
| `--session <file>` | pin this exact file as the active session (named after the file stem) |
| `--sessions-dir <dir>` | where named sessions live (default: `sessions/` in the data dir) |
| `--prefs <file>` | preferences file (default: `preferences.json` in the data dir) |
| `--demo` | load the demo graph instead of restoring |
| `--demo-big <N>` | load a deterministic N-component graph (scaling checks) |
| `--headless` | MCP server without a window (CI / remote) |

## MCP tools

| Tool | Purpose |
| --- | --- |
| `propose_graph` | replace the canvas with a titled graph (nodes, edges, focus, announce) |
| `update_graph` | atomic op list: upsert/remove nodes & edges, add/resolve choices, set status/focus/title, announce |
| `await_decisions` | block until the user sends decisions (default 240 s, then `status:"timeout"` → call again) |
| `get_state` | non-blocking full state + undelivered decisions (post-error resync) |
| `clear_session` | wipe canvas and decision log |
| `export_markdown` | the brainstorm as a Markdown decision record with an embedded Mermaid diagram (plain text — save it into the repo's docs); `format: "mermaid"` returns just the diagram |
| `list_sessions` | the named sessions with per-session counts and agent-waiting flags |
| `diff_sessions` | structural comparison of two sessions — components added/removed/changed, edges, decision drift — as plain Markdown |

Every tool also takes an optional `session: "name"` (default: the session
on screen); `propose_graph` creates missing sessions on the spot, and
awaits on different sessions run concurrently.

User positions, notes, and already-decided choices survive agent upserts;
re-opening a decided choice requires the agent to set `"reopen": true`.

## Development

```sh
cargo test                        # model, layout, store races, MCP round-trip
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Headless GUI verification on Linux (no display server needed):

```sh
Xvfb :77 -screen 0 1280x840x24 &
DISPLAY=:77 WEBKIT_DISABLE_DMABUF_RENDERER=1 dbus-run-session -- \
  cargo run -- --demo
DISPLAY=:77 scrot /tmp/nodestorm.png
```

(`dbus-run-session` matters: without a session bus, GTK's
`g_application_register` hangs silently before the window appears.)

Automated GUI verification on Windows:

```powershell
powershell -File scripts\verify-windows.ps1            # full E2E interaction
powershell -File scripts\verify-windows.ps1 -DemoShot  # render check + screenshot
```

The script finds UI elements by name through Windows UI Automation (WebView2
exposes the DOM as a UIA tree) and clicks them by posting `WM_LBUTTON*`
messages directly to the WebView2 render widget, so it needs neither the
cursor nor the foreground — it runs quietly in the background even while a
human uses the desktop (the app window is pushed to the bottom of the
z-order). Full mode runs `examples/drive.rs` as the agent, clicks through
both proposed choices in the real UI, waits for the autoflush delivery, and
fails unless the drive client actually receives the decisions over MCP —
then exercises user editing (add a component, rename it through the panel
form with window-targeted `WM_CHAR` typing, connect it, soft-delete an
agent node), creates and switches named sessions, opens the Timeline, and
exports via ⋯ More → Export ▾ — failing unless the record on disk contains the
user's edits and the session log. Screenshots and logs land in
`target\verify\`. Note that clicks land at an
element's *visual* position: close the choice panel (its `✕`) before
selecting a card the panel overlaps, as the script does.

Design cap: ~100 nodes per graph, one brainstorm session at a time.
Where this is going: see [ROADMAP.md](ROADMAP.md).
