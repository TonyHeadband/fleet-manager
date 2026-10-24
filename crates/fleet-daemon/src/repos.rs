//! Remembered repos: resolves a name, path or git URL, keeps one base clone per URL and hangs per-project worktrees off it.
use crate::config::{expand, is_safe_dir_name};
use crate::git::{self, GitError, Worktree};
use crate::store::{RepoRow, Store, StoreError};
use fleet_api::RepoSource;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Name(String),
    Path(PathBuf),
    Url(String),
}

/// Where a new worktree begins, decided when the source is resolved and read again at attach time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// HEAD of the repo's main checkout (a remembered path repo).
    MainHead,
    /// HEAD of this checkout: the one a path was typed or picked in.
    Checkout(PathBuf),
    /// Fetch, then the remote default branch (a URL repo).
    OriginHead,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub row: RepoRow,
    pub start: Start,
}

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("{0:?} is not a remembered repo name, a path (/abs, ~/rel or an existing directory) or a git URL (scheme://… or user@host:path)")]
    Unrecognised(String),
    #[error("{0} is not a git repository")]
    NotARepo(PathBuf),
    #[error("{0} is inside the projects root; pick the repo it was made from")]
    InsideProjects(PathBuf),
    #[error("{0} is a bare repository; bare repositories are not supported; pass a clone")]
    Bare(PathBuf),
    #[error("branch {branch} already exists in {repo}; delete or rename it, then try again")]
    BranchExists { branch: String, repo: String },
    #[error("cannot start from {path}: {reason}")]
    Start { path: PathBuf, reason: String },
    #[error("cannot clone {url}: {stderr}")]
    Clone { url: String, stderr: String },
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// `user@host:path` (scp-like): `^[^/@\s]+@[^:\s]+:.+`.
fn is_scp_like(s: &str) -> bool {
    let Some((user, rest)) = s.split_once('@') else { return false };
    let Some((host, path)) = rest.split_once(':') else { return false };
    let bad = |c: char| c.is_whitespace();
    !user.is_empty()
        && !user.contains('/')
        && !user.contains(bad)
        && !host.is_empty()
        && !host.contains(bad)
        && !path.is_empty()
}

/// Path, then URL. A remembered name is checked by `Repos::resolve` before this, since only the store knows it.
pub fn classify(input: &str, home: &Path) -> Result<Source, RepoError> {
    if input.starts_with('/') || input.starts_with('~') || Path::new(input).is_dir() {
        return Ok(Source::Path(expand(input, home)));
    }
    if input.contains("://") || is_scp_like(input) {
        return Ok(Source::Url(input.to_string()));
    }
    Err(RepoError::Unrecognised(input.to_string()))
}

/// What to put in a log line for a repo the user or an agent typed: a URL becomes its derived name, because a
/// clone URL can carry credentials.
pub fn log_name(input: &str) -> String {
    if input.contains("://") || is_scp_like(input) {
        derive_name(&Source::Url(input.to_string()))
    } else {
        input.to_string()
    }
}

