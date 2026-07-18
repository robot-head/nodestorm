use dioxus::prelude::*;

use crate::model::{DecisionKind, Node, NodeId, SessionDoc};
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

#[derive(Debug, Clone, PartialEq)]
enum QueueEditReplacement {
    Node(Node),
    Comment(String),
}

fn queued_edit_replacement(change: &QueuedChange) -> Option<QueueEditReplacement> {
    match &change.event.kind {
        DecisionKind::NodeAdded { node } => Some(QueueEditReplacement::Node(node.clone())),
        DecisionKind::FlushRequested {
            comment: Some(comment),
        } => Some(QueueEditReplacement::Comment(comment.clone())),
        _ => None,
    }
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
    let composer = use_context::<super::MessageComposer>();
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
                    let replacement = queued_edit_replacement(&change);
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
                                        let mut composer_comment = composer.comment;
                                        let mut composer_open = composer.open;
                                        let replacement = replacement.clone();
                                        move |_| {
                                            let target = if blocked {
                                                store.remove_blocked_change(seq)
                                            } else {
                                                store.remove_queued_change(seq)
                                            };
                                            match target {
                                                Ok(target) => {
                                                    match replacement.clone() {
                                                        Some(QueueEditReplacement::Node(node)) => {
                                                            let label = node.label.clone();
                                                            match store.add_user_node(
                                                                label.clone(),
                                                                node.kind,
                                                                node.position,
                                                            ) {
                                                                Ok(id) => {
                                                                    if !node.description.is_empty()
                                                                        && let Err(err) = store.edit_node(
                                                                            &id,
                                                                            label,
                                                                            node.kind,
                                                                            node.description,
                                                                        )
                                                                    {
                                                                        tracing::warn!(%err, "restore queued node details failed");
                                                                    }
                                                                    selected.set(Some(id));
                                                                }
                                                                Err(err) => tracing::warn!(%err, "restore queued node failed"),
                                                            }
                                                        }
                                                        Some(QueueEditReplacement::Comment(comment)) => {
                                                            composer_comment.set(comment);
                                                            composer_open.set(true);
                                                        }
                                                        None => selected.set(target.node_id),
                                                    }
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

    use crate::model::{
        DecisionEvent, DecisionKind, ElementStatus, Node, NodeId, NodeKind, Origin, SessionDoc,
    };
    use crate::store::QueuedChange;

    use super::{QueueEditReplacement, queued_change_label, queued_edit_replacement};

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

    #[test]
    fn editing_special_queued_events_chooses_a_replacement_surface() {
        let node = Node {
            id: NodeId::from("widget"),
            label: "Widget".into(),
            kind: NodeKind::Component,
            description: "needs tuning".into(),
            status: ElementStatus::Proposed,
            group: None,
            choices: vec![],
            notes: vec![],
            position: None,
            origin: Origin::User,
        };
        let added = QueuedChange {
            event: DecisionEvent {
                seq: 1,
                at: Utc::now(),
                kind: DecisionKind::NodeAdded { node },
            },
            blocked_reason: None,
        };
        let comment = QueuedChange {
            event: DecisionEvent {
                seq: 2,
                at: Utc::now(),
                kind: DecisionKind::FlushRequested {
                    comment: Some("please keep the cache local".into()),
                },
            },
            blocked_reason: None,
        };

        assert!(matches!(
            queued_edit_replacement(&added),
            Some(QueueEditReplacement::Node(node)) if node.label == "Widget" && node.description == "needs tuning"
        ));
        assert_eq!(
            queued_edit_replacement(&comment),
            Some(QueueEditReplacement::Comment(
                "please keep the cache local".into()
            ))
        );
    }
}
