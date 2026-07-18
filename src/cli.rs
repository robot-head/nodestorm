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

    /// Preferences file (default: `preferences.json` in the platform data
    /// dir). Lets tests and E2E runs avoid the real user preferences.
    #[arg(long)]
    pub prefs: Option<PathBuf>,

    /// Load the built-in demo graph instead of restoring the last session.
    #[arg(long)]
    pub demo: bool,

    /// Load a deterministic N-component graph (scaling checks).
    #[arg(long, value_name = "N")]
    pub demo_big: Option<usize>,

    /// Run the MCP server without opening a window (for CI and agent-only use).
    #[arg(long)]
    pub headless: bool,

    /// Initial window size in logical pixels, `WIDTHxHEIGHT` (default
    /// 1280x840). Lets demo recordings and E2E runs launch at a target
    /// size instead of resizing a running window.
    #[arg(long, value_name = "WxH", value_parser = parse_window_size)]
    pub window_size: Option<(f64, f64)>,
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

    /// The global preferences file: the `--prefs` override, or the
    /// platform default.
    pub fn prefs_path(&self) -> anyhow::Result<PathBuf> {
        match &self.prefs {
            Some(path) => Ok(path.clone()),
            None => crate::prefs::default_prefs_path(),
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

/// Parse `760x840` into logical (width, height); both in 200..=10000.
fn parse_window_size(s: &str) -> Result<(f64, f64), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("expected WIDTHxHEIGHT, e.g. 760x840, got '{s}'"))?;
    let dim = |v: &str, name: &str| -> Result<f64, String> {
        let n: f64 = v
            .trim()
            .parse()
            .map_err(|_| format!("{name} is not a number in '{s}'"))?;
        if (200.0..=10_000.0).contains(&n) {
            Ok(n)
        } else {
            Err(format!("{name} out of range 200..=10000 in '{s}'"))
        }
    };
    Ok((dim(w, "width")?, dim(h, "height")?))
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

    #[test]
    fn prefs_path_prefers_override() {
        let cli = Cli::parse_from(["nodestorm", "--prefs", "some/dir/my-prefs.json"]);
        assert_eq!(
            cli.prefs_path().unwrap(),
            PathBuf::from("some/dir/my-prefs.json")
        );
        let cli = Cli::parse_from(["nodestorm"]);
        assert_eq!(
            cli.prefs_path().unwrap(),
            crate::prefs::default_prefs_path().unwrap()
        );
    }

    #[test]
    fn window_size_parses_and_validates() {
        let cli = Cli::parse_from(["nodestorm", "--window-size", "760x840"]);
        assert_eq!(cli.window_size, Some((760.0, 840.0)));
        assert_eq!(Cli::parse_from(["nodestorm"]).window_size, None);
        assert!(Cli::try_parse_from(["nodestorm", "--window-size", "760"]).is_err());
        assert!(Cli::try_parse_from(["nodestorm", "--window-size", "10x840"]).is_err());
        assert!(Cli::try_parse_from(["nodestorm", "--window-size", "axb"]).is_err());
    }
}
