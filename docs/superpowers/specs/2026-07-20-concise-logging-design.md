# Concise Logging Design

## Goal

Make nodestorm's terminal output concise and easy to scan while retaining useful application lifecycle and error information. Routine `rmcp` internals must not print opaque debug structures full of unset fields.

## Design

Use `tracing-subscriber`'s existing compact, colorized formatter. The default filter will show nodestorm events at `INFO` and dependency events, including `rmcp`, at `WARN` or above. An explicit `RUST_LOG` value will continue to replace this default for protocol debugging.

Nodestorm will emit its own connection lifecycle events at the existing application boundaries where structured client information is already available:

```text
INFO  Claude Code 2.1.215 connected
INFO  Claude Code 2.1.215 disconnected
```

The summaries will include only the client display name and non-empty version. They will not copy capabilities, protocol metadata, optional URLs, icons, or transport session identifiers from `rmcp` debug output. Other nodestorm warnings and errors will keep their explicitly recorded fields.

No new logging dependency or parser for dependency-generated debug strings will be introduced.

## Data Flow

1. `main` installs the compact subscriber with the concise default filter.
2. MCP initialization reaches `NodestormServer::on_initialized`, which already receives the client name and version and registers the connection.
3. The application emits one concise connected event after successful registration.
4. The centralized disconnect path retrieves the registered client identity and emits one concise disconnected event only when a live connection actually transitions to disconnected.

## Error Handling

Warnings and errors remain visible. Invalid or missing optional identity data will not prevent connection tracking; the summary will omit an empty version and use the available client name. `RUST_LOG=rmcp=info` remains the escape hatch for full upstream lifecycle diagnostics.

## Testing

Tests will verify:

- the default filter enables nodestorm `INFO` and suppresses `rmcp` `INFO` while retaining `rmcp` warnings;
- concise formatting does not include targets or source locations in the normal layout;
- connection summaries contain the client name and optional non-empty version;
- disconnect logging occurs once for a real live-to-disconnected transition, not for repeated cleanup paths.

