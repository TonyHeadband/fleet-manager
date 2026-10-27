mod common;
use common::git;
use fleet_api::{GithubAuth, RepoPick};
use fleet_daemon::git;
use fleet_daemon::personas::PersonaEnv;
use fleet_daemon::repos::{
    check_host, classify, has_credentials, log_name, redact_url, same_repo_url, RepoError, Repos,
    Source, Start,
};
use fleet_daemon::store::{PersonaRow, ProductRow, RepoRow, Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Fx {
    tmp: tempfile::TempDir,
    repos: Repos,
    store: Arc<Store>,
    state: PathBuf,
    tasks: PathBuf,
}

/// The product most tests resolve in.
const P: &str = "p";

async fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let tasks = tmp.path().join("tasks");
    std::fs::create_dir_all(&state).unwrap();
    let store = Arc::new(Store::open(":memory:".as_ref()).await.unwrap());
    for id in ["p", "a", "b"] {
        store
            .insert_persona(&PersonaRow {
                id: id.into(),
                name: id.into(),
                host: "github.com".into(),
                commit_name: "t".into(),
                commit_email: "t@t".into(),
                github_auth: GithubAuth::Gh,
                ready: true,
                created_at: 1,
            })
            .await
            .unwrap();
    }
    for id in ["p", "q"] {
        store
            .insert_product(&ProductRow {
                id: id.into(),
                name: id.into(),
                default_persona: None,
                created_at: 1,
            })
            .await
            .unwrap();
    }
    let repos = Repos::new(
        store.clone(),
        state.clone(),
        tasks.clone(),
        tmp.path().join("home"),
    );
    Fx {
        tmp,
        repos,
        store,
        state,
        tasks,
    }
}

/// A typed path or URL, or a repo name, with no checkout named.
fn typed(input: &str) -> RepoPick {
    RepoPick {
        repo: input.into(),
        local: None,
    }
}

/// The repo `name`, started from the checkout at `local`.
fn at(name: &str, local: &Path) -> RepoPick {
    RepoPick {
        repo: name.into(),
        local: Some(local.to_string_lossy().into_owned()),
    }
}

async fn url_repo(f: &Fx, product: &str, name: &str, url: &str) {
    f.store
        .upsert_repo(&RepoRow {
            product: product.into(),
            name: name.into(),
            path: None,
            url: Some(url.into()),
            roster: vec![],
        })
        .await
        .unwrap();
}

fn env_for(id: &str, host: &str) -> PersonaEnv {
    PersonaEnv {
        id: id.into(),
        host: host.into(),
        vars: vec![],
    }
}

