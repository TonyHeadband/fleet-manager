//! Which program runs for a `claude` found on PATH. On Windows an npm install puts a `.cmd`/`.ps1` shim on PATH; a
//! pseudo-console needs the real `claude.exe`, which npm keeps beside the shim.
use std::path::{Path, PathBuf};

const SHIM_TARGET: &str = "node_modules/@anthropic-ai/claude-code/bin/claude.exe";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("found the claude npm shim at {} but not claude.exe next to it", .0.display())]
    ShimWithoutExe(PathBuf),
}

/// `found` itself, unless it is an npm shim: then the exe npm installed beside it.
pub fn shim_target(found: &Path) -> Result<PathBuf, ResolveError> {
    let is_shim = found
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("ps1"));
    if !is_shim {
        return Ok(found.to_path_buf());
    }
    let dir = found.parent().unwrap_or(Path::new("."));
    let exe = SHIM_TARGET
        .split('/')
        .fold(dir.to_path_buf(), |p, part| p.join(part));
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(ResolveError::ShimWithoutExe(found.to_path_buf()))
    }
}
