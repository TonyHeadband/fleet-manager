//! Read-only filesystem and git queries behind the folder picker. Sync: callers on an async runtime wrap them in
//! `spawn_blocking`. Nothing here writes to a repo. `migrate` is the one writer: it moves the app's own folders once.
pub mod dirs;
pub mod migrate;
pub mod shell;
pub use shell::posix_shell;

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("{0} is not a git repository")]
    NotARepo(PathBuf),
    #[error("git {args} failed: {stderr}")]
    Git { args: String, stderr: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn git(dir: &Path, args: &[&str]) -> Result<String, FsError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        // A repo's own `core.fsmonitor` is a command git would run on `status`; browsing must never execute
        // anything a repo configures.
        .args(["-c", "core.fsmonitor="])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        // `git status` otherwise refreshes the index and takes .git/index.lock, which would make a `git commit`
        // the user runs while the picker polls fail.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(FsError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

/// A path git printed, in the platform's own form. Git for Windows prints `C:/x/y` (and `//server/share/x`); with
/// its slashes turned into backslashes (never part of a Windows file name) it is `C:\x\y`, so it compares equal to
/// `canonical`'s paths and reads natively in the UI. Unchanged on Unix. (Rebuilding from `components()` would not
/// do: a UNC prefix keeps its forward slashes.)
fn from_git(s: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(s.replace('/', "\\"))
    } else {
        PathBuf::from(s)
    }
}

pub enum MainRoot {
    Worktree(PathBuf),
    Bare,
}

/// Where a path's repo is rooted, or None when it is no repo. A subdirectory maps to its toplevel and a linked
/// worktree to its main worktree, so worktrees never hang off a feature-branch checkout that may be deleted.
pub fn main_root(path: &Path) -> Result<Option<MainRoot>, FsError> {
    if git(path, &["rev-parse", "--git-dir"]).is_err() {
        return Ok(None);
    }
    let common = git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = from_git(&common);
    if git(&common, &["rev-parse", "--is-bare-repository"])? == "true" {
        return Ok(Some(MainRoot::Bare));
    }
    let git_dir = from_git(&git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    )?);
    let root = if git_dir == common {
        from_git(&git(path, &["rev-parse", "--show-toplevel"])?)
    } else {
        common
            .parent()
            .expect("git common dir has a parent")
            .to_path_buf()
    };
    Ok(Some(MainRoot::Worktree(root)))
}

/// The checkout containing `path`, or None when `path` is in none (not a repo, or a bare one).
pub fn toplevel(path: &Path) -> Result<Option<PathBuf>, FsError> {
    Ok(git(path, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|s| from_git(&s)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    /// None on an unborn branch (`git init`, no commit yet).
    pub commit: Option<String>,
    /// None on a detached HEAD.
    pub branch: Option<String>,
    pub subject: String,
}

/// The checked-out branch, None when detached or not a repo. symbolic-ref, unlike `rev-parse --abbrev-ref`, also
/// works before the first commit.
fn branch(checkout: &Path) -> Option<String> {
    git(checkout, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()
}

pub fn head(checkout: &Path) -> Result<Head, FsError> {
    if git(checkout, &["rev-parse", "--git-dir"]).is_err() {
        return Err(FsError::NotARepo(checkout.to_path_buf()));
    }
    let branch = branch(checkout);
    let commit = git(checkout, &["rev-parse", "--verify", "--quiet", "HEAD"]).ok();
    let subject = match commit {
        Some(_) => git(checkout, &["log", "-1", "--format=%s"])?,
        None => String::new(),
    };
    Ok(Head {
        commit,
        branch,
        subject,
    })
}

/// Uncommitted files: tracked changes plus untracked, not ignored. Submodule contents are not counted.
pub fn dirty_count(checkout: &Path) -> Result<usize, FsError> {
    Ok(git(
        checkout,
        &["status", "--porcelain", "--ignore-submodules=dirty"],
    )?
    .lines()
    .count())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
    pub path: PathBuf,
    pub main: bool,
    pub head: Head,
}

/// `canonicalize` without Windows' verbatim `\\?\` prefix whenever the plain form names the same file, so paths
/// compare equal to what users type and read normally in the UI. Identical to `std::fs::canonicalize` on Unix.
pub fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(p)
}

fn canonical_or(p: &Path) -> PathBuf {
    canonical(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `a/b/../c` as `a/c`, without reading the disk. A folder picker that follows the path the user went
/// through (symlinks included) and goes Up with `parent()` means the same by `..`.
pub fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            // `/..` is `/`; a relative path keeps the `..` it cannot resolve.
            Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            Component::ParentDir if out.has_root() => {}
            other => out.push(other),
        }
    }
    out
}

/// True when `path` is at or under one of `folders`. Both sides are expected canonical already.
pub fn is_under(path: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|f| path.starts_with(f))
}

