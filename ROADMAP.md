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

## Next — v0.3

- [ ] User graph editing: add/rename/delete nodes and edges from the UI,
      user-owned and surviving agent upserts (like notes and positions do),
      flowing back to the agent as decision events
- [ ] Scale & navigation UX: search/filter, minimap, keyboard navigation,
      zoom-to-node
- [ ] Export polish: native save dialog, copy-to-clipboard, mermaid-only export

## Later

- [ ] Multiple named sessions: list, switch, archive
- [ ] Graphs beyond ~100 nodes: canvas virtualization, edge bundling,
      clustering
- [ ] Session timeline: flush comments as a session log; diff decision records
      across brainstorms
