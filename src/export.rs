//! Pure Markdown + Mermaid export of a brainstorm session.
//!
//! No IO and no rmcp/Dioxus contact: plain model data in, `String` out, so
//! the renderers stay unit-testable from every caller (MCP tool, UI button).
//! Output is deterministic — nodes, edges, and groups are emitted in document
//! order and nothing iterates a `HashMap` on an output path — so identical
//! state renders byte-identically (the same rule `layout.rs` follows).

use std::collections::HashMap;
use std::fmt::Write as _;

use chrono::{DateTime, Utc};

use crate::model::{
    AnnotationKind, BuildStatus, Choice, ChoiceId, ChoiceOption, ChoiceStatus, DecisionEvent,
    DecisionKind, EdgeKind, ElementStatus, Node, NodeId, NodeKind, OptionId, Question, SessionDoc,
};

/// Render the whole session as a Markdown decision record: architecture
/// (Mermaid diagram + component inventory), decisions made (with pros/cons
/// and the user's `considered` exploration trail when the decision log has
/// one), dismissed choices, and open questions. Empty sections are omitted.
///
/// `exported_at` is injected by the caller (pass `Utc::now()`) so output is
/// deterministic under test; it is rendered date-only to keep re-exports
/// committed to a repo low-churn.
pub fn render_markdown(
    doc: &SessionDoc,
    log: &[DecisionEvent],
    exported_at: DateTime<Utc>,
) -> String {
    let mut out = String::new();
    let title = if doc.title.is_empty() {
        "Untitled brainstorm"
    } else {
        doc.title.as_str()
    };
    let _ = writeln!(out, "# {title}\n");
    // Deliberately no doc revision here: every store mutation bumps it —
    // including recording the export itself — so embedding it would make
    // back-to-back exports of an unchanged graph differ byte-wise.
    let _ = writeln!(
        out,
        "_Decision record exported from a nodestorm brainstorm on {}._\n",
        exported_at.format("%Y-%m-%d"),
    );
    if doc.nodes.is_empty() {
        out.push_str("_Empty session — nothing on the canvas yet._\n");
        // Still embed the snapshot so an early export is diff-able too.
        append_record_snapshot(&mut out, doc);
        return out;
    }

    let (mut decided, mut dismissed, mut open) = (0usize, 0usize, 0usize);
    for choice in doc.nodes.iter().flat_map(|n| &n.choices) {
        match choice.status {
            ChoiceStatus::Decided => decided += 1,
            ChoiceStatus::Dismissed => dismissed += 1,
            ChoiceStatus::Open => open += 1,
        }
    }
    let _ = writeln!(
        out,
        "**{} components · {decided} decided · {dismissed} dismissed · {open} open**\n",
        doc.nodes.len()
    );

    out.push_str("## Architecture\n\n");
    let _ = writeln!(out, "```mermaid\n{}```\n", render_mermaid(doc));
    out.push_str(
        "_Color = status: gray existing · blue proposed · amber modified · purple affected · \
         red dashed removed._\n\n",
    );
    render_components(&mut out, doc);

    if doc.nodes.iter().any(|n| n.build.is_some()) {
        out.push_str("## Implementation status\n\n");
        let tracked = doc.nodes.iter().filter(|n| n.build.is_some()).count();
        let shipped = doc
            .nodes
            .iter()
            .filter(|n| n.build.is_some_and(BuildStatus::is_shipped))
            .count();
        let _ = writeln!(out, "**{shipped}/{tracked} shipped**\n");
        for node in &doc.nodes {
            if let Some(build) = node.build {
                let mark = if build.is_shipped() { "x" } else { " " };
                let _ = writeln!(out, "- [{mark}] **{}** — {}", node.label, build.name());
            }
        }
        out.push('\n');
    }

    if decided > 0 {
        out.push_str("## Decisions\n\n");
        for node in &doc.nodes {
            for choice in &node.choices {
                if choice.status == ChoiceStatus::Decided {
                    render_decision(&mut out, node, choice, log);
                }
            }
        }
    }

    if dismissed > 0 {
        out.push_str("## Dismissed decisions\n\n");
        for node in &doc.nodes {
            for choice in &node.choices {
                if choice.status == ChoiceStatus::Dismissed {
                    render_dismissal(&mut out, node, choice, log);
                }
            }
        }
        out.push('\n');
    }

    // Free-form questions the agent posed: answered ones sit with the
    // decisions; unanswered ones join the open questions below.
    let answered_q = doc.questions.iter().filter(|q| q.is_answered()).count();
    let open_q = doc.questions.len() - answered_q;
    if answered_q > 0 {
        out.push_str("## Questions answered\n\n");
        for question in doc.questions.iter().filter(|q| q.is_answered()) {
            render_answered_question(&mut out, doc, question);
        }
        out.push('\n');
    }

    if open > 0 || open_q > 0 {
        out.push_str("## Open questions\n\n");
        for node in &doc.nodes {
            for choice in &node.choices {
                if choice.status == ChoiceStatus::Open {
                    render_open_question(&mut out, doc, node, choice);
                }
            }
        }
        for question in doc.questions.iter().filter(|q| !q.is_answered()) {
            render_unanswered_question(&mut out, doc, question);
        }
        out.push('\n');
    }

    if !doc.annotations.is_empty() {
        out.push_str("## Annotations\n\n");
        for annotation in &doc.annotations {
            let _ = write!(out, "- {}", annotation_kind_name(annotation.kind));
            if annotation.text.trim().is_empty() {
                let _ = writeln!(out, " at ({:.0}, {:.0})", annotation.x, annotation.y);
            } else {
                let _ = writeln!(out, ": {}", annotation.text.trim());
            }
        }
        out.push('\n');
    }

    if !log.is_empty() {
        out.push_str("## Session log\n\n");
        for event in log {
            let _ = writeln!(
                out,
                "- {} — {}",
                event.at.format("%Y-%m-%d %H:%M"),
                describe_event(doc, event)
            );
        }
    }

    append_record_snapshot(&mut out, doc);
    out
}

fn annotation_kind_name(kind: AnnotationKind) -> &'static str {
    match kind {
        AnnotationKind::Note => "note",
        AnnotationKind::Arrow => "arrow",
        AnnotationKind::Region => "region",
    }
}

/// Marker bracketing the embedded machine-readable snapshot (an HTML comment,
/// so it is invisible in rendered Markdown but lets a later session diff
/// against this record via [`parse_record`]).
const RECORD_SNAPSHOT_BEGIN: &str = "<!-- nodestorm-record v1";
const RECORD_SNAPSHOT_END: &str = "-->";

/// A normalized copy of `doc` for embedding: volatile view state (revision,
/// card positions) is dropped so re-exports of unchanged content stay
/// byte-identical (the diff ignores those fields anyway).
fn snapshot_doc(doc: &SessionDoc) -> SessionDoc {
    let mut snap = doc.clone();
    snap.revision = 0;
    for node in &mut snap.nodes {
        node.position = None;
    }
    snap
}

