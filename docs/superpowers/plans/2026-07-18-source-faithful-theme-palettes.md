# Source-faithful Theme Palettes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace approximate palette-role assignments with canonical colors while preserving the twelve selectable theme families and light/dark behavior.

**Architecture:** Keep the existing CSS-only theming architecture. Strengthen the colocated Rust stylesheet test with canonical source anchors, then update only the `@supports` family blocks in `assets/main.css` so all UI roles draw from their selected palette's established colors.

**Tech Stack:** Rust tests, CSS custom properties, `light-dark()`.

## Global Constraints

- Keep all twelve family ids, display names, picker order, persisted preferences, and `Mode` unchanged.
- Do not add dependencies or change application/UI logic.
- Preserve all 20 required custom properties in every family block.
- Use the source pair named in the approved design: Solarized Light/Dark, Gruvbox Light/Dark, Catppuccin Latte/Mocha, Nord Snow Storm/Polar Night, Tokyo Night Day/Night, One Light/One Dark, GitHub Primer Light/Dark, Everforest Light/Medium, and Rosé Pine Dawn/Main.
- Retain the established Alucard light companion for Dracula and the established Monokai light companion; only use colors native to those selected palettes.

---

### Task 1: Pin canonical palette anchors in the stylesheet test

**Files:**
- Modify: `src/theme.rs:121-215`
- Test: `src/theme.rs:tests`

**Interfaces:**
- Consumes: `block_for(selector) -> &'static str` and `FAMILIES`.
- Produces: `canonical_palette_anchors_remain_stable()` test.

- [ ] **Step 1: Add the failing palette-anchor test**

Add this test after `every_family_has_a_block_defining_all_tokens`:

```rust
    #[test]
    fn canonical_palette_anchors_remain_stable() {
        let anchors = [
            ("nodestorm", "--accent: light-dark(#3d6fe0, #6c9ef8);"),
            ("solarized", "--bg: light-dark(#fdf6e3, #002b36);"),
            ("gruvbox", "--bg: light-dark(#fbf1c7, #282828);"),
            ("catppuccin", "--bg: light-dark(#eff1f5, #1e1e2e);"),
            ("nord", "--bg: light-dark(#eceff4, #2e3440);"),
            ("dracula", "--accent: light-dark(#644ac9, #bd93f9);"),
            ("tokyo-night", "--bg: light-dark(#e1e2e7, #1a1b26);"),
            ("one", "--bg: light-dark(#fafafa, #282c34);"),
            ("github", "--bg: light-dark(#f6f8fa, #0d1117);"),
            ("everforest", "--bg: light-dark(#fdf6e3, #2d353b);"),
            ("rose-pine", "--bg: light-dark(#faf4ed, #191724);"),
            ("monokai", "--bg: light-dark(#fafaf4, #272822);"),
        ];

        for (id, declaration) in anchors {
            assert!(
                block_for(&format!("[data-theme=\"{id}\"]")).contains(declaration),
                "family {id} lost its canonical palette anchor"
            );
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test canonical_palette_anchors_remain_stable`

Expected: FAIL because several families, including Solarized, Gruvbox, Nord,
Catppuccin, Everforest, and Rosé Pine, currently use approximate background
values.

- [ ] **Step 3: Leave the test in place for the CSS change**

Do not change production Rust. The test is the regression boundary for the exact family/variant selection in Task 2.

### Task 2: Replace approximate CSS role colors with canonical palette values

**Files:**
- Modify: `assets/main.css:1419-1694`
- Test: `src/theme.rs:canonical_palette_anchors_remain_stable`

**Interfaces:**
- Consumes: the 20 names in `REQUIRED_TOKENS` and the approved source-pair matrix.
- Produces: source-faithful `light-dark(light, dark)` declarations for all twelve `[data-theme]` blocks.

- [ ] **Step 1: Update the structural background ladder first**

For each external family, set `--bg`, `--bg-panel`, `--bg-card`, `--bg-card-hover`, and `--border` from its canonical background/surface ladder. Do not introduce a hex color absent from that family. In particular, make these audited anchors exact:

```css
[data-theme="solarized"] { --bg: light-dark(#fdf6e3, #002b36); }
[data-theme="gruvbox"] { --bg: light-dark(#fbf1c7, #282828); }
[data-theme="catppuccin"] { --bg: light-dark(#eff1f5, #1e1e2e); }
[data-theme="nord"] { --bg: light-dark(#eceff4, #2e3440); }
[data-theme="everforest"] { --bg: light-dark(#fdf6e3, #2d353b); }
[data-theme="rose-pine"] { --bg: light-dark(#faf4ed, #191724); }
```

Retain `light-dark()` for every structural token, even if the source palette uses the same surface for two roles.

- [ ] **Step 2: Map semantic roles from the same palette**

For every family block, set `--text`/`--text-dim` from its foreground and comment/subtle colors; set `--accent` and `--status-proposed` to its canonical blue/accent; map modified/open to yellow/orange, affected to purple/magenta, removed to red, and decided to green. Use palette foregrounds for `--on-accent` and `--on-badge`, then use the palette's darkest/brightest surface for `--shadow` and `--dot-grid`.

Keep the comments beside the Dracula and Monokai selectors, but replace "derived" with the actual companion variant name and source. Do not alter the Nodestorm block except for a comment that marks it as the product palette.

- [ ] **Step 3: Run focused checks**

Run: `cargo fmt --check && cargo test theme::tests`

Expected: PASS, including the new anchor test and the existing 20-token and `light-dark()` integrity checks.

- [ ] **Step 4: Run the complete suite**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`

Expected: PASS with no warning output.

- [ ] **Step 5: Inspect the diff and commit**

Run: `git diff --check && git diff -- src/theme.rs assets/main.css`

Expected: only the palette anchor test and family palette declarations change.

Commit:

```bash
git add src/theme.rs assets/main.css
git commit -m "fix(themes): use canonical palette colors"
```
