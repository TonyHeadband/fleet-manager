#[cfg(unix)]
use fleet_daemon::internal::InternalError;
use fleet_daemon::internal::{Internal, MergeOutcome, SKELETON};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=T", "-c", "user.email=t@localhost"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit_file(wt: &Path, name: &str, content: &[u8]) -> String {
    let p = wt.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, content).unwrap();
    git(wt, &["add", name]);
    git(wt, &["commit", "-q", "-m", &format!("add {name}")]);
    git(wt, &["rev-parse", "HEAD"])
}

fn setup() -> (tempfile::TempDir, Internal) {
    let tmp = tempfile::tempdir().unwrap();
    let internal = Internal::new(tmp.path().join("internal"));
    (tmp, internal)
}

#[tokio::test]
async fn ensure_creates_main_with_one_commit() {
    let (_tmp, i) = setup();
    i.ensure("p").await.unwrap();
    assert_eq!(i.branches("p").await.unwrap(), ["main"]);
    let log = i.history("p", "main", None, 10).await.unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "Start internal resources");
    assert_eq!(log[0].author, "Fleet Manager");
}

#[tokio::test]
async fn ensure_seeds_the_skeleton() {
    let (_tmp, i) = setup();
    i.ensure("p").await.unwrap();
    let log = i.history("p", "main", None, 10).await.unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "Start internal resources");
    assert_eq!(log[0].author, "Fleet Manager");
    let mut want: Vec<&str> = SKELETON.iter().map(|(path, _)| *path).collect();
    want.sort();
    assert_eq!(
        want,
        [
            "README.md",
            "docs/README.md",
            "plans/README.md",
            "scripts/README.md",
            "specs/README.md",
            "tasks/README.md"
        ]
    );
    assert_eq!(i.files("p", "main").await.unwrap(), want);
}

