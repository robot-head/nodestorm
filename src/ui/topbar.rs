//! Top bar: session title, agent status pill, Export, and the Send-to-agent
//! control.

use dioxus::prelude::*;

use crate::cli::Cli;
use crate::model::{NodeId, NodeKind, SessionDoc};
use crate::store::UiMeta;

use super::app::use_store;

#[component]
pub fn TopBar(
    doc: Signal<SessionDoc>,
    meta: Signal<UiMeta>,
    selected: Signal<Option<NodeId>>,
) -> Element {
    let store = use_store();
    let cli = use_context::<Cli>();
    let mut comment = use_signal(String::new);
    let d = doc.read();
    let m = meta.read();
    let has_nodes = !d.nodes.is_empty();
    let open = m.open_choices;
    let plural = if open == 1 { "" } else { "s" };
    let title = if d.title.is_empty() {
        "untitled brainstorm".to_owned()
    } else {
        d.title.clone()
    };
    let can_send = m.undelivered > 0 || m.waiting_agents > 0;

    rsx! {
        header { class: "topbar",
            span { class: "topbar-brand", "nodestorm" }
            span { class: "topbar-title", "{title}" }
            span { class: "topbar-spacer" }
            if m.waiting_agents > 0 {
                span { class: "pill pill-waiting", "● agent is waiting for your decisions" }
            }
            if open > 0 {
                span { class: "pill pill-open", "{open} open decision{plural}" }
            }
            if m.undelivered > 0 {
                span { class: "pill pill-undelivered", "{m.undelivered} to send" }
            }
            button {
                class: "btn",
                title: "Add a component you own to the canvas (agents adopt it if they enrich it)",
                onclick: {
                    let store = store.clone();
                    let mut selected = selected;
                    move |_| {
                        match store.add_user_node(
                            "New component".into(),
                            NodeKind::Component,
                            None,
                        ) {
                            Ok(id) => selected.set(Some(id)),
                            Err(err) => tracing::warn!(%err, "add component failed"),
                        }
                    }
                },
                "+ Component"
            }
            button {
                class: "btn",
                disabled: !has_nodes,
                title: "Save a Markdown decision record (with Mermaid diagram) next to the session file",
                onclick: {
                    let store = store.clone();
                    move |_| {
                        let outcome = cli.session_path().and_then(|session| {
                            let path = crate::persist::export_path(&session);
                            let markdown = store.read(|s| {
                                crate::export::render_markdown(
                                    &s.doc,
                                    &s.decision_log,
                                    chrono::Utc::now(),
                                )
                            });
                            crate::persist::save_export(&path, &markdown)?;
                            Ok(path)
                        });
                        match outcome {
                            Ok(path) => store.record_export(&path),
                            Err(err) => {
                                tracing::warn!(%err, "export failed");
                                store.record_export_failed(&err.to_string());
                            }
                        }
                    }
                },
                "Export"
            }
            input {
                class: "send-comment",
                placeholder: "optional message to the agent…",
                value: "{comment}",
                oninput: move |ev| comment.set(ev.value()),
            }
            button {
                class: "btn btn-send",
                disabled: !can_send,
                title: "Deliver your decisions and notes to the waiting agent",
                onclick: {
                    let store = store.clone();
                    move |_| {
                        let text = comment.read().trim().to_owned();
                        store.request_flush(if text.is_empty() { None } else { Some(text) });
                        comment.set(String::new());
                    }
                },
                "Send to agent"
            }
        }
    }
}
