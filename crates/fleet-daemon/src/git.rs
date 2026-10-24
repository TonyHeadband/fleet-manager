//! One git worktree and branch per agent. Shells out to `git` with argv, never through a shell.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

const MAX_SLUG: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    /// The commit the branch started from.
    pub base: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("{0} is not a git repository")]
    NotARepo(PathBuf),
    #[error("git {args} failed: {stderr}")]
    Git { args: String, stderr: String },
    #[error("cannot run git: {0}")]
    Io(#[from] std::io::Error),
}

async fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .await?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(GitError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

/// Lowercase ASCII words joined by single dashes; safe in a branch name and a path.
pub fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let cut: String = slug.trim_matches('-').chars().take(MAX_SLUG).collect();
    let cut = cut.trim_end_matches('-');
    if cut.is_empty() { "task".to_string() } else { cut.to_string() }
}

/// True when `branch` already exists in `repo`.
pub async fn branch_exists(repo: &Path, branch: &str) -> Result<bool, GitError> {
    Ok(git(repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_ok())
}

/// Checks out `repo` at `dest` on a new branch `<prefix>/<slug>` started from HEAD. The caller picks `dest`
/// and a branch name that is free; this fails, leaving nothing behind, if either is taken.
pub async fn create(repo: &Path, prefix: &str, slug: &str, dest: &Path) -> Result<Worktree, GitError> {
    if git(repo, &["rev-parse", "--git-dir"]).await.is_err() {
        return Err(GitError::NotARepo(repo.to_path_buf()));
    }
    let base = git(repo, &["rev-parse", "--verify", "HEAD"]).await?;
    let branch = format!("{prefix}/{slug}");

    let parent = dest.parent().expect("worktree path has a parent");
    let made_parent = !parent.exists();
    std::fs::create_dir_all(parent)?;
    let undo_parent = || {
        if made_parent {
            let _ = std::fs::remove_dir(parent);
        }
    };
    let dest_arg = dest.to_string_lossy().into_owned();
    let existed = branch_exists(repo, &branch).await?;
    if let Err(e) = git(repo, &["worktree", "add", "-b", &branch, &dest_arg, "HEAD"]).await {
        // git can create the branch and then fail on the path; only a branch we made is ours to delete.
        if !existed && branch_exists(repo, &branch).await.unwrap_or(false) {
            let _ = git(repo, &["branch", "-D", &branch]).await;
        }
        undo_parent();
        return Err(e);
    }
    let wt = Worktree { path: dest.to_path_buf(), branch, base: base.clone() };
    // Recorded so has_changes can tell committed work from the starting point.
    if let Err(e) = git(repo, &["config", &format!("branch.{}.fleet-base", wt.branch), &base]).await {
        let _ = remove(repo, &wt).await;
        undo_parent();
        return Err(e);
    }
    Ok(wt)
}

pub async fn remove(repo: &Path, wt: &Worktree) -> Result<(), GitError> {
    let path_arg = wt.path.to_string_lossy().into_owned();
    if wt.path.exists() {
        git(repo, &["worktree", "remove", "--force", &path_arg]).await?;
    } else {
        git(repo, &["worktree", "prune"]).await?;
    }
    // Absent branch is fine: removal must be repeatable after a partial failure.
    if git(repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{}", wt.branch)]).await.is_ok() {
        git(repo, &["branch", "-D", &wt.branch]).await?;
    }
    Ok(())
}

/// Unified diff of a worktree against its `base` commit: committed work, uncommitted edits and untracked files.
pub struct Diff {
    pub text: String,
    /// True when `text` was cut at the byte cap.
    pub truncated: bool,
}

const MAX_UNTRACKED_FILES: usize = 200;

/// Like `git()` but returns stdout untrimmed and accepts exit code 1 (what `diff --no-index` returns on a difference).
async fn git_diff_output(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .await?;
    match out.status.code() {
        Some(0 | 1) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        _ => Err(GitError::Git { args: args.join(" "), stderr: String::from_utf8_lossy(&out.stderr).trim().to_string() }),
    }
}

/// Reads only: it never stages anything, so the agent's index is untouched.
pub async fn diff(wt: &Path, base: &str, cap: usize) -> Result<Diff, GitError> {
    let mut text = git_diff_output(wt, &["diff", "--no-color", base]).await?;
    let listed = git_diff_output(wt, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    for file in listed.split('\0').filter(|f| !f.is_empty()).take(MAX_UNTRACKED_FILES) {
        if text.len() > cap {
            break;
        }
        text.push_str(&git_diff_output(wt, &["diff", "--no-color", "--no-index", "--", "/dev/null", file]).await?);
    }
    if text.len() <= cap {
        return Ok(Diff { text, truncated: false });
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    Ok(Diff { text, truncated: true })
}

/// True when the worktree has uncommitted changes or commits beyond where it started.
pub async fn has_changes(wt: &Path) -> Result<bool, GitError> {
    if !git(wt, &["status", "--porcelain"]).await?.is_empty() {
        return Ok(true);
    }
    let branch = git(wt, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let Ok(base) = git(wt, &["config", "--get", &format!("branch.{branch}.fleet-base")]).await else {
        return Ok(false);
    };
    let ahead = git(wt, &["rev-list", "--count", &format!("{base}..HEAD")]).await?;
    Ok(ahead != "0")
}
