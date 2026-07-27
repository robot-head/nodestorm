//! Session-wide decisions panel: every choice in the brainstorm in one place,
//! with prev/next navigation over the ones still waiting on the user.
//!
//! The node panel only ever shows the choices of the node you happened to
//! click, which does not scale past a handful of components. This panel is the
//! [`super::questions_panel::QuestionsPanel`] pattern applied to choices —
//! same shared per-item block, same aggregate listing — plus a cursor that
//! walks the actionable decisions, selecting and centering each owning node on
//! the way so a session can be cleared in one pass.

use std::collections::HashMap;

use dioxus::prelude::*;

use crate::model::{Choice, ChoiceId, ChoiceRef, ChoiceStatus, NodeId, SessionDoc};

use super::choice_panel::{ChoiceBlock, ConsideredTrails};

/// Every choice still waiting on the user and not blocked by a dependency, in
/// document order (nodes as the doc lists them, choices as the node lists
/// them). Locked choices are deliberately absent: stepping onto one would park
/// the cursor on something that cannot be decided.
pub(crate) fn actionable_decisions(doc: &SessionDoc) -> Vec<ChoiceRef> {
    doc.nodes
        .iter()
        .flat_map(|node| {
            node.choices
                .iter()
                .filter(|choice| choice.is_open() && !doc.is_choice_locked(choice))
                .map(|choice| ChoiceRef {
                    node: node.id.clone(),
                    choice: choice.id.clone(),
                })
        })
        .collect()
}

/// Wrapping prev/next over `refs`. With no cursor, a forward step starts at
/// the first decision and a backward step at the last.
pub(crate) fn step_decision(
    refs: &[ChoiceRef],
    current: Option<&ChoiceRef>,
    step: isize,
) -> Option<ChoiceRef> {
    if refs.is_empty() {
        return None;
    }
    let len = refs.len() as isize;
    let index = match current.and_then(|c| refs.iter().position(|r| r == c)) {
        Some(i) => (i as isize + step).rem_euclid(len),
        None if step >= 0 => 0,
        None => len - 1,
    };
    Some(refs[index as usize].clone())
}

/// Document-order position of a choice — `(node index, choice index)` — or
/// `None` if the node or choice no longer exists.
fn position(doc: &SessionDoc, r: &ChoiceRef) -> Option<(usize, usize)> {
    let node = doc.nodes.iter().position(|n| n.id == r.node)?;
    let choice = doc.nodes[node]
        .choices
        .iter()
        .position(|c| c.id == r.choice)?;
    Some((node, choice))
}

/// Where the cursor lands once `current` stops being actionable: the first
/// actionable decision *after* it in document order, wrapping to the first if
/// it was the last. Deciding therefore walks forward through the session
/// rather than snapping back to the top. `None` when nothing is left.
pub(crate) fn resume_after(
    doc: &SessionDoc,
    refs: &[ChoiceRef],
    current: &ChoiceRef,
) -> Option<ChoiceRef> {
    let from = position(doc, current);
    refs.iter()
        .find(|r| match (from, position(doc, r)) {
            (Some(a), Some(b)) => b > a,
            _ => false,
        })
        .or_else(|| refs.first())
        .cloned()
}

/// The collapsed group hiding `node`, if any.
///
/// `layout::compute_view` replaces the members of a collapsed group with a
/// single synthetic `group:<name>` card and emits no rect for the members
/// themselves. Centering on a hidden member would therefore be dropped on the
/// floor by the canvas's zoom effect, leaving navigation looking broken —
/// expand the group first.
pub(crate) fn collapsed_group_of(
    doc: &SessionDoc,
    collapsed: &[String],
    node: &NodeId,
) -> Option<String> {
    let group = doc.node(node)?.group.clone()?;
    collapsed.contains(&group).then_some(group)
}

/// Park the navigation on `next`: reveal its node if a collapsed group hides
/// it, then select and center it. The single entry point every step, jump, and
/// auto-advance goes through.
pub(crate) fn focus_decision(
    store: &crate::store::Store,
    doc: Signal<SessionDoc>,
    mut selected: Signal<Option<NodeId>>,
    mut zoom_target: Signal<Option<NodeId>>,
    node: &NodeId,
) {
    // Scoped so the doc borrow is released before the store mutation below.
    let hidden = collapsed_group_of(&doc.read(), &store.snapshot_meta().collapsed_groups, node);
    if let Some(group) = hidden {
        store.toggle_group_collapsed(&group);
    }
    selected.set(Some(node.clone()));
    zoom_target.set(Some(node.clone()));
}

