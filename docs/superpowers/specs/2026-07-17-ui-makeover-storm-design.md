# UI makeover — "Storm" design (v0.8)

**Date:** 2026-07-17
**Status:** approved (direction and sections approved in-session; this file is the
written record)

## Problem

1. **Overflow:** the top bar is a single non-wrapping flex row with ~14 items.
   Only the two text inputs shrink; buttons never collapse or wrap, so on
   narrow windows the right end of the row — including **Send to agent** —
   slides off the window.
2. **Blandness:** near-uniform 12.5px type, flat cards, identical 6px radii,
   no depth or brand personality. Reads as a default dark dashboard.

## Direction

**Storm** (chosen over "Precision instrument" and "Cockpit" mockups): an
electric identity derived from each theme's own accent color. Distinctive but
professional; all 12 palette families × light/dark keep working with zero
per-theme edits.

## Constraints (all hard)

- Works across all 24 theme/mode combinations; new colors must derive from
  existing semantic tokens (`--accent`, `--bg`, …) via `color-mix`, never
  hardcoded per theme. The `[data-theme]` palette blocks in `main.css` are
  **not** edited (theme.rs registry tests keep them in lock-step).
- The Windows E2E script (`scripts/verify-windows.ps1`) finds controls by UIA
  accessible name. Every control whose visible text changes gets an
  `aria-label` pinning today's exact accessible name (see table below). The
  script is updated only where controls *move* (Export/Theme into the ⋯ menu).
- Node-card geometry is mirrored in `layout::estimate_height`; any card
  padding/rail change updates those constants and their tests in the same
  commit.
- Desktop app must work offline: the display font is embedded, not fetched.
- Exported records (Markdown/Mermaid) keep their fixed colors — no changes to
  `export.rs`.

## 1. Design language

- **New derived tokens** in `:root` (computed, theme-agnostic):
  - `--glow: color-mix(in srgb, var(--accent) 55%, transparent)` — selection
    rings and focus glows.
  - `--accent-soft: color-mix(in srgb, var(--accent) 12%, transparent)` —
    tinted fills (hovers, active-menu rows, status-chip segments).
- **Shape language:** interactive controls ("pods") use pill radius (999px);
  cards/panels/dropdowns use 12px; the choice-option rows 10px.
- **Type:** Space Grotesk variable woff2 (SIL OFL), one `@font-face` embedded
  as a base64 data URI in `main.css` (~80KB). Used for: topbar wordmark, node
  card titles, panel/empty-state headings, cluster cards. Body text keeps the
  current Inter/Segoe stack. License file added under `assets/fonts/`.
- **Depth, with restraint:** glow (`box-shadow` from `--glow`) appears only
  on: selected cards, ripple highlights, the agent-waiting pulse, Send hover,
  and focused inputs. Everything else stays flat.
- **Motion:** existing 100–160ms transitions kept; hover-raise on cards via
  `transform: translateY(-1px)` (no layout impact).

## 2. Top bar

New left→right order:

```
⚡wordmark · [session ▾] · [search] · title… · <spacer> ·
[status chip] · [↶][↷] [timeline] [+ node] [⋯] · [message input] [Send ⚡]
```

- **Fused status chip:** the three pills merge into one segmented pill —
  `● waiting · 2 open · 3 queued`. Segments with zero count vanish (as the
  pills do today). Waiting segment keeps the pulse animation.
- **⋯ menu (visible label ⋯, aria-label "More"):** permanently holds the
  **Export ▾** and **Theme ▾** entries as an accordion: clicking the
  "Export ▾" row expands the existing export rows inline beneath it;
  "Theme ▾" likewise expands the mode row + family list. The two existing
  dropdown bodies are reused unchanged as the expanded content. Rationale:
  occasional-use controls; deterministic location for E2E at every window
  width.
- **Buttons become pods:** Undo/Redo are icon-only pods at all widths;
  Timeline and + node are icon+label pods.
- **Send:** accent-filled pod, bolt icon, visible label "Send". The inline
  message input sits beside it exactly as today when there is room.
- **Responsive behavior — CSS container queries** on the header
  (`container-type: inline-size`); no JS/Rust width tracking:
  - ≤1080px: Timeline and + node drop their labels (icon pods + tooltips).
  - ≤900px: the document title hides (it ellipsizes from 1080→900).
  - ≤780px: search collapses to an icon pod that expands on focus
    (`:focus-within` width transition, pure CSS); the inline message input
    hides; a compose pod (✎, aria-label "Message to agent") appears between
    the action cluster and Send — it opens a right-anchored popover (existing
    menu-catcher + dropdown pattern) containing the message textarea and a
    "Send with message" button. The primary Send pod always sends
    immediately with the current draft, at every width.
  - ≤600px: session pod truncates harder (max-width 90px); status chip drops
    words → `● · 2 · 3`.
  - Old-WebKitGTK fallback: no container-query support → wide layout,
    identical to today's behavior (the `@container` rules simply don't
    apply). `light-dark()`-style `@supports` guard not needed.
