//! A product's repos: resolves a name, path or git URL within the product, keeps one clone per persona, product and
//! URL repo, and hangs per-task worktrees off a clone or the repo's local checkout.
use crate::config::{expand, is_safe_dir_name};
use crate::git::{self, GitError, Worktree};
use crate::personas::PersonaEnv;
use crate::store::{url_host, RepoRow, Store, StoreError};
use fleet_api::RepoPick;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Name(String),
    Path(PathBuf),
    Url(String),
}

/// Where a new worktree begins, decided when the repo is resolved and read again at attach time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// Fetch the persona's clone, then the remote default branch.
    OriginHead,
    /// HEAD of this checkout of the repo's local path, as the user left it; never fetched.
    Checkout(PathBuf),
}

/// A repo ready to attach: its row, the checkout worktrees hang off (the persona's clone or the repo's local
/// path), and where the branch starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub row: RepoRow,
    pub base: PathBuf,
    pub start: Start,
}

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("{0:?} is not a repo of this product, a path (/abs, ~/rel or an existing directory) or a git URL (scheme://… or user@host:path)")]
    Unrecognised(String),
    #[error("{0} is not a git repository")]
    NotARepo(PathBuf),
    #[error("{0} is inside the tasks root; pick the repo it was made from")]
    InsideTasks(PathBuf),
    #[error("{0} is a bare repository; bare repositories are not supported; pass a clone")]
    Bare(PathBuf),
    #[error("branch {branch} already exists in {repo}; delete or rename it, then try again")]
    BranchExists { branch: String, repo: String },
    #[error("cannot start from {path}: {reason}")]
    Start { path: PathBuf, reason: String },
    #[error("cannot clone {url}: {stderr}")]
    Clone { url: String, stderr: String },
    #[error("{0} carries credentials; remove them and use an SSH key or a git credential helper (a repo URL is stored, shown in the board and readable by agents)")]
    Credentials(String),
    #[error("{0} is an SSH URL; personas only use HTTPS (use https://…)")]
    Ssh(String),
    #[error("{0} is plain HTTP; personas only use HTTPS (use https://…)")]
    PlainHttp(String),
    #[error("{url} is on {host}, but persona {persona} is for {persona_host}; start the task under a persona for {host}")]
    WrongHost {
        url: String,
        host: String,
        persona: String,
        persona_host: String,
    },
    #[error("{0} has no URL; start from a local checkout")]
    NoUrl(String),
    #[error("{0} has no local path; start from its URL")]
    NoPath(String),
    #[error("no such product: {0}")]
    UnknownProduct(String),
    #[error("{0:?} is not a name a repo can have (letters, digits, `.`, `_` and `-`)")]
    BadName(String),
    #[error("a repo needs a path or a URL")]
    NeedPathOrUrl,
    #[error("{0} already has a URL; remove the repo and add it again to change it")]
    UrlChange(String),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What a URL typed for a product's repo must satisfy before it is stored: no credentials, HTTPS (or `file://`),
/// never SSH. The persona's host is checked at spawn (`check_host`), as a repo has no persona yet.
pub fn check_clone_url(url: &str) -> Result<(), RepoError> {
    if has_credentials(url) {
        return Err(RepoError::Credentials(redact_url(url)));
    }
    let Some((scheme, _)) = url.split_once("://") else {
        return if is_scp_like(url) || is_ssh_shorthand(url) {
            Err(RepoError::Ssh(redact_url(url)))
        } else {
            Err(RepoError::Unrecognised(url.to_string()))
        };
    };
    match scheme.to_ascii_lowercase().as_str() {
        "https" | "file" => Ok(()),
        "ssh" | "git" | "git+ssh" | "ssh+git" => Err(RepoError::Ssh(redact_url(url))),
        "http" => Err(RepoError::PlainHttp(redact_url(url))),
        _ => Err(RepoError::Unrecognised(url.to_string())),
    }
}

/// `user@host:path` (scp-like): `^[^/@\s]+@[^:\s]+:.+`.
fn is_scp_like(s: &str) -> bool {
    let Some((user, rest)) = s.split_once('@') else {
        return false;
    };
    let Some((host, path)) = rest.split_once(':') else {
        return false;
    };
    let bad = |c: char| c.is_whitespace();
    !user.is_empty()
        && !user.contains('/')
        && !user.contains(bad)
        && !host.is_empty()
        && !host.contains(bad)
        && !path.is_empty()
}

