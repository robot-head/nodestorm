//! The document model shared between agents (over MCP), the store, and the UI.
//!
//! Everything here is plain data with serde + schemars derives. Agent-supplied
//! structs use `deny_unknown_fields` so a typo in a tool call becomes a loud,
//! actionable error instead of silently dropped input.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
            Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    };
}

id_newtype!(
    /// Stable slug identifying a node, e.g. `"auth-service"`.
    NodeId
);
id_newtype!(
    /// Identifies a choice within its node, e.g. `"persistence"`.
    ChoiceId
);
id_newtype!(
    /// Identifies an option within its choice, e.g. `"sqlite"`.
    OptionId
);
id_newtype!(
    /// Identifies a user note (generated, opaque).
    NoteId
);
id_newtype!(
    /// Identifies a free-form agent question, e.g. `"deploy-target"`.
    QuestionId
);
id_newtype!(
    /// Identifies a freehand annotation (generated, opaque).
    AnnotationId
);

/// 2D canvas position in layout pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// What kind of architecture element a node represents. Drives the card glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Service,
    Module,
    #[default]
    Component,
    DataStore,
    Queue,
    Ui,
    External,
    /// Unrecognized kinds degrade to this instead of erroring.
    #[serde(other)]
    Other,
}

/// Lifecycle status of a node or edge within the current brainstorm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ElementStatus {
    /// Already exists in the codebase.
    Existing,
    /// Newly proposed by the agent.
    #[default]
    Proposed,
    /// Exists but will change under the current proposal.
    Modified,
    /// Not directly changed, but impacted by a pending or made decision.
    Affected,
    /// Proposed for removal.
    Removed,
}

/// Implementation-tracking lifecycle the agent advances as it builds. Distinct
/// from [`ElementStatus`] (which is about the *proposal*): this says how far
/// the agent has gotten actually shipping the component. `None` on a node means
/// not tracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    /// Decided and queued for implementation, not started.
    Planned,
    /// Implementation in progress.
    Building,
    /// Implemented; not yet checked.
    Built,
    /// Implemented and verified (tests pass / reviewed).
    Verified,
}

impl BuildStatus {
    /// Progress order, used for the topbar progress summary.
    pub const ORDER: [BuildStatus; 4] = [
        BuildStatus::Planned,
        BuildStatus::Building,
        BuildStatus::Built,
        BuildStatus::Verified,
    ];

    /// Whether the component has shipped (built or verified) — counts toward
    /// the "done" side of the progress summary.
    pub fn is_shipped(self) -> bool {
        matches!(self, BuildStatus::Built | BuildStatus::Verified)
    }

    /// Lowercase wire/display name.
    pub fn name(self) -> &'static str {
        match self {
            BuildStatus::Planned => "planned",
            BuildStatus::Building => "building",
            BuildStatus::Built => "built",
            BuildStatus::Verified => "verified",
        }
    }
}

/// Who authored a graph element. Agent-authored content follows the agent
/// merge rules; user-authored elements survive proposes that omit them.
/// The server forces everything arriving over MCP to `Agent`, so agents
/// cannot claim (and need not know about) user authorship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    Agent,
    User,
}

impl Origin {
    /// serde `skip_serializing_if` — agent origin is the wire default.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn is_agent(&self) -> bool {
        *self == Origin::Agent
    }
}

/// Relationship carried by an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    #[default]
    DependsOn,
    DataFlow,
    Contains,
    #[serde(other)]
    Other,
}

/// Whether a choice still needs the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceStatus {
    #[default]
    Open,
    Decided,
    Dismissed,
}

/// One selectable option within a [`Choice`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub id: OptionId,
    pub label: String,
    /// One or two sentences describing the option.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub pros: Vec<String>,
    #[serde(default)]
    pub cons: Vec<String>,
    /// Exactly one option per choice should carry this.
    #[serde(default)]
    pub recommended: bool,
    /// Nodes this option would ripple into if selected. Hovering the option
    /// highlights them on the canvas.
    #[serde(default)]
    pub affects: Vec<NodeId>,
}

/// A reference to another choice, by the node it lives on and its id. Used to
/// declare choice dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChoiceRef {
    pub node: NodeId,
    pub choice: ChoiceId,
}

/// A decision point the agent attaches to the node it belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub id: ChoiceId,
    /// The question, e.g. "Which persistence approach?".
    pub prompt: String,
    /// Why this decision exists / what triggered it.
    #[serde(default)]
    pub rationale: Option<String>,
    pub options: Vec<ChoiceOption>,
    #[serde(default)]
    pub selected: Option<OptionId>,
    #[serde(default)]
    pub status: ChoiceStatus,
    /// Other choices that must be resolved before this one is actionable. The
    /// UI locks a dependent ("waiting on X", options disabled) until every
    /// listed choice is decided or dismissed; cycles are rejected at the API.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<ChoiceRef>,
    /// Set by the store when a choice this one depended on is reopened, so the
    /// agent knows to re-scope it. Cleared when the agent re-scopes the choice.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub needs_review: bool,
    /// Agent-set escape hatch: a re-upserted choice may replace an already
    /// `Decided` one only when this is `true`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reopen: bool,
}

impl Choice {
    pub fn is_open(&self) -> bool {
        self.status == ChoiceStatus::Open
    }
}

/// A free-text note the user attached to a node. User-owned: agent upserts
/// never remove or rewrite notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub id: NoteId,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

/// What a freehand annotation draws. Deliberately *not* graph structure — a
/// margin layer over the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    /// A sticky note anchored at `(x, y)`.
    #[default]
    Note,
    /// An arrow from `(x, y)` to `(x + w, y + h)`.
    Arrow,
    /// A translucent highlight rectangle `(x, y, w, h)`.
    Region,
}

