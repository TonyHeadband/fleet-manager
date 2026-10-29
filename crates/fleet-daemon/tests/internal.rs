#[cfg(unix)]
use fleet_daemon::internal::InternalError;
use fleet_daemon::internal::{Internal, MergeOutcome, PushOutcome, SKELETON};
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

const NOW: i64 = 1_700_000_000;
const NO_ENV: &[(String, String)] = &[];

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
    i.attach_task("p", "t", &dest, None).await.unwrap();
    assert!(dest.join(".git").is_file());
    assert_eq!(i.branches("p").await.unwrap(), ["main", "task/t"]);
    assert!(i.branch_exists("p", "task/t").await.unwrap());
    // Attaching again while it is there changes nothing.
    i.attach_task("p", "t", &dest, None).await.unwrap();
}

#[tokio::test]
async fn attach_task_recreates_missing_worktree() {
    let (tmp, i) = setup();
    let dest = tmp.path().join("wt");
    i.attach_task("p", "t", &dest, None).await.unwrap();
    commit_file(&dest, "design.md", b"hi");
    std::fs::remove_dir_all(&dest).unwrap();
    i.attach_task("p", "t", &dest, None).await.unwrap();
    assert_eq!(git(&dest, &["branch", "--show-current"]), "task/t");
    assert!(dest.join("design.md").exists());
}

#[tokio::test]
async fn snapshot_is_detached_at_tip() {
    let (tmp, i) = setup();
    let wt = tmp.path().join("wt");
    let snap = tmp.path().join("snap");
    i.attach_task("p", "t", &wt, None).await.unwrap();
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
    i.attach_task("p", "t", &wt, None).await.unwrap();
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
    i.attach_task("p", "t", &wt, None).await.unwrap();
    commit_file(&wt, "design.md", b"d");
    let report = i.merge_task("p", "t", None, NOW).await.unwrap();
    assert_eq!(report.merge, MergeOutcome::Merged);
    assert_eq!(report.push.outcome, PushOutcome::LocalOnly);
    assert!(i
        .files("p", "main")
        .await
        .unwrap()
        .contains(&"design.md".to_string()));
    assert_eq!(
        i.merge_task("p", "t", None, NOW).await.unwrap().merge,
        MergeOutcome::UpToDate
    );
}

#[tokio::test]
async fn merge_conflict_aborts_and_leaves_main_clean() {
    let (tmp, i) = setup();
    let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
    i.attach_task("p", "a", &a, None).await.unwrap();
    i.attach_task("p", "b", &b, None).await.unwrap();
    commit_file(&a, "notes.md", b"from a");
    commit_file(&b, "notes.md", b"from b");
    assert_eq!(
        i.merge_task("p", "a", None, NOW).await.unwrap().merge,
        MergeOutcome::Merged
    );
    let report = i.merge_task("p", "b", None, NOW).await.unwrap();
    assert_eq!(
        report.merge,
        MergeOutcome::Conflict(vec!["notes.md".to_string()])
    );
    assert_eq!(report.push.outcome, PushOutcome::LocalOnly);
    assert_eq!(git(&i.repo_dir("p"), &["status", "--porcelain"]), "");
}

// Creating a symlink needs a privilege on Windows (Developer Mode); sub-project 2 decides how to test it there.
#[cfg(unix)]
#[tokio::test]
async fn read_refuses_bad_paths_large_and_binary() {
    let (tmp, i) = setup();
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt, None).await.unwrap();
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
    i.attach_task("p", "t", &wt, None).await.unwrap();
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
    i.attach_task("p", "t", &dest, None).await.unwrap();
    commit_file(&dest, "design.md", b"hi");
    std::fs::remove_dir_all(&dest).unwrap();
    i.attach_task("p", "t", &dest, None).await.unwrap();
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

/// `p` connected to a fresh remote (`origin.git`, `main` with `x.md`), and a plain clone of it playing the user.
async fn connected(tmp: &Path, i: &Internal) -> (std::path::PathBuf, std::path::PathBuf) {
    let bare = remote_with_x(tmp);
    i.connect("p", &remote_url_of(&bare), NO_ENV, NOW)
        .await
        .unwrap();
    let user = tmp.join("user");
    git(
        tmp,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            user.to_str().unwrap(),
        ],
    );
    (bare, user)
}

