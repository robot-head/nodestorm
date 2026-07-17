//! Global user preferences (theme family + color mode).
//!
//! Deliberately separate from session persistence: preferences apply to
//! every session and must never enter undo snapshots, MCP payloads, or
//! exports — keeping them out of `SessionState` makes that structural.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::theme::{self, Mode};

/// The persisted preferences file (`preferences.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    /// Format version, for forward compatibility.
    #[serde(default = "Preferences::current_version")]
    pub version: u32,
    /// Theme family id (see [`crate::theme::FAMILIES`]).
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Color mode; `auto` follows the OS.
    #[serde(default)]
    pub mode: Mode,
}

impl Preferences {
    pub const VERSION: u32 = 1;

    fn current_version() -> u32 {
        Self::VERSION
    }
}

fn default_theme() -> String {
    theme::DEFAULT_FAMILY.to_owned()
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            theme: default_theme(),
            mode: Mode::default(),
        }
    }
}

/// `preferences.json` in the platform data dir (next to the session data).
pub fn default_prefs_path() -> anyhow::Result<PathBuf> {
    let legacy = crate::persist::default_session_path()?;
    let parent = legacy.parent().unwrap_or_else(|| Path::new("."));
    Ok(parent.join("preferences.json"))
}

/// Load preferences. Missing file → defaults. A corrupt file is moved aside
/// (`.json.corrupt`, never silently clobbered) and yields defaults. An
/// unknown theme id falls back to the default family, preserving the mode.
pub fn load_or_default(path: &Path) -> Preferences {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Preferences::default(),
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "cannot read preferences file");
            return Preferences::default();
        }
    };
    let mut prefs = match serde_json::from_slice::<Preferences>(&bytes) {
        Ok(p) => p,
        Err(err) => {
            let backup = path.with_extension("json.corrupt");
            tracing::warn!(%err, backup = %backup.display(), "preferences unreadable — moving aside");
            let _ = std::fs::rename(path, &backup);
            return Preferences::default();
        }
    };
    if theme::family(&prefs.theme).is_none() {
        tracing::warn!(theme = %prefs.theme, "unknown theme in preferences — using default");
        prefs.theme = default_theme();
    }
    prefs
}

/// Atomic save (temp file + rename, like session saves).
pub fn save(path: &Path, prefs: &Preferences) -> anyhow::Result<()> {
    crate::persist::write_atomic(path, &serde_json::to_vec_pretty(prefs)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nodestorm-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn defaults_are_nodestorm_auto() {
        let prefs = Preferences::default();
        assert_eq!(prefs.version, Preferences::VERSION);
        assert_eq!(prefs.theme, theme::DEFAULT_FAMILY);
        assert_eq!(prefs.mode, Mode::Auto);
        assert_eq!(
            default_prefs_path().unwrap().file_name().unwrap(),
            "preferences.json"
        );
    }

    #[test]
    fn save_load_round_trip() {
        let path = tmp_path("prefs-roundtrip.json");
        let prefs = Preferences {
            version: Preferences::VERSION,
            theme: "gruvbox".into(),
            mode: Mode::Light,
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load_or_default(&path), prefs);
        // No temp residue.
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        assert!(!PathBuf::from(tmp).exists());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_file_is_default() {
        assert_eq!(
            load_or_default(Path::new("/definitely/not/here.json")),
            Preferences::default()
        );
    }

    #[test]
    fn corrupt_file_is_default_and_moved_aside() {
        let path = tmp_path("prefs-corrupt.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(load_or_default(&path), Preferences::default());
        assert!(!path.exists(), "corrupt file moved away");
        let backup = path.with_extension("json.corrupt");
        assert!(backup.exists());
        std::fs::remove_file(backup).ok();
    }

    #[test]
    fn unknown_theme_falls_back_preserving_mode() {
        let path = tmp_path("prefs-unknown-theme.json");
        std::fs::write(
            &path,
            br#"{ "version": 1, "theme": "vaporwave", "mode": "light" }"#,
        )
        .unwrap();
        let prefs = load_or_default(&path);
        assert_eq!(prefs.theme, theme::DEFAULT_FAMILY);
        assert_eq!(prefs.mode, Mode::Light);
        assert!(path.exists(), "a parseable file is not moved aside");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn json_uses_lowercase_mode() {
        // The E2E script parses this file; the wire format is part of the
        // contract.
        let path = tmp_path("prefs-wire.json");
        let prefs = Preferences {
            version: 1,
            theme: "gruvbox".into(),
            mode: Mode::Light,
        };
        save(&path, &prefs).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"theme\": \"gruvbox\""), "raw: {raw}");
        assert!(raw.contains("\"mode\": \"light\""), "raw: {raw}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_fields_get_defaults() {
        let path = tmp_path("prefs-sparse.json");
        std::fs::write(&path, br"{}").unwrap();
        assert_eq!(load_or_default(&path), Preferences::default());
        std::fs::remove_file(&path).ok();
    }
}
