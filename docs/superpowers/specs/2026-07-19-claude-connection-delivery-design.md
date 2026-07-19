# Claude connection status and targeted delivery

Approved 2026-07-19.

## Goal

Show every incoming Claude MCP session and deliver each user response only to
the Claude session that is waiting for it. Make Send report a deterministic
receipt state, recover interrupted delivery after a reconnect, and surface the
full error in a toast without consuming or rerouting queued decisions.

## Scope

Track MCP transport sessions independently from named brainstorm sessions. A
transport session has an internal connection id, MCP client name and version,
an optional agent id, and a live state. A waiting connection is also associated
with the canonical brainstorm name passed to `await_decisions`.

Preserve the existing decision ownership rules: a named agent receives events
owned by that agent plus unclaimed events, while a single anonymous agent
receives the complete batch. Do not add a recipient picker or require the user
to understand transport ids.

Do not add dependencies, change the persisted document schema, or expose
internal connection ids through MCP tools.

## Considered approaches

1. Extend the existing store delivery path with connection-scoped waiters and
   send receipts. This is the selected approach because it preserves the
   store's exactly-once cursors and keeps delivery decisions atomic with queue
   consumption.
2. Add a separate message broker between the UI and `await_decisions`. This
   gives transport concerns a clean boundary, but duplicates queueing,
   filtering, timeout, and exactly-once behavior already owned by `Store`.
3. Track connections only for display and retain agent-string delivery. This is
   the smallest UI diff, but two transports can claim the same agent or omit an
   agent id, so it cannot guarantee delivery to the correct Claude session.

## Connection lifecycle

Each `NodestormServer` instance represents one initialized streamable-HTTP MCP
session. Initialization registers its internal connection id and the client's
MCP implementation name/version. The registration is shared by clones and is
removed only when the final server instance for that transport is dropped.

Calling `await_decisions` moves the connection to `Waiting` and records the
canonical brainstorm name and optional agent id. While its response is being
prepared it is `Receiving`; after a normal timeout it returns to `Connected`.
A live connection that has not called `await_decisions` remains `Connected` and
is never selected merely because it exists.

Connection changes use their own watch channel. They must not reuse the named
session generation channel because that channel resets canvas selection and
other per-session UI state.

## Recipient identity and routing

Live transport connection ids answer where a response can be delivered now.
Stable recipient keys answer who may recover an interrupted response:

- A named waiter uses `agent:<agent-id>`.
- An anonymous waiter uses a generated anonymous key for its live transport.

On explicit Send, the active store snapshots its current waiters and the end of
the decision-log batch. Distinct named agents become distinct receipt targets;
each receives only its owned events plus unclaimed events. One anonymous waiter
may receive the complete batch.

The store rejects a Send without changing delivery cursors when routing is not
deterministic. Error cases include no waiter on the active brainstorm, multiple
live connections claiming the same named agent, multiple anonymous waiters, or
a mixture of named and anonymous waiters. This is intentionally stricter than
guessing from client name, connection order, or the most recent tool call.

The receipt's fixed batch end prevents user edits made during delivery from
riding on the in-flight response. Those later events remain queued for the next
Send. Delivery cursors advance only after every target in the receipt consumes
its assigned slice, so partial delivery cannot make the remaining target lose
its response.

The existing last-choice autoflush remains supported. When no waiter exists at
autoflush time, the flushed batch remains claimable rather than failing a
button interaction. A later unambiguous waiting cohort claims it using the same
named-agent or sole-anonymous rules.

## Send receipt states

One transient receipt per store records its batch boundary, recipient keys,
and per-recipient completion. The UI derives the Send label from that receipt:

- `Send` before a request or after the queue/waiting cohort changes.
- `Sending...` while at least one live target has not consumed its slice.
- `Sent` after every target consumes its slice.
- `Reconnecting...` when an unfinished target has no live connection.
- `Failed - Retry` after a non-recoverable routing or delivery error.

These transitions are driven by store and connection events, never by a timer.
A successful receipt remains visible until new queued work or a new waiting
cohort makes another Send meaningful. A failed explicit Send keeps all events
queued and can be retried after the connection state is corrected.