fn user_pushes(user: &Path, name: &str, content: &[u8]) {
    git(user, &["pull", "-q", "--ff-only"]);
    commit_file(user, name, content);
    git(user, &["push", "-q", "origin", "main"]);
}

fn remote_file(bare: &Path, rev: &str, path: &str) -> String {
    git(bare, &["show", &format!("{rev}:{path}")])
}

/// Takes the remote away (as a network failure would); `restore` puts it back.
fn take_away(bare: &Path) -> std::path::PathBuf {
    let kept = bare.with_extension("kept");
    std::fs::rename(bare, &kept).unwrap();
    kept
}

fn restore(kept: &Path, bare: &Path) {
    std::fs::rename(kept, bare).unwrap();
}

#[tokio::test]
async fn merge_task_pushes_main() {
    let (tmp, i) = setup();
    let (bare, _user) = connected(tmp.path(), &i).await;
    let wt = tmp.path().join("wt");
    assert_eq!(
        i.attach_task("p", "t", &wt, Some(NO_ENV)).await.unwrap(),
        None
    );
    commit_file(&wt, "tasks/t/plan.md", b"plan");

    let report = i.merge_task("p", "t", Some(NO_ENV), NOW).await.unwrap();

    assert_eq!(report.merge, MergeOutcome::Merged);
    assert_eq!(report.push.outcome, PushOutcome::Pushed);
    assert_eq!(remote_file(&bare, "main", "tasks/t/plan.md"), "plan");
    assert_eq!(i.remote_state("p").await.unwrap().unpushed, 0);
}

#[tokio::test]
async fn push_retries_after_remote_moves() {
    let (tmp, i) = setup();
    let (bare, user) = connected(tmp.path(), &i).await;
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt, Some(NO_ENV)).await.unwrap();
    commit_file(&wt, "tasks/t/plan.md", b"plan");
    user_pushes(&user, "docs/a.md", b"a");

    let report = i.merge_task("p", "t", Some(NO_ENV), NOW).await.unwrap();

    assert_eq!(report.merge, MergeOutcome::Merged);
    assert_eq!(report.push.outcome, PushOutcome::Pushed);
    assert_eq!(remote_file(&bare, "main", "tasks/t/plan.md"), "plan");
    assert_eq!(remote_file(&bare, "main", "docs/a.md"), "a");
}

#[tokio::test]
async fn push_failure_keeps_merge_and_counts_unpushed() {
    let (tmp, i) = setup();
    let (bare, _user) = connected(tmp.path(), &i).await;
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt, Some(NO_ENV)).await.unwrap();
    commit_file(&wt, "tasks/t/plan.md", b"plan");
    let kept = take_away(&bare);

    let report = i.merge_task("p", "t", Some(NO_ENV), NOW).await.unwrap();

    assert_eq!(report.merge, MergeOutcome::Merged);
    assert!(
        matches!(report.push.outcome, PushOutcome::Failed(_)),
        "{report:?}"
    );
    assert!(i
        .files("p", "main")
        .await
        .unwrap()
        .contains(&"tasks/t/plan.md".to_string()));
    let state = i.remote_state("p").await.unwrap();
    assert!(state.unpushed >= 1, "{state:?}");
    assert!(state.error.is_some());

    restore(&kept, &bare);
    assert_eq!(
        i.push_pending("p", NO_ENV, NOW).await.unwrap().outcome,
        PushOutcome::Pushed
    );
    let state = i.remote_state("p").await.unwrap();
    assert_eq!(state.unpushed, 0);
    assert_eq!(state.error, None);
    assert_eq!(remote_file(&bare, "main", "tasks/t/plan.md"), "plan");
    // Nothing pending is still a success.
    assert_eq!(
        i.push_pending("p", NO_ENV, NOW).await.unwrap().outcome,
        PushOutcome::Pushed
    );
}