/// The persona most tests resolve under; no vars, so git runs with the test's own environment.
fn pe() -> PersonaEnv {
    env_for("p", "github.com")
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
    git(
        root,
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let url = format!("file://{}", bare.display());
    (bare, url)
}

fn dirs_in(p: &Path) -> Vec<String> {
    match std::fs::read_dir(p) {
        Ok(rd) => rd
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => vec![],
    }
}

#[test]
fn log_name_never_carries_a_url() {
    assert_eq!(
        log_name("https://user:secret@example.com/org/core.git"),
        "core"
    );
    assert_eq!(log_name("git@example.com:org/web.git"), "web");
    assert_eq!(log_name("core"), "core");
}

#[test]
fn credentials_are_a_password_or_any_http_userinfo() {
    for u in [
        "https://user:secret@h/o/r.git",
        "https://ghp_token@h/o/r.git",
        "HTTP://u@h/r",
        "ssh://git:pw@h/r",
    ] {
        assert!(has_credentials(u), "{u}");
    }
    for u in [
        "https://h/o/r.git",
        "ssh://git@h/r",
        "git@h:o/r.git",
        "https://h/a@b",
        "file:///x@y/r.git",
    ] {
        assert!(!has_credentials(u), "{u}");
    }
    assert_eq!(
        redact_url("https://user:secret@h/o/r.git"),
        "https://h/o/r.git"
    );
    assert_eq!(redact_url("ssh://git@h/r"), "ssh://git@h/r");
}

#[tokio::test]
async fn a_url_with_credentials_is_refused_before_cloning() {
    let f = fx().await;
    let err = f
        .repos
        .resolve(
            P,
            &typed("https://user:secret@example.invalid/org/core.git"),
            &pe(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, RepoError::Credentials(u) if u == "https://example.invalid/org/core.git"),
        "{err:?}"
    );
    assert!(!err.to_string().contains("secret"));
    assert!(dirs_in(&f.state.join("repos")).is_empty());
    assert!(f.store.repos(P).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_unknown_product_is_refused() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let err = f
        .repos
        .resolve("nope", &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, RepoError::UnknownProduct(p) if p == "nope"),
        "{err:?}"
    );
    assert_eq!(err.to_string(), "no such product: nope");
}

/// A task the store knows whose folder is outside the current tasks root, as after the root moved.
async fn old_task(f: &Fx) -> PathBuf {
    let workdir = f.tmp.path().join("old-root").join("p1");
    std::fs::create_dir_all(&workdir).unwrap();
    let a: fleet_api::Agent = serde_json::from_value(serde_json::json!({
        "id": "old", "goal": "t", "workdir": workdir, "tmux_session": "s", "workspaces": [],
        "status": "review", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
        "owner": "o", "blocked_reason": null
    }))
    .unwrap();
    f.store.insert_agent(&a, "tok").await.unwrap();
    workdir
}

#[tokio::test]
async fn a_repo_in_a_task_under_an_old_root_is_refused() {
    let f = fx().await;
    let workdir = old_task(&f).await;
    let owned = local_repo(&workdir, "newtool");
    f.repos.remember_owned(&owned, P).await.unwrap();
    for pick in [typed(owned.to_str().unwrap()), at("newtool", &owned)] {
        let err = f.repos.resolve(P, &pick, &pe()).await.unwrap_err();
        assert!(
            matches!(err, RepoError::InsideTasks(_)),
            "{pick:?}: {err:?}"
        );
    }
    // Still in the product for its own task, but never offered to another.
    assert!(f.store.repo(P, "newtool").await.unwrap().is_some());
    assert!(f.repos.pickable(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_agent_without_a_folder_refuses_nothing() {
    let f = fx().await;
    let a: fleet_api::Agent = serde_json::from_value(serde_json::json!({
        "id": "blank", "goal": "t", "workdir": "", "tmux_session": "s", "workspaces": [],
        "status": "review", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
        "owner": "o", "blocked_reason": null
    }))
    .unwrap();
    f.store.insert_agent(&a, "tok").await.unwrap();
    let r = local_repo(f.tmp.path(), "proj");
    f.repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    assert_eq!(f.repos.pickable(None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn pickable_keeps_the_users_repos() {
    let f = fx().await;
    old_task(&f).await;
    let r = local_repo(f.tmp.path(), "proj");
    f.repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    url_repo(&f, "q", "other", "https://github.com/o/other.git").await;
    let names = |rows: Vec<RepoRow>| rows.into_iter().map(|r| r.name).collect::<Vec<_>>();
    assert_eq!(names(f.repos.pickable(Some(P)).await.unwrap()), ["proj"]);
    assert_eq!(names(f.repos.pickable(Some("q")).await.unwrap()), ["other"]);
    assert_eq!(names(f.repos.pickable(None).await.unwrap()).len(), 2);
}

#[test]
fn classify_orders_path_then_url() {
    let home = Path::new("/home/u");
    assert_eq!(classify("/x", home).unwrap(), Source::Path("/x".into()));
    assert_eq!(
        classify("~/x", home).unwrap(),
        Source::Path("/home/u/x".into())
    );
    for u in ["https://h/o/r.git", "ssh://h/r", "git@h:o/r.git"] {
        assert_eq!(classify(u, home).unwrap(), Source::Url(u.into()));
    }
    assert!(matches!(
        classify("hello world", home),
        Err(RepoError::Unrecognised(_))
    ));
}

#[tokio::test]
async fn resolve_remembers_a_path_once() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let a = f
        .repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap()
        .row;
    let b = f
        .repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap()
        .row;
    assert_eq!(a, b);
    assert_eq!(a.name, "proj");
    assert_eq!((a.path, a.url), (Some(r.canonicalize().unwrap()), None));
    assert_eq!(f.store.repos(P).await.unwrap().len(), 1);
}

#[tokio::test]
async fn typed_path_joins_the_product() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let resolved = f
        .repos
        .resolve("q", &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    let canon = r.canonicalize().unwrap();
    assert_eq!(resolved.start, Start::Checkout(canon.clone()));
    assert_eq!(resolved.base, canon);
    let stored = f.store.repo("q", "proj").await.unwrap().unwrap();
    assert_eq!((stored.path, stored.url), (Some(canon), None));
    assert!(f.store.repos(P).await.unwrap().is_empty());
    // By name it is the same repo, started from a checkout of it.
    let again = f.repos.resolve("q", &at("proj", &r), &pe()).await.unwrap();
    assert_eq!(again.row, resolved.row);
}

#[tokio::test]
async fn resolve_rejects_a_non_repo_dir() {
    let f = fx().await;
    let d = f.tmp.path().join("plain");
    std::fs::create_dir_all(&d).unwrap();
    assert!(matches!(
        f.repos.resolve(P, &typed(d.to_str().unwrap()), &pe()).await,
        Err(RepoError::NotARepo(_))
    ));
    assert!(f.store.repos(P).await.unwrap().is_empty());
}

#[tokio::test]
async fn resolve_rejects_a_path_inside_the_tasks_root() {
    let f = fx().await;
    let r = local_repo(&f.tasks, "p1");
    assert!(matches!(
        f.repos.resolve(P, &typed(r.to_str().unwrap()), &pe()).await,
        Err(RepoError::InsideTasks(_))
    ));
    assert!(f.store.repos(P).await.unwrap().is_empty());
}

#[tokio::test]
async fn resolve_clones_a_url_once() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let a = f.repos.resolve(P, &typed(&url), &pe()).await.unwrap();
    assert_eq!(
        (a.row.name.as_str(), a.row.url.as_deref(), &a.row.path),
        ("up", Some(url.as_str()), &None)
    );
    assert_eq!(a.start, Start::OriginHead);
    assert_eq!(a.base, f.state.join("repos/p/p/up"));
    assert_eq!(a.base, f.repos.clone_dir("p", P, "up"));
    assert!(a.base.join(".git").exists());
    let before = dirs_in(&f.state.join("repos/p/p"));
    let b = f.repos.resolve(P, &typed(&url), &pe()).await.unwrap();
    assert_eq!(a, b);
    assert_eq!(f.repos.resolve(P, &typed("up"), &pe()).await.unwrap(), a);
    assert_eq!(dirs_in(&f.state.join("repos/p/p")), before);
}

#[test]
fn same_repo_url_ignores_spelling() {
    for (a, b) in [
        ("https://github.com/o/core", "https://github.com/o/core.git"),
        ("https://github.com/o/core", "https://github.com/o/core/"),
        (
            "https://github.com/o/core.git/",
            "https://github.com/o/core",
        ),
        ("https://GitHub.com/o/core", "HTTPS://github.com/o/core"),
    ] {
        assert!(same_repo_url(a, b), "{a} {b}");
    }
    for (a, b) in [
        ("https://github.com/o/core", "https://github.com/o/Core"),
        ("https://github.com/o/core", "https://github.com/o/core-2"),
        ("https://github.com/o/core", "https://gitlab.com/o/core"),
    ] {
        assert!(!same_repo_url(a, b), "{a} {b}");
    }
}

#[tokio::test]
async fn a_url_typed_another_way_is_the_same_repo() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let a = f.repos.resolve(P, &typed(&url), &pe()).await.unwrap();
    let bare = url.strip_suffix(".git").unwrap();
    for other in [format!("{url}/"), bare.to_string(), format!("{bare}/")] {
        let b = f.repos.resolve(P, &typed(&other), &pe()).await.unwrap();
        assert_eq!(a, b, "{other}");
    }
    assert_eq!(f.store.repos(P).await.unwrap().len(), 1);
    // Editing the repo with another spelling of its URL is not a URL change, and keeps the stored one.
    let row = f.repos.upsert(P, "up", None, Some(bare)).await.unwrap();
    assert_eq!(row.url.as_deref(), Some(url.as_str()));
}

#[tokio::test]
async fn url_repo_clones_per_persona_and_product() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    url_repo(&f, P, "core", &url).await;
    url_repo(&f, "q", "core", &url).await;
    let a = f.repos.resolve(P, &typed("core"), &pe()).await.unwrap();
    let b = f.repos.resolve("q", &typed("core"), &pe()).await.unwrap();
    assert_ne!(a.base, b.base);
    for r in [&a, &b] {
        assert!(r.base.starts_with(f.state.join("repos/p")), "{:?}", r.base);
        assert!(r.base.join(".git").exists());
    }
    let mut products = dirs_in(&f.state.join("repos/p"));
    products.sort();
    assert_eq!(products, ["p", "q"]);
}

#[tokio::test]
async fn same_url_two_personas_two_clones() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let a = f
        .repos
        .resolve(P, &typed(&url), &env_for("a", "github.com"))
        .await
        .unwrap();
    let b = f
        .repos
        .resolve(P, &typed(&url), &env_for("b", "github.com"))
        .await
        .unwrap();
    assert_eq!(a.base, f.state.join("repos/a/p/up"));
    assert_eq!(b.base, f.state.join("repos/b/p/up"));
    // One repo of the product, whoever cloned it.
    assert_eq!(a.row, b.row);
    assert_eq!(f.store.repos(P).await.unwrap().len(), 1);
}

#[tokio::test]
async fn origin_start_needs_a_url() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    f.repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    let err = f.repos.resolve(P, &typed("proj"), &pe()).await.unwrap_err();
    assert!(
        matches!(&err, RepoError::NoUrl(n) if n == "proj"),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "proj has no URL; start from a local checkout"
    );
    assert!(dirs_in(&f.state.join("repos")).is_empty());
}

#[tokio::test]
async fn local_start_needs_a_path() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    url_repo(&f, P, "up", &url).await;
    let elsewhere = local_repo(f.tmp.path(), "elsewhere");
    let err = f
        .repos
        .resolve(P, &at("up", &elsewhere), &pe())
        .await
        .unwrap_err();
    assert!(matches!(&err, RepoError::NoPath(n) if n == "up"), "{err:?}");
    assert!(err.to_string().contains("up has no local path"), "{err}");
}

