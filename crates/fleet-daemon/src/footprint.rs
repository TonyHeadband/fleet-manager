//! How much disk a task's folder uses. Blocking walks: callers run them off the async threads.
use crate::agents::PROMPT_FILE;
use crate::fleet::ops::{INTERNAL_DIR, SNAPSHOTS_DIR};
use fleet_api::{Footprint, FootprintItem};
use std::path::{Path, PathBuf};

/// Regenerable build output, looked for at each worktree's root.
pub const BUILD_DIRS: [&str; 2] = ["target", "node_modules"];

/// The size of every regular file under `path`, in bytes of `len()`. Never follows a symlink (one pointing out of
/// the folder would count someone else's data, one pointing back would loop) and skips what it cannot read, so a
/// half-deleted tree still gets a number. 0 for a missing path.
pub fn dir_bytes(path: &Path) -> u64 {
    dir_bytes_except(path, &[])
}

/// `dir_bytes` without anything at or under one of `skip`.
pub fn dir_bytes_except(path: &Path, skip: &[&Path]) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(p) = pending.pop() {
        if skip.iter().any(|s| p.starts_with(s)) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                pending.extend(entries.flatten().map(|e| e.path()));
            }
        }
    }
    total
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// One worktree's weight with its build output, labelled `<repo> worktree`; None when it is gone.
pub fn worktree_item(repo: &str, path: &Path) -> Option<FootprintItem> {
    std::fs::symlink_metadata(path).ok()?;
    let builds: Vec<u64> = BUILD_DIRS
        .iter()
        .map(|d| path.join(d))
        // A link is not a build dir: `dir_bytes` would not follow it either.
        .filter(|d| std::fs::symlink_metadata(d).is_ok_and(|m| m.is_dir()))
        .map(|d| dir_bytes(&d))
        .collect();
    Some(FootprintItem {
        label: format!("{repo} worktree"),
        path: s(path),
        bytes: dir_bytes(path),
        build_bytes: (!builds.is_empty()).then(|| builds.iter().sum()),
    })
}

/// A task's shared build folder, labelled `build output`, when it is a real folder that is inside `task` once every
/// link is resolved; a link, or a folder reached through one, would weigh someone else's data.
pub fn build_item(task: &Path, path: &Path) -> Option<FootprintItem> {
    let inside = match (fleet_fs::canonical(path), fleet_fs::canonical(task)) {
        (Ok(p), Ok(t)) => p != t && p.starts_with(&t),
        _ => false,
    };
    if !inside || !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        return None;
    }
    let bytes = dir_bytes(path);
    Some(FootprintItem {
        label: "build output".into(),
        path: s(path),
        bytes,
        build_bytes: Some(bytes),
    })
}

/// Splits a task folder's weight into its worktrees (with their build output), the task's shared build folders
/// (`build`), the daemon's own files and whatever else sits in the folder. A worktree that is gone has no item.
pub fn measure(
    workdir: &Path,
    worktrees: &[(String, PathBuf)],
    daemon_files: &Path,
    build: &[PathBuf],
    now: i64,
) -> Footprint {
    let mut items: Vec<FootprintItem> = worktrees
        .iter()
        .filter_map(|(repo, path)| worktree_item(repo, path))
        .collect();
    let mut weighed: Vec<&Path> = worktrees.iter().map(|(_, w)| w.as_path()).collect();
    for b in build {
        if let Some(item) = build_item(workdir, b) {
            items.push(item);
            weighed.push(b);
        }
    }

    let mut daemon = dir_bytes(daemon_files);
    let mut other = 0;
    // The task's notes checkout and its specialists' snapshots weigh in as one item, whatever is in them.
    let mut internal = 0;
    if let Ok(entries) = std::fs::read_dir(workdir) {
        for e in entries.flatten() {
            let p = e.path();
            // A worktree or a build folder is weighed in its own item wherever it sits, so it is left out of
            // whatever holds it.
            if weighed.iter().any(|w| p.starts_with(w)) {
                continue;
            }
            let name = e.file_name();
            if name == "CLAUDE.md" || name == PROMPT_FILE {
                daemon += dir_bytes(&p);
            } else if name == INTERNAL_DIR || name == SNAPSHOTS_DIR {
                internal += dir_bytes(&p);
            } else {
                other += dir_bytes_except(&p, &weighed);
            }
        }
    }
    if internal > 0 {
        items.push(FootprintItem {
            label: "internal".into(),
            path: s(&workdir.join(INTERNAL_DIR)),
            bytes: internal,
            build_bytes: None,
        });
    }
    items.push(FootprintItem {
        label: "daemon files".into(),
        path: s(daemon_files),
        bytes: daemon,
        build_bytes: None,
    });
    if other > 0 {
        items.push(FootprintItem {
            label: "other files".into(),
            path: s(workdir),
            bytes: other,
            build_bytes: None,
        });
    }
    let total = items.iter().map(|i| i.bytes).sum();
    Footprint {
        measured_at: now,
        items,
        total,
    }
}