/// Dir name or last URL segment (minus `.git`), coerced to something `is_safe_dir_name` accepts.
fn derive_name(source: &Source) -> String {
    let raw = match source {
        Source::Path(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        Source::Url(u) => {
            let last = u.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("");
            last.strip_suffix(".git").unwrap_or(last).to_string()
        }
        Source::Name(n) => n.clone(),
    };
    let safe: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    let safe = safe.trim_start_matches(['.', '-']).to_string();
    if is_safe_dir_name(&safe) { safe } else { "repo".to_string() }
}

pub struct Repos {
    store: Arc<Store>,
    state_dir: PathBuf,
    projects_root: PathBuf,
    home: PathBuf,
    // Held across check-clone-insert so two resolves of one URL clone once.
    clone_lock: tokio::sync::Mutex<()>,
    // One per base: git writes `.git/config` and `FETCH_HEAD` under lock files, so two worktree adds, fetches or
    // removals on the same base at once fail with "could not lock config file".
    base_locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

impl Repos {
    pub fn new(store: Arc<Store>, state_dir: PathBuf, projects_root: PathBuf, home: PathBuf) -> Repos {
        Repos { store, state_dir, projects_root, home, clone_lock: tokio::sync::Mutex::new(()), base_locks: Mutex::new(HashMap::new()) }
    }

    /// Serialises git writes on one base clone; hold it across any worktree add or remove there.
    pub async fn lock_base(&self, base: &Path) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self.base_locks.lock().expect("base locks poisoned").entry(base.to_path_buf()).or_default().clone();
        lock.lock_owned().await
    }

    fn clones_dir(&self) -> PathBuf {
        self.state_dir.join("repos")
    }

    /// Removes the `.tmp-*` folders a clone leaves under `repos/` when the daemon stops during it. Only at
    /// startup, before any clone can be running.
    pub fn sweep_tmp_clones(&self) {
        let Ok(entries) = std::fs::read_dir(self.clones_dir()) else { return };
        for e in entries.flatten() {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && e.file_name().to_string_lossy().starts_with(".tmp-") {
                if let Err(err) = std::fs::remove_dir_all(e.path()) {
                    warn!(path = %e.path().display(), error = %err, "cannot remove the half-made clone");
                }
            }
        }
    }

    /// First free `name`, `name-2`, `name-3`, … in the store and under `repos/`.
    async fn free_name(&self, base: &str) -> Result<String, RepoError> {
        let mut n = 1;
        loop {
            let name = if n == 1 { base.to_string() } else { format!("{base}-{n}") };
            if self.store.repo(&name).await?.is_none() && !self.clones_dir().join(&name).exists() {
                return Ok(name);
            }
            n += 1;
        }
    }

    /// Remembered name, else path, else URL; clones a new URL into `<state_dir>/repos/<name>`; remembers new sources.
    pub async fn resolve(&self, input: &str) -> Result<Resolved, RepoError> {
        if let Some(row) = self.store.repo(input).await? {
            let start = if row.source == RepoSource::Url { Start::OriginHead } else { Start::MainHead };
            return Ok(Resolved { row, start });
        }
        let source = classify(input, &self.home)?;
        let _guard = self.clone_lock.lock().await;
        match source {
            Source::Path(p) => self.resolve_path(&p).await,
            Source::Url(u) => Ok(Resolved { row: self.resolve_url(&u).await?, start: Start::OriginHead }),
            Source::Name(n) => Err(RepoError::Unrecognised(n)),
        }
    }

    async fn resolve_path(&self, path: &Path) -> Result<Resolved, RepoError> {
        let path = path.canonicalize().map_err(|_| RepoError::NotARepo(path.to_path_buf()))?;
        let typed = path.clone();
        let root = match git::blocking(move || fleet_fs::main_root(&typed)).await? {
            None => return Err(RepoError::NotARepo(path)),
            Some(fleet_fs::MainRoot::Bare) => return Err(RepoError::Bare(path)),
            Some(fleet_fs::MainRoot::Worktree(root)) => root.canonicalize()?,
        };
        let typed = path.clone();
        let checkout = git::blocking(move || fleet_fs::toplevel(&typed)).await?.ok_or_else(|| RepoError::NotARepo(path.clone()))?;
        let start = Start::Checkout(checkout.canonicalize().unwrap_or(checkout));
        // After resolution: a worktree under the projects root maps to its main repo, which is fine.
        let projects = self.projects_root.canonicalize().unwrap_or_else(|_| self.projects_root.clone());
        if root.starts_with(&projects) {
            return Err(RepoError::InsideProjects(root));
        }
        let location = root.to_string_lossy().into_owned();
        if let Some(row) = self.store.repo_by_location(&location).await? {
            return Ok(Resolved { row, start });
        }
        let name = self.free_name_in_store(&derive_name(&Source::Path(root.clone()))).await?;
        let row = RepoRow { name, source: RepoSource::Path, location, base: root };
        self.store.insert_repo(&row).await?;
        Ok(Resolved { row, start })
    }

    /// Remembers a repo an agent `git init`ed inside its project folder. Unlike `resolve`, the projects root is
    /// allowed here: that repo has no other home. It must be its own toplevel (no subdir, worktree or bare repo).
    pub async fn remember_owned(&self, path: &Path) -> Result<RepoRow, RepoError> {
        let path = path.canonicalize().map_err(|_| RepoError::NotARepo(path.to_path_buf()))?;
        let typed = path.clone();
        match git::blocking(move || fleet_fs::main_root(&typed)).await? {
            Some(fleet_fs::MainRoot::Worktree(root)) if root.canonicalize()? == path => {}
            Some(fleet_fs::MainRoot::Bare) => return Err(RepoError::Bare(path)),
            _ => return Err(RepoError::NotARepo(path)),
        }
        let _guard = self.clone_lock.lock().await;
        let location = path.to_string_lossy().into_owned();
        if let Some(row) = self.store.repo_by_location(&location).await? {
            return Ok(row);
        }
        let name = self.free_name_in_store(&derive_name(&Source::Path(path.clone()))).await?;
        let row = RepoRow { name, source: RepoSource::Path, location, base: path };
        self.store.insert_repo(&row).await?;
        Ok(row)
    }

    /// Path repos have no clone dir, so only the store can clash.
    async fn free_name_in_store(&self, base: &str) -> Result<String, RepoError> {
        let mut n = 1;
        loop {
            let name = if n == 1 { base.to_string() } else { format!("{base}-{n}") };
            if self.store.repo(&name).await?.is_none() {
                return Ok(name);
            }
            n += 1;
        }
    }

    // Not recorded as a field: a clone URL can carry credentials.
    #[tracing::instrument(skip_all)]
    async fn resolve_url(&self, url: &str) -> Result<RepoRow, RepoError> {
        if let Some(row) = self.store.repo_by_location(url).await? {
            return Ok(row);
        }
        let clones = self.clones_dir();
        std::fs::create_dir_all(&clones)?;
        let name = self.free_name(&derive_name(&Source::Url(url.to_string()))).await?;
        let tmp = clones.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
        if let Err(e) = git::clone(url, &tmp).await {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(match e {
                GitError::Git { stderr, .. } => RepoError::Clone { url: url.to_string(), stderr },
                other => other.into(),
            });
        }
        let base = clones.join(&name);
        if let Err(e) = std::fs::rename(&tmp, &base) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e.into());
        }
        let row = RepoRow { name, source: RepoSource::Url, location: url.to_string(), base: base.clone() };
        if let Err(e) = self.store.insert_repo(&row).await {
            // Nothing remembered, so nothing may be left under repos/.
            let _ = std::fs::remove_dir_all(&base);
            return Err(e.into());
        }
        info!(repo = %row.name, "repo cloned");
        Ok(row)
    }

    /// Resolves the start point, then creates a worktree at `dest` on `branch` from the commit it names, recording
    /// the branch it was on. A URL repo fetches first (failure becomes a
    /// warning) and starts from `origin/HEAD` if it exists, else the base's HEAD; a path repo is never fetched, so
    /// work the user has not pushed is what the agent starts from. An unreadable start fails before anything is
    /// created.
    #[tracing::instrument(skip_all, fields(repo = %repo.name))]
    pub async fn attach(
        &self,
        repo: &RepoRow,
        start: &Start,
        dest: &Path,
        branch: &str,
    ) -> Result<(Worktree, Option<String>), RepoError> {
        let (prefix, slug) = branch
            .rsplit_once('/')
            .ok_or_else(|| RepoError::Unrecognised(format!("branch {branch:?} must look like prefix/slug")))?;
        let _base = self.lock_base(&repo.base).await;
        // Never adopt a branch left by an earlier project (Kill keeps branches): its commits are not this project's.
        if git::branch_exists(&repo.base, branch).await? {
            return Err(RepoError::BranchExists { branch: branch.to_string(), repo: repo.name.clone() });
        }
        let mut warning = None;
        let (commit, from_branch) = match start {
            Start::MainHead => head_of(&repo.base).await?,
            Start::Checkout(p) => {
                if !p.exists() {
                    return Err(RepoError::Start { path: p.clone(), reason: "the folder no longer exists".into() });
                }
                head_of(p).await?
            }
            Start::OriginHead => {
                if let Err(e) = git::fetch(&repo.base).await {
                    warning = Some(format!("could not fetch {}: {e}", repo.name));
                }
                match git::origin_head(&repo.base).await? {
                    Some((commit, name)) => (commit, Some(name)),
                    None => head_of(&repo.base).await?,
                }
            }
        };
        let wt = git::create(&repo.base, prefix, slug, dest, &commit, from_branch).await?;
        Ok((wt, warning))
    }
}

/// HEAD commit and branch of a checkout; an unborn or unreadable one is an error, never an empty start.
async fn head_of(checkout: &Path) -> Result<(String, Option<String>), RepoError> {
    let dir = checkout.to_path_buf();
    let head = match git::blocking(move || fleet_fs::head(&dir)).await {
        Ok(h) => h,
        Err(GitError::Fs(fleet_fs::FsError::NotARepo(_))) => {
            return Err(RepoError::Start { path: checkout.to_path_buf(), reason: "it is no longer a git checkout".into() });
        }
        Err(e) => return Err(e.into()),
    };
    match head.commit {
        Some(commit) => Ok((commit, head.branch)),
        None => Err(RepoError::Start { path: checkout.to_path_buf(), reason: "it has no commit yet".into() }),
    }
}