/// Append the hidden, diff-able snapshot of the graph to the record.
///
/// The JSON lives inside an HTML comment, so any `>` it carries (only possible
/// inside string values — e.g. a note or annotation containing `-->`) is
/// escaped to its `>` JSON form. The payload then never contains `-->` and
/// cannot close the comment early; `serde_json` decodes the escape back on
/// parse, so the round-trip stays lossless.
fn append_record_snapshot(out: &mut String, doc: &SessionDoc) {
    if let Ok(json) = serde_json::to_string(&snapshot_doc(doc)) {
        let safe = json.replace('>', "\\u003e");
        let _ = write!(
            out,
            "\n{RECORD_SNAPSHOT_BEGIN}\n{safe}\n{RECORD_SNAPSHOT_END}\n"
        );
    }
}

/// Recover the graph snapshot embedded in an exported record's text, if any.
/// `None` when the file predates snapshots or was hand-edited past recognition.
///
/// Uses the *last* begin marker so user text that happens to contain the marker
/// string (rendered earlier in the document) cannot shadow the real snapshot,
/// which is always appended last.
pub fn parse_record(text: &str) -> Option<SessionDoc> {
    let begin = text.rfind(RECORD_SNAPSHOT_BEGIN)?;
    let rest = &text[begin + RECORD_SNAPSHOT_BEGIN.len()..];
    let end = rest.find(RECORD_SNAPSHOT_END)?;
    serde_json::from_str(rest[..end].trim()).ok()
}

/// One human-readable line for a decision event — shared by the exported
/// `## Session log` section and the Timeline panel. Labels resolve through
/// the doc best-effort (raw ids when an element has since vanished).
pub(crate) fn describe_event(doc: &SessionDoc, event: &DecisionEvent) -> String {
    match &event.kind {
        DecisionKind::OptionSelected {
            node_id,
            choice_id,
            option_id,
            ..
        } => {
            let opt = doc
                .node(node_id)
                .and_then(|n| n.choice(choice_id))
                .and_then(|c| c.options.iter().find(|o| &o.id == option_id))
                .map_or_else(|| option_id.to_string(), |o| o.label.clone());
            format!("picked “{opt}” for {}", node_label(doc, node_id))
        }
        DecisionKind::ChoiceDismissed {
            node_id,
            choice_id,
            reason,
        } => {
            let prompt = doc
                .node(node_id)
                .and_then(|n| n.choice(choice_id))
                .map_or_else(|| choice_id.to_string(), |c| c.prompt.clone());
            match reason {
                Some(r) => format!("dismissed “{prompt}” (reason: {r})"),
                None => format!("dismissed “{prompt}”"),
            }
        }
        DecisionKind::NoteAdded { node_id, note } => {
            format!("note on {}: {}", node_label(doc, node_id), note.text)
        }
        DecisionKind::QuestionAnswered {
            question_id,
            answer,
        } => {
            let prompt = doc
                .question(question_id)
                .map_or_else(|| question_id.to_string(), |q| q.prompt.clone());
            format!("answered “{prompt}”: {answer}")
        }
        DecisionKind::AnnotationAdded { annotation } => {
            format!(
                "added a {} annotation",
                annotation_kind_name(annotation.kind)
            )
        }
        DecisionKind::AnnotationEdited { annotation } => {
            format!(
                "edited a {} annotation",
                annotation_kind_name(annotation.kind)
            )
        }
        DecisionKind::AnnotationDeleted { .. } => "deleted an annotation".to_owned(),
        DecisionKind::FlushRequested { comment } => match comment {
            Some(c) => format!("comment: “{c}”"),
            None => "sent decisions to the agent".into(),
        },
        DecisionKind::NodeAdded { node } => format!("added component “{}”", node.label),
        DecisionKind::NodeEdited { label, .. } => format!("edited “{label}”"),
        DecisionKind::NodeDeleted { node_id } => format!("deleted component {node_id}"),
        DecisionKind::RemovalRequested { node_id } => {
            format!("asked to remove “{}”", node_label(doc, node_id))
        }
        DecisionKind::EdgeAdded { from, to, .. } => {
            format!(
                "connected {} → {}",
                node_label(doc, from),
                node_label(doc, to)
            )
        }
        DecisionKind::EdgeDeleted { from, to, .. } => {
            format!(
                "removed the edge {} → {}",
                node_label(doc, from),
                node_label(doc, to)
            )
        }
    }
}

fn node_label(doc: &SessionDoc, id: &NodeId) -> String {
    doc.node(id)
        .map_or_else(|| id.to_string(), |n| n.label.clone())
}

/// `### Components`, grouped under `####` sub-headers when any node carries a
/// `group` (groups in first-appearance order, ungrouped last).
fn render_components(out: &mut String, doc: &SessionDoc) {
    out.push_str("### Components\n\n");
    if doc.nodes.iter().any(|n| n.group.is_some()) {
        let mut seen_groups: Vec<&str> = Vec::new();
        for group in doc.nodes.iter().filter_map(|n| n.group.as_deref()) {
            if !seen_groups.contains(&group) {
                seen_groups.push(group);
            }
        }
        for group in seen_groups {
            let _ = writeln!(out, "#### {group}\n");
            for node in doc
                .nodes
                .iter()
                .filter(|n| n.group.as_deref() == Some(group))
            {
                component_bullet(out, node);
            }
            out.push('\n');
        }
        if doc.nodes.iter().any(|n| n.group.is_none()) {
            out.push_str("#### Ungrouped\n\n");
            for node in doc.nodes.iter().filter(|n| n.group.is_none()) {
                component_bullet(out, node);
            }
            out.push('\n');
        }
    } else {
        for node in &doc.nodes {
            component_bullet(out, node);
        }
        out.push('\n');
    }
}

fn component_bullet(out: &mut String, node: &Node) {
    let _ = write!(
        out,
        "- **{}** ({}, {}",
        node.label,
        kind_name(node.kind),
        status_name(node.status)
    );
    if let Some(build) = node.build {
        let _ = write!(out, ", {}", build.name());
    }
    out.push(')');
    if node.description.is_empty() {
        out.push('\n');
    } else {
        let _ = writeln!(out, " — {}", node.description);
    }
    for note in &node.notes {
        let _ = writeln!(
            out,
            "  - note ({}): {}",
            note.created_at.format("%Y-%m-%d"),
            note.text
        );
    }
}

