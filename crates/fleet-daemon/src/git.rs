//! One git worktree and branch per agent. Shells out to `git` with argv, never through a shell.
use crate::personas::ambient_vars;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
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
    git_env(dir, args, &[]).await
}

/// What every daemon git (and gh) call runs with: the user's real PATH from the last tool check (`fleet_fs::child`),
/// no ambient token or git credential setup, never a prompt (on the terminal or through an askpass program, which on
/// a desktop pops up a window), then `env` (a persona's) on top. What reaches a remote is only what the persona's
/// credential helper hands out.
pub fn harden(cmd: &mut std::process::Command, env: &[(String, String)]) {
    fleet_fs::child::apply_path(cmd);
    for k in ambient_vars() {
        cmd.env_remove(k);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("GIT_ASKPASS", "");
    // Without a persona nothing should answer at all: the empty helper clears the user's own (a `store` file, a
    // keyring), which would otherwise hand their credentials to whatever remote a local repo names.
    if !env.iter().any(|(k, _)| k == "GIT_CONFIG_COUNT") {
        cmd.env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "credential.helper")
            .env("GIT_CONFIG_VALUE_0", "");
    }
    cmd.envs(env.iter().map(|(k, v)| (k, v)));
}

async fn git_env(dir: &Path, args: &[&str], env: &[(String, String)]) -> Result<String, GitError> {
    Ok(git_raw(dir, args, env).await?.trim().to_string())
}

/// `git_env` with stdout as git wrote it, for output where leading or trailing spaces are data (`-z` names, file
/// content).
async fn git_raw(dir: &Path, args: &[&str], env: &[(String, String)]) -> Result<String, GitError> {
    Ok(String::from_utf8_lossy(&git_bytes(dir, args, env).await?).into_owned())
}

