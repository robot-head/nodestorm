//! Top bar: session title, agent status pill, Export, and the Send-to-agent
//! control.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::cli::Cli;
use crate::model::{NodeId, NodeKind, SessionDoc};
use crate::store::{Store, UiMeta};

use super::app::use_store;

/// Render the full decision record from current store state.
fn render_markdown_now(store: &Arc<Store>) -> String {
    store.read(|s| crate::export::render_markdown(&s.doc, &s.decision_log, chrono::Utc::now()))
}

/// Activity-feed receipt for an export outcome.
fn report_export(store: &Arc<Store>, outcome: anyhow::Result<std::path::PathBuf>) {
    match outcome {
        Ok(path) => store.record_export(&path),
        Err(err) => {
            tracing::warn!(%err, "export failed");
            store.record_export_failed(&err.to_string());
        }
    }
}

/// Clipboard write via the WebView's `navigator.clipboard` (no native
/// clipboard dependency); the receipt lands in the activity feed.
fn copy_to_clipboard(store: &Arc<Store>, text: String, receipt: &str) {
    match serde_json::to_string(&text) {
        Ok(js) => {
            document::eval(&format!("navigator.clipboard.writeText({js});"));
            store.record_user_action(receipt.to_owned());
        }
        Err(err) => tracing::warn!(%err, "clipboard serialization failed"),
    }
}