/// git's own rule for `host:path` without a scheme: ssh, when no `/` comes before the first `:`.
fn is_ssh_shorthand(s: &str) -> bool {
    !s.contains("://")
        && s.split_once(':')
            .is_some_and(|(host, _)| !host.contains('/'))
}

/// The strict host rule: a persona reaches only its own GitHub host, and only over HTTPS, so its login is the only
/// one git can use. A local path or `file://` needs no login and passes.
pub fn check_host(url: &str, persona: &PersonaEnv) -> Result<(), RepoError> {
    let Some((scheme, rest)) = url.split_once("://") else {
        return if is_scp_like(url) || is_ssh_shorthand(url) {
            Err(RepoError::Ssh(redact_url(url)))
        } else {
            Ok(())
        };
    };
    let scheme = scheme.to_ascii_lowercase();
    let host = match scheme.as_str() {
        "file" => return Ok(()),
        "ssh" | "git" | "git+ssh" | "ssh+git" => return Err(RepoError::Ssh(redact_url(url))),
        // The helper would hand the persona's token to anyone between here and the host.
        "http" => return Err(RepoError::PlainHttp(redact_url(url))),
        "https" => url_host(&format!("{scheme}://{rest}")).unwrap_or_default(),
        // Any other transport is not the persona's host either; named as best it can be.
        _ => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let hostport = authority.rsplit('@').next().unwrap_or("");
            hostport
                .split(':')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
        }
    };
    if scheme == "https" && host == persona.host {
        return Ok(());
    }
    Err(RepoError::WrongHost {
        url: redact_url(url),
        host,
        persona: persona.id.clone(),
        persona_host: persona.host.clone(),
    })
}

/// Path, then URL. A repo name is checked by `Repos::resolve` before this, since only the store knows it.
pub fn classify(input: &str, home: &Path) -> Result<Source, RepoError> {
    if input.starts_with('/') || input.starts_with('~') || Path::new(input).is_dir() {
        return Ok(Source::Path(expand(input, home)));
    }
    if input.contains("://") || is_scp_like(input) {
        return Ok(Source::Url(input.to_string()));
    }
    Err(RepoError::Unrecognised(input.to_string()))
}

/// `(scheme, userinfo, rest after the '@')` of a `scheme://userinfo@host/...` URL; `None` without userinfo.
fn userinfo(url: &str) -> Option<(&str, &str, &str)> {
    let (scheme, after) = url.split_once("://")?;
    let authority_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let at = after[..authority_end].rfind('@')?;
    Some((scheme, &after[..at], &after[at + 1..]))
}

/// A password (`user:pass@`), or any userinfo on http(s)/ftp(s), where a bare token as the user name is how
/// hosts like GitHub take a PAT. An ssh or scp-like login name is not a secret.
pub fn has_credentials(url: &str) -> bool {
    userinfo(url).is_some_and(|(scheme, info, _)| {
        info.contains(':')
            || matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "ftp" | "ftps"
            )
    })
}

/// True when `a` and `b` name the same remote, spelled with or without `.git` or a trailing `/`, or with the
/// scheme or host in another case. SSH spellings never get here: `check_host` refuses them first.
pub fn same_repo_url(a: &str, b: &str) -> bool {
    url_key(a) == url_key(b)
}

fn url_key(url: &str) -> String {
    let url = url.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            format!(
                "{}://{}/{path}",
                scheme.to_ascii_lowercase(),
                authority.to_ascii_lowercase()
            )
        }
        None => url.to_string(),
    }
}

/// `url` without the credentials `has_credentials` finds, for anything shown or stored outside git itself.
pub fn redact_url(url: &str) -> String {
    match userinfo(url) {
        Some((scheme, _, rest)) if has_credentials(url) => format!("{scheme}://{rest}"),
        _ => url.to_string(),
    }
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
        Source::Path(p) => p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        Source::Url(u) => {
            let last = u
                .trim_end_matches('/')
                .rsplit(['/', ':'])
                .next()
                .unwrap_or("");
            last.strip_suffix(".git").unwrap_or(last).to_string()
        }
        Source::Name(n) => n.clone(),
    };
    let safe: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let safe = safe.trim_start_matches(['.', '-']).to_string();
    if is_safe_dir_name(&safe) {
        safe
    } else {
        "repo".to_string()
    }
}

