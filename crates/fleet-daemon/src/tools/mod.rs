//! The tools the daemon needs on its machine: what they are, how to find them, how to fix them.

pub mod path;
pub mod pkg;
pub mod table;

pub use path::{merged_path, PathSource, SystemPath};
pub use pkg::{detect_manager, Manager};
pub use table::{at_least, fix, install_command, parse_version, table, ToolSpec};