#[tokio::test]
async fn a_checkout_of_another_repo_is_refused() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    f.repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    let other = local_repo(f.tmp.path(), "other");
    let err = f
        .repos
        .resolve(P, &at("proj", &other), &pe())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, RepoError::Start { reason, .. } if reason.contains("not a checkout of proj")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_clone_of_another_url_in_the_clone_dir_is_refused() {
    let f = fx().await;
    let (_a, url_a) = bare_repo(f.tmp.path(), "a");
    let (_b, url_b) = bare_repo(f.tmp.path(), "b");
    url_repo(&f, P, "core", &url_a).await;
    f.repos.resolve(P, &typed("core"), &pe()).await.unwrap();
    // The repo is given another URL: the old clone must not be taken for it.
    url_repo(&f, P, "core", &url_b).await;
    let err = f.repos.resolve(P, &typed("core"), &pe()).await.unwrap_err();
    assert!(
        matches!(&err, RepoError::Start { reason, .. } if reason.contains("clone of another URL")),
        "{err:?}"
    );
    // A new URL typed under that name gets a free one instead.
    f.store.delete_repo(P, "core").await.unwrap();
    let (_c, url_c) = bare_repo(f.tmp.path(), "core");
    let typed_c = f.repos.resolve(P, &typed(&url_c), &pe()).await.unwrap();
    assert_eq!(typed_c.row.name, "core-2");
}

