//! A test binary of its own: the global git config it points at is process-wide, and its failing hooks would stop
//! every other test's commits.
use fleet_daemon::internal::{Internal, MergeOutcome, PushOutcome};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

const NO_ENV: &[(String, String)] = &[];

const HOOKS: [&str; 7] = [
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "pre-merge-commit",
    "post-merge",
    "post-checkout",
    "pre-push",
];

/// A path as git config reads it: forward slashes, since a backslash starts an escape there.
fn git_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn failing_hooks(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    for name in HOOKS {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\necho \"user hook $0 refuses\" >&2\nexit 1\n").unwrap();
        // The executable bit exists on Unix only; Git Bash runs a hook by its shebang regardless.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

/// The test's own commit, standing in for the orchestrator's: it skips the user's hooks so only the daemon's own
/// commits are under test.
fn commit_file(wt: &Path, name: &str) {
    std::fs::write(wt.join(name), name).unwrap();
    for args in [
        vec!["add", name],
        vec!["commit", "-q", "--no-verify", "-m", name],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(wt)
            .args(["-c", "user.name=T", "-c", "user.email=t@localhost"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(&args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Points this process's global git config at failing hooks (`core.hooksPath` and `init.templateDir`), once for
/// every test here: the variable is process-wide.
fn user_has_failing_hooks() {
    static CONFIG: OnceLock<tempfile::TempDir> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join("hooks");
        let templates = dir.path().join("templates");
        failing_hooks(&hooks);
        failing_hooks(&templates.join("hooks"));
        let config = dir.path().join("gitconfig");
        std::fs::write(
            &config,
            format!(
                "[core]\n\thooksPath = {}\n[init]\n\ttemplateDir = {}\n",
                git_path(&hooks),
                git_path(&templates)
            ),
        )
        .unwrap();
        // Set before any git runs in this process: every test calls this first, and the others wait for it.
        std::env::set_var("GIT_CONFIG_GLOBAL", &config);
        dir
    });
}

#[tokio::test]
async fn user_git_hooks_never_stop_daemon_commits() {
    user_has_failing_hooks();
    let tmp = tempfile::tempdir().unwrap();
    let i = Internal::new(tmp.path().join("internal"));
    i.ensure("p").await.unwrap();
    // Daemon commits point `core.hooksPath` at an empty folder in the state dir.
    assert!(tmp.path().join("no-hooks").is_dir());
    assert_eq!(i.history("p", "main", None, 10).await.unwrap().len(), 1);

    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt, None).await.unwrap();
    commit_file(&wt, "design.md");
    assert_eq!(
        i.merge_task("p", "t", None, 0).await.unwrap().merge,
        MergeOutcome::Merged
    );
    assert!(i
        .files("p", "main")
        .await
        .unwrap()
        .contains(&"design.md".to_string()));
    assert!(!i.repo_dir("p").join(".git/MERGE_HEAD").exists());
}

#[tokio::test]
async fn user_git_hooks_never_stop_daemon_pushes() {
    user_has_failing_hooks();
    let tmp = tempfile::tempdir().unwrap();
    let bare = tmp.path().join("origin.git");
    let out = Command::new("git")
        .args(["init", "-q", "--bare", "-b", "main"])
        .arg(&bare)
        .output()
        .unwrap();
    assert!(out.status.success());
    let url = format!("file://{}", git_path(&bare));
    let i = Internal::new(tmp.path().join("internal"));
    // An empty remote: connect seeds `main` and pushes it.
    i.connect("p", &url, NO_ENV, 0).await.unwrap();

    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt, Some(NO_ENV)).await.unwrap();
    commit_file(&wt, "design.md");
    let report = i.merge_task("p", "t", Some(&[]), 0).await.unwrap();

    assert_eq!(report.merge, MergeOutcome::Merged);
    assert_eq!(report.push.outcome, PushOutcome::Pushed);
    assert_eq!(i.remote_state("p").await.unwrap().unpushed, 0);
    // Now the remote has commits, so a clone checks `main` out.
    i.connect("q", &url, NO_ENV, 0).await.unwrap();
    assert!(i
        .files("q", "main")
        .await
        .unwrap()
        .contains(&"design.md".to_string()));
}
