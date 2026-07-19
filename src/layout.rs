//! Deterministic layered ("Sugiyama-lite") auto-layout.
//!
//! Left-to-right layered layout for a mostly-DAG architecture graph:
//! 1. break cycles by ignoring DFS back-edges,
//! 2. rank by longest path from sources (x = rank),
//! 3. order within ranks by a few barycenter sweeps,
//! 4. pack each rank vertically using estimated card heights.
//!
//! User-dragged nodes are *pinned*: they keep their exact position, are
//! excluded from packing, and auto-placed nodes are nudged off them. Every
//! tie-break uses document order, so identical inputs give identical output.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::model::{Edge, EdgeKind, ElementStatus, Node, NodeId, Origin, Point, SessionDoc};

pub const CARD_WIDTH: f64 = 260.0;
pub const RANK_GUTTER: f64 = 120.0;
pub const NODE_GAP: f64 = 24.0;
/// Vertical spread between edge anchors sharing one card side.
const PORT_SPREAD: f64 = 14.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn center_y(&self) -> f64 {
        self.y + self.h / 2.0
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

/// A rendered edge: SVG path data plus styling inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgePath {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub status: ElementStatus,
    pub label: Option<String>,
    /// SVG `d` attribute (cubic bezier).
    pub path: String,
    pub label_pos: Point,
    /// How many document edges this path aggregates (>1 only between
    /// collapsed clusters and their neighbors; rendered thicker with `×N`).
    pub bundle_count: usize,
}

/// A collapsed group rendered as one card.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterRect {
    pub group: String,
    pub rect: Rect,
    pub member_count: usize,
}

/// A horizontal swimlane band spanning the graph width. Present when a node
/// carries a `lane` or the user declared the lane; the layered layout
/// confines each lane's cards to its band. The unlabeled default lane (nodes
/// without a `lane`) draws no band.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneBand {
    pub label: String,
    /// The band: the base strip grown down/sideways to enclose the lane's
    /// pinned members. Drawn, hit-tested, and highlighted as one rect, so
    /// what the user sees is exactly the drop zone. Bands never overlap —
    /// each starts at least `LANE_SEP` below the grown band above it.
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub rects: HashMap<NodeId, Rect>,
    pub edges: Vec<EdgePath>,
    /// Collapsed groups, one synthetic card each (`group:<name>` ids inside
    /// `edges`; never leaked into the doc).
    pub clusters: Vec<ClusterRect>,
    /// Swimlane bands (empty unless the graph uses lanes), in lane order.
    pub lanes: Vec<LaneBand>,
    /// Union of all card rects, padded — used for zoom-to-fit.
    pub bounds: Rect,
}

/// What survives viewport culling at the current transform: node ids and
/// `layout.edges`/`layout.clusters` indices whose rects intersect the view
/// expanded by one screen of margin in every direction.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VisibleSet {
    pub nodes: std::collections::BTreeSet<NodeId>,
    pub edges: Vec<usize>,
    pub clusters: Vec<usize>,
}

/// Pure culling math (`tx`/`ty`/`scale` are the canvas transform).
pub fn visible_set(
    layout: &Layout,
    tx: f64,
    ty: f64,
    scale: f64,
    view_w: f64,
    view_h: f64,
) -> VisibleSet {
    let view = Rect {
        x: -tx / scale - view_w / scale,
        y: -ty / scale - view_h / scale,
        w: 3.0 * view_w / scale,
        h: 3.0 * view_h / scale,
    };
    let mut out = VisibleSet::default();
    for (id, rect) in &layout.rects {
        if rect.intersects(&view) {
            out.nodes.insert(id.clone());
        }
    }
    for (i, cluster) in layout.clusters.iter().enumerate() {
        if cluster.rect.intersects(&view) {
            out.clusters.push(i);
        }
    }
    let visible_cluster_ids: std::collections::BTreeSet<String> = out
        .clusters
        .iter()
        .map(|&i| format!("group:{}", layout.clusters[i].group))
        .collect();
    for (i, e) in layout.edges.iter().enumerate() {
        let end_visible =
            |id: &NodeId| out.nodes.contains(id) || visible_cluster_ids.contains(id.as_str());
        if end_visible(&e.from) || end_visible(&e.to) {
            out.edges.push(i);
        }
    }
    out
}

/// Estimated Near-tier card height in px. Must stay consistent with
/// `assets/main.css` (see the "node cards" geometry comment there): 20px
/// chrome (2px border + 18px vertical padding), head row min 24px with 18px
/// per label line (clamped to 3), 20px meta row, description 4px margin +
/// 18px per line (clamped to 4), 28px badge row. `open_questions` is the
/// node's unanswered-question count — those badges come from the doc, not
/// the node, so the caller supplies them.
pub fn estimate_height(node: &Node, open_questions: usize) -> f64 {
    const CHROME: f64 = 20.0; // 2px border + 8px top / 10px bottom padding
    const HEAD_MIN: f64 = 24.0;
    const META_H: f64 = 20.0;
    const LINE_H: f64 = 18.0;
    // Label: 13.5px Space Grotesk in 232px minus the 13px kind glyph + 8px
    // gap. Description: 12.5px body font across the full 232px.
    let label_lines = wrap_lines(&node.label, 211.0, 1.0).min(3);
    let head = (label_lines as f64 * LINE_H).max(HEAD_MIN);
    let desc = if node.description.is_empty() {
        0.0
    } else {
        4.0 + wrap_lines(&node.description, 232.0, 12.5 / 13.5).min(4) as f64 * LINE_H
    };
    let badge_row = if open_questions > 0
        || node.open_choice_count() > 0
        || node.choices.iter().any(|c| !c.is_open())
        || !node.notes.is_empty()
    {
        28.0
    } else {
        0.0
    };
    CHROME + head + META_H + desc + badge_row
}

/// Per-node estimated heights, including unanswered-question badge rows.
fn node_heights(doc: &SessionDoc) -> Vec<f64> {
    let mut open_q: HashMap<&NodeId, usize> = HashMap::new();
    for q in &doc.questions {
        if let Some(id) = &q.node_id
            && !q.is_answered()
        {
            *open_q.entry(id).or_default() += 1;
        }
    }
    doc.nodes
        .iter()
        .map(|n| estimate_height(n, open_q.get(&n.id).copied().unwrap_or(0)))
        .collect()
}

/// Approximate advance width in px of `c` at the 13.5px label size. Flat
/// per-character counts undercount wide glyphs (`W`-heavy or all-caps
/// labels wrap earlier than 1/22th of the width each), which would clip
/// content now that cards are max-height-clamped to the estimate — so bin
/// glyphs by width class instead. Leaning wide is the safe direction: it
/// only makes a band slightly taller.
fn char_w(c: char) -> f64 {
    match c {
        'i' | 'j' | 'l' | '.' | ',' | '\'' | '|' | '!' | ':' | ';' => 4.5,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '-' | '/' => 5.5,
        'm' | 'w' => 11.5,
        'M' | 'W' | '@' => 13.0,
        c if c.is_uppercase() || c.is_ascii_digit() => 9.5,
        _ => 7.5,
    }
}

/// Greedy word-wrap line count inside a `budget`-px line, with glyph widths
/// scaled by `scale` (1.0 = the 13.5px label size). Mirrors CSS
/// `overflow-wrap: anywhere`: over-long single words wrap mid-word.
fn wrap_lines(text: &str, budget: f64, scale: f64) -> usize {
    let space = 4.0 * scale;
    let mut lines = 1usize;
    let mut used = 0.0f64;
    for word in text.split_whitespace() {
        let w: f64 = word.chars().map(char_w).sum::<f64>() * scale;
        if used == 0.0 {
            used = w;
        } else if used + space + w <= budget {
            used += space + w;
        } else {
            lines += 1;
            used = w;
        }
        while used > budget {
            lines += 1;
            used -= budget;
        }
    }
    lines
}

pub fn compute(doc: &SessionDoc) -> Layout {
    compute_view(doc, &BTreeSet::new(), &[])
}

/// Layout with named groups collapsed. See [`compute_view`].
pub fn compute_collapsed(doc: &SessionDoc, collapsed: &BTreeSet<String>) -> Layout {
    compute_view(doc, collapsed, &[])
}

