//! Bottom terminal dock: one Ferroterm tab per launched agent.
//!
//! Tab bodies stay mounted (hidden tabs use display:none) so terminal state and
//! the WebSocket survive tab switches and dock collapse; the fit addon
//! re-measures when a tab becomes visible again.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::terminal::{TerminalManager, TerminalStatus};

const MOUNT_TEMPLATE: &str = include_str!("../../assets/ferroterm/mount.js");

fn mount_js(id: &str, port: u16, token: &str) -> String {
    MOUNT_TEMPLATE
        .replace("__ID__", id)
        .replace("__PORT__", &port.to_string())
        .replace("__TOKEN__", token)
}

fn dispose_js(id: &str) -> String {
    format!(
        "if (window.__nsTerms && window.__nsTerms[\"{id}\"]) {{ window.__nsTerms[\"{id}\"].dispose(); }}"
    )
}

#[component]
pub fn TerminalDock() -> Element {
    let cli = use_context::<crate::cli::Cli>();
    let manager = use_context::<Arc<TerminalManager>>();
    let terminals = use_context::<super::Terminals>().0;
    let panel = use_context::<super::TerminalPanel>();
    let mut open = panel.open;
    let mut focused = panel.focused;
    let mut confirm_close = panel.confirm_close;

    let list = terminals.read().clone();
    if list.is_empty() {
        return rsx! {};
    }
    // A stale focus (closed tab) falls back to the first tab.
    let focused_id = focused
        .read()
        .clone()
        .filter(|id| list.iter().any(|t| &t.id == id))
        .unwrap_or_else(|| list[0].id.clone());

    rsx! {
        div { class: if open() { "term-dock" } else { "term-dock collapsed" },
            div { class: "term-tabs",
                for info in list.iter().cloned() {
                    div {
                        key: "{info.id}",
                        class: if info.id == focused_id { "term-tab active" } else { "term-tab" },
                        onclick: {
                            let id = info.id.clone();
                            move |_| focused.set(Some(id.clone()))
                        },
                        span {
                            class: match info.status {
                                TerminalStatus::Running => "term-dot running",
                                TerminalStatus::Exited(_) => "term-dot exited",
                            },
                            title: match info.status {
                                TerminalStatus::Running => "running".to_owned(),
                                TerminalStatus::Exited(code) => format!("exited ({code})"),
                            },
                            "●"
                        }
                        span {
                            class: "term-tab-name",
                            style: "color: {super::agent_color(&info.id)};",
                            "{info.id}"
                        }
                        button {
                            class: "term-tab-close",
                            aria_label: "Close terminal {info.id}",
                            onclick: {
                                let id = info.id.clone();
                                let status = info.status;
                                let manager = manager.clone();
                                move |event: MouseEvent| {
                                    event.stop_propagation();
                                    if status == TerminalStatus::Running {
                                        confirm_close.set(Some(id.clone()));
                                    } else {
                                        close_tab(&manager, &id, &mut focused);
                                    }
                                }
                            },
                            "×"
                        }
                    }
                }
                button {
                    class: "term-collapse",
                    aria_label: if open() { "Collapse terminal panel" } else { "Expand terminal panel" },
                    onclick: move |_| open.toggle(),
                    if open() { "▾" } else { "▴" }
                }
            }
            div { class: "term-body",
                for info in list.iter() {
                    div {
                        key: "{info.id}",
                        id: "term-{info.id}",
                        class: "term-host",
                        style: if info.id == focused_id { "" } else { "display: none;" },
                        onmounted: {
                            let js = mount_js(&info.id, cli.port, manager.token());
                            move |_| {
                                document::eval(&js);
                            }
                        },
                    }
                }
            }
            if let Some(id) = confirm_close() {
                div { class: "term-confirm-overlay",
                    div { class: "term-confirm", role: "alertdialog",
                        p { "The agent in “{id}” is still running. Stop it and close the tab?" }
                        div { class: "term-confirm-actions",
                            button {
                                class: "btn",
                                onclick: move |_| confirm_close.set(None),
                                "Cancel"
                            }
                            button {
                                class: "btn btn-primary",
                                onclick: {
                                    let manager = manager.clone();
                                    move |_| {
                                        close_tab(&manager, &id, &mut focused);
                                        confirm_close.set(None);
                                    }
                                },
                                "Stop agent"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn close_tab(manager: &Arc<TerminalManager>, id: &str, focused: &mut Signal<Option<String>>) {
    document::eval(&dispose_js(id));
    manager.close(id);
    if focused.read().as_deref() == Some(id) {
        focused.set(None);
    }
}