/// A freehand annotation drawn on the canvas: a sticky note, arrow, or
/// highlight region. Origin-tracked like user nodes (user-drawn annotations
/// survive agent proposes) and delivered to the agent as annotation events.
/// Geometry is unified: `(x, y)` is the anchor/start, `(w, h)` the extent/delta
/// (unused for a note).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    pub id: AnnotationId,
    #[serde(default)]
    pub kind: AnnotationKind,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
    pub h: f64,
    /// Note body, or optional label for an arrow/region.
    #[serde(default)]
    pub text: String,
    /// Who drew it. Forced to `User` from the UI.
    #[serde(default, skip_serializing_if = "Origin::is_agent")]
    pub origin: Origin,
}

/// A free-form question the agent poses to the user, optionally attached to a
/// node. Unlike a [`Choice`] it has no options — the user replies with text.
/// The `answer` is user-owned: agent re-asks (upserts) never clobber it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: QuestionId,
    /// The open question, e.g. "What's the target deploy environment?".
    pub prompt: String,
    /// Optional node this question is about; the panel shows it inline there.
    #[serde(default)]
    pub node_id: Option<NodeId>,
    /// Why the agent is asking / what context prompted it.
    #[serde(default)]
    pub rationale: Option<String>,
    /// The user's free-text answer once given. User-owned.
    #[serde(default)]
    pub answer: Option<String>,
    /// When the user answered. Set alongside `answer`.
    #[serde(default)]
    pub answered_at: Option<DateTime<Utc>>,
}

impl Question {
    pub fn is_answered(&self) -> bool {
        self.answer.is_some()
    }
}

/// One architecture component on the canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: NodeId,
    pub label: String,
    #[serde(default)]
    pub kind: NodeKind,
    /// Short markdown-ish description shown on the card.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: ElementStatus,
    /// Implementation-tracking lifecycle the agent advances as it ships this
    /// component. `None` = not tracked. Agent-owned; preserved across upserts
    /// that do not restate it (like `position` for the user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildStatus>,
    /// Optional subsystem/layer grouping label.
    #[serde(default)]
    pub group: Option<String>,
    /// Optional swimlane label. Constrains the layered layout to a labeled
    /// horizontal band. Agent-assignable; user-overridable via the edit form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// User-owned; preserved across agent upserts.
    #[serde(default)]
    pub notes: Vec<Note>,
    /// In a multi-agent session, the id of the agent that authored this node.
    /// Drives color/badge attribution and routes the user's decisions on it
    /// back to that agent. `None` in single-agent sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// User-owned drag override. `None` means auto-layout places the node.
    #[serde(default)]
    pub position: Option<Point>,
    /// Who created this node. Forced to `Agent` for MCP-supplied nodes.
    #[serde(default, skip_serializing_if = "Origin::is_agent")]
    pub origin: Origin,
}

impl Node {
    pub fn choice(&self, id: &ChoiceId) -> Option<&Choice> {
        self.choices.iter().find(|c| &c.id == id)
    }

    pub fn open_choice_count(&self) -> usize {
        self.choices.iter().filter(|c| c.is_open()).count()
    }

    /// Merge an agent-authored upsert into this node.
    ///
    /// The agent wins on content; the user wins on `position`, `notes`, and
    /// already-`Decided` choices (unless the incoming choice sets `reopen`).
    pub fn merge_from_agent(&mut self, incoming: Node) {
        let previous = std::mem::replace(self, incoming);
        self.position = self.position.or(previous.position);
        // Build progress and the swimlane carry forward unless this upsert
        // restates them, so an unrelated re-upsert (or a re-propose, or a user
        // lane override) is never silently reset.
        self.build = self.build.or(previous.build);
        self.lane = self.lane.take().or(previous.lane);
        self.notes = previous.notes;
        for choice in &mut self.choices {
            if choice.reopen {
                choice.reopen = false;
                continue;
            }
            if let Some(prev) = previous.choices.iter().find(|c| c.id == choice.id)
                && prev.status == ChoiceStatus::Decided
            {
                *choice = prev.clone();
            }
        }
    }
}

/// A directed relationship between two nodes. Identity is `(from, to, kind)`.
/// The arrowhead renders at `to`: `depends_on` points dependent → dependency
/// (`from` needs `to`), `data_flow` points along the data (`from` produces,
/// `to` consumes), `contains` points container → member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    /// Origin side: the dependent for `depends_on`, the producer for
    /// `data_flow`, the container for `contains`.
    pub from: NodeId,
    /// Target side (where the arrowhead lands): the dependency, the data
    /// consumer, or the contained member.
    pub to: NodeId,
    #[serde(default)]
    pub kind: EdgeKind,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub status: ElementStatus,
    /// Who created this edge. Forced to `Agent` for MCP-supplied edges.
    #[serde(default, skip_serializing_if = "Origin::is_agent")]
    pub origin: Origin,
}

impl Edge {
    pub fn key(&self) -> (&NodeId, &NodeId, EdgeKind) {
        (&self.from, &self.to, self.kind)
    }
}

/// The whole brainstorm document: what the agent proposes and the UI renders.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionDoc {
    #[serde(default = "SessionDoc::current_version")]
    pub version: u32,
    #[serde(default)]
    pub title: String,
    /// Bumped by the store on every mutation. Agent-supplied values are ignored.
    #[serde(default)]
    pub revision: u64,
    /// The node the agent is currently discussing; the canvas centers on it.
    #[serde(default)]
    pub focus: Option<NodeId>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Free-form agent questions awaiting (or holding) a text answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<Question>,
    /// Freehand annotations (sticky notes, arrows, highlight regions) — a
    /// margin layer, not graph structure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

