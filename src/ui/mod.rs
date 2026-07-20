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

const APP_ICON_PNG: &[u8] = include_bytes!("../../assets/icons/nodestorm-256.png");

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

/// Store identity paired with the snapshots currently rendered by [`app::App`].
#[derive(Clone, Copy)]
pub(crate) struct ActiveStore(pub Signal<Arc<Store>>);

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

fn app_icon() -> dioxus::desktop::tao::window::Icon {
    let image = image::load_from_memory_with_format(APP_ICON_PNG, image::ImageFormat::Png)
        .expect("embedded app icon must be a valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    dioxus::desktop::tao::window::Icon::from_rgba(image.into_raw(), width, height)
        .expect("embedded app icon must have valid RGBA dimensions")
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
        .with_window_icon(Some(app_icon()))
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
    use yare::parameterized;

    #[test]
    fn embedded_app_icon_is_a_256px_rgba_png() {
        let image = image::load_from_memory_with_format(APP_ICON_PNG, image::ImageFormat::Png)
            .expect("embedded app icon must be a valid PNG");
        assert2::assert!((image.width(), image.height()) == (256, 256));
        assert2::assert!((image.color()) == (image::ColorType::Rgba8));
    }

    #[test]
    fn embedded_app_icon_builds_a_tao_icon() {
        let _icon = app_icon();
    }

    #[parameterized(
        auto = { crate::theme::Mode::Auto, None },
        dark = { crate::theme::Mode::Dark, Some(dioxus::desktop::tao::window::Theme::Dark) },
        light = { crate::theme::Mode::Light, Some(dioxus::desktop::tao::window::Theme::Light) },
    )]
    fn tao_theme_maps_modes(
        mode: crate::theme::Mode,
        expected: Option<dioxus::desktop::tao::window::Theme>,
    ) {
        assert2::assert!(tao_theme(mode) == expected);
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
        assert2::assert!(t.scale >= MIN_FIT_SCALE, "scale: {}", t.scale);
        let center_x = (1280.0 / 2.0 - t.tx) / t.scale;
        let center_y = (780.0 / 2.0 - t.ty) / t.scale;
        assert2::assert!((center_x - 50_000.0).abs() < 1.0, "center x: {center_x}");
        assert2::assert!((center_y - 30_000.0).abs() < 1.0, "center y: {center_y}");

        // Small graphs keep the old behavior (never zoom past 1:1).
        let small = crate::layout::Rect {
            x: 0.0,
            y: 0.0,
            w: 500.0,
            h: 300.0,
        };
        assert2::assert!((ViewTransform::fit(&small, 1280.0, 780.0).scale) == (1.0));
    }

    #[test]
    fn fit_uses_the_limiting_axis_and_centers_exactly() {
        let bounds = crate::layout::Rect {
            x: 10.0,
            y: 20.0,
            w: 400.0,
            h: 200.0,
        };
        assert2::assert!(
            (ViewTransform::fit(&bounds, 200.0, 300.0))
                == (ViewTransform {
                    tx: -5.0,
                    ty: 90.0,
                    scale: 0.5,
                })
        );
        assert2::assert!(
            (ViewTransform::fit(
                &crate::layout::Rect {
                    w: 100.0,
                    h: 400.0,
                    ..bounds
                },
                300.0,
                200.0,
            )) == (ViewTransform {
                tx: 120.0,
                ty: -10.0,
                scale: 0.5,
            })
        );
        assert2::assert!(
            (ViewTransform::fit(
                &crate::layout::Rect {
                    w: 0.0,
                    h: 10.0,
                    ..bounds
                },
                200.0,
                300.0,
            )) == (ViewTransform::default())
        );
        assert2::assert!(
            (ViewTransform::fit(
                &crate::layout::Rect {
                    w: 10.0,
                    h: 0.0,
                    ..bounds
                },
                200.0,
                300.0,
            )) == (ViewTransform::default())
        );
    }

    #[test]
    fn agent_colors_and_node_search_are_deterministic() {
        assert2::assert!((agent_hue("alice")) == (239));
        assert2::assert!((agent_hue("bob")) == (284));
        assert2::assert!((agent_hue("")) == (61));
        assert2::assert!((agent_color("alice")) == ("hsl(239, 62%, 55%)"));

        let mut node = crate::demo::demo_doc().nodes.remove(0);
        node.label = "Alpha Service".into();
        node.group = Some("Platform".into());
        assert2::assert!(node_matches(&node, "ALPHA"));
        assert2::assert!(node_matches(&node, "web-ui"));
        assert2::assert!(node_matches(&node, "platform"));
        assert2::assert!(!node_matches(&node, "missing"));
        assert2::assert!(!node_matches(&node, "   "));
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
        assert2::assert!((plane_center) == (625.0, 562.5));

        transform.reframe(old, new);

        assert2::assert!((transform.plane_center(new)) == (plane_center));
    }

    #[parameterized(
        zero_width = { 0.0, 780.0 },
        zero_height = { 520.0, 0.0 },
        negative_height = { 520.0, -1.0 },
        non_finite_width = { f64::NAN, 780.0 },
    )]
    fn viewport_size_rejects_invalid_observations(width: f64, height: f64) {
        assert2::assert!(ViewportSize::observed(width, height).is_none());
    }
}
