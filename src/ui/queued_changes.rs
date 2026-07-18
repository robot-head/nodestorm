use dioxus::prelude::*;

use crate::model::{NodeId, SessionDoc};
use crate::store::{QueuedChange, UiMeta};

use super::app::use_store;

fn queued_change_label(doc: &SessionDoc, change: &QueuedChange) -> String {
    let label = crate::export::describe_event(doc, &change.event);
    change
        .blocked_reason
        .as_ref()
        .map_or(label.clone(), |reason| {
            format!("{label} — blocked: {reason}")
        })
}

#[component]
pub fn QueuedChangesPanel(
    doc: Signal<SessionDoc>,
    meta: Signal<UiMeta>,
    on_close: EventHandler<()>,
    selected: Signal<Option<NodeId>>,
) -> Element {
    // `meta` changes with every queued mutation, including blocked replay
    // entries, so the rows remain live while this panel is open.
    let _ = meta.read();
    let store = use_store();
    let d = doc.read();
    let changes = store.queued_changes();

    rsx! {
        aside { class: "panel queued-changes",
            div { class: "panel-head",
                h2 { "Queued changes" }
                button {
                    class: "ctl-btn",
                    title: "Close",
                    onclick: move |_| on_close.call(()),
                    "✕"
                }
            }
            if changes.is_empty() {
                p { class: "panel-desc", "Nothing is queued for the agent." }
            }
            for change in changes {
                {
                    let seq = change.event.seq;
                    let blocked = change.blocked_reason.is_some();
                    let row_class = if blocked { "queue-row queue-blocked" } else { "queue-row" };
                    let at = change.event.at.format("%H:%M").to_string();
                    let label = queued_change_label(&d, &change);
                    let store = store.clone();
                    let on_close = on_close.clone();
                    rsx! {
                        div { class: "{row_class}", key: "{seq}-{blocked}",
                            span { class: "timeline-time", "{at}" }
                            span { class: "timeline-text", "{label}" }
                            div { class: "queue-actions",
                                button {
                                    class: "ctl-btn",
                                    title: "Edit queued change",
                                    onclick: {
                                        let store = store.clone();
                                        let mut selected = selected;
                                        let on_close = on_close.clone();
                                        move |_| {
                                            let target = if blocked {
                                                store.remove_blocked_change(seq)
                                            } else {
                                                store.remove_queued_change(seq)
                                            };
                                            match target {
                                                Ok(target) => {
                                                    selected.set(target.node_id);
                                                    on_close.call(());
                                                }
                                                Err(err) => tracing::warn!(%err, "edit queued change failed"),
                                            }
                                        }
                                    },
                                    "Edit"
                                }
                                button {
                                    class: "ctl-btn",
                                    title: "Remove queued change",
                                    onclick: {
                                        let store = store.clone();
                                        move |_| {
                                            let result = if blocked {
                                                store.remove_blocked_change(seq)
                                            } else {
                                                store.remove_queued_change(seq)
                                            };
                                            if let Err(err) = result {
                                                tracing::warn!(%err, "remove queued change failed");
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
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::model::{DecisionEvent, DecisionKind, NodeId, NodeKind, SessionDoc};
    use crate::store::QueuedChange;

    use super::queued_change_label;

    #[test]
    fn blocked_changes_explain_why_they_will_not_send() {
        let doc = SessionDoc::default();
        let change = QueuedChange {
            event: DecisionEvent {
                seq: 1,
                at: Utc::now(),
                kind: DecisionKind::NodeEdited {
                    node_id: NodeId::from("widget"),
                    label: "Widget".into(),
                    node_kind: NodeKind::Component,
                    description: String::new(),
                },
            },
            blocked_reason: Some("node widget no longer exists".into()),
        };

        assert_eq!(
            queued_change_label(&doc, &change),
            "edited “Widget” — blocked: node widget no longer exists"
        );
    }
}
