# Topbar title hover-card design

**Date:** 2026-07-18  
**Status:** approved

## Goal

Keep the compact, ellipsized topbar title while showing its complete value in
a styled card when the pointer hovers over it.

## Design

The topbar title retains the complete title in a `data-full-title` attribute.
CSS creates a positioned `::after` card from that value and reveals it with
`:hover`. The card wraps long tokens, stays inside the viewport, uses existing
theme tokens, and does not change the topbar's layout or add dependencies.

The existing native `title` attribute remains as a platform fallback.

## Testing

Extend the stylesheet/source contract tests to require the complete-title data
attribute and the hover-card's positioning, wrapping, viewport bound, and
hover visibility rules. Run the focused test and the full Rust test suite.
