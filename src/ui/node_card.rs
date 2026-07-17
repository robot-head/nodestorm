//! One architecture-component card on the canvas.

use dioxus::prelude::*;

use crate::layout::Rect;
use crate::model::{ElementStatus, Node, NodeKind};

fn kind_glyph(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Service => "⬢",
        NodeKind::Module => "▣",
        NodeKind::Component => "◆",
        NodeKind::DataStore => "🗃",
        NodeKind::Queue => "☰",
        NodeKind::Ui => "▢",
        NodeKind::External => "↗",
        NodeKind::Other => "●",
    }
}

fn kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Service => "service",
        NodeKind::Module => "module",
        NodeKind::Component => "component",
        NodeKind::DataStore => "data store",
        NodeKind::Queue => "queue",
        NodeKind::Ui => "ui",
        NodeKind::External => "external",
        NodeKind::Other => "other",
    }
}

pub(crate) fn status_class(status: ElementStatus) -> &'static str {
    match status {
        ElementStatus::Existing => "existing",
        ElementStatus::Proposed => "proposed",
        ElementStatus::Modified => "modified",
        ElementStatus::Affected => "affected",
        ElementStatus::Removed => "removed",
    }
}

#[component]
pub fn NodeCard(
    node: Node,
    rect: Rect,
    selected: bool,
    highlighted: bool,
    on_select: EventHandler<MouseEvent>,
    on_drag_start: EventHandler<MouseEvent>,
    search_hit: bool,
    search_dim: bool,
    on_connect_start: EventHandler<MouseEvent>,
    on_connect_drop: EventHandler<MouseEvent>,
    on_context: EventHandler<MouseEvent>,
    on_zoom: EventHandler<MouseEvent>,
) -> Element {
    let open = node.open_choice_count();
    let decided = node.choices.len() - open;
    let notes = node.notes.len();
    let status = status_class(node.status);
    let sel = if selected { " selected" } else { "" };
    let hl = if highlighted { " ripple" } else { "" };
    let hit = if search_hit { " search-hit" } else { "" };
    let dim = if search_dim { " search-dim" } else { "" };

    rsx! {
        div {
            class: "node-card status-{status}{sel}{hl}{hit}{dim}",
            style: "left: {rect.x}px; top: {rect.y}px; width: {rect.w}px;",
            onclick: move |ev| {
                ev.stop_propagation();
                on_select.call(ev);
            },
            onmousedown: move |ev| {
                ev.stop_propagation();
                on_drag_start.call(ev);
            },
            // No stop_propagation: a node drag also ends here and the
            // viewport's mouseup must still clear the gesture.
            onmouseup: move |ev| on_connect_drop.call(ev),
            ondoubleclick: move |ev| {
                ev.stop_propagation();
                on_zoom.call(ev);
            },
            oncontextmenu: move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                on_context.call(ev);
            },
            span {
                class: "connect-handle",
                title: "Drag onto another card to connect",
                onmousedown: move |ev| {
                    ev.stop_propagation();
                    on_connect_start.call(ev);
                },
                "◉"
            }
            div { class: "node-head",
                span { class: "node-glyph", "{kind_glyph(node.kind)}" }
                span { class: "node-label", "{node.label}" }
            }
            div { class: "node-meta",
                span { class: "node-kind", "{kind_label(node.kind)}" }
                if node.status != ElementStatus::Existing {
                    span { class: "node-status-tag tag-{status}", "{status}" }
                }
                if let Some(group) = &node.group {
                    span { class: "node-group", "{group}" }
                }
            }
            if !node.description.is_empty() {
                p { class: "node-desc", "{node.description}" }
            }
            if open > 0 || decided > 0 || notes > 0 {
                div { class: "node-badges",
                    if open > 0 {
                        span { class: "badge badge-open", "⚑ {open}" }
                    }
                    if decided > 0 {
                        span { class: "badge badge-decided", "✓ {decided}" }
                    }
                    if notes > 0 {
                        span { class: "badge badge-notes", "✎ {notes}" }
                    }
                }
            }
        }
    }
}
