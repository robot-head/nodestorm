# Long-content resilience implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep hostile long strings contained and readable while making every
meaningful user-facing session, node, edge, choice, note, and activity value
discoverable in the interface.

**Architecture:** Preserve compact canvas previews and make existing detail
surfaces complete. Native CSS handles wrapping, clamping, and bounded scrolling;
small pure Rust helpers cap SVG edge previews and format complete connection
details for focused tests.

**Tech Stack:** Rust 2024, Dioxus desktop/WebView, CSS, existing Rust unit tests,
existing MCP HTTP interface, Xvfb/WebKitGTK rendered verification.

## Global Constraints

- Add no dependency and change no MCP schema, persistence format, export format,
  or theme palette block.
- Node labels clamp to 3 lines; node descriptions remain clamped to 4 lines.
- Canvas edge labels longer than 32 characters render as 31 characters plus `…`.
- The selected-node panel is 360px when available and never wider than the
  viewport.
- Pros and cons stack at viewport widths of 420px or narrower.
- Session and More menus remain bounded by the viewport and vertically
  scrollable.
- Expanded Activity exposes all retained entries (the store cap remains 200).
- Existing topbar accessible names and responsive container-query folds remain
  unchanged.
- Styling must use existing semantic theme tokens and remain valid across all
  twelve palette families and Auto, Light, and Dark modes.

---

### Task 1: Contain compact card previews

**Files:**
- Modify: `src/layout.rs:130-170,986-999`
- Modify: `src/ui/node_card.rs:28-42,103-127`
- Modify: `src/ui/cluster_card.rs:27-36`
- Modify: `assets/main.css:1181-1320`

**Interfaces:**
- Consumes: `layout::wrap_lines(&str, usize) -> usize` and the existing 260px
  card geometry.
- Produces: a three-line node/cluster label preview whose layout height is
  bounded consistently with CSS; complete values remain in title text.

- [ ] **Step 1: Write the failing layout test**

Add beside the existing height tests in `src/layout.rs`:

```rust
#[test]
fn height_caps_long_labels_at_three_lines() {
    let mut three_lines = node("three-lines");
    three_lines.label = "x".repeat(66);
    let mut ten_lines = node("ten-lines");
    ten_lines.label = "x".repeat(220);

    assert_eq!(estimate_height(&three_lines), estimate_height(&ten_lines));
}
```

- [ ] **Step 2: Run the test and verify the expected failure**

Run:

```bash
cargo test layout::tests::height_caps_long_labels_at_three_lines -- --exact
```

Expected: FAIL because the 220-character label is currently estimated at ten
lines while the 66-character label is estimated at three.

- [ ] **Step 3: Cap the layout estimate at three label lines**

Replace the label calculation in `estimate_height`:

```rust
let label_lines = wrap_lines(&node.label, 22).min(3);
let label_extra = label_lines.saturating_sub(1);
```

Update the function and CSS geometry comments to say labels clamp to three
lines and descriptions clamp to four.

- [ ] **Step 4: Apply matching card CSS and complete title text**

Make `kind_label` in `src/ui/node_card.rs` reusable by the detail panel:

```rust
pub(crate) fn kind_label(kind: NodeKind) -> &'static str {
```

Add complete values to the rendered card fields:

```rust
span {
    class: "node-label",
    title: "{node.label}",
    "{node.label}"
}
```

```rust
span {
    class: "node-group",
    title: "{group} — collapse this group into one card",
    onclick: move |ev| {
        ev.stop_propagation();
        on_toggle_group.call(ev);
    },
    "{group}"
}
```

```rust
p {
    class: "node-desc",
    title: "{node.description}",
    "{node.description}"
}
```

Give the cluster label the same complete title in `cluster_card.rs`:

```rust
span { class: "node-label", title: "{group}", "{group}" }
```

Extend the card rules in `assets/main.css`:

