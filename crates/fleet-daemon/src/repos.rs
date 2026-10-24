//! Remembered repos: resolves a name, path or git URL, keeps one base clone per URL and hangs per-project worktrees off it.
use crate::config::{expand, is_safe_dir_name};
use crate::git::{self, GitError, Worktree};
use crate::store::{RepoRow, Store, StoreError};
use fleet_api::RepoSource;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Name(String),
    Path(PathBuf),
    Url(String),
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
                    eprintln!("fleet-daemon: warning: cannot remove the half-made clone {}: {err}", e.path().display());
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
    pub async fn resolve(&self, input: &str) -> Result<RepoRow, RepoError> {
        if let Some(row) = self.store.repo(input).await? {
            return Ok(row);
        }
        let source = classify(input, &self.home)?;
        let _guard = self.clone_lock.lock().await;
        match source {
            Source::Path(p) => self.resolve_path(&p).await,
            Source::Url(u) => self.resolve_url(&u).await,
            Source::Name(n) => Err(RepoError::Unrecognised(n)),
        }
    }

    async fn resolve_path(&self, path: &Path) -> Result<RepoRow, RepoError> {
        let path = path.canonicalize().map_err(|_| RepoError::NotARepo(path.to_path_buf()))?;
        let path = match git::main_root(&path).await? {
            None => return Err(RepoError::NotARepo(path)),
            Some(git::MainRoot::Bare) => return Err(RepoError::Bare(path)),
            Some(git::MainRoot::Worktree(root)) => root.canonicalize()?,
        };
        // After resolution: a worktree under the projects root maps to its main repo, which is fine.
        let root = self.projects_root.canonicalize().unwrap_or_else(|_| self.projects_root.clone());
        if path.starts_with(&root) {
            return Err(RepoError::InsideProjects(path));
        }
        let location = path.to_string_lossy().into_owned();
        if let Some(row) = self.store.repo_by_location(&location).await? {
            return Ok(row);
        }
        let name = self.free_name_in_store(&derive_name(&Source::Path(path.clone()))).await?;
        let row = RepoRow { name, source: RepoSource::Path, location, base: path };
        self.store.insert_repo(&row).await?;
        Ok(row)
    }

    /// Remembers a repo an agent `git init`ed inside its project folder. Unlike `resolve`, the projects root is
    /// allowed here: that repo has no other home. It must be its own toplevel (no subdir, worktree or bare repo).
    pub async fn remember_owned(&self, path: &Path) -> Result<RepoRow, RepoError> {
        let path = path.canonicalize().map_err(|_| RepoError::NotARepo(path.to_path_buf()))?;
        match git::main_root(&path).await? {
            Some(git::MainRoot::Worktree(root)) if root.canonicalize()? == path => {}
            Some(git::MainRoot::Bare) => return Err(RepoError::Bare(path)),
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
        Ok(row)
    }

    /// Fetch (URL always; path only with an `origin` remote; failure becomes a warning), then a worktree at `dest`
    /// on `branch` from `origin/HEAD` if it exists, else HEAD.
    pub async fn attach(&self, repo: &RepoRow, dest: &Path, branch: &str) -> Result<(Worktree, Option<String>), RepoError> {
        let (prefix, slug) = branch
            .rsplit_once('/')
            .ok_or_else(|| RepoError::Unrecognised(format!("branch {branch:?} must look like prefix/slug")))?;
        let _base = self.lock_base(&repo.base).await;
        let should_fetch = repo.source == RepoSource::Url || git::has_remote(&repo.base, "origin").await?;
        let warning = if should_fetch {
            git::fetch(&repo.base).await.err().map(|e| format!("could not fetch {}: {e}", repo.name))
        } else {
            None
        };
        // Never adopt a branch left by an earlier project (Kill keeps branches): its commits are not this project's.
        if git::branch_exists(&repo.base, branch).await? {
            return Err(RepoError::BranchExists { branch: branch.to_string(), repo: repo.name.clone() });
        }
        let start = git::default_start(&repo.base).await?;
        let wt = git::create(&repo.base, prefix, slug, dest, &start).await?;
        Ok((wt, warning))
    }
}
