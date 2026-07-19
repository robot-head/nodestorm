//! Dioxus desktop UI.

mod activity;
mod agent_launcher;
mod annotation_layer;
mod app;
mod canvas;
mod choice_panel;
mod cluster_card;
mod context_menu;
mod diff_panel;
mod edge_layer;
mod minimap;
mod more_menu;
mod node_card;
mod questions_panel;
mod queued_changes;
mod theme_menu;
mod timeline;
mod topbar;

pub(crate) const TOPBAR_H: f64 = 48.0;
/// Zoom-to-fit never goes below this: past ~100 nodes a full fit would make
/// cards invisible, so show a readable centered subset instead (culling
/// keeps the off-screen rest out of the DOM).
pub(crate) const MIN_FIT_SCALE: f64 = 0.15;

/// Live canvas content-box geometry. The fallback is used only until the
/// viewport's first ResizeObserver measurement arrives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ViewportSize {
    pub width: f64,
    pub height: f64,
}

impl ViewportSize {
    pub const FALLBACK: Self = Self {
        width: 1280.0,
        height: 780.0,
    };

    pub fn observed(width: f64, height: f64) -> Option<Self> {
        (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
            .then_some(Self { width, height })
    }
}

use std::sync::Arc;

use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

use crate::cli::Cli;
use crate::model::{Node, NodeId};
use crate::store::Store;

// Context wrappers: distinct types so same-shaped signals can coexist in
// Dioxus's type-keyed context.

/// Connect mode: `Some(source)` while the user is picking a target card.
#[derive(Clone, Copy)]
pub(crate) struct ConnectFrom(pub Signal<Option<NodeId>>);

/// One-shot request for the canvas to center+zoom on a node (search hits).
#[derive(Clone, Copy)]
pub(crate) struct ZoomTarget(pub Signal<Option<NodeId>>);

/// Live search query from the topbar box.
#[derive(Clone, Copy)]
pub(crate) struct SearchQuery(pub Signal<String>);

/// `Some(other)` while the diff panel compares the active session with
/// `other`.
#[derive(Clone, Copy)]
pub(crate) struct CompareWith(pub Signal<Option<String>>);

/// `Some(markdown)` while the diff panel shows a session-vs-exported-record
/// comparison (the text is the rendered diff, or an error message).
#[derive(Clone, Copy)]
pub(crate) struct RecordDiff(pub Signal<Option<String>>);

/// The top-bar message draft and popover state, shared with queued-change
/// editing so a removed comment can be revised before sending again.
#[derive(Clone, Copy)]
pub(crate) struct MessageComposer {
    pub comment: Signal<String>,
    pub open: Signal<bool>,
}

/// Whether the start-agent modal is open.
#[derive(Clone, Copy)]
pub(crate) struct AgentLauncherOpen(pub Signal<bool>);

/// The live theme/mode preference (loaded in [`launch`], edited by the
/// topbar's theme menu, persisted to the global preferences file).
#[derive(Clone, Copy)]
pub(crate) struct ThemePref(pub Signal<crate::prefs::Preferences>);

/// Map the color mode onto the native window chrome: explicit modes force
/// the title bar, `Auto` (tao's `None`) follows the OS.
pub(crate) fn tao_theme(mode: crate::theme::Mode) -> Option<dioxus::desktop::tao::window::Theme> {
    use dioxus::desktop::tao::window::Theme;
    match mode {
        crate::theme::Mode::Auto => None,
        crate::theme::Mode::Dark => Some(Theme::Dark),
        crate::theme::Mode::Light => Some(Theme::Light),
    }
}

/// Deterministic hue (0–359) for an agent id, so each agent gets a stable
/// color/badge across the canvas and feed in a multi-agent session.
pub(crate) fn agent_hue(name: &str) -> u32 {
    // FNV-1a over the bytes, folded into a hue.
    let mut h: u32 = 2_166_136_261;
    for b in name.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(16_777_619);
    }
    h % 360
}

/// `hsl(...)` accent color for an agent id.
pub(crate) fn agent_color(name: &str) -> String {
    format!("hsl({}, 62%, 55%)", agent_hue(name))
}

/// Case-insensitive substring match over label, id, and group.
pub(crate) fn node_matches(node: &Node, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return false;
    }
    node.label.to_lowercase().contains(&q)
        || node.id.as_str().to_lowercase().contains(&q)
        || node
            .group
            .as_deref()
            .is_some_and(|g| g.to_lowercase().contains(&q))
}

/// Clipboard write via the WebView's `navigator.clipboard` (no native
/// clipboard dependency); the receipt lands in the activity feed.
pub(crate) fn copy_to_clipboard(store: &Arc<Store>, text: String, receipt: &str) {
    match serde_json::to_string(&text) {
        Ok(js) => {
            document::eval(&format!("navigator.clipboard.writeText({js});"));
            store.record_user_action(receipt.to_owned());
        }
        Err(err) => tracing::warn!(%err, "clipboard serialization failed"),
    }
}

