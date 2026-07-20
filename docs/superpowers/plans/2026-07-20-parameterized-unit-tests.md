# Parameterized Unit Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert only natural repeated Rust unit-test families under `src/` to Yare tables, migrate every assertion in those unit-test modules to `assert2`, and use whole-value equality where the complete value is the contract.

**Architecture:** Keep tests beside their production modules. Yare replaces duplicated input/expected test bodies only; singleton and stateful workflows stay as ordinary `#[test]` or `#[tokio::test]` functions. Use explicit `assert2::assert!` calls so the final source audit is unambiguous.

**Tech Stack:** Rust, Cargo, `yare = "3"`, `assert2 = "0.4"`, Tokio test macros.

**Design:** `docs/superpowers/specs/2026-07-20-parameterized-unit-tests-design.md`

**Working-tree constraint:** Preserve the existing mutation-test edits in `.cargo/mutants.toml`, `src/layout.rs`, and `src/store.rs`. Do not discard or overwrite them. Because this migration overlaps two already-dirty files, use verified checkpoints without committing unless the user explicitly requests a commit.

## Task 1: Add dependencies and prove Yare syntax on a natural table

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/ui/topbar.rs`

- [ ] Add `assert2 = "0.4"` and `yare = "3"` under `[dev-dependencies]`, preserving the existing ordering convention.

- [ ] Run `cargo check --tests` once to resolve and lock both crates.

- [ ] In `src/ui/topbar.rs`, import `yare::parameterized` in the test module and convert `send_labels_are_receipt_driven` into one natural multi-case table. Keep its existing receipt inputs and expected labels; give each case a scenario name.

```rust
#[parameterized(
    idle = { SendStatus::Idle, "Send" },
    sending = { SendStatus::Sending, "Sending..." },
    sent = { SendStatus::Sent, "Sent" },
    reconnecting = { SendStatus::Reconnecting, "Reconnecting..." },
    failed = { SendStatus::Failed, "Failed - Retry" },
)]
fn send_labels_are_receipt_driven(status: SendStatus, expected: &str) {
    assert2::assert!(send_label(status) == expected);
}
```

Adapt the values to the actual `SendStatus` constructors in the file; do not change production types to fit the example.

- [ ] Convert every remaining assertion in `src/ui/topbar.rs` to explicit `assert2::assert!`, preserving custom messages and fail-fast behavior.

- [ ] Run `cargo test ui::topbar::tests` and fix macro imports, case argument ownership, and generated test names before continuing.

## Task 2: Migrate small core modules

**Files:**
- Modify: `src/agent_launcher.rs`
- Modify: `src/cli.rs`
- Modify: `src/demo.rs`
- Modify: `src/diff.rs`
- Modify: `src/icon.rs`
- Modify: `src/persist.rs`
- Modify: `src/prefs.rs`

- [ ] For each file, identify tests with the same setup/action/assertion body and multiple inputs or expected outputs. Parameterize only those natural families. Likely candidates to verify include CLI window-size parsing, SSH/path quoting cases, icon inside/boundary/outside points, export-path suffix mapping, and preference mode serialization.

- [ ] Keep unique filesystem workflows, subprocess workflows, and state transitions as ordinary tests. Do not introduce booleans, scenario enums, or setup closures merely to merge them.

- [ ] Replace `assert_eq!`, `assert_ne!`, and built-in `assert!` inside each test module—including its test-only helpers and closures—with `assert2::assert!`.

- [ ] Where all meaningful fields are already asserted, build the expected value and compare it once. Retain property assertions when a test deliberately ignores incidental fields.

- [ ] Run the affected suites:

```bash
cargo test agent_launcher::tests
cargo test cli::tests
cargo test demo::tests
cargo test diff::tests
cargo test icon::tests
cargo test persist::tests
cargo test prefs::tests
```

## Task 3: Migrate export, model, and theme suites

**Files:**
- Modify: `src/export.rs`
- Modify: `src/model.rs`
- Modify: `src/theme.rs`

- [ ] Consolidate natural rendering/escaping/name-mapping tables in `src/export.rs`. Keep large golden-record scenarios separate when their setup or expected document is unique.

- [ ] Consolidate natural wire-format, defaulting, and validation input/expected families in `src/model.rs`. Do not merge distinct merge/state-transition contracts into a branching mega-test.

- [ ] Consolidate natural theme lookup, mode serialization, palette-anchor, and selector cases in `src/theme.rs`. Preserve CSS contract failure messages.

- [ ] Convert every assertion in all three test modules to `assert2::assert!`. Prefer equality for complete serialized values and complete model values that already implement semantically appropriate `PartialEq`.

- [ ] Run:

```bash
cargo test export::tests
cargo test model::tests
cargo test theme::tests
```

## Task 4: Migrate layout without losing mutation coverage

**Files:**
- Modify: `src/layout.rs`

- [ ] Review the current dirty diff first with `git diff -- src/layout.rs`; record which tests/assertions were added to kill mutations and preserve every covered boundary.

- [ ] Convert genuine numeric input/expected families such as glyph-width classes, wrapping inputs, rectangle geometry cases, and lane hit/miss coordinates to Yare tables when they share one body.

- [ ] Keep graph construction and multi-step routing/layout scenarios separate when their fixtures or invariants differ. Do not combine mutation-killing boundary tests if doing so obscures the exact boundary.

- [ ] Migrate every assertion in the module and its helpers to `assert2::assert!`. Use whole `Rect`, point, bounds, band, and collection equality where the test already specifies the complete value.

- [ ] Compare `git diff -- src/layout.rs` with the pre-migration diff and confirm no mutation-test scenario or assertion disappeared.

- [ ] Run `cargo test layout::tests`.

## Task 5: Migrate session, store, and server suites

**Files:**
- Modify: `src/sessions.rs`
- Modify: `src/store.rs`
- Modify: `src/server/mod.rs`
- Modify: `src/server/tools.rs`

- [ ] In `src/store.rs`, review the dirty diff first and preserve all current mutation-killing cases. Natural candidates include the existing `slugify_cases` family and repeated pure input/expected mappings; leave receipt delivery, queue replay, undo/redo, and agent reconnect workflows as ordinary async or stateful tests unless two or more truly share one body.

- [ ] In `src/sessions.rs`, parameterize only repeated pure naming/path/summary cases. Keep filesystem lifecycle, notification, reconnect, and autosave workflows separate.

- [ ] In the server modules, parameterize repeated result/default mappings only. Keep router, connection-lifecycle, shutdown, and guard-drop workflows separate.

- [ ] If a genuine async family exists, use Yare in the documented order and preserve all Tokio options:

```rust
#[parameterized(first = { /* args */ }, second = { /* args */ })]
#[test_macro(tokio::test)]
async fn shared_async_contract(/* parameters */) {
    // existing shared body
}
```

Do not create an async table solely to demonstrate `test_macro`.

- [ ] Convert every test-module assertion and helper assertion to explicit `assert2::assert!`. Preserve custom error text and intermediate-state checks.

- [ ] Compare `git diff -- src/store.rs` with the pre-migration diff and confirm every mutation-killing scenario remains.

- [ ] Run:

```bash
cargo test sessions::tests
cargo test store::tests
cargo test server::tests
cargo test server::tools::tests
```

## Task 6: Migrate remaining UI suites

**Files:**
- Modify: `src/ui/activity.rs`
- Modify: `src/ui/agent_launcher.rs`
- Modify: `src/ui/canvas.rs`
- Modify: `src/ui/choice_panel.rs`
- Modify: `src/ui/edge_layer.rs`
- Modify: `src/ui/minimap.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/more_menu.rs`
- Modify: `src/ui/node_card.rs`
- Modify: `src/ui/queued_changes.rs`
- Modify: `src/ui/theme_menu.rs`

- [ ] Parameterize natural mapping and threshold families: edge-kind labels/classes, node glyph/label/status mappings, zoom tiers, theme-to-Tao mappings, group visibility, and equivalent repeated input/expected helpers.

- [ ] Keep rendering workflows, launch state machines, persistence workflows, and graph virtualization scenarios separate when their fixtures or assertions differ.

- [ ] Convert every assertion in these test modules and test-only helpers to explicit `assert2::assert!`. Use complete expected structs/collections where equality represents the whole contract.

- [ ] Run each affected module test filter, then run the aggregate UI filter:

```bash
cargo test ui::
```

## Task 7: Audit scope and verify the complete migration

**Files:**
- Review: `Cargo.toml`
- Review: `Cargo.lock`
- Review: every `#[cfg(test)] mod tests` under `src/`
- Review: `.cargo/mutants.toml`

