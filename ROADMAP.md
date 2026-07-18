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

## Done — v0.6

- [x] Undo/redo for user edits and undelivered decisions (snapshot stacks;
      cleared on delivery and on agent turns — no unsending facts, no
      clobbering agent work), topbar buttons + Ctrl+Z / Ctrl+Y
- [x] Channel-lane routing for rank-spanning edges (cosmetic bundling for
      expanded graphs)

## Done — v0.7

- [x] Theming: twelve terminal-palette families (Solarized, Gruvbox,
      Catppuccin, Nord, Dracula, Tokyo Night, One, GitHub, Everforest,
      Rosé Pine, Monokai + the Nodestorm default), each with dark and
      light variants via CSS `light-dark()`
- [x] Auto / Light / Dark mode — Auto follows the OS live; the native
      title bar tracks the mode (tao `set_theme`)
- [x] Theme ▾ picker in the top bar with per-family live swatches; choice
      persists globally in `preferences.json` (new `--prefs` flag), never
      touching sessions, undo, agents, or exports

## Done — v0.8

- [x] Storm UI: derived glow/soft tokens over every palette, pill pods,
      fused status chip, node-card status rails, Space Grotesk display
      face, storm empty state
- [x] Responsive top bar: container-query breakpoints, ⋯ More menu
      (Export/Theme accordions), compose popover for the narrow-width
      agent message, narrow-fit E2E assertion

## Next

Organized as themes, priority-ordered inside each (leverage-first:
features that extend existing machinery and unlock later ones come
first). Versions get assigned when work starts.

### Deepen the agent loop

- [ ] Free-form agent questions: an `ask` op on `update_graph` — the
      agent attaches an open question (optionally to a node); the user
      answers with text in the panel. Answers ride the decision queue
      with exactly-once delivery; unanswered questions export under
      *Open questions*, answered ones alongside decided choices.
- [ ] Implementation tracking: a build lifecycle on node status
      (`planned → building → built → verified`) the agent sets via
      `update_graph` as it implements. Status rails + a topbar progress
      summary make the canvas a live progress board; exports gain an
      implementation-status column — the record says what was decided
      *and* what shipped.
- [ ] Choice dependencies: choices declare `depends_on` other choices;
      dependents render locked with a "waiting on X" hint until the
      parent is decided (re-scoping the dependent stays the agent's
      job). Reopening a parent flags decided dependents for review;
      cycles rejected at the API.
- [ ] Multi-agent sessions (sketch — needs its own brainstorm):
      per-agent identity at the MCP layer, color/badge attribution on
      nodes, choices, and the feed; `await_decisions` returns only
      decisions addressed to (or unclaimed by) that agent; concurrent
      awaits on the *same* session become legal.

### Canvas & visualization power

- [ ] Semantic zoom: zoom-tiered rendering on top of viewport culling —
      far out, cards collapse to labeled chips and group outlines
      dominate; mid, title + status; close, the full card. Legibility
      on big graphs without manual collapsing.
- [ ] Swimlanes & layers: an optional `lane` on nodes
      (agent-assignable, user-overridable) constraining the layered
      layout to labeled horizontal lanes; plus toggleable edge-kind
      layers (e.g. data-flow only) to declutter dense graphs.
- [ ] Freehand annotations: sticky notes, arrows, and highlight regions
      drawn on the canvas — deliberately *not* graph structure.
      Origin-tracked like user nodes (survive agent proposes),
      delivered as note events, exported in an *Annotations* section.

### Scale & records

- [ ] Minimap virtualization for very large graphs
- [ ] Diff against exported record files (session-vs-session shipped in
      v0.5)