#[tokio::test]
async fn concurrent_resolves_of_one_url_clone_once() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let p = pe();
    let pick = typed(&url);
    let (a, b) = tokio::join!(f.repos.resolve(P, &pick, &p), f.repos.resolve(P, &pick, &p));
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(dirs_in(&f.state.join("repos")), vec!["p".to_string()]);
    assert_eq!(dirs_in(&f.state.join("repos/p/p")), vec!["up".to_string()]);
    assert_eq!(f.store.repos(P).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_clone_leaves_nothing() {
    let f = fx().await;
    let err = f
        .repos
        .resolve(P, &typed("file:///nope.git"), &pe())
        .await
        .unwrap_err();
    match err {
        RepoError::Clone { url, stderr } => {
            assert_eq!(url, "file:///nope.git");
            assert!(!stderr.is_empty());
        }
        e => panic!("{e:?}"),
    }
    assert!(dirs_in(&f.state.join("repos")).is_empty());
    assert!(f.store.repos(P).await.unwrap().is_empty());
}

#[tokio::test]
async fn name_clash_with_another_location_gets_a_suffix() {
    let f = fx().await;
    let a = local_repo(&f.tmp.path().join("one"), "proj");
    let b = local_repo(&f.tmp.path().join("two"), "proj");
    let ra = f
        .repos
        .resolve(P, &typed(a.to_str().unwrap()), &pe())
        .await
        .unwrap()
        .row;
    let rb = f
        .repos
        .resolve(P, &typed(b.to_str().unwrap()), &pe())
        .await
        .unwrap()
        .row;
    assert_eq!((ra.name.as_str(), rb.name.as_str()), ("proj", "proj-2"));
}

#[tokio::test]
async fn url_repo_still_starts_from_origin_head() {
    let f = fx().await;
    let (bare, url) = bare_repo(f.tmp.path(), "up");
    let r = f.repos.resolve(P, &typed(&url), &pe()).await.unwrap();
    assert_eq!(r.start, Start::OriginHead);
    // A commit lands upstream after the base was cloned.
    let other = f.tmp.path().join("other");
    git(
        f.tmp.path(),
        &["clone", "-q", &url, other.to_str().unwrap()],
    );
    commit(&other, "b");
    git(&other, &["push", "-q", "origin", "main"]);
    let upstream = git(&bare, &["rev-parse", "main"]);
    assert_ne!(git(&r.base, &["rev-parse", "HEAD"]), upstream);

    let dest = f.tasks.join("p").join("up");
    let (wt, warning) = f.repos.attach(&r, &dest, "fleet/p", &pe()).await.unwrap();
    assert_eq!(warning, None);
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), upstream);
    assert_eq!(wt.branch, "fleet/p");
    assert_eq!(wt.base, upstream);
    assert_eq!(wt.from_branch.as_deref(), Some("origin/main"));
}