impl SessionDoc {
    pub const VERSION: u32 = 1;

    fn current_version() -> u32 {
        Self::VERSION
    }

    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| &n.id == id)
    }

    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| &n.id == id)
    }

    pub fn open_choice_count(&self) -> usize {
        self.nodes.iter().map(Node::open_choice_count).sum()
    }

    pub fn question(&self, id: &QuestionId) -> Option<&Question> {
        self.questions.iter().find(|q| &q.id == id)
    }

    /// Questions the user has not yet answered.
    pub fn open_question_count(&self) -> usize {
        self.questions.iter().filter(|q| !q.is_answered()).count()
    }

    /// Dependencies of `choice` that are not yet resolved: the referenced
    /// choice is still `Open`, or the choice/node it names no longer exists (a
    /// dangling ref keeps the dependent locked so the gap is visible). Empty →
    /// the choice is actionable.
    pub fn unmet_dependencies<'a>(&self, choice: &'a Choice) -> Vec<&'a ChoiceRef> {
        choice
            .depends_on
            .iter()
            .filter(|dep| {
                self.node(&dep.node)
                    .and_then(|n| n.choice(&dep.choice))
                    .is_none_or(Choice::is_open)
            })
            .collect()
    }

    /// Whether `choice` is blocked by an unresolved dependency.
    pub fn is_choice_locked(&self, choice: &Choice) -> bool {
        !self.unmet_dependencies(choice).is_empty()
    }

    /// Check internal consistency. Errors reject a mutation; warnings ride
    /// along in tool results so the agent can fix them next turn.
    pub fn validate(&self) -> Validation {
        let mut v = Validation::default();
        let mut node_ids = std::collections::HashSet::new();
        for node in &self.nodes {
            if !node_ids.insert(&node.id) {
                v.error(format!("duplicate node id `{}`", node.id));
            }
            let mut choice_ids = std::collections::HashSet::new();
            for choice in &node.choices {
                if !choice_ids.insert(&choice.id) {
                    v.error(format!(
                        "duplicate choice id `{}` on node `{}`",
                        choice.id, node.id
                    ));
                }
                if choice.options.is_empty() {
                    v.error(format!(
                        "choice `{}` on node `{}` has no options",
                        choice.id, node.id
                    ));
                }
                let mut option_ids = std::collections::HashSet::new();
                for opt in &choice.options {
                    if !option_ids.insert(&opt.id) {
                        v.error(format!(
                            "duplicate option id `{}` in choice `{}` on node `{}`",
                            opt.id, choice.id, node.id
                        ));
                    }
                }
                if let Some(sel) = &choice.selected
                    && !choice.options.iter().any(|o| &o.id == sel)
                {
                    v.error(format!(
                        "choice `{}` on node `{}` selects unknown option `{sel}`",
                        choice.id, node.id
                    ));
                }
                if choice.options.iter().filter(|o| o.recommended).count() > 1 {
                    v.warn(format!(
                        "choice `{}` on node `{}` marks multiple options recommended",
                        choice.id, node.id
                    ));
                }
            }
        }
        let mut edge_keys = std::collections::HashSet::new();
        for edge in &self.edges {
            for end in [&edge.from, &edge.to] {
                if !node_ids.contains(end) {
                    v.error(format!(
                        "edge `{}` -> `{}` references unknown node `{end}`",
                        edge.from, edge.to
                    ));
                }
            }
            if !edge_keys.insert(edge.key()) {
                v.error(format!(
                    "duplicate edge `{}` -> `{}` ({:?})",
                    edge.from, edge.to, edge.kind
                ));
            }
        }
        // Dangling `affects` and `focus` are warnings: the agent may add the
        // target in a follow-up call, and the UI just skips the highlight.
        for node in &self.nodes {
            for choice in &node.choices {
                for opt in &choice.options {
                    for target in &opt.affects {
                        if !node_ids.contains(target) {
                            v.warn(format!(
                                "option `{}` in choice `{}` on node `{}` affects unknown node `{target}`",
                                opt.id, choice.id, node.id
                            ));
                        }
                    }
                }
            }
        }
        if let Some(focus) = &self.focus
            && !node_ids.contains(focus)
        {
            v.warn(format!("focus references unknown node `{focus}`"));
        }
        let mut question_ids = std::collections::HashSet::new();
        for question in &self.questions {
            if !question_ids.insert(&question.id) {
                v.error(format!("duplicate question id `{}`", question.id));
            }
            if let Some(node_id) = &question.node_id
                && !node_ids.contains(node_id)
            {
                v.warn(format!(
                    "question `{}` attaches to unknown node `{node_id}`",
                    question.id
                ));
            }
        }
        let mut annotation_ids = std::collections::HashSet::new();
        for annotation in &self.annotations {
            if !annotation_ids.insert(&annotation.id) {
                v.error(format!("duplicate annotation id `{}`", annotation.id));
            }
        }
        self.validate_choice_dependencies(&mut v);
        v
    }

    /// Dangling `depends_on` refs warn (the agent may add the parent next
    /// turn); dependency cycles are a hard error — they can never resolve.
    fn validate_choice_dependencies(&self, v: &mut Validation) {
        // Index every choice by (node, choice) → a dense id for the DFS.
        let mut index: std::collections::HashMap<(&NodeId, &ChoiceId), usize> =
            std::collections::HashMap::new();
        for node in &self.nodes {
            for choice in &node.choices {
                let next = index.len();
                index.entry((&node.id, &choice.id)).or_insert(next);
            }
        }
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); index.len()];
        for node in &self.nodes {
            for choice in &node.choices {
                let from = index[&(&node.id, &choice.id)];
                for dep in &choice.depends_on {
                    match index.get(&(&dep.node, &dep.choice)) {
                        Some(&to) => adj[from].push(to),
                        None => v.warn(format!(
                            "choice `{}` on node `{}` depends on unknown choice `{}` on node `{}`",
                            choice.id, node.id, dep.choice, dep.node
                        )),
                    }
                }
            }
        }
        // 3-color iterative DFS: a gray target on the stack is a back-edge.
        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Gray,
            Black,
        }
        let mut color = vec![Color::White; adj.len()];
        let mut cyclic = false;
        for start in 0..adj.len() {
            if color[start] != Color::White {
                continue;
            }
            color[start] = Color::Gray;
            let mut stack = vec![(start, 0usize)];
            while let Some(&mut (node, ref mut cursor)) = stack.last_mut() {
                if *cursor < adj[node].len() {
                    let next = adj[node][*cursor];
                    *cursor += 1;
                    match color[next] {
                        Color::Gray => cyclic = true,
                        Color::Black => {}
                        Color::White => {
                            color[next] = Color::Gray;
                            stack.push((next, 0));
                        }
                    }
                } else {
                    color[node] = Color::Black;
                    stack.pop();
                }
            }
        }
        if cyclic {
            v.error("choice dependencies form a cycle".into());
        }
    }
}

