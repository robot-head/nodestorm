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
  into your repo — or use the **Export ▾** menu (write next to the session
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

Finding your way around big graphs: the **search box** highlights matches
(Enter cycles + zooms, Esc clears, `/` focuses it) and the **minimap**
(bottom-right) pans on click/drag.

| Key | Action |
| --- | --- |
| arrows | move selection to the nearest card in that direction |
| `Tab` / `Shift-Tab` | cycle selection in document order |
| `Enter` | open the panel for the focus node when nothing is selected |
| `Del` | delete the selection |
| `+` / `-` / `0` | zoom in / out / fit |
| `n` | new component at the view center |
| `Esc` | cancel connect → close menu → clear search → deselect |
| double-click card | zoom to it |
| double-click background | new component there |

## CLI

| Flag | Meaning |
| --- | --- |
| `--port <N>` | MCP port (default 4747, loopback only) |
| `--session <file>` | session file (default: XDG data dir; autosaved, restored on start) |
| `--demo` | load the demo graph instead of restoring |
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
agent node) and exports via the menu, failing unless the record on disk
contains the user's edits. Screenshots and logs land in `target\verify\`. Note that clicks land at an
element's *visual* position: close the choice panel (its `✕`) before
selecting a card the panel overlaps, as the script does.

Design cap: ~100 nodes per graph, one brainstorm session at a time.
Where this is going: see [ROADMAP.md](ROADMAP.md).
