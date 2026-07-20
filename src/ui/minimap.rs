//! Corner overview map: every card as a status-colored rect plus the current
//! viewport window. Click or drag anywhere on it to pan the main canvas.
//!
//! Very large graphs are virtualized: past [`VIRTUALIZE_ABOVE`] nodes the map
//! renders a fixed coarse grid of density cells (one rect per occupied cell,
//! colored by the most notable status inside it) instead of one rect per node,
//! so its DOM stays bounded no matter how big the graph grows.

use dioxus::prelude::*;

use crate::layout::{Layout, Rect};
use crate::model::{ElementStatus, SessionDoc};

use super::node_card::status_class;
use super::{ViewTransform, ViewportSize};

/// Maximum minimap size in px; the actual svg keeps the bounds' aspect.
const MINI_MAX_W: f64 = 168.0;
const MINI_MAX_H: f64 = 112.0;

/// Above this node count the minimap switches to grid density cells.
const VIRTUALIZE_ABOVE: usize = 400;
/// Density grid resolution when virtualized (bounds the rendered rect count).
const GRID_COLS: usize = 64;
const GRID_ROWS: usize = 42;

/// One rectangle to draw on the minimap: its status class and plane rect.
struct MiniRect {
    status: &'static str,
    rect: Rect,
}

/// Show-the-most-notable ordering so an interesting node isn't hidden by a
/// plain one sharing its density cell.
fn status_priority(status: ElementStatus) -> u8 {
    match status {
        ElementStatus::Existing => 0,
        ElementStatus::Proposed => 1,
        ElementStatus::Modified => 2,
        ElementStatus::Affected => 3,
        ElementStatus::Removed => 4,
    }
}

/// Build the rects to draw: one per node for small graphs, or one per occupied
/// density cell (deterministic, bounded by the grid) for large ones.
fn mini_rects(doc: &SessionDoc, layout: &Layout, bounds: Rect) -> Vec<MiniRect> {
    if doc.nodes.len() <= VIRTUALIZE_ABOVE {
        return doc
            .nodes
            .iter()
            .filter_map(|node| {
                layout.rects.get(&node.id).map(|r| MiniRect {
                    status: status_class(node.status),
                    rect: *r,
                })
            })
            .collect();
    }

    // Aggregate node centers into a fixed grid; keep the most notable status
    // per cell. A BTreeMap keeps the output order deterministic.
    let cell_w = bounds.w / GRID_COLS as f64;
    let cell_h = bounds.h / GRID_ROWS as f64;
    let mut cells: std::collections::BTreeMap<(usize, usize), ElementStatus> =
        std::collections::BTreeMap::new();
    for node in &doc.nodes {
        let Some(r) = layout.rects.get(&node.id) else {
            continue;
        };
        let cx = r.x + r.w / 2.0;
        let cy = r.y + r.h / 2.0;
        let col = (((cx - bounds.x) / cell_w) as usize).min(GRID_COLS - 1);
        let row = (((cy - bounds.y) / cell_h) as usize).min(GRID_ROWS - 1);
        cells
            .entry((col, row))
            .and_modify(|s| {
                if status_priority(node.status) > status_priority(*s) {
                    *s = node.status;
                }
            })
            .or_insert(node.status);
    }
    cells
        .into_iter()
        .map(|((col, row), status)| MiniRect {
            status: status_class(status),
            rect: Rect {
                x: bounds.x + col as f64 * cell_w,
                y: bounds.y + row as f64 * cell_h,
                w: cell_w,
                h: cell_h,
            },
        })
        .collect()
}

