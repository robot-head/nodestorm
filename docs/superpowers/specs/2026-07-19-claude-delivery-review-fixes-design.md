# Claude Delivery Review Fixes Design

## Goal

Address the two unresolved review findings from merged PR #29 without changing
the public MCP tool contract or persisted session schema.

## Receiving lifecycle

`await_decisions` currently installs a guard that resets a waiting connection
to `Connected` on every return path. A delivered response sets `Receiving`
before that guard drops, so the guard immediately overwrites the intended
state.

On successful delivery, explicitly drop the reset guard before setting
`Receiving`. Timeout and error paths retain the existing automatic reset to
`Connected`. A subsequent await changes the state to `Waiting`, and transport
disconnect removes the live row as before.

## Absent-agent delivery

Named agents already have durable delivery positions in `agent_flush` and
`agent_cursors`. When a named agent registers, determine pending work from its
own `agent_flush` position instead of the global `delivered_flush_seq`. This
allows an agent that was absent during Send to create a claimable receipt and
receive its addressed events later, even after connected recipients completed
the global receipt.

Anonymous delivery continues to use `delivered_flush_seq` because anonymous
connections have no stable recipient key.

## Verification

- An MCP integration test observes `Receiving` after a delivered response.
- A store test sends while the addressed agent is absent, then proves the
  reestablished named agent receives its event exactly once.
- The complete Rust test, lint, formatting, and diff checks remain clean.
