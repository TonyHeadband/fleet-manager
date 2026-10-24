mod common;
use common::git;
use fleet_api::RepoSource;
use fleet_daemon::repos::{classify, RepoError, Repos, Source};
use fleet_daemon::store::Store;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Fx {
    tmp: tempfile::TempDir,
    repos: Repos,
    store: Arc<Store>,
    state: PathBuf,
    projects: PathBuf,
}

async fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let projects = tmp.path().join("projects");
    std::fs::create_dir_all(&state).unwrap();
    let store = Arc::new(Store::open(":memory:".as_ref()).await.unwrap());
    let repos = Repos::new(store.clone(), state.clone(), projects.clone(), tmp.path().join("home"));
    Fx { tmp, repos, store, state, projects }
}

fn commit(dir: &Path, file: &str) {
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", file]);
}

/// A local repo with one commit on `main`.
fn local_repo(root: &Path, name: &str) -> PathBuf {
    let d = root.join(name);
    std::fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q", "-b", "main"]);
    commit(&d, "a");
    d
}

/// A bare repo (HEAD -> main) seeded with one commit; returns (bare dir, file:// url).
fn bare_repo(root: &Path, name: &str) -> (PathBuf, String) {
    let seed = local_repo(root, &format!("{name}-seed"));
    let bare = root.join(format!("{name}.git"));
    git(root, &["clone", "-q", "--bare", seed.to_str().unwrap(), bare.to_str().unwrap()]);
    let url = format!("file://{}", bare.display());
    (bare, url)
}

fn dirs_in(p: &Path) -> Vec<String> {
    match std::fs::read_dir(p) {
        Ok(rd) => rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![],
    }
}

#[test]
fn classify_orders_path_then_url() {
    let home = Path::new("/home/u");
    assert_eq!(classify("/x", home).unwrap(), Source::Path("/x".into()));
    assert_eq!(classify("~/x", home).unwrap(), Source::Path("/home/u/x".into()));
    for u in ["https://h/o/r.git", "ssh://h/r", "git@h:o/r.git"] {
        assert_eq!(classify(u, home).unwrap(), Source::Url(u.into()));
    }
    assert!(matches!(classify("hello world", home), Err(RepoError::Unrecognised(_))));
}

#[tokio::test]
async fn resolve_remembers_a_path_once() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let a = f.repos.resolve(r.to_str().unwrap()).await.unwrap();
    let b = f.repos.resolve(r.to_str().unwrap()).await.unwrap();
    assert_eq!(a, b);
    assert_eq!(a.name, "proj");
    assert_eq!(a.source, RepoSource::Path);
    assert_eq!(f.store.repos().await.unwrap().len(), 1);
}

#[tokio::test]
async fn resolve_by_remembered_name() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let a = f.repos.resolve(r.to_str().unwrap()).await.unwrap();
    assert_eq!(f.repos.resolve("proj").await.unwrap(), a);
}

#[tokio::test]
async fn resolve_rejects_a_non_repo_dir() {
    let f = fx().await;
    let d = f.tmp.path().join("plain");
    std::fs::create_dir_all(&d).unwrap();
    assert!(matches!(f.repos.resolve(d.to_str().unwrap()).await, Err(RepoError::NotARepo(_))));
    assert!(f.store.repos().await.unwrap().is_empty());
}

#[tokio::test]
async fn resolve_rejects_a_path_inside_the_projects_root() {
    let f = fx().await;
    let r = local_repo(&f.projects, "p1");
    assert!(matches!(f.repos.resolve(r.to_str().unwrap()).await, Err(RepoError::InsideProjects(_))));
    assert!(f.store.repos().await.unwrap().is_empty());
}

#[tokio::test]
async fn resolve_clones_a_url_once() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let a = f.repos.resolve(&url).await.unwrap();
    assert_eq!(a.source, RepoSource::Url);
    assert_eq!(a.name, "up");
    assert!(a.base.starts_with(f.state.join("repos")));
    assert!(a.base.join(".git").exists());
    let before = dirs_in(&f.state.join("repos"));
    let b = f.repos.resolve(&url).await.unwrap();
    assert_eq!(a, b);
    assert_eq!(dirs_in(&f.state.join("repos")), before);
}

