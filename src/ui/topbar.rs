//! Top bar: session title, agent status pill, and the Send-to-agent control.

use dioxus::prelude::*;

use crate::model::SessionDoc;
use crate::store::UiMeta;

use super::app::use_store;

#[component]
pub fn TopBar(doc: Signal<SessionDoc>, meta: Signal<UiMeta>) -> Element {
    let store = use_store();
    let mut comment = use_signal(String::new);
    let d = doc.read();
    let m = meta.read();
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
