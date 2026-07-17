//! Command-line interface for nodestorm.

use std::path::PathBuf;

use clap::Parser;

/// Visual brainstorming canvas for agentic AI planning.
///
/// Hosts an MCP server (streamable HTTP) that agents like Claude Code use to
/// push architecture graphs and await your decisions.
#[derive(Debug, Clone, Parser)]
#[command(name = "nodestorm", version, about)]
pub struct Cli {
    /// Port for the MCP server (bound to 127.0.0.1 only).
    #[arg(long, default_value_t = 4747)]
    pub port: u16,

    /// Session file to load and autosave (defaults to the XDG data dir).
    #[arg(long)]
    pub session: Option<PathBuf>,

    /// Load the built-in demo graph instead of restoring the last session.
    #[arg(long)]
    pub demo: bool,

    /// Run the MCP server without opening a window (for CI and agent-only use).
    #[arg(long)]
    pub headless: bool,
}

impl Cli {
    /// The MCP endpoint URL agents should connect to.
    pub fn mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    /// The session file to load and autosave: the `--session` override, or
    /// the platform default.
    pub fn session_path(&self) -> anyhow::Result<PathBuf> {
        match &self.session {
            Some(path) => Ok(path.clone()),
            None => crate::persist::default_session_path(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_path_prefers_override() {
        let cli = Cli::parse_from(["nodestorm", "--session", "some/dir/mine.json"]);
        assert_eq!(
            cli.session_path().unwrap(),
            PathBuf::from("some/dir/mine.json")
        );
        let cli = Cli::parse_from(["nodestorm"]);
        assert_eq!(
            cli.session_path().unwrap(),
            crate::persist::default_session_path().unwrap()
        );
    }
}