#[tokio::test]
async fn path_repo_starts_from_local_head_without_fetching() {
    let f = fx().await;
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let clone = f.tmp.path().join("clone");
    git(
        f.tmp.path(),
        &["clone", "-q", &url, clone.to_str().unwrap()],
    );
    commit(&clone, "local-only");
    // A fetch would now fail, so a pass proves there was none.
    git(
        &clone,
        &["remote", "set-url", "origin", "/nonexistent/remote.git"],
    );
    let remote_main = git(&clone, &["rev-parse", "origin/main"]);

    let r = f
        .repos
        .resolve(P, &typed(clone.to_str().unwrap()), &pe())
        .await
        .unwrap();
    let dest = f.tasks.join("p").join("clone");
    let (wt, warning) = f.repos.attach(&r, &dest, "fleet/p", &pe()).await.unwrap();
    assert_eq!(warning, None);
    assert_eq!(wt.base, git(&clone, &["rev-parse", "HEAD"]));
    assert_ne!(wt.base, remote_main);
    assert_eq!(wt.from_branch.as_deref(), Some("main"));
    assert_eq!(git(&clone, &["rev-parse", "origin/main"]), remote_main);
}

/// `up`'s bare origin, a clone of it at `local` that is the product's repo `up` (path and URL), and a second clone
/// that pushed one more commit to `main`, so `local`'s `main` is one behind.
async fn behind_clone(f: &Fx) -> (PathBuf, String) {
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let local = f.tmp.path().join("local");
    git(
        f.tmp.path(),
        &["clone", "-q", &url, local.to_str().unwrap()],
    );
    let other = f.tmp.path().join("other");
    git(
        f.tmp.path(),
        &["clone", "-q", &url, other.to_str().unwrap()],
    );
    commit(&other, "ahead");
    git(&other, &["push", "-q", "origin", "main"]);
    f.store
        .upsert_repo(&RepoRow {
            product: P.into(),
            name: "up".into(),
            path: Some(local.canonicalize().unwrap()),
            url: Some(url.clone()),
            roster: vec![],
        })
        .await
        .unwrap();
    (local, url)
}

#[tokio::test]
async fn behind_origin_warns() {
    let f = fx().await;
    let (local, url) = behind_clone(&f).await;
    let r = f.repos.resolve(P, &at("up", &local), &pe()).await.unwrap();
    let Start::Checkout(checkout) = &r.start else {
        panic!("{:?}", r.start)
    };
    let warning = f.repos.behind_origin(&r.row, checkout, &pe()).await;
    let want = format!(
        "main in {} is behind {url}; pull first or start from the URL",
        checkout.display()
    );
    assert_eq!(warning.as_deref(), Some(want.as_str()));
    // Once pulled, nothing to say.
    git(&local, &["pull", "-q", "--ff-only"]);
    assert_eq!(f.repos.behind_origin(&r.row, checkout, &pe()).await, None);
}

#[tokio::test]
async fn behind_check_is_silent_when_the_remote_cannot_be_read() {
    let f = fx().await;
    let (local, _url) = behind_clone(&f).await;
    let mut row = f.store.repo(P, "up").await.unwrap().unwrap();
    row.url = Some("file:///nonexistent/up.git".into());
    assert_eq!(f.repos.behind_origin(&row, &local, &pe()).await, None);
    // A URL the persona may not reach is never asked.
    row.url = Some("https://gitlab.example/o/up.git".into());
    assert_eq!(f.repos.behind_origin(&row, &local, &pe()).await, None);
}

