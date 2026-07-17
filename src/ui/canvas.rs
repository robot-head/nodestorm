//! The pan/zoom canvas: HTML node cards over an SVG edge layer, both inside
//! one CSS-transformed plane.
//!
//! Interaction model:
//! - drag the background → pan
//! - mouse wheel → zoom around the cursor
//! - drag a card → move (and pin) that node
//! - click a card → select; click the background → deselect

use dioxus::prelude::*;

use crate::layout::Layout;
use crate::model::{NodeId, Point, SessionDoc};

use super::ViewTransform;
use super::app::use_store;
use super::edge_layer::EdgeLayer;
use super::node_card::NodeCard;

/// Nominal viewport for initial zoom-to-fit (the topbar is 48px).
const VIEW_W: f64 = 1280.0;
const VIEW_H: f64 = 780.0;
const TOPBAR_H: f64 = 48.0;
const MIN_SCALE: f64 = 0.15;
const MAX_SCALE: f64 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Gesture {
    Pan {
        start: (f64, f64),
        orig: (f64, f64),
    },
    DragNode {
        start: (f64, f64),
        orig: (f64, f64),
        moved: bool,
    },
}

/// Which node a drag applies to travels outside the enum to keep it `Copy`-free.
#[derive(Debug, Clone, PartialEq, Default)]
struct GestureState {
    gesture: Option<Gesture>,
    node: Option<NodeId>,
}

#[component]
pub fn Canvas(
    doc: Signal<SessionDoc>,
    layout: Memo<Layout>,
    selected: Signal<Option<NodeId>>,
    hovered_affects: Signal<Vec<NodeId>>,
) -> Element {
    let store = use_store();
    let mut transform = use_signal(|| ViewTransform::fit(&layout.read().bounds, VIEW_W, VIEW_H));
    let mut gesture: Signal<GestureState> = use_signal(GestureState::default);

    // When the agent moves the focus, pan so that node is centered.
    let mut last_focus = use_signal(|| doc.read().focus.clone());
    use_effect(move || {
        let focus = doc.read().focus.clone();
        if focus != last_focus() {
            last_focus.set(focus.clone());
            if let Some(id) = focus
                && let Some(rect) = layout.read().rects.get(&id)
            {
                transform.with_mut(|t| {
                    t.tx = VIEW_W / 2.0 - (rect.x + rect.w / 2.0) * t.scale;
                    t.ty = VIEW_H / 2.0 - (rect.y + rect.h / 2.0) * t.scale;
                });
            }
        }
    });

    let t = transform();
    let l = layout.read();
    let d = doc.read();
    let panning = matches!(gesture.read().gesture, Some(Gesture::Pan { .. }));
    let viewport_class = if panning {
        "canvas-viewport panning"
    } else {
        "canvas-viewport"
    };

    rsx! {
        div {
            class: "{viewport_class}",
            onmousedown: move |ev| {
                let c = ev.client_coordinates();
                let t = transform();
                gesture.set(GestureState {
                    gesture: Some(Gesture::Pan { start: (c.x, c.y), orig: (t.tx, t.ty) }),
                    node: None,
                });
            },
            onmousemove: {
                let store = store.clone();
                move |ev| {
                    let c = ev.client_coordinates();
                    let state = gesture();
                    match state.gesture {
                        Some(Gesture::Pan { start, orig }) => {
                            transform.with_mut(|t| {
                                t.tx = orig.0 + (c.x - start.0);
                                t.ty = orig.1 + (c.y - start.1);
                            });
                        }
                        Some(Gesture::DragNode { start, orig, moved }) => {
                            if let Some(id) = &state.node {
                                let t = transform();
                                let nx = orig.0 + (c.x - start.0) / t.scale;
                                let ny = orig.1 + (c.y - start.1) / t.scale;
                                store.set_position(id, Point { x: nx, y: ny });
                                if !moved {
                                    gesture.set(GestureState {
                                        gesture: Some(Gesture::DragNode { start, orig, moved: true }),
                                        node: state.node.clone(),
                                    });
                                }
                            }
                        }
                        None => {}
                    }
                }
            },
            onmouseup: move |_| {
                let state = gesture();
                if let Some(Gesture::Pan { start, orig }) = state.gesture {
                    // A motionless background click deselects.
                    let t = transform();
                    if (t.tx - orig.0).abs() < 3.0 && (t.ty - orig.1).abs() < 3.0 {
                        let _ = start;
                        selected.set(None);
                    }
                }
                gesture.set(GestureState::default());
            },
            onmouseleave: move |_| gesture.set(GestureState::default()),
            onwheel: move |ev| {
                ev.prevent_default();
                let delta = ev.delta().strip_units().y;
                let factor = (-delta * 0.0018).exp().clamp(0.6, 1.6);
                let c = ev.client_coordinates();
                let (cx, cy) = (c.x, c.y - TOPBAR_H);
                transform.with_mut(|t| {
                    let new_scale = (t.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
                    let real = new_scale / t.scale;
                    t.tx = cx - (cx - t.tx) * real;
                    t.ty = cy - (cy - t.ty) * real;
                    t.scale = new_scale;
                });
            },
            div {
                class: "canvas-plane",
                style: "transform: translate({t.tx}px, {t.ty}px) scale({t.scale});",
                EdgeLayer { layout }
                for node in d.nodes.iter() {
                    if let Some(rect) = l.rects.get(&node.id) {
                        NodeCard {
                            key: "{node.id}",
                            node: node.clone(),
                            rect: *rect,
                            selected: selected() == Some(node.id.clone()),
                            highlighted: hovered_affects.read().contains(&node.id),
                            on_select: {
                                let id = node.id.clone();
                                move |_| selected.set(Some(id.clone()))
                            },
                            on_drag_start: {
                                let id = node.id.clone();
                                let rect = *rect;
                                move |ev: MouseEvent| {
                                    let c = ev.client_coordinates();
                                    gesture.set(GestureState {
                                        gesture: Some(Gesture::DragNode {
                                            start: (c.x, c.y),
                                            orig: (rect.x, rect.y),
                                            moved: false,
                                        }),
                                        node: Some(id.clone()),
                                    });
                                }
                            },
                        }
                    }
                }
            }
            div { class: "canvas-controls",
                button {
                    class: "ctl-btn",
                    title: "Zoom to fit",
                    onclick: move |_| {
                        transform.set(ViewTransform::fit(&layout.read().bounds, VIEW_W, VIEW_H));
                    },
                    "⤢ fit"
                }
            }
        }
    }
}
