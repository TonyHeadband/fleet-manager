//! The window icon, dark or light to match the active theme half.
use eframe::egui;
use std::sync::Arc;

// Rendered from `packaging/fleet-manager.svg` at 256 px; re-render them when the SVG changes.
const DARK: &[u8] = include_bytes!("../assets/icon.png");
const LIGHT: &[u8] = include_bytes!("../assets/icon-light.png");

pub fn png(dark: bool) -> &'static [u8] {
    if dark {
        DARK
    } else {
        LIGHT
    }
}

/// Remembers which icon the window has, so a viewport command goes out on a change and not on every frame.
#[derive(Default)]
pub struct IconSync {
    last: Option<bool>,
}

impl IconSync {
    /// Returns true when it sent a new icon.
    pub fn update(&mut self, ctx: &egui::Context, dark: bool) -> bool {
        if self.last == Some(dark) {
            return false;
        }
        let Ok(data) = eframe::icon_data::from_png_bytes(png(dark)) else {
            // Bundled assets, so unreachable in practice; do not retry a broken PNG every frame.
            self.last = Some(dark);
            return false;
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(data))));
        self.last = Some(dark);
        true
    }
}
