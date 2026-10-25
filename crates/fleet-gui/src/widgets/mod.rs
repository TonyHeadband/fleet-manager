//! The shared pieces every component is built from; the only place a button or a badge is drawn.
mod badge;
mod button;
mod field;
mod icon;
mod path;
mod row;
mod tabs;

pub use badge::{badge, badge_width, BadgeTone};
pub use button::{button, chip, toggle, ButtonVariant};
pub use field::{badge_items, field};
pub use icon::{dot, icon_button, text_dot, Icon};
pub use path::{elide_middle, path_edit, path_label, prefixed_path_label};
pub use row::{list_row, nav_item};
pub use tabs::{tab_bar, tabs};

/// A size in SI units with one decimal ("7.6 GB", "12.0 KB"); plain bytes below a kilobyte. SI, not binary, so the
/// figure matches what `df -H` and file managers print.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