#[tokio::test]
async fn ensure_is_idempotent() {
    let (_tmp, i) = setup();
    i.ensure("p").await.unwrap();
    i.ensure("p").await.unwrap();
    assert_eq!(i.history("p", "main", None, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn attach_task_branches_from_main() {
    let (tmp, i) = setup();
    let dest = tmp.path().join("wt");
    i.attach_task("p", "t", &dest).await.unwrap();
    assert!(dest.join(".git").is_file());
    assert_eq!(i.branches("p").await.unwrap(), ["main", "task/t"]);
    assert!(i.branch_exists("p", "task/t").await.unwrap());
    // Attaching again while it is there changes nothing.
    i.attach_task("p", "t", &dest).await.unwrap();
}

#[tokio::test]
async fn attach_task_recreates_missing_worktree() {
    let (tmp, i) = setup();
    let dest = tmp.path().join("wt");
    i.attach_task("p", "t", &dest).await.unwrap();
    commit_file(&dest, "design.md", b"hi");
    std::fs::remove_dir_all(&dest).unwrap();
    i.attach_task("p", "t", &dest).await.unwrap();
    assert_eq!(git(&dest, &["branch", "--show-current"]), "task/t");
    assert!(dest.join("design.md").exists());
}

#[tokio::test]
async fn snapshot_is_detached_at_tip() {
    let (tmp, i) = setup();
    let wt = tmp.path().join("wt");
    let snap = tmp.path().join("snap");
    i.attach_task("p", "t", &wt).await.unwrap();
    let sha = commit_file(&wt, "plan.md", b"plan");
    assert_eq!(i.snapshot("p", "t", &snap).await.unwrap(), sha);
    assert!(snap.join("plan.md").exists());
    let detached = Command::new("git")
        .arg("-C")
        .arg(&snap)
        .args(["symbolic-ref", "-q", "HEAD"])
        .status()
        .unwrap();
    assert!(!detached.success());
}

#[tokio::test]
async fn remove_worktree_keeps_branch_and_tolerates_missing() {
    let (tmp, i) = setup();
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt).await.unwrap();
    assert!(!i.is_dirty(&wt).await.unwrap());
    std::fs::write(wt.join("scratch"), "x").unwrap();
    assert!(i.is_dirty(&wt).await.unwrap());
    i.remove_worktree("p", &wt).await.unwrap();
    assert!(!wt.exists());
    assert!(i.branch_exists("p", "task/t").await.unwrap());
    i.remove_worktree("p", &wt).await.unwrap();
}

#[tokio::test]
async fn merge_task_merges_and_reports_up_to_date() {
    let (tmp, i) = setup();
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt).await.unwrap();
    commit_file(&wt, "design.md", b"d");
    assert_eq!(i.merge_task("p", "t").await.unwrap(), MergeOutcome::Merged);
    assert!(i
        .files("p", "main")
        .await
        .unwrap()
        .contains(&"design.md".to_string()));
    assert_eq!(
        i.merge_task("p", "t").await.unwrap(),
        MergeOutcome::UpToDate
    );
}

#[tokio::test]
async fn merge_conflict_aborts_and_leaves_main_clean() {
    let (tmp, i) = setup();
    let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
    i.attach_task("p", "a", &a).await.unwrap();
    i.attach_task("p", "b", &b).await.unwrap();
    commit_file(&a, "notes.md", b"from a");
    commit_file(&b, "notes.md", b"from b");
    assert_eq!(i.merge_task("p", "a").await.unwrap(), MergeOutcome::Merged);
    assert_eq!(
        i.merge_task("p", "b").await.unwrap(),
        MergeOutcome::Conflict(vec!["notes.md".to_string()])
    );
    assert_eq!(git(&i.repo_dir("p"), &["status", "--porcelain"]), "");
}

// Creating a symlink needs a privilege on Windows (Developer Mode); sub-project 2 decides how to test it there.
#[cfg(unix)]
#[tokio::test]
async fn read_refuses_bad_paths_large_and_binary() {
    let (tmp, i) = setup();
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt).await.unwrap();
    commit_file(&wt, "ok.md", b"fine");
    commit_file(&wt, "big.md", &vec![b'a'; 1_048_577]);
    commit_file(&wt, "bin.md", &[0xff, 0xfe]);
    std::os::unix::fs::symlink("/etc/passwd", wt.join("link.md")).unwrap();
    git(&wt, &["add", "link.md"]);
    git(&wt, &["commit", "-q", "-m", "link"]);

    assert_eq!(i.read("p", "task/t", "ok.md").await.unwrap(), "fine");
    for bad in ["../x", "/etc/passwd", "a/../b", ""] {
        assert!(
            matches!(
                i.read("p", "task/t", bad).await,
                Err(InternalError::BadPath(_))
            ),
            "{bad}"
        );
    }
    assert!(matches!(
        i.read("p", "task/t", "link.md").await,
        Err(InternalError::BadPath(_))
    ));
    assert!(matches!(
        i.read("p", "task/t", "big.md").await,
        Err(InternalError::TooLarge)
    ));
    assert!(matches!(
        i.read("p", "task/t", "bin.md").await,
        Err(InternalError::NotText)
    ));
    assert!(matches!(
        i.read("p", "task/t", "nope.md").await,
        Err(InternalError::NotFound(_))
    ));
    assert!(matches!(
        i.read("p", "task/zzz", "ok.md").await,
        Err(InternalError::NotFound(_))
    ));
    assert!(matches!(
        i.read("ghost", "main", "ok.md").await,
        Err(InternalError::NotFound(_))
    ));
}

