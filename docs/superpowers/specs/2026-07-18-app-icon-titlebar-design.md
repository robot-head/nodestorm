# App icon and launcher branding design

## Goal

Replace the current letter-like icon with one recognisable Nodestorm mark and
use it consistently in the in-app topbar, native window chrome, and packaged
desktop launchers.

## Mark

The mark is a bold, right-leaning lightning path made from three thick
connected segments. Circular nodes are fused to both endpoints and the main
bend. The open zigzag must read as a lightning bolt first and must not form a
triangle, enclosure, or letter-like silhouette.

The geometry is monochrome and uses generous negative space. It must remain
recognisable at 16-17 px without glow, fine outlines, internal holes, or
decorative detail.

## Variants

The in-app variant is the standalone transparent mark. It inherits the active
theme accent through `currentColor` and has no background tile.

The OS-facing variant centers the same mark in white on a neutral charcoal
rounded tile with generous padding. This variant supplies the native window
icon and every packaged launcher icon. It uses no glow, border ornament, or
platform-specific redraw.

## Integration

- Keep one canonical SVG geometry for the bolt-and-nodes mark.
- Render the decorative topbar variant inline with `currentColor`, preserving
  the existing 17 px footprint, `nodestorm` wordmark, and responsive behavior.
- Generate the rounded-tile raster assets deterministically from the canonical
  geometry rather than maintaining unrelated hand-drawn variants.
- Embed the PNG used by Dioxus/Tao so the native window icon has no runtime
  filesystem dependency.
- Generate and package Windows launcher/Store sizes, macOS `.icns` sizes, and
  Linux hicolor PNGs from the same rounded-tile source.
- Install a Linux desktop entry that references the packaged hicolor icon.

## Scope and constraints

No app behavior, menus, or window sizing changes. The topbar mark must be
decorative (`aria-hidden`) and self-contained. Launcher generation must be
repeatable and must not require network access. Windows, macOS, and Linux
packages must each contain the icon format their launcher uses.

## Verification

- Validate the canonical geometry does not regress into an enclosed or
  letter-like form and that the topbar uses the standalone variant.
- Validate raster dimensions, alpha, native Tao icon construction, and package
  references for Windows, macOS, and Linux.
- Run Rust, packaging, formatting, and build checks.
- Manually inspect the topbar at wide and narrow widths and inspect the
  launcher/titlebar icon against light and dark system chrome.
