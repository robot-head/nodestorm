//! Collapsible activity feed: agent announces, user actions, system events.

use dioxus::prelude::*;

use crate::model::ActivityOrigin;
use crate::store::UiMeta;

const COLLAPSED_COUNT: usize = 1;

fn entry_count(total: usize, expanded: bool) -> usize {
    if expanded {
        total
    } else {
        total.min(COLLAPSED_COUNT)
    }
}

#[component]
pub fn ActivityFeed(meta: Signal<UiMeta>) -> Element {
    let mut expanded = use_signal(|| false);
    let m = meta.read();
    if m.activity.is_empty() {
        return rsx! {};
    }
    let count = entry_count(m.activity.len(), expanded());
    let entries: Vec<_> = m.activity.iter().rev().take(count).cloned().collect();
    let toggle_label = if expanded() { "▾" } else { "▸" };

    rsx! {
        div { class: if expanded() { "activity expanded" } else { "activity" },
            // Anywhere on the collapsed box expands it, not just the tiny head.
            onclick: move |_| {
                if !expanded() {
                    expanded.set(true);
                }
            },
            div {
                class: "activity-head",
                onclick: move |ev| {
                    ev.stop_propagation();
                    expanded.toggle();
                },
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
                        style: entry.agent.as_deref().map(|a| {
                            format!("color: {}", super::agent_color(a))
                        }),
                        "●"
                    }
                    if let Some(agent) = &entry.agent {
                        span {
                            class: "activity-agent",
                            style: "color: {super::agent_color(agent)};",
                            "{agent}"
                        }
                    }
                    span { class: "activity-text", "{entry.text}" }
                    span { class: "activity-time", {entry.at.format("%H:%M").to_string()} }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_feed_includes_every_retained_entry() {
        assert2::assert!((entry_count(200, true)) == (200));
        assert2::assert!((entry_count(200, false)) == (1));
        assert2::assert!((entry_count(0, false)) == (0));
    }
}