/// One `###` per decided choice: the chosen option with pros/cons, the
/// runners-up, and (when the decision log has the event) the decision date
/// plus the user's pre-decision exploration trail.
fn render_decision(out: &mut String, node: &Node, choice: &Choice, log: &[DecisionEvent]) {
    let _ = writeln!(out, "### {} — {}\n", choice.prompt, node.label);
    if choice.needs_review {
        out.push_str(
            "> ⚠ Flagged for review: a dependency was reopened after this was decided.\n\n",
        );
    }
    if let Some(rationale) = &choice.rationale {
        let _ = writeln!(out, "{rationale}\n");
    }
    let selected = choice
        .selected
        .as_ref()
        .and_then(|id| choice.options.iter().find(|o| &o.id == id));
    match (selected, &choice.selected) {
        (Some(opt), _) => {
            let _ = write!(out, "**Decision: {}**", option_title(opt));
            if opt.summary.is_empty() {
                out.push('\n');
            } else {
                let _ = writeln!(out, " — {}", opt.summary);
            }
            out.push('\n');
            if !opt.pros.is_empty() {
                let _ = writeln!(out, "- Pros: {}", opt.pros.join("; "));
            }
            if !opt.cons.is_empty() {
                let _ = writeln!(out, "- Cons (accepted): {}", opt.cons.join("; "));
            }
            if !opt.pros.is_empty() || !opt.cons.is_empty() {
                out.push('\n');
            }
            let others: Vec<&ChoiceOption> =
                choice.options.iter().filter(|o| o.id != opt.id).collect();
            if !others.is_empty() {
                out.push_str("Also considered:\n\n");
                for other in others {
                    let _ = write!(out, "- **{}**", option_title(other));
                    if !other.summary.is_empty() {
                        let _ = write!(out, " — {}", other.summary);
                    }
                    if !other.pros.is_empty() || !other.cons.is_empty() {
                        let _ = write!(
                            out,
                            " (pros: {}; cons: {})",
                            other.pros.join("; "),
                            other.cons.join("; ")
                        );
                    }
                    out.push('\n');
                }
                out.push('\n');
            }
        }
        // `Decided` with no recorded option is reachable via the agent's
        // `resolve_choice` op; `selected` pointing at a removed option is a
        // doc-warning state — fall back to the raw id.
        (None, Some(raw)) => {
            let _ = writeln!(out, "**Decision: {}**\n", raw.as_str());
        }
        (None, None) => {
            out.push_str("**Decision: closed without a recorded option**\n\n");
        }
    }
    if let Some((event, option_id, considered)) = last_selection(log, &node.id, &choice.id) {
        let _ = write!(out, "_Decided {}", event.at.format("%Y-%m-%d"));
        let trail = considered_labels(choice, considered, option_id);
        if !trail.is_empty() {
            let _ = write!(out, ", after first exploring {}", trail.join(", "));
        }
        out.push_str("._\n\n");
    }
}

fn render_dismissal(out: &mut String, node: &Node, choice: &Choice, log: &[DecisionEvent]) {
    let _ = write!(out, "- **{}** ({})", choice.prompt, node.label);
    if let Some((event, reason)) = last_dismissal(log, &node.id, &choice.id) {
        out.push_str(" — ");
        if let Some(reason) = reason {
            let _ = write!(out, "reason: {reason}, ");
        }
        let _ = write!(out, "dismissed {}", event.at.format("%Y-%m-%d"));
    }
    out.push('\n');
}

fn render_open_question(out: &mut String, doc: &SessionDoc, node: &Node, choice: &Choice) {
    let options = choice
        .options
        .iter()
        .map(|o| {
            if o.recommended {
                format!("{} ★", o.label)
            } else {
                o.label.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" / ");
    let _ = writeln!(
        out,
        "- **{}** ({}) — options: {options}",
        choice.prompt, node.label
    );
    if let Some(rationale) = &choice.rationale {
        let _ = writeln!(out, "  - why: {rationale}");
    }
    let blockers: Vec<String> = doc
        .unmet_dependencies(choice)
        .iter()
        .map(|dep| {
            doc.node(&dep.node)
                .and_then(|n| n.choice(&dep.choice))
                .map_or_else(
                    || format!("{} / {}", dep.node, dep.choice),
                    |c| c.prompt.clone(),
                )
        })
        .collect();
    if !blockers.is_empty() {
        let _ = writeln!(out, "  - locked, waiting on: {}", blockers.join("; "));
    }
}

/// An answered free-form question: prompt, the component it hangs off (if
/// any), and the user's reply.
fn render_answered_question(out: &mut String, doc: &SessionDoc, question: &Question) {
    let _ = write!(out, "- **{}**", question.prompt);
    if let Some(node_id) = &question.node_id {
        let _ = write!(out, " ({})", node_label(doc, node_id));
    }
    let _ = writeln!(
        out,
        " — {}",
        question.answer.as_deref().unwrap_or("").trim()
    );
    if let Some(at) = &question.answered_at {
        let _ = writeln!(out, "  - _answered {}_", at.format("%Y-%m-%d"));
    }
}

/// An unanswered free-form question in the Open questions section.
fn render_unanswered_question(out: &mut String, doc: &SessionDoc, question: &Question) {
    let _ = write!(out, "- **{}**", question.prompt);
    if let Some(node_id) = &question.node_id {
        let _ = write!(out, " ({})", node_label(doc, node_id));
    }
    out.push_str(" — awaiting answer\n");
    if let Some(rationale) = &question.rationale {
        let _ = writeln!(out, "  - why: {rationale}");
    }
}

/// Option label, with the agent's recommendation marker when it carried one.
fn option_title(opt: &ChoiceOption) -> String {
    if opt.recommended {
        format!("{} ★ agent-recommended", opt.label)
    } else {
        opt.label.clone()
    }
}

/// Last `option_selected` event for this choice (users can re-pick; the last
/// one matches the doc), if the log has any.
fn last_selection<'a>(
    log: &'a [DecisionEvent],
    node: &NodeId,
    choice: &ChoiceId,
) -> Option<(&'a DecisionEvent, &'a OptionId, &'a [OptionId])> {
    log.iter().rev().find_map(|event| match &event.kind {
        DecisionKind::OptionSelected {
            node_id,
            choice_id,
            option_id,
            considered,
        } if node_id == node && choice_id == choice => Some((event, option_id, &considered[..])),
        _ => None,
    })
}

/// Last `choice_dismissed` event for this choice, if the log has any.
fn last_dismissal<'a>(
    log: &'a [DecisionEvent],
    node: &NodeId,
    choice: &ChoiceId,
) -> Option<(&'a DecisionEvent, Option<&'a str>)> {
    log.iter().rev().find_map(|event| match &event.kind {
        DecisionKind::ChoiceDismissed {
            node_id,
            choice_id,
            reason,
        } if node_id == node && choice_id == choice => Some((event, reason.as_deref())),
        _ => None,
    })
}