```css
.node-glyph,
.node-kind,
.node-status-tag {
  flex: 0 0 auto;
}

.node-label {
  min-width: 0;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
  overflow-wrap: anywhere;
}

.node-meta {
  min-width: 0;
}

.node-group {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.node-desc {
  overflow-wrap: anywhere;
}
```

- [ ] **Step 5: Verify the focused behavior and all layout tests**

Run:

```bash
cargo test layout::tests::height_caps_long_labels_at_three_lines -- --exact
cargo test layout::tests
```

Expected: PASS; existing packing, collapsed-cluster, culling, and routing tests
remain green.

- [ ] **Step 6: Commit the card containment change**

```bash
git add src/layout.rs src/ui/node_card.rs src/ui/cluster_card.rs assets/main.css
git commit -m "fix(ui): contain long card text"
```

---

### Task 2: Expose complete node and edge details

**Files:**
- Modify: `src/ui/edge_layer.rs:1-114`
- Modify: `src/ui/choice_panel.rs:13-110,203-224`
- Modify: `assets/main.css:356-540,1173-1179`

**Interfaces:**
- Consumes: `model::Edge`, `model::SessionDoc`,
  `node_card::kind_label`, and `export::status_name_pub`.
- Produces: `edge_label_preview(&str) -> String` and
  `connection_display(&NodeId, &Edge, &SessionDoc) -> ConnectionDisplay`.

- [ ] **Step 1: Write failing edge-preview tests**

Add to `src/ui/edge_layer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_edge_labels_are_unchanged() {
        assert_eq!(edge_label_preview("read/write"), "read/write");
    }

    #[test]
    fn long_edge_labels_are_unicode_safe_and_32_chars() {
        let label = format!("{}éé", "a".repeat(31));
        let preview = edge_label_preview(&label);

        assert_eq!(preview.chars().count(), 32);
        assert_eq!(preview, format!("{}…", "a".repeat(31)));
    }
}
```

- [ ] **Step 2: Run the preview tests and verify they fail**

Run:

```bash
cargo test ui::edge_layer::tests
```

Expected: compilation FAIL because `edge_label_preview` does not exist.

- [ ] **Step 3: Implement and render the bounded edge preview**

Add above `EdgeLayer`:

```rust
const MAX_EDGE_LABEL_CHARS: usize = 32;

fn edge_label_preview(label: &str) -> String {
    if label.chars().count() <= MAX_EDGE_LABEL_CHARS {
        return label.to_owned();
    }
    label
        .chars()
        .take(MAX_EDGE_LABEL_CHARS - 1)
        .chain(std::iter::once('…'))
        .collect()
}
```

Render the preview while retaining complete SVG title text:

```rust
text {
    key: "label-{e.from}-{e.to}-{i}",
    class: "edge-label",
    x: "{e.label_pos.x}",
    y: "{e.label_pos.y}",
    text_anchor: "middle",
    title { "{label}" }
    {edge_label_preview(label)}
}
```

- [ ] **Step 4: Write failing complete-connection tests**