- **Overflow proof:** with labels dropped, worst-case bar contents at 500px
  are: glyph (24) + session (90) + search (32) + chip (~110) + 5 icon pods
  (~170) + compose (32) + Send (~70) + gaps ≈ 560px… still tight, so at
  ≤560px Undo/Redo fold into the ⋯ menu as "↶ Undo / ↷ Redo" rows (CSS
  shows the menu rows via the same container query that hides the pods).
  Bar fits at 500px with margin.

### Accessible-name pinning (E2E contract)

Existing controls keep today's exact accessible names via `aria-label`;
**More** is the one new control and gets a new name.

| Control (new look) | UIA name |
| --- | --- |
| ↶ icon pod | `↶ Undo` |
| ↷ icon pod | `↷ Redo` |
| timeline icon pod | `Timeline` |
| + node pod | `+ Component` |
| Send pod | `Send to agent` |
| status chip: waiting segment | `● agent is waiting for your decisions` |
| status chip: open segment | `{n} open decision(s)` (exact current format) |
| status chip: queued segment | `{n} to send` |
| ⋯ pod | `More` |
| Export entry (in ⋯) | `Export ▾` |
| Theme entry (in ⋯) | `Theme ▾` |

`verify-windows.ps1` changes: insert `Click-Element $hwnd 'More'` before the
`'Export ▾'` and `'Theme ▾'` clicks. Everything else runs at the default
(wide) window size and is unaffected.

## 3. Canvas & node cards

- **Card:** 12px radius; 3px status-colored **top rail** (a `div.node-rail`
  first child, full width) replaces the 3px left border; all card borders
  become uniform 1px, and horizontal padding grows 12→13px so the text
  content width stays exactly 232px — text wrapping (and therefore height
  estimation) is unaffected by the border change. Titles in Space Grotesk 600. Status becomes
  small-caps colored text in the meta row (boxed `.node-status-tag` styling
  retired, same text content). Badges: open = filled amber pill (unchanged),
  decided = outline green, notes dim.
- **States:** selected = accent border + `--glow` ring; ripple = affected
  color ring + glow (as today, retuned); hover = raise 1px + faint glow.
- **Geometry sync:** rail adds 3px; `layout::estimate_height` gains the rail
  constant and its tests update in the same commit. Card width stays 260.
- **Canvas:** dot grid kept; add a static, very subtle radial vignette
  (edges darkened via `color-mix(var(--shadow) 6%, transparent)`) layered
  into the `.canvas-viewport` background (it does not pan with the graph).
- **Minimap:** 10px radius, viewport rectangle glows while dragging.
- **Empty state:** inline-SVG bolt glyph (accent-colored), wordmark in Space
  Grotesk, tagline, and the `claude mcp add …` command in a click-to-copy pod
  (uses the existing `copy_to_clipboard` helper; receipt: "copied the connect
  command"). This is the only other new behavior besides the compose popover.

## 4. Panels & menus

Dropdowns (session/export/theme/⋯), context menu, choice panel, activity
feed, timeline, diff panel: 12px radii, `--accent-soft` hover fills, pod
buttons, Space Grotesk headings, picked options get a `--glow` ring, panel
stays 360px wide. No structural or behavioral changes.

## 5. Out of scope / non-goals

- No layout-algorithm, MCP, store, session, or export changes.
- No per-theme palette edits; no new themes.
- No new features beyond the compose popover and copy-command pod.
- Native title bar untouched (already follows mode).

## Docs

README: update the two passages that place **Export ▾** / **Theme ▾**
directly in the top bar (now inside **More**), and the keyboard table stays
valid. ROADMAP: add a v0.8 "Storm UI" entry when shipping.

## Testing

- `cargo test` (layout estimate tests updated), `cargo clippy -- -D
  warnings`, `cargo fmt --check`.
- `powershell -File scripts\verify-windows.ps1` (full E2E, updated for the
  More menu) and `-DemoShot`.
- Manual screenshot pass: {nodestorm, solarized-light, gruvbox-dark} ×
  {wide 1280px, narrow 640px} via the verify script's screenshot hook.