/// The exploration trail as labels: the `considered` ids minus the final
/// pick, deduplicated in order, mapped through the choice's options (raw id
/// when an option has since been removed).
fn considered_labels(choice: &Choice, considered: &[OptionId], picked: &OptionId) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    let mut seen: Vec<&OptionId> = Vec::new();
    for id in considered {
        if id == picked || seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let label = choice
            .options
            .iter()
            .find(|o| &o.id == id)
            .map_or_else(|| id.as_str().to_owned(), |o| o.label.clone());
        labels.push(label);
    }
    labels
}

/// Crate-visible aliases for the diff module.
pub(crate) fn kind_name_pub(kind: NodeKind) -> &'static str {
    kind_name(kind)
}

pub(crate) fn status_name_pub(status: ElementStatus) -> &'static str {
    status_name(status)
}

fn kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Service => "service",
        NodeKind::Module => "module",
        NodeKind::Component => "component",
        NodeKind::DataStore => "data_store",
        NodeKind::Queue => "queue",
        NodeKind::Ui => "ui",
        NodeKind::External => "external",
        NodeKind::Other => "other",
    }
}

fn status_name(status: ElementStatus) -> &'static str {
    STATUS_STYLES
        .iter()
        .find(|(s, ..)| *s == status)
        .map(|(_, name, ..)| *name)
        .expect("every status has a style")
}

/// `(class name, stroke/color hex, extra style)` per status, mirroring the
/// `--status-*` palette in `assets/main.css`. Stroke/color only — no fill —
/// so the diagram reads on light and dark backgrounds alike.
const STATUS_STYLES: [(ElementStatus, &str, &str, &str); 5] = [
    (ElementStatus::Existing, "existing", "#566076", ""),
    (ElementStatus::Proposed, "proposed", "#6c9ef8", ""),
    (ElementStatus::Modified, "modified", "#f0b34e", ""),
    (ElementStatus::Affected, "affected", "#b48af8", ""),
    (
        ElementStatus::Removed,
        "removed",
        "#f06a6a",
        ",stroke-dasharray:4 3",
    ),
];

/// Ids Mermaid reserves (or treats as edge syntax: a leading `o`/`x` after a
/// link forms circle/cross arrows). Compared lowercase.
const MERMAID_KEYWORDS: [&str; 12] = [
    "end",
    "subgraph",
    "graph",
    "flowchart",
    "direction",
    "classdef",
    "class",
    "style",
    "linkstyle",
    "click",
    "o",
    "x",
];

/// Render the graph as a Mermaid `flowchart LR` block body (no code fence).
///
/// Node shape encodes [`NodeKind`](crate::model::NodeKind); a `classDef` per
/// [`ElementStatus`](crate::model::ElementStatus) mirrors the app's status
/// palette (stroke/color only, so it reads in light and dark themes). Edge
/// arrows encode [`EdgeKind`](crate::model::EdgeKind); non-`existing` edge
/// statuses are colored via `linkStyle`. Ids are sanitized (Mermaid has
/// reserved words and a restricted id charset); labels are always quoted.
pub fn render_mermaid(doc: &SessionDoc) -> String {
    let ids = mermaid_ids(doc);
    let id_of: HashMap<&NodeId, &str> = doc
        .nodes
        .iter()
        .zip(&ids)
        .map(|(n, id)| (&n.id, id.as_str()))
        .collect();

    let mut out = String::from("flowchart LR\n");
    for (_, class, hex, extra) in STATUS_STYLES {
        let _ = writeln!(out, "    classDef {class} stroke:{hex},color:{hex}{extra}");
    }

    // Nodes, document order. The first node of a group pulls the whole group
    // into a subgraph at that point; later members are skipped when reached.
    let mut emitted = vec![false; doc.nodes.len()];
    let mut subgraph_count = 0usize;
    for i in 0..doc.nodes.len() {
        if emitted[i] {
            continue;
        }
        match &doc.nodes[i].group {
            None => {
                let _ = writeln!(out, "    {}", node_line(doc, i, &ids));
                emitted[i] = true;
            }
            Some(group) => {
                let _ = writeln!(
                    out,
                    "    subgraph sg_{subgraph_count}[\"{}\"]",
                    escape_label(group)
                );
                subgraph_count += 1;
                for (j, member) in doc.nodes.iter().enumerate().skip(i) {
                    if member.group.as_deref() == Some(group.as_str()) {
                        let _ = writeln!(out, "        {}", node_line(doc, j, &ids));
                        emitted[j] = true;
                    }
                }
                out.push_str("    end\n");
            }
        }
    }

    // Edges, document order; `linkStyle` indices refer to emitted edges only.
    // Dangling edges (rejected by validation on every mutation path) are
    // skipped defensively rather than emitted with unknown ids.
    let mut styled: Vec<(ElementStatus, Vec<usize>)> = STATUS_STYLES
        .iter()
        .filter(|(status, ..)| *status != ElementStatus::Existing)
        .map(|(status, ..)| (*status, Vec::new()))
        .collect();
    let mut edge_index = 0usize;
    for edge in &doc.edges {
        let (Some(from), Some(to)) = (id_of.get(&edge.from), id_of.get(&edge.to)) else {
            continue;
        };
        let arrow = match edge.kind {
            EdgeKind::DataFlow => "==>",
            EdgeKind::Contains => "-.->",
            EdgeKind::DependsOn | EdgeKind::Other => "-->",
        };
        match &edge.label {
            Some(label) => {
                let _ = writeln!(
                    out,
                    "    {from} {arrow}|\"{}\"| {to}",
                    escape_edge_label(label)
                );
            }
            None => {
                let _ = writeln!(out, "    {from} {arrow} {to}");
            }
        }
        if let Some((_, indices)) = styled.iter_mut().find(|(s, _)| *s == edge.status) {
            indices.push(edge_index);
        }
        edge_index += 1;
    }
    for (status, indices) in styled {
        if indices.is_empty() {
            continue;
        }
        let (_, _, hex, extra) = STATUS_STYLES
            .iter()
            .find(|(s, ..)| *s == status)
            .expect("styled statuses come from STATUS_STYLES");
        let list = indices
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(out, "    linkStyle {list} stroke:{hex}{extra}");
    }

    out
}

/// One node statement: sanitized id, kind-shaped quoted label, status class.
fn node_line(doc: &SessionDoc, index: usize, ids: &[String]) -> String {
    let node = &doc.nodes[index];
    let (open, close) = match node.kind {
        NodeKind::Service => ("{{\"", "\"}}"),
        NodeKind::Module => ("[[\"", "\"]]"),
        NodeKind::DataStore => ("[(\"", "\")]"),
        NodeKind::Queue => ("[/\"", "\"/]"),
        NodeKind::Ui => ("([\"", "\"])"),
        NodeKind::External => ("[\\\"", "\"\\]"),
        NodeKind::Component | NodeKind::Other => ("[\"", "\"]"),
    };
    let class = STATUS_STYLES
        .iter()
        .find(|(s, ..)| *s == node.status)
        .map(|(_, class, ..)| *class)
        .expect("every status has a style");
    format!(
        "{id}{open}{label}{close}:::{class}",
        id = ids[index],
        label = escape_label(&node.label)
    )
}