Import `Edge` and `ElementStatus` into `choice_panel.rs`, define the desired
display type signature, and add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ElementStatus, Origin};

    fn test_node(id: &str, label: &str) -> Node {
        Node {
            id: NodeId::from(id),
            label: label.into(),
            kind: NodeKind::Component,
            description: String::new(),
            status: ElementStatus::Existing,
            group: None,
            choices: vec![],
            notes: vec![],
            position: None,
            origin: Origin::Agent,
        }
    }

    fn test_doc() -> SessionDoc {
        SessionDoc {
            nodes: vec![test_node("api", "Public API"), test_node("queue", "Job Queue")],
            ..Default::default()
        }
    }

    fn test_edge() -> Edge {
        Edge {
            from: NodeId::from("api"),
            to: NodeId::from("queue"),
            kind: EdgeKind::DataFlow,
            label: Some("CompleteSecurityAuditEnvelopeIdentifier".into()),
            status: ElementStatus::Modified,
            origin: Origin::Agent,
        }
    }

    #[test]
    fn connection_display_exposes_complete_outgoing_edge() {
        let display = connection_display(&NodeId::from("api"), &test_edge(), &test_doc());

        assert_eq!(display.direction, "Outgoing to");
        assert_eq!(display.endpoint, "Job Queue");
        assert_eq!(display.kind, "data flow");
        assert_eq!(display.status, "modified");
        assert_eq!(
            display.label.as_deref(),
            Some("CompleteSecurityAuditEnvelopeIdentifier")
        );
    }

    #[test]
    fn connection_display_exposes_incoming_direction() {
        let display = connection_display(&NodeId::from("queue"), &test_edge(), &test_doc());

        assert_eq!(display.direction, "Incoming from");
        assert_eq!(display.endpoint, "Public API");
    }
}
```

- [ ] **Step 5: Run the connection tests and verify they fail**

Run:

```bash
cargo test ui::choice_panel::tests
```

Expected: compilation FAIL because `ConnectionDisplay` and
`connection_display` do not exist.

- [ ] **Step 6: Implement the pure formatter**

Add to `choice_panel.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConnectionDisplay {
    direction: &'static str,
    endpoint: String,
    kind: &'static str,
    status: &'static str,
    label: Option<String>,
}

fn connection_display(
    selected: &NodeId,
    edge: &Edge,
    doc: &SessionDoc,
) -> ConnectionDisplay {
    let (direction, endpoint_id) = if &edge.from == selected {
        ("Outgoing to", &edge.to)
    } else {
        ("Incoming from", &edge.from)
    };
    ConnectionDisplay {
        direction,
        endpoint: doc
            .node(endpoint_id)
            .map_or_else(|| endpoint_id.to_string(), |node| node.label.clone()),
        kind: edge_kind_phrase(edge.kind),
        status: crate::export::status_name_pub(edge.status),
        label: edge.label.clone(),
    }
}
```

Replace the incident tuple with `Vec<(Edge, ConnectionDisplay)>`, keeping each
edge clone for deletion:

```rust
let incident: Vec<(Edge, ConnectionDisplay)> = {
    let d = doc.read();
    d.edges
        .iter()
        .filter(|edge| edge.from == node.id || edge.to == node.id)
        .map(|edge| {
            (
                edge.clone(),
                connection_display(&node.id, edge, &d),
            )
        })
        .collect()
};
```

Render each connection as:

```rust
div { class: "conn-row", key: "{edge.from}-{edge.to}-{edge.kind:?}",
    div { class: "conn-content",
        div { class: "conn-primary",
            span { class: "conn-direction", "{display.direction}" }
            span { class: "conn-endpoint", "{display.endpoint}" }
        }
        div { class: "conn-meta", "{display.kind} · {display.status}" }
        if let Some(label) = &display.label {
            div { class: "conn-label", "{label}" }
        }
    }
    button {
        class: "ctl-btn",
        title: "Delete this edge",
        onclick: {
            let store = store.clone();
            let from = edge.from.clone();
            let to = edge.to.clone();
            let kind = edge.kind;
            move |_| {
                if let Err(err) = store.delete_edge(&from, &to, kind) {
                    tracing::warn!(%err, "delete_edge failed");
                }
            }
        },
        "✕"
    }
}
```

- [ ] **Step 7: Add complete node metadata to the panel**

Immediately below `.panel-head`, add:

```rust
dl { class: "panel-meta",
    div { class: "panel-meta-row",
        dt { "ID" }
        dd { code { "{node.id}" } }
    }
    div { class: "panel-meta-row",
        dt { "Kind" }
        dd { "{super::node_card::kind_label(node.kind)}" }
    }
    div { class: "panel-meta-row",
        dt { "Status" }
        dd { "{super::node_card::status_class(node.status)}" }
    }
    if let Some(group) = &node.group {
        div { class: "panel-meta-row",
            dt { "Group" }
            dd { "{group}" }
        }
    }
}
```

- [ ] **Step 8: Style metadata and resilient connection rows**

Add to `assets/main.css`:

```css
.panel-meta {
  display: grid;
  gap: 4px;
  margin: 8px 0 10px;
  font-size: 11.5px;
}