/// Layout with named groups collapsed and user-declared swimlanes. Each
/// collapsed group becomes one synthetic `group:<name>` node, member cards
/// disappear, and edges re-route to the cluster (parallel edges onto one
/// cluster merge into a `bundle_count`ed path labeled `×N`); group-internal
/// edges vanish. `declared` lists lanes the user created (including empty
/// ones) in display order; lanes only referenced by `node.lane` are appended
/// after, in first-appearance order, then the unlabeled default lane last.
pub fn compute_view(doc: &SessionDoc, collapsed: &BTreeSet<String>, declared: &[String]) -> Layout {
    if collapsed.is_empty() {
        return compute_inner(doc, &[], declared);
    }

    // Which groups actually have members, in first-appearance order.
    let mut group_order: Vec<&str> = Vec::new();
    let mut member_count: HashMap<&str, usize> = HashMap::new();
    for node in &doc.nodes {
        if let Some(g) = node.group.as_deref()
            && collapsed.contains(g)
        {
            if !group_order.contains(&g) {
                group_order.push(g);
            }
            *member_count.entry(g).or_default() += 1;
        }
    }

    let synthetic_id = |g: &str| NodeId::new(format!("group:{g}"));
    let map_end = |id: &NodeId| -> NodeId {
        doc.node(id)
            .and_then(|n| n.group.as_deref())
            .filter(|g| collapsed.contains(*g))
            .map_or_else(|| id.clone(), synthetic_id)
    };

    let mut nodes: Vec<Node> = Vec::new();
    let mut emitted_groups: HashSet<&str> = HashSet::new();
    for node in &doc.nodes {
        match node.group.as_deref().filter(|g| collapsed.contains(*g)) {
            None => nodes.push(node.clone()),
            Some(g) => {
                // The first member pulls the whole group in as one card.
                if emitted_groups.insert(g) {
                    nodes.push(Node {
                        id: synthetic_id(g),
                        label: g.to_owned(),
                        kind: crate::model::NodeKind::Module,
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
                    });
                }
            }
        }
    }

    // Re-map + bundle edges (insertion order = document order).
    let mut syn_edges: Vec<Edge> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();
    let mut slot: HashMap<(NodeId, NodeId, EdgeKind), usize> = HashMap::new();
    for e in &doc.edges {
        let from = map_end(&e.from);
        let to = map_end(&e.to);
        if from == to {
            continue; // group-internal (or degenerate) — nothing to draw
        }
        match slot.entry((from.clone(), to.clone(), e.kind)) {
            std::collections::hash_map::Entry::Occupied(o) => counts[*o.get()] += 1,
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(syn_edges.len());
                syn_edges.push(Edge {
                    from,
                    to,
                    kind: e.kind,
                    label: e.label.clone(),
                    status: e.status,
                    origin: e.origin,
                });
                counts.push(1);
            }
        }
    }
    for (i, &c) in counts.iter().enumerate() {
        if c > 1 {
            syn_edges[i].label = Some(format!("×{c}"));
        }
    }

    let syn_doc = SessionDoc {
        version: doc.version,
        title: doc.title.clone(),
        revision: doc.revision,
        focus: None,
        nodes,
        edges: syn_edges,
        // Kept so question badges still count toward visible cards' heights;
        // refs to collapsed members simply match no synthetic node.
        questions: doc.questions.clone(),
        annotations: vec![],
    };
    let mut layout = compute_inner(&syn_doc, &counts, declared);

    let mut clusters = Vec::new();
    for g in group_order {
        if let Some(rect) = layout.rects.remove(&synthetic_id(g)) {
            clusters.push(ClusterRect {
                group: g.to_owned(),
                rect,
                member_count: member_count[g],
            });
        }
    }
    layout.clusters = clusters;
    layout
}

/// `counts[i]` is the bundle size of `doc.edges[i]` (empty slice → all 1).
fn compute_inner(doc: &SessionDoc, counts: &[usize], declared: &[String]) -> Layout {
    let index: HashMap<&NodeId, usize> = doc
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (&n.id, i))
        .collect();
    let n = doc.nodes.len();
    if n == 0 {
        return Layout::default();
    }

    // Adjacency over unique, non-self edges with both endpoints present.
    let mut fwd: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut seen = HashSet::new();
    for e in &doc.edges {
        let (Some(&a), Some(&b)) = (index.get(&e.from), index.get(&e.to)) else {
            continue;
        };
        if a != b && seen.insert((a, b)) {
            fwd[a].push(b);
        }
    }
    for adj in &mut fwd {
        adj.sort_unstable();
    }

    let dag = break_cycles(&fwd);
    let ranks = longest_path_ranks(&dag);
    let order = barycenter_order(&dag, &ranks);
    let heights = node_heights(doc);
    // Swimlanes constrain vertical packing into labeled bands; without any
    // `lane` the layout is unchanged (centered ranks).
    let (rects, lanes) = if declared.is_empty() && !doc.nodes.iter().any(|n| n.lane.is_some()) {
        (place(doc, &ranks, &order, &heights), Vec::new())
    } else {
        place_laned(doc, &ranks, &order, declared, &heights)
    };
    let rank_of: HashMap<&NodeId, usize> = doc
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (&n.id, ranks[i]))
        .collect();
    let edges = route_edges(doc, &rects, counts, &rank_of);
    let mut bounds = compute_bounds(&rects);
    // Lane bands extend past the cards horizontally; include them in the fit.
    for lane in &lanes {
        bounds = union_rect(bounds, lane.rect);
    }

    Layout {
        rects,
        edges,
        clusters: Vec::new(),
        lanes,
        bounds,
    }
}

/// Returns the acyclic adjacency: DFS (deterministic order) drops back-edges.
fn break_cycles(fwd: &[Vec<usize>]) -> Vec<Vec<usize>> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        OnStack,
        Done,
    }
    let n = fwd.len();
    let mut state = vec![State::Unvisited; n];
    let mut dag: Vec<Vec<usize>> = vec![Vec::new(); n];
    // Iterative DFS to survive deep graphs.
    for root in 0..n {
        if state[root] != State::Unvisited {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        state[root] = State::OnStack;
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            if *next < fwd[node].len() {
                let child = fwd[node][*next];
                *next += 1;
                match state[child] {
                    State::OnStack => {} // back-edge: drop from the DAG
                    State::Done => dag[node].push(child),
                    State::Unvisited => {
                        dag[node].push(child);
                        state[child] = State::OnStack;
                        stack.push((child, 0));
                    }
                }
            } else {
                state[node] = State::Done;
                stack.pop();
            }
        }
    }
    dag
}

/// Longest-path ranking over an acyclic adjacency (Kahn order).
fn longest_path_ranks(dag: &[Vec<usize>]) -> Vec<usize> {
    let n = dag.len();
    let mut indegree = vec![0usize; n];
    for adj in dag {
        for &b in adj {
            indegree[b] += 1;
        }
    }
    let mut rank = vec![0usize; n];
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|&i| indegree[i] == 0).collect();
    while let Some(a) = queue.pop_front() {
        for &b in &dag[a] {
            rank[b] = rank[b].max(rank[a] + 1);
            indegree[b] -= 1;
            if indegree[b] == 0 {
                queue.push_back(b);
            }
        }
    }
    rank
}