#[tokio::test]
async fn history_filters_by_path() {
    let (tmp, i) = setup();
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt).await.unwrap();
    commit_file(&wt, "a.md", b"a");
    commit_file(&wt, "b.md", b"b");
    let all = i.history("p", "task/t", None, 10).await.unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].subject, "add b.md");
    let only_a = i.history("p", "task/t", Some("a.md"), 10).await.unwrap();
    assert_eq!(only_a.len(), 1);
    assert_eq!(only_a[0].subject, "add a.md");
    assert_eq!(i.history("p", "task/t", None, 1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn delete_removes_repo() {
    let (_tmp, i) = setup();
    i.ensure("p").await.unwrap();
    i.delete("p").await.unwrap();
    assert!(!i.repo_dir("p").exists());
    i.delete("p").await.unwrap();
}

// Creating a symlink needs a privilege on Windows (Developer Mode); sub-project 2 decides how to test it there.
#[cfg(unix)]
#[tokio::test]
async fn attach_task_recreates_worktree_behind_symlinked_parent() {
    let (tmp, i) = setup();
    let real = tmp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let dest = link.join("wt");
    i.attach_task("p", "t", &dest).await.unwrap();
    commit_file(&dest, "design.md", b"hi");
    std::fs::remove_dir_all(&dest).unwrap();
    i.attach_task("p", "t", &dest).await.unwrap();
    assert!(dest.join("design.md").exists());
}

fn bare_remote(tmp: &Path, name: &str) -> std::path::PathBuf {
    let bare = tmp.join(name);
    git(
        tmp,
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    bare
}

fn remote_url_of(bare: &Path) -> String {
    format!("file://{}", bare.display())
}

/// A bare remote whose `main` has one commit adding `x.md`.
fn remote_with_x(tmp: &Path) -> std::path::PathBuf {
    let bare = bare_remote(tmp, "origin.git");
    let seed = tmp.join("seed");
    git(
        tmp,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            seed.to_str().unwrap(),
        ],
    );
    git(&seed, &["checkout", "-q", "-b", "main"]);
    commit_file(&seed, "x.md", b"x");
    git(&seed, &["push", "-q", "origin", "main"]);
    bare
}

#[tokio::test]
async fn connect_clones_and_keeps_old_repo_aside() {
    let (tmp, i) = setup();
    let local = i.ensure("p").await.unwrap();
    commit_file(&local, "mine.md", b"m");
    let bare = remote_with_x(tmp.path());
    let url = remote_url_of(&bare);

    let aside = i
        .connect("p", &url, &[], 1_700_000_000)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        aside.file_name().unwrap().to_str().unwrap(),
        "p.local-20231114-221320"
    );
    assert!(aside.join("mine.md").exists());
    let files = i.files("p", "main").await.unwrap();
    assert!(files.contains(&"x.md".to_string()));
    assert!(!files.contains(&"mine.md".to_string()));
    let state = i.remote_state("p").await.unwrap();
    assert_eq!(state.url.as_deref(), Some(url.as_str()));
    assert_eq!(state.unpushed, 0);
    assert_eq!(state.error, None);
}

#[tokio::test]
async fn connect_without_a_repo_has_nothing_to_move_aside() {
    let (tmp, i) = setup();
    let bare = remote_with_x(tmp.path());
    let aside = i
        .connect("p", &remote_url_of(&bare), &[], 1_700_000_000)
        .await
        .unwrap();
    assert_eq!(aside, None);
    assert!(i
        .files("p", "main")
        .await
        .unwrap()
        .contains(&"x.md".to_string()));
}

#[tokio::test]
async fn connect_seeds_and_pushes_an_empty_remote() {
    let (tmp, i) = setup();
    let bare = bare_remote(tmp.path(), "empty.git");
    i.connect("p", &remote_url_of(&bare), &[], 1_700_000_000)
        .await
        .unwrap();
    let mut want: Vec<&str> = SKELETON.iter().map(|(path, _)| *path).collect();
    want.sort();
    let on_remote = git(&bare, &["ls-tree", "-r", "--name-only", "main"]);
    assert_eq!(on_remote.lines().collect::<Vec<_>>(), want);
    let state = i.remote_state("p").await.unwrap();
    assert_eq!(state.unpushed, 0);
}

#[tokio::test]
async fn connect_failure_leaves_repo_in_place() {
    let (tmp, i) = setup();
    let local = i.ensure("p").await.unwrap();
    let url = remote_url_of(&tmp.path().join("missing.git"));

    let err = i.connect("p", &url, &[], 1_700_000_000).await.unwrap_err();

    assert!(matches!(err, InternalError::Remote(_)), "{err:?}");
    assert_eq!(local, i.repo_dir("p"));
    assert!(local.join(".git").exists());
    let stray: Vec<_> = std::fs::read_dir(i.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != "p")
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
}

#[tokio::test]
async fn disconnect_keeps_history() {
    let (tmp, i) = setup();
    let bare = remote_with_x(tmp.path());
    i.connect("p", &remote_url_of(&bare), &[], 1_700_000_000)
        .await
        .unwrap();
    let before = i.history("p", "main", None, 10).await.unwrap();

    i.disconnect("p").await.unwrap();

    let state = i.remote_state("p").await.unwrap();
    assert_eq!(state.url, None);
    assert_eq!(state.unpushed, 0);
    assert_eq!(i.history("p", "main", None, 10).await.unwrap(), before);
}

#[tokio::test]
async fn remote_state_of_a_local_repo_or_none_is_local_only() {
    let (_tmp, i) = setup();
    assert_eq!(i.remote_state("p").await.unwrap().url, None);
    i.ensure("p").await.unwrap();
    assert_eq!(i.remote_state("p").await.unwrap().url, None);
}
