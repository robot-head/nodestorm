# App icon and titlebar branding design

## Goal

Give Nodestorm one recognisable, neutral app mark and use it consistently in
the native window titlebar and the in-app topbar.

## Mark

The mark is a compact monochrome graph: three connected circular nodes around
a lightning-bolt cutout. It uses no fixed colour so it works against light and
dark system titlebars and remains readable at small sizes.

The topbar renders this geometry as self-contained inline SVG, so it has no
runtime asset lookup. A transparent PNG rendition supplies the native Tao
window icon.

## Integration

- Render the decorative topbar SVG inline, using the theme accent through
  `currentColor` and a document-unique bolt-cutout mask id.
- Keep the PNG asset under `assets/`.
- Configure the Dioxus/Tao `WindowBuilder` with the PNG icon so supported
  native titlebars and window switchers display it.
- Replace the topbar's standalone `ϟ` text glyph with the SVG mark while
  retaining the existing `nodestorm` wordmark and layout.

## Scope and constraints

No app behaviour, menus, window sizing, or packaging tile artwork changes.
The native icon must load without a runtime filesystem dependency. The topbar
mark must be decorative (`aria-hidden`) and self-contained, with no runtime
filesystem asset dependency.

## Verification

- Add focused coverage for icon construction/loading where practical.
- Run the Rust test suite and formatting checks.
- Manually launch the desktop app to confirm the native window icon and the
  topbar mark render correctly.