/// Outcome of [`SessionDoc::validate`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Validation {
    fn error(&mut self, msg: String) {
        self.errors.push(msg);
    }

    fn warn(&mut self, msg: String) {
        self.warnings.push(msg);
    }

    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// One patch step inside an `update_graph` tool call. Ops apply in order and
/// atomically: if the resulting doc fails validation, nothing is committed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GraphOp {
    /// Insert or replace a node (merge rules preserve user-owned fields).
    UpsertNode {
        node: Node,
    },
    RemoveNode {
        id: NodeId,
    },
    /// Insert or replace the edge with the same `(from, to, kind)`.
    UpsertEdge {
        edge: Edge,
    },
    /// Remove edges from `from` to `to`; all kinds unless `kind` is given.
    RemoveEdge {
        from: NodeId,
        to: NodeId,
        #[serde(default)]
        kind: Option<EdgeKind>,
    },
    /// Attach a choice to a node (replaces a same-id choice, merge rules apply).
    AddChoice {
        node_id: NodeId,
        choice: Choice,
    },
    /// Close a choice: mark it `Decided` with `selected`, or `Dismissed`.
    ResolveChoice {
        node_id: NodeId,
        choice_id: ChoiceId,
        #[serde(default)]
        selected: Option<OptionId>,
        #[serde(default)]
        dismiss: bool,
    },
    /// Attach (or re-word) a free-form question for the user to answer in
    /// text. Upserting a same-id question keeps any answer the user gave.
    Ask {
        question: Question,
    },
    /// Retract a question. No-op if it was never asked.
    RemoveQuestion {
        id: QuestionId,
    },
    SetStatus {
        id: NodeId,
        status: ElementStatus,
    },
    /// Advance (or clear, with `null`) a node's implementation-tracking
    /// lifecycle: planned → building → built → verified.
    SetBuild {
        id: NodeId,
        #[serde(default)]
        build: Option<BuildStatus>,
    },
    /// Assign (or clear, with `null`) a node's swimlane.
    SetLane {
        id: NodeId,
        #[serde(default)]
        lane: Option<String>,
    },
    SetFocus {
        #[serde(default)]
        id: Option<NodeId>,
    },
    SetTitle {
        title: String,
    },
    /// Post a message to the UI activity feed (shown to the user).
    Announce {
        message: String,
    },
}

/// Something the user did that the agent needs to hear about.
///
/// No `deny_unknown_fields` here: it is incompatible with `flatten`, and this
/// type flows outward (store → agent) rather than being agent-authored.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DecisionEvent {
    /// 1-based position in the session's append-only decision log.
    pub seq: u64,
    pub at: DateTime<Utc>,
    /// Multi-agent routing: the agent this decision is addressed to, captured
    /// when the event was recorded (the author of the node/choice it concerns)
    /// so re-authoring or removing that node later cannot misroute it. `None`
    /// for unclaimed decisions delivered to every agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_agent: Option<String>,
    #[serde(flatten)]
    pub kind: DecisionKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionKind {
    OptionSelected {
        node_id: NodeId,
        choice_id: ChoiceId,
        option_id: OptionId,
        /// Options the user explored (clicked/expanded) before deciding, in
        /// order. Hesitation is signal worth asking about.
        #[serde(default)]
        considered: Vec<OptionId>,
    },
    ChoiceDismissed {
        node_id: NodeId,
        choice_id: ChoiceId,
        #[serde(default)]
        reason: Option<String>,
    },
    NoteAdded {
        node_id: NodeId,
        note: Note,
    },
    /// The user answered a free-form agent question with text.
    QuestionAnswered {
        question_id: QuestionId,
        answer: String,
    },
    /// The user drew a freehand annotation.
    AnnotationAdded {
        annotation: Annotation,
    },
    /// The user moved or re-worded an annotation (carries its new state).
    AnnotationEdited {
        annotation: Annotation,
    },
    /// The user erased an annotation.
    AnnotationDeleted {
        annotation_id: AnnotationId,
    },
    /// The user pressed "Send to agent" (possibly with no other events —
    /// that means "reviewed, proceed").
    FlushRequested {
        #[serde(default)]
        comment: Option<String>,
    },
    /// The user created a node on the canvas. Enriching it via upsert adopts
    /// it — from then on the agent carries it forward like its own nodes.
    NodeAdded {
        node: Node,
    },
    /// The user edited a card. Treat the new content as canonical.
    NodeEdited {
        node_id: NodeId,
        label: String,
        node_kind: NodeKind,
        description: String,
        /// The node's swimlane after the edit (user override). `None` clears it.
        #[serde(default)]
        lane: Option<String>,
    },
    /// The user hard-deleted a node they created (its edges went with it).
    NodeDeleted {
        node_id: NodeId,
    },
    /// The user marked an agent-authored node `removed`. Apply the real
    /// removal via `update_graph` (or push back with reasons).
    RemovalRequested {
        node_id: NodeId,
    },
    /// The user drew an edge.
    EdgeAdded {
        from: NodeId,
        to: NodeId,
        edge_kind: EdgeKind,
    },
    /// The user deleted an edge (any origin — edges always hard-delete).
    EdgeDeleted {
        from: NodeId,
        to: NodeId,
        edge_kind: EdgeKind,
    },
}

