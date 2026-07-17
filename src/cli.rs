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
    /// Pins that exact file as a session named after the file stem.
    #[arg(long)]
    pub session: Option<PathBuf>,

    /// Directory holding the named sessions (default: `sessions/` next to
    /// the legacy session file in the platform data dir).
    #[arg(long)]
    pub sessions_dir: Option<PathBuf>,

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
    /// the platform default. With named sessions this is the *pinned*
    /// session's path (used by the UI to derive export paths).
    pub fn session_path(&self) -> anyhow::Result<PathBuf> {
        match &self.session {
            Some(path) => Ok(path.clone()),
            None => crate::persist::default_session_path(),
        }
    }

    /// Where named sessions live: the `--sessions-dir` override, or
    /// `sessions/` next to the platform-default session file.
    pub fn sessions_dir(&self) -> anyhow::Result<PathBuf> {
        match &self.sessions_dir {
            Some(dir) => Ok(dir.clone()),
            None => {
                let legacy = crate::persist::default_session_path()?;
                let parent = legacy.parent().unwrap_or_else(|| std::path::Path::new("."));
                Ok(parent.join("sessions"))
            }
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
