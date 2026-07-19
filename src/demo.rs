//! Built-in demo document: `nodestorm --demo` render target and test fixture.

use crate::model::{
    BuildStatus, Choice, ChoiceOption, ChoiceRef, ChoiceStatus, Edge, EdgeKind, ElementStatus,
    Node, NodeId, NodeKind, Origin, Question, QuestionId, SessionDoc,
};

fn n(id: &str, label: &str, kind: NodeKind, status: ElementStatus, description: &str) -> Node {
    Node {
        id: NodeId::from(id),
        label: label.to_owned(),
        kind,
        description: description.to_owned(),
        status,
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

fn e(from: &str, to: &str, kind: EdgeKind, status: ElementStatus) -> Edge {
    Edge {
        from: NodeId::from(from),
        to: NodeId::from(to),
        kind,
        label: None,
        status,
        origin: Origin::Agent,
    }
}

fn opt(id: &str, label: &str, summary: &str, recommended: bool) -> ChoiceOption {
    ChoiceOption {
        id: id.into(),
        label: label.to_owned(),
        summary: summary.to_owned(),
        pros: vec![],
        cons: vec![],
        recommended,
        affects: vec![],
    }
}

/// A plausible mid-brainstorm session: adding realtime collaboration to a
/// note-taking app. Two open choices, ripple targets, one cycle.
pub fn demo_doc() -> SessionDoc {
    use EdgeKind::{Contains, DataFlow, DependsOn};
    use ElementStatus::{Affected, Existing, Modified, Proposed};
    use NodeKind::{Component, DataStore, External, Queue, Service, Ui};

    let mut nodes = vec![
        n(
            "web-ui",
            "Web UI",
            Ui,
            Existing,
            "React SPA for editing notes",
        ),
        n(
            "api-gateway",
            "API Gateway",
            Service,
            Existing,
            "Routes and authenticates all client traffic",
        ),
        n(
            "auth-service",
            "Auth Service",
            Service,
            Existing,
            "Sessions, tokens, permissions",
        ),
        n(
            "notes-service",
            "Notes Service",
            Service,
            Modified,
            "CRUD for notes; gains realtime hooks under this proposal",
        ),
        n(
            "sync-engine",
            "Sync Engine",
            Component,
            Proposed,
            "Merges concurrent edits from multiple clients",
        ),
        n(
            "ws-gateway",
            "WebSocket Gateway",
            Service,
            Proposed,
            "Long-lived connections pushing edits to clients",
        ),
        n(
            "postgres",
            "PostgreSQL",
            DataStore,
            Existing,
            "Primary storage for notes and users",
        ),
        n(
            "redis",
            "Redis",
            DataStore,
            Affected,
            "Session cache; may gain pub/sub fan-out duty",
        ),
        n(
            "search-index",
            "Search Index",
            Component,
            Existing,
            "Full-text search over notes",
        ),
        n(
            "email-provider",
            "Email Provider",
            External,
            Existing,
            "Transactional mail (share invites)",
        ),
        n(
            "job-queue",
            "Job Queue",
            Queue,
            Existing,
            "Background work: indexing, emails",
        ),
    ];

    // Choice 1: conflict resolution strategy, owned by the sync engine.
    let mut crdt = opt(
        "crdt",
        "CRDTs",
        "Conflict-free replicated data types (e.g. Yjs-style) merge automatically.",
        true,
    );
    crdt.pros = vec![
        "No central sequencing required".into(),
        "Offline edits merge cleanly".into(),
    ];
    crdt.cons = vec![
        "Document format changes (stored as CRDT state)".into(),
        "Larger payloads".into(),
    ];
    crdt.affects = vec![
        "notes-service".into(),
        "postgres".into(),
        "ws-gateway".into(),
    ];

    let mut ot = opt(
        "ot",
        "Operational Transform",
        "Central server transforms and sequences concurrent operations.",
        false,
    );
    ot.pros = vec![
        "Compact operations".into(),
        "Keeps current document format".into(),
    ];
    ot.cons = vec![
        "Server becomes a sequencing bottleneck".into(),
        "Transform functions are notoriously tricky".into(),
    ];
    ot.affects = vec!["ws-gateway".into(), "notes-service".into()];

    let mut lww = opt(
        "lww",
        "Last-write-wins",
        "Whole-note versioning; latest save replaces, with conflict warnings.",
        false,
    );
    lww.pros = vec!["Trivial to implement".into()];
    lww.cons = vec![
        "Concurrent edits lose data".into(),
        "Not really 'collaboration'".into(),
    ];
    lww.affects = vec!["notes-service".into()];

    nodes[4].choices.push(Choice {
        id: "conflict-resolution".into(),
        prompt: "How should concurrent edits be reconciled?".into(),
        rationale: Some(
            "Realtime collaboration means two clients editing one note at once; \
             the merge strategy shapes storage, transport, and the editor."
                .into(),
        ),
        options: vec![crdt, ot, lww],
        selected: None,
        status: ChoiceStatus::Open,
        depends_on: vec![],
        needs_review: false,
        reopen: false,
    });

    // Choice 2: where the websocket termination lives.
    let mut dedicated = opt(
        "dedicated",
        "Dedicated gateway service",
        "New service owning all realtime connections, scaled independently.",
        true,
    );
    dedicated.pros = vec![
        "Connection load isolated from request/response traffic".into(),
        "Independent deploys".into(),
    ];
    dedicated.cons = vec!["One more service to operate".into()];
    dedicated.affects = vec!["api-gateway".into(), "redis".into()];

    let mut inprocess = opt(
        "in-process",
        "Inside the API gateway",
        "Terminate websockets in the existing gateway process.",
        false,
    );
    inprocess.pros = vec!["No new service".into()];
    inprocess.cons = vec![
        "Gateway restarts drop every live connection".into(),
        "Mixed scaling profile".into(),
    ];
    inprocess.affects = vec!["api-gateway".into()];

    nodes[5].choices.push(Choice {
        id: "ws-deployment".into(),
        prompt: "Where should websocket connections terminate?".into(),
        rationale: None,
        options: vec![dedicated, inprocess],
        selected: None,
        status: ChoiceStatus::Open,
        // Where sockets terminate only matters once the merge strategy is
        // chosen — locked until conflict-resolution is decided.
        depends_on: vec![ChoiceRef {
            node: NodeId::from("sync-engine"),
            choice: "conflict-resolution".into(),
        }],
        needs_review: false,
        reopen: false,
    });

    // A little implementation progress to show the live build board.
    nodes[3].build = Some(BuildStatus::Building); // notes-service
    nodes[4].build = Some(BuildStatus::Planned); // sync-engine

    // Swimlanes: a tiered client → services → data → external arrangement.
    for (i, lane) in [
        (0, "Client"),
        (1, "Services"),
        (2, "Services"),
        (3, "Services"),
        (4, "Services"),
        (5, "Services"),
        (6, "Data"),
        (7, "Data"),
        (8, "Data"),
        (9, "External"),
        (10, "Services"),
    ] {
        nodes[i].lane = Some(lane.to_owned());
    }

    SessionDoc {
        version: SessionDoc::VERSION,
        title: "Realtime collaboration for the notes app".into(),
        revision: 0,
        focus: Some(NodeId::from("sync-engine")),
        nodes,
        edges: vec![
            e("web-ui", "api-gateway", DataFlow, Existing),
            e("web-ui", "ws-gateway", DataFlow, Proposed),
            e("api-gateway", "auth-service", DependsOn, Existing),
            e("api-gateway", "notes-service", DataFlow, Existing),
            e("ws-gateway", "sync-engine", DataFlow, Proposed),
            e("ws-gateway", "auth-service", DependsOn, Proposed),
            e("sync-engine", "notes-service", DataFlow, Proposed),
            e("sync-engine", "redis", DependsOn, Proposed),
            e("notes-service", "postgres", DependsOn, Existing),
            e("notes-service", "job-queue", DataFlow, Existing),
            e("job-queue", "search-index", DataFlow, Existing),
            e("job-queue", "email-provider", DataFlow, Existing),
            // Cycle on purpose: index rebuilds notify the notes service.
            e("search-index", "notes-service", DataFlow, Existing),
            e("auth-service", "postgres", DependsOn, Existing),
            e("auth-service", "redis", Contains, Existing),
        ],
        questions: vec![Question {
            id: QuestionId::from("history-retention"),
            prompt: "How long should full edit history be retained once notes go realtime?".into(),
            node_id: Some(NodeId::from("postgres")),
            rationale: Some(
                "CRDT/OT keep per-edit state — retention drives storage growth and \
                 compliance."
                    .into(),
            ),
            answer: None,
            answered_at: None,
        }],
        annotations: vec![],
    }
}

/// Deterministic large graph for scaling checks (`--demo-big N`): `n`
/// components in groups of 12, chained plus one cross-group link each.
pub fn big_doc(n: usize) -> SessionDoc {
    use EdgeKind::{DataFlow, DependsOn};
    use ElementStatus::Existing;
    use NodeKind::Component;

    let mut nodes = Vec::with_capacity(n);
    let mut edges = Vec::new();
    for i in 0..n {
        let mut node = self::n(
            &format!("node-{i}"),
            &format!("Component {i}"),
            Component,
            Existing,
            "",
        );
        node.group = Some(format!("cluster-{}", i / 12));
        nodes.push(node);
        if i > 0 {
            edges.push(e(
                &format!("node-{}", i - 1),
                &format!("node-{i}"),
                DependsOn,
                Existing,
            ));
        }
        if i >= 12 {
            edges.push(e(
                &format!("node-{}", i - 12),
                &format!("node-{i}"),
                DataFlow,
                Existing,
            ));
        }
    }
    SessionDoc {
        version: SessionDoc::VERSION,
        title: format!("big demo ({n} components)"),
        revision: 0,
        focus: None,
        nodes,
        edges,
        questions: vec![],
        annotations: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_doc_is_valid_and_deterministic() {
        let d = big_doc(200);
        assert_eq!(d.nodes.len(), 200);
        let v = d.validate();
        assert!(v.is_ok(), "errors: {:?}", v.errors);
        assert_eq!(big_doc(200), big_doc(200));
        let layout = crate::layout::compute(&d);
        assert_eq!(layout.rects.len(), 200);
    }

    #[test]
    fn big_doc_connects_adjacent_nodes_and_groups_at_the_boundary() {
        let d = big_doc(13);
        assert_eq!(d.edges.len(), 13);
        assert!(d.edges.iter().any(|edge| {
            edge.from == NodeId::from("node-0")
                && edge.to == NodeId::from("node-1")
                && edge.kind == EdgeKind::DependsOn
        }));
        assert!(d.edges.iter().any(|edge| {
            edge.from == NodeId::from("node-0")
                && edge.to == NodeId::from("node-12")
                && edge.kind == EdgeKind::DataFlow
        }));
    }

    #[test]
    fn demo_doc_is_valid() {
        let v = demo_doc().validate();
        assert!(v.is_ok(), "errors: {:?}", v.errors);
        assert!(v.warnings.is_empty(), "warnings: {:?}", v.warnings);
    }

    #[test]
    fn demo_doc_has_open_choices_and_a_cycle() {
        let doc = demo_doc();
        assert_eq!(doc.open_choice_count(), 2);
        let layout = crate::layout::compute(&doc);
        assert_eq!(layout.rects.len(), doc.nodes.len());
    }
}
