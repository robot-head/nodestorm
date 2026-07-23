# Title-bar control depth

Approved 2026-07-18.

## Goal

Make every title-bar control feel more tactile and less pill-like while
preserving Nodestorm's calm, light interface.

## Scope

Only `assets/main.css` changes. The session selector, search field, status
chip, action pods, More control, and message field gain the same visual
language: 10px corners, a subtle top highlight, a 2px lower edge, and a soft
drop shadow. Their existing semantic colors, labels, dimensions, responsive
rules, and behavior remain unchanged.

## Interaction

Hover and focus states retain their current affordances. Pressing a control
briefly reduces its raised effect so it feels physically settled without
changing layout or accessibility behavior.

## Verification

Run the existing CSS-appropriate project checks and inspect the title bar at
desktop and narrow responsive widths. Confirm all control types have the new
depth treatment while focus visibility and overflow behavior remain intact.
