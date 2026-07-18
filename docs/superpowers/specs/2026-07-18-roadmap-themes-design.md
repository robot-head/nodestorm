# Roadmap themes: agent-loop depth and canvas power

**Date:** 2026-07-18
**Status:** Approved
**Scope:** ROADMAP.md restructuring — replaces the "Later" bucket with
priority-ordered themes. This is a roadmap design, not an implementation
spec; each feature gets its own brainstorm → spec → plan cycle when work
starts.

## Context

The v0.1–v0.8 arc (tight propose→decide→apply loop, durable results,
scalable canvas) is complete, and v0.9 shipped multi-host plugin
packaging. The roadmap's "Later" bucket held only two stragglers. This
design defines the next arc.

## Decisions

1. **Directions:** deepen the agent loop, and grow canvas/visualization
   power. Explicitly deferred: durable-knowledge features (decision↔code
   links, record re-import) and reach/collaboration (web export, second
   viewer) — neither resonated now; history scrubbing was considered for
   the canvas theme and skipped.
2. **Structure:** unversioned themes rather than versioned milestones.
   Features are priority-ordered inside each theme; version numbers get
   assigned when work starts.
3. **Ordering principle:** leverage-first — features that extend
   existing machinery cheaply and unlock later ones come before riskier,
   protocol-heavy ones. Alternatives considered: quick-wins-first (fast
   visible progress but delays the highest-value agent-loop work) and
   flagship-first (leads with the riskiest, least-explored items).

## Theme 1 — Deepen the agent loop

Priority order and rationale:

1. **Free-form agent questions.** An `ask` op on `update_graph`; the
   agent attaches an open question, optionally to a node; the user
   answers with text in the panel. Answers ride the existing decision
   queue (exactly-once delivery). Exports: unanswered under *Open
   questions*, answered alongside decided choices. *Why first:* smallest
   lift — a choice-like entity on existing pipeline — and a prerequisite
   for multi-agent to feel good.
2. **Implementation tracking.** Node status grows a build lifecycle
   (`planned → building → built → verified`) driven by the agent via
   `update_graph` during the coding phase. Canvas: status rails plus a
   topbar progress summary. Exports gain an implementation-status
   column. *Why second:* extends existing status machinery; turns the
   record into "decided *and* shipped."
3. **Choice dependencies.** Choices declare `depends_on` other choices;
   dependents render locked with a "waiting on X" hint until the parent
   is decided. Re-scoping a dependent after the parent decision is the
   agent's job. Reopening a parent flags decided dependents for review.
   Cycles rejected at the API. *Why third:* real modeling work
   (invalidation, cycles) — benefits from the simpler pipeline
   extensions landing first.
4. **Multi-agent sessions (sketch only).** Per-agent identity declared
   at the MCP layer; color/badge attribution on nodes, choices, and the
   activity feed; `await_decisions` returns only decisions addressed to
   (or unclaimed by) that agent; concurrent awaits on the same session
   become legal. *Why last:* protocol-heaviest, least-explored design
   space; needs its own brainstorm before commitment.

## Theme 2 — Canvas & visualization power

1. **Semantic zoom.** Zoom-tiered rendering on top of the existing
   viewport culling: far out, cards collapse to labeled chips and group
   outlines dominate; mid, title + status; close, the full card with
   description and choice badges. No MCP surface change. *Why first:*
   pure rendering work that helps every graph immediately.
2. **Swimlanes & layers.** An optional `lane` field on nodes
   (agent-assignable, user-overridable); the layered layout constrains
   nodes to labeled horizontal lanes. Separately, toggleable edge-kind
   layers (e.g. show only data-flow edges). *Why second:* small agent-API
   addition plus layout work; benefits from semantic zoom's render
   tiers.
3. **Freehand annotations.** Sticky notes, arrows, and highlight regions
   drawn on the canvas — deliberately not graph structure. Origin-tracked
   like user nodes so they survive agent proposes; delivered to the
   agent as note events; exported in an *Annotations* section. *Why
   last:* independent of the other two and the most new-subsystem
   flavored.

## Theme 3 — Scale & records

The two pre-existing Later items, unchanged: minimap virtualization for
very large graphs, and diffing a session against exported record files.

## Non-goals

- No version assignments until a feature enters implementation.
- No detailed API/schema design here — each feature gets its own spec.
- Multi-agent is a direction marker, not a committed design.
