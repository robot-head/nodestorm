//! Chronological session-log panel: every decision-log event with its
//! timestamp, using the same line formatter as the exported record's
//! `## Session log` section.

use dioxus::prelude::*;

use crate::model::SessionDoc;
use crate::store::UiMeta;

use super::app::use_store;

#[component]
pub fn Timeline(
    doc: Signal<SessionDoc>,
    meta: Signal<UiMeta>,
    on_close: EventHandler<()>,
) -> Element {
    // Reading `meta` subscribes this component to every log append (the
    // undelivered count changes), so new events appear live.
    let _ = meta.read();
    let store = use_store();
    let d = doc.read();
    let lines: Vec<(u64, String, String)> = store.read(|s| {
        s.decision_log
            .iter()
            .map(|e| {
                (
                    e.seq,
                    e.at.format("%H:%M").to_string(),
                    crate::export::describe_event(&d, e),
                )
            })
            .collect()
    });

    rsx! {
        aside { class: "panel timeline",
            div { class: "panel-head",
                h2 { "Session timeline" }
                button {
                    class: "ctl-btn",
                    title: "Close",
                    onclick: move |_| on_close.call(()),
                    "✕"
                }
            }
            if lines.is_empty() {
                p { class: "panel-desc",
                    "Nothing yet — decisions, notes, and edits will appear here."
                }
            }
            for (seq, at, text) in lines {
                div { class: "timeline-row", key: "{seq}",
                    span { class: "timeline-time", "{at}" }
                    span { class: "timeline-text", "{text}" }
                }
            }
        }
    }
}
