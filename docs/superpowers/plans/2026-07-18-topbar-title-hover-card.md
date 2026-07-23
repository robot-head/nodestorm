# Topbar Title Hover Card Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reveal every compact topbar title in a themed, wrapping hover card.

**Architecture:** Keep `TopBar` as the value source and use a CSS pseudo-element for the card. The current native title remains a fallback; no state, component, or dependency is needed.

**Tech Stack:** Rust, Dioxus, CSS, Rust unit tests.

## Global Constraints

- Preserve the one-line, ellipsized topbar layout.
- Use only existing semantic CSS tokens and no dependencies.
- Bound and wrap the hover card for long unbroken titles.

---

### Task 1: Render and style the hover card

**Files:**
- Modify: `src/theme.rs`
- Modify: `src/ui/topbar.rs`
- Modify: `assets/main.css`

**Interfaces:**
- Consumes: the local `title: String` value in `TopBar`.
- Produces: a `data-full-title` attribute read by `.topbar-title::after`.

- [ ] **Step 1: Write the failing test**

Add a `src/theme.rs` test that requires `data-full-title` on the topbar title,
the pseudo-element card, a `max-width` bounded by the viewport, wrapping, and
a `:hover::after` visible state.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test theme::tests::topbar_title_has_a_wrapping_hover_card`

Expected: FAIL because the hover-card contract is not present.

- [ ] **Step 3: Write minimal implementation**

Store the complete title in the topbar span's `data-full-title` attribute. Add
a hidden, absolutely positioned `.topbar-title::after` card, reveal it with
`.topbar-title:hover::after`, and use existing background, border, text, and
shadow tokens.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test theme::tests::topbar_title_has_a_wrapping_hover_card && cargo test`

Expected: PASS with no failing Rust tests.