#[derive(Deserialize)]
struct CurrentDecisionEvent {
    seq: u64,
    at: DateTime<Utc>,
    #[serde(default)]
    target_agent: Option<String>,
    #[serde(flatten)]
    kind: DecisionKind,
}

#[derive(Deserialize)]
struct LegacyDecisionEvent {
    seq: u64,
    at: DateTime<Utc>,
    #[serde(flatten)]
    kind: LegacyDecisionKind,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LegacyDecisionKind {
    NoteAdded {
        node_id: NodeId,
        text: String,
    },
    NodeAdded {
        node_id: NodeId,
        label: String,
        node_kind: NodeKind,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DecisionEventWire {
    Current(CurrentDecisionEvent),
    Legacy(LegacyDecisionEvent),
}

impl<'de> Deserialize<'de> for DecisionEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match DecisionEventWire::deserialize(deserializer)? {
            DecisionEventWire::Current(event) => Ok(Self {
                seq: event.seq,
                at: event.at,
                target_agent: event.target_agent,
                kind: event.kind,
            }),
            DecisionEventWire::Legacy(event) => {
                let kind = match event.kind {
                    LegacyDecisionKind::NoteAdded { node_id, text } => DecisionKind::NoteAdded {
                        node_id,
                        note: Note {
                            id: NoteId::new(format!("legacy-note-{}", event.seq)),
                            text,
                            created_at: event.at,
                        },
                    },
                    LegacyDecisionKind::NodeAdded {
                        node_id,
                        label,
                        node_kind,
                    } => DecisionKind::NodeAdded {
                        node: Node {
                            id: node_id,
                            label,
                            kind: node_kind,
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
                };
                Ok(Self {
                    seq: event.seq,
                    at: event.at,
                    target_agent: None,
                    kind,
                })
            }
        }
    }
}

/// Entry in the UI activity feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub at: DateTime<Utc>,
    pub origin: ActivityOrigin,
    pub text: String,
    /// In a multi-agent session, which agent produced this entry (for the
    /// color/badge attribution in the feed). `None` for user/system entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityOrigin {
    Agent,
    User,
    System,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_lookups_and_open_question_count_are_exact() {
        let mut doc = crate::demo::demo_doc();
        assert_eq!(doc.node(&NodeId::from("web-ui")).unwrap().label, "Web UI");
        assert!(doc.node(&NodeId::from("missing")).is_none());
        assert_eq!(doc.open_question_count(), 1);
        doc.questions[0].answer = Some("answer".into());
        assert_eq!(doc.open_question_count(), 0);
    }

    fn option(id: &str) -> ChoiceOption {
        ChoiceOption {
            id: OptionId::from(id),
            label: id.to_owned(),
            summary: String::new(),
            pros: vec![],
            cons: vec![],
            recommended: false,
            affects: vec![],
        }
    }

    fn choice(id: &str, options: &[&str]) -> Choice {
        Choice {
            id: ChoiceId::from(id),
            prompt: format!("pick {id}"),
            rationale: None,
            options: options.iter().map(|o| option(o)).collect(),
            selected: None,
            status: ChoiceStatus::Open,
            depends_on: vec![],
            needs_review: false,
            reopen: false,
        }
    }

    fn node(id: &str) -> Node {
        Node {
            id: NodeId::from(id),
            label: id.to_owned(),
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

    #[test]
    fn doc_round_trips_through_json() {
        let mut n = node("api");
        n.choices
            .push(choice("persistence", &["sqlite", "postgres"]));
        n.notes.push(Note {
            id: NoteId::from("note-1"),
            text: "prefer simple".into(),
            created_at: Utc::now(),
        });
        n.position = Some(Point { x: 10.0, y: 20.5 });
        let doc = SessionDoc {
            version: SessionDoc::VERSION,
            title: "t".into(),
            revision: 7,
            focus: Some(NodeId::from("api")),
            nodes: vec![n, node("db")],
            edges: vec![edge("api", "db")],
            questions: vec![Question {
                id: QuestionId::from("deploy-target"),
                prompt: "Which environment ships first?".into(),
                node_id: Some(NodeId::from("api")),
                rationale: None,
                answer: Some("staging".into()),
                answered_at: Some(Utc::now()),
            }],
            annotations: vec![Annotation {
                id: AnnotationId::from("a1"),
                kind: AnnotationKind::Note,
                x: 10.0,
                y: 20.0,
                w: 0.0,
                h: 0.0,
                text: "revisit this".into(),
                origin: Origin::User,
            }],
        };
        let json = serde_json::to_string_pretty(&doc).unwrap();
        let back: SessionDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn origin_defaults_to_agent_and_is_skipped_in_json() {
        // Agent-facing JSON is unchanged: origin absent parses as Agent and
        // agent origin never serializes.
        let n: Node = serde_json::from_str(r#"{"id":"a","label":"A"}"#).unwrap();
        assert_eq!(n.origin, Origin::Agent);
        let out = serde_json::to_string(&n).unwrap();
        assert!(
            !out.contains("origin"),
            "agent origin must not serialize: {out}"
        );

        let mut user = n.clone();
        user.origin = Origin::User;
        let out = serde_json::to_string(&user).unwrap();
        assert!(
            out.contains(r#""origin":"user""#),
            "user origin must persist: {out}"
        );
        let back: Node = serde_json::from_str(&out).unwrap();
        assert_eq!(back.origin, Origin::User);
    }

    #[test]
    fn editing_event_wire_tags() {
        let at = Utc::now();
        let events = vec![
            DecisionEvent {
                seq: 1,
                at,
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
            },
            DecisionEvent {
                seq: 2,
                at,
                target_agent: None,
                kind: DecisionKind::NodeEdited {
                    node_id: "rate-limiter".into(),
                    label: "Rate Limiter v2".into(),
                    node_kind: NodeKind::Service,
                    description: "throttles".into(),
                    lane: Some("infra".into()),
                },
            },
            DecisionEvent {
                seq: 3,
                at,
                target_agent: None,
                kind: DecisionKind::NodeDeleted {
                    node_id: "rate-limiter".into(),
                },
            },
            DecisionEvent {
                seq: 4,
                at,
                target_agent: None,
                kind: DecisionKind::RemovalRequested {
                    node_id: "api".into(),
                },
            },
            DecisionEvent {
                seq: 5,
                at,
                target_agent: None,
                kind: DecisionKind::EdgeAdded {
                    from: "a".into(),
                    to: "b".into(),
                    edge_kind: EdgeKind::DataFlow,
                },
            },
            DecisionEvent {
                seq: 6,
                at,
                target_agent: None,
                kind: DecisionKind::EdgeDeleted {
                    from: "a".into(),
                    to: "b".into(),
                    edge_kind: EdgeKind::DataFlow,
                },
            },
        ];
        let json = serde_json::to_string(&events).unwrap();
        for tag in [
            r#""kind":"node_added""#,
            r#""kind":"node_edited""#,
            r#""kind":"node_deleted""#,
            r#""kind":"removal_requested""#,
            r#""kind":"edge_added""#,
            r#""kind":"edge_deleted""#,
        ] {
            assert!(json.contains(tag), "missing {tag} in: {json}");
        }
        let back: Vec<DecisionEvent> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, events, "round-trip");
    }

    #[test]
    fn edge_origin_defaults_and_round_trips() {
        let e: Edge = serde_json::from_str(r#"{"from":"a","to":"b"}"#).unwrap();
        assert_eq!(e.origin, Origin::Agent);
        let out = serde_json::to_string(&e).unwrap();
        assert!(!out.contains("origin"), "in: {out}");

        let mut user = e.clone();
        user.origin = Origin::User;
        let out = serde_json::to_string(&user).unwrap();
        assert!(out.contains(r#""origin":"user""#), "in: {out}");
        let back: Edge = serde_json::from_str(&out).unwrap();
        assert_eq!(back.origin, Origin::User);
    }

    #[test]
    fn agent_json_with_defaults_parses() {
        let doc: SessionDoc = serde_json::from_str(
            r#"{
                "title": "minimal",
                "nodes": [
                    {"id": "a", "label": "A"},
                    {"id": "b", "label": "B", "kind": "data_store", "status": "existing"}
                ],
                "edges": [{"from": "a", "to": "b"}]
            }"#,
        )
        .unwrap();
        assert_eq!(doc.version, SessionDoc::VERSION);
        assert_eq!(doc.nodes[0].kind, NodeKind::Component);
        assert_eq!(doc.nodes[1].kind, NodeKind::DataStore);
        assert_eq!(doc.nodes[1].status, ElementStatus::Existing);
        assert_eq!(doc.edges[0].kind, EdgeKind::DependsOn);
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = serde_json::from_str::<Node>(r#"{"id": "a", "label": "A", "colour": "red"}"#)
            .unwrap_err();
        assert!(err.to_string().contains("colour"), "{err}");
    }

    #[test]
    fn unknown_node_kind_degrades_to_other() {
        let n: Node =
            serde_json::from_str(r#"{"id": "a", "label": "A", "kind": "blockchain"}"#).unwrap();
        assert_eq!(n.kind, NodeKind::Other);
    }

    #[test]
    fn decision_event_wire_format_is_flat_and_tagged() {
        let ev = DecisionEvent {
            seq: 3,
            at: Utc::now(),
            target_agent: None,
            kind: DecisionKind::OptionSelected {
                node_id: NodeId::from("api"),
                choice_id: ChoiceId::from("persistence"),
                option_id: OptionId::from("sqlite"),
                considered: vec![OptionId::from("postgres"), OptionId::from("sqlite")],
            },
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "option_selected");
        assert_eq!(json["seq"], 3);
        assert_eq!(json["option_id"], "sqlite");
        let back: DecisionEvent = serde_json::from_value(json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn graph_op_wire_format() {
        let op: GraphOp = serde_json::from_str(
            r#"{"op": "resolve_choice", "node_id": "api", "choice_id": "persistence", "selected": "sqlite"}"#,
        )
        .unwrap();
        assert!(matches!(op, GraphOp::ResolveChoice { .. }));
        let op: GraphOp = serde_json::from_str(r#"{"op": "announce", "message": "hi"}"#).unwrap();
        assert!(matches!(op, GraphOp::Announce { .. }));
    }

    #[test]
    fn ask_op_and_question_answer_wire_format() {
        let op: GraphOp = serde_json::from_str(
            r#"{"op": "ask", "question": {"id": "deploy", "prompt": "Which env first?", "node_id": "api"}}"#,
        )
        .unwrap();
        match op {
            GraphOp::Ask { question } => {
                assert_eq!(question.id, QuestionId::from("deploy"));
                assert_eq!(question.node_id, Some(NodeId::from("api")));
                assert!(question.answer.is_none());
            }
            _ => panic!("expected ask"),
        }
        let op: GraphOp =
            serde_json::from_str(r#"{"op": "remove_question", "id": "deploy"}"#).unwrap();
        assert!(matches!(op, GraphOp::RemoveQuestion { .. }));

        let ev = DecisionEvent {
            seq: 1,
            at: Utc::now(),
            target_agent: None,
            kind: DecisionKind::QuestionAnswered {
                question_id: QuestionId::from("deploy"),
                answer: "staging".into(),
            },
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "question_answered");
        assert_eq!(json["answer"], "staging");
        let back: DecisionEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn set_lane_op_wire_and_merge_preserves_lane() {
        let op: GraphOp =
            serde_json::from_str(r#"{"op":"set_lane","id":"api","lane":"backend"}"#).unwrap();
        match op {
            GraphOp::SetLane { id, lane } => {
                assert_eq!(id, NodeId::from("api"));
                assert_eq!(lane.as_deref(), Some("backend"));
            }
            _ => panic!("expected set_lane"),
        }
        let op: GraphOp = serde_json::from_str(r#"{"op":"set_lane","id":"api"}"#).unwrap();
        assert!(matches!(op, GraphOp::SetLane { lane: None, .. }));

        // A user lane override survives an agent upsert that doesn't restate it.
        let mut current = node("a");
        current.lane = Some("frontend".into());
        current.merge_from_agent(node("a"));
        assert_eq!(current.lane.as_deref(), Some("frontend"));
        // An upsert that restates the lane wins.
        let mut incoming = node("a");
        incoming.lane = Some("backend".into());
        current.merge_from_agent(incoming);
        assert_eq!(current.lane.as_deref(), Some("backend"));
    }

    #[test]
    fn choice_dependencies_lock_and_warn_on_dangling() {
        // depends_on round-trips over the wire.
        let c: Choice = serde_json::from_str(
            r#"{"id":"b","prompt":"?","options":[{"id":"x","label":"X"}],"depends_on":[{"node":"n","choice":"a"}]}"#,
        )
        .unwrap();
        assert_eq!(
            c.depends_on,
            vec![ChoiceRef {
                node: NodeId::from("n"),
                choice: ChoiceId::from("a")
            }]
        );

        let mut doc = SessionDoc {
            nodes: vec![node("n")],
            ..Default::default()
        };
        let mut child = choice("b", &["y"]);
        child.depends_on = vec![ChoiceRef {
            node: NodeId::from("n"),
            choice: ChoiceId::from("a"),
        }];
        doc.nodes[0].choices = vec![choice("a", &["x"]), child];
        // Locked while the parent is open.
        assert!(doc.is_choice_locked(&doc.nodes[0].choices[1]));
        doc.nodes[0].choices[0].status = ChoiceStatus::Decided;
        assert!(!doc.is_choice_locked(&doc.nodes[0].choices[1]));
        // A dismissed parent also unblocks.
        doc.nodes[0].choices[0].status = ChoiceStatus::Dismissed;
        assert!(!doc.is_choice_locked(&doc.nodes[0].choices[1]));

        // A dangling dependency warns (not errors) and keeps the child locked.
        let mut doc2 = SessionDoc {
            nodes: vec![node("n")],
            ..Default::default()
        };
        let mut orphan = choice("b", &["y"]);
        orphan.depends_on = vec![ChoiceRef {
            node: NodeId::from("n"),
            choice: ChoiceId::from("ghost"),
        }];
        doc2.nodes[0].choices = vec![orphan];
        let v = doc2.validate();
        assert!(v.is_ok(), "dangling dep only warns: {:?}", v.errors);
        assert!(v.warnings.iter().any(|w| w.contains("unknown choice")));
        assert!(doc2.is_choice_locked(&doc2.nodes[0].choices[0]));
    }

    #[test]
    fn choice_dependency_cycles_are_rejected() {
        let mut doc = SessionDoc {
            nodes: vec![node("n")],
            ..Default::default()
        };
        let mut a = choice("a", &["x"]);
        let mut b = choice("b", &["y"]);
        a.depends_on = vec![ChoiceRef {
            node: NodeId::from("n"),
            choice: ChoiceId::from("b"),
        }];
        b.depends_on = vec![ChoiceRef {
            node: NodeId::from("n"),
            choice: ChoiceId::from("a"),
        }];
        doc.nodes[0].choices = vec![a, b];
        let v = doc.validate();
        assert!(!v.is_ok());
        assert!(v.errors.iter().any(|e| e.contains("cycle")));

        // A self-dependency is also a cycle.
        let mut doc2 = SessionDoc {
            nodes: vec![node("n")],
            ..Default::default()
        };
        let mut s = choice("s", &["x"]);
        s.depends_on = vec![ChoiceRef {
            node: NodeId::from("n"),
            choice: ChoiceId::from("s"),
        }];
        doc2.nodes[0].choices = vec![s];
        assert!(!doc2.validate().is_ok());
    }

    #[test]
    fn set_build_op_wire_and_merge_preserves_build() {
        let op: GraphOp =
            serde_json::from_str(r#"{"op":"set_build","id":"api","build":"verified"}"#).unwrap();
        match op {
            GraphOp::SetBuild { id, build } => {
                assert_eq!(id, NodeId::from("api"));
                assert_eq!(build, Some(BuildStatus::Verified));
            }
            _ => panic!("expected set_build"),
        }
        // Omitting build clears it.
        let op: GraphOp = serde_json::from_str(r#"{"op":"set_build","id":"api"}"#).unwrap();
        assert!(matches!(op, GraphOp::SetBuild { build: None, .. }));

        // Agent-owned but sticky: an upsert that doesn't restate build keeps it.
        let mut current = node("a");
        current.build = Some(BuildStatus::Built);
        current.merge_from_agent(node("a"));
        assert_eq!(current.build, Some(BuildStatus::Built));
        // A restating upsert wins.
        let mut incoming = node("a");
        incoming.build = Some(BuildStatus::Verified);
        current.merge_from_agent(incoming);
        assert_eq!(current.build, Some(BuildStatus::Verified));

        // `build: None` never serializes; a value round-trips.
        let plain = node("a");
        assert!(!serde_json::to_string(&plain).unwrap().contains("build"));
        assert!(
            serde_json::to_string(&current)
                .unwrap()
                .contains(r#""build":"verified""#)
        );
    }

    #[test]
    fn validate_catches_duplicate_question_ids_and_warns_dangling_attach() {
        let mut doc = SessionDoc {
            nodes: vec![node("api")],
            ..Default::default()
        };
        let q = |id: &str, node: Option<&str>| Question {
            id: QuestionId::from(id),
            prompt: "?".into(),
            node_id: node.map(NodeId::from),
            rationale: None,
            answer: None,
            answered_at: None,
        };
        doc.questions = vec![q("dup", None), q("dup", None), q("stray", Some("ghost"))];
        let v = doc.validate();
        assert!(v.errors.iter().any(|e| e.contains("duplicate question id")));
        assert!(
            v.warnings
                .iter()
                .any(|w| w.contains("unknown node `ghost`"))
        );
    }

    #[test]
    fn validate_catches_structural_errors() {
        let mut doc = SessionDoc {
            nodes: vec![node("a"), node("a")],
            edges: vec![edge("a", "ghost")],
            ..Default::default()
        };
        doc.nodes[0].choices.push(choice("c", &[]));
        let v = doc.validate();
        assert!(!v.is_ok());
        assert!(v.errors.iter().any(|e| e.contains("duplicate node id")));
        assert!(v.errors.iter().any(|e| e.contains("unknown node `ghost`")));
        assert!(v.errors.iter().any(|e| e.contains("has no options")));
    }

    #[test]
    fn validate_rejects_unknown_selection_and_duplicate_annotations() {
        let mut doc = SessionDoc {
            nodes: vec![node("a")],
            ..Default::default()
        };
        let mut c = choice("c", &["known"]);
        c.selected = Some("missing".into());
        doc.nodes[0].choices.push(c);
        let annotation = Annotation {
            id: "duplicate".into(),
            kind: AnnotationKind::Note,
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            text: String::new(),
            origin: Origin::User,
        };
        doc.annotations = vec![annotation.clone(), annotation];

        let v = doc.validate();
        assert!(
            v.errors
                .iter()
                .any(|e| e.contains("selects unknown option `missing`"))
        );
        assert!(
            v.errors
                .iter()
                .any(|e| e.contains("duplicate annotation id `duplicate`"))
        );
    }

    #[test]
    fn validate_warns_on_dangling_affects() {
        let mut doc = SessionDoc {
            nodes: vec![node("a")],
            ..Default::default()
        };
        let mut c = choice("c", &["x"]);
        c.options[0].affects.push(NodeId::from("missing"));
        doc.nodes[0].choices.push(c);
        let v = doc.validate();
        assert!(v.is_ok());
        assert_eq!(v.warnings.len(), 1);
    }

    #[test]
    fn merge_preserves_user_owned_fields() {
        let mut current = node("a");
        current.position = Some(Point { x: 1.0, y: 2.0 });
        current.notes.push(Note {
            id: NoteId::from("n1"),
            text: "keep me".into(),
            created_at: Utc::now(),
        });
        let mut decided = choice("stay", &["x", "y"]);
        decided.selected = Some(OptionId::from("x"));
        decided.status = ChoiceStatus::Decided;
        current.choices.push(decided);

        let mut incoming = node("a");
        incoming.label = "A v2".into();
        incoming.description = "updated".into();
        incoming.choices.push(choice("stay", &["x", "y", "z"]));

        current.merge_from_agent(incoming);
        assert_eq!(current.label, "A v2");
        assert_eq!(current.position, Some(Point { x: 1.0, y: 2.0 }));
        assert_eq!(current.notes.len(), 1);
        // Decided choice survives the upsert untouched.
        let c = current.choice(&ChoiceId::from("stay")).unwrap();
        assert_eq!(c.status, ChoiceStatus::Decided);
        assert_eq!(c.selected, Some(OptionId::from("x")));
        assert_eq!(c.options.len(), 2);
    }

    #[test]
    fn merge_reopen_overrides_decided_choice() {
        let mut current = node("a");
        let mut decided = choice("c", &["x"]);
        decided.selected = Some(OptionId::from("x"));
        decided.status = ChoiceStatus::Decided;
        current.choices.push(decided);

        let mut incoming = node("a");
        let mut fresh = choice("c", &["x", "y"]);
        fresh.reopen = true;
        incoming.choices.push(fresh);

        current.merge_from_agent(incoming);
        let c = current.choice(&ChoiceId::from("c")).unwrap();
        assert_eq!(c.status, ChoiceStatus::Open);
        assert_eq!(c.options.len(), 2);
        assert!(!c.reopen, "reopen flag is consumed, not persisted");
    }
}
