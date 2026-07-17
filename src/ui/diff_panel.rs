//! Right-side panel showing a structured diff between the active session
//! and another one (same text `diff_sessions` returns over MCP).

use dioxus::prelude::*;

#[component]
pub fn DiffPanel(text: String, on_close: EventHandler<()>) -> Element {
    rsx! {
        aside { class: "panel timeline",
            div { class: "panel-head",
                h2 { "Session diff" }
                button {
                    class: "ctl-btn",
                    title: "Close",
                    onclick: move |_| on_close.call(()),
                    "✕"
                }
            }
            pre { class: "diff-text", "{text}" }
        }
    }
}
