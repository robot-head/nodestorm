# Concise Logging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace noisy dependency lifecycle output with compact nodestorm connection summaries while preserving useful warnings, errors, and `RUST_LOG` overrides.

**Architecture:** A small `logging` module owns subscriber construction and the default filter. Existing connection registry boundaries emit one application-level event when a client becomes live and one when it becomes disconnected; upstream `rmcp` INFO events are suppressed by default.

**Tech Stack:** Rust 2024, `tracing`, `tracing-subscriber` 0.3, existing unit-test infrastructure.

## Global Constraints

- Use the existing compact, colorized `tracing-subscriber` formatter.
- Default to nodestorm `INFO` and dependencies at `WARN`; an explicit `RUST_LOG` replaces the default.
- Do not add dependencies or parse `rmcp` debug strings.
- Log the non-empty client display title when supplied, otherwise the protocol name, followed by a non-empty version.
- Keep explicitly recorded fields on nodestorm warnings and errors.

---

### Task 1: Compact subscriber and dependency filtering

**Files:**
- Create: `src/logging.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs:5-13`
- Test: `src/logging.rs`

**Interfaces:**
- Produces: `pub fn init()` as the binary's single logging entry point.
- Produces: `const DEFAULT_FILTER: &str = "warn,nodestorm=info"` inside `logging`.
- Consumes: `RUST_LOG` through `tracing_subscriber::EnvFilter::try_from_default_env()`.

- [x] **Step 1: Write a failing filter and formatting test**

Create `src/logging.rs` with a test-only shared writer and a test that constructs the wished-for subscriber, emits nodestorm and rmcp events, and asserts that only the useful lines remain. Add `pub mod logging;` to `src/lib.rs` so the test target compiles the new module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    struct BufferGuard(Buffer);

    impl Write for BufferGuard {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = BufferGuard;

        fn make_writer(&'a self) -> Self::Writer {
            BufferGuard(self.clone())
        }
    }

    #[test]
    fn default_output_is_compact_and_hides_dependency_info() {
        let output = Buffer::default();
        let subscriber = compact_subscriber(
            output.clone(),
            tracing_subscriber::EnvFilter::new(DEFAULT_FILTER),
        );

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "nodestorm", "app ready");
            tracing::info!(target: "rmcp::service", peer_info = "many fields", "initialized");
            tracing::warn!(target: "rmcp::service", "transport warning");
        });

        let rendered = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert!(rendered.contains("app ready"));
        assert!(rendered.contains("transport warning"));
        assert!(!rendered.contains("many fields"));
        assert!(!rendered.contains("rmcp::service"));
        assert!(!rendered.contains("nodestorm:"));
    }
}
```

- [x] **Step 2: Run the new test to verify RED**

Run: `cargo test logging::tests::default_output_is_compact_and_hides_dependency_info --lib`

Expected: compilation fails because `src/logging.rs`, `DEFAULT_FILTER`, and `compact_subscriber` do not exist yet.

- [x] **Step 3: Implement the minimal logging module**

Create `src/logging.rs`:

```rust
use tracing::Subscriber;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

const DEFAULT_FILTER: &str = "warn,nodestorm=info";

fn compact_subscriber<W>(writer: W, filter: EnvFilter) -> impl Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .compact()
        .with_target(false)
        .with_env_filter(filter)
        .with_writer(writer)
        .finish()
}

pub fn init() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    tracing::subscriber::set_global_default(compact_subscriber(std::io::stderr, filter))
        .expect("global tracing subscriber already set");
}
```

Export it from `src/lib.rs`:

```rust
pub mod logging;
```

Replace the current subscriber builder in `src/main.rs` with:

```rust
nodestorm::logging::init();
```

- [x] **Step 4: Run focused and library tests to verify GREEN**

Run: `cargo test logging::tests::default_output_is_compact_and_hides_dependency_info --lib`

Expected: PASS; the rmcp INFO payload and event targets are absent.

Run: `cargo test --lib`

Expected: all library tests PASS.

- [x] **Step 5: Commit the compact subscriber**

```bash
git add src/logging.rs src/lib.rs src/main.rs
git commit -m "feat(logging): add compact subscriber"
```

---

### Task 2: Concise client lifecycle summaries

**Files:**
- Modify: `src/sessions.rs:234-300`
- Modify: `src/server/tools.rs:607-619`
- Test: `src/sessions.rs`

**Interfaces:**
- Consumes: `ConnectionInfo { client_name, version, .. }` already stored by `Sessions`.
- Produces: private `client_label(title: Option<&str>, client_name: &str, version: &str) -> String`.
- Changes: `Sessions::disconnect_client(ConnectionId) -> Option<ConnectionInfo>` returns the identity only for the first live-to-disconnected transition; existing callers may ignore the return value.

- [x] **Step 1: Write failing label and disconnect-transition tests**

Add these tests to `src/sessions.rs`:

```rust
#[test]
fn client_label_prefers_title_and_omits_an_empty_version() {
    assert_eq!(
        client_label(Some("Claude Code"), "claude-code", "2.1.215"),
        "Claude Code 2.1.215"
    );
    assert_eq!(client_label(Some("   "), "claude-code", ""), "claude-code");
    assert_eq!(client_label(None, "claude-code", "   "), "claude-code");
}

