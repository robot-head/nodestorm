//! nodestorm — visual brainstorming canvas for agentic AI planning.
//!
//! An agent (e.g. Claude Code in plan mode) connects over MCP and pushes a
//! system-architecture graph: components as nodes, dependencies as edges,
//! pending implementation choices attached to the nodes they belong to. The
//! human picks options in the UI; the agent blocks on `await_decisions` until
//! the decisions are sent back.

pub mod agent_launcher;
pub mod cli;
pub mod demo;
pub mod diff;
pub mod export;
pub mod icon;
pub mod layout;
pub mod model;
pub mod persist;
pub mod prefs;
pub mod server;
pub mod sessions;
pub mod store;
pub mod terminal;
pub mod theme;
pub mod ui;
