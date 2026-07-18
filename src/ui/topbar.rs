//! Top bar: session title, agent status pill, Export, and the Send-to-agent
//! control.

use dioxus::prelude::*;

use crate::model::{NodeId, NodeKind, SessionDoc};
use crate::store::UiMeta;

use super::app::use_store;

#[component]
pub fn TopBar(
    doc: Signal<SessionDoc>,
    meta: Signal<UiMeta>,
    selected: Signal<Option<NodeId>>,
    session_name: Signal<String>,
    timeline_open: Signal<bool>,
) -> Element {
    let store = use_store();
    let sessions = use_context::<std::sync::Arc<crate::sessions::Sessions>>();
    let mut comment = use_signal(String::new);
    let mut sessions_open = use_signal(|| false);
    let mut new_session_draft = use_signal(String::new);
    let mut rename_draft = use_signal(String::new);
    let mut compare_with = use_context::<super::CompareWith>().0;
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
            span { class: "topbar-brand",
                span { class: "topbar-bolt", "ϟ" }
                span { class: "topbar-word", "nodestorm" }
            }
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
                            div {
                                key: "{info.name}",
                                class: if info.active { "session-row active" } else { "session-row" },
                                button {
                                    class: "sess-switch",
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
                                if !info.active {
                                    button {
                                        class: "ctl-btn",
                                        title: "Compare this session with the active one",
                                        onclick: {
                                            let name = info.name.clone();
                                            move |_| {
                                                compare_with.set(Some(name.clone()));
                                                sessions_open.set(false);
                                            }
                                        },
                                        "Compare"
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
                        div { class: "session-create",
                            input {
                                class: "session-name-input",
                                placeholder: "rename to…",
                                value: "{rename_draft}",
                                oninput: move |ev| rename_draft.set(ev.value()),
                            }
                            button {
                                class: "btn",
                                disabled: rename_draft.read().trim().is_empty(),
                                onclick: {
                                    let sessions = sessions.clone();
                                    move |_| {
                                        let name = rename_draft.read().trim().to_owned();
                                        let active = sessions.active_name();
                                        match sessions.rename(&active, &name) {
                                            Ok(_) => rename_draft.set(String::new()),
                                            Err(err) => tracing::warn!(%err, "rename failed"),
                                        }
                                        sessions_open.set(false);
                                    }
                                },
                                "Rename"
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
                        button {
                            class: "session-archive",
                            title: "Delete the active session permanently (file and all)",
                            onclick: {
                                let sessions = sessions.clone();
                                move |_| {
                                    let name = sessions.active_name();
                                    if let Err(err) = sessions.delete(&name) {
                                        tracing::warn!(%err, "delete failed");
                                    }
                                    sessions_open.set(false);
                                }
                            },
                            "Delete current"
                        }
                        {
                            let archived = sessions.list_archived();
                            rsx! {
                                if !archived.is_empty() {
                                    div { class: "archived-list",
                                        span { class: "archived-label", "Archived" }
                                        for name in archived {
                                            button {
                                                key: "{name}",
                                                class: "sess-switch",
                                                title: "Bring this archived session back",
                                                onclick: {
                                                    let sessions = sessions.clone();
                                                    let name = name.clone();
                                                    move |_| {
                                                        if let Err(err) = sessions.unarchive(&name) {
                                                            tracing::warn!(%err, "unarchive failed");
                                                        }
                                                        sessions_open.set(false);
                                                    }
                                                },
                                                "↩ {name}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
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
            span { class: "topbar-title", "{title}" }
            span { class: "topbar-spacer" }
            if m.waiting_agents > 0 || open > 0 || m.undelivered > 0 {
                span { class: "status-chip",
                    if m.waiting_agents > 0 {
                        span {
                            class: "seg seg-waiting",
                            role: "status",
                            aria_label: "● agent is waiting for your decisions",
                            title: "An agent is blocked in await_decisions",
                            span { class: "seg-dot", "●" }
                            span { class: "seg-word", "waiting" }
                        }
                    }
                    if open > 0 {
                        span {
                            class: "seg seg-open",
                            role: "status",
                            aria_label: "{open} open decision{plural}",
                            title: "Choices still waiting for a pick",
                            "{open}"
                            span { class: "seg-word", " open" }
                        }
                    }
                    if m.undelivered > 0 {
                        span {
                            class: "seg seg-queued",
                            role: "status",
                            aria_label: "{m.undelivered} to send",
                            title: "Decisions queued for the next Send",
                            "{m.undelivered}"
                            span { class: "seg-word", " queued" }
                        }
                    }
                }
            }
            button {
                class: "btn pod-icon pod-undo",
                disabled: !m.undo_available,
                aria_label: "↶ Undo",
                title: "Undo your last edit or undelivered decision (Ctrl+Z)",
                onclick: {
                    let store = store.clone();
                    move |_| {
                        store.undo();
                    }
                },
                "↶"
            }
            button {
                class: "btn pod-icon pod-redo",
                disabled: !m.redo_available,
                aria_label: "↷ Redo",
                title: "Redo (Ctrl+Y)",
                onclick: {
                    let store = store.clone();
                    move |_| {
                        store.redo();
                    }
                },
                "↷"
            }
            button {
                class: if timeline_open() { "btn btn-armed" } else { "btn" },
                aria_label: "Timeline",
                title: "Show the session log: every decision, note, and edit in order",
                onclick: {
                    let mut selected = selected;
                    let mut timeline_open = timeline_open;
                    move |_| {
                        selected.set(None);
                        timeline_open.toggle();
                    }
                },
                span { class: "pod-glyph", "◷" }
                span { class: "pod-label", "Timeline" }
            }
            button {
                class: "btn",
                aria_label: "+ Component",
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
                span { class: "pod-glyph", "+" }
                span { class: "pod-label", "node" }
            }
            super::more_menu::MoreMenu { has_nodes, suggested_name: suggested_name.clone() }
            input {
                class: "send-comment",
                placeholder: "optional message to the agent…",
                value: "{comment}",
                oninput: move |ev| comment.set(ev.value()),
            }
            button {
                class: "btn btn-send",
                disabled: !can_send,
                aria_label: "Send to agent",
                title: "Deliver your decisions and notes to the waiting agent",
                onclick: {
                    let store = store.clone();
                    move |_| {
                        let text = comment.read().trim().to_owned();
                        store.request_flush(if text.is_empty() { None } else { Some(text) });
                        comment.set(String::new());
                    }
                },
                span { class: "send-bolt", "ϟ" }
                "Send"
            }
        }
    }
}