/// Order nodes within each rank by repeated barycenter sweeps.
/// Returns `order[rank]` = node indices top-to-bottom.
fn barycenter_order(dag: &[Vec<usize>], ranks: &[usize]) -> Vec<Vec<usize>> {
    let n = dag.len();
    let max_rank = ranks.iter().copied().max().unwrap_or(0);
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for i in 0..n {
        order[ranks[i]].push(i); // document order initially
    }
    let mut back: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (a, adj) in dag.iter().enumerate() {
        for &b in adj {
            back[b].push(a);
        }
    }

    let mut pos = vec![0usize; n];
    let reindex = |order: &[Vec<usize>], pos: &mut [usize]| {
        for rank_nodes in order {
            for (p, &i) in rank_nodes.iter().enumerate() {
                pos[i] = p;
            }
        }
    };
    reindex(&order, &mut pos);

    for sweep in 0..4 {
        let downward = sweep % 2 == 0;
        let rank_range: Vec<usize> = if downward {
            (0..=max_rank).collect()
        } else {
            (0..=max_rank).rev().collect()
        };
        for r in rank_range {
            let neighbors = |i: usize| -> &[usize] { if downward { &back[i] } else { &dag[i] } };
            let mut keyed: Vec<(f64, usize, usize)> = order[r]
                .iter()
                .map(|&i| {
                    let ns = neighbors(i);
                    let bary = if ns.is_empty() {
                        pos[i] as f64
                    } else {
                        ns.iter().map(|&x| pos[x] as f64).sum::<f64>() / ns.len() as f64
                    };
                    (bary, pos[i], i)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            order[r] = keyed.into_iter().map(|(_, _, i)| i).collect();
            reindex(&order, &mut pos);
        }
    }
    order
}

/// Assign rects: ranks map to x columns, ranks pack vertically centered on 0.
/// Pinned nodes keep their user position; auto nodes are nudged off them.
fn place(
    doc: &SessionDoc,
    ranks: &[usize],
    order: &[Vec<usize>],
    heights: &[f64],
) -> HashMap<NodeId, Rect> {
    let mut rects: HashMap<NodeId, Rect> = HashMap::new();

    let pinned: Vec<usize> = (0..doc.nodes.len())
        .filter(|&i| doc.nodes[i].position.is_some())
        .collect();
    for &i in &pinned {
        let p = doc.nodes[i].position.expect("filtered on is_some");
        rects.insert(
            doc.nodes[i].id.clone(),
            Rect {
                x: p.x,
                y: p.y,
                w: CARD_WIDTH,
                h: heights[i],
            },
        );
    }

    let pinned_rects: Vec<Rect> = pinned.iter().map(|i| rects[&doc.nodes[*i].id]).collect();
    for (r, rank_nodes) in order.iter().enumerate() {
        let auto: Vec<usize> = rank_nodes
            .iter()
            .copied()
            .filter(|i| doc.nodes[*i].position.is_none())
            .collect();
        if auto.is_empty() {
            continue;
        }
        let total: f64 =
            auto.iter().map(|&i| heights[i]).sum::<f64>() + NODE_GAP * (auto.len() - 1) as f64;
        let x = r as f64 * (CARD_WIDTH + RANK_GUTTER);
        let mut y = -total / 2.0;
        for &i in &auto {
            let mut rect = Rect {
                x,
                y,
                w: CARD_WIDTH,
                h: heights[i],
            };
            // Deterministic downward nudge off pinned cards.
            let mut moved = true;
            while moved {
                moved = false;
                for p in &pinned_rects {
                    if rect.intersects(p) {
                        rect.y = p.y + p.h + NODE_GAP;
                        moved = true;
                    }
                }
            }
            y = rect.y + rect.h + NODE_GAP;
            rects.insert(doc.nodes[i].id.clone(), rect);
        }
        debug_assert!(ranks.len() >= rank_nodes.len());
    }
    rects
}

/// Union of two rects.
fn union_rect(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.w).max(b.x + b.w);
    let bottom = (a.y + a.h).max(b.y + b.h);
    Rect {
        x,
        y,
        w: right - x,
        h: bottom - y,
    }
}

const LANE_CARD_PAD: f64 = 20.0;
const LANE_TITLE_H: f64 = 36.0;
/// Minimum vertical gap between adjacent bands; bands never overlap.
const LANE_SEP: f64 = 18.0;
const LANE_HPAD: f64 = 30.0;
/// Minimum labeled-band height: title + one minimal card + padding, so an
/// empty lane is a comfortable drop target.
const LANE_MIN_H: f64 = 110.0;

/// Lane-constrained placement: cards pack into labeled horizontal bands by
/// their `lane` (the unlabeled default band holds nodes without one). Bands
/// stack top-to-bottom with declared lanes first (in declared order), then
/// lanes only referenced by nodes (first appearance), then the default lane
/// last; within a (lane, rank) cell cards keep their barycenter order. Pinned
/// nodes keep their exact position and sit outside the bands. Returns the
/// rects plus the visible (labeled) bands.
fn place_laned(
    doc: &SessionDoc,
    _ranks: &[usize],
    order: &[Vec<usize>],
    declared: &[String],
    heights: &[f64],
) -> (HashMap<NodeId, Rect>, Vec<LaneBand>) {
    let lane_key = |i: usize| doc.nodes[i].lane.clone().unwrap_or_default();

    // Lane order: declared lanes first (in the user's order), then lanes only
    // referenced by nodes (first appearance), then the unlabeled default last.
    let mut lane_order: Vec<String> = Vec::new();
    let mut lane_index: HashMap<String, usize> = HashMap::new();
    let push_lane = |order: &mut Vec<String>, index: &mut HashMap<String, usize>, key: String| {
        if !index.contains_key(&key) {
            index.insert(key.clone(), order.len());
            order.push(key);
        }
    };
    for label in declared {
        push_lane(&mut lane_order, &mut lane_index, label.clone());
    }
    for i in 0..doc.nodes.len() {
        let key = lane_key(i);
        if !key.is_empty() {
            push_lane(&mut lane_order, &mut lane_index, key);
        }
    }
    // The default lane is appended only if some node has no lane.
    if doc.nodes.iter().any(|n| n.lane.is_none()) {
        push_lane(&mut lane_order, &mut lane_index, String::new());
    }
    let num_lanes = lane_order.len();
    let num_ranks = order.len();
    let top_pad: Vec<f64> = lane_order
        .iter()
        .map(|label| {
            if label.is_empty() {
                LANE_CARD_PAD
            } else {
                LANE_TITLE_H
            }
        })
        .collect();

    // Band height = tallest (lane, rank) stack over the ranks.
    let mut lane_stack_h = vec![0.0f64; num_lanes];
    for rank_nodes in order {
        let mut per_lane = vec![0.0f64; num_lanes];
        for &i in rank_nodes {
            if doc.nodes[i].position.is_some() {
                continue;
            }
            let li = lane_index[&lane_key(i)];
            if per_lane[li] > 0.0 {
                per_lane[li] += NODE_GAP;
            }
            per_lane[li] += heights[i];
        }
        for (li, h) in per_lane.iter().enumerate() {
            lane_stack_h[li] = lane_stack_h[li].max(*h);
        }
    }

    let graph_w = if num_ranks == 0 {
        CARD_WIDTH
    } else {
        (num_ranks - 1) as f64 * (CARD_WIDTH + RANK_GUTTER) + CARD_WIDTH
    };

    // Pinned cards first: the bands below must grow to enclose them.
    let mut rects: HashMap<NodeId, Rect> = HashMap::new();
    for (i, node) in doc.nodes.iter().enumerate() {
        if let Some(p) = node.position {
            rects.insert(
                node.id.clone(),
                Rect {
                    x: p.x,
                    y: p.y,
                    w: CARD_WIDTH,
                    h: heights[i],
                },
            );
        }
    }

    // Stack bands top-to-bottom. Each labeled band grows downward and
    // sideways to enclose its pinned members, and the next band starts below
    // the grown bottom — bands never overlap and stay LANE_SEP apart.
    let mut lane_y = vec![0.0f64; num_lanes];
    let mut band = vec![
        Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        };
        num_lanes
    ];
    let mut acc = 0.0;
    for li in 0..num_lanes {
        lane_y[li] = acc;
        let label = &lane_order[li];
        let mut h = lane_stack_h[li] + top_pad[li] + LANE_CARD_PAD;
        if !label.is_empty() {
            h = h.max(LANE_MIN_H);
        }
        let mut r = Rect {
            x: -LANE_HPAD,
            y: acc,
            w: graph_w + 2.0 * LANE_HPAD,
            h,
        };
        if !label.is_empty() {
            for (i, node) in doc.nodes.iter().enumerate() {
                if node.position.is_some() && lane_key(i) == *label {
                    // A member stranded above the band (bands shifted down
                    // under it) is pulled down inside, below the title — the
                    // band's top edge never moves up into the band above.
                    let m = rects.get_mut(&node.id).expect("pinned rect placed");
                    m.y = m.y.max(r.y + top_pad[li]);
                    let m = *m;
                    let x0 = r.x.min(m.x - LANE_CARD_PAD);
                    let x1 = (r.x + r.w).max(m.x + m.w + LANE_CARD_PAD);
                    let y1 = (r.y + r.h).max(m.y + m.h + LANE_CARD_PAD);
                    r = Rect {
                        x: x0,
                        y: r.y,
                        w: x1 - x0,
                        h: y1 - r.y,
                    };
                }
            }
        }
        band[li] = r;
        acc = r.y + r.h + LANE_SEP;
    }

    for (r, rank_nodes) in order.iter().enumerate() {
        let x = r as f64 * (CARD_WIDTH + RANK_GUTTER);
        let mut cursor: Vec<f64> = lane_y
            .iter()
            .zip(&top_pad)
            .map(|(y, pad)| y + pad)
            .collect();
        for &i in rank_nodes {
            if doc.nodes[i].position.is_some() {
                continue;
            }
            let li = lane_index[&lane_key(i)];
            let rect = Rect {
                x,
                y: cursor[li],
                w: CARD_WIDTH,
                h: heights[i],
            };
            cursor[li] += heights[i] + NODE_GAP;
            rects.insert(doc.nodes[i].id.clone(), rect);
        }
    }

    let mut lanes: Vec<LaneBand> = Vec::new();
    for (li, label) in lane_order.iter().enumerate() {
        if label.is_empty() {
            continue; // the default lane draws no band
        }
        lanes.push(LaneBand {
            label: label.clone(),
            rect: band[li],
        });
    }
    (rects, lanes)
}

/// The lane whose band contains `(x, y)` in plane coords, or `None` if the
/// point is outside every band (the drop-outside case).
pub fn lane_at(lanes: &[LaneBand], x: f64, y: f64) -> Option<String> {
    lanes
        .iter()
        .find(|l| {
            x >= l.rect.x && x < l.rect.x + l.rect.w && y >= l.rect.y && y < l.rect.y + l.rect.h
        })
        .map(|l| l.label.clone())
}

/// Vertical spacing between channel lanes for rank-spanning edges.
const LANE_GAP: f64 = 12.0;
/// Clearance kept between a routed corridor and any card it passes.
const CORRIDOR_CLEAR: f64 = 18.0;
/// Horizontal swing a channel edge uses to enter/leave its corridor.
const CHANNEL_SWING: f64 = 60.0;

/// Pick a horizontal corridor near `desired` that crosses no card between
/// `x_lo` and `x_hi`: the blocked y-intervals (cards grown by `margin`) are
/// merged and `desired` snaps to the nearest free boundary.
fn clear_corridor_y<'a>(
    desired: f64,
    x_lo: f64,
    x_hi: f64,
    margin: f64,
    rects: impl Iterator<Item = &'a Rect>,
) -> f64 {
    let mut blocked: Vec<(f64, f64)> = rects
        .filter(|r| r.x < x_hi && x_lo < r.x + r.w)
        .map(|r| (r.y - margin, r.y + r.h + margin))
        .collect();
    blocked.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for iv in blocked {
        match merged.last_mut() {
            Some(last) if iv.0 <= last.1 => last.1 = last.1.max(iv.1),
            _ => merged.push(iv),
        }
    }
    for (lo, hi) in merged {
        if desired > lo && desired < hi {
            return if desired - lo <= hi - desired { lo } else { hi };
        }
    }
    desired
}