#[tokio::test]
async fn behind_check_never_fetches() {
    let f = fx().await;
    let (local, _url) = behind_clone(&f).await;
    let refs = || git(&local, &["for-each-ref", "refs/remotes/origin"]);
    let before = refs();
    let r = f.repos.resolve(P, &at("up", &local), &pe()).await.unwrap();
    let Start::Checkout(checkout) = &r.start else {
        panic!("{:?}", r.start)
    };
    assert!(f
        .repos
        .behind_origin(&r.row, checkout, &pe())
        .await
        .is_some());
    f.repos
        .attach(&r, &f.tasks.join("t").join("up"), "fleet/t", &pe())
        .await
        .unwrap();
    assert_eq!(refs(), before);
}

#[tokio::test]
async fn attach_without_origin_uses_head_and_no_warning() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let resolved = f
        .repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    let dest = f.tasks.join("p").join("proj");
    let (_wt, warning) = f
        .repos
        .attach(&resolved, &dest, "fleet/p", &pe())
        .await
        .unwrap();
    assert_eq!(warning, None);
    assert_eq!(
        git(&dest, &["rev-parse", "HEAD"]),
        git(&r, &["rev-parse", "HEAD"])
    );
}

/// A repo on `main` with a linked worktree `feat` that is one commit ahead.
fn repo_with_feat(f: &Fx) -> (PathBuf, PathBuf) {
    let r = local_repo(f.tmp.path(), "proj");
    let feat = f.tmp.path().join("feat");
    git(
        &r,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat",
            feat.to_str().unwrap(),
        ],
    );
    commit(&feat, "on-feat");
    (r, feat.canonicalize().unwrap())
}

#[tokio::test]
async fn picking_a_linked_worktree_starts_from_its_head() {
    let f = fx().await;
    let (r, feat) = repo_with_feat(&f);
    let resolved = f
        .repos
        .resolve(P, &typed(feat.to_str().unwrap()), &pe())
        .await
        .unwrap();
    assert_eq!(resolved.row.path, Some(r.canonicalize().unwrap()));
    assert_eq!(resolved.start, Start::Checkout(feat.clone()));
    assert_eq!(f.store.repos(P).await.unwrap().len(), 1);

    let dest = f.tasks.join("p").join("proj");
    let (wt, _) = f
        .repos
        .attach(&resolved, &dest, "fleet/p", &pe())
        .await
        .unwrap();
    assert_eq!(wt.base, git(&feat, &["rev-parse", "HEAD"]));
    assert_ne!(wt.base, git(&r, &["rev-parse", "HEAD"]));
    assert_eq!(wt.from_branch.as_deref(), Some("feat"));
}

#[tokio::test]
async fn picking_a_subfolder_of_a_worktree_uses_that_worktree() {
    let f = fx().await;
    let (_r, feat) = repo_with_feat(&f);
    std::fs::create_dir_all(feat.join("src")).unwrap();
    let resolved = f
        .repos
        .resolve(P, &typed(feat.join("src").to_str().unwrap()), &pe())
        .await
        .unwrap();
    assert_eq!(resolved.start, Start::Checkout(feat));
}

#[tokio::test]
async fn a_named_checkout_starts_from_its_head() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let first = f
        .repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    git(&r, &["checkout", "-q", "-b", "other"]);
    commit(&r, "on-other");
    let resolved = f.repos.resolve(P, &at("proj", &r), &pe()).await.unwrap();
    assert_eq!(resolved.row, first.row);
    assert_eq!(resolved.start, Start::Checkout(r.canonicalize().unwrap()));

    let dest = f.tasks.join("p").join("proj");
    let (wt, _) = f
        .repos
        .attach(&resolved, &dest, "fleet/p", &pe())
        .await
        .unwrap();
    assert_eq!(wt.base, git(&r, &["rev-parse", "HEAD"]));
    assert_eq!(wt.from_branch.as_deref(), Some("other"));
}

