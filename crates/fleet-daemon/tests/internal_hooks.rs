//! A test binary of its own: the global git config it points at is process-wide, and its failing hooks would stop
//! every other test's commits.
use fleet_daemon::internal::{Internal, MergeOutcome};
use std::path::Path;
use std::process::Command;

const HOOKS: [&str; 5] = [
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "pre-merge-commit",
    "post-merge",
];

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

#[tokio::test]
async fn user_git_hooks_never_stop_daemon_commits() {
    let tmp = tempfile::tempdir().unwrap();
    let hooks = tmp.path().join("hooks");
    let templates = tmp.path().join("templates");
    failing_hooks(&hooks);
    failing_hooks(&templates.join("hooks"));
    let config = tmp.path().join("gitconfig");
    std::fs::write(
        &config,
        format!(
            "[core]\n\thooksPath = {}\n[init]\n\ttemplateDir = {}\n",
            hooks.display(),
            templates.display()
        ),
    )
    .unwrap();
    // Set before any git runs in this process, and nothing else runs here.
    std::env::set_var("GIT_CONFIG_GLOBAL", &config);

    let i = Internal::new(tmp.path().join("internal"));
    i.ensure("p").await.unwrap();
    assert_eq!(i.history("p", "main", None, 10).await.unwrap().len(), 1);

    let wt = tmp.path().join("wt");
    i.attach_task("p", "t", &wt).await.unwrap();
    commit_file(&wt, "design.md");
    assert_eq!(i.merge_task("p", "t").await.unwrap(), MergeOutcome::Merged);
    assert!(i
        .files("p", "main")
        .await
        .unwrap()
        .contains(&"design.md".to_string()));
    assert!(!i.repo_dir("p").join(".git/MERGE_HEAD").exists());
}