/// Mermaid ids for every node, parallel to `doc.nodes`: sanitized, keyword-
/// safe, and made unique with `_2`, `_3`… suffixes in document order.
fn mermaid_ids(doc: &SessionDoc) -> Vec<String> {
    let mut ids: Vec<String> = Vec::with_capacity(doc.nodes.len());
    for node in &doc.nodes {
        let base = sanitize_id(node.id.as_str());
        let mut candidate = base.clone();
        let mut n = 2;
        while ids.contains(&candidate) {
            candidate = format!("{base}_{n}");
            n += 1;
        }
        ids.push(candidate);
    }
    ids
}

/// Restrict to `[A-Za-z0-9_-]` (everything else becomes `_`) and prefix `n_`
/// when the result is empty, does not start with a letter, or is a Mermaid
/// keyword.
fn sanitize_id(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let needs_prefix = !cleaned
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        || MERMAID_KEYWORDS.contains(&cleaned.to_ascii_lowercase().as_str());
    if needs_prefix {
        format!("n_{cleaned}")
    } else {
        cleaned
    }
}

/// Labels are always emitted double-quoted; quoting makes `]`, `)`, `/` safe
/// and `#quot;` is Mermaid's escaped double quote.
fn escape_label(label: &str) -> String {
    label.replace('"', "#quot;")
}

