//! Root component: bridges the *active session's* [`Store`] into Dioxus
//! signals and composes the screen.
//!
//! The bridge selects over two watch channels: the active store's revision
//! (re-snapshot the doc) and the session manager's generation (the list or
//! the active session changed — reset view state and re-subscribe to the
//! new active store).

use std::sync::Arc;

use dioxus::prelude::*;

use crate::cli::Cli;
use crate::layout::{self, Layout};
use crate::model::NodeId;
use crate::sessions::Sessions;
use crate::store::ToastLevel;

use super::activity::ActivityFeed;
use super::agent_launcher::AgentLauncher;
use super::canvas::Canvas;
use super::choice_panel::ChoicePanel;
use super::diff_panel::DiffPanel;
use super::questions_panel::QuestionsPanel;
use super::queued_changes::QueuedChangesPanel;
use super::timeline::Timeline;
use super::topbar::TopBar;

#[component]
pub fn App() -> Element {
    let cli = use_context::<Cli>();
    let sessions = use_context::<Arc<Sessions>>();
    let (initial_name, initial_store) = sessions
        .resolve_named(None)
        .expect("active session always exists");
    let initial_doc = initial_store.snapshot_doc();
    let initial_meta = initial_store.snapshot_meta();
    let mut active_store =
        use_context_provider(move || super::ActiveStore(Signal::new(initial_store))).0;
    let mut doc = use_signal(move || initial_doc);
    let mut meta = use_signal(move || initial_meta);
    let mut session_name = use_signal(move || initial_name);
    let mut connections = use_signal(|| sessions.connections());

    // Cross-component signals (topbar, panel, and canvas all touch them),
    // wrapped in newtypes because context is type-keyed.
    let mut connect_from = use_context_provider(|| super::ConnectFrom(Signal::new(None))).0;
    use_context_provider(|| super::ZoomTarget(Signal::new(None)));
    let mut search = use_context_provider(|| super::SearchQuery(Signal::new(String::new()))).0;
    let mut compare_with = use_context_provider(|| super::CompareWith(Signal::new(None))).0;
    let mut record_diff = use_context_provider(|| super::RecordDiff(Signal::new(None))).0;
    use_context_provider(|| super::MessageComposer {
        comment: Signal::new(String::new()),
        open: Signal::new(false),
    });
    let mut launcher_open = use_context_provider(|| super::AgentLauncherOpen(Signal::new(false))).0;

    // Theme preference: seeded from the file loaded in launch(); the CSS
    // reacts through data-theme/data-mode below, the native title bar
    // through set_theme (Auto = follow the OS).
    let initial_prefs = use_context::<crate::prefs::Preferences>();
    let theme_prefs = use_context_provider(move || super::ThemePref(Signal::new(initial_prefs))).0;
    let window = dioxus::desktop::use_window();
    use_effect(move || {
        window
            .window
            .set_theme(super::tao_theme(theme_prefs.read().mode));
    });

    let layout: Memo<Layout> = use_memo(move || {
        let collapsed: std::collections::BTreeSet<String> =
            meta.read().collapsed_groups.iter().cloned().collect();
        layout::compute_collapsed(&doc.read(), &collapsed)
    });
    let mut selected: Signal<Option<NodeId>> = use_signal(|| None);
    let hovered_affects: Signal<Vec<NodeId>> = use_signal(Vec::new);
    let mut timeline_open: Signal<bool> = use_signal(|| false);
    let mut queued_changes_open: Signal<bool> = use_signal(|| false);
    let mut questions_open: Signal<bool> = use_signal(|| false);

    // Connection changes are independent of active-session generation so
    // transport churn never resets canvas selection or panel state.
    use_future({
        let sessions = sessions.clone();
        move || {
            let sessions = sessions.clone();
            async move {
                let mut changes = sessions.subscribe_connections();
                connections.set(sessions.connections());
                while changes.changed().await.is_ok() {
                    connections.set(sessions.connections());
                }
            }
        }
    });

    // Store → UI bridge: revision changes re-snapshot; generation changes
    // re-subscribe to the new active store and reset per-session view state.
    use_future({
        let sessions = sessions.clone();
        move || {
            let sessions = sessions.clone();
            async move {
                let mut generation = sessions.subscribe_generation();
                loop {
                    let (name, store) = sessions
                        .resolve_named(None)
                        .expect("active session always exists");
                    doc.set(store.snapshot_doc());
                    meta.set(store.snapshot_meta());
                    session_name.set(name);
                    active_store.set(store.clone());
                    let mut rev = store.subscribe();
                    loop {
                        tokio::select! {
                            changed = rev.changed() => {
                                if changed.is_err() {
                                    return;
                                }
                                doc.set(store.snapshot_doc());
                                meta.set(store.snapshot_meta());
                            }
                            changed = generation.changed() => {
                                if changed.is_err() {
                                    return;
                                }
                                selected.set(None);
                                connect_from.set(None);
                                search.set(String::new());
                                compare_with.set(None);
                                record_diff.set(None);
                                queued_changes_open.set(false);
                                questions_open.set(false);
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    let mcp_url = cli.mcp_url();
    let has_nodes = !doc.read().nodes.is_empty();
    let selected_node = selected
        .read()
        .as_ref()
        .and_then(|id| doc.read().node(id).cloned());
    let compare_text = compare_with().and_then(|other| {
        sessions.get(&other).map(|store| {
            crate::diff::diff_docs(
                &session_name.read(),
                &doc.read(),
                &other,
                &store.snapshot_doc(),
            )
        })
    });

    rsx! {
        document::Style { {include_str!("../../assets/fonts.css")} }
        document::Style { {include_str!("../../assets/main.css")} }
        div {
            class: "app",
            "data-theme": "{theme_prefs.read().theme}",
            "data-mode": "{theme_prefs.read().mode.as_str()}",
            TopBar { doc, meta, connections, selected, session_name, timeline_open, queued_changes_open, questions_open }
            div { class: "main",
                if has_nodes {
                    Canvas { doc, layout, selected, hovered_affects }
                    ActivityFeed { meta }
                } else {
                    div { class: "empty-state",
                        span { class: "empty-bolt", "ϟ" }
                        h1 { "nodestorm" }
                        p { "Waiting for an agent to connect." }
                        div { class: "empty-actions",
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| launcher_open.set(true),
                                "Start an agentic session"
                            }
                            button {
                                class: "empty-cmd",
                                title: "Copy the connect command",
                                onclick: {
                                    let sessions = sessions.clone();
                                    let cmd = format!(
                                        "claude mcp add --transport http nodestorm {mcp_url}"
                                    );
                                    move |_| {
                                        super::copy_to_clipboard(
                                            &sessions.active_store(),
                                            cmd.clone(),
                                            "copied the connect command",
                                        );
                                    }
                                },
                                code { "claude mcp add --transport http nodestorm {mcp_url}" }
                                span { class: "empty-copy", "⧉" }
                            }
                        }
                    }
                }
                if let Some(node) = selected_node {
                    // Keyed so switching nodes remounts the panel and its
                    // edit-form drafts start from the new node's content.
                    // Selection takes the right-panel slot over Timeline.
                    ChoicePanel { key: "{node.id}", node, doc, selected, hovered_affects }
                } else if let Some(text) = compare_text {
                    DiffPanel { text, on_close: move |()| compare_with.set(None) }
                } else if let Some(text) = record_diff() {
                    DiffPanel { text, on_close: move |()| record_diff.set(None) }
                } else if timeline_open() {
                    Timeline { doc, meta, on_close: move |()| timeline_open.set(false) }
                } else if queued_changes_open() {
                    QueuedChangesPanel {
                        doc,
                        meta,
                        selected,
                        on_close: move |()| queued_changes_open.set(false),
                    }
                } else if questions_open() {
                    QuestionsPanel { doc, on_close: move |()| questions_open.set(false) }
                }
            }
            if launcher_open() {
                AgentLauncher {}
            }
            if let Some(toast) = meta.read().toast.clone() {
                div {
                    class: match toast.level {
                        ToastLevel::Warning => "delivery-toast delivery-toast-warning",
                        ToastLevel::Error => "delivery-toast delivery-toast-error",
                    },
                    role: "alert",
                    span { "{toast.message}" }
                    button {
                        aria_label: "Dismiss error",
                        onclick: {
                            let store = active_store.read().clone();
                            move |_| store.dismiss_toast()
                        },
                        "×"
                    }
                }
            }
        }
    }
}

/// Convenience for child components: the store backing the rendered snapshots.
pub fn use_store() -> Arc<crate::store::Store> {
    use_context::<super::ActiveStore>().0.read().clone()
}