#[tokio::test]
async fn push_pending_of_a_local_repo_is_local_only() {
    let (_tmp, i) = setup();
    i.ensure("p").await.unwrap();
    assert_eq!(
        i.push_pending("p", NO_ENV, NOW).await.unwrap().outcome,
        PushOutcome::LocalOnly
    );
}

#[tokio::test]
async fn diverged_main_is_parked_then_reset() {
    let (tmp, i) = setup();
    let (bare, user) = connected(tmp.path(), &i).await;
    let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
    i.attach_task("p", "a", &a, Some(NO_ENV)).await.unwrap();
    i.attach_task("p", "b", &b, Some(NO_ENV)).await.unwrap();
    commit_file(&a, "docs/a.md", b"x");
    commit_file(&b, "tasks/b/plan.md", b"plan");
    let kept = take_away(&bare);
    let failed = i.merge_task("p", "a", Some(NO_ENV), NOW).await.unwrap();
    assert!(
        matches!(failed.push.outcome, PushOutcome::Failed(_)),
        "{failed:?}"
    );
    restore(&kept, &bare);
    user_pushes(&user, "docs/a.md", b"y");

    let report = i.merge_task("p", "b", Some(NO_ENV), NOW).await.unwrap();

    assert_eq!(report.merge, MergeOutcome::Merged);
    assert_eq!(report.push.outcome, PushOutcome::Pushed);
    let branch = report.push.parked.expect("parked");
    assert_eq!(branch, "fleet/unpushed-20231114-221320");
    assert_eq!(remote_file(&bare, &branch, "docs/a.md"), "x");
    assert_eq!(remote_file(&bare, "main", "docs/a.md"), "y");
    assert_eq!(remote_file(&bare, "main", "tasks/b/plan.md"), "plan");
    let state = i.remote_state("p").await.unwrap();
    assert_eq!(state.unpushed, 0);
    assert_eq!(state.error, None);
}

#[tokio::test]
async fn park_then_conflict_reports_both() {
    let (tmp, i) = setup();
    let (bare, user) = connected(tmp.path(), &i).await;
    let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
    i.attach_task("p", "a", &a, Some(NO_ENV)).await.unwrap();
    i.attach_task("p", "b", &b, Some(NO_ENV)).await.unwrap();
    commit_file(&a, "docs/a.md", b"x");
    commit_file(&b, "docs/a.md", b"z");
    let kept = take_away(&bare);
    i.merge_task("p", "a", Some(NO_ENV), NOW).await.unwrap();
    restore(&kept, &bare);
    user_pushes(&user, "docs/a.md", b"y");

    let report = i.merge_task("p", "b", Some(NO_ENV), NOW).await.unwrap();

    assert_eq!(
        report.merge,
        MergeOutcome::Conflict(vec!["docs/a.md".to_string()])
    );
    assert_eq!(report.push.outcome, PushOutcome::TaskBranchPushed);
    let branch = report.push.parked.expect("parked");
    assert_eq!(remote_file(&bare, &branch, "docs/a.md"), "x");
    assert_eq!(remote_file(&bare, "task/b", "docs/a.md"), "z");
    assert_eq!(remote_file(&bare, "main", "docs/a.md"), "y");
}

#[tokio::test]
async fn merge_conflict_pushes_the_task_branch() {
    let (tmp, i) = setup();
    let (bare, user) = connected(tmp.path(), &i).await;
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt, Some(NO_ENV)).await.unwrap();
    commit_file(&wt, "docs/a.md", b"from the task");
    user_pushes(&user, "docs/a.md", b"from the user");

    let report = i.merge_task("p", "t", Some(NO_ENV), NOW).await.unwrap();

    assert_eq!(
        report.merge,
        MergeOutcome::Conflict(vec!["docs/a.md".to_string()])
    );
    assert_eq!(report.push.outcome, PushOutcome::TaskBranchPushed);
    assert_eq!(report.push.parked, None);
    assert_eq!(remote_file(&bare, "task/t", "docs/a.md"), "from the task");
    let repo = i.repo_dir("p");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert_eq!(
        git(&repo, &["rev-parse", "main"]),
        git(&bare, &["rev-parse", "main"])
    );
}

