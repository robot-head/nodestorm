# Microsoft Store logo assets — Electric Topology

**Date:** 2026-07-19
**Status:** approved in-session

## Goal

Create a coherent Microsoft Store image set for nodestorm that carries the
existing Storm identity into Store listings and remains recognizable from a
2160 px poster down to a 71 px tile.

The set uses a full wordmark only where there is enough room. Square box art
and tile icons use a symbol-only treatment for reliable small-size rendering.

## Art direction

**Electric Topology** combines a sharp lightning bolt with three connected
graph nodes. It represents both halves of the product: the electric Storm
identity and the live architecture canvas.

- Background: nodestorm midnight `#0f1117`.
- Primary mark: electric blue `#6c9ef8`, with a pale-blue highlight.
- Supporting detail: restrained canvas dots and connection paths using darker
  blue-gray values derived from the app UI.
- Type: lowercase `nodestorm` in Space Grotesk, matching the in-app wordmark.
- Finish: crisp geometric forms with controlled glow on large artwork.
- Exclusions: screenshots, slogans, unrelated weather imagery, photorealism,
  fine detail near crop edges, watermarks, and third-party branding.

The bolt/node symbol must have one stable silhouette across every asset. Small
variants simplify effects and supporting detail, but do not redraw or reorder
the symbol.

## Composition

### 9:16 poster art

The symbol is centered slightly above the visual midpoint. The lowercase
`nodestorm` wordmark sits beneath it with generous separation. A sparse node
network and dot grid add depth behind the mark without competing with it.

Keep all essential content inside the central 70% of the canvas so Store and
Xbox presentation crops cannot clip the mark or wordmark. The outer region is
background-only. Glow stays subtle enough that the mark retains a hard edge.

### 1:1 box art

Use the symbol alone, centered and substantially larger than on the poster.
A very subtle dot grid may remain, but the wordmark and background network are
omitted. Keep generous safe space around the silhouette for flexible Store
layouts.

### 1:1 app tile icons

Use the same symbol-only composition. Remove the grid, network, and ambient
glow. The 71 px version is a flat, high-contrast rendition with thicker nodes
and connections; the 150 px and 300 px variants may retain a restrained inner
highlight. All three use an opaque midnight background.

## Deliverables

All files are PNG, under 50 MB each, in `assets/store/`:

The Store labels the poster slot 9:16 even though the supplied pixel sizes are
2:3. The exact listed pixel dimensions below are authoritative for export.

| Store role | Dimensions | Filename |
| --- | ---: | --- |
| 9:16 poster art | 720 × 1080 | `poster-720x1080.png` |
| 9:16 poster art | 1440 × 2160 | `poster-1440x2160.png` |
| 1:1 box art | 1080 × 1080 | `box-art-1080x1080.png` |
| 1:1 box art | 2160 × 2160 | `box-art-2160x2160.png` |
| 1:1 app tile icon | 300 × 300 | `app-tile-300x300.png` |
| 1:1 app tile icon | 150 × 150 | `app-tile-150x150.png` |
| 1:1 app tile icon | 71 × 71 | `app-tile-71x71.png` |

The larger poster and box-art files are the master compositions. Their smaller
counterparts are high-quality downscales of the same composition. Tile icons
are purpose-built from the same symbol rather than downscaled box art.

## Production workflow

1. Generate one high-resolution poster master and one square symbol master
   from the approved direction.
2. Inspect both masters for symbol consistency, text accuracy, edge safety,
   artifacts, and unintended imagery.
3. Make a single targeted correction if inspection reveals a defect.
4. Export the two required poster sizes and two box-art sizes.
5. Derive simplified 300, 150, and 71 px tile variants from the approved
   symbol, adjusting stroke weight only to preserve legibility.
6. Store every final PNG in `assets/store/`; keep generation intermediates out
   of the committed deliverable set.

## Validation

- Verify every image is PNG and has the exact required pixel dimensions.
- Verify every file is below 50 MB.
- Compare the poster and box artwork side by side: the symbol silhouette,
  node count, and node placement must match.
- Inspect the poster at full size and at 25%: `nodestorm` must be spelled
  exactly and remain readable.
- Inspect the 71 px tile at native size: the bolt and three nodes must be
  immediately distinguishable, with no collapsed connections or muddy glow.
- Check that the poster's essential content remains inside its central 70%.
- Confirm every image is opaque and contains no accidental alpha fringe.

## Integration boundary

This work produces Store listing images only. It does not replace package
manifest assets or change `packaging/windows/prepare-layout.ps1`. Updating the
MSIX package icons from the current demo-poster crop is separate follow-up work.
