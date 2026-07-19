---
name: nodestorm
description: >
  Drive the Nodestorm visual architecture canvas while brainstorming or
  planning. Invoke automatically only when connected Nodestorm tools are
  available and the task involves a system design, implementation choices,
  session comparison, or a decision record. Also use when the user explicitly
  asks for Nodestorm; if tools are absent, offer the bundled trusted setup flow
  and ask before installing or launching anything.
---

# Nodestorm visual planning

Nodestorm is a native desktop canvas beside the terminal. It shows a proposed
architecture as components, dependency edges, and implementation choices
attached to the component they affect. The user chooses options, edits the
graph, drags cards, and writes notes; receive those actions through
`await_decisions`.

## Discover the tools

The logical tools are `propose_graph`, `update_graph`,
`await_decisions`, `get_state`, `clear_session`, `export_markdown`,
`list_sessions`, `diff_sessions`, and `diff_record`.

Direct MCP hosts may scope names differently. Match the logical suffix rather
than requiring one prefix. Common forms include Claude plugin names such as
`mcp__plugin_nodestorm_nodestorm__propose_graph`, Codex names such as
`mcp__nodestorm__propose_graph`, and OpenCode names such as
`nodestorm_propose_graph`. On Pi, call the always-available `nodestorm` proxy:

```json
{"tool":"propose_graph","args":{"nodes":[],"edges":[],"choices":[]}}
```

Treat automatic invocation as available only when all tools needed for the
current workflow are connected. Never run setup merely because a general
planning task resembles this skill.

If the user explicitly requested Nodestorm and the tools are absent:

1. Explain that the plugin is present but the native Nodestorm app is not
   reachable at `http://127.0.0.1:4747/mcp`.
2. Ask permission before installing and ask again before launching. A single
   clear confirmation may cover both only when the user explicitly approves
   both actions.
3. Run `scripts/setup.ps1` on Windows or `scripts/setup.sh` on Linux/macOS from
   this skill directory. After consent, pass `-ApproveInstall`/`--approve-install`
   plus either the approved launch flag or `-SkipLaunch`/`--skip-launch`; this
   makes the user's exact choice explicit in non-interactive hosts. Do not copy
   commands from the scripts into a looser fallback.
4. If setup refuses a trust, dependency, version, port, or readiness check,
   report that reason and continue without Nodestorm. Never substitute an
   unsigned download, source build, administrator install, PATH edit, or an
   unpinned latest release.

## The graph and decision loop

1. **Push early.** Once the system's shape is understood, call
   `propose_graph` with existing and proposed components, edges, and the first
   open choices. Include an `announce` message. The canvas is for thinking,
   not presenting a finished diagram. Optionally give nodes a `lane` (or
   `set_lane` later) to arrange the layout into labeled horizontal swimlanes
   (e.g. client / services / data); the user can override a lane from the edit
   form.
2. **Attach choices where they live.** Put a persistence decision on the data
   store and a protocol decision on the service that owns it. Give each choice
   2–4 options, exactly one `recommended: true`, honest `cons`, and `affects`
   node ids so the canvas can show the ripple. When the open point needs prose
   rather than a pick, use the `ask` op instead: attach a free-form question
   (optionally to a node); the user types an answer that comes back as a
   `question_answered` decision. Re-asking the same id keeps the user's answer.
   When one choice only makes sense after another, give it `depends_on` (a list
   of `{node, choice}` refs): the canvas locks it until its parents are decided.
   Keep dependencies acyclic — cycles are rejected. If you reopen a parent, its
   decided dependents are flagged for review; re-scope them.
3. **Wait for the user.** Tell the user what is on the canvas, then call
   `await_decisions`. On `status: "timeout"`, call it again immediately.
   Queued decisions are not lost. Act only on `status: "delivered"`;
   `decisions_so_far` on a timeout is a non-authoritative preview.
4. **Apply the ripple atomically.** After delivery, call `update_graph` with
   one ops list: mark impacted nodes `affected`, add follow-up choices, focus
   the active component, remove items the user asked to remove, and announce
   the result. The user's chosen option is already resolved.
5. **Read the trail.** `option_selected.considered` reveals hesitation.
   `note_added` is a constraint. `flush_requested` is the user speaking
   directly. Address all of them in the terminal.
6. **Track implementation.** As you actually build the decided components,
   advance their build lifecycle with `set_build`
   (`planned → building → built → verified`). The canvas becomes a live
   progress board and the exported record gains an implementation-status
   section, so it says both what was decided and what shipped.
7. **Leave a record.** When open choices are exhausted or the user is done,
   call `export_markdown` and save the returned Markdown in the repository,
   commonly under `docs/decisions/`. Ask before overwriting an existing file.

## Respect user edits

The canvas is a shared whiteboard. Handle delivered edit events as follows:

- `node_added`: acknowledge it. If `upsert_node` enriches it, the agent adopts
  it and must include it in later proposals.
- `node_edited`: treat the new label, kind, and description as canonical.
- `removal_requested`: remove the agent-owned node and related state with
  `update_graph`, or explain why it should remain before doing anything else.
- `node_deleted` and `edge_deleted`: never silently re-add them.
- `edge_added`: incorporate the dependency into the design.
- `question_answered`: the user's text reply to an `ask`; treat it as an
  authoritative answer and fold it into the design or the terminal discussion.
- `annotation_added` / `annotation_edited` / `annotation_deleted`: freehand
  sticky notes, arrows, and highlight regions the user drew. They are margin
  commentary, not graph structure — read them as intent, never convert them
  into nodes/edges. They survive proposes and export under *Annotations*.

User edits and decisions can be undone until delivered. Only events returned
by `await_decisions` are final.

## Named sessions

Every tool accepts an optional `session`. Omit it to use the active session.
Use short topic slugs when creating parallel sessions.

- Call `list_sessions` before assuming what exists.
- Call `diff_sessions` before re-proposing into an older brainstorm and cite
  the drift instead of overwriting it. Use `diff_record` to compare the live
  session against a decision record already committed to the repo (pass its
  `.md` path) and report what has changed since.
- The user sees only the active session. The agent cannot switch it, so say
  which session changed and do not expect decisions until the user opens it.
- Keep one topic per session.

## Multiple agents on one session

When several agents collaborate on the *same* session, each passes an `agent`
id (a short slug) on `propose_graph`, `update_graph`, and `await_decisions`:

- Nodes you propose/upsert are attributed to you (a colored badge on the card
  and in the feed).
- `await_decisions` with your `agent` returns only decisions on nodes you
  authored, plus unclaimed ones (user-drawn elements, annotations) — so
  several agents can wait on one session at once without stealing each other's
  decisions. Omit `agent` to receive every decision (single-agent).
- Attribution is forced from the id you send; you cannot claim another agent's
  work. Coordinate ids out of band so they stay stable.

## Etiquette and recovery

- Keep the terminal as the primary channel. Narrate every canvas change.
- Preserve user-owned positions, notes, and decisions. Reopen a choice only
  after explaining why and setting `reopen: true`.
- Keep stable slug ids. Changing an id creates a duplicate.
- Keep a brainstorm around 100 nodes or fewer.
- After an `await_decisions` transport error, call `get_state` to recover the
  document and undelivered decisions, then resume waiting.
- Call `clear_session` only when the user asks for a fresh start.

The bundled direct MCP configurations use a 600-second client timeout. The
server's normal `await_decisions` timeout remains shorter and may be repeated
indefinitely while the session is active.