/// Every live checkout of the repo `path` belongs to: the main one first, then linked worktrees by path.
/// Checkouts at or under any of `exclude` (fleet's own task folders) are left out.
pub fn checkouts(path: &Path, exclude: &[PathBuf]) -> Result<Vec<Checkout>, FsError> {
    let exclude: Vec<PathBuf> = exclude.iter().map(|x| canonical_or(x)).collect();
    let listing = git(path, &["worktree", "list", "--porcelain"])?;
    let mut found = Vec::new();
    for (i, block) in listing.split("\n\n").enumerate() {
        let mut wt_path = None;
        let (mut bare, mut prunable) = (false, false);
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                wt_path = Some(from_git(p));
            } else if line == "bare" {
                bare = true;
            } else if line == "prunable" || line.starts_with("prunable ") {
                prunable = true;
            }
        }
        let Some(wt_path) = wt_path else { continue };
        if bare || prunable || !wt_path.exists() {
            continue;
        }
        if is_under(&canonical_or(&wt_path), &exclude) {
            continue;
        }
        // One unreadable worktree (foreign owner on a shared mount, broken .git file) must not hide the rest.
        let Ok(head) = head(&wt_path) else { continue };
        found.push(Checkout {
            path: wt_path,
            main: i == 0,
            head,
        });
    }
    // The main checkout stays first; linked ones follow in path order.
    let linked_from = found.iter().take_while(|c| c.main).count();
    found[linked_from..].sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    Checkout { branch: Option<String> },
    Excluded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
}

/// The subfolders of `dir`, hidden ones skipped and symlinks to folders followed.
pub fn list(dir: &Path, exclude: &[PathBuf]) -> Result<Listing, FsError> {
    let dir = canonical(dir)?;
    let exclude: Vec<PathBuf> = exclude.iter().map(|x| canonical_or(x)).collect();
    let mut entries = Vec::new();
    for item in std::fs::read_dir(&dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        let path = dir.join(&name);
        // metadata() follows symlinks; a dangling link or an unreadable entry is simply not a folder.
        if name.starts_with('.') || !std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let kind = if is_under(&canonical_or(&path), &exclude) {
            EntryKind::Excluded
        } else if path.join(".git").exists() {
            EntryKind::Checkout {
                branch: branch(&path),
            }
        } else {
            EntryKind::Folder
        };
        entries.push(Entry { name, path, kind });
    }
    entries.sort_by_key(|e| e.name.to_lowercase());
    Ok(Listing { dir, entries })
}

#[cfg(test)]
mod tests {
    use super::from_git;
    use std::path::PathBuf;

    #[test]
    fn from_git_reads_paths_in_the_platform_form() {
        if cfg!(windows) {
            assert_eq!(
                from_git("C:/Users/x/dev/core-feat").to_str(),
                Some(r"C:\Users\x\dev\core-feat")
            );
            assert_eq!(from_git("C:/").to_str(), Some(r"C:\"));
            assert_eq!(
                from_git("//server/share/repo").to_str(),
                Some(r"\\server\share\repo")
            );
        } else {
            for p in ["/home/x/dev/core-feat", "/", "/tmp/a b/c"] {
                assert_eq!(from_git(p), PathBuf::from(p));
                assert_eq!(from_git(p).to_str(), Some(p));
            }
        }
    }
}
