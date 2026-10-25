//! How much disk a project's folder uses. Blocking walks: callers run them off the async threads.
use crate::agents::PROMPT_FILE;
use fleet_api::{Footprint, FootprintItem};
use std::path::{Path, PathBuf};

/// Regenerable build output, looked for at each worktree's root.
pub const BUILD_DIRS: [&str; 2] = ["target", "node_modules"];

/// The size of every regular file under `path`, in bytes of `len()`. Never follows a symlink (one pointing out of
/// the folder would count someone else's data, one pointing back would loop) and skips what it cannot read, so a
/// half-deleted tree still gets a number. 0 for a missing path.
pub fn dir_bytes(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(p) = pending.pop() {
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

/// Splits a project folder's weight into its worktrees (with their build output), the daemon's own files and
/// whatever else sits in the folder. A worktree that is gone has no item.
pub fn measure(
    workdir: &Path,
    worktrees: &[(String, PathBuf)],
    daemon_files: &Path,
    now: i64,
) -> Footprint {
    let mut items = Vec::new();
    for (repo, path) in worktrees {
        if std::fs::symlink_metadata(path).is_err() {
            continue;
        }
        let builds: Vec<u64> = BUILD_DIRS
            .iter()
            .map(|d| path.join(d))
            .filter(|d| d.is_dir())
            .map(|d| dir_bytes(&d))
            .collect();
        items.push(FootprintItem {
            label: format!("{repo} worktree"),
            path: s(path),
            bytes: dir_bytes(path),
            build_bytes: (!builds.is_empty()).then(|| builds.iter().sum()),
        });
    }

    let mut daemon = dir_bytes(daemon_files);
    let mut other = 0;
    if let Ok(entries) = std::fs::read_dir(workdir) {
        for e in entries.flatten() {
            let p = e.path();
            if worktrees.iter().any(|(_, w)| *w == p) {
                continue;
            }
            let name = e.file_name();
            if name == "CLAUDE.md" || name == PROMPT_FILE {
                daemon += dir_bytes(&p);
            } else {
                other += dir_bytes(&p);
            }
        }
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
