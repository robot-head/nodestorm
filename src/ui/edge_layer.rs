//! SVG layer drawing bezier edges beneath the node cards.

use dioxus::prelude::*;

use crate::layout::Layout;
use crate::model::{EdgeKind, ElementStatus};

const MAX_EDGE_LABEL_CHARS: usize = 32;

fn edge_label_preview(label: &str) -> String {
    if label.chars().count() <= MAX_EDGE_LABEL_CHARS {
        return label.to_owned();
    }
    label
        .chars()
        .take(MAX_EDGE_LABEL_CHARS - 1)
        .chain(std::iter::once('…'))
        .collect()
}

fn status_class(status: ElementStatus) -> &'static str {
    match status {
        ElementStatus::Existing => "existing",
        ElementStatus::Proposed => "proposed",
        ElementStatus::Modified => "modified",
        ElementStatus::Affected => "affected",
        ElementStatus::Removed => "removed",
    }
}

fn kind_class(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::DependsOn => "depends-on",
        EdgeKind::DataFlow => "data-flow",
        EdgeKind::Contains => "contains",
        EdgeKind::Other => "other",
    }
}

#[component]
pub fn EdgeLayer(
    layout: Memo<Layout>,
    /// Indices into `layout.edges` that survived viewport culling.
    visible: Vec<usize>,
    /// Rubber-band line while a connect drag is live: (from-center, cursor),
    /// both in plane coordinates.
    ghost: Option<((f64, f64), (f64, f64))>,
    /// Right-click on an edge path: `(from, to, kind, client_x, client_y)`.
    on_edge_context: EventHandler<(
        crate::model::NodeId,
        crate::model::NodeId,
        EdgeKind,
        f64,
        f64,
    )>,
) -> Element {
    let l = layout.read();
    rsx! {
        svg { class: "edge-layer", width: "1", height: "1",
            defs {
                for status in ["existing", "proposed", "modified", "affected", "removed"] {
                    marker {
                        id: "arrow-{status}",
                        class: "arrow arrow-{status}",
                        view_box: "0 0 10 10",
                        ref_x: "9",
                        ref_y: "5",
                        marker_width: "7",
                        marker_height: "7",
                        orient: "auto-start-reverse",
                        path { d: "M 0 1 L 9 5 L 0 9 z" }
                    }
                }
            }
            for (i, e) in visible.iter().map(|&i| (i, &l.edges[i])) {
                path {
                    key: "{e.from}-{e.to}-{i}",
                    class: if e.bundle_count > 1 {
                        "edge edge-bundled edge-{status_class(e.status)} edge-kind-{kind_class(e.kind)}"
                    } else {
                        "edge edge-{status_class(e.status)} edge-kind-{kind_class(e.kind)}"
                    },
                    style: if e.bundle_count > 1 {
                        format!(
                            "stroke-width: {:.1}px;",
                            1.5 + (e.bundle_count as f64).ln() * 1.4
                        )
                    } else {
                        String::new()
                    },
                    d: "{e.path}",
                    marker_end: "url(#arrow-{status_class(e.status)})",
                    oncontextmenu: {
                        let from = e.from.clone();
                        let to = e.to.clone();
                        let kind = e.kind;
                        move |ev: MouseEvent| {
                            ev.prevent_default();
                            ev.stop_propagation();
                            let c = ev.client_coordinates();
                            on_edge_context.call((from.clone(), to.clone(), kind, c.x, c.y));
                        }
                    },
                }
                if let Some(label) = &e.label {
                    text {
                        key: "label-{e.from}-{e.to}-{i}",
                        class: "edge-label",
                        x: "{e.label_pos.x}",
                        y: "{e.label_pos.y}",
                        text_anchor: "middle",
                        title { "{label}" }
                        {edge_label_preview(label)}
                    }
                }
            }
            if let Some(((x1, y1), (x2, y2))) = ghost {
                line {
                    class: "ghost-edge",
                    x1: "{x1}",
                    y1: "{y1}",
                    x2: "{x2}",
                    y2: "{y2}",
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yare::parameterized;

    #[test]
    fn short_edge_labels_are_unchanged() {
        assert2::assert!((edge_label_preview("read/write")) == ("read/write"));
    }

    #[test]
    fn long_edge_labels_are_unicode_safe_and_32_chars() {
        let label = format!("{}éé", "a".repeat(31));
        let preview = edge_label_preview(&label);

        assert2::assert!((preview.chars().count()) == (32));
        assert2::assert!((preview) == (format!("{}…", "a".repeat(31))));
    }

    #[parameterized(
        existing = { ElementStatus::Existing, "existing" },
        proposed = { ElementStatus::Proposed, "proposed" },
        modified = { ElementStatus::Modified, "modified" },
        affected = { ElementStatus::Affected, "affected" },
        removed = { ElementStatus::Removed, "removed" },
    )]
    fn edge_status_classes_are_exact(status: ElementStatus, expected: &str) {
        assert2::assert!(status_class(status) == expected);
    }

    #[parameterized(
        depends_on = { EdgeKind::DependsOn, "depends-on" },
        data_flow = { EdgeKind::DataFlow, "data-flow" },
        contains = { EdgeKind::Contains, "contains" },
        other = { EdgeKind::Other, "other" },
    )]
    fn edge_kind_classes_are_exact(kind: EdgeKind, expected: &str) {
        assert2::assert!(kind_class(kind) == expected);
    }
}
