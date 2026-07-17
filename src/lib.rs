//! nodestorm — visual brainstorming canvas for agentic AI planning.
//!
//! An agent (e.g. Claude Code in plan mode) connects over MCP and pushes a
//! system-architecture graph: components as nodes, dependencies as edges,
//! pending implementation choices attached to the nodes they belong to. The
//! human picks options in the UI; the agent blocks on `await_decisions` until
//! the decisions are sent back.

pub mod cli;
pub mod demo;
pub mod layout;
pub mod model;
pub mod persist;
pub mod server;
pub mod store;
pub mod ui;