- [ ] List all source test modules and confirm the inventory still covers the same 27 files:

```bash
rg -l '^\s*mod tests\s*\{' src --glob '*.rs' | sort
```

- [ ] Review every `#[parameterized]` body. Confirm it has at least two natural cases, descriptive case names, one shared body without scenario branching, and no lost assertions or custom messages.

- [ ] Review every remaining ordinary test. Confirm it is intentionally singleton, stateful, or fixture-specific rather than a missed natural table.

- [ ] Audit for built-in assertion macros. This expression deliberately does not reject namespaced `assert2::assert!`; inspect any matches and confirm they are production assertions outside the covered unit-test modules:

```bash
rg -n '(^|[^:[:alnum:]_])(assert|assert_eq|assert_ne|debug_assert)!' \
  src --glob '*.rs'
```

- [ ] Search for repeated adjacent field assertions and manually verify that each retained group is partial/invariant-based, not a complete value that should use one equality assertion:

```bash
rg -n 'assert2::assert!' src --glob '*.rs'
```

- [ ] Confirm integration tests, examples, JavaScript tests, production assertions, and debug assertions outside `src` unit-test modules were not migrated.

- [ ] Run formatting and lint verification:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

- [ ] Run the complete test suite:

```bash
cargo test --all-targets
```

- [ ] Inspect the final diff and confirm `.cargo/mutants.toml`, `src/layout.rs`, and `src/store.rs` still contain the pre-existing mutation work in addition to the requested test migration:

```bash
git diff --check
git diff --stat
git diff -- .cargo/mutants.toml src/layout.rs src/store.rs
```

- [ ] Report the number of Yare families/cases, the number of intentionally ordinary tests, and the exact verification commands and results. Do not claim completion unless every command above passes.