pub struct Repos {
    store: Arc<Store>,
    state_dir: PathBuf,
    tasks_root: PathBuf,
    home: PathBuf,
    // Held across look-up-and-add, so two resolves of one typed path or URL add one repo.
    add_lock: tokio::sync::Mutex<()>,
    // Held across check-and-clone, so two resolves of one URL clone once.
    clone_lock: tokio::sync::Mutex<()>,
    // One per base: git writes `.git/config` and `FETCH_HEAD` under lock files, so two worktree adds, fetches or
    // removals on the same base at once fail with "could not lock config file".
    base_locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

/// How long `behind_origin` waits for the remote; past it there is no warning.
const LS_REMOTE_TIMEOUT: Duration = Duration::from_secs(20);

impl Repos {
    pub fn new(store: Arc<Store>, state_dir: PathBuf, tasks_root: PathBuf, home: PathBuf) -> Repos {
        Repos {
            store,
            state_dir,
            tasks_root,
            home,
            add_lock: tokio::sync::Mutex::new(()),
            clone_lock: tokio::sync::Mutex::new(()),
            base_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Serialises git writes on one base clone; hold it across any worktree add or remove there.
    pub async fn lock_base(&self, base: &Path) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .base_locks
            .lock()
            .expect("base locks poisoned")
            .entry(base.to_path_buf())
            .or_default()
            .clone();
        lock.lock_owned().await
    }

    fn clones_dir(&self) -> PathBuf {
        self.state_dir.join("repos")
    }

    /// Where `persona` keeps its clone of `product`'s URL repo `repo`. Per product too: two products may each have
    /// a repo of one name.
    pub fn clone_dir(&self, persona: &str, product: &str, repo: &str) -> PathBuf {
        self.clones_dir().join(persona).join(product).join(repo)
    }

    /// Removes the `.tmp-*` folders a clone leaves under `repos/` when the daemon stops during it. Only at
    /// startup, before any clone can be running.
    pub fn sweep_tmp_clones(&self) {
        let Ok(entries) = std::fs::read_dir(self.clones_dir()) else {
            return;
        };
        for e in entries.flatten() {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && e.file_name().to_string_lossy().starts_with(".tmp-") {
                if let Err(err) = std::fs::remove_dir_all(e.path()) {
                    warn!(path = %e.path().display(), error = %err, "cannot remove the half-made clone");
                }
            }
        }
    }

    /// First free `name`, `name-2`, `name-3`, … among the product's repos and, for a URL repo, `persona`'s clones
    /// of the product (one left by a repo deleted since would be another repo's).
    async fn free_name(
        &self,
        product: &str,
        base: &str,
        persona: Option<&str>,
    ) -> Result<String, RepoError> {
        let mut n = 1;
        loop {
            let name = if n == 1 {
                base.to_string()
            } else {
                format!("{base}-{n}")
            };
            let cloned = persona.is_some_and(|p| self.clone_dir(p, product, &name).exists());
            if !cloned && self.store.repo(product, &name).await?.is_none() {
                return Ok(name);
            }
            n += 1;
        }
    }

    /// The pick a caller that names no checkout means: a repo with a URL starts from it, a path-only one from its
    /// local path. A path or URL not yet in the product is left to `resolve`.
    pub async fn default_pick(&self, product: &str, input: &str) -> Result<RepoPick, RepoError> {
        let input = input.trim();
        let local = match self.store.repo(product, input).await? {
            Some(RepoRow {
                url: None,
                path: Some(p),
                ..
            }) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        };
        Ok(RepoPick {
            repo: input.to_string(),
            local,
        })
    }

    /// The product's repo `pick.repo` names; a path or URL is added to the product first (a path then starts from
    /// the checkout typed), once it resolves, so a typo or a failed clone adds nothing. `pick.local` None starts
    /// from the URL: the persona's clone of it is made if missing, and the host rule is checked. A local checkout
    /// must be one of the repo's path, whose origin passes the host rule.
    pub async fn resolve(
        &self,
        product: &str,
        pick: &RepoPick,
        persona: &PersonaEnv,
    ) -> Result<Resolved, RepoError> {
        if self.store.product(product).await?.is_none() {
            return Err(RepoError::UnknownProduct(product.to_string()));
        }
        let input = pick.repo.trim();
        let local = pick.local.as_deref().map(|l| expand(l, &self.home));
        if let Some(row) = self.store.repo(product, input).await? {
            return self.resolve_row(row, local, persona).await;
        }
        // Held through the clone, so two resolves of one typed URL clone and add it once.
        let _guard = self.add_lock.lock().await;
        let (row, typed, new) = self.typed_row(product, input, persona).await?;
        let resolved = self.resolve_row(row, local.or(typed), persona).await?;
        if new {
            self.store.upsert_repo(&resolved.row).await?;
        }
        Ok(resolved)
    }

    async fn resolve_row(
        &self,
        row: RepoRow,
        local: Option<PathBuf>,
        persona: &PersonaEnv,
    ) -> Result<Resolved, RepoError> {
        match local {
            None => self.start_from_url(row, persona).await,
            Some(checkout) => self.start_from_checkout(row, &checkout, persona).await,
        }
    }

    /// The row a typed path or URL is, the product's own when it has it already (then `new` is false); a path comes
    /// back with the checkout it was typed in. Both pass the host rule first, so a repo the persona cannot reach is
    /// never added.
    async fn typed_row(
        &self,
        product: &str,
        input: &str,
        persona: &PersonaEnv,
    ) -> Result<(RepoRow, Option<PathBuf>, bool), RepoError> {
        match classify(input, &self.home)? {
            Source::Path(p) => {
                let (root, checkout) = self.checkout_of(&p).await?;
                if inside_any(&root, &self.agent_folders().await?) {
                    return Err(RepoError::InsideTasks(root));
                }
                if let Some(row) = self.repo_at_path(product, &root).await? {
                    return Ok((row, Some(checkout), false));
                }
                check_origin(&root, persona).await?;
                let name = self
                    .free_name(product, &derive_name(&Source::Path(root.clone())), None)
                    .await?;
                let row = RepoRow {
                    product: product.to_string(),
                    name,
                    path: Some(root),
                    url: None,
                    roster: vec![],
                };
                Ok((row, Some(checkout), true))
            }
            Source::Url(u) if has_credentials(&u) => Err(RepoError::Credentials(redact_url(&u))),
            Source::Url(u) => {
                check_host(&u, persona)?;
                let rows = self.store.repos(product).await?;
                if let Some(row) = rows
                    .into_iter()
                    .find(|r| r.url.as_deref().is_some_and(|had| same_repo_url(had, &u)))
                {
                    return Ok((row, None, false));
                }
                let name = self
                    .free_name(
                        product,
                        &derive_name(&Source::Url(u.clone())),
                        Some(&persona.id),
                    )
                    .await?;
                let row = RepoRow {
                    product: product.to_string(),
                    name,
                    path: None,
                    url: Some(u),
                    roster: vec![],
                };
                Ok((row, None, true))
            }
            Source::Name(n) => Err(RepoError::Unrecognised(n)),
        }
    }

    /// The product's repo whose local path is `root`.
    async fn repo_at_path(&self, product: &str, root: &Path) -> Result<Option<RepoRow>, RepoError> {
        Ok(self
            .store
            .repos(product)
            .await?
            .into_iter()
            .find(|r| r.path.as_deref().is_some_and(|p| same_dir(p, root))))
    }

    /// The main checkout and the toplevel of the checkout `path` is in, both canonical.
    async fn checkout_of(&self, path: &Path) -> Result<(PathBuf, PathBuf), RepoError> {
        let path =
            fleet_fs::canonical(path).map_err(|_| RepoError::NotARepo(path.to_path_buf()))?;
        let typed = path.clone();
        let root = match git::blocking(move || fleet_fs::main_root(&typed)).await? {
            None => return Err(RepoError::NotARepo(path)),
            Some(fleet_fs::MainRoot::Bare) => return Err(RepoError::Bare(path)),
            Some(fleet_fs::MainRoot::Worktree(root)) => fleet_fs::canonical(&root)?,
        };
        let typed = path.clone();
        let checkout = git::blocking(move || fleet_fs::toplevel(&typed))
            .await?
            .ok_or_else(|| RepoError::NotARepo(path.clone()))?;
        Ok((root, fleet_fs::canonical(&checkout).unwrap_or(checkout)))
    }

    async fn start_from_url(
        &self,
        row: RepoRow,
        persona: &PersonaEnv,
    ) -> Result<Resolved, RepoError> {
        let Some(url) = row.url.clone() else {
            return Err(RepoError::NoUrl(row.name));
        };
        // Checked at every start: the persona is the task's, and the repo is the product's.
        check_host(&url, persona)?;
        let base = self.clone_dir(&persona.id, &row.product, &row.name);
        let _guard = self.clone_lock.lock().await;
        if base.exists() {
            // A repo deleted from the product and added again under its name with another URL finds the old
            // clone here; starting from it would give the task the wrong repo.
            let origin = git::configured_remote_url(&base, "origin")
                .await
                .ok()
                .flatten();
            if origin.as_deref() != Some(url.as_str()) {
                return Err(RepoError::Start {
                    path: base,
                    reason: format!(
                        "it is a clone of another URL, not {}; move or remove it",
                        redact_url(&url)
                    ),
                });
            }
        } else {
            self.clone_into(&url, &base, persona).await?;
            info!(repo = %row.name, product = %row.product, "repo cloned");
        }
        Ok(Resolved {
            row,
            base,
            start: Start::OriginHead,
        })
    }

    // Not recorded as a field: a clone URL can carry credentials.
    #[tracing::instrument(skip_all)]
    async fn clone_into(
        &self,
        url: &str,
        dest: &Path,
        persona: &PersonaEnv,
    ) -> Result<(), RepoError> {
        debug_assert!(is_safe_dir_name(&persona.id));
        let clones = self.clones_dir();
        std::fs::create_dir_all(&clones)?;
        // Next to the persona folders rather than in one, so `sweep_tmp_clones` finds every half-made clone.
        let tmp = clones.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
        if let Err(e) = git::clone(url, &tmp, &persona.vars).await {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(match e {
                GitError::Git { stderr, .. } => RepoError::Clone {
                    url: redact_url(url),
                    stderr,
                },
                other => other.into(),
            });
        }
        let parent = dest.parent().expect("a clone dir has a parent");
        if let Err(e) = std::fs::create_dir_all(parent).and_then(|()| std::fs::rename(&tmp, dest)) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    async fn start_from_checkout(
        &self,
        row: RepoRow,
        checkout: &Path,
        persona: &PersonaEnv,
    ) -> Result<Resolved, RepoError> {
        let Some(path) = row.path.clone() else {
            return Err(RepoError::NoPath(row.name));
        };
        // A repo an agent `git init`ed is its task's own, never another task's start.
        if inside_any(&path, &self.agent_folders().await?) {
            return Err(RepoError::InsideTasks(path));
        }
        let (root, checkout) = match self.checkout_of(checkout).await {
            Ok(x) => x,
            Err(RepoError::NotARepo(p)) if !p.exists() => {
                return Err(RepoError::Start {
                    path: p,
                    reason: "the folder no longer exists".into(),
                })
            }
            Err(e) => return Err(e),
        };
        if !same_dir(&root, &path) {
            return Err(RepoError::Start {
                path: checkout,
                reason: format!("it is not a checkout of {}", row.name),
            });
        }
        // Checked again at every start: a checkout's origin can have moved since the repo was added.
        check_origin(&path, persona).await?;
        Ok(Resolved {
            row,
            base: path,
            start: Start::Checkout(checkout),
        })
    }

    /// A warning when the branch `checkout` is on is behind the same branch at the repo's URL: the remote tip, read
    /// with `git ls-remote` (nothing is fetched or changed), is missing from the local repo. None when the repo has
    /// no URL, the checkout is on no branch, the URL fails the host rule, or the remote cannot be read in time.
    pub async fn behind_origin(
        &self,
        row: &RepoRow,
        checkout: &Path,
        persona: &PersonaEnv,
    ) -> Option<String> {
        let url = row.url.as_deref()?;
        check_host(url, persona).ok()?;
        let branch = git::current_branch(checkout).await.ok().flatten()?;
        let remote = tokio::time::timeout(
            LS_REMOTE_TIMEOUT,
            git::ls_remote_branch(checkout, url, &branch, &persona.vars),
        )
        .await
        .ok()?
        .ok()
        .flatten()?;
        if git::has_commit(checkout, &remote).await.ok()? {
            return None;
        }
        Some(format!(
            "{branch} in {} is behind {}; pull first or start from the URL",
            checkout.display(),
            redact_url(url)
        ))
    }

    /// Adds a repo an agent `git init`ed inside its task folder to the task's product, as a path repo. Unlike
    /// `resolve`, the tasks root is allowed here: that repo has no other home. It must be its own toplevel (no
    /// subdir, worktree or bare repo).
    pub async fn remember_owned(&self, path: &Path, product: &str) -> Result<RepoRow, RepoError> {
        let path =
            fleet_fs::canonical(path).map_err(|_| RepoError::NotARepo(path.to_path_buf()))?;
        let typed = path.clone();
        match git::blocking(move || fleet_fs::main_root(&typed)).await? {
            Some(fleet_fs::MainRoot::Worktree(root)) if fleet_fs::canonical(&root)? == path => {}
            Some(fleet_fs::MainRoot::Bare) => return Err(RepoError::Bare(path)),
            _ => return Err(RepoError::NotARepo(path)),
        }
        let _guard = self.add_lock.lock().await;
        if let Some(row) = self.repo_at_path(product, &path).await? {
            return Ok(row);
        }
        let name = self
            .free_name(product, &derive_name(&Source::Path(path.clone())), None)
            .await?;
        let row = RepoRow {
            product: product.to_string(),
            name,
            path: Some(path),
            url: None,
            roster: vec![],
        };
        self.store.upsert_repo(&row).await?;
        Ok(row)
    }

    /// Adds the product's repo `name`, or fills in what it lacks: a path for a URL-only repo, a URL for a path-only
    /// one. The roster is kept. A repo that has a URL keeps it: personas hold clones of it, so changing it would
    /// leave them pointing at the old one (`UrlChange`). The path must be a main checkout's own toplevel outside
    /// the task folders.
    pub async fn upsert(
        &self,
        product: &str,
        name: &str,
        path: Option<&str>,
        url: Option<&str>,
    ) -> Result<RepoRow, RepoError> {
        let name = name.trim();
        if !is_safe_dir_name(name) {
            return Err(RepoError::BadName(name.to_string()));
        }
        let (path, url) = (
            path.map(str::trim).filter(|s| !s.is_empty()),
            url.map(str::trim).filter(|s| !s.is_empty()),
        );
        if path.is_none() && url.is_none() {
            return Err(RepoError::NeedPathOrUrl);
        }
        let path = match path {
            Some(p) => Some(self.toplevel_path(&expand(p, &self.home)).await?),
            None => None,
        };
        if let Some(u) = url {
            check_clone_url(u)?;
        }
        // Held across the read and the write, so two calls never both start from the same stale row.
        let _guard = self.add_lock.lock().await;
        if self.store.product(product).await?.is_none() {
            return Err(RepoError::UnknownProduct(product.to_string()));
        }
        let old = self.store.repo(product, name).await?;
        let url = match (old.as_ref().and_then(|r| r.url.as_deref()), url) {
            (Some(had), Some(new)) if !same_repo_url(had, new) => {
                return Err(RepoError::UrlChange(name.to_string()))
            }
            // The stored spelling stays: it is what the clones' origin was set to.
            (Some(had), _) => Some(had.to_string()),
            (None, new) => new.map(str::to_string),
        };
        let row = RepoRow {
            product: product.to_string(),
            name: name.to_string(),
            path: path.or_else(|| old.as_ref().and_then(|r| r.path.clone())),
            url,
            roster: old.map(|r| r.roster).unwrap_or_default(),
        };
        self.store.upsert_repo(&row).await?;
        Ok(row)
    }

    /// `path` canonical, when it is the toplevel of a main checkout and not an agent's folder.
    async fn toplevel_path(&self, path: &Path) -> Result<PathBuf, RepoError> {
        let path =
            fleet_fs::canonical(path).map_err(|_| RepoError::NotARepo(path.to_path_buf()))?;
        let typed = path.clone();
        match git::blocking(move || fleet_fs::main_root(&typed)).await? {
            Some(fleet_fs::MainRoot::Worktree(root)) if fleet_fs::canonical(&root)? == path => {}
            Some(fleet_fs::MainRoot::Bare) => return Err(RepoError::Bare(path)),
            _ => return Err(RepoError::NotARepo(path)),
        }
        if inside_any(&path, &self.agent_folders().await?) {
            return Err(RepoError::InsideTasks(path));
        }
        Ok(path)
    }

    /// The tasks root and every stored task's folder, canonical where they still exist: anything under them
    /// is an agent's. A task made before the root moved keeps its folder under the old root.
    async fn agent_folders(&self) -> Result<Vec<PathBuf>, RepoError> {
        let mut dirs = vec![self.tasks_root.clone()];
        // Every path starts with an empty one, so a row without a real folder must not count.
        dirs.extend(
            self.store
                .snapshot()
                .await?
                .agents
                .into_iter()
                .map(|a| PathBuf::from(a.workdir))
                .filter(|d| d.is_absolute()),
        );
        Ok(dirs
            .into_iter()
            .map(|d| fleet_fs::canonical(&d).unwrap_or(d))
            .collect())
    }

    /// Every repo of every product.
    async fn all_rows(&self) -> Result<Vec<RepoRow>, RepoError> {
        let mut rows = Vec::new();
        for p in self.store.products().await? {
            rows.extend(self.store.repos(&p.id).await?);
        }
        Ok(rows)
    }

    /// The repos a user or agent may pick, one product's or all: not the ones an agent `git init`ed inside a task
    /// folder, which belong to that task.
    pub async fn pickable(&self, product: Option<&str>) -> Result<Vec<RepoRow>, RepoError> {
        let folders = self.agent_folders().await?;
        let rows = match product {
            Some(p) => self.store.repos(p).await?,
            None => self.all_rows().await?,
        };
        Ok(rows
            .into_iter()
            .filter(|r| !r.path.as_deref().is_some_and(|p| inside_any(p, &folders)))
            .collect())
    }

    /// Every checkout worktrees may hang off: each repo's local path and every persona's clone on disk
    /// (`repos/<persona>/<product>/<repo>`), whether or not its repo is still in a product.
    pub async fn all_bases(&self) -> Result<Vec<PathBuf>, RepoError> {
        let mut bases: Vec<PathBuf> = self
            .all_rows()
            .await?
            .into_iter()
            .filter_map(|r| r.path)
            .collect();
        let subdirs = |d: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(d)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                        .map(|e| e.path())
                        .collect()
                })
                .unwrap_or_default()
        };
        let is_repo = |d: &Path| std::fs::symlink_metadata(d.join(".git")).is_ok();
        // A clone from before products (`repos/<persona>/<repo>`) is a repo, not a product folder: its own subfolders
        // are no bases.
        for persona in subdirs(&self.clones_dir()) {
            for product in subdirs(&persona).into_iter().filter(|d| !is_repo(d)) {
                bases.extend(subdirs(&product).into_iter().filter(|d| is_repo(d)));
            }
        }
        bases.sort();
        bases.dedup();
        Ok(bases)
    }

