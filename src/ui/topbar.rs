//! Top bar: brand, session switcher, search, the fused status chip, the
//! action pods, the ⋯ More menu mount, and the Send-to-agent control.

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
    queued_changes_open: Signal<bool>,
    questions_open: Signal<bool>,
) -> Element {
    let store = use_store();
    let sessions = use_context::<std::sync::Arc<crate::sessions::Sessions>>();
    let composer = use_context::<super::MessageComposer>();
    let mut comment = composer.comment;
    let mut compose_open = composer.open;
    let mut sessions_open = use_signal(|| false);
    let mut new_session_draft = use_signal(String::new);
    let mut rename_draft = use_signal(String::new);
    let mut manage_open = use_signal(|| false);
    let mut delete_pending = use_signal(|| false);
    let mut compare_with = use_context::<super::CompareWith>().0;
    let d = doc.read();
    let m = meta.read();
    let has_nodes = !d.nodes.is_empty();
    let open = m.open_choices;
    let plural = if open == 1 { "" } else { "s" };
    let open_q = d.open_question_count();
    let build_tracked = d.nodes.iter().filter(|n| n.build.is_some()).count();
    let build_shipped = d
        .nodes
        .iter()
        .filter(|n| n.build.is_some_and(crate::model::BuildStatus::is_shipped))
        .count();
    let title = if d.title.is_empty() {
        "untitled brainstorm".to_owned()
    } else {
        d.title.clone()
    };
    let can_send = m.undelivered > 0 || m.waiting_agents > 0;
    // Blocked replay entries are also actionable queued changes, even though
    // they cannot be sent; retain access to their remove/edit controls.
    let queued_count = store.queued_changes().len();
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
                    class: "btn sess-pod",
                    title: "Switch, create, or archive named sessions",
                    onclick: move |_| {
                        if sessions_open() {
                            new_session_draft.set(String::new());
                            rename_draft.set(String::new());
                            manage_open.set(false);
                            delete_pending.set(false);
                            sessions_open.set(false);
                        } else {
                            sessions_open.set(true);
                        }
                    },
                    "{session_name} ▾"
                }
                if sessions_open() {
                    div {
                        class: "menu-catcher",
                        onclick: move |_| {
                            new_session_draft.set(String::new());
                            rename_draft.set(String::new());
                            manage_open.set(false);
                            delete_pending.set(false);
                            sessions_open.set(false);
                        },
                    }
                    div { class: "export-dropdown sessions-dropdown",
                        div { class: "session-doc-title", title: "{title}",
                            span { "Brainstorm" }
                            div { class: "session-doc-heading",
                                strong { "{title}" }
                                span { class: "sess-badges",
                                    if open > 0 {
                                        span { class: "pill pill-open", "{open}" }
                                    }
                                    if m.waiting_agents > 0 {
                                        span { class: "pill pill-waiting", "●" }
                                    }
                                }
                            }
                        }
                        for info in sessions.list() {
                            if !info.active {
                                div {
                                    key: "{info.name}",
                                    class: "session-row",
                                    button {
                                        class: "sess-switch",
                                        onclick: {
                                            let sessions = sessions.clone();
                                            let name = info.name.clone();
                                            move |_| {
                                                if let Err(err) = sessions.switch(&name) {
                                                    tracing::warn!(%err, "switch failed");
                                                }
                                                new_session_draft.set(String::new());
                                                rename_draft.set(String::new());
                                                manage_open.set(false);
                                                delete_pending.set(false);
                                                sessions_open.set(false);
                                            }
                                        },
                                        span { class: "sess-name", title: "{info.name}", "{info.name}" }
                                        span { class: "sess-badges",
                                            if info.open_choices > 0 {
                                                span { class: "pill pill-open", "{info.open_choices}" }
                                            }
                                            if info.agent_waiting {
                                                span { class: "pill pill-waiting", "●" }
                                            }
                                        }
                                    }
                                    button {
                                        class: "ctl-btn",
                                        title: "Compare this session with the active one",
                                        onclick: {
                                            let name = info.name.clone();
                                            move |_| {
                                                compare_with.set(Some(name.clone()));
                                                new_session_draft.set(String::new());
                                                rename_draft.set(String::new());
                                                manage_open.set(false);
                                                delete_pending.set(false);
                                                sessions_open.set(false);
                                            }
                                        },
                                        "Compare"
                                    }
                                }
                            }
                        }
                        button {
                            class: "session-manage-toggle",
                            onclick: move |_| { delete_pending.set(false); manage_open.toggle(); },
                            "Manage session"
                        }
                        if manage_open() {
                            div { class: "session-manage",
                                div { class: "session-form",
                                    label { r#for: "rename-session", "Rename current session" }
                                    div { class: "session-form-row",
                                        input {
                                            id: "rename-session",
                                            class: "session-name-input",
                                            placeholder: "new name…",
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
                                                    new_session_draft.set(String::new());
                                                    rename_draft.set(String::new());
                                                    manage_open.set(false);
                                                    delete_pending.set(false);
                                                    sessions_open.set(false);
                                                }
                                            },
                                            "Rename"
                                        }
                                    }
                                }
                                div { class: "session-form",
                                    label { r#for: "create-session", "Create new session" }
                                    div { class: "session-form-row",
                                        input {
                                            id: "create-session",
                                            class: "session-name-input",
                                            placeholder: "session name…",
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
                                                    new_session_draft.set(String::new());
                                                    rename_draft.set(String::new());
                                                    manage_open.set(false);
                                                    delete_pending.set(false);
                                                    sessions_open.set(false);
                                                }
                                            },
                                            "Create"
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "session-danger",
                            span { class: "session-section-label", "Danger zone" }
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
                                        new_session_draft.set(String::new());
                                        rename_draft.set(String::new());
                                        manage_open.set(false);
                                        delete_pending.set(false);
                                        sessions_open.set(false);
                                    }
                                },
                                "Archive session"
                            }
                            if delete_pending() {
                                div { class: "delete-confirm",
                                    span { "Delete {session_name} permanently?" }
                                    button {
                                        class: "session-delete-confirm",
                                        onclick: {
                                            let sessions = sessions.clone();
                                            move |_| {
                                                let name = sessions.active_name();
                                                if let Err(err) = sessions.delete(&name) {
                                                    tracing::warn!(%err, "delete failed");
                                                }
                                                new_session_draft.set(String::new());
                                                rename_draft.set(String::new());
                                                manage_open.set(false);
                                                delete_pending.set(false);
                                                sessions_open.set(false);
                                            }
                                        },
                                        "Confirm delete"
                                    }
                                    button {
                                        class: "session-cancel",
                                        onclick: move |_| delete_pending.set(false),
                                        "Cancel"
                                    }
                                }
                            } else {
                                button {
                                    class: "session-delete",
                                    onclick: move |_| delete_pending.set(true),
                                    "Delete session"
                                }
                            }
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
                                                title: "Restore archived session {name}",
                                                onclick: {
                                                    let sessions = sessions.clone();
                                                    let name = name.clone();
                                                    move |_| {
                                                        if let Err(err) = sessions.unarchive(&name) {
                                                            tracing::warn!(%err, "unarchive failed");
                                                        }
                                                        new_session_draft.set(String::new());
                                                        rename_draft.set(String::new());
                                                        manage_open.set(false);
                                                        delete_pending.set(false);
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
            span { class: "search-pod",
                span { class: "search-glyph", "⌕" }
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
            }
            span { class: "topbar-title", "data-full-title": "{title}",
                span { class: "topbar-title-text", title: "{title}", "{title}" }
            }
            span { class: "topbar-spacer" }
            if m.waiting_agents > 0 || open > 0 || open_q > 0 || build_tracked > 0 || queued_count > 0 {
                span { class: "status-chip",
                    if build_tracked > 0 {
                        span {
                            class: "seg seg-build",
                            role: "status",
                            aria_label: "{build_shipped} of {build_tracked} components shipped",
                            title: "Implementation progress: built or verified out of tracked",
                            span { class: "seg-glyph", "▸" }
                            "{build_shipped}/{build_tracked}"
                            span { class: "seg-word", " built" }
                        }
                    }
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
                    if open_q > 0 {
                        button {
                            class: if questions_open() { "seg seg-questions btn-armed" } else { "seg seg-questions" },
                            aria_label: "{open_q} open questions",
                            title: "Answer the agent's free-form questions",
                            onclick: {
                                let mut selected = selected;
                                let mut timeline_open = timeline_open;
                                let mut queued_changes_open = queued_changes_open;
                                let mut questions_open = questions_open;
                                let mut compare_with = compare_with;
                                move |_| {
                                    if !questions_open() {
                                        selected.set(None);
                                        timeline_open.set(false);
                                        queued_changes_open.set(false);
                                        compare_with.set(None);
                                    }
                                    questions_open.toggle();
                                }
                            },
                            span { class: "seg-glyph", "?" }
                            "{open_q}"
                            span { class: "seg-word", " to answer" }
                        }
                    }
                    if queued_count > 0 {
                        button {
                            class: if queued_changes_open() { "seg seg-queued btn-armed" } else { "seg seg-queued" },
                            aria_label: "{queued_count} queued changes",
                            title: "Review, edit, or remove queued changes",
                            onclick: {
                                let mut selected = selected;
                                let mut timeline_open = timeline_open;
                                let mut queued_changes_open = queued_changes_open;
                                let mut questions_open = questions_open;
                                let mut compare_with = compare_with;
                                move |_| {
                                    if !queued_changes_open() {
                                        selected.set(None);
                                        timeline_open.set(false);
                                        questions_open.set(false);
                                        compare_with.set(None);
                                    }
                                    queued_changes_open.toggle();
                                }
                            },
                            "{queued_count}"
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
                    let mut queued_changes_open = queued_changes_open;
                    let mut questions_open = questions_open;
                    move |_| {
                        selected.set(None);
                        queued_changes_open.set(false);
                        questions_open.set(false);
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
            div { class: "export-menu compose-menu",
                button {
                    class: "btn pod-icon pod-compose",
                    aria_label: "Message to agent",
                    title: "Attach an optional message to your next Send",
                    onclick: move |_| compose_open.toggle(),
                    "✎"
                }
                if compose_open() {
                    div {
                        class: "menu-catcher",
                        onclick: move |_| compose_open.set(false),
                    }
                    div { class: "export-dropdown compose-pop",
                        textarea {
                            class: "note-input compose-input",
                            placeholder: "optional message to the agent…",
                            value: "{comment}",
                            oninput: move |ev| comment.set(ev.value()),
                        }
                        button {
                            class: "btn btn-send",
                            disabled: !can_send,
                            onclick: {
                                let store = store.clone();
                                move |_| {
                                    let text = comment.read().trim().to_owned();
                                    store.request_flush(if text.is_empty() { None } else { Some(text) });
                                    comment.set(String::new());
                                    compose_open.set(false);
                                }
                            },
                            "Send with message"
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
