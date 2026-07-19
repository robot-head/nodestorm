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
use crate::model::{AnnotationKind, EdgeKind, NodeId, NodeKind, Point, SessionDoc};

use super::ViewTransform;
use super::annotation_layer::AnnotationLayer;
use super::app::use_store;
use super::cluster_card::ClusterCard;
use super::context_menu::{ContextMenu, MenuAction, MenuTarget};
use super::edge_layer::EdgeLayer;
use super::minimap::Minimap;
use super::node_card::{NodeCard, ZoomTier};

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

/// Short toggle label for an edge kind (the edge-kind layer buttons).
fn edge_kind_label(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::DependsOn => "depends",
        EdgeKind::DataFlow => "data",
        EdgeKind::Contains => "contains",
        EdgeKind::Other => "other",
    }
}

/// Bounding rects (padded) of each expanded group's member cards, in
/// first-appearance order. Collapsed groups are drawn as cluster cards
/// instead, so their members are absent from `layout.rects` and skipped here.
fn group_outline_rects(doc: &SessionDoc, layout: &Layout) -> Vec<(String, crate::layout::Rect)> {
    use crate::layout::Rect;
    const PAD: f64 = 18.0;
    let mut order: Vec<String> = Vec::new();
    let mut bounds: std::collections::HashMap<String, Rect> = std::collections::HashMap::new();
    for node in &doc.nodes {
        let (Some(group), Some(r)) = (node.group.as_deref(), layout.rects.get(&node.id)) else {
            continue;
        };
        match bounds.get_mut(group) {
            Some(b) => {
                let (right, bottom) = (b.x + b.w, b.y + b.h);
                b.x = b.x.min(r.x);
                b.y = b.y.min(r.y);
                b.w = right.max(r.x + r.w) - b.x;
                b.h = bottom.max(r.y + r.h) - b.y;
            }
            None => {
                order.push(group.to_owned());
                bounds.insert(group.to_owned(), *r);
            }
        }
    }
    order
        .into_iter()
        .map(|g| {
            let mut r = bounds[&g];
            r.x -= PAD;
            r.y -= PAD;
            r.w += 2.0 * PAD;
            r.h += 2.0 * PAD;
            (g, r)
        })
        .collect()
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
    // Edge-kind layers the user has toggled off (view state, not persisted).
    let mut hidden_kinds: Signal<std::collections::HashSet<EdgeKind>> =
        use_signal(std::collections::HashSet::new);
    // Freehand-annotation drawing: the active tool and the in-progress stroke
    // `(x0, y0, x1, y1)` in plane coords.
    let mut annotate_tool: Signal<Option<AnnotationKind>> = use_signal(|| None);
    let mut draw: Signal<Option<(f64, f64, f64, f64)>> = use_signal(|| None);

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
    // Semantic-zoom tier for this frame; also gates the group outlines that
    // dominate when zoomed out.
    let tier = ZoomTier::from_scale(t.scale);
    let group_outlines: Vec<(String, crate::layout::Rect)> = if tier == ZoomTier::Near {
        Vec::new()
    } else {
        group_outline_rects(&d, &l)
    };
    // Edge-kind layers: drop toggled-off kinds from the visible edge set, and
    // gather the kinds present so the toggles only offer what exists.
    let hidden = hidden_kinds.read().clone();
    let visible_edges: Vec<usize> = vis
        .edges
        .iter()
        .copied()
        .filter(|&i| !hidden.contains(&l.edges[i].kind))
        .collect();
    let kinds_present: Vec<EdgeKind> = {
        let mut seen: Vec<EdgeKind> = Vec::new();
        for e in &d.edges {
            if !seen.contains(&e.kind) {
                seen.push(e.kind);
            }
        }
        seen
    };
    let panning = matches!(gesture.read().gesture, Some(Gesture::Pan { .. }));
    let viewport_class = match (panning, connect_from().is_some(), annotate_tool().is_some()) {
        (true, _, _) => "canvas-viewport panning",
        (false, _, true) => "canvas-viewport annotating",
        (false, true, false) => "canvas-viewport connecting",
        (false, false, false) => "canvas-viewport",
    };
    // The live annotation preview passed to the layer.
    let anno_preview = draw().map(|(x0, y0, x1, y1)| {
        (
            x0,
            y0,
            x1,
            y1,
            annotate_tool().unwrap_or(AnnotationKind::Note),
        )
    });

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
                            // First non-empty wins: annotate tool → connect →
                            // menu → search → selection.
                            if annotate_tool().is_some() {
                                annotate_tool.set(None);
                                draw.set(None);
                            } else if connect_from().is_some() {
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
                // An armed annotation tool turns a background press into a draw.
                if annotate_tool().is_some() {
                    let (px, py) = to_plane(c.x, c.y);
                    draw.set(Some((px, py, px, py)));
                    return;
                }
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
                    // While drawing an annotation, track the cursor as the end.
                    if draw().is_some() {
                        let (px, py) = to_plane(c.x, c.y);
                        draw.with_mut(|d| {
                            if let Some(d) = d {
                                d.2 = px;
                                d.3 = py;
                            }
                        });
                        return;
                    }
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
            onmouseup: {
                let store = store.clone();
                move |_| {
                if let Some((x0, y0, x1, y1)) = draw() {
                    let kind = annotate_tool().unwrap_or(AnnotationKind::Note);
                    match kind {
                        AnnotationKind::Note => {
                            store.add_annotation(kind, x0, y0, 0.0, 0.0, String::new());
                        }
                        AnnotationKind::Arrow => {
                            if (x1 - x0).hypot(y1 - y0) > 8.0 {
                                store.add_annotation(kind, x0, y0, x1 - x0, y1 - y0, String::new());
                            }
                        }
                        AnnotationKind::Region => {
                            let (rw, rh) = ((x1 - x0).abs(), (y1 - y0).abs());
                            if rw > 8.0 && rh > 8.0 {
                                store.add_annotation(
                                    kind, x0.min(x1), y0.min(y1), rw, rh, String::new(),
                                );
                            }
                        }
                    }
                    draw.set(None);
                    return;
                }
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
                }
            },
            onmouseleave: move |_| {
                ghost_to.set(None);
                gesture.set(GestureState::default());
                // Abandon an in-progress annotation stroke released off-canvas,
                // so re-entering doesn't drop a stray note/arrow/region.
                draw.set(None);
            },
            ondoubleclick: {
                let store = store.clone();
                move |ev: MouseEvent| {
                    // With an annotation tool armed, double-click is part of
                    // drawing, not a new component.
                    if annotate_tool().is_some() {
                        return;
                    }
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
                for lane in l.lanes.iter() {
                    div {
                        key: "lane-{lane.label}",
                        class: "swimlane",
                        style: "left: {lane.rect.x}px; top: {lane.rect.y}px; width: {lane.rect.w}px; height: {lane.rect.h}px;",
                        span { class: "swimlane-label", "{lane.label}" }
                    }
                }
                for (group, r) in group_outlines.iter() {
                    div {
                        key: "outline-{group}",
                        class: "group-outline",
                        style: "left: {r.x}px; top: {r.y}px; width: {r.w}px; height: {r.h}px;",
                        span { class: "group-outline-label", "{group}" }
                    }
                }
                EdgeLayer {
                    layout,
                    visible: visible_edges.clone(),
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
                            zoom: tier,
                            open_questions: d.questions.iter().filter(|q| {
                                q.node_id.as_ref() == Some(&node.id) && !q.is_answered()
                            }).count(),
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
                AnnotationLayer { annotations: d.annotations.clone(), preview: anno_preview }
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
                div { class: "annotate-tools", title: "Draw annotations (Esc to cancel)",
                    for (kind, glyph, label) in [
                        (AnnotationKind::Note, "🗒", "note"),
                        (AnnotationKind::Arrow, "↗", "arrow"),
                        (AnnotationKind::Region, "▢", "region"),
                    ] {
                        {
                            let active = annotate_tool() == Some(kind);
                            rsx! {
                                button {
                                    key: "{label}",
                                    class: if active { "annotate-btn active" } else { "annotate-btn" },
                                    title: "Draw a {label} annotation",
                                    onclick: move |_| {
                                        annotate_tool.set(if active { None } else { Some(kind) });
                                        draw.set(None);
                                    },
                                    "{glyph}"
                                }
                            }
                        }
                    }
                }
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
                if kinds_present.len() > 1 {
                    div { class: "layer-toggles", title: "Show/hide edge kinds",
                        for kind in kinds_present.iter().copied() {
                            {
                                let active = !hidden_kinds.read().contains(&kind);
                                rsx! {
                                    button {
                                        key: "{kind:?}",
                                        class: if active { "layer-btn active" } else { "layer-btn" },
                                        title: if active { "Hide {edge_kind_label(kind)} edges" } else { "Show {edge_kind_label(kind)} edges" },
                                        onclick: move |_| {
                                            hidden_kinds.with_mut(|h| {
                                                if !h.remove(&kind) {
                                                    h.insert(kind);
                                                }
                                            });
                                        },
                                        "{edge_kind_label(kind)}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Minimap { doc, layout, transform, viewport: viewport_size }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::group_outline_rects;

    #[test]
    fn group_outlines_bound_each_expanded_group() {
        // big_doc(24) lays out two groups (cluster-0, cluster-1), all expanded.
        let doc = crate::demo::big_doc(24);
        let layout = crate::layout::compute(&doc);
        let outlines = group_outline_rects(&doc, &layout);
        assert_eq!(outlines.len(), 2, "one outline per expanded group");
        for (group, rect) in &outlines {
            // Every member card sits inside its group's outline.
            for node in doc
                .nodes
                .iter()
                .filter(|n| n.group.as_deref() == Some(group.as_str()))
            {
                let m = layout.rects[&node.id];
                assert!(
                    rect.x <= m.x
                        && rect.y <= m.y
                        && rect.x + rect.w >= m.x + m.w
                        && rect.y + rect.h >= m.y + m.h,
                    "member {} escapes outline {group}",
                    node.id
                );
            }
        }
    }
}