#[tokio::test]
async fn attach_task_offline_starts_from_local_main() {
    let (tmp, i) = setup();
    let (bare, _user) = connected(tmp.path(), &i).await;
    std::fs::remove_dir_all(&bare).unwrap();
    let wt = tmp.path().join("wt");

    let line = i
        .attach_task("p", "t", &wt, Some(NO_ENV))
        .await
        .unwrap()
        .expect("a warning");

    assert!(line.contains("local copy"), "{line}");
    assert_eq!(git(&wt, &["branch", "--show-current"]), "task/t");
    assert!(wt.join("x.md").exists());
}

#[tokio::test]
async fn attach_task_starts_from_remote_main() {
    let (tmp, i) = setup();
    let (_bare, user) = connected(tmp.path(), &i).await;
    user_pushes(&user, "docs/b.md", b"b");
    let wt = tmp.path().join("wt");

    assert_eq!(
        i.attach_task("p", "t", &wt, Some(NO_ENV)).await.unwrap(),
        None
    );

    assert!(wt.join("docs/b.md").exists());
}

#[tokio::test]
async fn push_rejected_every_round_gives_up_and_keeps_the_merge() {
    let (tmp, i) = setup();
    let (bare, user) = connected(tmp.path(), &i).await;
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt, Some(NO_ENV)).await.unwrap();
    commit_file(&wt, "tasks/t/plan.md", b"plan");
    // Fetches read `origin.git`; pushes go to `ahead.git`, which always has a commit the fetch never brings.
    let ahead = tmp.path().join("ahead.git");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            "--bare",
            bare.to_str().unwrap(),
            ahead.to_str().unwrap(),
        ],
    );
    commit_file(&user, "docs/late.md", b"late");
    git(&user, &["push", "-q", ahead.to_str().unwrap(), "main"]);
    let env = [
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "remote.origin.pushurl"),
        ("GIT_CONFIG_VALUE_0", &remote_url_of(&ahead)),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));

    let report = i.merge_task("p", "t", Some(&env), NOW).await.unwrap();

    assert_eq!(report.merge, MergeOutcome::Merged);
    let PushOutcome::Failed(why) = &report.push.outcome else {
        panic!("{report:?}");
    };
    assert!(why.contains("rejected"), "{why}");
    let state = i.remote_state("p").await.unwrap();
    assert_eq!(state.unpushed, 2);
    assert_eq!(state.error.as_deref(), Some(why.as_str()));
}

// The remote's own `update` hook moves `main` once, after the push checked it: the remote refuses the ref update
// (`[remote rejected] ... incorrect old value`), as when another push lands meanwhile, and the merge goes back to
// the fetch.
#[cfg(unix)]
#[tokio::test]
async fn push_racing_another_push_goes_back_to_the_fetch() {
    use std::os::unix::fs::PermissionsExt;
    let (tmp, i) = setup();
    let (bare, user) = connected(tmp.path(), &i).await;
    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt, Some(NO_ENV)).await.unwrap();
    commit_file(&wt, "tasks/t/plan.md", b"plan");
    commit_file(&user, "docs/late.md", b"late");
    git(&user, &["push", "-q", "origin", "main:refs/heads/late"]);
    let marker = tmp.path().join("moved");
    let hook = bare.join("hooks/update");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\n[ \"$1\" = refs/heads/main ] && [ ! -e '{m}' ] || exit 0\ntouch '{m}'\n\
             env -u GIT_QUARANTINE_PATH git update-ref refs/heads/main refs/heads/late\n",
            m = marker.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let report = i.merge_task("p", "t", Some(NO_ENV), NOW).await.unwrap();

    assert!(marker.exists());
    assert_eq!(report.merge, MergeOutcome::Merged);
    assert_eq!(report.push.outcome, PushOutcome::Pushed);
    assert_eq!(remote_file(&bare, "main", "tasks/t/plan.md"), "plan");
    assert_eq!(remote_file(&bare, "main", "docs/late.md"), "late");
}
