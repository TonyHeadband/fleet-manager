//! One git worktree and branch per agent. Shells out to `git` with argv, never through a shell.
use crate::personas::ambient_vars;
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
    git_env(dir, args, &[]).await
}

/// What every daemon git call runs with: no ambient token or git credential setup, never a prompt (on the terminal
/// or through an askpass program, which on a desktop pops up a window), then `env` (a persona's) on top. What
/// reaches a remote is only what the persona's credential helper hands out.
pub fn harden(cmd: &mut std::process::Command, env: &[(String, String)]) {
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
    let mut cmd = Command::new("git");
    harden(cmd.as_std_mut(), env);
    let out = cmd
        .arg("-C")
        .arg(dir)
        .args(args)
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
pub async fn clone(url: &str, dest: &Path, env: &[(String, String)]) -> Result<(), GitError> {
    let parent = dest.parent().expect("clone path has a parent");
    let dest_arg = dest.to_string_lossy().into_owned();
    // `--` so a URL can never be read as an option.
    git_env(parent, &["clone", "--quiet", "--", url, &dest_arg], env)
        .await
        .map(|_| ())
}

pub async fn fetch(repo: &Path, env: &[(String, String)]) -> Result<(), GitError> {
    git_env(repo, &["fetch", "--quiet", "origin"], env)
        .await
        .map(|_| ())
}

/// The URL `remote` points at, or None when the repo has no such remote.
pub async fn remote_url(repo: &Path, remote: &str) -> Result<Option<String>, GitError> {
    if !has_remote(repo, remote).await? {
        return Ok(None);
    }
    git(repo, &["remote", "get-url", remote]).await.map(Some)
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
    let out = cmd
        .arg("-C")
        .arg(dir)
        .args(args)
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

#[cfg(test)]
mod tests {
    use super::harden;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// `git credential fill` for a host nobody serves: it reads config and runs helpers and askpass, and never
    /// touches the network.
    fn fill(dir: &Path, harden_it: bool) -> bool {
        let rec = |name: &str| dir.join(name).display().to_string();
        let mut cmd = Command::new("git");
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
        let ok = Command::new("git")
            .args(["init", "-q"])
            .arg(&src)
            .env("HOME", dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let url = format!("file://{}", src.display());
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
        let mut cmd = Command::new("git");
        cmd.env("HOME", dir)
            .env("XDG_CONFIG_HOME", dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_SSH_COMMAND", dir.join("ssh"));
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
        let mut cmd = Command::new("git");
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
                dir.join("user-helper").display()
            ),
        )
        .unwrap();
        let fill = |harden_it: bool| {
            let mut cmd = Command::new("git");
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
}