.panel-meta-row {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr);
  gap: 8px;
}

.panel-meta dt {
  color: var(--text-dim);
}

.panel-meta dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}

.conn-row {
  align-items: flex-start;
}

.conn-content {
  min-width: 0;
  flex: 1;
  overflow-wrap: anywhere;
}

.conn-primary {
  display: flex;
  gap: 4px;
}

.conn-direction,
.conn-meta {
  color: var(--text-dim);
}

.conn-endpoint {
  min-width: 0;
  font-weight: 600;
  overflow-wrap: anywhere;
}

.conn-meta,
.conn-label {
  margin-top: 2px;
  font-size: 11.5px;
}

.conn-row > .ctl-btn {
  flex: 0 0 auto;
}
```

- [ ] **Step 9: Verify and commit complete edge discovery**

Run:

```bash
cargo test ui::edge_layer::tests
cargo test ui::choice_panel::tests
cargo test
```

Expected: PASS.

```bash
git add src/ui/edge_layer.rs src/ui/choice_panel.rs assets/main.css
git commit -m "fix(ui): expose full node and edge data"
```

---

### Task 3: Make panels, menus, choices, and activity resilient

**Files:**
- Modify: `src/ui/activity.rs:8-49`
- Modify: `src/ui/topbar.rs:51-226,260-260`
- Modify: `assets/main.css:40-940,1015-1065,1710-1810`
- Modify: `src/theme.rs:127-240`

**Interfaces:**
- Consumes: the existing `UiMeta.activity: Vec<ActivityEntry>` capped at 200
  by `store::ACTIVITY_CAP`.
- Produces: `entry_count(total: usize, expanded: bool) -> usize`, a complete
  session-menu title surface, and CSS overflow contracts for all dynamic text.

- [ ] **Step 1: Write the failing activity-discovery test**

Add to `src/ui/activity.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_feed_includes_every_retained_entry() {
        assert_eq!(entry_count(200, true), 200);
        assert_eq!(entry_count(200, false), 1);
        assert_eq!(entry_count(0, false), 0);
    }
}
```

- [ ] **Step 2: Run the activity test and verify it fails**

Run:

```bash
cargo test ui::activity::tests::expanded_feed_includes_every_retained_entry -- --exact
```

Expected: compilation FAIL because `entry_count` does not exist.

- [ ] **Step 3: Show every retained activity entry when expanded**

Replace `EXPANDED_COUNT` with:

```rust
fn entry_count(total: usize, expanded: bool) -> usize {
    if expanded {
        total
    } else {
        total.min(COLLAPSED_COUNT)
    }
}
```

Use it in the component and expose expanded state to CSS:

```rust
let count = entry_count(m.activity.len(), expanded());
```

```rust
div { class: if expanded() { "activity expanded" } else { "activity" },
```

- [ ] **Step 4: Write a failing stylesheet contract test**

Extend `src/theme.rs` tests with:

```rust
fn assert_block_contains(selector: &str, declaration: &str) {
    let block = block_for(selector);
    assert!(
        block.contains(declaration),
        "{selector} must contain `{declaration}`, got: {block}"
    );
}

#[test]
fn long_content_surfaces_have_overflow_contracts() {
    assert_block_contains(".panel {", "width: min(360px, 100vw)");
    assert_block_contains(".panel {", "overflow-x: hidden");
    assert_block_contains(".panel-head h2 {", "overflow-wrap: anywhere");
    assert_block_contains(".option-label {", "overflow-wrap: anywhere");
    assert_block_contains(".export-dropdown {", "max-height: calc(100vh - 64px)");
    assert_block_contains(".export-dropdown {", "overflow-y: auto");
    assert_block_contains(".activity.expanded {", "overflow-y: auto");
    assert_block_contains(".activity-text {", "overflow-wrap: anywhere");
    assert_block_contains(".diff-text {", "overflow-wrap: anywhere");
    assert_block_contains(".empty-cmd {", "max-width: 100%");
}
```

- [ ] **Step 5: Run the CSS contract and verify it fails**

Run:

```bash
cargo test theme::tests::long_content_surfaces_have_overflow_contracts -- --exact
```

Expected: FAIL on the first missing long-content declaration.

- [ ] **Step 6: Make the document title and session names discoverable**

In `topbar.rs`, add complete title text to the compact topbar:

```rust
span { class: "topbar-title", title: "{title}", "{title}" }
```

At the top of `.sessions-dropdown`, add:

```rust
div { class: "session-doc-title", title: "{title}",
    span { "Brainstorm" }
    strong { "{title}" }
}
```

Give every live session name complete title text:

```rust
span { class: "sess-name", title: "{info.name}", "{info.name}" }
```

For archived rows, interpolate the complete value into the existing action
title:

```rust
title: "Restore archived session {name}",
```

- [ ] **Step 7: Add the complete wrapping and bounded-scroll CSS**

Modify the existing blocks and add the following focused rules:

```css
.empty-state {
  min-width: 0;
  padding: 16px;
}

.empty-cmd {
  max-width: 100%;
  flex-wrap: wrap;
  justify-content: center;
  overflow-wrap: anywhere;
}

.empty-state code {
  max-width: 100%;
  overflow-wrap: anywhere;
  word-break: break-word;
}

.panel {
  width: min(360px, 100vw);
  overflow-x: hidden;
  overflow-wrap: anywhere;
}

.panel-head {
  align-items: flex-start;
}

.panel-head h2 {
  min-width: 0;
  overflow-wrap: anywhere;
}

.panel-head .ctl-btn {
  flex: 0 0 auto;
}

.panel-actions {
  flex-wrap: wrap;
}

.choice-head h3,
.choice-rationale,
.option-label,
.option-summary,
.pros-cons li,
.option-affects,
.note,
.timeline-text,
.diff-text {
  min-width: 0;
  overflow-wrap: anywhere;
}

.option-label {
  overflow-wrap: anywhere;
}

.option-rec,
.option-radio,
.timeline-time,
.activity-time,
.activity-dot {
  flex: 0 0 auto;
}

.pros-cons ul {
  min-width: 0;
}

@media (max-width: 420px) {
  .pros-cons {
    flex-direction: column;
  }
}

.export-dropdown {
  max-height: calc(100vh - 64px);
  overflow-x: hidden;
  overflow-y: auto;
}

.session-doc-title {
  display: grid;
  gap: 2px;
  padding: 6px 10px 8px;
  border-bottom: 1px solid var(--border);
  overflow-wrap: anywhere;
}

.session-doc-title span {
  color: var(--text-dim);
  font-size: 10.5px;
  text-transform: uppercase;
  letter-spacing: 0.08em;
}

.sess-name {
  min-width: 0;
  overflow-wrap: anywhere;
}

.sess-badges,
.session-row > .ctl-btn {
  flex: 0 0 auto;
}

.activity {
  max-width: min(420px, calc(100vw - 28px));
  overflow-wrap: anywhere;
}

.activity.expanded {
  max-height: min(60vh, 520px);
  overflow-x: hidden;
  overflow-y: auto;
}

.activity-entry {
  align-items: flex-start;
}

.activity-text {
  min-width: 0;
  overflow-wrap: anywhere;
}

.diff-text {
  overflow-wrap: anywhere;
}
```

If a selector already exists, merge the declarations into that block instead
of creating duplicate blocks. Keep the responsive topbar section last.

- [ ] **Step 8: Verify activity, CSS, and the complete Rust suite**

Run:

```bash
cargo test ui::activity::tests::expanded_feed_includes_every_retained_entry -- --exact
cargo test theme::tests::long_content_surfaces_have_overflow_contracts -- --exact
cargo test
```

Expected: PASS.

- [ ] **Step 9: Commit resilient detail surfaces**

```bash
git add src/ui/activity.rs src/ui/topbar.rs src/theme.rs assets/main.css
git commit -m "fix(ui): keep long detail text readable"
```

---

### Task 4: Verify hostile content in the rendered application

**Files:**
- Verify only: `src/ui/*.rs`, `src/layout.rs`, `assets/main.css`
- Evidence only (do not commit): `/tmp/nodestorm-ux-after-wide.png`,
  `/tmp/nodestorm-ux-after-panel.png`, `/tmp/nodestorm-ux-after-session.png`,
  `/tmp/nodestorm-ux-after-narrow.png`, `/tmp/nodestorm-ux-after-light.png`

**Interfaces:**
- Consumes: the existing `nodestorm` binary and streamable-HTTP MCP endpoint.
- Produces: automated verification output and rendered before/after evidence for
  the approved design requirements.

- [ ] **Step 1: Run formatting, linting, and the complete test suite**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

Expected: all commands exit 0 with no warnings or whitespace errors.

- [ ] **Step 2: Launch an isolated rendered test instance**

Run Xvfb on an unused display, then launch the app in a separate terminal
session:

```bash
Xvfb :98 -screen 0 1280x840x24
```

```bash
DISPLAY=:98 WEBKIT_DISABLE_DMABUF_RENDERER=1 dbus-run-session -- \
  cargo run -- --demo --window-size 1280x840 \
  --sessions-dir /tmp/nodestorm-ux-sessions \
  --prefs /tmp/nodestorm-ux-prefs.json --port 4848
```

Expected: the app logs the MCP URL and renders at 1280x840.

- [ ] **Step 3: Inject the hostile stress session over MCP**

Run this exact initialization sequence and payload:

```bash
UX_INIT_RESPONSE=$(curl -sS -i -X POST http://127.0.0.1:4848/mcp \
  -H 'Accept: application/json, text/event-stream' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ux-audit","version":"1"}}}')
UX_MCP_SESSION_ID=$(printf '%s\n' "$UX_INIT_RESPONSE" | \
  sed -n 's/^mcp-session-id: \([^[:space:]]*\).*/\1/p' | tr -d '\r')
test -n "$UX_MCP_SESSION_ID"

curl -sS -X POST http://127.0.0.1:4848/mcp \
  -H 'Accept: application/json, text/event-stream' \
  -H 'Content-Type: application/json' \
  -H "Mcp-Session-Id: $UX_MCP_SESSION_ID" \
  --data '{"jsonrpc":"2.0","method":"notifications/initialized"}'

jq -nc '{
  jsonrpc:"2.0", id:2, method:"tools/call",
  params:{name:"propose_graph",arguments:{
    title:"EnterpriseArchitectureModernizationProgramForGloballyDistributedMultiTenantMissionCriticalWorkloadsWithZeroDowntimeMigration",
    announce:"ImportedTheCompleteEnterpriseArchitectureModernizationProposalWithEveryDecisionAndDependencyReadyForHumanReview",
    focus:"identity-orchestration",
    nodes:[
      {
        id:"identity-orchestration",
        label:"IdentityAuthenticationAuthorizationAndFineGrainedPolicyOrchestrationGatewayForEveryInternalAndExternalConsumer",
        kind:"service", status:"proposed",
        group:"PlatformSecurityIdentityAndComplianceFoundationsSharedAcrossEveryBusinessUnit",
        description:"CoordinatesOIDCOAuth2SAMLWebAuthnSCIMAndPolicyEvaluationAcrosshttps://identity.example.invalid/organizations/0123456789abcdefghijklmnopqrstuvwxyz/realms/production-critical-workloads-with-a-very-long-path",
        choices:[{
          id:"credential-and-session-strategy",
          prompt:"WhichCredentialAndSessionLifecycleStrategyShouldGovernEveryInteractiveMachineAndFederatedIdentityAcrossAllRegions?",
          rationale:"This decision affects revocation latency, disaster recovery, auditability, operator workflows, and compatibility with long-lived integrations that cannot rotate instantly.",
          options:[
            {
              id:"centralized",
              label:"CentralizedOpaqueSessionsWithContinuouslyEvaluatedRiskSignalsAndImmediateGlobalRevocation",
              summary:"Keep authoritative session state in a globally replicated control plane and evaluate device, network, and behavior signals on every privileged transition.",
              pros:["ImmediateGlobalRevocationAcrossEveryRegionWithoutWaitingForTokenExpiry","One audit trail for security responders and compliance reviewers"],
              cons:["CriticalPathDependencyOnTheGlobalSessionControlPlaneDuringRegionalNetworkPartitions","Higher coordination cost and more complex multi-region failover"],
              recommended:true,
              affects:["audit-and-compliance-event-processing-pipeline"]
            },
            {
              id:"signed",
              label:"SelfContainedSignedTokensWithShortExpiryAndRegionalKeyDistribution",
              summary:"Issue short-lived self-contained credentials and rely on bounded expiry plus regional key rotation.",
              pros:["Regional autonomy during control-plane partitions"],
              cons:["RevocationIsNeverTrulyImmediateForAlreadyIssuedCredentialsUntilTheirExpirationBoundary"],
              affects:["audit-and-compliance-event-processing-pipeline"]
            }
          ]
        }]
      },
      {
        id:"audit-and-compliance-event-processing-pipeline",
        label:"AuditComplianceEvidenceCollectionNormalizationEnrichmentRetentionAndRegulatorExportProcessingPipeline",
        kind:"queue", status:"affected",
        group:"PlatformSecurityIdentityAndComplianceFoundationsSharedAcrossEveryBusinessUnit",
        description:"ConsumesEveryAuthenticationAuthorizationAdministrationAndBreakGlassEventThenNormalizesEnrichesRetainsAndExportsEvidenceWithoutDroppingOrReorderingRecords"
      },
      {
        id:"downstream", label:"DownstreamConsumer", kind:"external",
        status:"existing",
        description:"Compact control node used to verify complete long edge-label discovery."
      }
    ],
    edges:[
      {
        from:"identity-orchestration",
        to:"audit-and-compliance-event-processing-pipeline",
        kind:"data_flow", status:"proposed",
        label:"SecurityDecisionAuditEvidenceWithCorrelationIdentifiersTenantContextPolicyVersionsAndCompleteRiskSignalProvenance"
      },
      {
        from:"audit-and-compliance-event-processing-pipeline",
        to:"downstream", kind:"data_flow", status:"existing",
        label:"ComplianceEvidenceExports"
      }
    ]
  }}
}' | curl -sS -X POST http://127.0.0.1:4848/mcp \
  -H 'Accept: application/json, text/event-stream' \
  -H 'Content-Type: application/json' \
  -H "Mcp-Session-Id: $UX_MCP_SESSION_ID" --data-binary @-