    /// The checkout a task's worktree hangs off: read from the worktree's own `.git` file, else (its folder gone)
    /// the persona's clone of the repo, else the repo's local path. None when none of them exists.
    pub async fn base_of(
        &self,
        worktree: &Path,
        persona: &str,
        product: &str,
        repo: &str,
    ) -> Option<PathBuf> {
        let wt = worktree.to_path_buf();
        if let Ok(Some(fleet_fs::MainRoot::Worktree(root))) =
            git::blocking(move || fleet_fs::main_root(&wt)).await
        {
            if root.as_path() != worktree {
                return Some(root);
            }
        }
        let clone = self.clone_dir(persona, product, repo);
        if clone.exists() {
            return Some(clone);
        }
        let row = self.store.repo(product, repo).await.ok().flatten()?;
        row.path.filter(|p| p.exists())
    }

    /// Where the repo list reads a repo's workflow from, without fetching: any persona's clone of its URL, as last
    /// fetched, else its local path. None when neither is on disk.
    pub fn listing_base(&self, row: &RepoRow) -> Option<(PathBuf, Start)> {
        if row.url.is_some() {
            if let Ok(personas) = std::fs::read_dir(self.clones_dir()) {
                let mut clones: Vec<PathBuf> = personas
                    .flatten()
                    .map(|e| e.path().join(&row.product).join(&row.name))
                    .filter(|p| p.exists())
                    .collect();
                clones.sort();
                if let Some(c) = clones.into_iter().next() {
                    return Some((c, Start::OriginHead));
                }
            }
        }
        let path = row.path.clone().filter(|p| p.exists())?;
        Some((path.clone(), Start::Checkout(path)))
    }

