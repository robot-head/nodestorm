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
use crate::model::{EdgeKind, NodeId, NodeKind, Point, SessionDoc};

use super::ViewTransform;
use super::app::use_store;
use super::context_menu::{ContextMenu, MenuAction, MenuTarget};
use super::edge_layer::EdgeLayer;
use super::minimap::Minimap;
use super::node_card::NodeCard;

use super::{TOPBAR_H, VIEW_H, VIEW_W};

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
    let mut connect_from = use_context::<super::ConnectFrom>().0;
    let mut zoom_target = use_context::<super::ZoomTarget>().0;
    let search = use_context::<super::SearchQuery>().0;
    // Cursor position (plane coords) while a connect drag is live.
    let mut ghost_to: Signal<Option<(f64, f64)>> = use_signal(|| None);
    // Open right-click menu: (client_x, client_y, target).
    let mut menu: Signal<Option<(f64, f64, MenuTarget)>> = use_signal(|| None);

    // Client → plane coordinates (the canvas sits below the 48px topbar).
    let to_plane = move |cx: f64, cy: f64| {
        let t = transform();
        ((cx - t.tx) / t.scale, (cy - TOPBAR_H - t.ty) / t.scale)
    };
    // Center the view on a node and zoom in a step (double-click a card).
    let mut zoom_to = move |rect: crate::layout::Rect| {
        transform.with_mut(|t| {
            t.scale = t.scale.max(1.2).clamp(MIN_SCALE, MAX_SCALE);
            t.tx = VIEW_W / 2.0 - (rect.x + rect.w / 2.0) * t.scale;
            t.ty = VIEW_H / 2.0 - (rect.y + rect.h / 2.0) * t.scale;
        });
    };
    // One-shot zoom requests (search Enter-cycling).
    use_effect(move || {
        if let Some(id) = zoom_target() {
            let rect = layout.read().rects.get(&id).copied();
            if let Some(rect) = rect {
                zoom_to(rect);
            }
            zoom_target.set(None);
        }
    });

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
    let viewport_class = match (panning, connect_from().is_some()) {
        (true, _) => "canvas-viewport panning",
        (false, true) => "canvas-viewport connecting",
        (false, false) => "canvas-viewport",
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
                    if connect_from().is_some() {
                        ghost_to.set(Some(to_plane(c.x, c.y)));
                    }
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
                    // A motionless background click deselects — or cancels an
                    // armed connect.
                    let t = transform();
                    if (t.tx - orig.0).abs() < 3.0 && (t.ty - orig.1).abs() < 3.0 {
                        let _ = start;
                        if connect_from().is_some() {
                            connect_from.set(None);
                        } else {
                            selected.set(None);
                        }
                    }
                } else if state.gesture.is_none() && connect_from().is_some() {
                    // A connect drag released over the background: cancel.
                    connect_from.set(None);
                }
                ghost_to.set(None);
                gesture.set(GestureState::default());
            },
            onmouseleave: move |_| {
                ghost_to.set(None);
                gesture.set(GestureState::default());
            },
            ondoubleclick: {
                let store = store.clone();
                move |ev: MouseEvent| {
                    let c = ev.client_coordinates();
                    let (px, py) = to_plane(c.x, c.y);
                    match store.add_user_node(
                        "New component".into(),
                        NodeKind::Component,
                        Some(Point { x: px, y: py }),
                    ) {
                        Ok(id) => selected.set(Some(id)),
                        Err(err) => tracing::warn!(%err, "add component failed"),
                    }
                }
            },
            oncontextmenu: move |ev: MouseEvent| {
                ev.prevent_default();
                let c = ev.client_coordinates();
                let (px, py) = to_plane(c.x, c.y);
                menu.set(Some((
                    c.x,
                    c.y,
                    MenuTarget::Background {
                        plane_x: px,
                        plane_y: py,
                    },
                )));
            },
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
                EdgeLayer {
                    layout,
                    ghost: connect_from().and_then(|from| {
                        let l = layout.read();
                        let r = l.rects.get(&from)?;
                        let to = ghost_to()?;
                        Some(((r.x + r.w / 2.0, r.y + r.h / 2.0), to))
                    }),
                    on_edge_context: move |(from, to, kind, cx, cy)| {
                        menu.set(Some((cx, cy, MenuTarget::Edge(from, to, kind))));
                    },
                }
                for node in d.nodes.iter() {
                    if let Some(rect) = l.rects.get(&node.id) {
                        NodeCard {
                            key: "{node.id}",
                            node: node.clone(),
                            rect: *rect,
                            selected: selected() == Some(node.id.clone()),
                            highlighted: hovered_affects.read().contains(&node.id),
                            search_hit: super::node_matches(node, &search.read()),
                            search_dim: !search.read().trim().is_empty()
                                && !super::node_matches(node, &search.read()),
                            on_select: {
                                let id = node.id.clone();
                                let store = store.clone();
                                move |_| {
                                    // An armed connect turns the next card
                                    // click into edge creation.
                                    if let Some(from) = connect_from() {
                                        if from != id
                                            && let Err(err) = store.add_user_edge(
                                                &from,
                                                &id,
                                                EdgeKind::DependsOn,
                                            )
                                        {
                                            tracing::warn!(%err, "connect failed");
                                        }
                                        connect_from.set(None);
                                    } else {
                                        selected.set(Some(id.clone()));
                                    }
                                }
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
                            on_connect_start: {
                                let id = node.id.clone();
                                move |_| {
                                    connect_from.set(Some(id.clone()));
                                    ghost_to.set(None);
                                }
                            },
                            on_connect_drop: {
                                let id = node.id.clone();
                                let store = store.clone();
                                move |_| {
                                    // End of a handle drag over this card.
                                    if let Some(from) = connect_from() {
                                        if from != id
                                            && let Err(err) = store.add_user_edge(
                                                &from,
                                                &id,
                                                EdgeKind::DependsOn,
                                            )
                                        {
                                            tracing::warn!(%err, "connect failed");
                                        }
                                        connect_from.set(None);
                                        ghost_to.set(None);
                                    }
                                }
                            },
                            on_context: {
                                let id = node.id.clone();
                                move |ev: MouseEvent| {
                                    let c = ev.client_coordinates();
                                    menu.set(Some((c.x, c.y, MenuTarget::Node(id.clone()))));
                                }
                            },
                            on_zoom: {
                                let rect = *rect;
                                move |_| zoom_to(rect)
                            },
                        }
                    }
                }
            }
            if let Some((mx, my, target)) = menu() {
                ContextMenu {
                    x: mx,
                    y: my,
                    target,
                    on_action: {
                        let store = store.clone();
                        move |action: MenuAction| {
                            match action {
                                MenuAction::SelectNode(id) => selected.set(Some(id)),
                                MenuAction::Connect(id) => connect_from.set(Some(id)),
                                MenuAction::DeleteNode(id) => {
                                    if let Err(err) = store.delete_node(&id) {
                                        tracing::warn!(%err, "delete_node failed");
                                    }
                                    if selected() == Some(id) {
                                        selected.set(None);
                                    }
                                }
                                MenuAction::DeleteEdge(from, to, kind) => {
                                    if let Err(err) = store.delete_edge(&from, &to, kind) {
                                        tracing::warn!(%err, "delete_edge failed");
                                    }
                                }
                                MenuAction::AddHere { plane_x, plane_y } => {
                                    match store.add_user_node(
                                        "New component".into(),
                                        NodeKind::Component,
                                        Some(Point {
                                            x: plane_x,
                                            y: plane_y,
                                        }),
                                    ) {
                                        Ok(id) => selected.set(Some(id)),
                                        Err(err) => {
                                            tracing::warn!(%err, "add component failed");
                                        }
                                    }
                                }
                            }
                            menu.set(None);
                        }
                    },
                    on_close: move |()| menu.set(None),
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
            Minimap { doc, layout, transform }
        }
    }
}
