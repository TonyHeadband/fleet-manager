use fleet_daemon::git::{
    branch_exists, changed_files, clone, commit_file, commit_subjects, create, diff, has_changes,
    merge_ff_only, remove, show_file, slugify, uncommitted_files, GitError,
};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Where task `name` keeps its checkout of the repo: `<root>/<task-dir>/<repo>`.
fn dest(f: &Fx, task: &str) -> PathBuf {
    f.root.join(task).join(f.repo.file_name().unwrap())
}

struct Fx {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    root: PathBuf,
}

fn fixture(repo_dir: &str) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join(repo_dir);
    let root = tmp.path().join("worktrees");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    Fx {
        _tmp: tmp,
        repo,
        root,
    }
}

#[tokio::test]
async fn creates_branch_from_head_in_new_worktree() {
    let f = fixture("core");
    let wt = create(
        &f.repo,
        "fleet",
        "fix-tests",
        &dest(&f, "fix-tests"),
        "HEAD",
        None,
    )
    .await
    .unwrap();
    assert_eq!(wt.branch, "fleet/fix-tests");
    assert!(wt.path.starts_with(&f.root) && wt.path.is_dir());
    assert_eq!(
        git(&wt.path, &["rev-parse", "HEAD"]),
        git(&f.repo, &["rev-parse", "main"])
    );
    assert_eq!(
        git(&wt.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "fleet/fix-tests"
    );
}

#[tokio::test]
async fn create_puts_worktree_at_dest_and_records_the_base_commit() {
    let f = fixture("core");
    let d = f.root.join("proj-a").join("core");
    let wt = create(&f.repo, "fleet", "proj-a", &d, "HEAD", None)
        .await
        .unwrap();
    assert_eq!(wt.path, d);
    assert!(d.join("README").is_file(), "worktree has the repo's files");
    assert_eq!(wt.base, git(&f.repo, &["rev-parse", "main"]));
}

#[tokio::test]
async fn creating_over_an_existing_branch_fails_and_leaves_the_first_intact() {
    let f = fixture("core");
    let first = create(
        &f.repo,
        "fleet",
        "fix-tests",
        &dest(&f, "fix-tests"),
        "HEAD",
        None,
    )
    .await
    .unwrap();
    assert!(branch_exists(&f.repo, "fleet/fix-tests").await.unwrap());
    assert!(!branch_exists(&f.repo, "fleet/other").await.unwrap());
    let second = f.root.join("fix-tests-2").join("core");
    assert!(create(&f.repo, "fleet", "fix-tests", &second, "HEAD", None)
        .await
        .is_err());
    assert!(!second.exists(), "failed create left a directory");
    assert!(first.path.is_dir());
}

#[tokio::test]
async fn creating_at_an_occupied_path_fails() {
    let f = fixture("core");
    let d = dest(&f, "x");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("keep.txt"), "mine").unwrap();
    assert!(create(&f.repo, "fleet", "x", &d, "HEAD", None)
        .await
        .is_err());
    assert_eq!(std::fs::read_to_string(d.join("keep.txt")).unwrap(), "mine");
    assert!(
        !branch_exists(&f.repo, "fleet/x").await.unwrap(),
        "branch left behind"
    );
}

#[tokio::test]
async fn repo_path_with_spaces_works() {
    let f = fixture("my repo");
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    assert!(wt.path.is_dir());
}

#[tokio::test]
async fn not_a_git_repo_is_typed_error_and_leaves_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let root = tmp.path().join("worktrees");
    let err = create(&plain, "fleet", "x", &root, "HEAD", None)
        .await
        .unwrap_err();
    assert!(matches!(err, GitError::NotARepo(_)), "{err}");
    assert!(!root.exists() || std::fs::read_dir(&root).unwrap().next().is_none());
}