    /// Creates a worktree at `dest` on `branch` from where `r` starts, recording the branch it was on. A URL
    /// start fetches first (failure becomes a warning) and starts from `origin/HEAD` if it exists, else the clone's
    /// HEAD; a checkout is never fetched, so work the user has not pushed is what the agent starts from. An
    /// unreadable start fails before anything is created.
    #[tracing::instrument(skip_all, fields(repo = %r.row.name))]
    pub async fn attach(
        &self,
        r: &Resolved,
        dest: &Path,
        branch: &str,
        persona: &PersonaEnv,
    ) -> Result<(Worktree, Option<String>), RepoError> {
        let (prefix, slug) = split_branch(branch)?;
        let _base = self.lock_base(&r.base).await;
        self.refuse_existing(&r.base, &r.row.name, branch).await?;
        let mut warning = None;
        let (commit, from_branch) = match &r.start {
            Start::Checkout(p) => {
                if !p.exists() {
                    return Err(RepoError::Start {
                        path: p.clone(),
                        reason: "the folder no longer exists".into(),
                    });
                }
                let head = head_of(p).await?;
                // The folder can hold another repo by now; its commit would make `worktree add` fail with a raw
                // git message instead of this.
                if !git::has_commit(&r.base, &head.0).await? {
                    let reason = format!("it is no longer a checkout of {}", r.row.name);
                    return Err(RepoError::Start {
                        path: p.clone(),
                        reason,
                    });
                }
                head
            }
            Start::OriginHead => {
                if let Err(e) = git::fetch(&r.base, &persona.vars).await {
                    warning = Some(format!("could not fetch {}: {e}", r.row.name));
                }
                match git::origin_head(&r.base).await? {
                    Some((commit, name)) => (commit, Some(name)),
                    None => head_of(&r.base).await?,
                }
            }
        };
        let wt = git::create(&r.base, prefix, slug, dest, &commit, from_branch).await?;
        Ok((wt, warning))
    }