/// Move the cursor `by` places through `refs`, bringing the canvas along.
/// Shared by the panel's ‹ › controls and the canvas's `]` / `[` keys.
pub(crate) fn step_and_focus(
    store: &crate::store::Store,
    doc: Signal<SessionDoc>,
    refs: &[ChoiceRef],
    mut cursor: Signal<Option<ChoiceRef>>,
    selected: Signal<Option<NodeId>>,
    zoom_target: Signal<Option<NodeId>>,
    by: isize,
) {
    let Some(next) = step_decision(refs, cursor().as_ref(), by) else {
        return;
    };
    focus_decision(store, doc, selected, zoom_target, &next.node);
    cursor.set(Some(next));
}

/// Sibling key for one choice block. Node and choice ids are unrestricted
/// strings, so plain concatenation is ambiguous — `("a-b", "c")` and
/// `("a", "b-c")` would both render `a-b-c` and collide. Prefixing the node
/// id's length makes the split point unambiguous.
fn block_key(node: &NodeId, choice: &ChoiceId) -> String {
    format!("{}:{}:{}", node.as_str().len(), node, choice)
}

/// The four buckets the panel lists, in the order it lists them.
fn bucket(doc: &SessionDoc, choice: &Choice) -> usize {
    match choice.status {
        ChoiceStatus::Open if doc.is_choice_locked(choice) => 1,
        ChoiceStatus::Open => 0,
        ChoiceStatus::Decided => 2,
        ChoiceStatus::Dismissed => 3,
    }
}