jq -nc '{
  jsonrpc:"2.0", id:3, method:"tools/call",
  params:{name:"update_graph",arguments:{ops:[
    {op:"announce",message:"SecondLongActivityReceiptWithoutAnyConvenientBreakpointsForTheExpandedScrollableFeed"},
    {op:"announce",message:"Third activity receipt with ordinary prose to verify readable wrapping and timestamps."},
    {op:"announce",message:"FourthLongActivityReceiptWithoutAnyConvenientBreakpointsForTheExpandedScrollableFeed"}
  ]}}
}' | curl -sS -X POST http://127.0.0.1:4848/mcp \
  -H 'Accept: application/json, text/event-stream' \
  -H 'Content-Type: application/json' \
  -H "Mcp-Session-Id: $UX_MCP_SESSION_ID" --data-binary @-

jq -nc '{
  jsonrpc:"2.0", id:4, method:"tools/call",
  params:{name:"propose_graph",arguments:{
    session:"securityarchitecturemodernizationprogramforeveryregionbusinessunitandregulatoryjurisdictionwithextendedretention",
    title:"Long named session", nodes:[{id:"one",label:"One"}], edges:[]
  }}
}' | curl -sS -X POST http://127.0.0.1:4848/mcp \
  -H 'Accept: application/json, text/event-stream' \
  -H 'Content-Type: application/json' \
  -H "Mcp-Session-Id: $UX_MCP_SESSION_ID" --data-binary @-

