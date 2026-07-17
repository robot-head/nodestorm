//! Dioxus desktop UI.

mod activity;
mod app;
mod canvas;
mod choice_panel;
mod cluster_card;
mod context_menu;
mod diff_panel;
mod edge_layer;
mod minimap;
mod node_card;
mod theme_menu;
mod timeline;
mod topbar;

/// Nominal viewport for zoom math (the topbar is 48px tall).
pub(crate) const VIEW_W: f64 = 1280.0;
pub(crate) const VIEW_H: f64 = 780.0;
pub(crate) const TOPBAR_H: f64 = 48.0;
/// Zoom-to-fit never goes below this: past ~100 nodes a full fit would make
/// cards invisible, so show a readable centered subset instead (culling
/// keeps the off-screen rest out of the DOM).
pub(crate) const MIN_FIT_SCALE: f64 = 0.15;

use std::sync::Arc;

use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

use crate::cli::Cli;
use crate::model::{Node, NodeId};

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
        .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(1280.0, 840.0))
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
}
