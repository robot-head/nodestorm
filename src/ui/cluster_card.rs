//! One collapsed group rendered as a single dashed card. Double-click,
//! right-click, or the ⊞ button expands it again.

use dioxus::prelude::*;

use crate::layout::Rect;

#[component]
pub fn ClusterCard(
    group: String,
    rect: Rect,
    member_count: usize,
    on_expand: EventHandler<()>,
) -> Element {
    rsx! {
        div {
            class: "node-card cluster-card status-existing",
            style: "left: {rect.x}px; top: {rect.y}px; width: {rect.w}px;",
            onmousedown: move |ev| ev.stop_propagation(),
            onclick: move |ev| ev.stop_propagation(),
            ondoubleclick: move |ev| {
                ev.stop_propagation();
                on_expand.call(());
            },
            oncontextmenu: move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                on_expand.call(());
            },
            div { class: "node-head",
                span { class: "node-glyph", "▣" }
                span { class: "node-label", title: "{group}", "{group}" }
            }
            div { class: "node-meta",
                span { class: "node-kind", "{member_count} components" }
            }
            button {
                class: "btn cluster-expand",
                title: "Expand this group back into its components",
                onclick: move |ev| {
                    ev.stop_propagation();
                    on_expand.call(());
                },
                "⊞ expand"
            }
        }
    }
}
