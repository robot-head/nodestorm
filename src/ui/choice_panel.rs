//! Right-hand panel for the selected node: description, open choices with
//! option cards (pros/cons, recommended), and the note composer.
//!
//! Hovering an option highlights the nodes it `affects` on the canvas — the
//! ripple preview. Every option click is recorded into a per-choice
//! "considered" trail that rides along with the final decision, so the agent
//! sees hesitation, not just the outcome.

use std::collections::HashMap;

use dioxus::prelude::*;

use crate::model::{
    Choice, ChoiceId, ChoiceStatus, Edge, EdgeKind, Node, NodeId, NodeKind, OptionId, Origin,
    SessionDoc,
};

use super::app::use_store;

/// `<select>` values ↔ [`NodeKind`], in display order.
const KIND_VALUES: [(&str, NodeKind); 8] = [
    ("service", NodeKind::Service),
    ("module", NodeKind::Module),
    ("component", NodeKind::Component),
    ("data_store", NodeKind::DataStore),
    ("queue", NodeKind::Queue),
    ("ui", NodeKind::Ui),
    ("external", NodeKind::External),
    ("other", NodeKind::Other),
];

fn kind_value(kind: NodeKind) -> &'static str {
    KIND_VALUES
        .iter()
        .find(|(_, k)| *k == kind)
        .map(|(v, _)| *v)
        .unwrap_or("component")
}

fn kind_from_value(value: &str) -> NodeKind {
    KIND_VALUES
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, k)| *k)
        .unwrap_or(NodeKind::Component)
}