#[test]
fn disconnect_reports_only_the_first_live_transition() {
    let root = tmp_root("disconnect-transition");
    let sessions = Sessions::open(root.join("sessions"), None).unwrap();
    let id = sessions.next_connection_id();
    sessions.connect_client(id, "claude-code".into(), "2.1.215".into());

    assert_eq!(
        sessions.disconnect_client(id).map(|info| info.client_name),
        Some("claude-code".into())
    );
    assert!(sessions.disconnect_client(id).is_none());
    let _ = std::fs::remove_dir_all(root);
}
```

- [x] **Step 2: Run the tests to verify RED**

Run: `cargo test sessions::tests:: --lib`

Expected: compilation fails because `client_label` does not exist and `disconnect_client` returns `()`.

- [x] **Step 3: Add minimal lifecycle logging**

Add the label helper near the connection types:

```rust
fn client_label(title: Option<&str>, client_name: &str, version: &str) -> String {
    let client_name = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(client_name);
    match version.trim() {
        "" => client_name.to_owned(),
        version => format!("{client_name} {version}"),
    }
}
```

Keep the original no-title API as a fallback, and add the MCP-specific entry point that accepts the optional client title. Store the rendered label privately with the registry entry so disconnect uses the same identity:

```rust
pub fn connect_client(&self, id: ConnectionId, client_name: String, version: String) {
    self.connect_client_with_title(id, client_name, version, None);
}

pub(crate) fn connect_client_with_title(
    &self,
    id: ConnectionId,
    client_name: String,
    version: String,
    title: Option<String>,
) {
    let info = ConnectionInfo {
        id,
        client_name,
        version,
        state: ConnectionState::Connected,
    };
    let log_label = client_label(title.as_deref(), &info.client_name, &info.version);
    self.connections.lock().expect("connections mutex poisoned").insert(
        id,
        RegistryEntry { info, log_label: log_label.clone(), live: true },
    );
    tracing::info!("{log_label} connected");
    self.bump_connections();
}
```

In `NodestormServer::on_initialized`, call `connect_client_with_title` and pass `info.client_info.title.clone()` after the existing name and version arguments.

Change `disconnect_client` to return and log only the actual transition:

```rust
pub fn disconnect_client(&self, id: ConnectionId) -> Option<ConnectionInfo> {
    let disconnected = self
        .connections
        .lock()
        .expect("connections mutex poisoned")
        .get_mut(&id)
        .and_then(|entry| {
            if !entry.live {
                return None;
            }
            entry.live = false;
            Some((entry.info.clone(), entry.log_label.clone()))
        });
    if let Some((_, log_label)) = &disconnected {
        tracing::info!("{log_label} disconnected");
        self.bump_connections();
    }
    disconnected.map(|(info, _)| info)
}
```

- [x] **Step 4: Run focused and full tests to verify GREEN**

Run: `cargo test sessions::tests::client_label_prefers_title_and_omits_an_empty_version --lib`

Expected: PASS.

Run: `cargo test sessions::tests::disconnect_reports_only_the_first_live_transition --lib`

Expected: PASS.

Run: `cargo test --all-targets`

Expected: all tests PASS with no new warnings.

- [x] **Step 5: Inspect real headless output**

Run nodestorm headless with a temporary port and connect the existing MCP round-trip client or Claude Code. Confirm the normal output contains compact nodestorm startup/connection lines, does not contain `peer_info=Some(InitializeRequestParams`, and reveals upstream INFO events when started with `RUST_LOG=rmcp=info`.

- [x] **Step 6: Commit lifecycle summaries**

```bash
git add src/sessions.rs
git commit -m "feat(logging): summarize MCP connections"
```