#[tokio::test]
async fn a_deleted_checkout_fails_and_leaves_nothing() {
    let f = fx().await;
    let (r, feat) = repo_with_feat(&f);
    let resolved = f
        .repos
        .resolve(P, &typed(feat.to_str().unwrap()), &pe())
        .await
        .unwrap();
    std::fs::remove_dir_all(&feat).unwrap();

    let dest = f.tasks.join("p").join("proj");
    let err = f
        .repos
        .attach(&resolved, &dest, "fleet/x", &pe())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, RepoError::Start { path, reason } if *path == feat && reason.contains("no longer exists")),
        "{err:?}"
    );
    assert!(err.to_string().contains(feat.to_str().unwrap()));
    assert!(!git::branch_exists(&r, "fleet/x").await.unwrap());
    assert!(!dest.exists());
    // Picked again by name, the same gone checkout says so before anything is made.
    let err = f
        .repos
        .resolve(P, &at("proj", &feat), &pe())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, RepoError::Start { reason, .. } if reason.contains("no longer exists")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_checkout_replaced_by_another_repo_fails_clearly() {
    let f = fx().await;
    let (r, feat) = repo_with_feat(&f);
    let resolved = f
        .repos
        .resolve(P, &typed(feat.to_str().unwrap()), &pe())
        .await
        .unwrap();
    std::fs::remove_dir_all(&feat).unwrap();
    local_repo(
        feat.parent().unwrap(),
        feat.file_name().unwrap().to_str().unwrap(),
    );
    commit(&feat, "unrelated");

    let dest = f.tasks.join("p").join("proj");
    let err = f
        .repos
        .attach(&resolved, &dest, "fleet/x", &pe())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, RepoError::Start { path, reason } if *path == feat && reason.contains("no longer a checkout of proj")),
        "{err:?}"
    );
    assert!(!git::branch_exists(&r, "fleet/x").await.unwrap());
    assert!(!dest.exists());
}

#[tokio::test]
async fn an_unborn_main_checkout_fails_clearly() {
    let f = fx().await;
    let d = f.tmp.path().join("empty");
    std::fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q", "-b", "main"]);
    for pick in [typed(d.to_str().unwrap()), at("empty", &d)] {
        let resolved = f.repos.resolve(P, &pick, &pe()).await.unwrap();
        let dest = f.tasks.join("p").join("empty");
        let err = f
            .repos
            .attach(&resolved, &dest, "fleet/x", &pe())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, RepoError::Start { reason, .. } if reason.contains("no commit")),
            "{err:?}"
        );
        assert!(!dest.exists());
    }
}

#[tokio::test]
async fn resolving_a_subdirectory_uses_the_repo_toplevel() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    std::fs::create_dir_all(r.join("src")).unwrap();
    let resolved = f
        .repos
        .resolve(P, &typed(r.join("src").to_str().unwrap()), &pe())
        .await
        .unwrap();
    assert_eq!(resolved.row.path, Some(r.canonicalize().unwrap()));
    assert_eq!(resolved.base, r.canonicalize().unwrap());
    assert_eq!(resolved.row.name, "proj");
}

#[tokio::test]
async fn resolving_a_linked_worktree_uses_its_main_repo() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    let wt = f.tmp.path().join("feature-wt");
    git(
        &r,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
    );
    let row = f
        .repos
        .resolve(P, &typed(wt.to_str().unwrap()), &pe())
        .await
        .unwrap()
        .row;
    assert_eq!(row.path, Some(r.canonicalize().unwrap()));
    assert_eq!(
        f.repos
            .resolve(P, &typed(r.to_str().unwrap()), &pe())
            .await
            .unwrap()
            .row,
        row
    );
}

#[tokio::test]
async fn a_bare_repo_path_gets_a_clear_error() {
    let f = fx().await;
    let (bare, _) = bare_repo(f.tmp.path(), "up");
    let err = f
        .repos
        .resolve(P, &typed(bare.to_str().unwrap()), &pe())
        .await
        .unwrap_err();
    assert!(matches!(err, RepoError::Bare(_)));
    assert!(err
        .to_string()
        .contains("bare repositories are not supported; pass a clone"));
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
    let mut left: Vec<String> = std::fs::read_dir(&clones)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, [".tmp-file", "core"]);
    // No clones dir yet (a fresh install) is not an error.
    fx().await.repos.sweep_tmp_clones();
}

#[test]
fn https_on_persona_host_is_accepted() {
    check_host("https://GitHub.com:443/x/y.git", &pe()).unwrap();
    check_host("https://www.github.com/x/y", &pe()).unwrap();
}

#[test]
fn plain_http_is_refused_even_on_the_persona_host() {
    for u in ["http://github.com/x/y", "HTTP://www.github.com/x/y"] {
        let err = check_host(u, &pe()).unwrap_err();
        assert!(
            matches!(&err, RepoError::PlainHttp(url) if url == u),
            "{u}: {err:?}"
        );
        assert!(err.to_string().contains("use https://"), "{err}");
    }
}