    /// Creates a worktree at `dest` on `branch` from the tip of the local branch `from` of `base` (worktrees share
    /// refs, so whichever checkout has it). For a specialist, whose step starts from its task's branch.
    pub async fn attach_from_branch(
        &self,
        base: &Path,
        repo: &str,
        from: &str,
        dest: &Path,
        branch: &str,
    ) -> Result<Worktree, RepoError> {
        let (prefix, slug) = split_branch(branch)?;
        let _base = self.lock_base(base).await;
        self.refuse_existing(base, repo, branch).await?;
        let Some(commit) = git::branch_tip(base, from).await? else {
            return Err(RepoError::Start {
                path: base.to_path_buf(),
                reason: format!("branch {from} no longer exists"),
            });
        };
        Ok(git::create(base, prefix, slug, dest, &commit, Some(from.to_string())).await?)
    }

    /// Never adopt a branch left by an earlier task (Kill keeps branches): its commits are not this task's.
    async fn refuse_existing(
        &self,
        base: &Path,
        repo: &str,
        branch: &str,
    ) -> Result<(), RepoError> {
        if git::branch_exists(base, branch).await? {
            return Err(RepoError::BranchExists {
                branch: branch.to_string(),
                repo: repo.to_string(),
            });
        }
        Ok(())
    }
}

