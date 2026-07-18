# Title-bar Control Depth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every Nodestorm title-bar control a squarer, subtly raised Soft Deck treatment without changing its behavior or responsive layout.

**Architecture:** Keep the change CSS-only and scope all new rules below `.topbar` so buttons outside the title bar retain their existing appearance. A single grouped selector gives buttons, text fields, and the status chip identical geometry and depth; small interaction overrides make actionable controls lift on hover and settle while pressed.

**Tech Stack:** CSS custom properties and selectors in `assets/main.css`; Node.js static assertion; Cargo formatting and tests.

## Global Constraints

- Modify only `assets/main.css`; do not change Rust markup, dimensions, labels, or responsive rules.
- Cover the session selector, search field, status chip, action pods, More control, and message field.
- Use 10px corners, a subtle top highlight, a 2px lower edge, and a soft drop shadow.
- Preserve semantic accent/status colors, keyboard focus visibility, disabled behavior, and narrow-title-bar overflow behavior.

---

### Task 1: Style the title-bar control surface

**Files:**
- Modify: `assets/main.css:205-238` (insert the scoped title-bar overrides after the base `.btn` block)
- Test: static assertion run from the repository root; existing Rust checks

**Interfaces:**
- Consumes: Existing `.topbar`, `.btn`, `.search-box`, `.send-comment`, `.status-chip`, `--border`, `--shadow`, and `--accent` selectors/tokens.
- Produces: A shared Soft Deck visual treatment for all title-bar controls without new markup, tokens, dependencies, or runtime interfaces.

- [ ] **Step 1: Write the failing static CSS assertion**

Run this command before editing. It must fail because the scoped title-bar selectors do not yet exist:

```bash
node --input-type=module -e '
import { readFileSync } from "node:fs";
const css = readFileSync("assets/main.css", "utf8");
for (const selector of [".topbar .btn,", ".topbar .send-comment,", ".topbar .search-box,", ".topbar .status-chip {"]) {
  if (!css.includes(selector)) throw new Error(`Missing ${selector}`);
}
'
```

Expected: the process exits nonzero with `Error: Missing .topbar .btn,`.

- [ ] **Step 2: Add the minimal scoped Soft Deck rules**

Insert this block immediately after the closing brace of the base `.btn` declaration and before `.btn:hover:not(:disabled)`. Do not alter the existing base `.btn`, input, status-segment, or responsive declarations.

```css
.topbar .btn,
.topbar .send-comment,
.topbar .search-box,
.topbar .status-chip {
  border-radius: 10px;
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, white 65%, transparent),
    0 2px 0 color-mix(in srgb, var(--border) 72%, transparent),
    0 5px 10px color-mix(in srgb, var(--shadow) 20%, transparent);
}

.topbar .btn,
.topbar .search-box,
.topbar .send-comment {
  transition: border-color 120ms ease, background 120ms ease, box-shadow 120ms ease,
    transform 120ms ease;
}

.topbar .btn:hover:not(:disabled),
.topbar .search-box:hover,
.topbar .send-comment:hover {
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, white 72%, transparent),
    0 3px 0 color-mix(in srgb, var(--border) 72%, transparent),
    0 7px 12px color-mix(in srgb, var(--shadow) 24%, transparent);
}

.topbar .btn:active:not(:disabled),
.topbar .search-box:active,
.topbar .send-comment:active {
  transform: translateY(2px);
  box-shadow:
    inset 0 1px 2px color-mix(in srgb, var(--shadow) 20%, transparent),
    0 1px 0 color-mix(in srgb, var(--border) 72%, transparent),
    0 2px 5px color-mix(in srgb, var(--shadow) 14%, transparent);
}

.topbar .btn:focus-visible,
.topbar .search-box:focus-visible,
.topbar .send-comment:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
```

- [ ] **Step 3: Re-run the static assertion**

Run the Step 1 command unchanged.

Expected: process exits `0` with no output.

- [ ] **Step 4: Verify repository checks and the responsive surface**

Run:

```bash
cargo fmt --check
cargo test
git diff --check
```

Expected: all three commands exit `0`. Then launch the existing desktop app and inspect the top bar at widths above 1080px, around 780px, and below 560px. Confirm the session selector, search field, visible status chip, undo/redo, Timeline, node, More, message field, and Send control share the 10px raised treatment; focus remains visible; pressed buttons settle; and the existing narrow-width hiding/collapsing rules still work.

- [ ] **Step 5: Commit the implementation**

```bash
git add assets/main.css
git commit -m "style: add titlebar control depth"
```

Expected: Git creates one commit containing only the CSS visual treatment.
