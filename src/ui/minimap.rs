//! Corner overview map: every card as a status-colored rect plus the current
//! viewport window. Click or drag anywhere on it to pan the main canvas.

use dioxus::prelude::*;

use crate::layout::Layout;
use crate::model::SessionDoc;

use super::node_card::status_class;
use super::{ViewTransform, ViewportSize};

/// Maximum minimap size in px; the actual svg keeps the bounds' aspect.
const MINI_MAX_W: f64 = 168.0;
const MINI_MAX_H: f64 = 112.0;

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
            for node in doc.read().nodes.iter() {
                if let Some(r) = l.rects.get(&node.id) {
                    rect {
                        key: "{node.id}",
                        class: "mini-node mini-{status_class(node.status)}",
                        x: "{r.x}",
                        y: "{r.y}",
                        width: "{r.w}",
                        height: "{r.h}",
                    }
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
