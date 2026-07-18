//! Theme registry: the palette families and color modes the UI offers.
//!
//! The actual colors live in `assets/main.css` as `[data-theme="<id>"]`
//! blocks of `light-dark()` custom properties; this module is the Rust-side
//! source of truth for *which* families exist. The colocated tests
//! `include_str!` the stylesheet and keep the two in lock-step: every
//! family here must have a CSS block defining all [`REQUIRED_TOKENS`], and
//! every `[data-theme]` block in the CSS must be registered here.

use serde::{Deserialize, Serialize};

/// One selectable palette family; its dark and light variants live in CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeFamily {
    /// Stable id used in the `data-theme` attribute and `preferences.json`.
    pub id: &'static str,
    /// Display name for the picker.
    pub name: &'static str,
}

/// Every selectable family, in picker order.
pub const FAMILIES: [ThemeFamily; 12] = [
    ThemeFamily {
        id: "nodestorm",
        name: "Nodestorm",
    },
    ThemeFamily {
        id: "solarized",
        name: "Solarized",
    },
    ThemeFamily {
        id: "gruvbox",
        name: "Gruvbox",
    },
    ThemeFamily {
        id: "catppuccin",
        name: "Catppuccin",
    },
    ThemeFamily {
        id: "nord",
        name: "Nord",
    },
    ThemeFamily {
        id: "dracula",
        name: "Dracula",
    },
    ThemeFamily {
        id: "tokyo-night",
        name: "Tokyo Night",
    },
    ThemeFamily {
        id: "one",
        name: "One",
    },
    ThemeFamily {
        id: "github",
        name: "GitHub",
    },
    ThemeFamily {
        id: "everforest",
        name: "Everforest",
    },
    ThemeFamily {
        id: "rose-pine",
        name: "Rosé Pine",
    },
    ThemeFamily {
        id: "monokai",
        name: "Monokai",
    },
];

/// Fallback family for fresh installs and unknown ids in a prefs file.
pub const DEFAULT_FAMILY: &str = "nodestorm";

/// Every custom property a `[data-theme]` block must define.
pub const REQUIRED_TOKENS: [&str; 20] = [
    "--bg",
    "--bg-panel",
    "--bg-card",
    "--bg-card-hover",
    "--border",
    "--text",
    "--text-dim",
    "--accent",
    "--status-existing",
    "--status-proposed",
    "--status-modified",
    "--status-affected",
    "--status-removed",
    "--badge-open",
    "--badge-decided",
    "--badge-notes",
    "--on-accent",
    "--on-badge",
    "--shadow",
    "--dot-grid",
];

/// Color mode. `Auto` follows the OS (`color-scheme: light dark` lets the
/// CSS `light-dark()` values track `prefers-color-scheme` live).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Auto,
    Dark,
    Light,
}

impl Mode {
    /// The `data-mode` attribute value (matches the serde form).
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Dark => "dark",
            Mode::Light => "light",
        }
    }
}

