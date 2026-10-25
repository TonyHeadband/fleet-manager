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
    /// The branch that commit was on, when it was on one.
    pub from_branch: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("{0} is not a git repository")]
    NotARepo(PathBuf),
    #[error("git {args} failed: {stderr}")]
    Git { args: String, stderr: String },
    #[error("cannot run git: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Fs(#[from] fleet_fs::FsError),
}

/// Runs a sync fleet-fs query off the async runtime.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, fleet_fs::FsError> + Send + 'static,
) -> Result<T, GitError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(std::io::Error::other)?
        .map_err(GitError::Fs)
}

async fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        // A clone or fetch outliving its caller (the daemon shutting down) would keep writing into a folder
        // nothing tracks any more.
        .kill_on_drop(true)
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
    if cut.is_empty() {
        "task".to_string()
    } else {
        cut.to_string()
    }
}

/// True when `branch` already exists in `repo`.
pub async fn branch_exists(repo: &Path, branch: &str) -> Result<bool, GitError> {
    Ok(git(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .is_ok())
}

/// Checks out `repo` at `dest` on a new branch `<prefix>/<slug>` started from `start`. The caller picks `dest`
/// and a branch name that is free; this fails, leaving nothing behind, if either is taken.
pub async fn create(
    repo: &Path,
    prefix: &str,
    slug: &str,
    dest: &Path,
    start: &str,
    from_branch: Option<String>,
) -> Result<Worktree, GitError> {
    if git(repo, &["rev-parse", "--git-dir"]).await.is_err() {
        return Err(GitError::NotARepo(repo.to_path_buf()));
    }
    let base = git(repo, &["rev-parse", "--verify", start]).await?;
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
    if let Err(e) = git(
        repo,
        &[
            "worktree",
            "add",
            "--no-track",
            "-b",
            &branch,
            &dest_arg,
            start,
        ],
    )
    .await
    {
        // git can create the branch and then fail on the path; only a branch we made is ours to delete.
        if !existed && branch_exists(repo, &branch).await.unwrap_or(false) {
            let _ = git(repo, &["branch", "-D", &branch]).await;
        }
        undo_parent();
        return Err(e);
    }
    let wt = Worktree {
        path: dest.to_path_buf(),
        branch,
        base: base.clone(),
        from_branch,
    };
    // Recorded so has_changes can tell committed work from the starting point.
    if let Err(e) = git(
        repo,
        &["config", &format!("branch.{}.fleet-base", wt.branch), &base],
    )
    .await
    {
        let _ = remove(repo, &wt).await;
        undo_parent();
        return Err(e);
    }
    Ok(wt)
}

pub async fn clone(url: &str, dest: &Path) -> Result<(), GitError> {
    let parent = dest.parent().expect("clone path has a parent");
    let dest_arg = dest.to_string_lossy().into_owned();
    // `--` so a URL can never be read as an option.
    git(parent, &["clone", "--quiet", "--", url, &dest_arg])
        .await
        .map(|_| ())
}

pub async fn fetch(repo: &Path) -> Result<(), GitError> {
    git(repo, &["fetch", "--quiet", "origin"]).await.map(|_| ())
}

/// Whether `repo`'s object store has `commit`. A non-zero exit is "no"; failing to run git at all is an error.
pub async fn has_commit(repo: &Path, commit: &str) -> Result<bool, GitError> {
    match git(repo, &["cat-file", "-e", &format!("{commit}^{{commit}}")]).await {
        Ok(_) => Ok(true),
        Err(GitError::Git { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

pub async fn has_remote(repo: &Path, name: &str) -> Result<bool, GitError> {
    Ok(git(repo, &["remote"]).await?.lines().any(|l| l == name))
}

/// The commit and short name (`origin/main`) of the remote default branch, when the repo has one: new work from a
/// URL starts there, not at a stale local checkout.
pub async fn origin_head(repo: &Path) -> Result<Option<(String, String)>, GitError> {
    let Ok(commit) = git(repo, &["rev-parse", "--verify", "--quiet", "origin/HEAD"]).await else {
        return Ok(None);
    };
    Ok(Some((
        commit,
        git(repo, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).await?,
    )))
}

/// The checked-out branch, or None on a detached HEAD. Works on an unborn branch (`git init`, no commit yet),
/// where `rev-parse --abbrev-ref HEAD` fails.
pub async fn current_branch(repo: &Path) -> Result<Option<String>, GitError> {
    if git(repo, &["rev-parse", "--git-dir"]).await.is_err() {
        return Err(GitError::NotARepo(repo.to_path_buf()));
    }
    Ok(git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .ok())
}

/// Removes the checkout only; the branch, and the commits on it, stay in `repo`.
pub async fn remove_worktree(repo: &Path, wt: &Worktree) -> Result<(), GitError> {
    remove_worktree_at(repo, &wt.path).await
}

/// `remove_worktree` for a worktree known only by its path, as an orphan is.
pub async fn remove_worktree_at(repo: &Path, path: &Path) -> Result<(), GitError> {
    let path_arg = path.to_string_lossy().into_owned();
    if path.exists() {
        git(repo, &["worktree", "remove", "--force", &path_arg]).await?;
    } else {
        git(repo, &["worktree", "prune"]).await?;
    }
    Ok(())
}

/// How many files in the worktree are changed, untracked or ignored: what an archive loses, since only commits stay
/// on the branch. Each untracked file counts, not just its folder. Ignored files count too (`.env`, local notes),
/// but not the build output under `footprint::BUILD_DIRS`, which can be built again; an ignored folder
/// counts once, since git does not look inside it (a `node_modules` would make this walk slow).
pub async fn uncommitted_files(wt: &Path) -> Result<u32, GitError> {
    let out = git(
        wt,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )
    .await?;
    Ok(out
        .lines()
        .filter(|l| !l.is_empty())
        .filter(|l| !l.strip_prefix("!! ").is_some_and(is_build_output))
        .count() as u32)
}

/// A path from `git status` (quoted when it holds unusual characters) inside a build folder at the worktree root.
fn is_build_output(path: &str) -> bool {
    let path = path.trim_start_matches('"');
    let top = path.split('/').next().unwrap_or(path);
    crate::footprint::BUILD_DIRS.contains(&top)
}

/// Forgets the worktree at `path` after its folder was deleted by hand. Prune skips a locked worktree, which would
/// leave its branch marked as checked out, so it is unlocked first (failing harmlessly when it was not locked).
pub async fn forget_worktree(repo: &Path, path: &Path) -> Result<(), GitError> {
    let _ = git(repo, &["worktree", "unlock", &path.to_string_lossy()]).await;
    git(repo, &["worktree", "prune"]).await.map(|_| ())
}

/// Removes the checkout and its branch: only for undoing a `create` whose branch nobody has used yet.
pub async fn remove(repo: &Path, wt: &Worktree) -> Result<(), GitError> {
    remove_worktree(repo, wt).await?;
    // Absent branch is fine: removal must be repeatable after a partial failure.
    if git(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", wt.branch),
        ],
    )
    .await
    .is_ok()
    {
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
        // A clone or fetch outliving its caller (the daemon shutting down) would keep writing into a folder
        // nothing tracks any more.
        .kill_on_drop(true)
        .output()
        .await?;
    match out.status.code() {
        Some(0 | 1) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        _ => Err(GitError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        }),
    }
}

/// Reads only: it never stages anything, so the agent's index is untouched.
pub async fn diff(wt: &Path, base: &str, cap: usize) -> Result<Diff, GitError> {
    let mut text = git_diff_output(wt, &["diff", "--no-color", base]).await?;
    let listed = git_diff_output(wt, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    for file in listed
        .split('\0')
        .filter(|f| !f.is_empty())
        .take(MAX_UNTRACKED_FILES)
    {
        if text.len() > cap {
            break;
        }
        text.push_str(
            &git_diff_output(
                wt,
                &["diff", "--no-color", "--no-index", "--", "/dev/null", file],
            )
            .await?,
        );
    }
    if text.len() <= cap {
        return Ok(Diff {
            text,
            truncated: false,
        });
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    Ok(Diff {
        text,
        truncated: true,
    })
}

/// True when the worktree has uncommitted changes or commits beyond where it started.
pub async fn has_changes(wt: &Path) -> Result<bool, GitError> {
    if !git(wt, &["status", "--porcelain"]).await?.is_empty() {
        return Ok(true);
    }
    let branch = git(wt, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let Ok(base) = git(
        wt,
        &["config", "--get", &format!("branch.{branch}.fleet-base")],
    )
    .await
    else {
        return Ok(false);
    };
    let ahead = git(wt, &["rev-list", "--count", &format!("{base}..HEAD")]).await?;
    Ok(ahead != "0")
}

/// `has_changes` for a repo the agent `git init`ed: it started from nothing, so any commit is its work too.
pub async fn owned_has_changes(repo: &Path) -> Result<bool, GitError> {
    if !git(repo, &["status", "--porcelain"]).await?.is_empty() {
        return Ok(true);
    }
    match git(repo, &["rev-parse", "--verify", "--quiet", "HEAD"]).await {
        Ok(_) => Ok(true),
        // Unborn: no commit yet.
        Err(GitError::Git { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}
