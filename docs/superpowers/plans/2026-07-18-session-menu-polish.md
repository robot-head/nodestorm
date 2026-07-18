# Session Menu Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the session dropdown quick to scan, hide inactive name fields behind a management disclosure, and protect deletion with an explicit confirmation.

**Architecture:** `TopBar` retains transient open, manage, delete-confirmation, and name-draft state. Semantic menu sections drive the visual hierarchy in CSS; existing `theme.rs` stylesheet and source contracts protect the behavior without a UI-test dependency.

**Tech Stack:** Rust 2024, Dioxus, CSS, Rust unit tests.

## Global Constraints

- Preserve session persistence, MCP behavior, theme tokens, viewport bounds, scrolling, status badges, Compare, archive, delete, and restore behavior.
- Do not show name-entry fields before `Manage session` is opened.
- Confirm delete with the active session name before calling `Sessions::delete`.
- Add no dependency.

---

### Task 1: Disclose management and confirm deletion

**Files:**

- Modify: `src/ui/topbar.rs:17-221`
- Test: `src/theme.rs:328-345`

**Interfaces:**

- Consumes: `Sessions::{list, active_name, create, rename, archive, delete, list_archived, unarchive}`.
- Produces: local `manage_open: Signal<bool>` and `delete_pending: Signal<bool>` state reset whenever the menu closes.

- [ ] **Step 1: Write the failing source-contract test**

Add beside the existing session-row test in `src/theme.rs`:

```rust
#[test]
fn session_menu_discloses_management_and_confirms_delete() {
    for markup in [
        "let mut manage_open = use_signal(|| false);",
        "let mut delete_pending = use_signal(|| false);",
        "if !info.active {",
        "Manage session",
        "Rename current session",
        "Create new session",
        "Confirm delete",
        "Cancel",
    ] {
        assert!(TOPBAR_SOURCE.contains(markup), "missing `{markup}`");
    }
}
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test theme::tests::session_menu_discloses_management_and_confirms_delete -- --exact`

Expected: FAIL because management forms are permanent and deletion is one-click.

- [ ] **Step 3: Implement the transient states and semantic menu structure**

Add state under the two existing name drafts:

```rust
let mut manage_open = use_signal(|| false);
let mut delete_pending = use_signal(|| false);
```

Reset both values and both drafts before every existing menu close (the session-pod toggle when closing and `menu-catcher`). Only render saved-session rows inside `if !info.active { ... }` so the title header is the active-session representation.

Replace the two permanent `session-create` blocks and the archive/delete buttons with this layout, moving the existing create/rename/archive/delete handler bodies into the marked buttons:

```rust
button {
    class: "session-manage-toggle",
    onclick: move |_| { delete_pending.set(false); manage_open.toggle(); },
    "Manage session"
}
if manage_open() {
    div { class: "session-manage",
        div { class: "session-form",
            label { r#for: "rename-session", "Rename current session" }
            div { class: "session-form-row",
                input { id: "rename-session", class: "session-name-input", placeholder: "new name…", value: "{rename_draft}", oninput: move |ev| rename_draft.set(ev.value()) }
                button { class: "btn", disabled: rename_draft.read().trim().is_empty(), /* existing rename handler */ "Rename" }
            }
        }
        div { class: "session-form",
            label { r#for: "create-session", "Create new session" }
            div { class: "session-form-row",
                input { id: "create-session", class: "session-name-input", placeholder: "session name…", value: "{new_session_draft}", oninput: move |ev| new_session_draft.set(ev.value()) }
                button { class: "btn", disabled: new_session_draft.read().trim().is_empty(), /* existing create handler */ "Create" }
            }
        }
    }
}
div { class: "session-danger",
    span { class: "session-section-label", "Danger zone" }
    button { class: "session-archive", /* existing archive handler */ "Archive session" }
    if delete_pending() {
        div { class: "delete-confirm",
            span { "Delete {session_name} permanently?" }
            button { class: "session-delete-confirm", /* existing delete handler, then clear delete_pending */ "Confirm delete" }
            button { class: "session-cancel", onclick: move |_| delete_pending.set(false), "Cancel" }
        }
    } else {
        button { class: "session-delete", onclick: move |_| delete_pending.set(true), "Delete session" }
    }
}
```

Keep the archived-session list after `.session-danger`. Existing successful handlers still close the menu; delete also clears `delete_pending`.