/// Cubic bezier from the right edge of `from` to the left edge of `to`.
/// Port anchors on each card side are sorted by the far endpoint's y (not
/// document order), so edges fan out without crossing right at the card.
/// Rank-spanning edges are channel-routed: each `(from_rank, to_rank)` group
/// shares a horizontal corridor snapped clear of every card it would cross,
/// one lane per edge (document order). Backward (cycle) edges spanning ranks
/// get the mirrored corridor treatment instead of a blind bulge.
/// `counts[i]` is the bundle size of `doc.edges[i]` (missing → 1).
fn route_edges(
    doc: &SessionDoc,
    rects: &HashMap<NodeId, Rect>,
    counts: &[usize],
    rank_of: &HashMap<&NodeId, usize>,
) -> Vec<EdgePath> {
    // Channel groups keyed by the rank pair: forward edges spanning more
    // than one rank, and every rank-backward edge.
    let mut fwd_groups: std::collections::BTreeMap<(usize, usize), Vec<usize>> =
        std::collections::BTreeMap::new();
    let mut back_groups: std::collections::BTreeMap<(usize, usize), Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, e) in doc.edges.iter().enumerate() {
        let (Some(&ra), Some(&rb)) = (rank_of.get(&e.from), rank_of.get(&e.to)) else {
            continue;
        };
        if !rects.contains_key(&e.from) || !rects.contains_key(&e.to) {
            continue;
        }
        if rb > ra + 1 {
            fwd_groups.entry((ra, rb)).or_default().push(i);
        } else if rb < ra {
            back_groups.entry((rb, ra)).or_default().push(i);
        }
    }
    // edge index → (lane, group size, corridor y), per direction.
    let mut chan_fwd: HashMap<usize, (usize, usize, f64)> = HashMap::new();
    let mut chan_back: HashMap<usize, (usize, usize, f64)> = HashMap::new();
    for (groups, chan, forward) in [
        (&fwd_groups, &mut chan_fwd, true),
        (&back_groups, &mut chan_back, false),
    ] {
        for idxs in groups.values() {
            let mut x_lo = f64::MAX;
            let mut x_hi = f64::MIN;
            let mut mean = 0.0;
            for &i in idxs {
                let e = &doc.edges[i];
                let (a, b) = (&rects[&e.from], &rects[&e.to]);
                // The corridor spans the gap between the two cards' facing
                // edges, inset by the swing on each side.
                let (lo, hi) = if forward {
                    (a.x + a.w, b.x)
                } else {
                    (b.x + b.w, a.x)
                };
                x_lo = x_lo.min(lo + CHANNEL_SWING);
                x_hi = x_hi.max(hi - CHANNEL_SWING);
                mean += (a.center_y() + b.center_y()) / 2.0;
            }
            mean /= idxs.len() as f64;
            let margin = CORRIDOR_CLEAR + (idxs.len() as f64 - 1.0) / 2.0 * LANE_GAP;
            let base = clear_corridor_y(mean, x_lo, x_hi, margin, rects.values());
            for (lane, &i) in idxs.iter().enumerate() {
                chan.insert(i, (lane, idxs.len(), base));
            }
        }
    }
    // Port anchors: each card side hands out slots ordered by the far
    // endpoint's y, so fan-out at the card matches where edges are headed.
    let mut out_order: HashMap<&NodeId, Vec<usize>> = HashMap::new();
    let mut in_order: HashMap<&NodeId, Vec<usize>> = HashMap::new();
    for (i, e) in doc.edges.iter().enumerate() {
        if rects.contains_key(&e.from) && rects.contains_key(&e.to) {
            out_order.entry(&e.from).or_default().push(i);
            in_order.entry(&e.to).or_default().push(i);
        }
    }
    let far_y = |order: &mut HashMap<&NodeId, Vec<usize>>, from_side: bool| {
        for list in order.values_mut() {
            list.sort_by(|&x, &y| {
                let far = |i: usize| {
                    let e = &doc.edges[i];
                    rects[if from_side { &e.to } else { &e.from }].center_y()
                };
                far(x).total_cmp(&far(y)).then(x.cmp(&y))
            });
        }
    };
    far_y(&mut out_order, true);
    far_y(&mut in_order, false);
    let mut out_slot = vec![(0usize, 0usize); doc.edges.len()];
    let mut in_slot = vec![(0usize, 0usize); doc.edges.len()];
    for list in out_order.values() {
        for (s, &i) in list.iter().enumerate() {
            out_slot[i] = (s, list.len());
        }
    }
    for list in in_order.values() {
        for (s, &i) in list.iter().enumerate() {
            in_slot[i] = (s, list.len());
        }
    }
    let spread = |(slot, total): (usize, usize)| -> f64 {
        (slot as f64 - (total as f64 - 1.0) / 2.0) * PORT_SPREAD
    };

    doc.edges
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let a = rects.get(&e.from)?;
            let b = rects.get(&e.to)?;
            // Forward edges leave the right side and enter the left side;
            // backward (cycle) edges flip sides so the curve stays between
            // the two cards instead of ballooning past the graph.
            let backward = b.x + b.w / 2.0 < a.x + a.w / 2.0;
            let y1 = (a.center_y() + spread(out_slot[i])).clamp(a.y + 8.0, a.y + a.h - 8.0);
            let y2 = (b.center_y() + spread(in_slot[i])).clamp(b.y + 8.0, b.y + b.h - 8.0);
            let (x1, x2, c1, c2) = if backward {
                let x1 = a.x;
                let x2 = b.x + b.w;
                let dx = ((x1 - x2) / 2.0).abs().max(60.0);
                (x1, x2, x1 - dx, x2 + dx)
            } else {
                let x1 = a.x + a.w;
                let x2 = b.x;
                let dx = ((x2 - x1) / 2.0).abs().max(60.0);
                (x1, x2, x1 + dx, x2 - dx)
            };
            let corridor = |lane: usize, n: usize, base: f64| -> f64 {
                base + (lane as f64 - (n as f64 - 1.0) / 2.0) * LANE_GAP
            };
            let fwd_chan = chan_fwd
                .get(&i)
                .filter(|_| !backward && x2 - x1 >= 2.0 * CHANNEL_SWING);
            let back_chan = chan_back
                .get(&i)
                .filter(|_| backward && x1 - x2 >= 2.0 * CHANNEL_SWING);
            let (path, label_pos) = if let Some(&(lane, n, base)) = fwd_chan {
                // Channel routing: swing into the shared corridor, run
                // horizontally through card-free space, swing out at the
                // target.
                let cy = corridor(lane, n, base);
                let gx1 = x1 + CHANNEL_SWING;
                let gx2 = x2 - CHANNEL_SWING;
                (
                    format!(
                        "M {x1:.1} {y1:.1} C {:.1} {y1:.1} {:.1} {cy:.1} {gx1:.1} {cy:.1} \
                         L {gx2:.1} {cy:.1} \
                         C {:.1} {cy:.1} {:.1} {y2:.1} {x2:.1} {y2:.1}",
                        x1 + 30.0,
                        gx1 - 30.0,
                        gx2 + 30.0,
                        x2 - 30.0,
                    ),
                    Point {
                        x: (gx1 + gx2) / 2.0,
                        y: cy - 8.0,
                    },
                )
            } else if let Some(&(lane, n, base)) = back_chan {
                // Mirrored channel: leftward corridor between the cards.
                let cy = corridor(lane, n, base);
                let gx1 = x1 - CHANNEL_SWING;
                let gx2 = x2 + CHANNEL_SWING;
                (
                    format!(
                        "M {x1:.1} {y1:.1} C {:.1} {y1:.1} {:.1} {cy:.1} {gx1:.1} {cy:.1} \
                         L {gx2:.1} {cy:.1} \
                         C {:.1} {cy:.1} {:.1} {y2:.1} {x2:.1} {y2:.1}",
                        x1 - 30.0,
                        gx1 + 30.0,
                        gx2 - 30.0,
                        x2 + 30.0,
                    ),
                    Point {
                        x: (gx1 + gx2) / 2.0,
                        y: cy - 8.0,
                    },
                )
            } else {
                (
                    format!("M {x1:.1} {y1:.1} C {c1:.1} {y1:.1} {c2:.1} {y2:.1} {x2:.1} {y2:.1}"),
                    Point {
                        x: (x1 + x2) / 2.0,
                        y: (y1 + y2) / 2.0 - 8.0,
                    },
                )
            };
            Some(EdgePath {
                from: e.from.clone(),
                to: e.to.clone(),
                kind: e.kind,
                status: e.status,
                label: e.label.clone(),
                path,
                label_pos,
                bundle_count: counts.get(i).copied().unwrap_or(1),
            })
        })
        .collect()
}

