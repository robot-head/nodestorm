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

use std::collections::{HashMap, HashSet};

use crate::model::{EdgeKind, ElementStatus, Node, NodeId, Point, SessionDoc};

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
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub rects: HashMap<NodeId, Rect>,
    pub edges: Vec<EdgePath>,
    /// Union of all card rects, padded — used for zoom-to-fit.
    pub bounds: Rect,
}

/// Estimated card height in px. Must stay consistent with `assets/main.css`
/// (card width 260, 18px line height, description clamped to 4 lines).
pub fn estimate_height(node: &Node) -> f64 {
    const BASE: f64 = 54.0; // padding + kind row + first label line
    const LINE_H: f64 = 18.0;
    let label_extra = wrap_lines(&node.label, 22).saturating_sub(1);
    let desc_lines = if node.description.is_empty() {
        0
    } else {
        wrap_lines(&node.description, 36).min(4)
    };
    let badge_row = if node.open_choice_count() > 0
        || node.choices.iter().any(|c| !c.is_open())
        || !node.notes.is_empty()
    {
        28.0
    } else {
        0.0
    };
    BASE + (label_extra + desc_lines) as f64 * LINE_H + badge_row
}

/// Greedy word-wrap line count at `per_line` characters.
fn wrap_lines(text: &str, per_line: usize) -> usize {
    let mut lines = 1usize;
    let mut current = 0usize;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        if current == 0 {
            current = len;
        } else if current + 1 + len <= per_line {
            current += 1 + len;
        } else {
            lines += 1;
            current = len;
        }
        // Very long single words wrap mid-word.
        while current > per_line {
            lines += 1;
            current -= per_line;
        }
    }
    lines
}

pub fn compute(doc: &SessionDoc) -> Layout {
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
    let rects = place(doc, &ranks, &order);
    let edges = route_edges(doc, &rects);
    let bounds = compute_bounds(&rects);

    Layout {
        rects,
        edges,
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
fn place(doc: &SessionDoc, ranks: &[usize], order: &[Vec<usize>]) -> HashMap<NodeId, Rect> {
    let mut rects: HashMap<NodeId, Rect> = HashMap::new();
    let heights: Vec<f64> = doc.nodes.iter().map(estimate_height).collect();

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

/// Cubic bezier from the right edge of `from` to the left edge of `to`, with
/// per-port vertical fan-out when several edges share a card side.
fn route_edges(doc: &SessionDoc, rects: &HashMap<NodeId, Rect>) -> Vec<EdgePath> {
    // Count edges per (node, side) so anchors can spread.
    let mut out_total: HashMap<&NodeId, usize> = HashMap::new();
    let mut in_total: HashMap<&NodeId, usize> = HashMap::new();
    for e in &doc.edges {
        *out_total.entry(&e.from).or_default() += 1;
        *in_total.entry(&e.to).or_default() += 1;
    }
    let mut out_seen: HashMap<&NodeId, usize> = HashMap::new();
    let mut in_seen: HashMap<&NodeId, usize> = HashMap::new();

    let spread = |slot: usize, total: usize| -> f64 {
        (slot as f64 - (total as f64 - 1.0) / 2.0) * PORT_SPREAD
    };

    doc.edges
        .iter()
        .filter_map(|e| {
            let a = rects.get(&e.from)?;
            let b = rects.get(&e.to)?;
            let so = {
                let slot = out_seen.entry(&e.from).or_default();
                let s = spread(*slot, out_total[&e.from]);
                *slot += 1;
                s
            };
            let si = {
                let slot = in_seen.entry(&e.to).or_default();
                let s = spread(*slot, in_total[&e.to]);
                *slot += 1;
                s
            };
            // Forward edges leave the right side and enter the left side;
            // backward (cycle) edges flip sides so the curve stays between
            // the two cards instead of ballooning past the graph.
            let backward = b.x + b.w / 2.0 < a.x + a.w / 2.0;
            let y1 = (a.center_y() + so).clamp(a.y + 8.0, a.y + a.h - 8.0);
            let y2 = (b.center_y() + si).clamp(b.y + 8.0, b.y + b.h - 8.0);
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
            let path =
                format!("M {x1:.1} {y1:.1} C {c1:.1} {y1:.1} {c2:.1} {y2:.1} {x2:.1} {y2:.1}");
            Some(EdgePath {
                from: e.from.clone(),
                to: e.to.clone(),
                kind: e.kind,
                status: e.status,
                label: e.label.clone(),
                path,
                label_pos: Point {
                    x: (x1 + x2) / 2.0,
                    y: (y1 + y2) / 2.0 - 8.0,
                },
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
    use crate::model::{Edge, ElementStatus, Node, NodeKind, Origin};

    fn node(id: &str) -> Node {
        Node {
            id: NodeId::from(id),
            label: format!("Node {id}"),
            kind: NodeKind::Component,
            description: String::new(),
            status: ElementStatus::Proposed,
            group: None,
            choices: vec![],
            notes: vec![],
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
    fn empty_doc_is_empty_layout() {
        let l = compute(&SessionDoc::default());
        assert!(l.rects.is_empty());
        assert!(l.edges.is_empty());
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
        let mut d = doc(&["a", "b"], &[]);
        // Pin `a` exactly where rank-0 packing would put `b`'s column start.
        let h_b = estimate_height(&d.nodes[1]);
        d.nodes[0].position = Some(Point {
            x: 0.0,
            y: -h_b / 2.0,
        });
        let l = compute(&d);
        let ra = &l.rects[&NodeId::from("a")];
        let rb = &l.rects[&NodeId::from("b")];
        assert!(!ra.intersects(rb), "pinned {ra:?} vs auto {rb:?}");
    }

    #[test]
    fn height_grows_with_content() {
        let plain = node("x");
        let mut rich = node("x");
        rich.description =
            "A long description that certainly wraps across multiple lines of card text".into();
        assert!(estimate_height(&rich) > estimate_height(&plain));
    }

    #[test]
    fn wrap_lines_counts_greedily() {
        assert_eq!(wrap_lines("short", 20), 1);
        assert_eq!(wrap_lines("two words", 5), 2);
        assert_eq!(wrap_lines("supercalifragilistic", 10), 2);
    }
}