- [ ] **Step 4: Verify the source contract passes**

Run: `cargo test theme::tests::session_menu_discloses_management_and_confirms_delete -- --exact`

Expected: PASS.

- [ ] **Step 5: Commit the interaction structure**

```bash
git add src/ui/topbar.rs src/theme.rs
git commit -m "feat(ui): streamline session menu actions"
```

### Task 2: Style management forms and the danger zone

**Files:**

- Modify: `assets/main.css:885-1008`
- Test: `src/theme.rs:198-345`

**Interfaces:**

- Consumes: `.session-manage-toggle`, `.session-manage`, `.session-form`, `.session-form-row`, `.session-danger`, `.session-delete`, `.session-delete-confirm`, and `.session-cancel` from `TopBar`.
- Produces: labeled rectangular inputs and a separated archive/delete footer while retaining existing narrow-viewport menu behavior.

- [ ] **Step 1: Write the failing stylesheet contract test**

Add after the source contract test in `src/theme.rs`:

```rust
#[test]
fn session_management_forms_and_danger_zone_are_distinct() {
    assert_block_contains(".session-manage", "border-top: 1px solid var(--border)");
    assert_block_contains(".session-form", "display: grid");
    assert_block_contains(".session-form-row", "display: flex");
    assert_block_contains(".session-name-input", "border-radius: 7px");
    assert_block_contains(".session-name-input", "width: 100%");
    assert_block_contains(".session-danger", "border-top: 1px solid var(--border)");
    assert_block_contains(".session-delete", "color: var(--status-removed)");
    assert_block_contains(".delete-confirm", "background: var(--accent-soft)");
}
```

- [ ] **Step 2: Verify the test fails**

Run: `cargo test theme::tests::session_management_forms_and_danger_zone_are_distinct -- --exact`

Expected: FAIL because the legacy form uses pill inputs and no semantic sections exist.

- [ ] **Step 3: Replace the legacy session form rules**

Replace `.session-create` through `.session-archive` in `assets/main.css` with:

```css
.session-manage-toggle { font-weight: 500; }
.session-manage { display: grid; gap: 10px; margin-top: 4px; padding: 10px; border-top: 1px solid var(--border); background: color-mix(in srgb, var(--bg-card) 55%, transparent); }
.session-form { display: grid; gap: 5px; }
.session-form label, .session-section-label { color: var(--text-dim); font-size: 10.5px; letter-spacing: 0.06em; text-transform: uppercase; }
.session-form-row { display: flex; gap: 6px; }
.session-name-input { box-sizing: border-box; width: 100%; min-width: 0; background: var(--bg-panel); border: 1px solid var(--border); border-radius: 7px; color: var(--text); font: inherit; font-size: 12.5px; padding: 7px 9px; }
.session-form-row .btn { flex: 0 0 auto; }
.session-danger { display: grid; gap: 3px; margin-top: 4px; padding-top: 6px; border-top: 1px solid var(--border); }
.session-archive { color: var(--text-dim) !important; }
.session-delete, .session-delete-confirm { color: var(--status-removed) !important; }
.delete-confirm { display: grid; grid-template-columns: 1fr auto auto; align-items: center; gap: 4px; margin: 2px 4px; padding: 5px 6px; background: var(--accent-soft); border-radius: 6px; color: var(--text); font-size: 11.5px; }
.delete-confirm button { width: auto; padding: 5px 7px; }
```

Retain `.session-name-input:focus` immediately after this replacement and do not alter the existing mobile dropdown or session-row rules.

- [ ] **Step 4: Verify the focused contracts pass**

Run: `cargo test theme::tests::session_menu_discloses_management_and_confirms_delete theme::tests::session_management_forms_and_danger_zone_are_distinct -- --exact`

Expected: PASS for both tests.

- [ ] **Step 5: Run verification and inspect the rendered menu**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- --demo
```

Expected: checks exit 0. At desktop and 520px widths, verify the initial menu has no empty fields or duplicate active slug; Manage reveals labeled rectangular Rename/Create fields; archive, cancel, confirm-delete, and restore work; and the menu scrolls vertically rather than overflowing.

- [ ] **Step 6: Commit the completed polish**

```bash
git add assets/main.css src/theme.rs
git commit -m "style(ui): polish session management menu"
```