/// `git_raw` without the UTF-8 conversion, for content that may not be text.
async fn git_bytes(
    dir: &Path,
    args: &[&str],
    env: &[(String, String)],
) -> Result<Vec<u8>, GitError> {
    let mut cmd = Command::new("git");
    harden(cmd.as_std_mut(), env);
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        // A clone or fetch outliving its caller (the daemon shutting down) would keep writing into a folder
        // nothing tracks any more.
        .kill_on_drop(true);
    let out = output(cmd).await?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(GitError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

/// `cmd.output()`, except that a cancelled call (a fetch timeout) ends everything git started, not only git: the
/// transport it runs (`sh -c <sshCommand>`, ssh) would otherwise live on. On Windows that also hung the call, as
/// git on PATH is often a launcher (`cmd\git.exe`) for the real one and the survivors kept the output pipes open.
/// The call's process group (Unix) or Job Object (Windows) is killed when the call is dropped.
async fn output(cmd: Command) -> std::io::Result<std::process::Output> {
    use crate::session::tree::Tree;
    let mut cmd = cmd;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    cmd.process_group(0);
    // Suspended until it is in the job, so the launcher cannot start the real git outside it.
    #[cfg(windows)]
    cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    let mut child = cmd.spawn()?;
    let tree = match child.id().map(Tree::adopt) {
        Some(Ok(t)) => t,
        Some(Err(e)) => {
            let _ = child.start_kill();
            return Err(e);
        }
        None => return Err(std::io::Error::other("git exited at once")),
    };
    #[cfg(windows)]
    resume(&child)?;
    let guard = KillTree(Some(tree));
    let out = child.wait_with_output().await;
    // Reaped: on Unix the group id may be reused from here. Dropping the Windows job still ends what git left.
    drop(guard.disarm());
    out
}

/// Kills the tree when dropped while armed: the call was cancelled before git was reaped.
struct KillTree(Option<crate::session::tree::Tree>);

impl KillTree {
    fn disarm(mut self) -> Option<crate::session::tree::Tree> {
        self.0.take()
    }
}

impl Drop for KillTree {
    fn drop(&mut self) {
        if let Some(t) = &self.0 {
            t.kill();
        }
    }
}

/// Starts the threads of a process spawned suspended; std hands out no thread handle, so through ntdll.
#[cfg(windows)]
fn resume(child: &tokio::process::Child) -> std::io::Result<()> {
    #[link(name = "ntdll")]
    extern "system" {
        fn NtResumeProcess(process: windows_sys::Win32::Foundation::HANDLE) -> i32;
    }
    let handle = child
        .raw_handle()
        .ok_or_else(|| std::io::Error::other("git exited at once"))?;
    // SAFETY: a live process handle owned by `child`; the call only resumes its threads.
    let status = unsafe { NtResumeProcess(handle as _) };
    if status < 0 {
        return Err(std::io::Error::other(format!(
            "cannot resume git: NTSTATUS {status:#x}"
        )));
    }
    Ok(())
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

/// The commit `refs/heads/<branch>` points at, or None when there is no such branch.
pub async fn branch_tip(repo: &Path, branch: &str) -> Result<Option<String>, GitError> {
    if !branch_exists(repo, branch).await? {
        return Ok(None);
    }
    let r = format!("refs/heads/{branch}^{{commit}}");
    Ok(Some(git(repo, &["rev-parse", "--verify", &r]).await?))
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

/// `env` is the persona's: its credential helper and identity reach git only through the environment, so nothing
/// of it is written to the clone's `.git/config`.
/// With `no_hooks`, the user's hooks (its `post-checkout`) do not run.
pub async fn clone(
    url: &str,
    dest: &Path,
    env: &[(String, String)],
    no_hooks: Option<&Path>,
) -> Result<(), GitError> {
    let parent = dest.parent().expect("clone path has a parent");
    let dest_arg = dest.to_string_lossy().into_owned();
    let mut args = match no_hooks {
        Some(dir) => hookless(dir)?,
        None => Vec::new(),
    };
    // `--` so a URL can never be read as an option.
    args.extend(["clone", "--quiet", "--", url, &dest_arg].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git_env(parent, &args, env).await.map(|_| ())
}

pub async fn fetch(repo: &Path, env: &[(String, String)]) -> Result<(), GitError> {
    git_env(repo, &["fetch", "--quiet", "origin"], env)
        .await
        .map(|_| ())
}

/// How long a call that talks to a remote (fetch, push, `gh pr create`) may take. None can prompt, so only a stuck
/// network gets this far.
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(300);

/// Fetches `+refs/heads/<branch>:refs/remotes/origin/<branch>` from `origin`, so `origin/<branch>` is what the remote
/// has now whatever the local refs say.
pub async fn fetch_branch(
    repo: &Path,
    branch: &str,
    env: &[(String, String)],
) -> Result<(), GitError> {
    let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    git_env(repo, &["fetch", "--quiet", "origin", &spec], env)
        .await
        .map(|_| ())
}

/// Fetches `+refs/heads/<pattern>:refs/remotes/origin/<pattern>` from `origin` for each pattern (which may hold a
/// `*`), and drops the tracking refs those patterns cover whose branch the remote no longer has.
pub async fn fetch_pruned(
    repo: &Path,
    patterns: &[&str],
    env: &[(String, String)],
) -> Result<(), GitError> {
    let specs: Vec<String> = patterns
        .iter()
        .map(|p| format!("+refs/heads/{p}:refs/remotes/origin/{p}"))
        .collect();
    let mut args = vec!["fetch", "--quiet", "--prune", "origin"];
    args.extend(specs.iter().map(String::as_str));
    git_env(repo, &args, env).await.map(|_| ())
}

/// The branches of `origin` that match `pattern` (a `for-each-ref` glob), as the last fetch or push left their
/// tracking refs; nothing asks the remote.
pub async fn tracking_branches(repo: &Path, pattern: &str) -> Result<Vec<String>, GitError> {
    let refs = format!("refs/remotes/origin/{pattern}");
    let out = git(repo, &["for-each-ref", "--format=%(refname)", &refs]).await?;
    Ok(out
        .lines()
        .filter_map(|l| l.strip_prefix("refs/remotes/origin/"))
        .map(String::from)
        .collect())
}

/// The commit `refs/heads/<branch>` points at on `url`, read with `ls-remote` (no ref of `dir` changes), or None
/// when the remote has no such branch. `dir` is only where git runs.
pub async fn ls_remote_branch(
    dir: &Path,
    url: &str,
    branch: &str,
    env: &[(String, String)],
) -> Result<Option<String>, GitError> {
    let r = format!("refs/heads/{branch}");
    // `--` so a URL can never be read as an option.
    let out = git_env(dir, &["ls-remote", "--", url, &r], env).await?;
    Ok(out
        .lines()
        .find_map(|l| l.split_once('\t').filter(|(_, name)| *name == r))
        .map(|(sha, _)| sha.to_string()))
}

/// The content of `path` at `rev`, or None when the commit has no such path.
pub async fn show_file(repo: &Path, rev: &str, path: &str) -> Result<Option<String>, GitError> {
    // ls-tree fails on a bad rev but is empty for a missing path, which `show` reports only through its message.
    if git(repo, &["ls-tree", "--name-only", rev, "--", path])
        .await?
        .is_empty()
    {
        return Ok(None);
    }
    git_raw(repo, &["show", &format!("{rev}:{path}")], &[])
        .await
        .map(Some)
}

/// The commit `rev` names, as a full sha.
pub async fn rev_parse_commit(repo: &Path, rev: &str) -> Result<String, GitError> {
    git(
        repo,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )
    .await
}

/// How many commits `refs/heads/<branch>` has that `base` does not.
pub async fn commits_beyond(repo: &Path, base: &str, branch: &str) -> Result<u32, GitError> {
    let range = format!("{base}..refs/heads/{branch}");
    let out = git(repo, &["rev-list", "--count", &range, "--"]).await?;
    out.parse().map_err(|_| GitError::Git {
        args: format!("rev-list --count {range}"),
        stderr: format!("unreadable count {out:?}"),
    })
}

/// The best common ancestor of `a` and `b`.
pub async fn merge_base(repo: &Path, a: &str, b: &str) -> Result<String, GitError> {
    git(repo, &["merge-base", a, b]).await
}

/// Files that differ between `from` and `to`, limited to `pathspecs` (all of them when empty). A rename counts as
/// both sides, so a step watching the old directory still sees the file leave.
pub async fn changed_files(
    repo: &Path,
    from: &str,
    to: &str,
    pathspecs: &[String],
) -> Result<Vec<String>, GitError> {
    // Always `--`: a ref-looking pathspec must never be read as a revision.
    let mut args = vec!["diff", "--name-only", "--no-renames", "-z", from, to, "--"];
    args.extend(pathspecs.iter().map(String::as_str));
    Ok(git_raw(repo, &args, &[])
        .await?
        .split('\0')
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect())
}

/// The URL `remote` points at, or None when the repo has no such remote.
pub async fn remote_url(repo: &Path, remote: &str) -> Result<Option<String>, GitError> {
    if !has_remote(repo, remote).await? {
        return Ok(None);
    }
    git(repo, &["remote", "get-url", remote]).await.map(Some)
}

/// The URL `remote` is configured with, as written (no `insteadOf` rewriting), or None when it has none.
pub async fn configured_remote_url(repo: &Path, remote: &str) -> Result<Option<String>, GitError> {
    match git(repo, &["config", "--get", &format!("remote.{remote}.url")]).await {
        Ok(url) => Ok(Some(url)),
        // `config --get` exits 1 for a missing key.
        Err(GitError::Git { .. }) => Ok(None),
        Err(e) => Err(e),
    }
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
    let mut cmd = Command::new("git");
    harden(cmd.as_std_mut(), &[]);
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        // A clone or fetch outliving its caller (the daemon shutting down) would keep writing into a folder
        // nothing tracks any more.
        .kill_on_drop(true);
    let out = output(cmd).await?;
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

/// True when the worktree has anything `git status` lists: changes, staged or not, and untracked files.
pub async fn is_dirty(wt: &Path) -> Result<bool, GitError> {
    Ok(!git(wt, &["status", "--porcelain"]).await?.is_empty())
}

/// True while a merge waits to be committed or aborted: once its conflicts are staged the status can be clean.
pub async fn merging(wt: &Path) -> Result<bool, GitError> {
    Ok(git(wt, &["rev-parse", "--quiet", "--verify", "MERGE_HEAD"])
        .await
        .is_ok())
}

/// The commit HEAD points at.
pub async fn head_commit(wt: &Path) -> Result<String, GitError> {
    git(wt, &["rev-parse", "--verify", "HEAD^{commit}"]).await
}

/// Pushes exactly `commit` to `branch` on `origin`, with `env` (a persona's) for the credentials, whatever the local
/// branch points at by then.
pub async fn push_commit(
    wt: &Path,
    commit: &str,
    branch: &str,
    env: &[(String, String)],
) -> Result<(), GitError> {
    let spec = format!("{commit}:refs/heads/{branch}");
    git_env(wt, &["push", "origin", &spec], env).await?;
    Ok(())
}

/// What `push -u` would have set: `branch` follows the branch of the same name on `origin`.
pub async fn set_upstream(wt: &Path, branch: &str) -> Result<(), GitError> {
    git(
        wt,
        &["config", &format!("branch.{branch}.remote"), "origin"],
    )
    .await?;
    let merge = format!("refs/heads/{branch}");
    git(wt, &["config", &format!("branch.{branch}.merge"), &merge]).await?;
    Ok(())
}

/// Pushes local branch `local` to `remote_branch` on `origin`, without the user's hooks (`pre-push`). Never forced:
/// a remote that moved on rejects it (see `is_rejected`).
pub async fn push_branch(
    repo: &Path,
    no_hooks: &Path,
    local: &str,
    remote_branch: &str,
    env: &[(String, String)],
) -> Result<(), GitError> {
    let spec = format!("refs/heads/{local}:refs/heads/{remote_branch}");
    let mut args = hookless(no_hooks)?;
    args.extend(["push", "--quiet", "origin", &spec].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git_env(repo, &args, env).await.map(|_| ())
}

/// Whether a failed push was refused because the remote has commits the local branch lacks: checked before sending
/// (`[rejected]`), or found by the remote when another push landed meanwhile (`[remote rejected]`, its ref moved).
pub fn is_rejected(e: &GitError) -> bool {
    const MARKS: &[&str] = &[
        "[rejected]",
        "non-fast-forward",
        "fetch first",
        "[remote rejected]",
        "incorrect old value",
        "cannot lock ref",
    ];
    matches!(e, GitError::Git { stderr, .. } if MARKS.iter().any(|m| stderr.contains(m)))
}

pub async fn remove_remote(repo: &Path, name: &str) -> Result<(), GitError> {
    git(repo, &["remote", "remove", name]).await.map(|_| ())
}

/// Commits in `range` (`a..b`).
pub async fn rev_count(repo: &Path, range: &str) -> Result<u32, GitError> {
    let out = git(repo, &["rev-list", "--count", range]).await?;
    Ok(out.parse().unwrap_or(0))
}

/// Fast-forwards the checked-out branch to `rev`. False only when `rev` is not a descendant of `HEAD` (the branches
/// diverged), with nothing changed; any other failure (a missing `rev`, a lock, a dirty tree) is an error. The user's
/// hooks (`post-merge`) do not run.
pub async fn merge_ff_only(repo: &Path, no_hooks: &Path, rev: &str) -> Result<bool, GitError> {
    match git(repo, &["merge-base", "--is-ancestor", "HEAD", rev]).await {
        Ok(_) => {}
        // `--is-ancestor` exits 1 without a message for "no"; a real failure (exit 128) explains itself.
        Err(GitError::Git { stderr, .. }) if stderr.is_empty() => return Ok(false),
        Err(e) => return Err(e),
    }
    let mut args = hookless(no_hooks)?;
    args.extend(["merge", "--ff-only", "--quiet", rev].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(repo, &args).await.map(|_| true)
}

pub async fn reset_hard(repo: &Path, rev: &str) -> Result<(), GitError> {
    git(repo, &["reset", "--hard", "--quiet", rev])
        .await
        .map(|_| ())
}

/// Without the user's hooks (`post-checkout`).
pub async fn checkout(repo: &Path, no_hooks: &Path, branch: &str) -> Result<(), GitError> {
    let mut args = hookless(no_hooks)?;
    args.extend(["checkout", "--quiet", branch].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(repo, &args).await.map(|_| ())
}

/// Points `HEAD` at `branch`, which may not exist yet (the first commit then creates it).
pub async fn set_head(repo: &Path, branch: &str) -> Result<(), GitError> {
    git(
        repo,
        &["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")],
    )
    .await
    .map(|_| ())
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

/// What the daemon's own commits (the internal repo's first commit and its merges, by this identity rather than a
/// persona's) run with: the identity, and none of the user's hooks (a global `core.hooksPath`, or hooks
/// `init.templateDir` copied in), which could refuse or stop a commit nobody is there to fix. `no_hooks` is an empty
/// folder, created here; `/dev/null` is no folder on Windows.
fn daemon_ident(no_hooks: &Path) -> Result<Vec<String>, GitError> {
    let mut args = hookless(no_hooks)?;
    args.extend(
        [
            "-c",
            "user.name=Fleet Manager",
            "-c",
            "user.email=fleet-manager@localhost",
            "-c",
            "commit.gpgsign=false",
        ]
        .map(String::from),
    );
    Ok(args)
}

/// Leading `-c` arguments pointing `core.hooksPath` at `no_hooks` (an empty folder, created when missing), so none
/// of the user's hooks (global, or copied in from `init.templateDir`) runs.
fn hookless(no_hooks: &Path) -> Result<Vec<String>, GitError> {
    std::fs::create_dir_all(no_hooks)?;
    Ok(vec![
        "-c".to_string(),
        format!(
            "core.hooksPath={}",
            no_hooks.to_string_lossy().replace('\\', "/")
        ),
    ])
}

/// `git init -b main` in `dir` (created when missing), then one commit adding `files` (`(path, text)`; none makes
/// it empty) so `main` exists to branch from.
/// Repeatable: an `init` over a repo is harmless, and the commit is skipped when `main` already has one.
pub async fn init_main(
    dir: &Path,
    no_hooks: &Path,
    message: &str,
    files: &[(&str, &str)],
) -> Result<(), GitError> {
    tokio::fs::create_dir_all(dir).await?;
    git(dir, &["init", "-q", "-b", "main"]).await?;
    if git(
        dir,
        &["rev-parse", "--verify", "--quiet", "refs/heads/main"],
    )
    .await
    .is_ok()
    {
        return Ok(());
    }
    for (path, text) in files {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&file, text).await?;
        git(dir, &["add", "--", path]).await?;
    }
    let mut args = daemon_ident(no_hooks)?;
    args.extend(
        [
            "commit",
            "-q",
            "--no-verify",
            "--allow-empty",
            "-m",
            message,
        ]
        .map(String::from),
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(dir, &args).await.map(|_| ())
}

/// Checks out the existing `branch` at `dest`, without the user's hooks (`post-checkout`).
pub async fn worktree_add_branch(
    repo: &Path,
    no_hooks: &Path,
    dest: &Path,
    branch: &str,
) -> Result<(), GitError> {
    let mut args = hookless(no_hooks)?;
    args.extend(["worktree", "add", &dest.to_string_lossy(), branch].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(repo, &args).await.map(|_| ())
}

/// Checks out `rev` at `dest` with a detached HEAD, so nothing committed there lands on a branch. Without the
/// user's hooks (`post-checkout`).
pub async fn worktree_add_detached(
    repo: &Path,
    no_hooks: &Path,
    dest: &Path,
    rev: &str,
) -> Result<(), GitError> {
    let mut args = hookless(no_hooks)?;
    args.extend(["worktree", "add", "--detach", &dest.to_string_lossy(), rev].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(repo, &args).await.map(|_| ())
}

/// A new branch `name` at `start`, not checked out.
pub async fn create_branch(repo: &Path, name: &str, start: &str) -> Result<(), GitError> {
    git(repo, &["branch", name, start]).await.map(|_| ())
}

/// True when `path` is a worktree `repo` knows of (its folder may be gone).
pub async fn worktree_listed(repo: &Path, path: &Path) -> Result<bool, GitError> {
    let out = git(repo, &["worktree", "list", "--porcelain"]).await?;
    let want = normalize(path);
    Ok(out
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .any(|p| normalize(Path::new(p)) == want))
}

/// The worktree of `repo` that has `branch` checked out, if any (its folder may be gone).
pub async fn worktree_of_branch(repo: &Path, branch: &str) -> Result<Option<PathBuf>, GitError> {
    let out = git(repo, &["worktree", "list", "--porcelain"]).await?;
    let want = format!("branch refs/heads/{branch}");
    let mut current: Option<&str> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current = Some(p);
        } else if line == want {
            return Ok(current.map(fleet_fs::from_git));
        }
    }
    Ok(None)
}

/// Deletes the local `branch`, with commits nobody else holds; absent is fine so a repeat after a partial failure works.
pub async fn delete_branch(repo: &Path, branch: &str) -> Result<(), GitError> {
    if branch_exists(repo, branch).await? {
        git(repo, &["branch", "-D", branch]).await?;
    }
    Ok(())
}

/// The path with symlinks resolved. Git records a worktree by its real path, and once the folder is gone only the
/// parent can be resolved, so the last component is joined back on.
fn normalize(path: &Path) -> PathBuf {
    if let Ok(p) = fleet_fs::canonical(path) {
        return p;
    }
    match (
        path.parent().and_then(|d| fleet_fs::canonical(d).ok()),
        path.file_name(),
    ) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => path.to_path_buf(),
    }
}

/// How `merge_no_ff` ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Merge {
    /// Merged, or had nothing to merge.
    Done,
    /// Stopped mid-merge (conflicts, or anything else that left `MERGE_HEAD`), with git's message. Left in place for
    /// `conflicted_paths` and `merge_abort`.
    Stopped(String),
}

/// Merges `branch` into the checked-out branch of `repo` with a merge commit, even when it could fast-forward.
pub async fn merge_no_ff(repo: &Path, no_hooks: &Path, branch: &str) -> Result<Merge, GitError> {
    let mut args = daemon_ident(no_hooks)?;
    args.extend(["merge", "--no-ff", "--no-edit", "--no-verify", branch].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match git(repo, &args).await {
        Ok(_) => Ok(Merge::Done),
        Err(GitError::Git { stderr, .. }) if merging(repo).await? => Ok(Merge::Stopped(stderr)),
        Err(e) => Err(e),
    }
}

pub async fn merge_abort(repo: &Path) -> Result<(), GitError> {
    git(repo, &["merge", "--abort"]).await.map(|_| ())
}

/// Paths with unresolved merge conflicts.
pub async fn conflicted_paths(repo: &Path) -> Result<Vec<String>, GitError> {
    let out = git_raw(repo, &["diff", "--name-only", "--diff-filter=U", "-z"], &[]).await?;
    Ok(split_nul(&out))
}

fn split_nul(out: &str) -> Vec<String> {
    out.split('\0')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Every file at `rev`, as repo-relative paths.
pub async fn ls_tree_names(repo: &Path, rev: &str) -> Result<Vec<String>, GitError> {
    let out = git_raw(repo, &["ls-tree", "-r", "--name-only", "-z", rev], &[]).await?;
    Ok(split_nul(&out))
}

/// The mode and object type (`blob`, `tree`, `commit`) of exactly `path` at `rev`; None when it is not there.
pub async fn tree_entry(
    repo: &Path,
    rev: &str,
    path: &str,
) -> Result<Option<(String, String)>, GitError> {
    let out = git_raw(repo, &["ls-tree", "-z", rev, "--", path], &[]).await?;
    for entry in out.split('\0').filter(|e| !e.is_empty()) {
        let Some((meta, name)) = entry.split_once('\t') else {
            continue;
        };
        if name != path {
            continue;
        }
        let mut f = meta.split(' ');
        if let (Some(mode), Some(kind)) = (f.next(), f.next()) {
            return Ok(Some((mode.to_string(), kind.to_string())));
        }
    }
    Ok(None)
}

/// Size in bytes of the blob at `rev:path`.
pub async fn blob_size(repo: &Path, rev: &str, path: &str) -> Result<u64, GitError> {
    let out = git(repo, &["cat-file", "-s", &format!("{rev}:{path}")]).await?;
    out.parse().map_err(|_| GitError::Git {
        args: "cat-file -s".to_string(),
        stderr: format!("unexpected size {out:?}"),
    })
}

/// The blob at `rev:path`, as stored.
pub async fn blob_bytes(repo: &Path, rev: &str, path: &str) -> Result<Vec<u8>, GitError> {
    git_bytes(repo, &["cat-file", "blob", &format!("{rev}:{path}")], &[]).await
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub sha: String,
    pub at: i64,
    pub author: String,
    pub subject: String,
}

/// Up to `limit` commits reachable from `rev`, newest first, only those touching `path` when given.
pub async fn log_entries(
    repo: &Path,
    rev: &str,
    path: Option<&str>,
    limit: usize,
) -> Result<Vec<LogEntry>, GitError> {
    let n = format!("-n{limit}");
    let mut args = vec!["log", &n, "-z", "--format=%H%x1f%ct%x1f%an%x1f%s", rev];
    if let Some(p) = path {
        args.extend(["--", p]);
    }
    let out = git_raw(repo, &args, &[]).await?;
    Ok(out
        .split('\0')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|r| {
            let mut f = r.trim_start_matches('\n').splitn(4, '\x1f');
            Some(LogEntry {
                sha: f.next()?.to_string(),
                at: f.next()?.parse().ok()?,
                author: f.next()?.to_string(),
                subject: f.next()?.to_string(),
            })
        })
        .collect())
}

/// Names of the local branches, sorted by git.
pub async fn local_branches(repo: &Path) -> Result<Vec<String>, GitError> {
    let out = git(
        repo,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
    .await?;
    Ok(out.lines().map(String::from).collect())
}

#[cfg(test)]
mod tests {
    use super::{harden, is_rejected, GitError};
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};

    /// A git with nothing of the environment's own git setup: an agent's (where the gate runs) sets a protocol list,
    /// an empty `credential.helper` through `GIT_CONFIG_*` and more, which would decide the controls below.
    fn test_git() -> Command {
        let mut cmd = Command::new("git");
        for (k, _) in std::env::vars_os() {
            if k.to_string_lossy().starts_with("GIT_") {
                cmd.env_remove(k);
            }
        }
        cmd
    }

    /// `p` with forward slashes: git runs a `!` helper and `GIT_SSH_COMMAND` through `sh -c`, which reads `\` as an
    /// escape, and so does a gitconfig file. Git for Windows' sh takes `C:/x`.
    fn fwd(p: &Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }

    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        // Executable bit on Unix only; Windows has none, and Git Bash runs the script by its shebang.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// `git credential fill` for a host nobody serves: it reads config and runs helpers and askpass, and never
    /// touches the network.
    fn fill(dir: &Path, harden_it: bool) -> bool {
        let rec = |name: &str| fwd(&dir.join(name));
        let mut cmd = test_git();
        // Ambient, as the daemon could inherit it; each records being run. HOME and XDG point at the test folder so
        // the real user's helpers (a keyring that pops a window) are never reached.
        cmd.env("HOME", dir)
            .env("XDG_CONFIG_HOME", dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", dir.join("git-askpass"))
            .env("SSH_ASKPASS", dir.join("ssh-askpass"))
            .env(
                "GIT_CONFIG_PARAMETERS",
                format!("'credential.helper'='!{}'", rec("param-helper")),
            );
        // What a persona's env sets: inherited helpers cleared, then its own (gh, here one that knows nothing).
        let persona: Vec<(String, String)> = [
            ("GIT_CONFIG_COUNT", "2".to_string()),
            ("GIT_CONFIG_KEY_0", "credential.helper".into()),
            ("GIT_CONFIG_VALUE_0", String::new()),
            ("GIT_CONFIG_KEY_1", "credential.helper".into()),
            ("GIT_CONFIG_VALUE_1", format!("!{}", rec("persona-helper"))),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        if harden_it {
            harden(&mut cmd, &persona);
        } else {
            cmd.envs(persona.iter().map(|(k, v)| (k, v)));
        }
        let mut child = cmd
            .args(["credential", "fill"])
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"protocol=https\nhost=example.invalid\n\n")
            .unwrap();
        child.wait().unwrap().success()
    }

    /// A persona's clone of a file:// repo the user's own gitconfig rewrites to ssh. `GIT_SSH_COMMAND` is a script
    /// that records being run, so no ssh is ever started; `allowed` says whether the persona env is applied.
    fn clone_rewritten_to_ssh(dir: &Path, allowed: bool) -> (bool, String) {
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let ok = test_git()
            .args(["init", "-q"])
            .arg(&src)
            .env("HOME", dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let url = format!("file:///{}", fwd(&src).trim_start_matches('/'));
        std::fs::write(
            dir.join(".gitconfig"),
            format!("[url \"ssh://example.invalid/\"]\n\tinsteadOf = {url}\n"),
        )
        .unwrap();
        script(
            &dir.join("ssh"),
            &format!("touch '{}'\nexit 1", dir.join("ssh-called").display()),
        );
        let row = crate::store::PersonaRow {
            id: "p".into(),
            name: "p".into(),
            host: "github.com".into(),
            commit_name: "T".into(),
            commit_email: "t@e".into(),
            github_auth: fleet_api::GithubAuth::Gh,
            ready: true,
            created_at: 0,
        };
        let persona = crate::personas::persona_env(&row, dir, "/nonexistent/gh").vars;
        let mut cmd = test_git();
        cmd.env("HOME", dir)
            .env("XDG_CONFIG_HOME", dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_SSH_COMMAND", fwd(&dir.join("ssh")));
        let env: Vec<(String, String)> = if allowed {
            persona
        } else {
            persona
                .into_iter()
                .filter(|(k, _)| k != "GIT_ALLOW_PROTOCOL")
                .collect()
        };
        harden(&mut cmd, &env);
        let dest = dir.join(if allowed { "dest" } else { "dest-control" });
        let out = cmd
            .args(["clone", "--quiet", "--", &url])
            .arg(&dest)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    #[test]
    fn a_persona_clone_never_turns_into_ssh() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // The control: without the protocol list the user's insteadOf sends git to ssh.
        let (ok, _) = clone_rewritten_to_ssh(dir, false);
        assert!(
            !ok && dir.join("ssh-called").exists(),
            "the rewrite did not reach ssh"
        );
        std::fs::remove_file(dir.join("ssh-called")).unwrap();

        let (ok, stderr) = clone_rewritten_to_ssh(dir, true);
        assert!(!ok);
        assert!(stderr.contains("transport 'ssh' not allowed"), "{stderr}");
        assert!(!dir.join("ssh-called").exists(), "ssh was started");
    }

    /// `git credential fill` under a persona's env for `host`, with its gh at a path that needs quoting; true when
    /// that gh was asked.
    fn persona_helper_asked(dir: &Path, host: &str) -> bool {
        let gh_dir = dir.join("my gh");
        std::fs::create_dir_all(&gh_dir).unwrap();
        let marker = dir.join("gh-called");
        let _ = std::fs::remove_file(&marker);
        script(&gh_dir.join("gh"), &format!("touch '{}'", marker.display()));
        let row = crate::store::PersonaRow {
            id: "p".into(),
            name: "p".into(),
            host: "github.com".into(),
            commit_name: "T".into(),
            commit_email: "t@e".into(),
            github_auth: fleet_api::GithubAuth::Gh,
            ready: true,
            created_at: 0,
        };
        let gh = gh_dir.join("gh").display().to_string();
        let env = crate::personas::persona_env(&row, dir, &gh).vars;
        let mut cmd = test_git();
        cmd.env("HOME", dir)
            .env("XDG_CONFIG_HOME", dir)
            .env("GIT_CONFIG_NOSYSTEM", "1");
        harden(&mut cmd, &env);
        let mut child = cmd
            .args(["credential", "fill"])
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("protocol=https\nhost={host}\n\n").as_bytes())
            .unwrap();
        child.wait().unwrap();
        marker.exists()
    }

    #[test]
    fn git_without_a_persona_asks_no_helper() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        script(
            &dir.join("user-helper"),
            &format!("touch '{}'", dir.join("user-called").display()),
        );
        std::fs::write(
            dir.join(".gitconfig"),
            format!(
                "[credential]\n\thelper = !{}\n",
                fwd(&dir.join("user-helper"))
            ),
        )
        .unwrap();
        let fill = |harden_it: bool| {
            let mut cmd = test_git();
            cmd.env("HOME", dir)
                .env("XDG_CONFIG_HOME", dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_ASKPASS", "");
            if harden_it {
                harden(&mut cmd, &[]);
            }
            let mut child = cmd
                .args(["credential", "fill"])
                .current_dir(dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"protocol=https\nhost=example.invalid\n\n")
                .unwrap();
            child.wait().unwrap();
        };
        // The control: the user's helper answers plain git.
        fill(false);
        assert!(dir.join("user-called").exists());
        std::fs::remove_file(dir.join("user-called")).unwrap();
        fill(true);
        assert!(
            !dir.join("user-called").exists(),
            "the user's helper was asked"
        );
    }

    /// An ambient `GIT_CONFIG_COUNT` (an agent's, when the daemon runs inside one) with a helper past the count harden
    /// sets: git reads only the first `GIT_CONFIG_COUNT` entries, so the stale ones never answer.
    #[test]
    fn an_inherited_config_count_adds_no_helper() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        script(
            &dir.join("ambient-helper"),
            &format!("touch '{}'", dir.join("ambient-called").display()),
        );
        let persona: Vec<(String, String)> = [
            ("GIT_CONFIG_COUNT", "2"),
            ("GIT_CONFIG_KEY_0", "credential.helper"),
            ("GIT_CONFIG_VALUE_0", ""),
            ("GIT_CONFIG_KEY_1", "credential.helper"),
            ("GIT_CONFIG_VALUE_1", ""),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let asked = |hardened: Option<&[(String, String)]>| {
            let _ = std::fs::remove_file(dir.join("ambient-called"));
            let mut cmd = test_git();
            cmd.env("HOME", dir)
                .env("XDG_CONFIG_HOME", dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_ASKPASS", "")
                .env("GIT_CONFIG_COUNT", "3")
                .env("GIT_CONFIG_KEY_2", "credential.helper")
                .env(
                    "GIT_CONFIG_VALUE_2",
                    format!("!{}", fwd(&dir.join("ambient-helper"))),
                );
            for i in 0..2 {
                cmd.env(format!("GIT_CONFIG_KEY_{i}"), "credential.helper")
                    .env(format!("GIT_CONFIG_VALUE_{i}"), "");
            }
            if let Some(env) = hardened {
                harden(&mut cmd, env);
            }
            let mut child = cmd
                .args(["credential", "fill"])
                .current_dir(dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"protocol=https\nhost=example.invalid\n\n")
                .unwrap();
            child.wait().unwrap();
            dir.join("ambient-called").exists()
        };
        // The control: plain git reads all three.
        assert!(asked(None));
        assert!(!asked(Some(&[])), "asked without a persona");
        assert!(!asked(Some(&persona)), "asked under a persona");
    }

    #[test]
    fn the_personas_gh_answers_for_its_host_only() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(persona_helper_asked(tmp.path(), "github.com"));
        assert!(!persona_helper_asked(tmp.path(), "example.invalid"));
    }

    #[test]
    fn hardened_git_asks_only_the_personas_helper() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        script(
            &dir.join("persona-helper"),
            &format!("touch '{}'", dir.join("persona-called").display()),
        );
        script(
            &dir.join("param-helper"),
            &format!("touch '{}'", dir.join("param-called").display()),
        );
        for (name, marker) in [
            ("git-askpass", "git-askpass-called"),
            ("ssh-askpass", "ssh-askpass-called"),
        ] {
            script(
                &dir.join(name),
                &format!("touch '{}'\necho x", dir.join(marker).display()),
            );
        }
        let called = |m: &str| dir.join(m).exists();

        // The control: without hardening the ambient helper and askpass both answer.
        assert!(fill(dir, false), "ambient askpass should have answered");
        assert!(called("param-called") && called("git-askpass-called"));
        for m in [
            "persona-called",
            "param-called",
            "git-askpass-called",
            "ssh-askpass-called",
        ] {
            let _ = std::fs::remove_file(dir.join(m));
        }

        assert!(
            !fill(dir, true),
            "nothing may answer but the persona's helper"
        );
        assert!(called("persona-called"));
        for m in ["param-called", "git-askpass-called", "ssh-askpass-called"] {
            assert!(!called(m), "{m}");
        }
    }

    #[test]
    fn is_rejected_matches_a_remote_that_moved_on() {
        let push_error = |stderr: &str| GitError::Git {
            args: "push --quiet origin refs/heads/main:refs/heads/main".into(),
            stderr: stderr.into(),
        };
        for stderr in [
            " ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs",
            " ! [rejected]        main -> main (non-fast-forward)",
            "remote: error: cannot lock ref 'refs/heads/main': is at 61ca but expected 711b\n \
             ! [remote rejected] main -> main (incorrect old value provided)",
            " ! [remote rejected] main -> main (cannot lock ref 'refs/heads/main')",
        ] {
            assert!(is_rejected(&push_error(stderr)), "{stderr}");
        }
        for stderr in [
            "fatal: '/gone/origin.git' does not appear to be a git repository",
            "fatal: Authentication failed for 'https://example.com/notes.git/'",
        ] {
            assert!(!is_rejected(&push_error(stderr)), "{stderr}");
        }
        assert!(!is_rejected(&GitError::Io(std::io::Error::other(
            "[rejected]"
        ))));
    }
}
