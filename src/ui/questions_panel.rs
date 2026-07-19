//! Free-form agent questions: the shared per-question block (prompt, optional
//! attached component, and a text answer box) plus the standalone Questions
//! panel listing every question in the session.
//!
//! Questions ride the same decision queue as choices and notes — answering
//! one records a `question_answered` event delivered exactly once on the next
//! Send. Attached questions also surface inline in their node's panel.

use dioxus::prelude::*;

use crate::model::{Question, SessionDoc};

use super::app::use_store;

/// One question: prompt, rationale, an optional attached-node hint, and either
/// the given answer plus a revise box, or a compose box. Shared by the node
/// panel (`show_attachment: false`) and the Questions panel (`true`).
#[component]
pub fn QuestionBlock(
    question: Question,
    show_attachment: bool,
    doc: Signal<SessionDoc>,
) -> Element {
    let store = use_store();
    let mut draft = use_signal(|| question.answer.clone().unwrap_or_default());
    let answered = question.is_answered();
    let attached_label = question
        .node_id
        .as_ref()
        .and_then(|id| doc.read().node(id).map(|n| n.label.clone()));
    let status_class = if answered { "answered" } else { "open" };

    rsx! {
        section { class: "question question-{status_class}",
            div { class: "question-head",
                span { class: "question-flag", "?" }
                h3 { "{question.prompt}" }
            }
            if show_attachment {
                if let Some(label) = &attached_label {
                    p { class: "question-attach", "about {label}" }
                }
            }
            if let Some(rationale) = &question.rationale {
                p { class: "question-rationale", "{rationale}" }
            }
            if answered {
                p { class: "question-answer", "{question.answer.clone().unwrap_or_default()}" }
            }
            textarea {
                class: "note-input question-input",
                placeholder: if answered { "revise your answer…" } else { "type your answer…" },
                value: "{draft}",
                oninput: move |ev| draft.set(ev.value()),
            }
            button {
                class: "btn",
                disabled: draft.read().trim().is_empty(),
                onclick: {
                    let store = store.clone();
                    let id = question.id.clone();
                    move |_| {
                        let text = draft.read().trim().to_owned();
                        if !text.is_empty()
                            && let Err(err) = store.answer_question(&id, text)
                        {
                            tracing::warn!(%err, "answer_question failed");
                        }
                    }
                },
                if answered { "Update answer" } else { "Answer" }
            }
        }
    }
}

/// Right-hand panel listing every question in the session — open ones first,
/// then answered — each answerable in place.
#[component]
pub fn QuestionsPanel(doc: Signal<SessionDoc>, on_close: EventHandler<()>) -> Element {
    let d = doc.read();
    let open: Vec<Question> = d
        .questions
        .iter()
        .filter(|q| !q.is_answered())
        .cloned()
        .collect();
    let answered: Vec<Question> = d
        .questions
        .iter()
        .filter(|q| q.is_answered())
        .cloned()
        .collect();

    rsx! {
        aside { class: "panel questions-panel",
            div { class: "panel-head",
                h2 { "Questions" }
                button {
                    class: "ctl-btn",
                    title: "Close",
                    onclick: move |_| on_close.call(()),
                    "✕"
                }
            }
            if d.questions.is_empty() {
                p { class: "panel-desc", "The agent hasn't asked anything yet." }
            }
            if !open.is_empty() {
                h3 { class: "questions-section", "Open" }
                for q in open {
                    QuestionBlock { key: "{q.id}", question: q, show_attachment: true, doc }
                }
            }
            if !answered.is_empty() {
                h3 { class: "questions-section", "Answered" }
                for q in answered {
                    QuestionBlock { key: "{q.id}", question: q, show_attachment: true, doc }
                }
            }
        }
    }
}
