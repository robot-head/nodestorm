//! Dioxus desktop UI.

mod activity;
mod app;
mod canvas;
mod choice_panel;
mod edge_layer;
mod node_card;
mod topbar;

use std::sync::Arc;

use dioxus::desktop::{Config, WindowBuilder};

use crate::cli::Cli;
use crate::store::Store;

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
pub fn launch(store: Arc<Store>, cli: Cli) {
    let window = WindowBuilder::new()
        .with_title("nodestorm")
        .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(1280.0, 840.0));
    dioxus::LaunchBuilder::new()
        .with_cfg(Config::new().with_window(window).with_menu(None))
        .with_context(cli)
        .with_context(store)
        .launch(app::App);
}
