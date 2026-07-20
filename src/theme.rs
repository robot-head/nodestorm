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
    use yare::parameterized;

    const CSS: &str = include_str!("../assets/main.css");
    const APP_SOURCE: &str = include_str!("ui/app.rs");
    const TOPBAR_SOURCE: &str = include_str!("ui/topbar.rs");

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
    /// Contract rules are flat with one exact selector per line.
    fn block_for_in<'a>(css: &'a str, selector: &str) -> &'a str {
        let mut offset = 0;
        let start = css
            .split_inclusive('\n')
            .find_map(|line| {
                let start = offset;
                offset += line.len();
                let rest = line.trim_start().strip_prefix(selector)?;
                rest.find('{')
                    .filter(|open| rest[..*open].trim().is_empty())
                    .map(|_| start)
            })
            .unwrap_or_else(|| panic!("no CSS block for selector {selector}"));
        let open = css[start..].find('{').expect("opening brace") + start;
        let close = css[open..].find('}').expect("closing brace") + open;
        &css[open + 1..close]
    }

    fn block_for(selector: &str) -> &'static str {
        block_for_in(CSS, selector)
    }

    fn assert_block_contains(selector: &str, declaration: &str) {
        let block = block_for(selector);
        assert2::assert!(
            block.contains(declaration),
            "{selector} must contain `{declaration}`, got: {block}"
        );
    }

    #[test]
    fn topbar_uses_the_shared_graph_bolt_mark() {
        let brand = TOPBAR_SOURCE
            .split_once("span { class: \"topbar-brand\"")
            .expect("topbar brand")
            .1
            .split_once("div { class: \"export-menu\"")
            .expect("topbar export menu")
            .0;
        assert2::assert!(brand.contains("topbar-mark"));
        assert2::assert!(brand.contains("polyline"));
        assert2::assert!(brand.contains("BOLT_POINTS"));
        assert2::assert!(brand.contains("NODE_INDICES"));
        assert2::assert!(brand.contains("currentColor"));
        assert2::assert!(!brand.contains("mask"));
        assert2::assert!(!brand.contains("topbar-bolt-cutout"));
        assert2::assert!(!brand.contains("\"ϟ\""));
    }

    #[test]
    fn exact_selector_lookup_ignores_longer_selector() {
        let css = ".session-row.active .sess-name {\n\
                     color: red;\n\
                   }\n\
                   .sess-name {\n\
                     flex: 1 1 auto;\n\
                   }\n";

        assert2::assert!((block_for_in(css, ".sess-name").trim()) == ("flex: 1 1 auto;"));
    }

    #[test]
    fn long_content_surfaces_have_overflow_contracts() {
        assert_block_contains(".panel", "width: min(360px, 100vw)");
        assert_block_contains(".panel", "overflow-x: hidden");
        assert_block_contains(".panel-head h2", "overflow-wrap: anywhere");
        assert_block_contains(".option-label", "overflow-wrap: anywhere");
        assert_block_contains(".export-dropdown", "max-height: calc(100vh - 64px)");
        assert_block_contains(".export-dropdown", "overflow-y: auto");
        assert_block_contains(".activity.expanded", "overflow-y: auto");
        assert_block_contains(".activity.expanded", "z-index: 16");
        assert_block_contains(".activity-text", "overflow-wrap: anywhere");
        assert_block_contains(".diff-text", "overflow-wrap: anywhere");
        assert_block_contains(".empty-cmd", "max-width: 100%");
    }

    #[test]
    fn canvas_cards_raise_on_hover_and_selection() {
        assert_block_contains(".node-card", "z-index: 1");
        assert_block_contains(".node-card:hover", "z-index: 2");
        assert_block_contains(".node-card.selected", "z-index: 3");
    }

    #[test]
    fn swimlane_label_stays_above_cards() {
        assert_block_contains(".swimlane", "z-index: auto");
        assert_block_contains(".swimlane-label", "z-index: 4");
        assert_block_contains(
            ".swimlane-label",
            "background: color-mix(in srgb, var(--bg) 88%, transparent)",
        );
    }

    #[test]
    fn swimlane_labels_are_interactive() {
        // The label sits over an otherwise non-interactive band; its controls
        // must receive pointer events.
        assert_block_contains(".swimlane-label", "pointer-events: auto");
        assert2::assert!(
            !block_for(".add-lane-btn").trim().is_empty(),
            "add-lane control is styled"
        );
        assert2::assert!(
            !block_for(".lane-delete").trim().is_empty(),
            "delete control is styled"
        );
    }

    #[test]
    fn dragged_card_highlights_its_target_lane() {
        // The drop-target band is visibly distinct via the accent color.
        assert_block_contains(".swimlane.drop-target", "var(--accent)");
    }

    #[test]
    fn dropdowns_fit_the_supported_viewport() {
        assert_block_contains(
            ".export-dropdown",
            "min-width: min(190px, calc(100vw - 32px))",
        );
        assert_block_contains(".export-dropdown", "max-width: calc(100vw - 32px)");
        assert_block_contains(
            ".sessions-dropdown",
            "min-width: min(230px, calc(100vw - 64px))",
        );
        assert_block_contains(".sessions-dropdown", "max-width: calc(100vw - 64px)");
        assert_block_contains(
            ".more-dropdown",
            "min-width: min(230px, calc(100vw - 32px))",
        );
        assert_block_contains(".more-dropdown", "max-width: calc(100vw - 32px)");
        assert_block_contains(".theme-dropdown", "min-width: 0");
        assert_block_contains(".compose-pop", "min-width: min(260px, calc(100vw - 32px))");
    }

    #[test]
    fn status_chip_keeps_its_complete_wide_layout_label() {
        assert_block_contains(".status-chip", "flex-shrink: 0");
    }

    #[test]
    fn claude_connections_send_receipt_and_error_toast_are_accessible() {
        assert2::assert!(TOPBAR_SOURCE.contains(r#"aria_label: "Claude MCP connections""#));
        assert2::assert!(TOPBAR_SOURCE.contains(r#"class: "connection-row""#));
        assert2::assert!(APP_SOURCE.contains(r#"role: "alert""#));
        assert2::assert!(APP_SOURCE.contains(r#"store.dismiss_toast()"#));
        assert2::assert!(APP_SOURCE.contains(r#"aria_label: "Dismiss notification""#));
        assert_block_contains(".connection-pop", "max-height: calc(100vh - 64px)");
        assert_block_contains(".connection-row", "display: grid");
        assert_block_contains(".delivery-toast", "position: fixed");
        assert_block_contains(".delivery-toast", "z-index: 30");
        assert_block_contains(".delivery-toast-error", "color: var(--status-removed)");
        assert_block_contains(".btn-send.sent", "color: var(--on-badge)");
        assert_block_contains(".btn-send.failed", "color: var(--on-badge)");
    }

    #[test]
    fn connection_bridge_subscribes_before_its_initial_resnapshot() {
        let subscribe = APP_SOURCE
            .find("let mut changes = sessions.subscribe_connections()")
            .expect("connection bridge subscribes");
        let bridge = &APP_SOURCE[subscribe..];
        let resnapshot = bridge
            .find("connections.set(sessions.connections())")
            .expect("connection bridge immediately resnapshots");
        let await_change = bridge
            .find("while changes.changed().await.is_ok()")
            .expect("connection bridge awaits later changes");

        assert2::assert!(
            resnapshot < await_change,
            "the subscribe/snapshot gap must close before awaiting changes"
        );
    }

    #[test]
    fn rendered_handlers_use_the_snapshotted_store() {
        let helper = APP_SOURCE
            .find("pub fn use_store()")
            .map(|start| &APP_SOURCE[start..])
            .expect("render-bound store helper");
        assert2::assert!(helper.contains("use_context::<super::ActiveStore>()"));

        let toast = APP_SOURCE
            .find("if let Some(toast)")
            .map(|start| &APP_SOURCE[start..])
            .expect("toast renderer");
        assert2::assert!(toast.contains("let store = active_store.read().clone()"));
    }

    #[test]
    fn active_session_name_and_store_resolve_together() {
        let resolve = APP_SOURCE
            .find("let (name, store) = sessions")
            .map(|start| &APP_SOURCE[start..])
            .expect("the bridge resolves the active name and store together");
        assert2::assert!(resolve.contains(".resolve_named(None)"));
        assert2::assert!(resolve.contains("session_name.set(name)"));
    }

    #[test]
    fn active_store_bridge_subscribes_before_resnapshotting() {
        let active = APP_SOURCE
            .find("let (name, store) = sessions")
            .map(|start| &APP_SOURCE[start..])
            .expect("active store bridge");
        let subscribe = active
            .find("let mut rev = store.subscribe()")
            .expect("store revision subscription");
        let doc = active
            .find("doc.set(store.snapshot_doc())")
            .expect("doc snapshot");
        let meta = active
            .find("meta.set(store.snapshot_meta())")
            .expect("metadata snapshot");

        assert2::assert!(subscribe < doc && subscribe < meta);
    }

    #[test]
    fn empty_state_copy_uses_the_snapshotted_store() {
        let empty = APP_SOURCE
            .find(r#"div { class: "empty-state""#)
            .map(|start| &APP_SOURCE[start..])
            .expect("empty-state renderer");
        let handler_and_rest = empty
            .find(r#"title: "Copy the connect command""#)
            .map(|start| &empty[start..])
            .expect("copy handler");
        let handler = handler_and_rest
            .find(r#"code { "claude mcp add"#)
            .map(|end| &handler_and_rest[..end])
            .expect("end of copy handler");

        assert2::assert!(!handler.contains(r#"code { "claude mcp add"#));
        assert2::assert!(handler.contains("let store = active_store.read().clone()"));
        assert2::assert!(!handler.contains("sessions.active_store()"));
    }

    #[test]
    fn queue_panel_is_available_when_the_document_is_empty() {
        let empty_state = APP_SOURCE
            .find("} else {\n                    div { class: \"empty-state\"")
            .expect("App must render an empty state when there are no nodes");
        let queue_panel = APP_SOURCE
            .find("} else if queued_changes_open()")
            .expect("App must render the queued changes panel");

        assert2::assert!(
            queue_panel > empty_state,
            "the queued changes panel must not be gated by the canvas branch"
        );
    }

    #[test]
    fn queue_segment_button_resets_native_button_chrome() {
        for declaration in [
            "background: none;",
            "border: 0;",
            "cursor: pointer;",
            "font: inherit;",
        ] {
            assert_block_contains(".status-chip button.seg", declaration);
        }
    }

    #[test]
    fn opening_the_queue_closes_an_active_comparison() {
        let queue_button = TOPBAR_SOURCE
            .find("title: \"Review, edit, or remove queued changes\"")
            .expect("TopBar must provide a queue button");
        let queue_handler = &TOPBAR_SOURCE[queue_button..];
        let handler_end = queue_handler
            .find("},\n                            \"{queued_count}\"")
            .expect("queue button must have an onclick handler");
        let handler = &queue_handler[..handler_end];

        for reset in [
            "selected.set(None);",
            "timeline_open.set(false);",
            "compare_with.set(None);",
        ] {
            assert2::assert!(
                handler.contains(reset),
                "opening the queue must reset `{reset}` so it owns the right-panel slot"
            );
        }
    }

    #[test]
    fn minimum_viewport_keeps_topbar_and_menus_reachable() {
        const MEDIA: &str = "@media (max-width: 519px) {";
        let media_start = CSS
            .rfind(MEDIA)
            .unwrap_or_else(|| panic!("stylesheet must end with a {MEDIA} rule"));
        let rule = &CSS[media_start..];

        for base_selector in [".compose-pop {", ".theme-dropdown {"] {
            let base_start = CSS
                .find(base_selector)
                .unwrap_or_else(|| panic!("missing base {base_selector} rule"));
            assert2::assert!(
                media_start > base_start,
                "minimum-viewport rule must appear after base {base_selector} rule"
            );
        }

        let topbar = block_for_in(rule, ".topbar");
        for declaration in ["overflow-x: auto;", "scrollbar-width: none;"] {
            assert2::assert!(
                topbar.contains(declaration),
                "minimum-viewport .topbar must contain `{declaration}`"
            );
        }
        assert2::assert!(
            block_for_in(rule, ".topbar::-webkit-scrollbar").contains("display: none;"),
            "minimum-viewport topbar must hide the WebKit scrollbar"
        );

        let dropdown = block_for_in(rule, ".export-dropdown");
        for declaration in [
            "position: fixed;",
            "top: 52px;",
            "left: 8px;",
            "right: 8px;",
            "width: auto;",
            "min-width: 0;",
            "max-width: none;",
            "max-height: calc(100vh - 60px);",
        ] {
            assert2::assert!(
                dropdown.contains(declaration),
                "minimum-viewport .export-dropdown must contain `{declaration}`"
            );
        }

        let session_row = block_for_in(rule, ".session-row");
        for declaration in ["flex-direction: column;", "align-items: stretch;"] {
            assert2::assert!(
                session_row.contains(declaration),
                "minimum-viewport .session-row must contain `{declaration}`"
            );
        }
        for selector in [".session-row > .sess-switch", ".session-row > .ctl-btn"] {
            assert2::assert!(
                block_for_in(rule, selector).contains("width: 100%;"),
                "minimum-viewport {selector} must span the full row"
            );
        }
    }

    #[test]
    fn collapsed_activity_preview_is_bounded() {
        assert_block_contains(
            ".activity:not(.expanded) .activity-text",
            "display: -webkit-box",
        );
        assert_block_contains(
            ".activity:not(.expanded) .activity-text",
            "-webkit-line-clamp: 2",
        );
        assert_block_contains(
            ".activity:not(.expanded) .activity-text",
            "-webkit-box-orient: vertical",
        );
        assert_block_contains(
            ".activity:not(.expanded) .activity-text",
            "overflow: hidden",
        );
    }

    #[test]
    fn narrow_panel_clears_activity_overlay() {
        const CLEARANCE_RULE: &str =
            "@media (max-width: 600px) {\n  .panel {\n    padding-bottom: 104px;\n  }\n}";

        assert2::assert!(
            (CSS.matches(CLEARANCE_RULE).count()) == (1),
            "stylesheet must contain one exact narrow panel clearance rule"
        );
    }

    #[test]
    fn session_row_keeps_names_badges_and_actions_discoverable() {
        assert_block_contains(".session-row > .sess-switch", "width: auto");
        assert_block_contains(".session-row > .sess-switch", "flex: 1 1 0");
        assert_block_contains(".session-row > .sess-switch", "min-width: 0");
        assert_block_contains(".sess-name", "flex: 1 1 auto");
        assert_block_contains(".sess-name", "min-width: 0");
        assert_block_contains(".sess-name", "overflow-wrap: anywhere");
        assert_block_contains(".sess-badges", "flex: 0 0 auto");
        assert_block_contains(".session-row > .ctl-btn", "flex: 0 0 auto");
        assert_block_contains(".session-row > .ctl-btn", "width: auto");
        assert2::assert!(
            TOPBAR_SOURCE
                .contains(r#"span { class: "sess-name", title: "{info.name}", "{info.name}" }"#),
            "session name markup must expose the complete name as a native title"
        );
    }

    #[test]
    fn topbar_title_has_a_wrapping_hover_card() {
        assert2::assert!(
            TOPBAR_SOURCE.contains(r#"class: "topbar-title""#)
                && TOPBAR_SOURCE.contains(r#""data-full-title": "{title}""#),
            "topbar title markup must provide its complete text to the hover card"
        );
        assert2::assert!(
            TOPBAR_SOURCE.contains(r#"class: "topbar-title-text""#),
            "the clipped title text must be separate from the hover-card host"
        );
        assert_block_contains(".topbar-title", "position: relative");
        assert_block_contains(".topbar-title", "min-width: 0");
        assert_block_contains(".topbar-title-text", "overflow: hidden");
        assert_block_contains(".topbar-title-text", "text-overflow: ellipsis");
        assert_block_contains(".topbar-title::after", "content: attr(data-full-title)");
        assert_block_contains(
            ".topbar-title::after",
            "max-width: min(420px, calc(100vw - 32px))",
        );
        assert_block_contains(".topbar-title::after", "overflow-wrap: anywhere");
        assert_block_contains(".topbar-title:hover::after", "opacity: 1");
    }

    #[test]
    fn session_menu_discloses_management_and_confirms_delete() {
        for markup in [
            "let mut manage_open = use_signal(|| false);",
            "let mut delete_pending = use_signal(|| false);",
            "if !info.active {",
            "Manage session",
            "Rename current session",
            "Create new session",
            "Confirm delete",
            "Cancel",
        ] {
            assert2::assert!(TOPBAR_SOURCE.contains(markup), "missing `{markup}`");
        }
    }

    #[test]
    fn active_session_statuses_stay_in_the_session_menu_header() {
        assert2::assert!(
            TOPBAR_SOURCE.contains(
                r#"div { class: "session-doc-heading",
                                strong { "{title}" }
                                span { class: "sess-badges",
                                    if open > 0 {"#
            ),
            "session header must retain the active session's open-choice badge"
        );
        assert2::assert!(
            TOPBAR_SOURCE.contains(
                r#"if m.waiting_agents > 0 {
                                        span { class: "pill pill-waiting", "●" }"#
            ),
            "session header must retain the active session's waiting badge"
        );
        assert_block_contains(".session-doc-heading", "display: flex");
        assert_block_contains(".session-doc-title > span", "text-transform: uppercase");
    }

    #[test]
    fn session_management_forms_and_danger_zone_are_distinct() {
        assert_block_contains(".session-manage", "border-top: 1px solid var(--border)");
        assert_block_contains(".session-form", "display: grid");
        assert_block_contains(".session-form-row", "display: flex");
        assert_block_contains(".session-form-row .btn", "width: auto");
        assert_block_contains(".session-name-input", "border-radius: 7px");
        assert_block_contains(".session-name-input", "width: 100%");
        assert_block_contains(".session-danger", "border-top: 1px solid var(--border)");
        assert_block_contains(".session-delete", "color: var(--status-removed)");
        assert_block_contains(".delete-confirm", "background: var(--accent-soft)");
    }

    #[test]
    fn family_ids_are_unique_and_slug_safe() {
        let mut seen = std::collections::BTreeSet::new();
        for f in FAMILIES {
            assert2::assert!(seen.insert(f.id), "duplicate family id {}", f.id);
            assert2::assert!(
                f.id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "family id {} is not slug-safe",
                f.id
            );
            assert2::assert!(!f.name.is_empty());
        }
        assert2::assert!(seen.contains(DEFAULT_FAMILY), "default family missing");
    }

    #[parameterized(
        known = { "gruvbox", Some("Gruvbox") },
        unknown = { "no-such-theme", None },
    )]
    fn family_lookup_works(id: &str, expected: Option<&str>) {
        assert2::assert!(family(id).map(|family| family.name) == expected);
    }

    #[parameterized(
        auto = { Mode::Auto, "\"auto\"" },
        dark = { Mode::Dark, "\"dark\"" },
        light = { Mode::Light, "\"light\"" },
    )]
    fn mode_serde_round_trips_lowercase(mode: Mode, text: &str) {
        assert2::assert!(serde_json::to_string(&mode).unwrap() == text);
        assert2::assert!(serde_json::from_str::<Mode>(text).unwrap() == mode);
    }

    #[test]
    fn mode_defaults_to_auto() {
        assert2::assert!(Mode::default() == Mode::Auto);
    }

    #[parameterized(auto = { Mode::Auto }, dark = { Mode::Dark }, light = { Mode::Light })]
    fn mode_as_str_matches_serde(mode: Mode) {
        let serde_form = serde_json::to_string(&mode).unwrap();
        assert2::assert!(format!("\"{}\"", mode.as_str()) == serde_form);
    }

    #[test]
    fn root_fallback_defines_every_token() {
        let root = block_for(":root");
        for token in REQUIRED_TOKENS {
            assert2::assert!(
                root.contains(&format!("{token}:")),
                ":root fallback missing {token}"
            );
        }
        assert2::assert!(
            root.contains("--mini-viewport-fill:"),
            ":root missing the derived --mini-viewport-fill"
        );
    }

    #[test]
    fn every_family_has_a_block_defining_all_tokens() {
        for f in FAMILIES {
            let block = block_for(&format!("[data-theme=\"{}\"]", f.id));
            for token in REQUIRED_TOKENS {
                assert2::assert!(
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
                assert2::assert!(
                    decl.contains("light-dark("),
                    "family {} must define {token} via light-dark()",
                    f.id
                );
            }
        }
    }

    #[test]
    fn canonical_palette_anchors_remain_stable() {
        let anchors = [
            ("nodestorm", "--accent: light-dark(#3d6fe0, #6c9ef8);"),
            ("solarized", "--bg: light-dark(#fdf6e3, #002b36);"),
            ("gruvbox", "--bg: light-dark(#fbf1c7, #282828);"),
            ("catppuccin", "--bg: light-dark(#eff1f5, #1e1e2e);"),
            ("nord", "--bg: light-dark(#eceff4, #2e3440);"),
            ("dracula", "--accent: light-dark(#644ac9, #bd93f9);"),
            ("tokyo-night", "--bg: light-dark(#e1e2e7, #1a1b26);"),
            ("one", "--bg: light-dark(#fafafa, #282c34);"),
            ("github", "--bg: light-dark(#f6f8fa, #0d1117);"),
            ("everforest", "--bg: light-dark(#fdf6e3, #2d353b);"),
            ("rose-pine", "--bg: light-dark(#faf4ed, #191724);"),
            ("monokai", "--bg: light-dark(#fafaf4, #272822);"),
            (
                "solarized",
                "--bg-card-hover: light-dark(#eee8d5, #002b36);",
            ),
            ("nord", "--status-existing: light-dark(#4c566a, #d8dee9);"),
            ("nord", "--shadow: light-dark(#2e344059, #000000);"),
        ];

        for (id, declaration) in anchors {
            assert2::assert!(
                block_for(&format!("[data-theme=\"{id}\"]")).contains(declaration),
                "family {id} lost its canonical palette anchor"
            );
        }
    }

    #[test]
    fn no_orphan_theme_blocks_in_css() {
        for (i, _) in CSS.match_indices("[data-theme=\"") {
            let rest = &CSS[i + "[data-theme=\"".len()..];
            let id = rest.split('"').next().unwrap_or_default();
            assert2::assert!(
                family(id).is_some(),
                "CSS has a [data-theme=\"{id}\"] block with no registered family"
            );
        }
    }

    #[test]
    fn mode_rules_set_color_scheme() {
        assert2::assert!(block_for("[data-mode=\"dark\"]").contains("color-scheme: dark;"));
        assert2::assert!(block_for("[data-mode=\"light\"]").contains("color-scheme: light;"));
        assert2::assert!(block_for("[data-mode=\"auto\"]").contains("color-scheme: light dark;"));
    }

    #[test]
    fn family_blocks_are_supports_guarded() {
        let guard = CSS
            .find("@supports (color: light-dark(")
            .expect("no @supports light-dark() guard in CSS");
        for (i, _) in CSS.match_indices("[data-theme=\"") {
            assert2::assert!(
                i > guard,
                "a [data-theme] block sits outside the @supports guard"
            );
        }
    }
}
