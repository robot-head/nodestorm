//! Collapsible activity feed: agent announces, user actions, system events.

use dioxus::prelude::*;

use crate::model::ActivityOrigin;
use crate::store::UiMeta;

const COLLAPSED_COUNT: usize = 1;
const EXPANDED_COUNT: usize = 10;

#[component]
pub fn ActivityFeed(meta: Signal<UiMeta>) -> Element {
    let mut expanded = use_signal(|| false);
    let m = meta.read();
    if m.activity.is_empty() {
        return rsx! {};
    }
    let count = if expanded() {
        EXPANDED_COUNT
    } else {
        COLLAPSED_COUNT
    };
    let entries: Vec<_> = m.activity.iter().rev().take(count).cloned().collect();
    let toggle_label = if expanded() { "▾" } else { "▸" };

    rsx! {
        div { class: "activity",
            div {
                class: "activity-head",
                onclick: move |_| expanded.toggle(),
                span { "{toggle_label} activity" }
            }
            for (i, entry) in entries.iter().enumerate() {
                div { class: "activity-entry", key: "{i}",
                    span {
                        class: match entry.origin {
                            ActivityOrigin::Agent => "activity-dot dot-agent",
                            ActivityOrigin::User => "activity-dot dot-user",
                            ActivityOrigin::System => "activity-dot dot-system",
                        },
                        "●"
                    }
                    span { class: "activity-text", "{entry.text}" }
                    span { class: "activity-time", {entry.at.format("%H:%M").to_string()} }
                }
            }
        }
    }
}
