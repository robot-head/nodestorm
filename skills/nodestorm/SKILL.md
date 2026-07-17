---
name: nodestorm
description: >
  Drive the nodestorm visual brainstorming canvas while designing or planning
  system architecture. Use when nodestorm MCP tools (mcp__nodestorm__*) are
  available AND you are brainstorming a design, presenting implementation
  choices, or in plan mode weighing approaches. Pushes the architecture as a
  node graph, attaches each decision to the component it belongs to, and
  blocks on await_decisions while the user clicks their picks.
---

# Driving nodestorm during brainstorming and planning

nodestorm is a desktop canvas the user has open next to the terminal. It shows
your proposed architecture as a graph: components as cards, dependencies as
edges, and **each implementation choice attached to the node it belongs to**.
The user picks options, drags cards, and writes notes; you receive everything
through `await_decisions`.

Availability check: the tools are named `mcp__nodestorm__propose_graph`,
`mcp__nodestorm__update_graph`, `mcp__nodestorm__await_decisions`,
`mcp__nodestorm__get_state`, `mcp__nodestorm__clear_session`,
`mcp__nodestorm__export_markdown`. If they are not
connected, tell the user they can start nodestorm and run
`claude mcp add --transport http nodestorm http://127.0.0.1:4747/mcp`, then
continue without it — never block on its absence.

## The loop

1. **Push the graph early.** As soon as you understand the system's shape,
   call `propose_graph` with the components (existing AND proposed), edges,
   and the first open choices. Don't wait for a perfect design — the canvas
   is for thinking, not presenting. Include an `announce` message describing
   what you proposed.
2. **Attach choices where they live.** A persistence decision goes on the
   datastore node; a protocol decision on the edge-owning service. 2–4
   options each, exactly one `recommended: true`, honest `cons` (a con-free
   option reads as salesmanship), and `affects` listing the node ids the
   option would ripple into — hovering an option highlights those nodes for
   the user.
3. **Block on `await_decisions`.** Tell the user in the terminal what's on
   the canvas and that you're waiting, then call it. On `status: "timeout"`
   call it again immediately — decisions are queued server-side and never
   lost; the re-call loop is the protocol, not an error. Act only on
   `status: "delivered"`; `decisions_so_far` in a timeout response is a
   non-authoritative preview.
4. **Apply the ripple.** After a delivery, use `update_graph` (one atomic ops
   list) to: `resolve_choice` is already done (the user's pick is recorded);
   mark impacted nodes `set_status: affected`; `add_choice` for the follow-up
   decisions the pick created; `set_focus` to the node you're discussing;
   `announce` a one-line summary. Then continue the conversation in the
   terminal and loop back to step 3 while open choices remain.
5. **Read the trail.** Each `option_selected` carries `considered` — the
   options the user clicked before settling. Visible hesitation (e.g. they
   toggled between two options) is worth a follow-up question. `note_added`
   events are constraints; treat them as requirements. A `flush_requested`
   comment is the user talking to you directly.
6. **Leave a record.** When the brainstorm winds down (no open choices, or
   the user says they're done), call `export_markdown` and write the result
   into the repo's design docs (e.g. `docs/decisions/<topic-slug>.md`). Tell
   the user the path in the terminal, and ask before overwriting an existing
   record. The user can also click **Export** in the app — that copy lands
   next to the session file, not in the repo.

## Etiquette

- **The terminal stays the primary channel.** Narrate what's on the canvas
  each turn ("I've put the webhook design on the canvas — two decisions on
  the Dispatcher and Delivery Store nodes"). Never assume the user saw a
  canvas change you didn't mention.
- **Respect user-owned state.** Card positions, notes, and decided choices
  survive your upserts by design. Re-opening a decided choice requires
  `"reopen": true` on the choice — do that only after telling the user why.
- **Keep ids stable.** Node ids are slugs (`auth-service`); reusing an id
  updates that node, changing ids duplicates it. Same for choice/option ids.
- **Scale to the graph, not your enthusiasm.** ~100 nodes is the practical
  cap; one brainstorm at a time (last writer wins on the shared canvas).
- **Recover cleanly.** After any transport error on `await_decisions`, call
  `get_state` — it returns the doc plus any undelivered decisions — then
  resume the loop. `clear_session` only when the user asks for a fresh start.

## Configuration notes

The default `await_decisions` timeout (240 s) stays under Claude Code's
5-minute idle abort even without progress notifications; nodestorm also sends
progress heartbeats when the client provides a progress token. For very long
sessions the user can raise the client-side ceiling in `.mcp.json`:

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
