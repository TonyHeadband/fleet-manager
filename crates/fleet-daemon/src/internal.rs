//! A product's internal resources: a private git repo the daemon owns, local-only unless connected to a remote. An orchestrated task keeps
//! its designs and plans on a `task/<name>` branch checked out in a worktree; specialists read a detached
//! snapshot; the branch merges into `main` when the task opens its PR. Nothing here reaches a product repo.
use crate::git::{self, GitError};
use crate::repos::redact_url;
use fleet_api::InternalCommit;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Largest file `read` returns.
const MAX_READ: u64 = 1_048_576;
const MAX_HISTORY: usize = 200;
/// Pushes of `main` the remote rejects for having moved on, before giving up.
const PUSH_ROUNDS: usize = 3;
const ORIGIN_MAIN: &str = "refs/remotes/origin/main";

/// The layout every internal repo starts with, as `(repo path, text)`.
pub const SKELETON: &[(&str, &str)] = &[
    ("README.md", include_str!("../builtin/internal/README.md")),
    (
        "specs/README.md",
        include_str!("../builtin/internal/specs.md"),
    ),
    (
        "plans/README.md",
        include_str!("../builtin/internal/plans.md"),
    ),
    (
        "tasks/README.md",
        include_str!("../builtin/internal/tasks.md"),
    ),
    (
        "docs/README.md",
        include_str!("../builtin/internal/docs.md"),
    ),
    (
        "scripts/README.md",
        include_str!("../builtin/internal/scripts.md"),
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum InternalError {
    #[error("{0} not found")]
    NotFound(String),
    #[error("bad path: {0}")]
    BadPath(String),
    #[error("file is larger than 1 MiB")]
    TooLarge,
    #[error("not a text file")]
    NotText,
    /// A failed clone, fetch or push, with git's message and the URL redacted.
    #[error("{0}")]
    Remote(String),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged,
    UpToDate,
    Conflict(Vec<String>),
    /// Stopped with nothing in conflict, with git's message; aborted like a conflict.
    Stopped(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeReport {
    pub merge: MergeOutcome,
    pub push: PushOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushOutcome {
    /// No `origin`, or no environment to reach it with.
    LocalOnly,
    Pushed,
    /// With git's message; local `main` keeps its commits for the next push.
    Failed(String),
    /// Local `main`'s unpushed commits conflicted with the remote's: they were pushed to `branch`, local `main`
    /// restarted from the remote's, and the merge and push happened on that.
    Parked {
        branch: String,
    },
    /// The merge stopped; the task branch was pushed for the user to resolve.
    TaskBranchPushed,
}

/// Where `catch_up` left local `main`.
enum CatchUp {
    /// Even with the remote's, or ahead of it.
    Ready,
    Parked(String),
    /// Not synced, with git's message; local `main` is as it was.
    Unreachable(String),
}

/// A persona's git environment: how calls to the remote authenticate.
pub type GitEnv<'a> = &'a [(String, String)];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteState {
    /// `origin`, credentials redacted; None while local-only.
    pub url: Option<String>,
    /// Commits on `main` the remote does not have yet.
    pub unpushed: u32,
    /// The last push failure, until a push succeeds.
    pub error: Option<String>,
}

pub struct Internal {
    root: PathBuf,
    push_errors: Mutex<HashMap<String, String>>,
    // One per product: git writes under lock files in `.git`, so two worktree adds or a merge racing an add on
    // one repo fail with "could not lock".
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Internal {
    pub fn new(root: PathBuf) -> Self {
        Internal {
            root,
            push_errors: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// An empty folder beside the repos (in the state dir) that daemon commits point `core.hooksPath` at.
    fn no_hooks(&self) -> PathBuf {
        self.root.with_file_name("no-hooks")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn repo_dir(&self, product: &str) -> PathBuf {
        self.root.join(product)
    }

    pub fn task_branch(task_name: &str) -> String {
        format!("task/{task_name}")
    }

    async fn lock(&self, product: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .locks
            .lock()
            .expect("internal locks poisoned")
            .entry(product.to_string())
            .or_default()
            .clone();
        lock.lock_owned().await
    }

    /// The repo of a product that has one.
    async fn existing(&self, product: &str) -> Result<PathBuf, InternalError> {
        let dir = self.repo_dir(product);
        if tokio::fs::try_exists(dir.join(".git")).await? {
            Ok(dir)
        } else {
            Err(InternalError::NotFound(format!(
                "internal repo of {product}"
            )))
        }
    }

    /// Creates the repo on first use: `main` with one commit holding the `SKELETON`. Repeatable.
    pub async fn ensure(&self, product: &str) -> Result<PathBuf, InternalError> {
        let _guard = self.lock(product).await;
        self.ensure_locked(product).await
    }

    async fn ensure_locked(&self, product: &str) -> Result<PathBuf, InternalError> {
        let dir = self.repo_dir(product);
        // Also finishes a repo whose first commit a crash cut off.
        git::init_main(&dir, &self.no_hooks(), "Start internal resources", SKELETON).await?;
        Ok(dir)
    }

    pub async fn branch_exists(&self, product: &str, branch: &str) -> Result<bool, InternalError> {
        match self.existing(product).await {
            Ok(dir) => Ok(git::branch_exists(&dir, branch).await?),
            Err(InternalError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Checks out `task/<task_name>` at `dest`. A new branch starts from `main`, first brought up to the remote's
    /// when connected and `env` is given; a failed fetch is no failure but the returned line for the task log.
    pub async fn attach_task(
        &self,
        product: &str,
        task_name: &str,
        dest: &Path,
        env: Option<GitEnv<'_>>,
    ) -> Result<Option<String>, InternalError> {
        let _guard = self.lock(product).await;
        let repo = self.ensure_locked(product).await?;
        let branch = Self::task_branch(task_name);
        if git::worktree_listed(&repo, dest).await? {
            if dest.exists() {
                return Ok(None);
            }
            // Restart path: the folder is gone but git still lists it, which would refuse the branch as checked out.
            git::forget_worktree(&repo, dest).await?;
        }
        let mut warning = None;
        if !git::branch_exists(&repo, &branch).await? {
            if let Some((url, env)) = remote_of(&repo, env).await? {
                match remote(git::fetch_branch(&repo, "main", env)).await {
                    Ok(()) => {
                        // Diverged (unpushed commits): `main` stays as it is, and the next merge reconciles it.
                        git::merge_ff_only(&repo, ORIGIN_MAIN).await?;
                    }
                    Err(e) => {
                        warning = Some(format!(
                            "internal notes: cannot fetch the remote ({}); starting from the local copy",
                            remote_error(&url, e)
                        ))
                    }
                }
            }
            git::create_branch(&repo, &branch, "main").await?;
        }
        git::worktree_add_branch(&repo, dest, &branch).await?;
        Ok(warning)
    }

    /// Undoes `attach_task` for a task that never came to exist: the worktree if one is checked out, then the
    /// branch with whatever was committed on it. Absent is fine.
    pub async fn delete_task_branch(
        &self,
        product: &str,
        task_name: &str,
    ) -> Result<(), InternalError> {
        let _guard = self.lock(product).await;
        let repo = match self.existing(product).await {
            Ok(r) => r,
            Err(InternalError::NotFound(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let branch = Self::task_branch(task_name);
        if let Some(path) = git::worktree_of_branch(&repo, &branch).await? {
            git::remove_worktree_at(&repo, &path).await?;
            git::forget_worktree(&repo, &path).await?;
        }
        git::delete_branch(&repo, &branch).await?;
        Ok(())
    }

    /// Checks out the tip of `task/<task_name>` at `dest`, detached, and returns that commit.
    pub async fn snapshot(
        &self,
        product: &str,
        task_name: &str,
        dest: &Path,
    ) -> Result<String, InternalError> {
        let _guard = self.lock(product).await;
        let repo = self.existing(product).await?;
        let branch = Self::task_branch(task_name);
        let tip = git::branch_tip(&repo, &branch)
            .await?
            .ok_or(InternalError::NotFound(format!("branch {branch}")))?;
        git::worktree_add_detached(&repo, dest, &tip).await?;
        Ok(tip)
    }

    /// Removes a checkout; its branch and commits stay.
    pub async fn remove_worktree(&self, product: &str, path: &Path) -> Result<(), InternalError> {
        let _guard = self.lock(product).await;
        let repo = self.existing(product).await?;
        git::remove_worktree_at(&repo, path).await?;
        git::forget_worktree(&repo, path).await?;
        Ok(())
    }

    pub async fn is_dirty(&self, worktree: &Path) -> Result<bool, InternalError> {
        Ok(git::is_dirty(worktree).await?)
    }

    /// Merges the task branch into `main`. A merge that stops (a conflict, or anything else) is aborted, leaving
    /// `main` as it was. When connected and `env` is given, `main` is first brought up to the remote's and then
    /// pushed (see `sync`); a stopped merge pushes the task branch instead. `now` stamps a parked branch.
    pub async fn merge_task(
        &self,
        product: &str,
        task_name: &str,
        env: Option<GitEnv<'_>>,
        now: i64,
    ) -> Result<MergeReport, InternalError> {
        let _guard = self.lock(product).await;
        let repo = self.existing(product).await?;
        let branch = Self::task_branch(task_name);
        if !git::branch_exists(&repo, &branch).await? {
            return Err(InternalError::NotFound(format!("branch {branch}")));
        }
        let Some((url, env)) = remote_of(&repo, env).await? else {
            return Ok(MergeReport {
                merge: self.merge_into_main(&repo, &branch).await?,
                push: PushOutcome::LocalOnly,
            });
        };
        let (merge, push) = self.sync(&repo, &url, env, Some(&branch), now).await?;
        self.record_push(product, &push);
        Ok(MergeReport {
            merge: merge.expect("sync merges the task it is given"),
            push,
        })
    }

    /// Pushes what an earlier push left on local `main`, catching up with the remote first as `merge_task` does.
    /// `Pushed` also when nothing was pending.
    pub async fn push_pending(
        &self,
        product: &str,
        env: GitEnv<'_>,
        now: i64,
    ) -> Result<PushOutcome, InternalError> {
        let _guard = self.lock(product).await;
        let repo = match self.existing(product).await {
            Ok(r) => r,
            Err(InternalError::NotFound(_)) => return Ok(PushOutcome::LocalOnly),
            Err(e) => return Err(e),
        };
        let Some((url, env)) = remote_of(&repo, Some(env)).await? else {
            return Ok(PushOutcome::LocalOnly);
        };
        let (_, push) = self.sync(&repo, &url, env, None, now).await?;
        self.record_push(product, &push);
        Ok(push)
    }

    fn record_push(&self, product: &str, push: &PushOutcome) {
        match push {
            PushOutcome::Failed(why) => self.set_push_error(product, why.clone()),
            PushOutcome::Pushed | PushOutcome::Parked { .. } => self.clear_push_error(product),
            PushOutcome::LocalOnly | PushOutcome::TaskBranchPushed => {}
        }
    }

    /// Catches up with the remote, merges `task` (when given) into `main` and pushes `main`, back to the fetch
    /// while the remote rejects the push for having moved on. When the remote cannot be reached the merge still
    /// happens locally. Returns the task's merge outcome and the push's.
    async fn sync(
        &self,
        repo: &Path,
        url: &str,
        env: GitEnv<'_>,
        task: Option<&str>,
        now: i64,
    ) -> Result<(Option<MergeOutcome>, PushOutcome), InternalError> {
        let mut merge = None;
        let mut parked = None;
        let mut rejected = String::new();
        for _ in 0..PUSH_ROUNDS {
            let caught_up = self.catch_up(repo, url, env, now).await?;
            if let CatchUp::Parked(side) = &caught_up {
                parked = Some(side.clone());
            }
            if let Some(task) = task {
                let outcome = self.merge_into_main(repo, task).await?;
                if matches!(
                    outcome,
                    MergeOutcome::Conflict(_) | MergeOutcome::Stopped(_)
                ) {
                    let push = match caught_up {
                        CatchUp::Unreachable(why) => PushOutcome::Failed(why),
                        _ => match remote(git::push_branch(repo, task, task, env)).await {
                            Ok(()) => PushOutcome::TaskBranchPushed,
                            Err(e) => PushOutcome::Failed(remote_error(url, e).to_string()),
                        },
                    };
                    return Ok((Some(outcome), push));
                }
                // A later round finds the task already merged, unless a park restarted `main` without it.
                if merge.is_none() || outcome == MergeOutcome::Merged {
                    merge = Some(outcome);
                }
            }
            if let CatchUp::Unreachable(why) = caught_up {
                return Ok((merge, PushOutcome::Failed(why)));
            }
            match remote(git::push_branch(repo, "main", "main", env)).await {
                Ok(()) => {
                    let push = match parked {
                        Some(branch) => PushOutcome::Parked { branch },
                        None => PushOutcome::Pushed,
                    };
                    return Ok((merge, push));
                }
                Err(e) if git::is_rejected(&e) => rejected = remote_error(url, e).to_string(),
                Err(e) => {
                    return Ok((merge, PushOutcome::Failed(remote_error(url, e).to_string())))
                }
            }
        }
        Ok((merge, PushOutcome::Failed(rejected)))
    }

    /// Fetches the remote's `main` and brings local `main` up to it: a fast-forward, or a merge when local `main`
    /// holds commits an earlier push did not deliver. If that merge stops, those commits are pushed to a side
    /// branch `fleet/unpushed-<stamp>` and local `main` restarts from the remote's.
    async fn catch_up(
        &self,
        repo: &Path,
        url: &str,
        env: GitEnv<'_>,
        now: i64,
    ) -> Result<CatchUp, InternalError> {
        if let Err(e) = remote(git::fetch_branch(repo, "main", env)).await {
            return Ok(CatchUp::Unreachable(remote_error(url, e).to_string()));
        }
        if git::merge_ff_only(repo, ORIGIN_MAIN).await? {
            return Ok(CatchUp::Ready);
        }
        if git::merge_no_ff(repo, &self.no_hooks(), ORIGIN_MAIN).await? == git::Merge::Done {
            return Ok(CatchUp::Ready);
        }
        git::merge_abort(repo).await?;
        let side = format!("fleet/unpushed-{}", stamp(now));
        if let Err(e) = remote(git::push_branch(repo, "main", &side, env)).await {
            return Ok(CatchUp::Unreachable(remote_error(url, e).to_string()));
        }
        // Safe only now: every commit the reset drops from local `main` is on the remote's side branch.
        git::reset_hard(repo, ORIGIN_MAIN).await?;
        Ok(CatchUp::Parked(side))
    }

    /// `merge --no-ff` of `branch` into `main`, aborted when it stops.
    async fn merge_into_main(
        &self,
        repo: &Path,
        branch: &str,
    ) -> Result<MergeOutcome, InternalError> {
        let before = git::head_commit(repo).await?;
        if let git::Merge::Stopped(reason) =
            git::merge_no_ff(repo, &self.no_hooks(), &format!("refs/heads/{branch}")).await?
        {
            let paths = git::conflicted_paths(repo).await?;
            git::merge_abort(repo).await?;
            return Ok(if paths.is_empty() {
                MergeOutcome::Stopped(reason)
            } else {
                MergeOutcome::Conflict(paths)
            });
        }
        Ok(if git::head_commit(repo).await? == before {
            MergeOutcome::UpToDate
        } else {
            MergeOutcome::Merged
        })
    }

    /// `main` first, then the task branches by name.
    pub async fn branches(&self, product: &str) -> Result<Vec<String>, InternalError> {
        let repo = self.existing(product).await?;
        let mut names = git::local_branches(&repo).await?;
        names.retain(|b| b == "main" || b.starts_with("task/"));
        names.sort_by(|a, b| (a != "main", a).cmp(&(b != "main", b)));
        Ok(names)
    }

    pub async fn files(&self, product: &str, branch: &str) -> Result<Vec<String>, InternalError> {
        let (repo, rev) = self.rev(product, branch).await?;
        Ok(git::ls_tree_names(&repo, &rev).await?)
    }

    pub async fn read(
        &self,
        product: &str,
        branch: &str,
        path: &str,
    ) -> Result<String, InternalError> {
        let path = clean_path(path)?;
        let (repo, rev) = self.rev(product, branch).await?;
        // The mode comes from the tree, not the filesystem: a symlink committed to the branch must not be followed
        // out of the repo.
        let (mode, kind) = git::tree_entry(&repo, &rev, &path)
            .await?
            .ok_or_else(|| InternalError::NotFound(format!("{path} on {branch}")))?;
        if mode == "120000" {
            return Err(InternalError::BadPath(format!("{path} is a symlink")));
        }
        if kind != "blob" {
            return Err(InternalError::NotFound(format!("{path} on {branch}")));
        }
        if git::blob_size(&repo, &rev, &path).await? > MAX_READ {
            return Err(InternalError::TooLarge);
        }
        let bytes = git::blob_bytes(&repo, &rev, &path).await?;
        String::from_utf8(bytes).map_err(|_| InternalError::NotText)
    }

    /// Newest first; `limit` is capped at 200.
    pub async fn history(
        &self,
        product: &str,
        branch: &str,
        path: Option<&str>,
        limit: usize,
    ) -> Result<Vec<InternalCommit>, InternalError> {
        let path = path.map(clean_path).transpose()?;
        let (repo, rev) = self.rev(product, branch).await?;
        let entries =
            git::log_entries(&repo, &rev, path.as_deref(), limit.min(MAX_HISTORY)).await?;
        Ok(entries
            .into_iter()
            .map(|e| InternalCommit {
                sha: e.sha,
                at: e.at,
                author: e.author,
                subject: e.subject,
            })
            .collect())
    }

    /// Clones `url` into a temp dir under root; an empty remote is seeded (SKELETON) and pushed. Only then an
    /// existing repo moves aside to `<product>.local-<stamp>` (UTC, from `now` in unix seconds), and the clone takes
    /// its place. Returns the aside path.
    pub async fn connect(
        &self,
        product: &str,
        url: &str,
        env: GitEnv<'_>,
        now: i64,
    ) -> Result<Option<PathBuf>, InternalError> {
        let _guard = self.lock(product).await;
        tokio::fs::create_dir_all(&self.root).await?;
        let tmp = self.root.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
        let result = self.swap_in_clone(product, url, env, now, &tmp).await;
        match &result {
            Ok(_) => self.clear_push_error(product),
            Err(_) => {
                let _ = tokio::fs::remove_dir_all(&tmp).await;
            }
        }
        result
    }

    async fn swap_in_clone(
        &self,
        product: &str,
        url: &str,
        env: GitEnv<'_>,
        now: i64,
        tmp: &Path,
    ) -> Result<Option<PathBuf>, InternalError> {
        git::clone(url, tmp, env)
            .await
            .map_err(|e| remote_error(url, e))?;
        self.settle_main(tmp, url, env).await?;
        let dir = self.repo_dir(product);
        let aside = if tokio::fs::try_exists(&dir).await? {
            let aside = self.root.join(format!("{product}.local-{}", stamp(now)));
            tokio::fs::rename(&dir, &aside).await?;
            Some(aside)
        } else {
            None
        };
        if let Err(e) = tokio::fs::rename(tmp, &dir).await {
            if let Some(aside) = &aside {
                let _ = tokio::fs::rename(aside, &dir).await;
            }
            return Err(e.into());
        }
        Ok(aside)
    }

    /// Leaves the clone on `main` tracking `origin/main`; an empty remote gets a `main` with the skeleton, pushed.
    async fn settle_main(
        &self,
        clone: &Path,
        url: &str,
        env: GitEnv<'_>,
    ) -> Result<(), InternalError> {
        if git::rev_parse_commit(clone, "refs/remotes/origin/main")
            .await
            .is_ok()
        {
            if git::current_branch(clone).await?.as_deref() != Some("main") {
                git::checkout(clone, "main").await?;
            }
            return Ok(());
        }
        if !git::local_branches(clone).await?.is_empty() {
            return Err(InternalError::Remote(format!(
                "{} has no main branch",
                redact_url(url)
            )));
        }
        git::set_head(clone, "main").await?;
        git::init_main(
            clone,
            &self.no_hooks(),
            "Start internal resources",
            SKELETON,
        )
        .await?;
        git::push_branch(clone, "main", "main", env)
            .await
            .map_err(|e| remote_error(url, e))?;
        git::set_upstream(clone, "main").await?;
        Ok(())
    }

    /// Removes `origin`; the repo and its history stay, local-only from now on.
    pub async fn disconnect(&self, product: &str) -> Result<(), InternalError> {
        let _guard = self.lock(product).await;
        let repo = match self.existing(product).await {
            Ok(r) => r,
            Err(InternalError::NotFound(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        if git::has_remote(&repo, "origin").await? {
            git::remove_remote(&repo, "origin").await?;
        }
        self.clear_push_error(product);
        Ok(())
    }

    pub async fn remote_state(&self, product: &str) -> Result<RemoteState, InternalError> {
        let error = self
            .push_errors
            .lock()
            .expect("push errors poisoned")
            .get(product)
            .cloned();
        let local_only = RemoteState {
            url: None,
            unpushed: 0,
            error: None,
        };
        let repo = match self.existing(product).await {
            Ok(r) => r,
            Err(InternalError::NotFound(_)) => return Ok(local_only),
            Err(e) => return Err(e),
        };
        let Some(url) = git::configured_remote_url(&repo, "origin").await? else {
            return Ok(local_only);
        };
        Ok(RemoteState {
            url: Some(redact_url(&url)),
            unpushed: git::rev_count(&repo, "origin/main..main").await?,
            error,
        })
    }

    /// Records why the last push failed; `remote_state` reports it until `clear_push_error`.
    pub fn set_push_error(&self, product: &str, message: String) {
        self.push_errors
            .lock()
            .expect("push errors poisoned")
            .insert(product.to_string(), message);
    }

    pub fn clear_push_error(&self, product: &str) {
        self.push_errors
            .lock()
            .expect("push errors poisoned")
            .remove(product);
    }

    pub async fn delete(&self, product: &str) -> Result<(), InternalError> {
        let _guard = self.lock(product).await;
        match tokio::fs::remove_dir_all(self.repo_dir(product)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// The repo and the full ref of `branch`, or NotFound naming whichever is missing.
    async fn rev(&self, product: &str, branch: &str) -> Result<(PathBuf, String), InternalError> {
        let repo = self.existing(product).await?;
        if !git::branch_exists(&repo, branch).await? {
            return Err(InternalError::NotFound(format!("branch {branch}")));
        }
        Ok((repo, format!("refs/heads/{branch}")))
    }
}

/// A failed remote call as `InternalError::Remote`: git's stderr, where the URL (which may carry credentials)
/// shows redacted. Anything that is not git's own failure stays a `GitError`.
fn remote_error(url: &str, e: GitError) -> InternalError {
    match e {
        GitError::Git { stderr, .. } => {
            InternalError::Remote(stderr.replace(url, &redact_url(url)))
        }
        e => e.into(),
    }
}

/// `origin`'s URL and the environment to reach it with, when the repo is connected and there is one.
async fn remote_of<'a>(
    repo: &Path,
    env: Option<GitEnv<'a>>,
) -> Result<Option<(String, GitEnv<'a>)>, InternalError> {
    let Some(env) = env else {
        return Ok(None);
    };
    Ok(git::configured_remote_url(repo, "origin")
        .await?
        .map(|url| (url, env)))
}

/// A call to the remote, made under the product's lock, so it gets `git::REMOTE_TIMEOUT`.
async fn remote<T>(
    call: impl std::future::Future<Output = Result<T, GitError>>,
) -> Result<T, GitError> {
    tokio::time::timeout(git::REMOTE_TIMEOUT, call)
        .await
        .unwrap_or_else(|_| {
            Err(GitError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the remote did not answer in time",
            )))
        })
}

/// `YYYYMMDD-HHMMSS` in UTC.
fn stamp(unix: i64) -> String {
    fleet_log::format_time(unix)
        .replace(['-', ':'], "")
        .replace('T', "-")
        .trim_end_matches('Z')
        .to_string()
}

/// A relative path with plain components only, rebuilt with `/` so git sees exactly what was checked.
fn clean_path(path: &str) -> Result<String, InternalError> {
    let mut parts = Vec::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(p) => parts.push(
                p.to_str()
                    .ok_or_else(|| InternalError::BadPath(path.to_string()))?,
            ),
            _ => return Err(InternalError::BadPath(path.to_string())),
        }
    }
    if parts.is_empty() {
        return Err(InternalError::BadPath(path.to_string()));
    }
    Ok(parts.join("/"))
}