fn edge_kind_phrase(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::DependsOn => "depends on",
        EdgeKind::DataFlow => "data flow",
        EdgeKind::Contains => "contains",
        EdgeKind::Other => "relates to",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConnectionDisplay {
    direction: &'static str,
    endpoint: String,
    kind: &'static str,
    status: &'static str,
    label: Option<String>,
}

fn connection_display(selected: &NodeId, edge: &Edge, doc: &SessionDoc) -> ConnectionDisplay {
    let (direction, endpoint_id) = if &edge.from == selected {
        ("Outgoing to", &edge.to)
    } else {
        ("Incoming from", &edge.from)
    };
    ConnectionDisplay {
        direction,
        endpoint: doc
            .node(endpoint_id)
            .map_or_else(|| endpoint_id.to_string(), |node| node.label.clone()),
        kind: edge_kind_phrase(edge.kind),
        status: crate::export::status_name_pub(edge.status),
        label: edge.label.clone(),
    }
}

#[component]
pub fn ChoicePanel(
    node: Node,
    doc: Signal<SessionDoc>,
    selected: Signal<Option<NodeId>>,
    hovered_affects: Signal<Vec<NodeId>>,
) -> Element {
    let considered: Signal<HashMap<ChoiceId, Vec<OptionId>>> = use_signal(HashMap::new);
    let mut note_draft = use_signal(String::new);
    let mut label_draft = use_signal(|| node.label.clone());
    let mut kind_draft = use_signal(|| kind_value(node.kind).to_owned());
    let mut desc_draft = use_signal(|| node.description.clone());
    let mut edit_open = use_signal(|| false);
    let store = use_store();
    let mut connect_from = use_context::<super::ConnectFrom>().0;
    let node_id = node.id.clone();
    let connecting = connect_from() == Some(node.id.clone());
    let delete_title = if node.origin == Origin::User {
        "Delete this component (yours — removed immediately, with its edges)"
    } else {
        "Mark removed and ask the agent to apply the removal"
    };
    let incident: Vec<(Edge, ConnectionDisplay)> = {
        let d = doc.read();
        d.edges
            .iter()
            .filter(|edge| edge.from == node.id || edge.to == node.id)
            .map(|edge| (edge.clone(), connection_display(&node.id, edge, &d)))
            .collect()
    };

    rsx! {
        aside { class: "panel",
            div { class: "panel-head",
                h2 { "{node.label}" }
                button {
                    class: "ctl-btn",
                    title: "Close",
                    onclick: move |_| selected.set(None),
                    "✕"
                }
            }
            dl { class: "panel-meta",
                div { class: "panel-meta-row",
                    dt { "ID" }
                    dd { code { "{node.id}" } }
                }
                div { class: "panel-meta-row",
                    dt { "Kind" }
                    dd { "{super::node_card::kind_label(node.kind)}" }
                }
                div { class: "panel-meta-row",
                    dt { "Status" }
                    dd { "{super::node_card::status_class(node.status)}" }
                }
                if let Some(group) = &node.group {
                    div { class: "panel-meta-row",
                        dt { "Group" }
                        dd { "{group}" }
                    }
                }
            }
            if !node.description.is_empty() {
                p { class: "panel-desc", "{node.description}" }
            }

            div { class: "panel-actions",
                // A plain toggled button (not <details>/<summary>): WebView2
                // does not reliably expose a collapsed summary by name in the
                // UIA tree, and the E2E script drives this by name.
                button {
                    class: if edit_open() { "btn btn-armed" } else { "btn" },
                    title: "Edit this component's label, kind, and description",
                    onclick: move |_| edit_open.toggle(),
                    "Edit"
                }
                button {
                    class: if connecting { "btn btn-armed" } else { "btn" },
                    title: "Then click the target card to draw an edge from this component",
                    onclick: {
                        let node_id = node_id.clone();
                        move |_| {
                            if connecting {
                                connect_from.set(None);
                            } else {
                                connect_from.set(Some(node_id.clone()));
                            }
                        }
                    },
                    if connecting { "Cancel connect" } else { "Connect →" }
                }
                button {
                    class: "btn btn-danger",
                    title: "{delete_title}",
                    onclick: {
                        let store = store.clone();
                        let node_id = node_id.clone();
                        move |_| {
                            if let Err(err) = store.delete_node(&node_id) {
                                tracing::warn!(%err, "delete_node failed");
                            }
                            selected.set(None);
                        }
                    },
                    "Delete"
                }
            }

            if edit_open() {
                div { class: "edit-form",
                    input {
                        class: "edit-label",
                        value: "{label_draft}",
                        placeholder: "component label",
                        oninput: move |ev| label_draft.set(ev.value()),
                    }
                    select {
                        class: "edit-kind",
                        value: "{kind_draft}",
                        onchange: move |ev| kind_draft.set(ev.value()),
                        for (value, _) in KIND_VALUES {
                            option {
                                value: "{value}",
                                selected: *kind_draft.read() == value,
                                "{value}"
                            }
                        }
                    }
                    textarea {
                        class: "edit-desc",
                        value: "{desc_draft}",
                        placeholder: "description",
                        oninput: move |ev| desc_draft.set(ev.value()),
                    }
                    button {
                        class: "btn",
                        disabled: label_draft.read().trim().is_empty(),
                        onclick: {
                            let store = store.clone();
                            let node_id = node_id.clone();
                            move |_| {
                                let label = label_draft.read().trim().to_owned();
                                if let Err(err) = store.edit_node(
                                    &node_id,
                                    label,
                                    kind_from_value(&kind_draft.read()),
                                    desc_draft.read().trim().to_owned(),
                                ) {
                                    tracing::warn!(%err, "edit_node failed");
                                }
                            }
                        },
                        "Save"
                    }
                }
            }

            if !incident.is_empty() {
                div { class: "connections",
                    h3 { "Connections" }
                    for (edge, display) in incident {
                        div { class: "conn-row", key: "{edge.from}-{edge.to}-{edge.kind:?}",
                            div { class: "conn-content",
                                div { class: "conn-primary",
                                    span { class: "conn-direction", "{display.direction}" }
                                    span { class: "conn-endpoint", "{display.endpoint}" }
                                }
                                div { class: "conn-meta", "{display.kind} · {display.status}" }
                                if let Some(label) = &display.label {
                                    div { class: "conn-label", "{label}" }
                                }
                            }
                            button {
                                class: "ctl-btn",
                                title: "Delete this edge",
                                onclick: {
                                    let store = store.clone();
                                    let from = edge.from.clone();
                                    let to = edge.to.clone();
                                    let kind = edge.kind;
                                    move |_| {
                                        if let Err(err) = store.delete_edge(&from, &to, kind) {
                                            tracing::warn!(%err, "delete_edge failed");
                                        }
                                    }
                                },
                                "✕"
                            }
                        }
                    }
                }
            }

            for choice in node.choices.iter() {
                ChoiceBlock {
                    key: "{choice.id}",
                    node_id: node.id.clone(),
                    choice: choice.clone(),
                    considered,
                    hovered_affects,
                }
            }

            div { class: "panel-notes",
                h3 { "Notes for the agent" }
                for note in node.notes.iter() {
                    p { class: "note", key: "{note.id}", "{note.text}" }
                }
                textarea {
                    class: "note-input",
                    placeholder: "Constraints, context, questions…",
                    value: "{note_draft}",
                    oninput: move |ev| note_draft.set(ev.value()),
                }
                button {
                    class: "btn",
                    disabled: note_draft.read().trim().is_empty(),
                    onclick: {
                        let store = store.clone();
                        let node_id = node_id.clone();
                        move |_| {
                            let text = note_draft.read().trim().to_owned();
                            if !text.is_empty() {
                                if let Err(err) = store.add_note(&node_id, text) {
                                    tracing::warn!(%err, "add_note failed");
                                }
                                note_draft.set(String::new());
                            }
                        }
                    },
                    "Add note"
                }
            }
        }
    }
}

#[component]
fn ChoiceBlock(
    node_id: NodeId,
    choice: Choice,
    considered: Signal<HashMap<ChoiceId, Vec<OptionId>>>,
    hovered_affects: Signal<Vec<NodeId>>,
) -> Element {
    let store = use_store();
    let status_class = match choice.status {
        ChoiceStatus::Open => "open",
        ChoiceStatus::Decided => "decided",
        ChoiceStatus::Dismissed => "dismissed",
    };

    rsx! {
        section { class: "choice choice-{status_class}",
            div { class: "choice-head",
                span { class: "choice-flag", "⚑" }
                h3 { "{choice.prompt}" }
            }
            if let Some(rationale) = &choice.rationale {
                p { class: "choice-rationale", "{rationale}" }
            }
            div { class: "options",
                for opt in choice.options.iter() {
                    {
                        let picked = choice.selected.as_ref() == Some(&opt.id);
                        let option_id = opt.id.clone();
                        let affects = opt.affects.clone();
                        let choice_id = choice.id.clone();
                        let node_id = node_id.clone();
                        let store = store.clone();
                        rsx! {
                            div {
                                class: if picked { "option picked" } else { "option" },
                                key: "{opt.id}",
                                onmouseenter: {
                                    let affects = affects.clone();
                                    move |_| hovered_affects.set(affects.clone())
                                },
                                onmouseleave: move |_| hovered_affects.set(Vec::new()),
                                onclick: move |_| {
                                    let trail = considered.with_mut(|map| {
                                        let trail = map.entry(choice_id.clone()).or_default();
                                        if trail.last() != Some(&option_id) {
                                            trail.push(option_id.clone());
                                        }
                                        trail.clone()
                                    });
                                    if let Err(err) =
                                        store.select_option(&node_id, &choice_id, &option_id, trail)
                                    {
                                        tracing::warn!(%err, "select_option failed");
                                    }
                                },
                                div { class: "option-head",
                                    span { class: "option-radio",
                                        if picked { "●" } else { "○" }
                                    }
                                    span { class: "option-label", "{opt.label}" }
                                    if opt.recommended {
                                        span { class: "option-rec", title: "Recommended by the agent", "★" }
                                    }
                                }
                                if !opt.summary.is_empty() {
                                    p { class: "option-summary", "{opt.summary}" }
                                }
                                if !opt.pros.is_empty() || !opt.cons.is_empty() {
                                    div { class: "pros-cons",
                                        if !opt.pros.is_empty() {
                                            ul { class: "pros",
                                                for p in opt.pros.iter() {
                                                    li { key: "{p}", "{p}" }
                                                }
                                            }
                                        }
                                        if !opt.cons.is_empty() {
                                            ul { class: "cons",
                                                for c in opt.cons.iter() {
                                                    li { key: "{c}", "{c}" }
                                                }
                                            }
                                        }
                                    }
                                }
                                if !affects.is_empty() {
                                    div { class: "option-affects",
                                        "ripples to: "
                                        for (i, a) in affects.iter().enumerate() {
                                            span { class: "affect-chip", key: "{a}",
                                                if i > 0 { ", " }
                                                "{a}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if choice.status == ChoiceStatus::Open {
                button {
                    class: "btn btn-ghost",
                    onclick: {
                        let store = store.clone();
                        let node_id = node_id.clone();
                        let choice_id = choice.id.clone();
                        move |_| {
                            if let Err(err) = store.dismiss_choice(&node_id, &choice_id, None) {
                                tracing::warn!(%err, "dismiss_choice failed");
                            }
                        }
                    },
                    "Skip this decision"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ElementStatus, Origin};

    fn test_node(id: &str, label: &str) -> Node {
        Node {
            id: NodeId::from(id),
            label: label.into(),
            kind: NodeKind::Component,
            description: String::new(),
            status: ElementStatus::Existing,
            group: None,
            choices: vec![],
            notes: vec![],
            position: None,
            origin: Origin::Agent,
        }
    }

    fn test_doc() -> SessionDoc {
        SessionDoc {
            nodes: vec![
                test_node("api", "Public API"),
                test_node("queue", "Job Queue"),
            ],
            ..Default::default()
        }
    }

    fn test_edge() -> Edge {
        Edge {
            from: NodeId::from("api"),
            to: NodeId::from("queue"),
            kind: EdgeKind::DataFlow,
            label: Some("CompleteSecurityAuditEnvelopeIdentifier".into()),
            status: ElementStatus::Modified,
            origin: Origin::Agent,
        }
    }

    #[test]
    fn connection_display_exposes_complete_outgoing_edge() {
        let display = connection_display(&NodeId::from("api"), &test_edge(), &test_doc());

        assert_eq!(display.direction, "Outgoing to");
        assert_eq!(display.endpoint, "Job Queue");
        assert_eq!(display.kind, "data flow");
        assert_eq!(display.status, "modified");
        assert_eq!(
            display.label.as_deref(),
            Some("CompleteSecurityAuditEnvelopeIdentifier")
        );
    }

    #[test]
    fn connection_display_exposes_incoming_direction() {
        let display = connection_display(&NodeId::from("queue"), &test_edge(), &test_doc());

        assert_eq!(display.direction, "Incoming from");
        assert_eq!(display.endpoint, "Public API");
    }
}
