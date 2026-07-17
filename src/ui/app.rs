//! Root component: bridges the shared [`Store`] into Dioxus signals and
//! composes the screen.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::cli::Cli;
use crate::layout::{self, Layout};
use crate::model::NodeId;
use crate::store::Store;

use super::activity::ActivityFeed;
use super::canvas::Canvas;
use super::choice_panel::ChoicePanel;
use super::topbar::TopBar;

#[component]
pub fn App() -> Element {
    let cli = use_context::<Cli>();
    let store = use_context::<Arc<Store>>();
    let mut doc = use_signal(|| store.snapshot_doc());
    let mut meta = use_signal(|| store.snapshot_meta());

    // Store → UI bridge: whenever any mutation bumps the revision, re-snapshot.
    // `watch` is executor-agnostic, so awaiting it on the Dioxus runtime is fine.
    use_future({
        let store = store.clone();
        move || {
            let store = store.clone();
            async move {
                let mut rev = store.subscribe();
                loop {
                    if rev.changed().await.is_err() {
                        break;
                    }
                    doc.set(store.snapshot_doc());
                    meta.set(store.snapshot_meta());
                }
            }
        }
    });

    let layout: Memo<Layout> = use_memo(move || layout::compute(&doc.read()));
    let selected: Signal<Option<NodeId>> = use_signal(|| None);
    let hovered_affects: Signal<Vec<NodeId>> = use_signal(Vec::new);
    // Cross-component signals (topbar, panel, and canvas all touch them),
    // wrapped in newtypes because context is type-keyed.
    use_context_provider(|| super::ConnectFrom(Signal::new(None)));
    use_context_provider(|| super::ZoomTarget(Signal::new(None)));
    use_context_provider(|| super::SearchQuery(Signal::new(String::new())));
    let mcp_url = cli.mcp_url();
    let has_nodes = !doc.read().nodes.is_empty();
    let selected_node = selected
        .read()
        .as_ref()
        .and_then(|id| doc.read().node(id).cloned());

    rsx! {
        document::Style { {include_str!("../../assets/main.css")} }
        div { class: "app",
            TopBar { doc, meta, selected }
            div { class: "main",
                if has_nodes {
                    Canvas { doc, layout, selected, hovered_affects }
                    ActivityFeed { meta }
                    if let Some(node) = selected_node {
                        // Keyed so switching nodes remounts the panel and its
                        // edit-form drafts start from the new node's content.
                        ChoicePanel { key: "{node.id}", node, doc, selected, hovered_affects }
                    }
                } else {
                    div { class: "empty-state",
                        h1 { "nodestorm" }
                        p { "Waiting for an agent to connect." }
                        code { "claude mcp add --transport http nodestorm {mcp_url}" }
                    }
                }
            }
        }
    }
}

/// Convenience for child components: the store from context.
pub fn use_store() -> Arc<Store> {
    use_context::<Arc<Store>>()
}
