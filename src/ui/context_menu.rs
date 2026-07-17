//! Floating right-click menu. Plain HTML (no native menus in the WebView),
//! which also keeps every item visible to UI Automation for the E2E script.

use dioxus::prelude::*;

use crate::model::{EdgeKind, NodeId};

/// What was right-clicked.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuTarget {
    /// A node card (with its group, when it has one, for Collapse group).
    Node(NodeId, Option<String>),
    Edge(NodeId, NodeId, EdgeKind),
    Background {
        plane_x: f64,
        plane_y: f64,
    },
}

/// What the user picked; the canvas owns the store calls.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    SelectNode(NodeId),
    Connect(NodeId),
    DeleteNode(NodeId),
    DeleteEdge(NodeId, NodeId, EdgeKind),
    AddHere { plane_x: f64, plane_y: f64 },
    ToggleGroup(String),
}

#[component]
pub fn ContextMenu(
    x: f64,
    y: f64,
    target: MenuTarget,
    on_action: EventHandler<MenuAction>,
    on_close: EventHandler<()>,
) -> Element {
    let items: Vec<(&'static str, MenuAction)> = match &target {
        MenuTarget::Node(id, group) => {
            let mut items = vec![
                ("Rename…", MenuAction::SelectNode(id.clone())),
                ("Connect →", MenuAction::Connect(id.clone())),
                ("Delete", MenuAction::DeleteNode(id.clone())),
            ];
            if let Some(g) = group {
                items.push(("Collapse group", MenuAction::ToggleGroup(g.clone())));
            }
            items
        }
        MenuTarget::Edge(from, to, kind) => vec![(
            "Delete edge",
            MenuAction::DeleteEdge(from.clone(), to.clone(), *kind),
        )],
        MenuTarget::Background { plane_x, plane_y } => vec![(
            "Add component here",
            MenuAction::AddHere {
                plane_x: *plane_x,
                plane_y: *plane_y,
            },
        )],
    };

    rsx! {
        // Full-viewport transparent catcher: any click outside closes.
        div {
            class: "menu-catcher",
            onclick: move |_| on_close.call(()),
            oncontextmenu: move |ev| {
                ev.prevent_default();
                on_close.call(());
            },
        }
        div { class: "context-menu", style: "left: {x}px; top: {y}px;",
            for (label, action) in items {
                button {
                    key: "{label}",
                    onclick: move |_| on_action.call(action.clone()),
                    "{label}"
                }
            }
        }
    }
}
