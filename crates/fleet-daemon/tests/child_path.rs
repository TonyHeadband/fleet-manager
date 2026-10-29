//! The daemon's PATH reaches every child it starts. It is process-wide, and this test empties the process's own
//! PATH, so it has a test binary of its own.
use fleet_api::Os;
use fleet_daemon::tools::{PathSource, Tools};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

struct FakePath(Mutex<Option<OsString>>);

impl PathSource for FakePath {
    fn user_path(&self) -> Option<OsString> {
        self.0.lock().unwrap().clone()
    }
}

fn run(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}

#[tokio::test]
async fn git_installed_while_the_daemon_runs_is_used_after_a_recheck() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run(&repo, &["init", "-q", "-b", "main"]);
    run(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "i",
        ],
    );
    let git_dir = which::which("git").unwrap().parent().unwrap().to_path_buf();
    // The daemon was started where git is on no PATH at all; the installer then adds git's folder to the user's.
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    std::env::set_var("PATH", &empty);
    let source = Arc::new(FakePath(Mutex::new(None)));
    let tools = Tools::new(
        Os::current(),
        source.clone(),
        tmp.path().join("home"),
        "gh".into(),
        vec!["claude".into()],
    )
    .with_inherited_path(Some(empty.clone().into_os_string()))
    .for_children();
    let store = Arc::new(
        fleet_daemon::store::Store::open(":memory:".as_ref())
            .await
            .unwrap(),
    );
    store
        .insert_product(&fleet_daemon::store::ProductRow {
            id: "p".into(),
            name: "p".into(),
            default_persona: None,
            internal_url: None,
            created_at: 1,
        })
        .await
        .unwrap();
    let repos = fleet_daemon::repos::Repos::new(
        store,
        tmp.path().join("state"),
        tmp.path().join("tasks"),
        tmp.path().join("home"),
    );
    let path = repo.to_string_lossy().into_owned();

    // Neither the folder picker's git (fleet-fs) nor the daemon's own finds it yet.
    assert!(repos.upsert("p", "core", Some(&path), None).await.is_err());
    assert!(fleet_daemon::git::current_branch(&repo).await.is_err());

    *source.0.lock().unwrap() = Some(git_dir.into_os_string());
    tools.check().await;
    let row = repos.upsert("p", "core", Some(&path), None).await.unwrap();
    assert!(row.path.is_some());
    assert_eq!(
        fleet_daemon::git::current_branch(&repo).await.unwrap(),
        Some("main".to_string())
    );
}
