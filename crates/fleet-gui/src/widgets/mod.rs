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

/// Shared with the daemon, which writes sizes into project logs.
pub use fleet_api::format_bytes;
