//! Structured diff between two brainstorm docs, rendered as Markdown.
//!
//! Pure and deterministic like `export.rs`: document order everywhere,
//! sections omitted when empty, byte-identical output for identical input.

use std::fmt::Write as _;

use crate::model::{ChoiceStatus, SessionDoc};

/// Markdown summary of how brainstorm `b` differs from brainstorm `a`:
/// components added/removed/changed (field-level), edges added/removed by
/// `(from, to, kind)`, and decision drift. `_No differences._` when equal.
pub fn diff_docs(a_name: &str, a: &SessionDoc, b_name: &str, b: &SessionDoc) -> String {
    let mut out = format!("# Diff: {a_name} → {b_name}\n\n");

    let kind_name = |k| crate::export::kind_name_pub(k);
    let status_name = |s| crate::export::status_name_pub(s);

    // ---- components ----
    let mut comp_lines: Vec<String> = Vec::new();
    for nb in &b.nodes {
        match a.node(&nb.id) {
            None => comp_lines.push(format!(
                "+ added: **{}** ({}, {})",
                nb.label,
                kind_name(nb.kind),
                status_name(nb.status)
            )),
            Some(na) => {
                let mut changes: Vec<String> = Vec::new();
                if na.label != nb.label {
                    changes.push(format!("label: {} → {}", na.label, nb.label));
                }
                if na.kind != nb.kind {
                    changes.push(format!(
                        "kind: {} → {}",
                        kind_name(na.kind),
                        kind_name(nb.kind)
                    ));
                }
                if na.status != nb.status {
                    changes.push(format!(
                        "status: {} → {}",
                        status_name(na.status),
                        status_name(nb.status)
                    ));
                }
                if na.group != nb.group {
                    changes.push(format!(
                        "group: {} → {}",
                        na.group.as_deref().unwrap_or("—"),
                        nb.group.as_deref().unwrap_or("—")
                    ));
                }
                if na.description != nb.description {
                    changes.push("description changed".into());
                }
                if !changes.is_empty() {
                    comp_lines.push(format!(
                        "~ changed: **{}** ({})",
                        nb.label,
                        changes.join(", ")
                    ));
                }
            }
        }
    }
    for na in &a.nodes {
        if b.node(&na.id).is_none() {
            comp_lines.push(format!("- removed: **{}**", na.label));
        }
    }

    // ---- edges ----
    let label_of = |doc: &SessionDoc, id: &crate::model::NodeId| {
        doc.node(id)
            .map_or_else(|| id.to_string(), |n| n.label.clone())
    };
    let mut edge_lines: Vec<String> = Vec::new();
    for eb in &b.edges {
        if !a.edges.iter().any(|ea| ea.key() == eb.key()) {
            edge_lines.push(format!(
                "+ added: {} —{}→ {}",
                label_of(b, &eb.from),
                edge_kind_name(eb.kind),
                label_of(b, &eb.to)
            ));
        }
    }
    for ea in &a.edges {
        if !b.edges.iter().any(|eb| eb.key() == ea.key()) {
            edge_lines.push(format!(
                "- removed: {} —{}→ {}",
                label_of(a, &ea.from),
                edge_kind_name(ea.kind),
                label_of(a, &ea.to)
            ));
        }
    }

    // ---- decisions ----
    let mut decision_lines: Vec<String> = Vec::new();
    for nb in &b.nodes {
        for cb in &nb.choices {
            let ca = a.node(&nb.id).and_then(|n| n.choice(&cb.id));
            let opt_label = |c: &crate::model::Choice, id: &Option<crate::model::OptionId>| {
                id.as_ref()
                    .and_then(|i| c.options.iter().find(|o| &o.id == i))
                    .map_or_else(
                        || id.as_ref().map_or("—".into(), |i| i.to_string()),
                        |o| o.label.clone(),
                    )
            };
            match (ca.map(|c| c.status), cb.status) {
                (Some(ChoiceStatus::Decided), ChoiceStatus::Decided) => {
                    let ca = ca.expect("matched Some");
                    if ca.selected != cb.selected {
                        decision_lines.push(format!(
                            "decided differently: “{}” (a: {}, b: {})",
                            cb.prompt,
                            opt_label(ca, &ca.selected),
                            opt_label(cb, &cb.selected)
                        ));
                    }
                }
                (_, ChoiceStatus::Decided) => decision_lines.push(format!(
                    "newly decided: “{}” → {}",
                    cb.prompt,
                    opt_label(cb, &cb.selected)
                )),
                (Some(ChoiceStatus::Decided), ChoiceStatus::Open) => {
                    decision_lines.push(format!("reopened: “{}”", cb.prompt));
                }
                (Some(ChoiceStatus::Dismissed), _) | (_, ChoiceStatus::Open) => {}
                (_, ChoiceStatus::Dismissed) => {
                    decision_lines.push(format!("newly dismissed: “{}”", cb.prompt));
                }
            }
        }
    }

    if comp_lines.is_empty() && edge_lines.is_empty() && decision_lines.is_empty() {
        out.push_str("_No differences._\n");
        return out;
    }
    for (title, lines) in [
        ("## Components", comp_lines),
        ("## Edges", edge_lines),
        ("## Decisions", decision_lines),
    ] {
        if !lines.is_empty() {
            let _ = writeln!(out, "{title}\n");
            for line in lines {
                let _ = writeln!(out, "{line}");
            }
            out.push('\n');
        }
    }
    out
}

