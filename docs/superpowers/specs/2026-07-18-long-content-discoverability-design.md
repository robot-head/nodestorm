# Long-content resilience and data discoverability design

**Date:** 2026-07-18
**Status:** approved

## Problem

Nodestorm accepts descriptive data from agents and users without practical
length limits. The interface currently assumes prose with convenient spaces.
Long identifiers, URLs, paths, session names, or generated labels can therefore
escape their boxes, overlap nearby content, or push controls outside the visible
panel.

A live WebKitGTK stress pass at 1280x840 reproduced the failures with long
document titles, node labels and descriptions, group names, choice prompts,
option content, activity messages, session names, and edge labels:

- unbroken node text crossed card boundaries and collided with edge labels;
- long SVG edge labels ran through several cards;
- the selected-node heading pushed its close button off-screen;
- connection rows omitted edge labels and edge status entirely;
- choice prompts, option labels, pros, cons, and affected-node identifiers
  overflowed the 360px detail panel;
- the activity feed exposed only the newest ten of up to 200 retained entries;
- responsive rules intentionally hid the document title without another
  complete on-screen representation.

The interface must remain scannable as a graph while making every meaningful
human-facing value discoverable without exporting the session.

## Chosen direction

Keep canvas cards compact and make the selected-node panel the complete source
of truth. Cards provide readable previews; selecting one exposes its full data
without clipping or horizontal scrolling.

This was chosen over:

1. **Wrapping only.** Smallest change, but edge labels, node metadata, old
   activity entries, and responsive document titles would remain hidden.
2. **Fully expanding cards.** Directly displays all content, but a few verbose
   nodes dominate the canvas and force zoom-to-fit to make every card illegible.

## Requirements

### 1. Wrapping policy

- Human-facing prose and identifiers wrap at normal word boundaries and may
  break anywhere when a token is wider than its container.
- Flex children that contain dynamic text have `min-width: 0`, preventing them
  from forcing sibling controls out of their boxes.
- Deliberate truncation is allowed only for canvas previews and the compact
  topbar. The complete value must be available in a nearby detail surface.
- Panels and menus do not gain horizontal scrollbars for ordinary content.

### 2. Canvas cards

- Node labels wrap but clamp to three lines.
- Node descriptions retain their existing four-line clamp.
- A card exposes the complete label and description through its accessible
  title text while the selected-node panel displays both in full.
- Long group pills stay within the metadata row. Their visible value may
  truncate to one line, with the full group available in the selected-node
  panel and title text.
- Card badges and the connect handle remain reachable.
- The layout height estimate matches the three-line label clamp and existing
  four-line description clamp so cards never overlap.
- Cluster cards use the same label behavior and expose the full group name.

### 3. Edge labels and connections

- Canvas edge labels are preview text, not the only representation of an edge.
  Labels longer than 32 characters render as the first 31 characters plus an
  ellipsis, with the complete label in SVG title text and the connection list.
- The selected-node panel lists every incident edge with:
  - direction relative to the selected node;
  - other endpoint label;
  - edge kind;
  - edge status;
  - complete optional edge label.
- Connection content wraps independently of the delete button, which remains
  visible and usable.

### 4. Selected-node panel

- The panel width is 360px when available and never exceeds the viewport.
- The heading is allowed to wrap while the close button remains pinned at the
  top-right.
- A compact metadata block exposes the node identifier, kind, status, and full
  group value. Empty group values are omitted.
- The complete description, every choice prompt and rationale, option label and
  summary, pro and con, affected-node identifier, note, and connection value
  wraps without clipping.
- Pros and cons use two columns above a 420px viewport and stack at 420px or
  narrower.
- Edit, connect, delete, save, dismiss, and option-selection controls remain
  reachable with long content.

Internal choice and option identifiers remain implementation details; they are
not human-facing content and are not added to the interface.

### 5. Global discoverability

- The compact topbar may continue to ellipsize or hide the document title, but
  the full title appears at the top of the session menu on every viewport.
- Session and More menus are bounded by the viewport and scroll vertically, so
  long names and every menu action remain reachable.
- Session names wrap or ellipsize without displacing badges and Compare
  controls; each complete name is available as title text.
- Expanding Activity shows all retained activity entries in a bounded,
  vertically scrollable feed. Long messages wrap, while timestamps remain
  visible.
- Timeline and session-diff panels wrap long tokens and remain vertically
  scrollable.
- The empty-state connection command wraps inside the available viewport.

### 6. Responsive and accessibility behavior

- Existing container-query topbar folds and accessible names remain intact.
- At narrow widths the detail panel occupies at most the viewport width.
- Text remains selectable in panels and menus where it is useful to copy.
- Native `title` text supplements, but never replaces, the complete visible
  representation required in the detail or session surfaces.
- All changes derive colors from existing semantic theme tokens and work across
  the twelve palette families in Auto, Light, and Dark modes.

## Implementation boundaries

- Reuse CSS wrapping, clamping, flex, and overflow primitives; add no dependency.
- Keep the model, MCP schemas, persistence, export format, and layout algorithm
  unchanged apart from the card-height estimate required by the label clamp.
- Add a small pure connection-display formatter rather than duplicating edge
  wording in the component.
- Do not add a permanent stress-demo CLI flag. Runtime verification can inject
  hostile data through the existing MCP interface.

## Testing

### Automated

- Add a failing layout test proving a very long label is capped at three lines
  in the card-height estimate.
- Add failing formatter tests proving connection details include direction,
  kind, status, endpoint label, and complete edge label.
- Add a stylesheet contract test for the selectors that enforce wrapping,
  pinned controls, bounded menus, activity scrolling, and narrow panel width.
- Run `cargo test`.
- Run `cargo clippy --all-targets -- -D warnings`.
- Run `cargo fmt --check`.

### Rendered stress pass

Inject a session through the existing MCP endpoint containing:

- spaced prose and 100+ character unbroken identifiers;
- a long URL/path;
- long document, node, group, session, choice, option, edge, and activity text;
- multiple retained activity entries;
- enough choice content to require panel scrolling.

Verify at 1280x840 and 520x840:

- no text crosses a card, panel, menu, or activity boundary;
- no horizontal scrollbar is needed to read complete detail content;
- card selection, panel close, option selection, connection deletion, session
  switching, Compare, and More-menu actions remain reachable;
- every previewed or truncated value has a complete representation in the
  selected-node panel or session menu;
- the activity feed exposes all retained entries by scrolling;
- dark and light modes remain readable.

Capture before/after screenshots as audit evidence; screenshots are verification
artifacts and are not committed to the repository.