jq -nc '{
  jsonrpc:"2.0", id:5, method:"tools/call",
  params:{name:"list_sessions",arguments:{}}
}' | curl -sS -X POST http://127.0.0.1:4848/mcp \
  -H 'Accept: application/json, text/event-stream' \
  -H 'Content-Type: application/json' \
  -H "Mcp-Session-Id: $UX_MCP_SESSION_ID" --data-binary @-
```

Expected tool response: `isError: false`, three or more nodes, one or more open
choices, and the long named session appears in `list_sessions`.

- [ ] **Step 4: Capture and inspect the wide canvas and detail panel**

Capture the canvas, select the hostile node, and capture the panel.

Expected:

- card labels stay inside cards and descriptions remain four-line previews;
- SVG edge previews are at most 32 characters;
- the panel close control stays visible;
- ID, kind, status, and group are visible;
- complete description, choice, option, pro/con, affected IDs, connection
  status, and complete edge label wrap without horizontal scrolling;
- every edit, choice, delete-edge, and note control remains reachable by
  vertical scrolling.

- [ ] **Step 5: Inspect global discovery surfaces**

Open the session menu and Activity feed, then capture both.

Expected:

- the complete document title appears at the top of the session menu;
- the long session name stays inside the menu and Compare remains visible;
- menus scroll within the viewport;
- expanded Activity shows every retained entry, wraps long messages, and keeps
  timestamps visible.

- [ ] **Step 6: Verify narrow and light variants**

Stop the wide app, keep Xvfb running, and launch a fresh narrow instance for
reliable WebView sizing:

```bash
DISPLAY=:98 WEBKIT_DISABLE_DMABUF_RENDERER=1 dbus-run-session -- \
  cargo run -- --demo --window-size 520x840 \
  --sessions-dir /tmp/nodestorm-ux-narrow-sessions \
  --prefs /tmp/nodestorm-ux-narrow-prefs.json --port 4849