#[tokio::test]
async fn repo_without_commits_is_an_error_and_leaves_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("empty");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    let root = tmp.path().join("worktrees");
    assert!(create(&repo, "fleet", "x", &root, "HEAD", None)
        .await
        .is_err());
    assert!(git(&repo, &["branch", "--list"]).is_empty());
    assert!(
        !root.exists()
            || std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .all(|e| std::fs::read_dir(e.path()).unwrap().next().is_none())
    );
}

#[tokio::test]
async fn dirty_main_checkout_does_not_leak_into_the_worktree() {
    let f = fixture("core");
    std::fs::write(f.repo.join("scratch.txt"), "wip").unwrap();
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    assert!(!has_changes(&wt.path).await.unwrap());
}

#[tokio::test]
async fn remove_deletes_worktree_and_branch() {
    let f = fixture("core");
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    remove(&f.repo, &wt).await.unwrap();
    assert!(!wt.path.exists());
    assert!(git(&f.repo, &["branch", "--list", "fleet/x"]).is_empty());
    assert!(!git(&f.repo, &["worktree", "list"]).contains("fleet/x"));
}

#[tokio::test]
async fn remove_works_with_uncommitted_changes() {
    let f = fixture("core");
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    std::fs::write(wt.path.join("dirty"), "x").unwrap();
    remove(&f.repo, &wt).await.unwrap();
    assert!(!wt.path.exists());
}

/// An ignored file that is not build output (`.env`, local notes) is lost with the worktree like an untracked one.
#[tokio::test]
async fn uncommitted_files_counts_ignored_files_but_not_build_output() {
    let f = fixture("core");
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    std::fs::write(wt.path.join(".gitignore"), ".env\ntarget/\n").unwrap();
    git(&wt.path, &["add", ".gitignore"]);
    git(&wt.path, &["commit", "-q", "-m", "ignore"]);
    std::fs::write(wt.path.join(".env"), "SECRET=1").unwrap();
    std::fs::create_dir(wt.path.join("target")).unwrap();
    std::fs::write(wt.path.join("target/x"), "build").unwrap();
    assert_eq!(uncommitted_files(&wt.path).await.unwrap(), 1);
}

#[tokio::test]
async fn has_changes_false_fresh_true_when_dirty_or_committed() {
    let f = fixture("core");
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    assert!(!has_changes(&wt.path).await.unwrap());
    std::fs::write(wt.path.join("new.txt"), "x").unwrap();
    assert!(has_changes(&wt.path).await.unwrap());
    git(&wt.path, &["add", "."]);
    git(&wt.path, &["commit", "-q", "-m", "work"]);
    assert!(
        has_changes(&wt.path).await.unwrap(),
        "committed work counts"
    );
}

#[tokio::test]
async fn diff_is_empty_for_a_fresh_worktree() {
    let f = fixture("core");
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    let d = diff(&wt.path, &wt.base, 1 << 20).await.unwrap();
    assert_eq!((d.text.as_str(), d.truncated), ("", false));
}

#[tokio::test]
async fn diff_shows_committed_uncommitted_and_untracked_changes() {
    let f = fixture("core");
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    std::fs::write(wt.path.join("committed.txt"), "from a commit\n").unwrap();
    git(&wt.path, &["add", "."]);
    git(&wt.path, &["commit", "-q", "-m", "work"]);
    std::fs::write(wt.path.join("README"), "hi\nedited line\n").unwrap();
    std::fs::write(wt.path.join("untracked.txt"), "brand new\n").unwrap();
    let d = diff(&wt.path, &wt.base, 1 << 20).await.unwrap().text;
    for needle in [
        "committed.txt",
        "+from a commit",
        "+edited line",
        "untracked.txt",
        "+brand new",
    ] {
        assert!(d.contains(needle), "missing {needle:?} in:\n{d}");
    }
    assert!(
        git(&wt.path, &["status", "--porcelain"]).contains("?? untracked.txt"),
        "diff must not stage anything"
    );
}

