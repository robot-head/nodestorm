//! Right-hand panel for the selected node: description, open choices with
//! option cards (pros/cons, recommended), and the note composer.
//!
//! Hovering an option highlights the nodes it `affects` on the canvas — the
//! ripple preview. Every option click is recorded into a per-choice
//! "considered" trail that rides along with the final decision, so the agent
//! sees hesitation, not just the outcome.

use std::collections::HashMap;

use dioxus::prelude::*;

use crate::model::{Choice, ChoiceId, ChoiceStatus, Node, NodeId, OptionId};

use super::app::use_store;

#[component]
pub fn ChoicePanel(
    node: Node,
    selected: Signal<Option<NodeId>>,
    hovered_affects: Signal<Vec<NodeId>>,
) -> Element {
    let considered: Signal<HashMap<ChoiceId, Vec<OptionId>>> = use_signal(HashMap::new);
    let mut note_draft = use_signal(String::new);
    let store = use_store();
    let node_id = node.id.clone();

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
            if !node.description.is_empty() {
                p { class: "panel-desc", "{node.description}" }
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