```

Inject the same payload with the URL port changed from 4848 to 4849, then
capture the selected-node panel and session menu. Stop the narrow app. Create
`/tmp/nodestorm-ux-light-prefs.json` with this exact content using
`apply_patch`:

```json
{"theme":"nodestorm","mode":"light"}
```

Launch the light instance:

```bash
DISPLAY=:98 WEBKIT_DISABLE_DMABUF_RENDERER=1 dbus-run-session -- \
  cargo run -- --demo --window-size 1280x840 \
  --sessions-dir /tmp/nodestorm-ux-light-sessions \
  --prefs /tmp/nodestorm-ux-light-prefs.json --port 4850
```

Inject the same payload with the URL port changed to 4850 and capture the wide
canvas and selected-node panel.

Expected:

- the panel never exceeds the viewport;
- pros and cons stack;
- topbar Send, compose, and More controls retain their current responsive
  behavior;
- all complete values remain discoverable through the panel/session surfaces;
- text remains readable in both dark and light modes.

- [ ] **Step 7: Stop all verification processes**

Send `Ctrl+C` to the active app and Xvfb terminal sessions. Confirm ports 4848,
4849, and 4850 no longer accept connections.

- [ ] **Step 8: Perform the completion audit**

Compare rendered evidence and automated output against every numbered section
of `docs/superpowers/specs/2026-07-18-long-content-discoverability-design.md`.
Record any mismatch as incomplete and return to the responsible task; do not
declare completion from green unit tests alone.