#[component]
pub fn Minimap(
    doc: Signal<SessionDoc>,
    layout: Memo<Layout>,
    transform: Signal<ViewTransform>,
    viewport: ViewportSize,
) -> Element {
    let l = layout.read();
    let b = l.bounds;
    if b.w <= 0.0 || b.h <= 0.0 {
        return rsx! {};
    }
    // Exact-aspect scale (no letterboxing) so cursor→plane math is linear.
    let scale = (MINI_MAX_W / b.w).min(MINI_MAX_H / b.h);
    let (w, h) = (b.w * scale, b.h * scale);
    let t = transform();
    // The viewport window in plane coordinates.
    let (vx, vy) = (-t.tx / t.scale, -t.ty / t.scale);
    let (vw, vh) = (viewport.width / t.scale, viewport.height / t.scale);
    let mut dragging = use_signal(|| false);

    let rects = mini_rects(&doc.read(), &l, b);

    let mut pan_to = move |ex: f64, ey: f64| {
        let px = b.x + ex / scale;
        let py = b.y + ey / scale;
        transform.with_mut(|t| {
            t.tx = viewport.width / 2.0 - px * t.scale;
            t.ty = viewport.height / 2.0 - py * t.scale;
        });
    };

    rsx! {
        svg {
            class: "minimap",
            width: "{w}",
            height: "{h}",
            view_box: "{b.x} {b.y} {b.w} {b.h}",
            onmousedown: move |ev| {
                ev.stop_propagation();
                dragging.set(true);
                let c = ev.element_coordinates();
                pan_to(c.x, c.y);
            },
            onmousemove: move |ev| {
                if dragging() {
                    ev.stop_propagation();
                    let c = ev.element_coordinates();
                    pan_to(c.x, c.y);
                }
            },
            onmouseup: move |ev| {
                ev.stop_propagation();
                dragging.set(false);
            },
            onmouseleave: move |_| dragging.set(false),
            for (i, m) in rects.iter().enumerate() {
                rect {
                    key: "{i}",
                    class: "mini-node mini-{m.status}",
                    x: "{m.rect.x}",
                    y: "{m.rect.y}",
                    width: "{m.rect.w}",
                    height: "{m.rect.h}",
                }
            }
            rect {
                class: "mini-viewport",
                x: "{vx}",
                y: "{vy}",
                width: "{vw}",
                height: "{vh}",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yare::parameterized;

    #[parameterized(
        existing = { ElementStatus::Existing, 0 },
        proposed = { ElementStatus::Proposed, 1 },
        modified = { ElementStatus::Modified, 2 },
        affected = { ElementStatus::Affected, 3 },
        removed = { ElementStatus::Removed, 4 },
    )]
    fn status_priorities_are_exact(status: ElementStatus, expected: u8) {
        assert2::assert!(status_priority(status) == expected);
    }

    #[test]
    fn small_graphs_render_one_rect_per_node() {
        let doc = crate::demo::demo_doc();
        let layout = crate::layout::compute(&doc);
        let rects = mini_rects(&doc, &layout, layout.bounds);
        assert2::assert!((rects.len()) == (doc.nodes.len()));
    }

    #[test]
    fn large_graphs_are_virtualized_to_a_bounded_grid() {
        let doc = crate::demo::big_doc(1500);
        let layout = crate::layout::compute(&doc);
        let rects = mini_rects(&doc, &layout, layout.bounds);
        // Far fewer rects than nodes, and never more than the grid.
        assert2::assert!(
            rects.len() < doc.nodes.len(),
            "virtualized: {}",
            rects.len()
        );
        assert2::assert!(rects.len() <= GRID_COLS * GRID_ROWS);
        // Deterministic.
        let again = mini_rects(&doc, &layout, layout.bounds);
        assert2::assert!((rects.len()) == (again.len()));
    }

    #[test]
    fn virtualized_cells_use_exact_geometry_and_highest_status_priority() {
        let mut doc = crate::demo::big_doc(VIRTUALIZE_ABOVE + 1);
        for node in &mut doc.nodes {
            node.status = ElementStatus::Existing;
        }
        doc.nodes.last_mut().unwrap().status = ElementStatus::Removed;
        let mut layout = Layout::default();
        for node in &doc.nodes {
            layout.rects.insert(
                node.id.clone(),
                Rect {
                    x: 20.0,
                    y: 40.0,
                    w: 10.0,
                    h: 10.0,
                },
            );
        }
        let rects = mini_rects(
            &doc,
            &layout,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 640.0,
                h: 420.0,
            },
        );
        assert2::assert!((rects.len()) == (1));
        assert2::assert!((rects[0].status) == ("removed"));
        assert2::assert!(
            (rects[0].rect)
                == (Rect {
                    x: 20.0,
                    y: 40.0,
                    w: 10.0,
                    h: 10.0,
                })
        );

        for rect in layout.rects.values_mut() {
            *rect = Rect {
                x: 19.0,
                y: 39.0,
                w: 4.0,
                h: 4.0,
            };
        }
        let boundary = mini_rects(
            &doc,
            &layout,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 640.0,
                h: 420.0,
            },
        );
        assert2::assert!((boundary.len()) == (1));
        assert2::assert!((boundary[0].rect.x) == (20.0));
        assert2::assert!((boundary[0].rect.y) == (40.0));

        for rect in layout.rects.values_mut() {
            rect.x = 1_000.0;
        }
        let clamped = mini_rects(
            &doc,
            &layout,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 640.0,
                h: 420.0,
            },
        );
        assert2::assert!((clamped.len()) == (1));
        assert2::assert!((clamped[0].rect.x) == (640.0));

        for rect in layout.rects.values_mut() {
            rect.y = 1_000.0;
        }
        let clamped = mini_rects(
            &doc,
            &layout,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 640.0,
                h: 420.0,
            },
        );
        assert2::assert!((clamped.len()) == (1));
        assert2::assert!((clamped[0].rect.y) == (430.0));
    }
}