#[test]
fn other_host_is_refused() {
    let err = check_host("https://ghe.example.invalid/x/y", &pe()).unwrap_err();
    assert!(
        matches!(&err, RepoError::WrongHost { host, persona, persona_host, .. }
            if host == "ghe.example.invalid" && persona == "p" && persona_host == "github.com"),
        "{err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("ghe.example.invalid") && msg.contains("github.com"),
        "{msg}"
    );
}

#[test]
fn ssh_is_refused() {
    for u in [
        "git@github.com:x/y",
        "ssh://git@github.com/x/y",
        "git://github.com/x/y",
    ] {
        let err = check_host(u, &pe()).unwrap_err();
        assert!(
            matches!(&err, RepoError::Ssh(url) if url == u),
            "{u}: {err:?}"
        );
        assert!(err.to_string().contains("https://"), "{err}");
    }
}

#[test]
fn file_url_is_accepted() {
    check_host("file:///srv/git/x.git", &pe()).unwrap();
    check_host("/home/u/src/x", &pe()).unwrap();
}

#[tokio::test]
async fn path_repo_checks_origin() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    // No origin: nothing to check.
    f.repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    git(
        &r,
        &["remote", "add", "origin", "https://ghe.example.invalid/a/b"],
    );
    for pick in [typed(r.to_str().unwrap()), at("proj", &r)] {
        let err = f.repos.resolve(P, &pick, &pe()).await.unwrap_err();
        assert!(
            matches!(&err, RepoError::WrongHost { host, .. } if host == "ghe.example.invalid"),
            "{pick:?}: {err:?}"
        );
    }
    let fresh = local_repo(f.tmp.path(), "fresh");
    git(&fresh, &["remote", "add", "origin", "git@github.com:a/b"]);
    assert!(matches!(
        f.repos
            .resolve(P, &typed(fresh.to_str().unwrap()), &pe())
            .await,
        Err(RepoError::Ssh(_))
    ));
    assert_eq!(
        f.store.repos(P).await.unwrap().len(),
        1,
        "a refused path was remembered"
    );
}

#[tokio::test]
async fn a_url_on_another_host_is_refused_at_start() {
    let f = fx().await;
    url_repo(&f, P, "lab", "https://gitlab.example/o/lab.git").await;
    let err = f.repos.resolve(P, &typed("lab"), &pe()).await.unwrap_err();
    assert!(
        matches!(&err, RepoError::WrongHost { host, .. } if host == "gitlab.example"),
        "{err:?}"
    );
    assert!(dirs_in(&f.state.join("repos")).is_empty());
}

#[tokio::test]
async fn remember_owned_adds_a_path_repo_to_the_product() {
    let f = fx().await;
    let workdir = old_task(&f).await;
    let owned = local_repo(&workdir, "newtool");
    let row = f.repos.remember_owned(&owned, "q").await.unwrap();
    assert_eq!(
        (
            row.product.as_str(),
            row.name.as_str(),
            row.path.clone(),
            row.url.clone()
        ),
        ("q", "newtool", Some(owned.canonicalize().unwrap()), None)
    );
    assert_eq!(f.repos.remember_owned(&owned, "q").await.unwrap(), row);
    assert!(f.store.repos(P).await.unwrap().is_empty());
}

#[tokio::test]
async fn all_bases_lists_paths_and_clones() {
    let f = fx().await;
    let r = local_repo(f.tmp.path(), "proj");
    f.repos
        .resolve(P, &typed(r.to_str().unwrap()), &pe())
        .await
        .unwrap();
    let (_bare, url) = bare_repo(f.tmp.path(), "up");
    let clone = f
        .repos
        .resolve("q", &typed(&url), &pe())
        .await
        .unwrap()
        .base;
    // A clone whose repo left its product still holds branches.
    f.store.delete_repo("q", "up").await.unwrap();
    // A clone from before products sits where a product folder would, `repos/<persona>/<repo>`.
    let old = local_repo(&f.state.join("repos/p"), "old");
    std::fs::create_dir_all(old.join("src")).unwrap();
    let bases = f.repos.all_bases().await.unwrap();
    assert!(bases.contains(&r.canonicalize().unwrap()), "{bases:?}");
    assert!(bases.contains(&clone), "{bases:?}");
    assert!(!bases.iter().any(|b| b.starts_with(&old)), "{bases:?}");
}
