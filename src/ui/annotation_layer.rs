//! Renders freehand annotations (sticky notes, arrows, highlight regions) on
//! the canvas plane, plus the live preview while one is being drawn.
//!
//! Notes are editable in place and every annotation carries a delete handle.
//! The SVG overlay (arrows + preview) is pointer-transparent so it never
//! blocks card interaction; the HTML overlays capture their own events.

use dioxus::prelude::*;

use crate::model::{Annotation, AnnotationKind};

use super::app::use_store;

#[component]
pub fn AnnotationLayer(
    annotations: Vec<Annotation>,
    /// `(x0, y0, x1, y1, kind)` in plane coords while the user is drawing.
    preview: Option<(f64, f64, f64, f64, AnnotationKind)>,
) -> Element {
    let store = use_store();

    rsx! {
        // Arrow lines and the drag preview, in a pointer-transparent overlay.
        svg { class: "annotation-svg", width: "1", height: "1",
            defs {
                marker {
                    id: "anno-arrow",
                    class: "anno-arrow-head",
                    view_box: "0 0 10 10",
                    ref_x: "9",
                    ref_y: "5",
                    marker_width: "8",
                    marker_height: "8",
                    orient: "auto-start-reverse",
                    path { d: "M 0 1 L 9 5 L 0 9 z" }
                }
            }
            for a in annotations.iter().filter(|a| a.kind == AnnotationKind::Arrow) {
                line {
                    key: "arr-{a.id}",
                    class: "annotation-arrow",
                    x1: "{a.x}",
                    y1: "{a.y}",
                    x2: "{a.x + a.w}",
                    y2: "{a.y + a.h}",
                    marker_end: "url(#anno-arrow)",
                }
            }
            if let Some((x0, y0, x1, y1, kind)) = preview {
                match kind {
                    AnnotationKind::Arrow => rsx! {
                        line {
                            class: "annotation-arrow annotation-preview",
                            x1: "{x0}",
                            y1: "{y0}",
                            x2: "{x1}",
                            y2: "{y1}",
                            marker_end: "url(#anno-arrow)",
                        }
                    },
                    AnnotationKind::Region => rsx! {
                        rect {
                            class: "annotation-region-preview",
                            x: "{x0.min(x1)}",
                            y: "{y0.min(y1)}",
                            width: "{(x1 - x0).abs()}",
                            height: "{(y1 - y0).abs()}",
                        }
                    },
                    AnnotationKind::Note => rsx! {},
                }
            }
        }

        // Highlight regions (behind notes).
        for a in annotations.iter().filter(|a| a.kind == AnnotationKind::Region) {
            {
                let store = store.clone();
                let id = a.id.clone();
                rsx! {
                    div {
                        key: "reg-{a.id}",
                        class: "annotation-region",
                        style: "left: {a.x}px; top: {a.y}px; width: {a.w}px; height: {a.h}px;",
                        onmousedown: move |ev| ev.stop_propagation(),
                        if !a.text.is_empty() {
                            span { class: "annotation-region-label", "{a.text}" }
                        }
                        button {
                            class: "annotation-del",
                            title: "Delete annotation",
                            onmousedown: move |ev| ev.stop_propagation(),
                            onclick: {
                                let store = store.clone();
                                let id = id.clone();
                                move |ev: MouseEvent| {
                                    ev.stop_propagation();
                                    if let Err(err) = store.delete_annotation(&id) {
                                        tracing::warn!(%err, "delete annotation failed");
                                    }
                                }
                            },
                            "✕"
                        }
                    }
                }
            }
        }

        // Sticky notes.
        for a in annotations.iter().filter(|a| a.kind == AnnotationKind::Note) {
            {
                let store = store.clone();
                let a = a.clone();
                let id = a.id.clone();
                rsx! {
                    div {
                        key: "note-{a.id}",
                        class: "annotation-note",
                        style: "left: {a.x}px; top: {a.y}px;",
                        onmousedown: move |ev| ev.stop_propagation(),
                        ondoubleclick: move |ev| ev.stop_propagation(),
                        textarea {
                            class: "annotation-note-input",
                            placeholder: "note…",
                            value: "{a.text}",
                            onchange: {
                                let store = store.clone();
                                let a = a.clone();
                                move |ev: FormEvent| {
                                    if let Err(err) = store.edit_annotation(
                                        &a.id, a.x, a.y, a.w, a.h, ev.value(),
                                    ) {
                                        tracing::warn!(%err, "edit annotation failed");
                                    }
                                }
                            },
                        }
                        button {
                            class: "annotation-del",
                            title: "Delete note",
                            onmousedown: move |ev| ev.stop_propagation(),
                            onclick: {
                                let store = store.clone();
                                let id = id.clone();
                                move |ev: MouseEvent| {
                                    ev.stop_propagation();
                                    if let Err(err) = store.delete_annotation(&id) {
                                        tracing::warn!(%err, "delete annotation failed");
                                    }
                                }
                            },
                            "✕"
                        }
                    }
                }
            }
        }

        // Delete handles for arrows (placed at the arrowhead).
        for a in annotations.iter().filter(|a| a.kind == AnnotationKind::Arrow) {
            {
                let store = store.clone();
                let id = a.id.clone();
                rsx! {
                    button {
                        key: "arrdel-{a.id}",
                        class: "annotation-del annotation-arrow-del",
                        style: "left: {a.x + a.w}px; top: {a.y + a.h}px;",
                        title: "Delete arrow",
                        onmousedown: move |ev| ev.stop_propagation(),
                        onclick: {
                            let store = store.clone();
                            let id = id.clone();
                            move |ev: MouseEvent| {
                                ev.stop_propagation();
                                if let Err(err) = store.delete_annotation(&id) {
                                    tracing::warn!(%err, "delete annotation failed");
                                }
                            }
                        },
                        "✕"
                    }
                }
            }
        }
    }
}
