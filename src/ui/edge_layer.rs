//! SVG layer drawing bezier edges beneath the node cards.

use dioxus::prelude::*;

use crate::layout::Layout;
use crate::model::{EdgeKind, ElementStatus};

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
pub fn EdgeLayer(layout: Memo<Layout>) -> Element {
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
            for (i, e) in l.edges.iter().enumerate() {
                path {
                    key: "{e.from}-{e.to}-{i}",
                    class: "edge edge-{status_class(e.status)} edge-kind-{kind_class(e.kind)}",
                    d: "{e.path}",
                    marker_end: "url(#arrow-{status_class(e.status)})",
                }
                if let Some(label) = &e.label {
                    text {
                        key: "label-{e.from}-{e.to}-{i}",
                        class: "edge-label",
                        x: "{e.label_pos.x}",
                        y: "{e.label_pos.y}",
                        text_anchor: "middle",
                        "{label}"
                    }
                }
            }
        }
    }
}
