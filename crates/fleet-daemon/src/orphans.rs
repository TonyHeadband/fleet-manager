//! Finds what no project record owns: project folders, worktrees, daemon agent folders. Finds only; archiving an
//! orphan is a separate, confirmed step. Blocking: callers run `scan` off the async threads.
use fleet_api::{Orphan, OrphanKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing::warn;

fn orphan(kind: OrphanKind, path: &Path, worktrees: Vec<String>) -> Orphan {
    let path = path.to_string_lossy().into_owned();
    let tag = match kind {
        OrphanKind::ProjectFolder => "project_folder",
        OrphanKind::AgentFolder => "agent_folder",
        OrphanKind::Worktree => "worktree",
        OrphanKind::Session => "session",
    };
    Orphan {
        id: format!("{tag}:{path}"),
        kind,
        path,
        worktrees,
        bytes: None,
    }
}

/// The linked worktrees `base` has registered, whether or not their folders still exist. The first entry of
/// `git worktree list` is the checkout itself, which is not a worktree of anything.
fn registered_worktrees(base: &Path) -> Vec<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(base)
        .args(["worktree", "list", "--porcelain"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output();
    let out = match out {
        Ok(o) if o.status.success() => o,
        // A remembered repo that was moved or deleted has no worktrees to report; it must not stop the scan.
        Ok(o) => {
            warn!(base = %base.display(), stderr = %String::from_utf8_lossy(&o.stderr).trim(), "cannot list worktrees");
            return Vec::new();
        }
        Err(e) => {
            warn!(base = %base.display(), error = %e, "cannot run git to list worktrees");
            return Vec::new();
        }
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .skip(1)
        .map(PathBuf::from)
        .collect()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        // Not following links: a symlink under the root is not a folder the daemon made.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

/// Folders directly under `projects_root` that are not in `known_workdirs` (with the worktrees registered inside
/// them), agent folders under `agents_dir` whose name is not in `known_agent_ids`, and worktrees registered in one
/// of `bases` under `projects_root` that sit in neither an orphan nor a known folder. `known_workdirs` holds every
/// project record's folder, archived ones included, so a folder an archive kept is not reported again. Only
/// `projects_root` is scanned: a project under an older root is known by its record, not by where it lies.
pub fn scan(
    projects_root: &Path,
    agents_dir: &Path,
    bases: &[PathBuf],
    known_workdirs: &HashSet<PathBuf>,
    known_agent_ids: &HashSet<String>,
) -> Vec<Orphan> {
    // git reports real paths; the root may be reached through a link.
    let real_root =
        std::fs::canonicalize(projects_root).unwrap_or_else(|_| projects_root.to_path_buf());
    let mut worktrees: Vec<PathBuf> = Vec::new();
    for base in bases {
        for wt in registered_worktrees(base) {
            let under_root = wt
                .strip_prefix(&real_root)
                .or_else(|_| wt.strip_prefix(projects_root))
                .map(|rel| projects_root.join(rel));
            if let Ok(p) = under_root {
                if !worktrees.contains(&p) {
                    worktrees.push(p);
                }
            }
        }
    }

    let mut found = Vec::new();
    let mut claimed: Vec<PathBuf> = known_workdirs.iter().cloned().collect();
    for folder in subdirs(projects_root) {
        if known_workdirs.contains(&folder) {
            continue;
        }
        let inside = worktrees
            .iter()
            .filter(|w| w.starts_with(&folder))
            .map(|w| w.to_string_lossy().into_owned())
            .collect();
        found.push(orphan(OrphanKind::ProjectFolder, &folder, inside));
        claimed.push(folder);
    }
    for dir in subdirs(agents_dir) {
        let known = dir
            .file_name()
            .is_some_and(|n| known_agent_ids.contains(n.to_string_lossy().as_ref()));
        if !known {
            found.push(orphan(OrphanKind::AgentFolder, &dir, Vec::new()));
        }
    }
    for wt in worktrees {
        if !claimed.iter().any(|c| wt.starts_with(c)) {
            found.push(orphan(OrphanKind::Worktree, &wt, Vec::new()));
        }
    }
    found
}

/// The live sessions with the daemon's prefix that none of `owned` names, as orphans.
pub fn orphan_sessions(live: Vec<String>, owned: &HashSet<String>) -> Vec<Orphan> {
    live.into_iter()
        .filter(|s| !owned.contains(s))
        .map(|s| orphan(OrphanKind::Session, Path::new(&s), Vec::new()))
        .collect()
}