#[tokio::test]
async fn concurrent_resolves_of_one_url_clone_once() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let (a, b) = tokio::join!(f.repos.resolve(&url), f.repos.resolve(&url));
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(dirs_in(&f.state.join("repos")), vec!["up".to_string()]);
    assert_eq!(f.store.repos().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_clone_leaves_nothing() {
    let f = fx().await;
    let err = f.repos.resolve("file:///nope.git").await.unwrap_err();
    match err {
        RepoError::Clone { url, stderr } => {
            assert_eq!(url, "file:///nope.git");
            assert!(!stderr.is_empty());
        }
        e => panic!("{e:?}"),
    }
    assert!(dirs_in(&f.state.join("repos")).is_empty());
    assert!(f.store.repos().await.unwrap().is_empty());
}

#[tokio::test]
async fn name_clash_with_another_location_gets_a_suffix() {
    let f = fx().await;
    let a = local_repo(&f.tmp.path().join("one"), "proj");
    let b = local_repo(&f.tmp.path().join("two"), "proj");
    let ra = f.repos.resolve(a.to_str().unwrap()).await.unwrap();
    let rb = f.repos.resolve(b.to_str().unwrap()).await.unwrap();
    assert_eq!((ra.name.as_str(), rb.name.as_str()), ("proj", "proj-2"));
}

#[tokio::test]
async fn attach_starts_from_the_remote_default_branch() {
    let f = fx().await;
    let (bare, url) = bare_repo(f.tmp.path(), "up");
    let row = f.repos.resolve(&url).await.unwrap();
    // A commit lands upstream after the base was cloned.
    let other = f.tmp.path().join("other");
    git(f.tmp.path(), &["clone", "-q", &url, other.to_str().unwrap()]);
    commit(&other, "b");
    git(&other, &["push", "-q", "origin", "main"]);
    let upstream = git(&bare, &["rev-parse", "main"]);
    assert_ne!(git(&row.base, &["rev-parse", "HEAD"]), upstream);

    let dest = f.projects.join("p").join("up");
    let (wt, warning) = f.repos.attach(&row, &dest, "fleet/p").await.unwrap();
    assert_eq!(warning, None);
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), upstream);
    assert_eq!(wt.branch, "fleet/p");
    assert_eq!(wt.base, upstream);
}

#[tokio::test]
async fn attach_without_origin_uses_head_and_no_warning() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let row = f.repos.resolve(r.to_str().unwrap()).await.unwrap();
    let dest = f.projects.join("p").join("proj");
    let (_wt, warning) = f.repos.attach(&row, &dest, "fleet/p").await.unwrap();
    assert_eq!(warning, None);
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), git(&r, &["rev-parse", "HEAD"]));
}

#[tokio::test]
async fn resolving_a_subdirectory_uses_the_repo_toplevel() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    std::fs::create_dir_all(r.join("src")).unwrap();
    let row = f.repos.resolve(r.join("src").to_str().unwrap()).await.unwrap();
    assert_eq!(row.base, r.canonicalize().unwrap());
    assert_eq!(row.name, "proj");
}

#[tokio::test]
async fn resolving_a_linked_worktree_uses_its_main_repo() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let wt = f.tmp.path().join("feature-wt");
    git(&r, &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()]);
    let row = f.repos.resolve(wt.to_str().unwrap()).await.unwrap();
    assert_eq!(row.base, r.canonicalize().unwrap());
    assert_eq!(f.repos.resolve(r.to_str().unwrap()).await.unwrap(), row);
}

#[tokio::test]
async fn a_bare_repo_path_gets_a_clear_error() {
    let f = fx().await;
    let (bare, _) = bare_repo(f.tmp.path(), "up");
    let err = f.repos.resolve(bare.to_str().unwrap()).await.unwrap_err();
    assert!(matches!(err, RepoError::Bare(_)));
    assert!(err.to_string().contains("bare repositories are not supported; pass a clone"));
}

#[tokio::test]
async fn sweep_removes_only_half_made_clones() {
    let f = fx().await;
    let clones = f.state.join("repos");
    for d in [".tmp-1234", ".tmp-abcd/objects", "core"] {
        std::fs::create_dir_all(clones.join(d)).unwrap();
    }
    std::fs::write(clones.join(".tmp-file"), "not a clone dir").unwrap();
    f.repos.sweep_tmp_clones();
    let mut left: Vec<String> = std::fs::read_dir(&clones).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, [".tmp-file", "core"]);
    // No clones dir yet (a fresh install) is not an error.
    fx().await.repos.sweep_tmp_clones();
}
