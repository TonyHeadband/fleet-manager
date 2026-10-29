//! The programs this process starts: the PATH they get, and how a background one is started.
//!
//! The PATH is the daemon's: the daemon computes the user's real PATH on every tool check and sets it here, and the
//! GUI sets the one the daemon reports. Until one is set, children inherit this process's own.
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Command;
use std::sync::RwLock;

static PATH: RwLock<Option<OsString>> = RwLock::new(None);

/// The PATH children get, when one has been set.
pub fn path() -> Option<OsString> {
    PATH.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Sets the PATH every child started from now on gets.
pub fn set_path(path: OsString) {
    *PATH.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// Gives `cmd` the PATH set here, if any.
pub fn apply_path(cmd: &mut Command) {
    if let Some(path) = path() {
        cmd.env("PATH", path);
    }
}

/// Where `program` is on the children's PATH (this process's own until one is set).
pub fn which(program: impl AsRef<OsStr>) -> Option<PathBuf> {
    match path() {
        Some(path) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            which::which_in(program, Some(path), cwd).ok()
        }
        None => which::which(program).ok(),
    }
}

/// Windows: the child gets no console window. A GUI process has no console, so a console program it starts would
/// otherwise get a window of its own, flashing up for as long as it runs.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A program run in the background (git, gh): no window on Windows, the children's PATH. Terminals the person must
/// see (an install, a login) are not started this way.
pub fn background(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    apply_path(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