`Sent` means Nodestorm successfully handed each targeted batch to the matching
`await_decisions` request and constructed its MCP result. MCP has no separate
application-level acknowledgement proving that Claude acted on the result, so
the UI must not claim that stronger guarantee.

## Reconnection recovery

A dropped transport never causes its unfinished batch to be consumed or sent
to a different agent. Its receipt target becomes orphaned and the connection
indicator retains it as `Reconnecting`.

When a new connection calls `await_decisions` for the same brainstorm and the
same named agent, it claims that agent's orphaned target and delivery resumes.
An anonymous connection may claim an orphaned anonymous target only when there
is exactly one anonymous orphan and one anonymous claimant for that brainstorm.
Ambiguity leaves the receipt untouched and produces an error.

The existing persisted flush sequence and delivery cursors keep queued
decisions recoverable if the application itself restarts. Connection rows and
button presentation are transient; after an app restart a later unambiguous
`await_decisions` call reconstructs the actionable delivery state from the
persisted queue rather than restoring stale transport ids.

## User interface

Add a compact Claude connection indicator beside Send. With no incoming MCP
session it shows a disconnected/zero state. Its popover lists every live
connection with client name/version, agent identity when present, canonical
brainstorm name when waiting, and one of `Connected`, `Waiting`, `Receiving`,
or `Reconnecting`. A disconnected entry disappears unless it owns an
unfinished receipt; recoverable entries remain until reclaimed or resolved.

Both the main Send button and the compose-popover Send button use the same
receipt-driven action and label. The optional comment is cleared only after a
Send request is accepted; a rejected Send leaves it available for retry.

Render one app-level toast for the latest delivery error. It uses
`role="alert"`, includes the concrete error message, and has a dismiss button.
A transport drop during delivery produces a warning toast explaining that the
response is waiting for reconnection, while retaining the recoverable receipt.
Routing ambiguity and other non-recoverable errors produce an error toast and
the `Failed - Retry` button state. Toasts do not disappear on an arbitrary
timeout.

## Components and boundaries

- `src/server/tools.rs` owns MCP initialization metadata and updates the
  connection registration around `await_decisions`.
- `src/sessions.rs` owns the global connection registry, its snapshot, and its
  independent watch channel because the UI and every MCP server instance
  already share `Arc<Sessions>`.
- `src/store.rs` owns waiter registration, recipient validation, fixed batch
  boundaries, exactly-once consumption, reconnectable receipts, and delivery
  errors exposed through `UiMeta`.
- `src/ui/app.rs` bridges the independent connection watch channel and mounts
  the app-level toast.
- `src/ui/topbar.rs` renders the connection indicator and the shared
  receipt-driven Send controls.
- `assets/main.css` styles the indicator, status rows, button states, and toast
  using existing theme tokens.

## Error handling

All routing validation happens before incrementing a flush or moving a cursor.
The error text names the brainstorm and conflicting/missing recipient identity
without exposing internal transport ids. Serialization or shutdown errors also
leave undelivered decisions intact whenever consumption has not completed.

Connection cleanup is guard-based so cancellation, timeout, client shutdown,
and dropped futures cannot leave a session permanently marked `Waiting`.
Recoverable disconnects are status transitions, not destructive cleanup.

## Verification

Follow test-driven development for each behavior:

- Store tests prove fixed batch boundaries, distinct-agent routing, shared
  event delivery, no-cursor movement on ambiguity, receipt state transitions,
  partial completion, and anonymous/named reconnect claims.
- Session registry tests prove initialization metadata, independent change
  notification, clone-safe cleanup, and retention of orphaned receipt rows.
- MCP round-trip tests use two real clients to prove each receives only its
  owned decisions, a wrong or duplicate claimant receives nothing, and a new
  connection with the same agent id recovers an interrupted delivery.
- UI contract tests cover connection labels, deterministic Send labels, shared
  send handling, and accessible error-toast markup and styling.
- Run focused tests first, then formatting, the full Rust test suite, Clippy,
  and the repository's existing release-gate checks.