#[tokio::test]
async fn diff_is_cut_at_the_cap_on_a_character_boundary() {
    let f = fixture("core");
    let wt = create(&f.repo, "fleet", "x", &dest(&f, "x"), "HEAD", None)
        .await
        .unwrap();
    std::fs::write(wt.path.join("big.txt"), "é".repeat(5000)).unwrap();
    let d = diff(&wt.path, &wt.base, 1000).await.unwrap();
    assert!(d.truncated);
    assert!(d.text.len() <= 1000);
    assert!(d.text.contains("big.txt"));
}

#[test]
fn slugify_strips_shell_metacharacters_and_caps_length() {
    assert_eq!(slugify("fix; rm -rf /"), "fix-rm-rf");
    assert_eq!(
        slugify("  Add   $(whoami) `id` support!  "),
        "add-whoami-id-support"
    );
    assert_eq!(slugify("-- --leading"), "leading");
    assert_eq!(slugify(&"a".repeat(200)).len(), 40);
    assert_eq!(slugify("!!!"), "task");
    assert_eq!(slugify(""), "task");
    assert_eq!(slugify("héllo wörld"), "h-llo-w-rld");
    assert!(
        !slugify(&"word ".repeat(20)).ends_with('-'),
        "cut at 40 must not leave a trailing dash"
    );
}

#[tokio::test]
async fn clone_uses_persona_env() {
    let f = fixture("up");
    let real = format!("file://{}", f.repo.display());
    let fake = "https://example.invalid/x/y";
    let env: Vec<(String, String)> = [
        ("GIT_CONFIG_COUNT", "1".to_string()),
        ("GIT_CONFIG_KEY_0", format!("url.{real}.insteadOf")),
        ("GIT_CONFIG_VALUE_0", fake.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let dest = f.root.join("clone");
    std::fs::create_dir_all(&f.root).unwrap();
    // Only the env's rewrite makes this URL reachable, so a successful clone proves git got it.
    clone(fake, &dest, &env, None).await.unwrap();
    assert!(dest.join("README").exists());
    let config = std::fs::read_to_string(dest.join(".git/config")).unwrap();
    assert!(!config.contains("insteadOf"), "{config}");
    assert!(!config.contains("insteadof"), "{config}");
    assert!(matches!(
        clone(fake, &f.root.join("plain"), &[], None).await,
        Err(GitError::Git { .. })
    ));
}

#[tokio::test]
async fn changed_files_keeps_spaces_around_names() {
    let f = fixture("core");
    let from = git(&f.repo, &["rev-parse", "HEAD"]);
    // ` notes.md` sorts first and `z ` last, where a trim of the whole output would cut them.
    // Straight into the index: Windows drops a trailing space from a file name on disk, and Git for Windows refuses
    // one in the index unless told this repo is not checked out on NTFS.
    let src = f.repo.join(".git").join("blob-src");
    std::fs::write(&src, "x").unwrap();
    let blob = git(&f.repo, &["hash-object", "-w", src.to_str().unwrap()]);
    for name in [" notes.md", "mid.txt", "z "] {
        let entry = format!("100644,{},{name}", blob.trim());
        git(
            &f.repo,
            &[
                "-c",
                "core.protectNTFS=false",
                "update-index",
                "--add",
                "--cacheinfo",
                &entry,
            ],
        );
    }
    git(&f.repo, &["commit", "-q", "-m", "spaces"]);
    let got = changed_files(&f.repo, &from, "HEAD", &[]).await.unwrap();
    assert_eq!(got, [" notes.md", "mid.txt", "z "]);
}

#[tokio::test]
async fn show_file_keeps_the_text_as_is() {
    let f = fixture("core");
    std::fs::write(f.repo.join("f.txt"), "  indented\n\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-q", "-m", "f"]);
    assert_eq!(
        show_file(&f.repo, "HEAD", "f.txt")
            .await
            .unwrap()
            .as_deref(),
        Some("  indented\n\n")
    );
    assert_eq!(show_file(&f.repo, "HEAD", "none.txt").await.unwrap(), None);
    assert!(matches!(
        show_file(&f.repo, "no-such-rev", "f.txt").await,
        Err(GitError::Git { .. })
    ));
}

#[tokio::test]
async fn merge_ff_only_is_false_only_for_diverged_branches() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    let no_hooks = tempfile::tempdir().unwrap();
    let no_hooks = no_hooks.path();
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["commit", "-q", "--allow-empty", "-m", "base"]);
    git(repo, &["branch", "ahead"]);
    git(repo, &["checkout", "-q", "ahead"]);
    git(repo, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
    git(repo, &["checkout", "-q", "main"]);
    assert!(merge_ff_only(repo, no_hooks, "ahead").await.unwrap());
    assert_eq!(
        git(repo, &["rev-parse", "main"]),
        git(repo, &["rev-parse", "ahead"])
    );

    git(repo, &["commit", "-q", "--allow-empty", "-m", "main only"]);
    git(repo, &["checkout", "-q", "-b", "other", "ahead~1"]);
    git(repo, &["commit", "-q", "--allow-empty", "-m", "other only"]);
    git(repo, &["checkout", "-q", "main"]);
    let before = git(repo, &["rev-parse", "HEAD"]);
    assert!(!merge_ff_only(repo, no_hooks, "other").await.unwrap());
    assert_eq!(git(repo, &["rev-parse", "HEAD"]), before);

    assert!(merge_ff_only(repo, no_hooks, "no-such-rev").await.is_err());
}