#[component]
pub fn TopBar(
    doc: Signal<SessionDoc>,
    meta: Signal<UiMeta>,
    selected: Signal<Option<NodeId>>,
    session_name: Signal<String>,
) -> Element {
    let store = use_store();
    let cli = use_context::<Cli>();
    let sessions = use_context::<std::sync::Arc<crate::sessions::Sessions>>();
    let mut comment = use_signal(String::new);
    let mut sessions_open = use_signal(|| false);
    let mut new_session_draft = use_signal(String::new);
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
    let mut export_open = use_signal(|| false);
    let suggested_name = {
        let slug = if d.title.is_empty() {
            "brainstorm".to_owned()
        } else {
            crate::store::slugify(&d.title)
        };
        format!("{slug}-decisions.md")
    };

    let mut search = use_context::<super::SearchQuery>().0;
    let mut zoom_target = use_context::<super::ZoomTarget>().0;
    let mut search_cursor = use_signal(|| 0usize);

    rsx! {
        header { class: "topbar",
            span { class: "topbar-brand", "nodestorm" }
            div { class: "export-menu",
                button {
                    class: "btn",
                    title: "Switch, create, or archive named sessions",
                    onclick: move |_| sessions_open.toggle(),
                    "{session_name} ▾"
                }
                if sessions_open() {
                    div {
                        class: "menu-catcher",
                        onclick: move |_| sessions_open.set(false),
                    }
                    div { class: "export-dropdown sessions-dropdown",
                        for info in sessions.list() {
                            button {
                                key: "{info.name}",
                                class: if info.active { "session-row active" } else { "session-row" },
                                onclick: {
                                    let sessions = sessions.clone();
                                    let name = info.name.clone();
                                    move |_| {
                                        if let Err(err) = sessions.switch(&name) {
                                            tracing::warn!(%err, "switch failed");
                                        }
                                        sessions_open.set(false);
                                    }
                                },
                                span { class: "sess-name", "{info.name}" }
                                span { class: "sess-badges",
                                    if info.open_choices > 0 {
                                        span { class: "pill pill-open", "{info.open_choices}" }
                                    }
                                    if info.agent_waiting {
                                        span { class: "pill pill-waiting", "●" }
                                    }
                                }
                            }
                        }
                        div { class: "session-create",
                            input {
                                class: "session-name-input",
                                placeholder: "new session…",
                                value: "{new_session_draft}",
                                oninput: move |ev| new_session_draft.set(ev.value()),
                            }
                            button {
                                class: "btn",
                                disabled: new_session_draft.read().trim().is_empty(),
                                onclick: {
                                    let sessions = sessions.clone();
                                    move |_| {
                                        let name = new_session_draft.read().trim().to_owned();
                                        match sessions.create(&name) {
                                            Ok(slug) => {
                                                let _ = sessions.switch(&slug);
                                                new_session_draft.set(String::new());
                                            }
                                            Err(err) => {
                                                tracing::warn!(%err, "create session failed");
                                            }
                                        }
                                        sessions_open.set(false);
                                    }
                                },
                                "Create"
                            }
                        }
                        button {
                            class: "session-archive",
                            title: "Save this session's file into sessions/archive/ and drop it from the list",
                            onclick: {
                                let sessions = sessions.clone();
                                move |_| {
                                    let name = sessions.active_name();
                                    if let Err(err) = sessions.archive(&name) {
                                        tracing::warn!(%err, "archive failed");
                                    }
                                    sessions_open.set(false);
                                }
                            },
                            "Archive current"
                        }
                    }
                }
            }
            span { class: "topbar-title", "{title}" }
            input {
                class: "search-box",
                placeholder: "search components…",
                value: "{search}",
                oninput: move |ev| {
                    search.set(ev.value());
                    search_cursor.set(0);
                },
                onkeydown: move |ev| {
                    if ev.key() == Key::Enter {
                        // Cycle through matches in document order, zooming to
                        // each.
                        let matches: Vec<_> = doc
                            .read()
                            .nodes
                            .iter()
                            .filter(|n| crate::ui::node_matches(n, &search.read()))
                            .map(|n| n.id.clone())
                            .collect();
                        if !matches.is_empty() {
                            let i = search_cursor() % matches.len();
                            zoom_target.set(Some(matches[i].clone()));
                            search_cursor.set(i + 1);
                        }
                    } else if ev.key() == Key::Escape {
                        search.set(String::new());
                        search_cursor.set(0);
                    }
                },
            }
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
            div { class: "export-menu",
                button {
                    class: "btn",
                    disabled: !has_nodes,
                    title: "Export the brainstorm as a decision record",
                    onclick: move |_| export_open.toggle(),
                    "Export ▾"
                }
                if export_open() {
                    div {
                        class: "menu-catcher",
                        onclick: move |_| export_open.set(false),
                    }
                    div { class: "export-dropdown",
                        button {
                            title: "Write the Markdown record next to the session file",
                            onclick: {
                                let store = store.clone();
                                let cli = cli.clone();
                                move |_| {
                                    export_open.set(false);
                                    let outcome = cli.session_path().and_then(|session| {
                                        let path = crate::persist::export_path(&session);
                                        crate::persist::save_export(
                                            &path,
                                            &render_markdown_now(&store),
                                        )?;
                                        Ok(path)
                                    });
                                    report_export(&store, outcome);
                                }
                            },
                            "Export"
                        }
                        button {
                            title: "Pick where the Markdown record is saved",
                            onclick: {
                                let store = store.clone();
                                let suggested = suggested_name.clone();
                                move |_| {
                                    export_open.set(false);
                                    let Some(path) = rfd::FileDialog::new()
                                        .set_file_name(&suggested)
                                        .save_file()
                                    else {
                                        return;
                                    };
                                    let outcome = crate::persist::save_export(
                                        &path,
                                        &render_markdown_now(&store),
                                    )
                                    .map(|()| path);
                                    report_export(&store, outcome);
                                }
                            },
                            "Export As…"
                        }
                        button {
                            title: "Copy the Markdown record to the clipboard",
                            onclick: {
                                let store = store.clone();
                                move |_| {
                                    export_open.set(false);
                                    copy_to_clipboard(
                                        &store,
                                        render_markdown_now(&store),
                                        "copied the decision record to the clipboard",
                                    );
                                }
                            },
                            "Copy Markdown"
                        }
                        button {
                            title: "Copy just the Mermaid diagram to the clipboard",
                            onclick: {
                                let store = store.clone();
                                move |_| {
                                    export_open.set(false);
                                    let text = format!(
                                        "```mermaid\n{}```\n",
                                        store.read(|s| crate::export::render_mermaid(&s.doc)),
                                    );
                                    copy_to_clipboard(
                                        &store,
                                        text,
                                        "copied the Mermaid diagram to the clipboard",
                                    );
                                }
                            },
                            "Copy Mermaid"
                        }
                        button {
                            title: "Write just the Mermaid diagram next to the session file",
                            onclick: {
                                let store = store.clone();
                                let cli = cli.clone();
                                move |_| {
                                    export_open.set(false);
                                    let outcome = cli.session_path().and_then(|session| {
                                        let path =
                                            crate::persist::mermaid_export_path(&session);
                                        let body = store
                                            .read(|s| crate::export::render_mermaid(&s.doc));
                                        crate::persist::save_export(
                                            &path,
                                            &format!("```mermaid\n{body}```\n"),
                                        )?;
                                        Ok(path)
                                    });
                                    report_export(&store, outcome);
                                }
                            },
                            "Export Mermaid only"
                        }
                    }
                }
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
