//! One architecture-component card on the canvas.

use dioxus::prelude::*;

use crate::layout::Rect;
use crate::model::{BuildStatus, ElementStatus, Node, NodeKind};

/// Semantic-zoom tier derived from the canvas scale: far out cards collapse to
/// labeled chips, mid shows title + status, close shows the full card. Layered
/// on top of viewport culling to keep big graphs legible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ZoomTier {
    Far,
    Mid,
    Near,
}

impl ZoomTier {
    pub(crate) fn from_scale(scale: f64) -> Self {
        if scale < 0.4 {
            ZoomTier::Far
        } else if scale < 0.78 {
            ZoomTier::Mid
        } else {
            ZoomTier::Near
        }
    }

    fn class(self) -> &'static str {
        match self {
            ZoomTier::Far => "zoom-far",
            ZoomTier::Mid => "zoom-mid",
            ZoomTier::Near => "zoom-near",
        }
    }
}

/// Small glyph tracking implementation progress on the card.
pub(crate) fn build_glyph(build: BuildStatus) -> &'static str {
    match build {
        BuildStatus::Planned => "○",
        BuildStatus::Building => "◐",
        BuildStatus::Built => "●",
        BuildStatus::Verified => "✓",
    }
}

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

pub(crate) fn kind_label(kind: NodeKind) -> &'static str {
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
    on_toggle_group: EventHandler<MouseEvent>,
    /// Unanswered agent questions attached to this node (from the doc).
    #[props(default = 0)]
    open_questions: usize,
    /// Semantic-zoom tier: how much of the card to draw at the current scale.
    #[props(default = ZoomTier::Near)]
    zoom: ZoomTier,
) -> Element {
    let open = node.open_choice_count();
    let decided = node.choices.len() - open;
    let notes = node.notes.len();
    let status = status_class(node.status);
    let sel = if selected { " selected" } else { "" };
    let hl = if highlighted { " ripple" } else { "" };
    let hit = if search_hit { " search-hit" } else { "" };
    let dim = if search_dim { " search-dim" } else { "" };
    let bld = node
        .build
        .map(|b| format!(" build-{}", b.name()))
        .unwrap_or_default();
    let zoom_class = zoom.class();
    // Semantic zoom: far → label chip only; mid → title + status; near → all.
    let show_glyph = zoom != ZoomTier::Far;
    let show_meta = zoom != ZoomTier::Far;
    let show_body = zoom == ZoomTier::Near;

    rsx! {
        div {
            class: "node-card {zoom_class} status-{status}{sel}{hl}{hit}{dim}{bld}",
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
            if show_body {
                span {
                    class: "connect-handle",
                    title: "Drag onto another card to connect",
                    onmousedown: move |ev| {
                        ev.stop_propagation();
                        on_connect_start.call(ev);
                    },
                    "◉"
                }
            }
            div { class: "node-head",
                if show_glyph {
                    span { class: "node-glyph", "{kind_glyph(node.kind)}" }
                }
                span {
                    class: "node-label",
                    title: "{node.label}",
                    "{node.label}"
                }
            }
            if show_meta {
                div { class: "node-meta",
                    span { class: "node-kind", "{kind_label(node.kind)}" }
                    if node.status != ElementStatus::Existing {
                        span { class: "node-status-tag tag-{status}", "{status}" }
                    }
                    if let Some(build) = node.build {
                        span {
                            class: "node-build build-tag-{build.name()}",
                            title: "Implementation: {build.name()}",
                            "{build_glyph(build)} {build.name()}"
                        }
                    }
                    if let Some(agent) = &node.agent {
                        span {
                            class: "node-agent",
                            style: "color: {super::agent_color(agent)}; border-color: {super::agent_color(agent)};",
                            title: "Proposed by agent “{agent}”",
                            "◆ {agent}"
                        }
                    }
                    if let Some(group) = &node.group {
                        span {
                            class: "node-group",
                            title: "{group} — collapse this group into one card",
                            onclick: move |ev| {
                                ev.stop_propagation();
                                on_toggle_group.call(ev);
                            },
                            "{group}"
                        }
                    }
                }
            }
            if show_body && !node.description.is_empty() {
                p {
                    class: "node-desc",
                    title: "{node.description}",
                    "{node.description}"
                }
            }
            if show_body && (open > 0 || decided > 0 || notes > 0 || open_questions > 0) {
                div { class: "node-badges",
                    if open > 0 {
                        span { class: "badge badge-open", "⚑ {open}" }
                    }
                    if open_questions > 0 {
                        span { class: "badge badge-question", "? {open_questions}" }
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

#[cfg(test)]
mod tests {
    use super::ZoomTier;

    #[test]
    fn zoom_tier_thresholds() {
        assert_eq!(ZoomTier::from_scale(0.15), ZoomTier::Far);
        assert_eq!(ZoomTier::from_scale(0.39), ZoomTier::Far);
        assert_eq!(ZoomTier::from_scale(0.40), ZoomTier::Mid);
        assert_eq!(ZoomTier::from_scale(0.77), ZoomTier::Mid);
        assert_eq!(ZoomTier::from_scale(0.78), ZoomTier::Near);
        assert_eq!(ZoomTier::from_scale(2.5), ZoomTier::Near);
    }
}