/// Edge labels additionally cannot contain the `|` delimiter.
fn escape_edge_label(label: &str) -> String {
    escape_label(label).replace('|', "#124;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_doc;
    use crate::model::{
        Edge, EdgeKind, ElementStatus, Node, NodeId, NodeKind, Origin, QuestionId, SessionDoc,
    };

    fn tnode(id: &str, label: &str) -> Node {
        Node {
            id: NodeId::from(id),
            label: label.to_owned(),
            kind: NodeKind::Component,
            description: String::new(),
            status: ElementStatus::Existing,
            build: None,
            group: None,
            lane: None,
            choices: vec![],
            notes: vec![],
            agent: None,
            position: None,
            origin: Origin::Agent,
        }
    }

    fn tedge(from: &str, to: &str, kind: EdgeKind, status: ElementStatus) -> Edge {
        Edge {
            from: NodeId::from(from),
            to: NodeId::from(to),
            kind,
            label: None,
            status,
            origin: Origin::Agent,
        }
    }

    fn tdoc(nodes: Vec<Node>, edges: Vec<Edge>) -> SessionDoc {
        SessionDoc {
            version: SessionDoc::VERSION,
            title: "test".into(),
            revision: 0,
            focus: None,
            nodes,
            edges,
            questions: vec![],
            annotations: vec![],
        }
    }

    #[test]
    fn mermaid_demo_has_all_nodes_and_edges() {
        let doc = demo_doc();
        let m = render_mermaid(&doc);
        assert!(m.starts_with("flowchart LR\n"), "got: {m}");
        for class_def in [
            "    classDef existing stroke:#566076,color:#566076",
            "    classDef proposed stroke:#6c9ef8,color:#6c9ef8",
            "    classDef modified stroke:#f0b34e,color:#f0b34e",
            "    classDef affected stroke:#b48af8,color:#b48af8",
            "    classDef removed stroke:#f06a6a,color:#f06a6a,stroke-dasharray:4 3",
        ] {
            assert!(m.contains(class_def), "missing {class_def:?} in: {m}");
        }
        // One line per node, shape per kind, status class attached.
        for node_line in [
            r#"web-ui(["Web UI"]):::existing"#,                 // ui → stadium
            r#"api-gateway{{"API Gateway"}}:::existing"#,       // service → hexagon
            r#"sync-engine["Sync Engine"]:::proposed"#,         // component → rect
            r#"postgres[("PostgreSQL")]:::existing"#,           // data_store → cylinder
            r#"job-queue[/"Job Queue"/]:::existing"#,           // queue → parallelogram
            r#"email-provider[\"Email Provider"\]:::existing"#, // external
            r#"redis[("Redis")]:::affected"#,
            r#"notes-service{{"Notes Service"}}:::modified"#,
        ] {
            assert!(m.contains(node_line), "missing {node_line:?} in: {m}");
        }
        // Arrow per edge kind.
        assert!(m.contains("    web-ui ==> api-gateway"), "data_flow: {m}");
        assert!(
            m.contains("    api-gateway --> auth-service"),
            "depends_on: {m}"
        );
        assert!(m.contains("    auth-service -.-> redis"), "contains: {m}");
        // Every edge is emitted.
        let arrows = m
            .lines()
            .filter(|l| l.contains("-->") || l.contains("==>") || l.contains("-.->"))
            .count();
        assert_eq!(arrows, doc.edges.len(), "in: {m}");
    }

    #[test]
    fn mermaid_is_deterministic() {
        let doc = demo_doc();
        assert_eq!(render_mermaid(&doc), render_mermaid(&doc));
    }

    #[test]
    fn mermaid_sanitizes_hostile_ids() {
        let doc = tdoc(
            vec![
                tnode("has space", "A"),
                tnode("end", "B"),
                tnode("näme", "C"),
                tnode("has?space", "D"), // collides with "has space" after sanitizing
                tnode("2fa", "E"),
            ],
            vec![tedge(
                "has space",
                "end",
                EdgeKind::DependsOn,
                ElementStatus::Existing,
            )],
        );
        let m = render_mermaid(&doc);
        assert!(m.contains(r#"has_space["A"]"#), "in: {m}");
        assert!(m.contains(r#"n_end["B"]"#), "keyword prefixed, in: {m}");
        assert!(m.contains(r#"n_me["C"]"#), "unicode replaced, in: {m}");
        assert!(
            m.contains(r#"has_space_2["D"]"#),
            "collision suffixed, in: {m}"
        );
        assert!(m.contains(r#"n_2fa["E"]"#), "digit start prefixed, in: {m}");
        assert!(
            m.contains("    has_space --> n_end"),
            "edge uses map, in: {m}"
        );
    }

    #[test]
    fn mermaid_escapes_labels() {
        let mut edge = tedge("a", "b", EdgeKind::DependsOn, ElementStatus::Existing);
        edge.label = Some("read|write".into());
        let doc = tdoc(
            vec![tnode("a", r#"say "hi""#), tnode("b", "plain")],
            vec![edge],
        );
        let m = render_mermaid(&doc);
        assert!(m.contains(r#"a["say #quot;hi#quot;"]"#), "in: {m}");
        assert!(m.contains(r#"a -->|"read#124;write"| b"#), "in: {m}");
        assert!(
            !m.contains(r#"say "hi""#),
            "raw quote must not survive: {m}"
        );
    }

    #[test]
    fn mermaid_groups_become_subgraphs() {
        let mut a = tnode("a", "A");
        a.group = Some("Platform".into());
        let b = tnode("b", "B"); // ungrouped
        let mut c = tnode("c", "C");
        c.group = Some("Platform".into());
        let mut d = tnode("d", "D");
        d.group = Some("Edge".into());
        let m = render_mermaid(&tdoc(vec![a, b, c, d], vec![]));

        let sg0 = m.find(r#"subgraph sg_0["Platform"]"#).expect("sg_0");
        let sg1 = m.find(r#"subgraph sg_1["Edge"]"#).expect("sg_1");
        assert!(sg0 < sg1, "first-appearance order, in: {m}");
        let sg0_end = m[sg0..]
            .find("\n    end\n")
            .map(|i| sg0 + i)
            .expect("sg_0 end");
        for member in [r#"a["A"]"#, r#"c["C"]"#] {
            let pos = m.find(member).expect(member);
            assert!(sg0 < pos && pos < sg0_end, "{member} inside sg_0, in: {m}");
        }
        let b_pos = m.find(r#"b["B"]"#).expect("b");
        assert!(
            !(sg0 < b_pos && b_pos < sg0_end),
            "ungrouped b outside sg_0, in: {m}"
        );
    }

    #[test]
    fn mermaid_edge_status_linkstyle() {
        let doc = tdoc(
            vec![tnode("a", "A"), tnode("b", "B"), tnode("c", "C")],
            vec![
                tedge("a", "b", EdgeKind::DependsOn, ElementStatus::Existing),
                tedge("b", "c", EdgeKind::DependsOn, ElementStatus::Proposed),
                tedge("a", "c", EdgeKind::DependsOn, ElementStatus::Removed),
            ],
        );
        let m = render_mermaid(&doc);
        assert!(m.contains("    linkStyle 1 stroke:#6c9ef8"), "in: {m}");
        assert!(
            m.contains("    linkStyle 2 stroke:#f06a6a,stroke-dasharray:4 3"),
            "in: {m}"
        );
        assert!(!m.contains("linkStyle 0"), "existing edges unstyled: {m}");

        let plain = tdoc(
            vec![tnode("a", "A"), tnode("b", "B")],
            vec![tedge(
                "a",
                "b",
                EdgeKind::DependsOn,
                ElementStatus::Existing,
            )],
        );
        assert!(
            !render_mermaid(&plain).contains("linkStyle"),
            "all-existing doc needs no linkStyle"
        );
    }

    #[test]
    fn mermaid_empty_doc() {
        let m = render_mermaid(&SessionDoc::default());
        assert!(m.starts_with("flowchart LR\n"), "got: {m}");
        assert!(m.contains("classDef existing"), "in: {m}");
        assert!(!m.contains("subgraph"), "in: {m}");
        assert!(!m.contains("-->"), "in: {m}");
    }

    // ---------- markdown ----------

    use chrono::TimeZone;

    use crate::model::{ChoiceStatus, DecisionEvent, DecisionKind, Note};

    fn ts() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 7, 17, 12, 30, 0)
            .unwrap()
    }

    fn selected_event(
        seq: u64,
        node: &str,
        choice: &str,
        option: &str,
        considered: &[&str],
    ) -> DecisionEvent {
        DecisionEvent {
            seq,
            at: ts(),
            target_agent: None,
            kind: DecisionKind::OptionSelected {
                node_id: node.into(),
                choice_id: choice.into(),
                option_id: option.into(),
                considered: considered.iter().map(|c| (*c).into()).collect(),
            },
        }
    }

    fn dismissed_event(seq: u64, node: &str, choice: &str, reason: Option<&str>) -> DecisionEvent {
        DecisionEvent {
            seq,
            at: ts(),
            target_agent: None,
            kind: DecisionKind::ChoiceDismissed {
                node_id: node.into(),
                choice_id: choice.into(),
                reason: reason.map(str::to_owned),
            },
        }
    }

    /// Demo doc with choice 1 decided (crdt) and choice 2 dismissed, plus the
    /// matching decision-log events.
    fn decided_demo() -> (SessionDoc, Vec<DecisionEvent>) {
        let mut doc = demo_doc();
        let c = &mut doc.nodes[4].choices[0];
        c.selected = Some("crdt".into());
        c.status = ChoiceStatus::Decided;
        let c = &mut doc.nodes[5].choices[0];
        c.status = ChoiceStatus::Dismissed;
        if let Some(q) = doc.questions.first_mut() {
            q.answer = Some("90 days of full history, then squash to daily snapshots".into());
        }
        let log = vec![
            selected_event(
                1,
                "sync-engine",
                "conflict-resolution",
                "crdt",
                &["ot", "crdt"],
            ),
            dismissed_event(
                2,
                "ws-gateway",
                "ws-deployment",
                Some("out of scope for v1"),
            ),
        ];
        (doc, log)
    }

    #[test]
    fn markdown_demo_full_record() {
        let (doc, log) = decided_demo();
        let md = render_markdown(&doc, &log, ts());
        assert!(
            md.starts_with("# Realtime collaboration for the notes app\n"),
            "got: {md}"
        );
        // No session revision in the preamble: every store mutation bumps it
        // (including recording the export itself), so embedding it would make
        // back-to-back exports of an unchanged graph differ.
        assert!(
            md.contains("_Decision record exported from a nodestorm brainstorm on 2026-07-17._"),
            "preamble, in: {md}"
        );
        assert!(!md.contains("session revision"), "in: {md}");
        assert!(
            md.contains("**11 components · 1 decided · 1 dismissed · 0 open**"),
            "counts, in: {md}"
        );
        assert!(md.contains("## Architecture"), "in: {md}");
        assert!(md.contains("```mermaid\nflowchart LR\n"), "fence, in: {md}");
        assert!(md.contains("### Components"), "in: {md}");
        assert!(
            md.contains(
                "- **PostgreSQL** (data_store, existing) — Primary storage for notes and users"
            ),
            "component line, in: {md}"
        );
        assert!(md.contains("## Decisions"), "in: {md}");
        assert!(
            md.contains("### How should concurrent edits be reconciled? — Sync Engine"),
            "in: {md}"
        );
        assert!(
            md.contains("Realtime collaboration means two clients editing one note at once"),
            "rationale, in: {md}"
        );
        assert!(
            md.contains("**Decision: CRDTs ★ agent-recommended** — Conflict-free replicated data types (e.g. Yjs-style) merge automatically."),
            "in: {md}"
        );
        assert!(
            md.contains("- Pros: No central sequencing required; Offline edits merge cleanly"),
            "in: {md}"
        );
        assert!(
            md.contains(
                "- Cons (accepted): Document format changes (stored as CRDT state); Larger payloads"
            ),
            "in: {md}"
        );
        assert!(md.contains("Also considered:"), "in: {md}");
        assert!(
            md.contains("- **Operational Transform** — Central server transforms and sequences concurrent operations."),
            "in: {md}"
        );
        assert!(
            md.contains("_Decided 2026-07-17, after first exploring Operational Transform._"),
            "trail (final pick filtered out), in: {md}"
        );
        assert!(md.contains("## Dismissed decisions"), "in: {md}");
        assert!(
            md.contains("- **Where should websocket connections terminate?** (WebSocket Gateway) — reason: out of scope for v1, dismissed 2026-07-17"),
            "in: {md}"
        );
        assert!(md.contains("## Questions answered"), "in: {md}");
        assert!(
            md.contains("**How long should full edit history be retained once notes go realtime?** (PostgreSQL) — 90 days of full history, then squash to daily snapshots"),
            "in: {md}"
        );
        assert!(!md.contains("## Open questions"), "none open, in: {md}");
    }

    #[test]
    fn markdown_open_choice_shows_dependency_lock() {
        // demo: ws-deployment is locked behind conflict-resolution.
        let md = render_markdown(&demo_doc(), &[], ts());
        assert!(
            md.contains("locked, waiting on: How should concurrent edits be reconciled?"),
            "{md}"
        );
    }

    #[test]
    fn markdown_flags_reviewed_decision() {
        let mut node = tnode("api", "API");
        node.choices = vec![Choice {
            id: "x".into(),
            prompt: "Persistence?".into(),
            rationale: None,
            options: vec![ChoiceOption {
                id: "sqlite".into(),
                label: "SQLite".into(),
                summary: String::new(),
                pros: vec![],
                cons: vec![],
                recommended: false,
                affects: vec![],
            }],
            selected: Some("sqlite".into()),
            status: ChoiceStatus::Decided,
            depends_on: vec![],
            needs_review: true,
            reopen: false,
        }];
        let md = render_markdown(&tdoc(vec![node], vec![]), &[], ts());
        assert!(md.contains("Flagged for review"), "{md}");
    }

    #[test]
    fn markdown_renders_implementation_status() {
        let mut api = tnode("api", "API");
        api.build = Some(BuildStatus::Verified);
        let mut cache = tnode("cache", "Cache");
        cache.build = Some(BuildStatus::Building);
        let doc = tdoc(vec![api, cache, tnode("web", "Web")], vec![]);
        let md = render_markdown(&doc, &[], ts());
        assert!(md.contains("## Implementation status"), "{md}");
        assert!(md.contains("**1/2 shipped**"), "{md}");
        assert!(md.contains("- [x] **API** — verified"), "{md}");
        assert!(md.contains("- [ ] **Cache** — building"), "{md}");
        // The component inventory gains the build column, untracked nodes omit it.
        assert!(
            md.contains("- **API** (component, existing, verified)"),
            "{md}"
        );
        assert!(md.contains("- **Web** (component, existing)"), "{md}");
    }

    #[test]
    fn markdown_renders_answered_and_open_questions() {
        let mut doc = tdoc(vec![tnode("api", "API")], vec![]);
        doc.questions = vec![
            Question {
                id: QuestionId::from("deploy"),
                prompt: "Which environment ships first?".into(),
                node_id: Some(NodeId::from("api")),
                rationale: None,
                answer: Some("staging".into()),
                answered_at: None,
            },
            Question {
                id: QuestionId::from("budget"),
                prompt: "What's the latency budget?".into(),
                node_id: None,
                rationale: Some("shapes the cache".into()),
                answer: None,
                answered_at: None,
            },
        ];
        let md = render_markdown(&doc, &[], ts());
        assert!(md.contains("## Questions answered"), "{md}");
        assert!(
            md.contains("**Which environment ships first?** (API) — staging"),
            "{md}"
        );
        assert!(md.contains("## Open questions"), "{md}");
        assert!(
            md.contains("**What's the latency budget?** — awaiting answer"),
            "{md}"
        );
        assert!(md.contains("why: shapes the cache"), "{md}");
    }

    #[test]
    fn markdown_renders_annotations_section() {
        use crate::model::{Annotation, AnnotationId, Origin};
        let mut doc = tdoc(vec![tnode("api", "API")], vec![]);
        doc.annotations = vec![
            Annotation {
                id: AnnotationId::from("n1"),
                kind: AnnotationKind::Note,
                x: 12.0,
                y: 34.0,
                w: 0.0,
                h: 0.0,
                text: "revisit auth".into(),
                origin: Origin::User,
            },
            Annotation {
                id: AnnotationId::from("r1"),
                kind: AnnotationKind::Region,
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 80.0,
                text: String::new(),
                origin: Origin::User,
            },
        ];
        let md = render_markdown(&doc, &[], ts());
        assert!(md.contains("## Annotations"), "{md}");
        assert!(md.contains("- note: revisit auth"), "{md}");
        assert!(md.contains("- region at (0, 0)"), "{md}");
    }

    #[test]
    fn empty_record_still_carries_a_diffable_snapshot() {
        let md = render_markdown(&SessionDoc::default(), &[], ts());
        assert!(md.contains("_Empty session"), "{md}");
        let parsed = parse_record(&md).expect("even an empty record is diff-able");
        assert!(parsed.nodes.is_empty());
    }

    #[test]
    fn snapshot_survives_comment_terminator_in_user_text() {
        use crate::model::{Annotation, AnnotationId, AnnotationKind, Note, NoteId, Origin};
        let mut node = tnode("api", "API");
        node.notes = vec![Note {
            id: NoteId::from("n"),
            text: "watch out --> here".into(),
            created_at: ts(),
        }];
        let mut doc = tdoc(vec![node], vec![]);
        doc.annotations = vec![Annotation {
            id: AnnotationId::from("a"),
            kind: AnnotationKind::Note,
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            text: "also --> this".into(),
            origin: Origin::User,
        }];
        let md = render_markdown(&doc, &[], ts());
        let parsed = parse_record(&md).expect("snapshot round-trips despite -->");
        assert_eq!(parsed.nodes[0].notes[0].text, "watch out --> here");
        assert_eq!(parsed.annotations[0].text, "also --> this");
    }

    #[test]
    fn record_snapshot_round_trips_and_stays_hidden() {
        let doc = demo_doc();
        let md = render_markdown(&doc, &[], ts());
        // Embedded as an HTML comment (invisible when rendered), not a heading.
        assert!(md.contains("<!-- nodestorm-record v1"), "{md}");
        let parsed = parse_record(&md).expect("snapshot present");
        assert_eq!(parsed.nodes.len(), doc.nodes.len());
        assert_eq!(parsed.title, doc.title);
        assert_eq!(parsed.revision, 0, "revision normalized");
        assert!(
            parsed.nodes.iter().all(|n| n.position.is_none()),
            "positions stripped"
        );
        // Deterministic: re-render is byte-identical.
        assert_eq!(md, render_markdown(&doc, &[], ts()));
        // Nothing to parse in unrelated text.
        assert!(parse_record("just some markdown, no snapshot").is_none());
    }

    #[test]
    fn markdown_is_deterministic() {
        let (doc, log) = decided_demo();
        assert_eq!(
            render_markdown(&doc, &log, ts()),
            render_markdown(&doc, &log, ts())
        );
    }

    #[test]
    fn markdown_empty_doc() {
        let md = render_markdown(&SessionDoc::default(), &[], ts());
        assert!(md.starts_with("# Untitled brainstorm\n"), "got: {md}");
        assert!(
            md.contains("_Empty session — nothing on the canvas yet._"),
            "in: {md}"
        );
        assert!(!md.contains("## Architecture"), "in: {md}");
        assert!(!md.contains("components ·"), "no counts line, in: {md}");
        assert!(!md.contains("## Decisions"), "in: {md}");
        assert!(!md.contains("## Open questions"), "in: {md}");
    }

    #[test]
    fn markdown_open_only() {
        let md = render_markdown(&demo_doc(), &[], ts());
        assert!(md.contains("## Open questions"), "in: {md}");
        assert!(
            md.contains("- **How should concurrent edits be reconciled?** (Sync Engine) — options: CRDTs ★ / Operational Transform / Last-write-wins"),
            "in: {md}"
        );
        assert!(
            md.contains("- **Where should websocket connections terminate?** (WebSocket Gateway) — options: Dedicated gateway service ★ / Inside the API gateway"),
            "in: {md}"
        );
        assert!(!md.contains("## Decisions"), "in: {md}");
        assert!(!md.contains("## Dismissed decisions"), "in: {md}");
        assert!(
            md.contains("**11 components · 0 decided · 0 dismissed · 2 open**"),
            "in: {md}"
        );
    }

    #[test]
    fn markdown_dismissed_only() {
        let mut doc = demo_doc();
        doc.nodes[5].choices[0].status = ChoiceStatus::Dismissed;
        let md = render_markdown(&doc, &[], ts());
        assert!(md.contains("## Dismissed decisions"), "in: {md}");
        // No log event → no reason, no date.
        assert!(
            md.contains(
                "- **Where should websocket connections terminate?** (WebSocket Gateway)\n"
            ),
            "bare bullet, in: {md}"
        );
        assert!(!md.contains("reason:"), "in: {md}");
        assert!(!md.contains("## Decisions"), "in: {md}");
    }

    #[test]
    fn markdown_decided_without_event_or_option() {
        // Agent-side resolve_choice writes no log event.
        let mut doc = demo_doc();
        doc.nodes[4].choices[0].selected = Some("crdt".into());
        doc.nodes[4].choices[0].status = ChoiceStatus::Decided;
        let md = render_markdown(&doc, &[], ts());
        assert!(md.contains("**Decision: CRDTs"), "in: {md}");
        assert!(
            !md.contains("_Decided"),
            "no event → no date line, in: {md}"
        );
        assert!(!md.contains("after first exploring"), "in: {md}");

        // Decided with no recorded option (reachable via GraphOp::ResolveChoice).
        let mut doc = demo_doc();
        doc.nodes[4].choices[0].selected = None;
        doc.nodes[4].choices[0].status = ChoiceStatus::Decided;
        let md = render_markdown(&doc, &[], ts());
        assert!(
            md.contains("**Decision: closed without a recorded option**"),
            "in: {md}"
        );
    }

    #[test]
    fn markdown_notes_render_under_components() {
        let mut doc = demo_doc();
        doc.nodes[6].notes.push(Note {
            id: "note-1".into(),
            text: "Must keep working offline".into(),
            created_at: ts(),
        });
        let md = render_markdown(&doc, &[], ts());
        assert!(
            md.contains("  - note (2026-07-17): Must keep working offline"),
            "in: {md}"
        );
    }

    #[test]
    fn markdown_session_log_section() {
        let (doc, mut log) = decided_demo();
        log.push(DecisionEvent {
            seq: 3,
            at: ts(),
            target_agent: None,
            kind: DecisionKind::FlushRequested {
                comment: Some("looks good, proceed".into()),
            },
        });
        log.push(DecisionEvent {
            seq: 4,
            at: ts(),
            target_agent: None,
            kind: DecisionKind::NodeAdded {
                node: Node {
                    id: "rate-limiter".into(),
                    label: "Rate Limiter".into(),
                    kind: NodeKind::Component,
                    description: String::new(),
                    status: ElementStatus::Proposed,
                    build: None,
                    group: None,
                    lane: None,
                    choices: vec![],
                    notes: vec![],
                    agent: None,
                    position: None,
                    origin: Origin::User,
                },
            },
        });
        let md = render_markdown(&doc, &log, ts());
        let log_at = md.find("## Session log").expect("session log section");
        let tail = &md[log_at..];
        for (i, line) in [
            "- 2026-07-17 12:30 — picked “CRDTs” for Sync Engine",
            "- 2026-07-17 12:30 — dismissed “Where should websocket connections terminate?” (reason: out of scope for v1)",
            "- 2026-07-17 12:30 — comment: “looks good, proceed”",
            "- 2026-07-17 12:30 — added component “Rate Limiter”",
        ]
        .iter()
        .enumerate()
        {
            let pos = tail.find(line);
            assert!(pos.is_some(), "line {i} missing: {line}\nin: {tail}");
        }
        // Chronological (seq) order within the section.
        let picked = tail.find("picked").unwrap();
        let dismissed = tail.find("dismissed").unwrap();
        let comment = tail.find("comment:").unwrap();
        let added = tail.find("added component").unwrap();
        assert!(picked < dismissed && dismissed < comment && comment < added);
    }

    #[test]
    fn markdown_no_log_no_session_log_section() {
        let md = render_markdown(&demo_doc(), &[], ts());
        assert!(!md.contains("## Session log"), "in: {md}");
    }

    #[test]
    fn markdown_groups_components() {
        let mut a = tnode("a", "Alpha");
        a.group = Some("Platform".into());
        let b = tnode("b", "Beta");
        let md = render_markdown(&tdoc(vec![a, b], vec![]), &[], ts());
        assert!(md.contains("#### Platform"), "in: {md}");
        assert!(md.contains("#### Ungrouped"), "in: {md}");
        let plat = md.find("#### Platform").unwrap();
        let ungrouped = md.find("#### Ungrouped").unwrap();
        let alpha = md.find("- **Alpha**").unwrap();
        assert!(
            plat < alpha && alpha < ungrouped,
            "alpha under Platform: {md}"
        );
    }
}