/// Right-hand panel listing every choice in the session — actionable first,
/// then dependency-blocked, decided, and skipped — each decidable in place.
#[component]
pub fn DecisionsPanel(
    doc: Signal<SessionDoc>,
    selected: Signal<Option<NodeId>>,
    hovered_affects: Signal<Vec<NodeId>>,
    on_close: EventHandler<()>,
) -> Element {
    let considered: Signal<ConsideredTrails> = use_signal(HashMap::new);
    let mut cursor = use_context::<super::DecisionNav>().cursor;
    let zoom_target = use_context::<super::ZoomTarget>().0;
    let store = super::app::use_store();

    // Auto-advance: when the parked decision stops being actionable — the user
    // just decided or skipped it, or the agent resolved it from its side —
    // walk to the next one instead of stranding the cursor. Only fires when
    // the target has genuinely left the list, so it never fights the user.
    use_effect({
        let store = store.clone();
        move || {
            let next = {
                let d = doc.read();
                let refs = actionable_decisions(&d);
                let Some(current) = cursor() else { return };
                if refs.contains(&current) {
                    return;
                }
                resume_after(&d, &refs, &current)
            };
            if let Some(next) = &next {
                focus_decision(&store, doc, selected, zoom_target, &next.node);
            }
            cursor.set(next);
        }
    });

    // Snapshot everything the render needs, then drop the doc borrow.
    let (buckets, refs, has_any) = {
        let d = doc.read();
        let mut buckets: [Vec<(NodeId, Choice)>; 4] = Default::default();
        for node in &d.nodes {
            for choice in &node.choices {
                buckets[bucket(&d, choice)].push((node.id.clone(), choice.clone()));
            }
        }
        let has_any = buckets.iter().any(|b| !b.is_empty());
        (buckets, actionable_decisions(&d), has_any)
    };

    let total = refs.len();
    let at = cursor().and_then(|c| refs.iter().position(|r| *r == c));
    let count_text = match (total, at) {
        (0, _) => "none open".to_owned(),
        (n, None) => format!("{n} open"),
        (n, Some(i)) => format!("{} of {n}", i + 1),
    };

    rsx! {
        aside { class: "panel decisions-panel",
            div { class: "panel-head",
                h2 { "Decisions" }
                span { class: "panel-nav",
                    button {
                        class: "ctl-btn",
                        aria_label: "Previous open decision",
                        title: "Previous open decision ([)",
                        disabled: total == 0,
                        onclick: {
                            let refs = refs.clone();
                            let store = store.clone();
                            move |_| {
                                step_and_focus(
                                    &store, doc, &refs, cursor, selected, zoom_target, -1,
                                );
                            }
                        },
                        "‹"
                    }
                    span { class: "panel-nav-count", "{count_text}" }
                    button {
                        class: "ctl-btn",
                        aria_label: "Next open decision",
                        title: "Next open decision (])",
                        disabled: total == 0,
                        onclick: {
                            let refs = refs.clone();
                            let store = store.clone();
                            move |_| {
                                step_and_focus(
                                    &store, doc, &refs, cursor, selected, zoom_target, 1,
                                );
                            }
                        },
                        "›"
                    }
                }
                button {
                    class: "ctl-btn",
                    title: "Close",
                    onclick: move |_| on_close.call(()),
                    "✕"
                }
            }
            if !has_any {
                p { class: "panel-desc", "The agent hasn't proposed any choices yet." }
            }
            for (heading, entries) in ["Open", "Waiting on dependencies", "Decided", "Skipped"]
                .into_iter()
                .zip(buckets)
                .filter(|(_, entries)| !entries.is_empty())
            {
                h3 { class: "decisions-section", key: "{heading}", "{heading}" }
                for (node_id, choice) in entries {
                    ChoiceBlock {
                        key: "{block_key(&node_id, &choice.id)}",
                        node_id,
                        choice,
                        doc,
                        considered,
                        hovered_affects,
                        selected: Some(selected),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChoiceId, ChoiceOption, ElementStatus, Node, NodeKind, OptionId, Origin};
    use yare::parameterized;

    fn choice(id: &str, status: ChoiceStatus, depends_on: Vec<ChoiceRef>) -> Choice {
        Choice {
            id: ChoiceId::from(id),
            prompt: format!("{id}?"),
            rationale: None,
            options: vec![ChoiceOption {
                id: OptionId::from("a"),
                label: "A".into(),
                summary: String::new(),
                pros: vec![],
                cons: vec![],
                recommended: false,
                affects: vec![],
            }],
            selected: None,
            status,
            depends_on,
            needs_review: false,
            reopen: false,
        }
    }

    fn node(id: &str, choices: Vec<Choice>) -> Node {
        Node {
            id: NodeId::from(id),
            label: id.to_uppercase(),
            kind: NodeKind::Component,
            description: String::new(),
            status: ElementStatus::Existing,
            build: None,
            group: None,
            lane: None,
            choices,
            notes: vec![],
            agent: None,
            position: None,
            origin: Origin::Agent,
        }
    }

    fn cref(node: &str, choice: &str) -> ChoiceRef {
        ChoiceRef {
            node: NodeId::from(node),
            choice: ChoiceId::from(choice),
        }
    }

    /// api: open `a`, decided `b`; queue: dismissed `c`, open `d` blocked on
    /// api/`a`; store: open `e`. Actionable is therefore [api/a, store/e].
    fn doc() -> SessionDoc {
        SessionDoc {
            nodes: vec![
                node(
                    "api",
                    vec![
                        choice("a", ChoiceStatus::Open, vec![]),
                        choice("b", ChoiceStatus::Decided, vec![]),
                    ],
                ),
                node(
                    "queue",
                    vec![
                        choice("c", ChoiceStatus::Dismissed, vec![]),
                        choice("d", ChoiceStatus::Open, vec![cref("api", "a")]),
                    ],
                ),
                node("store", vec![choice("e", ChoiceStatus::Open, vec![])]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn actionable_skips_resolved_and_locked_choices_in_document_order() {
        assert2::assert!(
            actionable_decisions(&doc()) == vec![cref("api", "a"), cref("store", "e")]
        );
    }

    #[test]
    fn actionable_unlocks_a_dependent_once_its_parent_resolves() {
        let mut d = doc();
        d.nodes[0].choices[0].status = ChoiceStatus::Decided;

        // api/a leaves the list and queue/d joins it, still in document order.
        assert2::assert!(actionable_decisions(&d) == vec![cref("queue", "d"), cref("store", "e")]);
    }

    #[test]
    fn actionable_is_empty_for_a_doc_with_nothing_to_decide() {
        assert2::assert!(actionable_decisions(&SessionDoc::default()).is_empty());
    }

    #[parameterized(
        forward_from_none = { None, 1, Some(cref("api", "a")) },
        backward_from_none = { None, -1, Some(cref("store", "e")) },
        forward = { Some(cref("api", "a")), 1, Some(cref("store", "e")) },
        forward_wraps = { Some(cref("store", "e")), 1, Some(cref("api", "a")) },
        backward = { Some(cref("store", "e")), -1, Some(cref("api", "a")) },
        backward_wraps = { Some(cref("api", "a")), -1, Some(cref("store", "e")) },
        // A cursor that has left the list (decided, or locked) restarts.
        stale_cursor_restarts = { Some(cref("api", "b")), 1, Some(cref("api", "a")) },
    )]
    fn step_walks_the_actionable_list(
        current: Option<ChoiceRef>,
        by: isize,
        expected: Option<ChoiceRef>,
    ) {
        let refs = actionable_decisions(&doc());
        assert2::assert!(step_decision(&refs, current.as_ref(), by) == expected);
    }

    #[test]
    fn step_on_an_empty_list_has_nowhere_to_go() {
        assert2::assert!(step_decision(&[], None, 1).is_none());
        assert2::assert!(step_decision(&[], Some(&cref("api", "a")), -1).is_none());
    }

    #[test]
    fn step_on_a_single_decision_stays_put() {
        let refs = vec![cref("api", "a")];
        assert2::assert!(
            step_decision(&refs, Some(&cref("api", "a")), 1) == Some(cref("api", "a"))
        );
        assert2::assert!(
            step_decision(&refs, Some(&cref("api", "a")), -1) == Some(cref("api", "a"))
        );
    }

    #[test]
    fn resume_walks_forward_from_the_decision_just_resolved() {
        // Decide api/a: the cursor should move on to the newly-unlocked
        // queue/d, which follows it in document order — not back to the top.
        let mut d = doc();
        d.nodes[0].choices[0].status = ChoiceStatus::Decided;
        let refs = actionable_decisions(&d);

        assert2::assert!(resume_after(&d, &refs, &cref("api", "a")) == Some(cref("queue", "d")));
    }

    #[test]
    fn resume_wraps_when_the_resolved_decision_was_the_last() {
        let mut d = doc();
        d.nodes[2].choices[0].status = ChoiceStatus::Decided;
        let refs = actionable_decisions(&d);

        assert2::assert!(resume_after(&d, &refs, &cref("store", "e")) == Some(cref("api", "a")));
    }

    #[test]
    fn resume_yields_nothing_once_every_decision_is_resolved() {
        let mut d = doc();
        for node in &mut d.nodes {
            for choice in &mut node.choices {
                choice.status = ChoiceStatus::Decided;
            }
        }
        let refs = actionable_decisions(&d);

        assert2::assert!(refs.is_empty());
        assert2::assert!(resume_after(&d, &refs, &cref("api", "a")).is_none());
    }

    #[test]
    fn resume_falls_back_to_the_first_when_the_old_position_is_gone() {
        // The node carrying the cursor was deleted; position() returns None,
        // so there is no "after" to walk to — restart at the top.
        let mut d = doc();
        d.nodes.remove(0);
        let refs = actionable_decisions(&d);

        assert2::assert!(resume_after(&d, &refs, &cref("api", "a")) == refs.first().cloned());
    }

    #[test]
    fn a_collapsed_group_hides_its_members_from_centering() {
        // Members of a collapsed group have no layout rect, so navigation must
        // expand the group before asking the canvas to center on one.
        let mut d = doc();
        d.nodes[0].group = Some("Platform".into());

        assert2::assert!(
            collapsed_group_of(&d, &["Platform".to_owned()], &NodeId::from("api"))
                == Some("Platform".to_owned())
        );
        // Expanded, ungrouped, and unknown nodes need no reveal.
        assert2::assert!(collapsed_group_of(&d, &[], &NodeId::from("api")).is_none());
        assert2::assert!(
            collapsed_group_of(&d, &["Platform".to_owned()], &NodeId::from("store")).is_none()
        );
        assert2::assert!(
            collapsed_group_of(&d, &["Platform".to_owned()], &NodeId::from("ghost")).is_none()
        );
    }

    #[test]
    fn sibling_keys_stay_distinct_when_ids_contain_the_separator() {
        // ("a-b", "c") and ("a", "b-c") both concatenate to "a-b-c"; the
        // length prefix keeps them apart. Duplicate sibling keys make Dioxus
        // reconcile the wrong block and retain the other choice's handlers.
        assert2::assert!(
            block_key(&NodeId::from("a-b"), &ChoiceId::from("c"))
                != block_key(&NodeId::from("a"), &ChoiceId::from("b-c"))
        );
        // Colons are just as legal inside an id, and must not collide either.
        assert2::assert!(
            block_key(&NodeId::from("a:b"), &ChoiceId::from("c"))
                != block_key(&NodeId::from("a"), &ChoiceId::from("b:c"))
        );
    }

    #[parameterized(
        open = { ChoiceStatus::Open, vec![], 0 },
        locked = { ChoiceStatus::Open, vec![cref("api", "a")], 1 },
        decided = { ChoiceStatus::Decided, vec![], 2 },
        dismissed = { ChoiceStatus::Dismissed, vec![], 3 },
    )]
    fn choices_land_in_the_section_matching_their_state(
        status: ChoiceStatus,
        depends_on: Vec<ChoiceRef>,
        expected: usize,
    ) {
        assert2::assert!(bucket(&doc(), &choice("x", status, depends_on)) == expected);
    }
}
