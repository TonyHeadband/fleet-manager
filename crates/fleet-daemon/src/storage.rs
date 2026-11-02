//! Fleet Manager's live disk use by kind (`StorageSummary`). Blocking walks: callers run them off the async threads.
//!
//! The task kinds come from each live task's last footprint (`footprint::measure`, kept fresh by the measurer in
//! `agents.rs`), so a request never walks a worktree. The state folder is walked here, because nothing else weighs
//! `transcripts/` or the daemon's own files.
use crate::chat::kept;
use crate::footprint::{dir_bytes, dir_bytes_except};
use fleet_api::{StorageSummary, Task, TaskState};
use std::path::Path;

/// The footprint item labels `footprint::measure` and `archive` give to what is not a worktree: the task's
/// daemon files sit in the state folder, which `state_kinds` weighs once for all tasks.
const DAEMON_FILES: &str = "daemon files";
const BUILD_OUTPUT: &str = "build output";

/// What the state folder holds, by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateKinds {
    pub repos: u64,
    pub transcripts: u64,
    pub daemon: u64,
}

/// Bytes of live tasks' worktrees (with the rest of their folders) and of their build output, from their last
/// footprints. An archived task counts for nothing, and neither does one with no footprint yet.
pub fn task_kinds(tasks: &[Task]) -> (u64, u64) {
    let (mut worktrees, mut build) = (0u64, 0u64);
    for fp in tasks
        .iter()
        .filter(|t| t.state != TaskState::Archived)
        .filter_map(|t| t.footprint.as_ref())
    {
        for i in &fp.items {
            if i.label == DAEMON_FILES {
                continue;
            }
            // A shared build folder is all build output; a worktree's `build_bytes` is the part of `bytes`.
            let b = if i.label == BUILD_OUTPUT {
                i.bytes
            } else {
                i.build_bytes.unwrap_or(0).min(i.bytes)
            };
            build += b;
            worktrees += i.bytes - b;
        }
    }
    (worktrees, build)
}

/// Weighs the state folder: `repos/`, the kept chats in `transcripts/`, and the daemon's files, which are all the
/// rest: everything under `state_dir` except those two folders and `tasks_root` (should it lie inside).
pub fn state_kinds(state_dir: &Path, tasks_root: &Path) -> StateKinds {
    let repos = state_dir.join("repos");
    let transcripts = state_dir.join(kept::DIR);
    StateKinds {
        repos: dir_bytes(&repos),
        transcripts: dir_bytes(&transcripts),
        daemon: dir_bytes_except(state_dir, &[&repos, &transcripts, tasks_root]),
    }
}

/// `(free, total)` bytes of the volume holding `path`; free is what is available to this user. None when the
/// system will not say.
#[cfg(unix)]
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path and `st` a valid out-pointer for the call.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let unit = st.f_frsize as u64;
    Some((st.f_bavail as u64 * unit, st.f_blocks as u64 * unit))
}

#[cfg(windows)]
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: `wide` is NUL-terminated and the out-pointers are valid; the total-free pointer may be null.
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some((free, total))
}

/// The summary: `tasks` from the store, `state` from `state_kinds`, the space from the volume of `tasks_root`
/// (or the nearest parent that exists, as the folder is made on first use).
pub fn summarize(tasks: &[Task], state: StateKinds, tasks_root: &Path) -> StorageSummary {
    let (worktrees_bytes, build_bytes) = task_kinds(tasks);
    let (disk_free_bytes, disk_total_bytes) = tasks_root
        .ancestors()
        .find(|p| p.exists())
        .and_then(disk_space)
        .unwrap_or((0, 0));
    StorageSummary {
        worktrees_bytes,
        build_bytes,
        repos_bytes: state.repos,
        transcripts_bytes: state.transcripts,
        daemon_bytes: state.daemon,
        disk_free_bytes,
        disk_total_bytes,
    }
}
