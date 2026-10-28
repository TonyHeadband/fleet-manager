//! The tools the daemon needs on its machine: what they are, how to find them, how to fix them.

pub mod pkg;
pub mod table;

pub use pkg::{detect_manager, Manager};
pub use table::{at_least, fix, install_command, parse_version, table, ToolSpec};
