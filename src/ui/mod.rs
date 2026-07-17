//! Dioxus desktop UI.

mod activity;
mod app;
mod canvas;
mod choice_panel;
mod cluster_card;
mod context_menu;
mod edge_layer;
mod minimap;
mod node_card;
mod timeline;
mod topbar;

/// Nominal viewport for zoom math (the topbar is 48px tall).
pub(crate) const VIEW_W: f64 = 1280.0;
pub(crate) const VIEW_H: f64 = 780.0;
pub(crate) const TOPBAR_H: f64 = 48.0;

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
    /// Fit `bounds` into a viewport of the given size (never zooming past 1:1).
    pub fn fit(bounds: &crate::layout::Rect, view_w: f64, view_h: f64) -> Self {
        if bounds.w <= 0.0 || bounds.h <= 0.0 {
            return Self::default();
        }
        let scale = (view_w / bounds.w).min(view_h / bounds.h).min(1.0);
        Self {
            tx: -bounds.x * scale + (view_w - bounds.w * scale) / 2.0,
            ty: -bounds.y * scale + (view_h - bounds.h * scale) / 2.0,
            scale,
        }
    }
}

/// Launch the desktop window. Must be called on the main thread.
pub fn launch(sessions: Arc<crate::sessions::Sessions>, cli: Cli) {
    let window = WindowBuilder::new()
        .with_title("nodestorm")
        .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(1280.0, 840.0));
    dioxus::LaunchBuilder::new()
        .with_cfg(Config::new().with_window(window).with_menu(None))
        .with_context(cli)
        .with_context(sessions)
        .launch(app::App);
}