fn split_branch(branch: &str) -> Result<(&str, &str), RepoError> {
    branch.rsplit_once('/').ok_or_else(|| {
        RepoError::Unrecognised(format!("branch {branch:?} must look like prefix/slug"))
    })
}

/// Whether two paths name one folder, once links are resolved where they exist.
fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| fleet_fs::canonical(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// A path repo's `origin` under the host rule; a repo without one (or gone, which attach reports) passes.
async fn check_origin(repo: &Path, persona: &PersonaEnv) -> Result<(), RepoError> {
    if !repo.exists() {
        return Ok(());
    }
    match git::remote_url(repo, "origin").await? {
        Some(url) => check_host(&url, persona),
        None => Ok(()),
    }
}

fn inside_any(path: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|f| path.starts_with(f))
}

/// HEAD commit and branch of a checkout; an unborn or unreadable one is an error, never an empty start.
async fn head_of(checkout: &Path) -> Result<(String, Option<String>), RepoError> {
    let dir = checkout.to_path_buf();
    let head = match git::blocking(move || fleet_fs::head(&dir)).await {
        Ok(h) => h,
        Err(GitError::Fs(fleet_fs::FsError::NotARepo(_))) => {
            return Err(RepoError::Start {
                path: checkout.to_path_buf(),
                reason: "it is no longer a git checkout".into(),
            });
        }
        Err(e) => return Err(e.into()),
    };
    match head.commit {
        Some(commit) => Ok((commit, head.branch)),
        None => Err(RepoError::Start {
            path: checkout.to_path_buf(),
            reason: "it has no commit yet".into(),
        }),
    }
}
