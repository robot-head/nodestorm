//! Top bar: brand, session switcher, search, the fused status chip, the
//! action pods, the ⋯ More menu mount, and the Send-to-agent control.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::model::{NodeId, NodeKind, SessionDoc};
use crate::sessions::{ConnectionInfo, ConnectionState};
use crate::store::{SendStatus, Store, UiMeta};

use super::app::use_store;

fn send_succeeded<E>(result: Result<(), E>) -> bool {
    result.is_ok()
}

fn send_label(status: SendStatus) -> &'static str {
    match status {
        SendStatus::Idle => "Send",
        SendStatus::Sending => "Sending...",
        SendStatus::Sent => "Sent",
        SendStatus::Reconnecting => "Reconnecting...",
        SendStatus::Failed => "Failed - Retry",
    }
}

fn connection_state_label(state: &ConnectionState) -> String {
    let scoped = |label: &str, session: &str, agent: &Option<String>| match agent {
        Some(agent) => format!("{label} · {session} · {agent}"),
        None => format!("{label} · {session}"),
    };
    match state {
        ConnectionState::Connected => "Connected".into(),
        ConnectionState::Waiting { session, agent } => scoped("Waiting", session, agent),
        ConnectionState::Receiving { session, agent } => scoped("Receiving", session, agent),
        ConnectionState::Reconnecting { session, agent } => scoped("Reconnecting", session, agent),
    }
}

fn submit_send(store: &Arc<Store>, mut comment: Signal<String>, mut compose_open: Signal<bool>) {
    let text = comment.read().trim().to_owned();
    if send_succeeded(store.request_flush((!text.is_empty()).then_some(text))) {
        comment.set(String::new());
        compose_open.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::{connection_state_label, send_label, send_succeeded};
    use crate::sessions::ConnectionState;
    use crate::store::SendStatus;

    #[test]
    fn send_error_preserves_the_draft() {
        assert!(send_succeeded(Ok::<(), ()>(())), "success clears the draft");
        assert!(
            !send_succeeded(Err::<(), ()>(())),
            "failure preserves the draft"
        );
    }

    #[test]
    fn send_labels_are_receipt_driven() {
        assert_eq!(send_label(SendStatus::Idle), "Send");
        assert_eq!(send_label(SendStatus::Sending), "Sending...");
        assert_eq!(send_label(SendStatus::Sent), "Sent");
        assert_eq!(send_label(SendStatus::Reconnecting), "Reconnecting...");
        assert_eq!(send_label(SendStatus::Failed), "Failed - Retry");
    }

    #[test]
    fn connection_labels_name_state_session_and_agent() {
        assert_eq!(
            connection_state_label(&ConnectionState::Connected),
            "Connected"
        );
        assert_eq!(
            connection_state_label(&ConnectionState::Waiting {
                session: "plan".into(),
                agent: Some("alpha".into()),
            }),
            "Waiting · plan · alpha"
        );
        assert_eq!(
            connection_state_label(&ConnectionState::Reconnecting {
                session: "plan".into(),
                agent: None,
            }),
            "Reconnecting · plan"
        );
    }
}

#[component]
pub fn TopBar(
    doc: Signal<SessionDoc>,
    meta: Signal<UiMeta>,
    connections: Signal<Vec<ConnectionInfo>>,
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
    let mut launcher_open = use_context::<super::AgentLauncherOpen>().0;
    let mut connections_open = use_signal(|| false);
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
    let can_send = (m.undelivered > 0 || m.waiting_agents > 0)
        && matches!(m.send_status, SendStatus::Idle | SendStatus::Failed);
    let send_text = send_label(m.send_status);
    let send_class = match m.send_status {
        SendStatus::Sent => "btn btn-send sent",
        SendStatus::Failed => "btn btn-send failed",
        _ if can_send => "btn btn-send armed",
        _ => "btn btn-send",
    };
    let connection_rows = connections.read().clone();
    let live_connections = connection_rows
        .iter()
        .filter(|connection| !matches!(connection.state, ConnectionState::Reconnecting { .. }))
        .count();
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
    let mark_points = crate::icon::svg_points();

    rsx! {
        header { class: "topbar",
            span { class: "topbar-brand",
                svg {
                    class: "topbar-mark",
                    view_box: crate::icon::VIEW_BOX,
                    "aria-hidden": "true",
                    polyline {
                        points: "{mark_points}",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "{crate::icon::STROKE_WIDTH}",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                    }
                    for index in crate::icon::NODE_INDICES {
                        circle {
                            cx: "{crate::icon::BOLT_POINTS[index].0}",
                            cy: "{crate::icon::BOLT_POINTS[index].1}",
                            r: "{crate::icon::NODE_RADIUS}",
                            fill: "currentColor",
                        }
                    }
                }
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
                            onclick: move |_| {
                                new_session_draft.set(String::new());
                                rename_draft.set(String::new());
                                manage_open.set(false);
                                delete_pending.set(false);
                                sessions_open.set(false);
                                launcher_open.set(true);
                            },
                            "Start agent"
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
                            class: send_class,
                            disabled: !can_send,
                            onclick: {
                                let store = store.clone();
                                move |_| submit_send(&store, comment, compose_open)
                            },
                            "{send_text}"
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
            div { class: "export-menu connection-pod",
                button {
                    class: "btn connection-toggle",
                    aria_label: "Claude MCP connections",
                    title: if live_connections > 0 {
                        format!("{live_connections} connected Claude MCP client(s)")
                    } else {
                        "No connected Claude MCP clients".to_owned()
                    },
                    onclick: move |_| connections_open.toggle(),
                    span {
                        class: if live_connections > 0 { "connection-dot live" } else { "connection-dot" },
                        "●"
                    }
                    span { class: "connection-count", "{live_connections}" }
                }
                if connections_open() {
                    div {
                        class: "menu-catcher",
                        onclick: move |_| connections_open.set(false),
                    }
                    div { class: "export-dropdown connection-pop",
                        for connection in connection_rows {
                            {
                                let state = connection_state_label(&connection.state);
                                let state_class = if matches!(
                                    connection.state,
                                    ConnectionState::Reconnecting { .. }
                                ) {
                                    "connection-state reconnecting"
                                } else {
                                    "connection-state"
                                };
                                rsx! {
                                    div { key: "{connection.id.0}", class: "connection-row",
                                        span { class: "connection-client", "{connection.client_name}" }
                                        span { class: state_class, "{state}" }
                                        span { class: "connection-meta", "{connection.version}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            button {
                class: send_class,
                disabled: !can_send,
                aria_label: "Send to agent",
                title: "Deliver your decisions and notes to the waiting agent",
                onclick: {
                    let store = store.clone();
                    move |_| submit_send(&store, comment, compose_open)
                },
                span { class: "send-bolt", "ϟ" }
                "{send_text}"
            }
        }
    }
}