/// Look up a family by id.
pub fn family(id: &str) -> Option<&'static ThemeFamily> {
    FAMILIES.iter().find(|f| f.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSS: &str = include_str!("../assets/main.css");

    /// Tokens that can never share a value between light and dark variants,
    /// so every family block must define them via `light-dark()`. Accent-ish
    /// tokens may be single-valued (Solarized's accents are designed to read
    /// on both backgrounds).
    const STRUCTURAL_TOKENS: [&str; 7] = [
        "--bg",
        "--bg-panel",
        "--bg-card",
        "--bg-card-hover",
        "--border",
        "--text",
        "--text-dim",
    ];

    /// The declaration body of the first CSS block for `selector`.
    /// Token blocks are flat (no nested braces), so a naive brace scan works.
    fn block_for(selector: &str) -> &'static str {
        let start = CSS
            .find(selector)
            .unwrap_or_else(|| panic!("no CSS block for selector {selector}"));
        let open = CSS[start..].find('{').expect("opening brace") + start;
        let close = CSS[open..].find('}').expect("closing brace") + open;
        &CSS[open + 1..close]
    }

    fn assert_block_contains(selector: &str, declaration: &str) {
        let block = block_for(selector);
        assert!(
            block.contains(declaration),
            "{selector} must contain `{declaration}`, got: {block}"
        );
    }

    #[test]
    fn long_content_surfaces_have_overflow_contracts() {
        assert_block_contains(".panel {", "width: min(360px, 100vw)");
        assert_block_contains(".panel {", "overflow-x: hidden");
        assert_block_contains(".panel-head h2 {", "overflow-wrap: anywhere");
        assert_block_contains(".option-label {", "overflow-wrap: anywhere");
        assert_block_contains(".export-dropdown {", "max-height: calc(100vh - 64px)");
        assert_block_contains(".export-dropdown {", "overflow-y: auto");
        assert_block_contains(".activity.expanded {", "overflow-y: auto");
        assert_block_contains(".activity-text {", "overflow-wrap: anywhere");
        assert_block_contains(".diff-text {", "overflow-wrap: anywhere");
        assert_block_contains(".empty-cmd {", "max-width: 100%");
        assert_block_contains(".session-row > .sess-switch {", "width: auto");
        assert_block_contains(".session-row > .sess-switch {", "flex: 1 1 0");
        assert_block_contains(".session-row > .sess-switch {", "min-width: 0");
        assert_block_contains(".sess-name {", "flex: 1 1 auto");
    }

    #[test]
    fn family_ids_are_unique_and_slug_safe() {
        let mut seen = std::collections::BTreeSet::new();
        for f in FAMILIES {
            assert!(seen.insert(f.id), "duplicate family id {}", f.id);
            assert!(
                f.id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "family id {} is not slug-safe",
                f.id
            );
            assert!(!f.name.is_empty());
        }
        assert!(seen.contains(DEFAULT_FAMILY), "default family missing");
    }

    #[test]
    fn family_lookup_works() {
        assert_eq!(family("gruvbox").map(|f| f.name), Some("Gruvbox"));
        assert!(family("no-such-theme").is_none());
    }

    #[test]
    fn mode_serde_round_trips_lowercase() {
        for (mode, text) in [
            (Mode::Auto, "\"auto\""),
            (Mode::Dark, "\"dark\""),
            (Mode::Light, "\"light\""),
        ] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), text);
            assert_eq!(serde_json::from_str::<Mode>(text).unwrap(), mode);
        }
        assert_eq!(Mode::default(), Mode::Auto);
    }

    #[test]
    fn mode_as_str_matches_serde() {
        for mode in [Mode::Auto, Mode::Dark, Mode::Light] {
            let serde_form = serde_json::to_string(&mode).unwrap();
            assert_eq!(format!("\"{}\"", mode.as_str()), serde_form);
        }
    }

    #[test]
    fn root_fallback_defines_every_token() {
        let root = block_for(":root");
        for token in REQUIRED_TOKENS {
            assert!(
                root.contains(&format!("{token}:")),
                ":root fallback missing {token}"
            );
        }
        assert!(
            root.contains("--mini-viewport-fill:"),
            ":root missing the derived --mini-viewport-fill"
        );
    }

    #[test]
    fn every_family_has_a_block_defining_all_tokens() {
        for f in FAMILIES {
            let block = block_for(&format!("[data-theme=\"{}\"]", f.id));
            for token in REQUIRED_TOKENS {
                assert!(
                    block.contains(&format!("{token}:")),
                    "family {} missing {token}",
                    f.id
                );
            }
            for token in STRUCTURAL_TOKENS {
                let decl = block
                    .split(&format!("{token}:"))
                    .nth(1)
                    .and_then(|rest| rest.split(';').next())
                    .unwrap_or_default();
                assert!(
                    decl.contains("light-dark("),
                    "family {} must define {token} via light-dark()",
                    f.id
                );
            }
        }
    }

    #[test]
    fn no_orphan_theme_blocks_in_css() {
        for (i, _) in CSS.match_indices("[data-theme=\"") {
            let rest = &CSS[i + "[data-theme=\"".len()..];
            let id = rest.split('"').next().unwrap_or_default();
            assert!(
                family(id).is_some(),
                "CSS has a [data-theme=\"{id}\"] block with no registered family"
            );
        }
    }

    #[test]
    fn mode_rules_set_color_scheme() {
        assert!(block_for("[data-mode=\"dark\"]").contains("color-scheme: dark;"));
        assert!(block_for("[data-mode=\"light\"]").contains("color-scheme: light;"));
        assert!(block_for("[data-mode=\"auto\"]").contains("color-scheme: light dark;"));
    }

    #[test]
    fn family_blocks_are_supports_guarded() {
        let guard = CSS
            .find("@supports (color: light-dark(")
            .expect("no @supports light-dark() guard in CSS");
        for (i, _) in CSS.match_indices("[data-theme=\"") {
            assert!(
                i > guard,
                "a [data-theme] block sits outside the @supports guard"
            );
        }
    }
}