/// Shared view-transform for the pan/zoom plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub tx: f64,
    pub ty: f64,
    pub scale: f64,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        }
    }
}

impl ViewTransform {
    /// Fit `bounds` into a viewport of the given size — never zooming past
    /// 1:1, and never below [`MIN_FIT_SCALE`] (huge graphs get a readable,
    /// centered subset instead of microscopic cards).
    pub fn fit(bounds: &crate::layout::Rect, view_w: f64, view_h: f64) -> Self {
        if bounds.w <= 0.0 || bounds.h <= 0.0 {
            return Self::default();
        }
        let scale = (view_w / bounds.w)
            .min(view_h / bounds.h)
            .clamp(MIN_FIT_SCALE, 1.0);
        Self {
            tx: -bounds.x * scale + (view_w - bounds.w * scale) / 2.0,
            ty: -bounds.y * scale + (view_h - bounds.h * scale) / 2.0,
            scale,
        }
    }

    pub(crate) fn plane_center(self, viewport: ViewportSize) -> (f64, f64) {
        (
            (viewport.width / 2.0 - self.tx) / self.scale,
            (viewport.height / 2.0 - self.ty) / self.scale,
        )
    }

    pub(crate) fn reframe(&mut self, old: ViewportSize, new: ViewportSize) {
        let (plane_x, plane_y) = self.plane_center(old);
        self.tx = new.width / 2.0 - plane_x * self.scale;
        self.ty = new.height / 2.0 - plane_y * self.scale;
    }
}

/// Launch the desktop window. Must be called on the main thread.
pub fn launch(sessions: Arc<crate::sessions::Sessions>, cli: Cli) {
    // Load preferences before the window exists so the native title bar is
    // right from first paint; the App seeds its signal from this context.
    let prefs = cli
        .prefs_path()
        .map(|p| crate::prefs::load_or_default(&p))
        .unwrap_or_default();
    let window = WindowBuilder::new()
        .with_title("nodestorm")
        .with_inner_size({
            let (w, h) = cli.window_size.unwrap_or((1280.0, 840.0));
            dioxus::desktop::tao::dpi::LogicalSize::new(w, h)
        })
        .with_theme(tao_theme(prefs.mode));
    dioxus::LaunchBuilder::new()
        .with_cfg(Config::new().with_window(window).with_menu(None))
        .with_context(cli)
        .with_context(sessions)
        .with_context(prefs)
        .launch(app::App);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tao_theme_maps_modes() {
        use dioxus::desktop::tao::window::Theme;
        assert_eq!(tao_theme(crate::theme::Mode::Auto), None);
        assert_eq!(tao_theme(crate::theme::Mode::Dark), Some(Theme::Dark));
        assert_eq!(tao_theme(crate::theme::Mode::Light), Some(Theme::Light));
    }

    #[test]
    fn fit_never_zooms_below_floor() {
        // A 300-node graph must not fit down to invisible cards: clamp at
        // the floor and center on the bounds' center instead.
        let huge = crate::layout::Rect {
            x: 0.0,
            y: 0.0,
            w: 100_000.0,
            h: 60_000.0,
        };
        let t = ViewTransform::fit(&huge, 1280.0, 780.0);
        assert!(t.scale >= MIN_FIT_SCALE, "scale: {}", t.scale);
        let center_x = (1280.0 / 2.0 - t.tx) / t.scale;
        let center_y = (780.0 / 2.0 - t.ty) / t.scale;
        assert!((center_x - 50_000.0).abs() < 1.0, "center x: {center_x}");
        assert!((center_y - 30_000.0).abs() < 1.0, "center y: {center_y}");

        // Small graphs keep the old behavior (never zoom past 1:1).
        let small = crate::layout::Rect {
            x: 0.0,
            y: 0.0,
            w: 500.0,
            h: 300.0,
        };
        assert_eq!(ViewTransform::fit(&small, 1280.0, 780.0).scale, 1.0);
    }

    #[test]
    fn viewport_reframe_preserves_plane_center() {
        let old = ViewportSize::FALLBACK;
        let new = ViewportSize::observed(520.0, 792.0).expect("positive viewport");
        let mut transform = ViewTransform {
            tx: 140.0,
            ty: -60.0,
            scale: 0.8,
        };
        let plane_center = transform.plane_center(old);

        transform.reframe(old, new);

        assert_eq!(transform.plane_center(new), plane_center);
    }

    #[test]
    fn viewport_size_rejects_invalid_observations() {
        assert!(ViewportSize::observed(0.0, 780.0).is_none());
        assert!(ViewportSize::observed(520.0, -1.0).is_none());
        assert!(ViewportSize::observed(f64::NAN, 780.0).is_none());
    }
}
