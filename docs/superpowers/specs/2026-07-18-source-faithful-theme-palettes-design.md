# Source-faithful theme palettes

Approved 2026-07-18.

## Goal

Keep the existing twelve theme families, picker order, persisted ids, and
light/dark mode behavior, while replacing approximate UI colors with the
established colors and role pairings for the theme named by each family.

## Palette policy

- Use the upstream palette's canonical light and dark variants when both exist.
- Map UI roles only to colors from that variant: background ladders use its
  background/surface colors; foregrounds use its text/comment colors; accents,
  statuses, and badges use its established semantic colors.
- Do not mix colors between families or introduce new derived hues merely to
  fill a role.
- Preserve a documented, source-faithful fallback for a dark-only family:
  its light variant may use a widely established companion theme, with the
  exact source named in the CSS comment.
- Nodestorm remains the product's own palette; its current family stays the
  default and is tuned only from its existing token set.

## Scope

Only `assets/main.css` palette declarations change. The Rust registry,
preference format, picker, token names, CSS `light-dark()` mechanism, and
all twelve stable family ids remain unchanged. There are no new dependencies
or runtime behavior changes.

## Source selection

The audit uses each palette project's maintained source rather than a third
party recolor. Canonical pairs are: Solarized Light/Dark, Gruvbox Light/Dark,
Catppuccin Latte/Mocha, Nord Polar Night/Snow Storm, Tokyo Night Night/Day,
One Dark/One Light, GitHub Primer Light/Dark, Everforest Light/Medium, and
Rosé Pine Dawn/Main. Dracula and Monokai use the most established light
companions currently represented in the app, but their token values must be
drawn only from the selected companion palette.

## Verification

The existing `src/theme.rs` integrity tests continue to assert that every
family defines all 20 required tokens using `light-dark()`. Add focused test
coverage that rejects color literals in a family block when they do not
belong to that family’s audited palette list. Run formatting, the focused
theme tests, and the full Rust test suite.

