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
use super::cluster_card::ClusterCard;
use super::context_menu::{ContextMenu, MenuAction, MenuTarget};
use super::edge_layer::EdgeLayer;
use super::minimap::Minimap;
use super::node_card::NodeCard;

use super::{TOPBAR_H, ViewportSize};

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
    let mut viewport = use_signal(|| ViewportSize::FALLBACK);
    let fallback = ViewportSize::FALLBACK;
    let mut transform =
        use_signal(|| ViewTransform::fit(&layout.read().bounds, fallback.width, fallback.height));
    let mut gesture: Signal<GestureState> = use_signal(GestureState::default);
    let mut connect_from = use_context::<super::ConnectFrom>().0;
    let mut zoom_target = use_context::<super::ZoomTarget>().0;
    let mut search = use_context::<super::SearchQuery>().0;
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
        let viewport = viewport();
        transform.with_mut(|t| {
            t.scale = t.scale.max(1.2).clamp(MIN_SCALE, MAX_SCALE);
            t.tx = viewport.width / 2.0 - (rect.x + rect.w / 2.0) * t.scale;
            t.ty = viewport.height / 2.0 - (rect.y + rect.h / 2.0) * t.scale;
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
    // Zoom about the viewport center (keyboard +/-).
    let mut zoom_step = move |factor: f64| {
        let viewport = viewport();
        transform.with_mut(|t| {
            let (cx, cy) = (viewport.width / 2.0, viewport.height / 2.0);
            let new_scale = (t.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
            let real = new_scale / t.scale;
            t.tx = cx - (cx - t.tx) * real;
            t.ty = cy - (cy - t.ty) * real;
            t.scale = new_scale;
        });
    };
    // Arrow-key selection: nearest node inside a cone in that direction.
    let mut move_selection = move |dir: (f64, f64)| {
        let l = layout.read();
        let d = doc.read();
        let Some(cur) = selected() else {
            if let Some(first) = d.nodes.first() {
                selected.set(Some(first.id.clone()));
            }
            return;
        };
        let Some(cr) = l.rects.get(&cur) else { return };
        let (cx, cy) = (cr.x + cr.w / 2.0, cr.y + cr.h / 2.0);
        let mut best: Option<(f64, NodeId)> = None;
        for n in &d.nodes {
            if n.id == cur {
                continue;
            }
            let Some(r) = l.rects.get(&n.id) else {
                continue;
            };
            let (dx, dy) = (r.x + r.w / 2.0 - cx, r.y + r.h / 2.0 - cy);
            let along = dx * dir.0 + dy * dir.1;
            if along <= 0.0 {
                continue;
            }
            let ortho = (dx * dir.1 - dy * dir.0).abs();
            if ortho > along * 1.6 {
                continue;
            }
            let score = along + ortho * 0.8;
            if best.as_ref().is_none_or(|(s, _)| score < *s) {
                best = Some((score, n.id.clone()));
            }
        }
        if let Some((_, id)) = best {
            selected.set(Some(id));
        }
    };
    // Tab cycling in document order.
    let mut cycle_selection = move |step: isize| {
        let d = doc.read();
        if d.nodes.is_empty() {
            return;
        }
        let len = d.nodes.len() as isize;
        let cur = selected()
            .and_then(|id| d.nodes.iter().position(|n| n.id == id))
            .map_or(0, |i| (i as isize + step).rem_euclid(len));
        selected.set(Some(d.nodes[cur as usize].id.clone()));
    };

    // When the agent moves the focus, pan so that node is centered.
    let mut last_focus = use_signal(|| doc.read().focus.clone());
    use_effect(move || {
        let focus = doc.read().focus.clone();
        if focus != last_focus() {
            last_focus.set(focus.clone());
            if let Some(id) = focus
                && let Some(rect) = layout.read().rects.get(&id)
            {
                let viewport = viewport();
                transform.with_mut(|t| {
                    t.tx = viewport.width / 2.0 - (rect.x + rect.w / 2.0) * t.scale;
                    t.ty = viewport.height / 2.0 - (rect.y + rect.h / 2.0) * t.scale;
                });
            }
        }
    });

    let t = transform();
    let viewport_size = viewport();
    let l = layout.read();
    let d = doc.read();
    // Viewport culling: only on-screen(±margin) cards, clusters, and edges
    // hit the DOM — the difference between 30 and 300 components.
    let vis = crate::layout::visible_set(
        &l,
        t.tx,
        t.ty,
        t.scale,
        viewport_size.width,
        viewport_size.height,
    );
    let panning = matches!(gesture.read().gesture, Some(Gesture::Pan { .. }));
    let viewport_class = match (panning, connect_from().is_some()) {
        (true, _) => "canvas-viewport panning",
        (false, true) => "canvas-viewport connecting",
        (false, false) => "canvas-viewport",
    };

    rsx! {
        div {
            class: "{viewport_class}",
            tabindex: "0",
            onresize: move |ev: ResizeEvent| {
                let Ok(size) = ev.data().get_content_box_size() else {
                    return;
                };
                let Some(next) = ViewportSize::observed(size.width, size.height) else {
                    return;
                };
                let old = viewport();
                if next != old {
                    transform.with_mut(|t| t.reframe(old, next));
                    viewport.set(next);
                }
            },
            onkeydown: {
                let store = store.clone();
                move |ev: KeyboardEvent| {
                    match ev.key() {
                        Key::Delete | Key::Backspace => {
                            if let Some(id) = selected() {
                                if let Err(err) = store.delete_node(&id) {
                                    tracing::warn!(%err, "delete_node failed");
                                }
                                selected.set(None);
                            }
                        }
                        Key::Escape => {
                            // First non-empty wins: connect → menu → search
                            // → selection.
                            if connect_from().is_some() {
                                connect_from.set(None);
                                ghost_to.set(None);
                            } else if menu().is_some() {
                                menu.set(None);
                            } else if !search.read().is_empty() {
                                search.set(String::new());
                            } else {
                                selected.set(None);
                            }
                        }
                        Key::ArrowRight => move_selection((1.0, 0.0)),
                        Key::ArrowLeft => move_selection((-1.0, 0.0)),
                        Key::ArrowDown => move_selection((0.0, 1.0)),
                        Key::ArrowUp => move_selection((0.0, -1.0)),
                        Key::Tab => {
                            ev.prevent_default();
                            cycle_selection(if ev.modifiers().shift() { -1 } else { 1 });
                        }
                        Key::Enter => {
                            if selected().is_none() {
                                let d = doc.read();
                                let id = d.focus.clone().or_else(|| {
                                    d.nodes.first().map(|n| n.id.clone())
                                });
                                if let Some(id) = id {
                                    selected.set(Some(id));
                                }
                            }
                        }
                        Key::Character(c) if ev.modifiers().ctrl() => match c.as_str() {
                            "z" => {
                                store.undo();
                            }
                            "y" => {
                                store.redo();
                            }
                            _ => {}
                        },
                        Key::Character(c) => match c.as_str() {
                            "+" | "=" => zoom_step(1.25),
                            "-" => zoom_step(0.8),
                            "0" => {
                                let viewport = viewport();
                                transform.set(ViewTransform::fit(
                                    &layout.read().bounds,
                                    viewport.width,
                                    viewport.height,
                                ));
                            }
                            "n" => {
                                let t = transform();
                                let viewport = viewport();
                                let px = (viewport.width / 2.0 - t.tx) / t.scale;
                                let py = (viewport.height / 2.0 - t.ty) / t.scale;
                                match store.add_user_node(
                                    "New component".into(),
                                    NodeKind::Component,
                                    Some(Point { x: px, y: py }),
                                ) {
                                    Ok(id) => selected.set(Some(id)),
                                    Err(err) => {
                                        tracing::warn!(%err, "add component failed");
                                    }
                                }
                            }
                            "/" => {
                                ev.prevent_default();
                                document::eval(
                                    "const b = document.querySelector('.search-box'); \
                                     if (b) b.focus();",
                                );
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            },
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
                    visible: vis.edges.clone(),
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
                for &ci in vis.clusters.iter() {
                    ClusterCard {
                        key: "{l.clusters[ci].group}",
                        group: l.clusters[ci].group.clone(),
                        rect: l.clusters[ci].rect,
                        member_count: l.clusters[ci].member_count,
                        on_expand: {
                            let store = store.clone();
                            let group = l.clusters[ci].group.clone();
                            move |()| store.toggle_group_collapsed(&group)
                        },
                    }
                }
                for node in d.nodes.iter().filter(|n| vis.nodes.contains(&n.id)) {
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
                                let store = store.clone();
                                move |ev: MouseEvent| {
                                    // One undo entry per drag, not per move.
                                    store.checkpoint_position(&id);
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
                                let group = node.group.clone();
                                move |ev: MouseEvent| {
                                    let c = ev.client_coordinates();
                                    menu.set(Some((
                                        c.x,
                                        c.y,
                                        MenuTarget::Node(id.clone(), group.clone()),
                                    )));
                                }
                            },
                            on_zoom: {
                                let rect = *rect;
                                move |_| zoom_to(rect)
                            },
                            on_toggle_group: {
                                let store = store.clone();
                                let group = node.group.clone();
                                move |_| {
                                    if let Some(g) = &group {
                                        store.toggle_group_collapsed(g);
                                    }
                                }
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
                                MenuAction::ToggleGroup(group) => {
                                    store.toggle_group_collapsed(&group);
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
                        let viewport = viewport();
                        transform.set(ViewTransform::fit(
                            &layout.read().bounds,
                            viewport.width,
                            viewport.height,
                        ));
                    },
                    "⤢ fit"
                }
            }
            Minimap { doc, layout, transform, viewport: viewport_size }
        }
    }
}
