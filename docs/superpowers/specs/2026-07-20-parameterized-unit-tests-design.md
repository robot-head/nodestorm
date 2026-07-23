# Parameterized Unit Tests Design

## Goal

Convert natural Rust unit-test tables under `src/` to `yare` parameterized
tests, use `assert2` for every unit-test assertion, and prefer equality of
complete expected values over repeated field assertions when the complete
value is the test contract.

## Scope

The migration covers every `#[cfg(test)]` unit-test module under `src/`. The
current inventory is 363 test functions across 27 modules. Rust integration
tests under `tests/`, JavaScript tests, examples, production assertions, and
debug assertions are outside this migration because they are not Rust unit
tests embedded in the crate sources.

Existing uncommitted mutation-test changes in `.cargo/mutants.toml`,
`src/layout.rs`, and `src/store.rs` must be preserved. Test migration edits may
overlap the two Rust files, but must retain their current assertions and
coverage semantics.

## Dependencies

Add these development-only dependencies:

```toml
assert2 = "0.4"
yare = "3"
```

Each test module with a natural parameterized family imports
`yare::parameterized`. Assertions use the explicit `assert2::assert!` path,
avoiding ambiguity with Rust's built-in `assert!` macro and making
repository-wide auditing mechanical.

## Parameterization Rules

Tests that exercise the same behavior with different inputs and expected
outputs are combined into one multi-case table. Case names describe the
scenario and remain individually selectable by the Rust test harness. Values
passed to cases are ordinary Rust expressions, including constructors and
small setup blocks where needed.

Tests with unique setup, assertions, or stateful workflows remain ordinary
`#[test]` or `#[tokio::test]` functions. A test is not wrapped in a singleton
parameter table merely for consistency. Tests are combined only when they can
share one readable body without mode flags, scenario enums, large conditional
branches, or opaque setup closures.

Natural async test families use Yare's documented custom test macro, in the
required order:

```rust
#[parameterized(
    immediate = { 0 },
    delayed = { 10 },
)]
#[test_macro(tokio::test)]
async fn async_workflow(delay_ms: u64) {
    // shared async test body
}
```

Existing `tokio::test` options such as `start_paused = true` are preserved
inside `#[test_macro(tokio::test(...))]`.

## Assertion Rules

Every assertion inside the covered unit tests uses `assert2::assert!`:

- `assert_eq!(actual, expected)` becomes
  `assert2::assert!(actual == expected)`.
- `assert_ne!(actual, expected)` becomes
  `assert2::assert!(actual != expected)`.
- Boolean assertions remain boolean expressions in `assert2::assert!`.
- Pattern assertions prefer `assert2::assert!(let Pattern = expression)` so
  the matched values remain available when later checks need them.
- Existing custom failure messages are retained as macro arguments.
- Assertions inside test-only helper functions and closures are migrated too.

The migration preserves fail-fast behavior by using `assert2::assert!`, not
`assert2::check!`. It does not change production assertions outside test
modules.

## Whole-Value Equality

When a test enumerates all meaningful fields of a value, construct the
expected value and compare once:

```rust
assert2::assert!(actual == Rect {
    x: 10.0,
    y: 20.0,
    w: 30.0,
    h: 40.0,
});
```

This applies especially to geometry values, summaries, paths, configuration
objects, serialized model values, and collections of those values. Existing
`PartialEq` implementations are reused.

Field or property assertions remain appropriate when:

- the test intentionally specifies only part of a larger value;
- nondeterministic or incidental fields are deliberately ignored;
- different intermediate states are asserted between mutations;
- the type does not have semantically valid whole-value equality; or
- a property assertion communicates an invariant better than an exact value.

Production types will not gain `PartialEq` solely to make a test rewrite
possible unless whole-value equality is already semantically correct for that
type.

## Migration Strategy

Work in independently verifiable batches:

1. Add dependencies and prove a synchronous multi-case syntax spike using a
   natural existing family. If the audit identifies a natural async family,
   prove its custom test-macro syntax in that batch rather than manufacturing
   an async table solely as a framework demonstration.
2. Migrate smaller pure modules and consolidate natural tables.
3. Migrate layout, model, theme, export, and other larger synchronous suites.
4. Migrate sessions, store, server, and async/stateful suites.
5. Migrate UI unit-test modules.
6. Audit the complete `src/` tree and run full verification.

After each batch, run the affected module's tests. Compile errors from macro
syntax, ownership, or case argument types are fixed within that batch before
moving on. No production behavior changes are part of the migration.

## Completion Evidence

The migration is complete only when all of these checks pass:

1. `Cargo.toml` and `Cargo.lock` contain `yare` and `assert2` as resolved
   development dependencies.
2. A file-by-file review identifies every natural repeated input/expected
   family and confirms it uses `#[parameterized(...)]`; singleton and stateful
   tests remain ordinary tests.
3. A carefully scoped source audit finds no built-in `assert!`, `assert_eq!`,
   `assert_ne!`, or `debug_assert!` invocations inside those modules.
4. Every parameterized async family uses `#[test_macro(...)]` and preserves
   its prior Tokio test options.
5. A review of repeated field assertions confirms that complete values are
   compared when all fields are part of the expected contract.
6. `cargo fmt --all -- --check` passes.
7. `cargo clippy --all-targets -- -D warnings` passes.
8. `cargo test --all-targets` passes.

## Non-Goals

- Do not alter application behavior or public APIs.
- Do not introduce a project-specific test macro around `yare` or `assert2`.
- Do not create singleton parameter tables or force unlike scenarios into one
  test body.
- Do not parameterize integration tests, JavaScript tests, or examples.
- Do not weaken, delete, or combine tests when doing so loses a scenario,
  assertion, failure message, async option, or mutation-killing distinction.