#[tokio::test]
async fn commit_file_commits_only_its_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    let no_hooks = tempfile::tempdir().unwrap();
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["commit", "-q", "--allow-empty", "-m", "base"]);
    std::fs::write(repo.join("other.md"), "staged by someone else").unwrap();
    git(repo, &["add", "other.md"]);

    commit_file(
        repo,
        no_hooks.path(),
        "notes/plan.md",
        "plan",
        "add the plan",
    )
    .await
    .unwrap();

    assert_eq!(
        git(repo, &["show", "--name-only", "--format=", "HEAD"]),
        "notes/plan.md"
    );
    assert_eq!(git(repo, &["diff", "--cached", "--name-only"]), "other.md");
}

#[tokio::test]
async fn commit_subjects_lists_the_range_oldest_first() {
    let f = fixture("core");
    let repo = &f.repo;
    let base = git(repo, &["rev-parse", "HEAD"]);
    git(
        repo,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "fix(daemon): first",
            "-m",
            "body line",
        ],
    );
    let first = git(repo, &["rev-parse", "--short", "HEAD"]);
    git(
        repo,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "Roll back failed transactions",
        ],
    );
    let head = git(repo, &["rev-parse", "HEAD"]);

    let got = commit_subjects(repo, &base, &head).await.unwrap();
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0].subject, "fix(daemon): first");
    assert_eq!(got[0].sha, first);
    assert_eq!(got[1].subject, "Roll back failed transactions");
    assert!(commit_subjects(repo, &head, &head)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn commit_subjects_skips_merge_commits() {
    let f = fixture("core");
    let repo = &f.repo;
    let base = git(repo, &["rev-parse", "HEAD"]);
    git(repo, &["checkout", "-q", "-b", "side"]);
    git(
        repo,
        &["commit", "-q", "--allow-empty", "-m", "fix(daemon): side"],
    );
    git(repo, &["checkout", "-q", "main"]);
    git(
        repo,
        &["commit", "-q", "--allow-empty", "-m", "fix(daemon): main"],
    );
    git(
        repo,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'side'",
            "side",
        ],
    );
    let head = git(repo, &["rev-parse", "HEAD"]);

    let mut subjects: Vec<String> = commit_subjects(repo, &base, &head)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.subject)
        .collect();
    subjects.sort();
    assert_eq!(subjects, ["fix(daemon): main", "fix(daemon): side"]);
}
