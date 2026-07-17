# nodestorm roadmap

Direction: keep the agent-proposes → human-decides → agent-applies loop
tight, make its results durable, then make the canvas scale.

## Done — v0.1

- [x] Desktop canvas (Dioxus WebView): node cards, edges, pan/zoom, drag-to-pin
- [x] MCP server (streamable HTTP, loopback): `propose_graph`, `update_graph`,
      `await_decisions` with exactly-once delivery, `get_state`, `clear_session`
- [x] Choices attached to nodes: pros/cons, ★ recommendation, ripple preview
      (`affects`), considered-trail capture, notes, dismissals
- [x] Session persistence: debounced atomic autosave, restore on start
- [x] Deterministic layered auto-layout with user pinning
- [x] Claude Code skill (`skills/nodestorm`) and agent simulator (`examples/drive`)
- [x] Automated GUI verification: Windows UIA E2E, headless Linux recipe

## Done — v0.2: Export & decision records

- [x] `src/export.rs`: pure, deterministic Markdown decision-record renderer
      (decisions with pros/cons and considered trails, dismissed choices with
      reasons, open questions, notes, grouped component inventory)
- [x] Embedded Mermaid diagram: shape per kind, color per status, subgraph per
      group, edge style per kind/status
- [x] `export_markdown` MCP tool so the agent can pull the record into repo docs
- [x] Export button in the top bar: writes next to the session file, path
      surfaced in the activity feed
- [x] Skill etiquette: write the record into the user's repo at session end

## Done — v0.3

- [x] User graph editing: add/rename/delete nodes and edges from the UI
      (buttons, gestures, and keyboard), origin-tracked so user elements
      survive agent proposes until adopted, flowing back to the agent as
      decision events (`node_added`, `node_edited`, `node_deleted`,
      `removal_requested`, `edge_added`, `edge_deleted`)
- [x] Scale & navigation UX: search with highlight/zoom-cycle, minimap with
      click/drag panning, keyboard navigation, zoom-to-node
- [x] Export polish: Export ▾ menu — Save As… (native dialog),
      copy Markdown/Mermaid to clipboard, mermaid-only export, and a
      `format` param on `export_markdown`

## Done — v0.4

- [x] Concurrent named sessions: per-session stores, `session` param on
      every MCP tool (propose auto-creates; awaits on different sessions run
      concurrently), `list_sessions`, session switcher with badges,
      create/archive, legacy `session.json` migration
- [x] Graphs beyond ~100 nodes: always-on viewport culling, collapsible
      groups as cluster cards, aggregated `×N` cluster edges (structural
      bundling), `--demo-big N`
- [x] Session timeline: Timeline panel + `## Session log` in exported
      records (flush comments included)

## Done — v0.5

- [x] Session lifecycle: rename (file follows, waiting agents unaffected),
      hard delete, unarchive from the switcher's Manage block
- [x] Cross-brainstorm diffing: structural session-vs-session comparison
      (components/edges/decision drift) via the `diff_sessions` tool and a
      per-session Compare panel

## Later

- [ ] Cosmetic curve bundling for expanded graphs
- [ ] Minimap virtualization for very large graphs
