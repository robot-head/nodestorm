//! The ⋯ ("More") menu: a permanent home for the Export and Theme
//! dropdowns as accordion sections, so the top bar stays short at every
//! window width. Also hosts the narrow-width Undo/Redo fallback rows
//! (hidden by CSS above the fold breakpoint).

use std::sync::Arc;

use dioxus::prelude::*;

use crate::cli::Cli;
use crate::store::Store;

use super::app::use_store;
use super::theme_menu::ThemeRows;

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

#[derive(Clone, Copy, PartialEq)]
enum Section {
    None,
    Export,
    Theme,
}

#[component]
pub fn MoreMenu(has_nodes: bool, suggested_name: String) -> Element {
    let store = use_store();
    let cli = use_context::<Cli>();
    let sessions = use_context::<Arc<crate::sessions::Sessions>>();
    let mut record_diff = use_context::<super::RecordDiff>().0;
    let mut open = use_signal(|| false);
    let mut section = use_signal(|| Section::None);

    rsx! {
        div { class: "export-menu",
            button {
                class: "btn pod-icon",
                aria_label: "More",
                title: "Export, themes, and more",
                onclick: move |_| {
                    section.set(Section::None);
                    open.toggle();
                },
                "⋯"
            }
            if open() {
                div {
                    class: "menu-catcher",
                    onclick: move |_| open.set(false),
                }
                div { class: "export-dropdown more-dropdown",
                    button {
                        class: "row-undo",
                        onclick: {
                            let store = store.clone();
                            move |_| {
                                store.undo();
                            }
                        },
                        "↶ Undo"
                    }
                    button {
                        class: "row-redo",
                        onclick: {
                            let store = store.clone();
                            move |_| {
                                store.redo();
                            }
                        },
                        "↷ Redo"
                    }
                    button {
                        class: "accordion-head",
                        disabled: !has_nodes,
                        onclick: move |_| {
                            section.set(if section() == Section::Export {
                                Section::None
                            } else {
                                Section::Export
                            });
                        },
                        "Export ▾"
                    }
                    if section() == Section::Export {
                        div { class: "accordion-body",
                            button {
                                title: "Write the Markdown record next to the session file",
                                onclick: {
                                    let store = store.clone();
                                    let cli = cli.clone();
                                    move |_| {
                                        open.set(false);
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
                                        open.set(false);
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
                                        open.set(false);
                                        super::copy_to_clipboard(
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
                                        open.set(false);
                                        let text = format!(
                                            "```mermaid\n{}```\n",
                                            store.read(|s| crate::export::render_mermaid(&s.doc)),
                                        );
                                        super::copy_to_clipboard(
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
                                        open.set(false);
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
                            button {
                                title: "Compare this session against a previously exported record file",
                                disabled: !has_nodes,
                                onclick: {
                                    let store = store.clone();
                                    let sessions = sessions.clone();
                                    move |_| {
                                        open.set(false);
                                        let Some(path) = rfd::FileDialog::new()
                                            .add_filter("Markdown", &["md"])
                                            .pick_file()
                                        else {
                                            return;
                                        };
                                        let text = match std::fs::read_to_string(&path) {
                                            Ok(t) => t,
                                            Err(err) => {
                                                store.record_export_failed(
                                                    &format!("cannot read record: {err}"),
                                                );
                                                return;
                                            }
                                        };
                                        let record_name = path
                                            .file_name()
                                            .map_or_else(
                                                || "record".to_owned(),
                                                |n| n.to_string_lossy().into_owned(),
                                            );
                                        let doc = store.snapshot_doc();
                                        let result = crate::diff::diff_doc_vs_record(
                                            &record_name,
                                            &text,
                                            &sessions.active_name(),
                                            &doc,
                                        );
                                        record_diff.set(Some(match result {
                                            Ok(diff) => diff,
                                            Err(err) => format!("# Compare with record\n\n_{err}_\n"),
                                        }));
                                    }
                                },
                                "Compare with record file…"
                            }
                        }
                    }
                    button {
                        class: "accordion-head",
                        onclick: move |_| {
                            section.set(if section() == Section::Theme {
                                Section::None
                            } else {
                                Section::Theme
                            });
                        },
                        "Theme ▾"
                    }
                    if section() == Section::Theme {
                        div { class: "accordion-body theme-dropdown",
                            ThemeRows { on_pick: move |()| open.set(false) }
                        }
                    }
                }
            }
        }
    }
}