fn edge_kind_name(kind: crate::model::EdgeKind) -> &'static str {
    match kind {
        crate::model::EdgeKind::DependsOn => "depends_on",
        crate::model::EdgeKind::DataFlow => "data_flow",
        crate::model::EdgeKind::Contains => "contains",
        crate::model::EdgeKind::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_doc;
    use crate::model::{ChoiceStatus, ElementStatus, NodeId};

    #[test]
    fn identical_docs_say_no_differences() {
        let d = demo_doc();
        let md = diff_docs("a", &d, "b", &d);
        assert!(md.starts_with("# Diff: a → b\n"), "got: {md}");
        assert!(md.contains("_No differences._"), "in: {md}");
        assert!(!md.contains("## Components"), "in: {md}");
    }

    #[test]
    fn component_changes_reported() {
        let a = demo_doc();
        let mut b = demo_doc();
        // Added in b.
        b.nodes.push({
            let mut n = a.nodes[0].clone();
            n.id = NodeId::from("rate-limiter");
            n.label = "Rate Limiter".into();
            n
        });
        // Removed from b.
        b.nodes.retain(|n| n.id.as_str() != "email-provider");
        b.edges
            .retain(|e| e.from.as_str() != "email-provider" && e.to.as_str() != "email-provider");
        // Changed in b.
        {
            let n = b.node_mut(&NodeId::from("redis")).unwrap();
            n.label = "Redis Cluster".into();
            n.status = ElementStatus::Modified;
        }
        let md = diff_docs("a", &a, "b", &b);
        assert!(md.contains("## Components"), "in: {md}");
        assert!(md.contains("+ added: **Rate Limiter**"), "in: {md}");
        assert!(md.contains("- removed: **Email Provider**"), "in: {md}");
        assert!(
            md.contains("~ changed: **Redis Cluster** (label: Redis → Redis Cluster, status: affected → modified)"),
            "in: {md}"
        );
        assert!(!md.contains("_No differences._"), "in: {md}");
    }

    #[test]
    fn edge_changes_reported() {
        let a = demo_doc();
        let mut b = demo_doc();
        b.edges.remove(0); // web-ui ==> api-gateway (data_flow)
        b.edges.push(crate::model::Edge {
            from: NodeId::from("web-ui"),
            to: NodeId::from("postgres"),
            kind: crate::model::EdgeKind::DependsOn,
            label: None,
            status: ElementStatus::Proposed,
            origin: crate::model::Origin::Agent,
        });
        let md = diff_docs("a", &a, "b", &b);
        assert!(md.contains("## Edges"), "in: {md}");
        assert!(
            md.contains("+ added: Web UI —depends_on→ PostgreSQL"),
            "in: {md}"
        );
        assert!(
            md.contains("- removed: Web UI —data_flow→ API Gateway"),
            "in: {md}"
        );
    }

    #[test]
    fn decision_drift_reported() {
        let a = demo_doc();
        let mut b = demo_doc();
        {
            let c = &mut b.node_mut(&NodeId::from("sync-engine")).unwrap().choices[0];
            c.selected = Some("crdt".into());
            c.status = ChoiceStatus::Decided;
        }
        let md = diff_docs("a", &a, "b", &b);
        assert!(md.contains("## Decisions"), "in: {md}");
        assert!(
            md.contains("newly decided: “How should concurrent edits be reconciled?” → CRDTs"),
            "in: {md}"
        );

        // Decided differently.
        let mut a2 = b.clone();
        {
            let c = &mut a2.node_mut(&NodeId::from("sync-engine")).unwrap().choices[0];
            c.selected = Some("ot".into());
        }
        let md = diff_docs("a", &a2, "b", &b);
        assert!(
            md.contains("decided differently: “How should concurrent edits be reconciled?” (a: Operational Transform, b: CRDTs)"),
            "in: {md}"
        );

        // Reopened: decided in a, open in b.
        let md = diff_docs("a", &b, "b", &a);
        assert!(
            md.contains("reopened: “How should concurrent edits be reconciled?”"),
            "in: {md}"
        );

        // Newly dismissed.
        let mut b2 = demo_doc();
        b2.node_mut(&NodeId::from("ws-gateway")).unwrap().choices[0].status =
            ChoiceStatus::Dismissed;
        let md = diff_docs("a", &a, "b", &b2);
        assert!(
            md.contains("newly dismissed: “Where should websocket connections terminate?”"),
            "in: {md}"
        );
    }

    #[test]
    fn diff_is_deterministic() {
        let a = demo_doc();
        let mut b = demo_doc();
        b.nodes.retain(|n| n.id.as_str() != "redis");
        b.edges
            .retain(|e| e.from.as_str() != "redis" && e.to.as_str() != "redis");
        assert_eq!(diff_docs("a", &a, "b", &b), diff_docs("a", &a, "b", &b));
    }
}