fn compute_bounds(rects: &HashMap<NodeId, Rect>) -> Rect {
    const PAD: f64 = 48.0;
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for r in rects.values() {
        min_x = min_x.min(r.x);
        min_y = min_y.min(r.y);
        max_x = max_x.max(r.x + r.w);
        max_y = max_y.max(r.y + r.h);
    }
    if rects.is_empty() {
        return Rect::default();
    }
    Rect {
        x: min_x - PAD,
        y: min_y - PAD,
        w: (max_x - min_x) + PAD * 2.0,
        h: (max_y - min_y) + PAD * 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Edge, ElementStatus, Node, NodeKind, Note, Origin};

    fn node(id: &str) -> Node {
        Node {
            id: NodeId::from(id),
            label: format!("Node {id}"),
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
            origin: Origin::Agent,
        }
    }

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: NodeId::from(from),
            to: NodeId::from(to),
            kind: EdgeKind::DependsOn,
            label: None,
            status: ElementStatus::Proposed,
            origin: Origin::Agent,
        }
    }

    fn doc(nodes: &[&str], edges: &[(&str, &str)]) -> SessionDoc {
        SessionDoc {
            nodes: nodes.iter().map(|n| node(n)).collect(),
            edges: edges.iter().map(|(a, b)| edge(a, b)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn rect_geometry_handles_centers_overlap_and_touching_edges() {
        let rect = Rect {
            x: 0.0,
            y: 10.0,
            w: 10.0,
            h: 6.0,
        };
        assert_eq!(rect.center_y(), 13.0);
        assert!(rect.intersects(&Rect {
            x: 9.0,
            y: 15.0,
            w: 2.0,
            h: 2.0,
        }));
        assert!(!rect.intersects(&Rect {
            x: 10.0,
            y: 11.0,
            w: 2.0,
            h: 2.0,
        }));
        assert!(!rect.intersects(&Rect {
            x: -2.0,
            y: 11.0,
            w: 2.0,
            h: 2.0,
        }));
        assert!(!rect.intersects(&Rect {
            x: 1.0,
            y: 16.0,
            w: 2.0,
            h: 2.0,
        }));
        assert!(!rect.intersects(&Rect {
            x: 1.0,
            y: 8.0,
            w: 2.0,
            h: 2.0,
        }));
    }

    #[test]
    fn empty_doc_is_empty_layout() {
        let l = compute(&SessionDoc::default());
        assert!(l.rects.is_empty());
        assert!(l.edges.is_empty());
    }

    fn grouped_doc() -> SessionDoc {
        let mut d = doc(
            &["outside", "m1", "m2", "m3", "other"],
            &[
                ("outside", "m1"),
                ("outside", "m2"),
                ("outside", "m3"),
                ("m3", "other"),
            ],
        );
        for id in ["m1", "m2", "m3"] {
            d.node_mut(&NodeId::from(id)).unwrap().group = Some("Platform".into());
        }
        d
    }

    #[test]
    fn collapsed_group_becomes_cluster() {
        let d = grouped_doc();
        let collapsed: std::collections::BTreeSet<String> =
            std::iter::once("Platform".to_owned()).collect();
        let l = compute_collapsed(&d, &collapsed);
        assert_eq!(l.clusters.len(), 1, "one cluster rect");
        assert_eq!(l.clusters[0].group, "Platform");
        assert_eq!(l.clusters[0].member_count, 3);
        for id in ["m1", "m2", "m3"] {
            assert!(
                !l.rects.contains_key(&NodeId::from(id)),
                "member {id} hidden"
            );
        }
        assert!(l.rects.contains_key(&NodeId::from("outside")));
        assert!(l.rects.contains_key(&NodeId::from("other")));
        // Cluster rect doesn't overlap visible cards.
        for r in l.rects.values() {
            assert!(!l.clusters[0].rect.intersects(r), "cluster overlaps a card");
        }
    }

    #[test]
    fn cluster_edges_reroute_and_bundle() {
        let d = grouped_doc();
        let collapsed: std::collections::BTreeSet<String> =
            std::iter::once("Platform".to_owned()).collect();
        let l = compute_collapsed(&d, &collapsed);
        // outside→{m1,m2,m3} become ONE bundled edge to the cluster.
        let to_cluster: Vec<_> = l
            .edges
            .iter()
            .filter(|e| e.from.as_str() == "outside")
            .collect();
        assert_eq!(to_cluster.len(), 1, "bundled: {:?}", l.edges);
        assert_eq!(to_cluster[0].bundle_count, 3);
        assert_eq!(to_cluster[0].label.as_deref(), Some("×3"));
        // m3→other leaves the cluster as a single count-1 edge.
        let from_cluster: Vec<_> = l
            .edges
            .iter()
            .filter(|e| e.to.as_str() == "other")
            .collect();
        assert_eq!(from_cluster.len(), 1);
        assert_eq!(from_cluster[0].bundle_count, 1);
        assert_eq!(from_cluster[0].label, None);
        // Determinism.
        let again = compute_collapsed(&d, &collapsed);
        assert_eq!(l, again);
    }

    #[test]
    fn expanded_has_no_clusters_and_unit_bundles() {
        let l = compute_collapsed(&grouped_doc(), &std::collections::BTreeSet::new());
        assert!(l.clusters.is_empty());
        assert!(l.edges.iter().all(|e| e.bundle_count == 1));
        assert_eq!(l, compute(&grouped_doc()), "compute() is the empty set");
    }

    #[test]
    fn duplicate_edges_do_not_change_node_placement() {
        let edges = &[("a", "c"), ("a", "d"), ("b", "c"), ("b", "d")];
        let base = compute(&doc(&["a", "b", "c", "d"], edges));
        let mut duplicated = doc(&["a", "b", "c", "d"], edges);
        duplicated.edges.insert(2, edge("a", "d"));
        assert_eq!(compute(&duplicated).rects, base.rects);
    }

    #[test]
    fn barycenter_sweeps_use_neighbor_averages_in_both_directions() {
        assert_eq!(
            barycenter_order(&[vec![3], vec![2], vec![], vec![]], &[0, 0, 1, 1]),
            vec![vec![0, 1], vec![3, 2]]
        );
        assert_eq!(
            barycenter_order(&[vec![1, 2], vec![], vec![], vec![]], &[0, 1, 1, 0]),
            vec![vec![0, 3], vec![1, 2]]
        );
        assert_eq!(
            barycenter_order(
                &[vec![4, 5], vec![2, 3], vec![3], vec![], vec![], vec![]],
                &[0, 0, 1, 2, 1, 1],
            ),
            vec![vec![1, 0], vec![2, 4, 5], vec![3]]
        );
    }

    #[test]
    fn place_centers_rank_with_exact_gaps() {
        let d = doc(&["a", "b", "c"], &[]);
        let rects = place(&d, &[0, 0, 0], &[vec![0, 1, 2]]);
        assert_eq!(rects[&NodeId::from("a")].y, -105.0);
        assert_eq!(rects[&NodeId::from("b")].y, -27.0);
        assert_eq!(rects[&NodeId::from("c")].y, 51.0);

        let d = doc(&["left", "right"], &[]);
        let rects = place(&d, &[0, 1], &[vec![0], vec![1]]);
        assert_eq!(rects[&NodeId::from("left")].x, 0.0);
        assert_eq!(rects[&NodeId::from("right")].x, 380.0);
    }

    #[test]
    fn long_edges_share_channel_lanes() {
        // s1→t and s2→t span three ranks (via the s1→m1→m2→t chain);
        // they must route through the shared channel (an `L` segment) in
        // distinct lanes, while rank-adjacent edges keep the plain curve.
        let d = doc(
            &["s1", "s2", "m1", "m2", "t"],
            &[
                ("s1", "m1"),
                ("m1", "m2"),
                ("m2", "t"),
                ("s1", "t"),
                ("s2", "t"),
            ],
        );
        let l = compute(&d);
        let path_of = |from: &str, to: &str| {
            l.edges
                .iter()
                .find(|e| e.from.as_str() == from && e.to.as_str() == to)
                .map(|e| e.path.clone())
                .expect("edge present")
        };
        let long_a = path_of("s1", "t");
        let long_b = path_of("s2", "t");
        assert!(long_a.contains(" L "), "channel segment: {long_a}");
        assert!(long_b.contains(" L "), "channel segment: {long_b}");
        assert_ne!(long_a, long_b, "distinct lanes");
        for (from, to) in [("s1", "m1"), ("m1", "m2"), ("m2", "t")] {
            let short = path_of(from, to);
            assert!(
                !short.contains(" L "),
                "adjacent edges keep the plain curve: {short}"
            );
        }
    }

    /// y of the path's `M x y` start point.
    fn first_move_y(path: &str) -> f64 {
        let mut t = path.split_whitespace();
        assert_eq!(t.next(), Some("M"), "path starts with M: {path}");
        t.next().unwrap();
        t.next().unwrap().parse().unwrap()
    }

    /// y of the horizontal corridor (`L x y`) segment.
    fn corridor_y(path: &str) -> f64 {
        let mut t = path.split_whitespace().skip_while(|s| *s != "L");
        assert_eq!(t.next(), Some("L"), "path has a corridor: {path}");
        t.next().unwrap();
        t.next().unwrap().parse().unwrap()
    }

    #[test]
    fn out_ports_fan_toward_their_targets() {
        // Edges are declared t3, t1, t2 — document order must not decide
        // ports, or the curves cross right at the hub's edge.
        let d = doc(
            &["hub", "t1", "t2", "t3"],
            &[("hub", "t3"), ("hub", "t1"), ("hub", "t2")],
        );
        let l = compute(&d);
        let ty = |id: &str| l.rects[&NodeId::from(id)].center_y();
        assert!(
            ty("t1") < ty("t2") && ty("t2") < ty("t3"),
            "stacked targets"
        );
        let start_y = |to: &str| {
            first_move_y(
                &l.edges
                    .iter()
                    .find(|e| e.to.as_str() == to)
                    .expect("edge present")
                    .path,
            )
        };
        assert!(start_y("t1") < start_y("t2"));
        assert!(start_y("t2") < start_y("t3"));
    }

    #[test]
    fn long_corridor_snaps_clear_of_cards() {
        // a→d spans ranks 0→3; the mean-y corridor would cut straight
        // through b and c, so it must snap outside every intermediate card.
        let d = doc(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("b", "c"), ("c", "d"), ("a", "d")],
        );
        let l = compute(&d);
        let long = l
            .edges
            .iter()
            .find(|e| e.from.as_str() == "a" && e.to.as_str() == "d")
            .unwrap();
        let cy = corridor_y(&long.path);
        for id in ["b", "c"] {
            let r = l.rects[&NodeId::from(id)];
            assert!(
                cy <= r.y || cy >= r.y + r.h,
                "corridor {cy} cuts card {id} {r:?}"
            );
        }
    }

    #[test]
    fn backward_edges_route_through_a_clear_corridor() {
        let d = doc(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("c", "a")]);
        let l = compute(&d);
        let back = l
            .edges
            .iter()
            .find(|e| e.from.as_str() == "c" && e.to.as_str() == "a")
            .unwrap();
        let cy = corridor_y(&back.path);
        let rb = l.rects[&NodeId::from("b")];
        assert!(
            cy <= rb.y || cy >= rb.y + rb.h,
            "backward corridor {cy} cuts intermediate card {rb:?}"
        );
    }

    #[test]
    fn question_badge_grows_the_lane_band() {
        use crate::model::{Question, QuestionId};
        let mut d = doc(&["a"], &[]);
        d.nodes[0].lane = Some("data".into());
        d.questions.push(Question {
            id: QuestionId::from("q1"),
            prompt: "which engine?".into(),
            node_id: Some(NodeId::from("a")),
            rationale: None,
            answer: None,
            answered_at: None,
        });
        let with_q = compute(&d);
        d.questions.clear();
        let without_q = compute(&d);
        let r = with_q.rects[&NodeId::from("a")];
        let band = &with_q.lanes[0].rect;
        assert!(
            r.y + r.h <= band.y + band.h,
            "badged card {r:?} stays inside its band {band:?}"
        );
        assert_eq!(
            r.h - without_q.rects[&NodeId::from("a")].h,
            28.0,
            "open question adds the badge row"
        );
    }

    #[test]
    fn edge_ports_spread_symmetrically() {
        let d = doc(&["a", "b", "c"], &[("a", "b"), ("a", "c")]);
        let rects = HashMap::from([
            (
                "a".into(),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 20.0,
                },
            ),
            (
                "b".into(),
                Rect {
                    x: 300.0,
                    y: 0.0,
                    w: 100.0,
                    h: 20.0,
                },
            ),
            (
                "c".into(),
                Rect {
                    x: 300.0,
                    y: 100.0,
                    w: 100.0,
                    h: 20.0,
                },
            ),
        ]);
        let rank_of: HashMap<&NodeId, usize> = d
            .nodes
            .iter()
            .map(|node| (&node.id, if node.id.as_str() == "a" { 0 } else { 1 }))
            .collect();
        let edges = route_edges(&d, &rects, &[], &rank_of);
        assert_eq!(
            edges[0].path,
            "M 100.0 8.0 C 200.0 8.0 200.0 10.0 300.0 10.0"
        );
        assert_eq!(edges[0].label_pos, Point { x: 200.0, y: 1.0 });
        assert_eq!(
            edges[1].path,
            "M 100.0 12.0 C 200.0 12.0 200.0 110.0 300.0 110.0"
        );
        assert_eq!(edges[1].label_pos, Point { x: 200.0, y: 53.0 });
    }

    #[test]
    fn backward_edges_flip_sides_without_changing_centers() {
        let d = doc(&["a", "b"], &[("a", "b")]);
        let rects = HashMap::from([
            (
                "a".into(),
                Rect {
                    x: 300.0,
                    y: 0.0,
                    w: 100.0,
                    h: 100.0,
                },
            ),
            (
                "b".into(),
                Rect {
                    x: 0.0,
                    y: 100.0,
                    w: 100.0,
                    h: 100.0,
                },
            ),
        ]);
        let rank_of: HashMap<&NodeId, usize> = d
            .nodes
            .iter()
            .map(|node| (&node.id, usize::from(node.id.as_str() == "b") * 2))
            .collect();
        let edge = route_edges(&d, &rects, &[], &rank_of).remove(0);
        assert_eq!(
            edge.path,
            "M 300.0 50.0 C 200.0 50.0 200.0 150.0 100.0 150.0"
        );
        assert_eq!(edge.label_pos, Point { x: 200.0, y: 92.0 });
    }

    #[test]
    fn incoming_ports_and_forward_direction_use_exact_centers() {
        let d = doc(&["a", "c", "b"], &[("a", "b"), ("c", "b")]);
        let rects = HashMap::from([
            (
                "a".into(),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 20.0,
                },
            ),
            (
                "c".into(),
                Rect {
                    x: 0.0,
                    y: 100.0,
                    w: 100.0,
                    h: 20.0,
                },
            ),
            (
                "b".into(),
                Rect {
                    x: 20.0,
                    y: 50.0,
                    w: 200.0,
                    h: 20.0,
                },
            ),
        ]);
        let rank_of: HashMap<&NodeId, usize> = d
            .nodes
            .iter()
            .map(|node| (&node.id, usize::from(node.id.as_str() == "b")))
            .collect();
        let edges = route_edges(&d, &rects, &[], &rank_of);
        assert_eq!(
            edges[0].path,
            "M 100.0 10.0 C 160.0 10.0 -40.0 58.0 20.0 58.0"
        );
        assert_eq!(
            edges[1].path,
            "M 100.0 110.0 C 160.0 110.0 -40.0 62.0 20.0 62.0"
        );

        let equal_centers = HashMap::from([
            (
                "a".into(),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 20.0,
                },
            ),
            (
                "b".into(),
                Rect {
                    x: 0.0,
                    y: 50.0,
                    w: 100.0,
                    h: 20.0,
                },
            ),
        ]);
        let one = doc(&["a", "b"], &[("a", "b")]);
        let ranks: HashMap<&NodeId, usize> = one.nodes.iter().map(|node| (&node.id, 0)).collect();
        assert!(
            route_edges(&one, &equal_centers, &[], &ranks)[0]
                .path
                .starts_with("M 100.0")
        );
        let mut left_center = equal_centers;
        left_center.get_mut(&NodeId::from("b")).unwrap().x = -25.0;
        assert!(
            route_edges(&one, &left_center, &[], &ranks)[0]
                .path
                .starts_with("M 0.0")
        );
        let mut wide_left = left_center;
        *wide_left.get_mut(&NodeId::from("b")).unwrap() = Rect {
            x: -80.0,
            y: 50.0,
            w: 200.0,
            h: 20.0,
        };
        assert!(
            route_edges(&one, &wide_left, &[], &ranks)[0]
                .path
                .starts_with("M 0.0")
        );
    }

    #[test]
    fn long_edge_channels_center_on_the_group_mean() {
        let d = doc(&["a", "b", "c", "d"], &[("a", "b"), ("c", "d")]);
        let rects = HashMap::from([
            (
                "a".into(),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 100.0,
                },
            ),
            (
                "b".into(),
                Rect {
                    x: 600.0,
                    y: 100.0,
                    w: 100.0,
                    h: 100.0,
                },
            ),
            (
                "c".into(),
                Rect {
                    x: 0.0,
                    y: 200.0,
                    w: 100.0,
                    h: 100.0,
                },
            ),
            (
                "d".into(),
                Rect {
                    x: 600.0,
                    y: 300.0,
                    w: 100.0,
                    h: 100.0,
                },
            ),
        ]);
        let rank_of: HashMap<&NodeId, usize> = d
            .nodes
            .iter()
            .map(|node| {
                (
                    &node.id,
                    if matches!(node.id.as_str(), "a" | "c") {
                        0
                    } else {
                        2
                    },
                )
            })
            .collect();
        let edges = route_edges(&d, &rects, &[], &rank_of);
        assert_eq!(
            edges[0].path,
            "M 100.0 50.0 C 130.0 50.0 130.0 194.0 160.0 194.0 L 540.0 194.0 C 570.0 194.0 570.0 150.0 600.0 150.0"
        );
        assert_eq!(edges[0].label_pos, Point { x: 350.0, y: 186.0 });
        assert_eq!(
            edges[1].path,
            "M 100.0 250.0 C 130.0 250.0 130.0 206.0 160.0 206.0 L 540.0 206.0 C 570.0 206.0 570.0 350.0 600.0 350.0"
        );
        assert_eq!(edges[1].label_pos, Point { x: 350.0, y: 198.0 });
    }

    #[test]
    fn bundled_routing_is_deterministic() {
        let d = doc(
            &["s1", "s2", "m1", "m2", "t"],
            &[
                ("s1", "m1"),
                ("m1", "m2"),
                ("m2", "t"),
                ("s1", "t"),
                ("s2", "t"),
            ],
        );
        assert_eq!(compute(&d), compute(&d));
    }

    #[test]
    fn visible_set_culls() {
        let d = crate::demo::big_doc(200);
        let l = compute(&d);
        // A view genuinely covering the whole bounds sees everything.
        // (Built by hand: `ViewTransform::fit` deliberately clamps at a
        // readability floor and would show only a subset of 200 nodes.)
        let scale = (1280.0 / l.bounds.w).min(780.0 / l.bounds.h).min(1.0);
        let tx = -l.bounds.x * scale + (1280.0 - l.bounds.w * scale) / 2.0;
        let ty = -l.bounds.y * scale + (780.0 - l.bounds.h * scale) / 2.0;
        let all = visible_set(&l, tx, ty, scale, 1280.0, 780.0);
        assert_eq!(all.nodes.len(), 200);
        assert_eq!(all.edges.len(), l.edges.len());
        // A tight corner view sees strictly fewer.
        let corner = visible_set(&l, 0.0, 0.0, 1.0, 1280.0, 780.0);
        assert!(corner.nodes.len() < 200, "culled: {}", corner.nodes.len());
        // Every visible edge touches at least one visible-ish rect.
        for &i in &corner.edges {
            let e = &l.edges[i];
            assert!(
                corner.nodes.contains(&e.from)
                    || corner.nodes.contains(&e.to)
                    || e.from.as_str().starts_with("group:")
                    || e.to.as_str().starts_with("group:"),
                "edge {i} has no visible endpoint"
            );
        }
    }

    #[test]
    fn visible_set_applies_pan_zoom_and_one_viewport_margin() {
        let mut layout = Layout::default();
        for (id, x, y) in [
            ("center", 0.0, 0.0),
            ("left-inner", -59.0, 0.0),
            ("right-inner", 89.0, 0.0),
            ("top-inner", 0.0, -4.0),
            ("bottom-inner", 0.0, 69.0),
            ("left-outer", -61.0, 0.0),
            ("right-outer", 90.0, 0.0),
            ("top-outer", 0.0, -6.0),
            ("bottom-outer", 0.0, 70.0),
        ] {
            layout.rects.insert(
                NodeId::from(id),
                Rect {
                    x,
                    y,
                    w: 1.0,
                    h: 1.0,
                },
            );
        }
        layout.edges = vec![
            EdgePath {
                from: "center".into(),
                to: "right-outer".into(),
                kind: EdgeKind::DependsOn,
                status: ElementStatus::Proposed,
                label: None,
                path: String::new(),
                label_pos: Point { x: 0.0, y: 0.0 },
                bundle_count: 1,
            },
            EdgePath {
                from: "right-outer".into(),
                to: "center".into(),
                kind: EdgeKind::DependsOn,
                status: ElementStatus::Proposed,
                label: None,
                path: String::new(),
                label_pos: Point { x: 0.0, y: 0.0 },
                bundle_count: 1,
            },
        ];

        let visible = visible_set(&layout, 20.0, -40.0, 2.0, 100.0, 50.0);
        assert_eq!(
            visible.nodes,
            [
                "bottom-inner",
                "center",
                "left-inner",
                "right-inner",
                "top-inner",
            ]
            .into_iter()
            .map(NodeId::from)
            .collect()
        );
        assert_eq!(visible.edges, vec![0, 1]);
    }

    #[test]
    fn swimlanes_confine_cards_to_stacked_bands() {
        let mut d = doc(&["a", "b", "c", "d"], &[("a", "b"), ("c", "d")]);
        for id in ["a", "b"] {
            d.node_mut(&NodeId::from(id)).unwrap().lane = Some("frontend".into());
        }
        for id in ["c", "d"] {
            d.node_mut(&NodeId::from(id)).unwrap().lane = Some("backend".into());
        }
        let layout = compute(&d);
        assert_eq!(layout.lanes.len(), 2);
        assert_eq!(layout.lanes[0].label, "frontend");
        assert_eq!(layout.lanes[1].label, "backend");
        // Every card sits inside its lane band.
        for (label, ids) in [("frontend", ["a", "b"]), ("backend", ["c", "d"])] {
            let band = layout.lanes.iter().find(|l| l.label == label).unwrap().rect;
            for id in ids {
                let r = layout.rects[&NodeId::from(id)];
                assert!(
                    r.y >= band.y + 36.0,
                    "{id} overlaps the {label} title strip"
                );
                assert!(
                    r.y >= band.y && r.y + r.h <= band.y + band.h,
                    "{id} escapes {label}"
                );
            }
        }
        // Bands stack: frontend sits entirely above backend.
        assert!(layout.lanes[0].rect.y + layout.lanes[0].rect.h <= layout.lanes[1].rect.y);
        // Ranking still applies inside a lane (a → b is left → right).
        assert!(layout.rects[&NodeId::from("a")].x < layout.rects[&NodeId::from("b")].x);
        // No lanes → no bands (default packing unchanged).
        assert!(compute(&doc(&["x", "y"], &[("x", "y")])).lanes.is_empty());
    }

    #[test]
    fn layout_is_deterministic() {
        let d = doc(
            &["ui", "api", "auth", "db", "queue", "worker"],
            &[
                ("ui", "api"),
                ("api", "auth"),
                ("api", "db"),
                ("api", "queue"),
                ("queue", "worker"),
                ("worker", "db"),
            ],
        );
        let a = compute(&d);
        let b = compute(&d);
        assert_eq!(a, b);
    }

    #[test]
    fn ranks_are_monotonic_along_edges() {
        let d = doc(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("b", "c"), ("a", "c"), ("c", "d")],
        );
        let l = compute(&d);
        for e in &d.edges {
            let fx = l.rects[&e.from].x;
            let tx = l.rects[&e.to].x;
            assert!(
                fx < tx,
                "{} ({fx}) should be left of {} ({tx})",
                e.from,
                e.to
            );
        }
    }

    #[test]
    fn auto_nodes_do_not_overlap() {
        let d = doc(
            &["r", "a", "b", "c", "d", "e"],
            &[("r", "a"), ("r", "b"), ("r", "c"), ("r", "d"), ("r", "e")],
        );
        let l = compute(&d);
        let rects: Vec<&Rect> = l.rects.values().collect();
        for (i, r1) in rects.iter().enumerate() {
            for r2 in &rects[i + 1..] {
                assert!(!r1.intersects(r2), "{r1:?} overlaps {r2:?}");
            }
        }
    }

    #[test]
    fn cycles_terminate_and_layout_all_nodes() {
        let d = doc(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("c", "a")]);
        let l = compute(&d);
        assert_eq!(l.rects.len(), 3);
        assert_eq!(l.edges.len(), 3, "back-edge still renders");
    }

    #[test]
    fn pinned_node_keeps_exact_position() {
        let mut d = doc(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
        d.nodes[1].position = Some(Point {
            x: 777.0,
            y: -333.0,
        });
        let l = compute(&d);
        let r = &l.rects[&NodeId::from("b")];
        assert_eq!((r.x, r.y), (777.0, -333.0));
    }

    #[test]
    fn adding_a_node_does_not_move_pins() {
        let mut d = doc(&["a", "b"], &[("a", "b")]);
        d.nodes[0].position = Some(Point { x: 100.0, y: 100.0 });
        let before = compute(&d);
        d.nodes.push(node("c"));
        d.edges.push(edge("a", "c"));
        let after = compute(&d);
        assert_eq!(
            before.rects[&NodeId::from("a")],
            after.rects[&NodeId::from("a")]
        );
    }

    #[test]
    fn auto_nodes_are_nudged_off_pinned_cards() {
        let mut d = doc(&["a", "b", "c"], &[]);
        // Pin `a` exactly where rank-0 packing would put `b`'s column start.
        let h_b = estimate_height(&d.nodes[1], 0);
        d.nodes[0].position = Some(Point {
            x: 0.0,
            y: -h_b / 2.0,
        });
        let l = compute(&d);
        let ra = &l.rects[&NodeId::from("a")];
        let rb = &l.rects[&NodeId::from("b")];
        assert!(!ra.intersects(rb), "pinned {ra:?} vs auto {rb:?}");
        assert_eq!(rb.y, 51.0);
        assert_eq!(l.rects[&NodeId::from("c")].y, 129.0);
    }

    #[test]
    fn union_rect_spans_both_inputs() {
        assert_eq!(
            union_rect(
                Rect {
                    x: 2.0,
                    y: 3.0,
                    w: 5.0,
                    h: 7.0,
                },
                Rect {
                    x: -4.0,
                    y: 8.0,
                    w: 3.0,
                    h: 6.0,
                },
            ),
            Rect {
                x: -4.0,
                y: 3.0,
                w: 11.0,
                h: 11.0,
            }
        );
        assert_eq!(
            union_rect(
                Rect {
                    x: 2.0,
                    y: 3.0,
                    w: 5.0,
                    h: 20.0,
                },
                Rect {
                    x: 10.0,
                    y: 8.0,
                    w: 3.0,
                    h: 6.0,
                },
            ),
            Rect {
                x: 2.0,
                y: 3.0,
                w: 11.0,
                h: 20.0,
            }
        );
    }

    #[test]
    fn bounds_span_all_rects_with_exact_padding() {
        let rects = HashMap::from([
            (
                "a".into(),
                Rect {
                    x: 2.0,
                    y: 3.0,
                    w: 5.0,
                    h: 7.0,
                },
            ),
            (
                "b".into(),
                Rect {
                    x: -4.0,
                    y: 8.0,
                    w: 3.0,
                    h: 6.0,
                },
            ),
        ]);
        assert_eq!(
            compute_bounds(&rects),
            Rect {
                x: -52.0,
                y: -45.0,
                w: 107.0,
                h: 107.0
            }
        );
    }

    #[test]
    fn laned_place_uses_one_gap_between_cards() {
        let mut d = doc(&["a", "b"], &[]);
        for node in &mut d.nodes {
            node.lane = Some("lane".into());
        }
        let (rects, lanes) = place_laned(&d, &[0, 0], &[vec![0, 1]], &[]);
        assert_eq!(rects[&NodeId::from("a")].y, 36.0);
        assert_eq!(rects[&NodeId::from("b")].y, 114.0);
        assert_eq!(lanes[0].rect.h, 188.0);
    }

    #[test]
    fn laned_place_stacks_bands_and_spaces_ranks_exactly() {
        let mut d = doc(&["a", "b", "c"], &[]);
        d.nodes[0].lane = Some("one".into());
        d.nodes[1].lane = Some("one".into());
        d.nodes[2].lane = Some("two".into());

        let (rects, lanes) = place_laned(&d, &[0, 1, 0], &[vec![0, 2], vec![1]], &[]);
        assert_eq!(
            (rects[&NodeId::from("a")].x, rects[&NodeId::from("a")].y),
            (0.0, 36.0)
        );
        assert_eq!(
            (rects[&NodeId::from("b")].x, rects[&NodeId::from("b")].y),
            (380.0, 36.0)
        );
        assert_eq!(
            (rects[&NodeId::from("c")].x, rects[&NodeId::from("c")].y),
            (0.0, 164.0)
        );
        assert_eq!(
            lanes[0].rect,
            Rect {
                x: -30.0,
                y: 0.0,
                w: 700.0,
                h: 110.0
            }
        );
        assert_eq!(
            lanes[1].rect,
            Rect {
                x: -30.0,
                y: 128.0,
                w: 700.0,
                h: 110.0
            }
        );
    }

    #[test]
    fn height_grows_with_content() {
        let plain = node("x");
        let mut rich = node("x");
        rich.description =
            "A long description that certainly wraps across multiple lines of card text".into();
        assert!(estimate_height(&rich, 0) > estimate_height(&plain, 0));
    }

    #[test]
    fn open_question_badge_adds_height() {
        let n = node("x");
        assert_eq!(
            estimate_height(&n, 1) - estimate_height(&n, 0),
            28.0,
            "unanswered question badge row"
        );
    }

    #[test]
    fn height_accounts_for_wrapping_and_each_badge_source() {
        let plain = node("plain");
        assert_eq!(estimate_height(&plain), 54.0);

        let mut wrapped_label = plain.clone();
        wrapped_label.label = "x".repeat(23);
        assert_eq!(estimate_height(&wrapped_label), 72.0);

        let mut wrapped_description = plain.clone();
        wrapped_description.description = "x".repeat(37);
        assert_eq!(estimate_height(&wrapped_description), 90.0);

        let mut open_choice = plain.clone();
        open_choice.choices = crate::demo::demo_doc().nodes[4].choices.clone();
        assert_eq!(estimate_height(&open_choice), 82.0);

        let mut closed_choice = open_choice.clone();
        closed_choice.choices[0].status = crate::model::ChoiceStatus::Dismissed;
        assert_eq!(estimate_height(&closed_choice), 82.0);

        let mut noted = plain;
        noted.notes.push(Note {
            id: "note".into(),
            text: "note".into(),
            created_at: chrono::Utc::now(),
        });
        assert_eq!(estimate_height(&noted), 82.0);
    }

    #[test]
    fn height_caps_long_labels_at_three_lines() {
        let mut three_lines = node("three-lines");
        three_lines.label = "x".repeat(66);
        let mut ten_lines = node("ten-lines");
        ten_lines.label = "x".repeat(220);

        assert_eq!(
            estimate_height(&three_lines, 0),
            estimate_height(&ten_lines, 0)
        );
    }

    #[test]
    fn wrap_lines_counts_greedily() {
        assert_eq!(wrap_lines("short", 211.0, 1.0), 1);
        // Two ~37px words never fit a 40px line together.
        assert_eq!(wrap_lines("goose goose", 40.0, 1.0), 2);
        // A single over-long word wraps mid-word (overflow-wrap: anywhere).
        assert_eq!(wrap_lines("supercalifragilistic", 80.0, 1.0), 2);
    }

    #[test]
    fn wide_glyphs_wrap_earlier_than_narrow_ones() {
        // 22 W's overflow the 211px label budget (real wrap at ~16 wide
        // glyphs); 22 i's fit on one line. A flat per-character count would
        // treat these identically and clip the wide label's second line.
        let mut wide = node("wide");
        wide.label = "W".repeat(22);
        let mut narrow = node("narrow");
        narrow.label = "i".repeat(22);
        assert!(estimate_height(&wide, 0) > estimate_height(&narrow, 0));
        assert_eq!(wrap_lines(&"W".repeat(22), 211.0, 1.0), 2);
        assert_eq!(wrap_lines(&"i".repeat(22), 211.0, 1.0), 1);
    }

    #[test]
    fn declared_empty_lane_gets_a_band_in_declared_order() {
        // No node references "review"; it exists only in the declared list.
        let mut d = doc(&["a", "b"], &[("a", "b")]);
        d.node_mut(&NodeId::from("a")).unwrap().lane = Some("build".into());
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["review".to_owned(), "build".to_owned()],
        );
        let labels: Vec<&str> = layout.lanes.iter().map(|l| l.label.as_str()).collect();
        // Declared order wins; the empty "review" lane still renders a band.
        assert_eq!(labels, vec!["review", "build"]);
        let review = layout.lanes.iter().find(|l| l.label == "review").unwrap();
        assert!(
            review.rect.h > 0.0,
            "empty declared lane has a visible band"
        );
    }

    #[test]
    fn referenced_only_lane_follows_declared_lanes() {
        let mut d = doc(&["a", "b"], &[("a", "b")]);
        d.node_mut(&NodeId::from("a")).unwrap().lane = Some("adhoc".into()); // not declared
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["build".to_owned()], // declared but unreferenced
        );
        let labels: Vec<&str> = layout.lanes.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["build", "adhoc"],
            "declared before referenced-only"
        );
    }

    #[test]
    fn pinned_card_is_enclosed_by_its_grown_band() {
        let mut d = doc(&["a", "b"], &[("a", "b")]);
        d.node_mut(&NodeId::from("a")).unwrap().lane = Some("build".into());
        d.node_mut(&NodeId::from("b")).unwrap().lane = Some("build".into());
        // Pin b far below where the auto strip would sit.
        d.node_mut(&NodeId::from("b")).unwrap().position = Some(Point { x: 40.0, y: 400.0 });
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["build".to_owned()],
        );
        let band = layout.lanes.iter().find(|l| l.label == "build").unwrap();
        let rb = layout.rects[&NodeId::from("b")];
        assert!(
            band.rect.y <= rb.y && band.rect.y + band.rect.h >= rb.y + rb.h,
            "grown band {:?} must enclose pinned card {:?}",
            band.rect,
            rb
        );
        // Growth is downward only: the band's top edge stays put.
        assert!(
            band.rect.y.abs() < 1e-6,
            "band top does not chase pinned cards"
        );
    }

    #[test]
    fn pinned_card_only_grows_its_own_lane() {
        let mut d = doc(&["a", "b"], &[]);
        d.node_mut(&NodeId::from("a")).unwrap().lane = Some("build".into());
        d.node_mut(&NodeId::from("b")).unwrap().lane = Some("review".into());
        d.node_mut(&NodeId::from("b")).unwrap().position = Some(Point {
            x: 4_000.0,
            y: 4_000.0,
        });
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["build".to_owned(), "review".to_owned()],
        );
        let build = layout
            .lanes
            .iter()
            .find(|lane| lane.label == "build")
            .unwrap();
        assert!(build.rect.x + build.rect.w < 1_000.0);
        assert!(build.rect.y + build.rect.h < 1_000.0);
    }

    #[test]
    fn lane_at_hits_bands_and_misses_the_gap() {
        let mut d = doc(&["a", "b"], &[("a", "b")]);
        d.node_mut(&NodeId::from("a")).unwrap().lane = Some("build".into());
        d.node_mut(&NodeId::from("b")).unwrap().lane = Some("review".into());
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["build".to_owned(), "review".to_owned()],
        );
        let build = layout.lanes.iter().find(|l| l.label == "build").unwrap();
        let review = layout.lanes.iter().find(|l| l.label == "review").unwrap();
        let cx = build.rect.x + build.rect.w / 2.0;
        let cy = build.rect.y + build.rect.h / 2.0;
        assert_eq!(lane_at(&layout.lanes, cx, cy).as_deref(), Some("build"));
        assert_eq!(
            lane_at(&layout.lanes, build.rect.x, build.rect.y).as_deref(),
            Some("build")
        );
        assert_eq!(lane_at(&layout.lanes, build.rect.x + build.rect.w, cy), None);
        assert_eq!(lane_at(&layout.lanes, cx, build.rect.y + build.rect.h), None);
        // The separation gap between two bands belongs to no lane.
        let gap_y = (build.rect.y + build.rect.h + review.rect.y) / 2.0;
        assert_eq!(lane_at(&layout.lanes, cx, gap_y), None, "gap = no lane");
        assert_eq!(
            lane_at(&layout.lanes, cx, build.rect.y - 5000.0),
            None,
            "far away = no lane"
        );
    }

    #[test]
    fn whole_grown_band_is_a_drop_target() {
        // All members pinned: the auto-packed strip is empty, but the drawn
        // band still encloses the cards — dropping anywhere inside it must
        // keep membership (regression: only the thin title strip counted).
        let mut d = doc(&["a", "b"], &[("a", "b")]);
        for id in ["a", "b"] {
            let n = d.node_mut(&NodeId::from(id)).unwrap();
            n.lane = Some("build".into());
        }
        d.node_mut(&NodeId::from("a")).unwrap().position = Some(Point { x: 0.0, y: 250.0 });
        d.node_mut(&NodeId::from("b")).unwrap().position = Some(Point { x: 40.0, y: 400.0 });
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["build".to_owned()],
        );
        let band = layout.lanes.iter().find(|l| l.label == "build").unwrap();
        let rb = layout.rects[&NodeId::from("b")];
        assert!(
            band.rect.y + band.rect.h >= rb.y + rb.h,
            "band {:?} encloses pinned card {rb:?}",
            band.rect
        );
        assert_eq!(
            lane_at(&layout.lanes, rb.x + rb.w / 2.0, rb.y + rb.h / 2.0).as_deref(),
            Some("build"),
            "a drop deep inside the grown band stays in the lane"
        );
    }

    #[test]
    fn bands_never_overlap_and_keep_min_gap() {
        // Lane "a" grows far down to enclose a pinned member; lane "b" (and
        // its auto-placed card) must be pushed below it, never overlapped.
        let mut d = doc(&["m", "n"], &[]);
        d.node_mut(&NodeId::from("m")).unwrap().lane = Some("a".into());
        d.node_mut(&NodeId::from("m")).unwrap().position = Some(Point { x: 0.0, y: 400.0 });
        d.node_mut(&NodeId::from("n")).unwrap().lane = Some("b".into());
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["a".to_owned(), "b".to_owned()],
        );
        let a = layout.lanes.iter().find(|l| l.label == "a").unwrap();
        let b = layout.lanes.iter().find(|l| l.label == "b").unwrap();
        assert!(
            b.rect.y >= a.rect.y + a.rect.h + LANE_SEP - 1e-6,
            "band b {:?} sits at least LANE_SEP below grown band a {:?}",
            b.rect,
            a.rect
        );
        let rn = layout.rects[&NodeId::from("n")];
        assert!(
            rn.y >= b.rect.y,
            "b's auto-placed card {rn:?} moved down with its band {:?}",
            b.rect
        );
    }

    #[test]
    fn pinned_member_stranded_above_its_band_is_pulled_into_it() {
        // "m" belongs to the second lane but is pinned at y = 0, above where
        // that band stacks (below "a"). Downward-only growth alone would
        // leave it floating inside band "a"; the layout must pull it down
        // into its own band instead.
        let mut d = doc(&["m", "n"], &[]);
        d.node_mut(&NodeId::from("n")).unwrap().lane = Some("a".into());
        d.node_mut(&NodeId::from("m")).unwrap().lane = Some("b".into());
        d.node_mut(&NodeId::from("m")).unwrap().position = Some(Point { x: 0.0, y: 0.0 });
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["a".to_owned(), "b".to_owned()],
        );
        let b = layout.lanes.iter().find(|l| l.label == "b").unwrap();
        let rm = layout.rects[&NodeId::from("m")];
        assert!(
            rm.y >= b.rect.y && rm.y + rm.h <= b.rect.y + b.rect.h,
            "member {rm:?} sits inside its band {:?}",
            b.rect
        );
        assert_eq!(
            lane_at(&layout.lanes, rm.x + rm.w / 2.0, rm.y + rm.h / 2.0).as_deref(),
            Some("b"),
            "the member's center resolves to its own lane"
        );
    }

    #[test]
    fn empty_declared_lane_has_a_comfortable_drop_zone() {
        let d = doc(&["a"], &[]);
        let layout = compute_view(
            &d,
            &std::collections::BTreeSet::new(),
            &["review".to_owned()],
        );
        let review = layout.lanes.iter().find(|l| l.label == "review").unwrap();
        assert!(
            review.rect.h >= LANE_MIN_H,
            "empty band {:?} is at least LANE_MIN_H tall",
            review.rect
        );
    }
}
