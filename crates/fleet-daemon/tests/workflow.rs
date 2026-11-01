mod common;
use common::{git, make_repo};
use fleet_daemon::fleet::workflow::{
    fetch, fetch_within, parse, select, Workflow, WorkflowError, WorkflowStep,
};
use std::path::{Path, PathBuf};

fn step(name: &str, paths: &[&str]) -> WorkflowStep {
    WorkflowStep {
        name: name.into(),
        run: "true".into(),
        paths: paths.iter().map(|p| p.to_string()).collect(),
    }
}

fn wf(ignore: &[&str]) -> Workflow {
    Workflow {
        ignore: ignore.iter().map(|p| p.to_string()).collect(),
        steps: vec![
            step("fmt", &[]),
            step("gui", &["crates/gui/**"]),
            step("daemon", &["crates/daemon/**"]),
        ],
        pr: Default::default(),
        env: Default::default(),
    }
}

fn write(repo: &Path, path: &str, text: &str) {
    let p = repo.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn commit(repo: &Path, msg: &str) -> String {
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", msg]);
    git(repo, &["rev-parse", "HEAD"])
}

/// A repo with a file in each crate, plus its base commit.
fn task(dir: &Path) -> (PathBuf, String) {
    let repo = make_repo(dir);
    write(&repo, "crates/gui/x.rs", "x");
    write(&repo, "crates/daemon/d.rs", "d");
    let base = commit(&repo, "files");
    (repo, base)
}

fn names(s: &fleet_daemon::fleet::workflow::Selection) -> Vec<&str> {
    s.run.iter().map(|p| p.step.name.as_str()).collect()
}

#[tokio::test]
async fn select_runs_pathless_and_matching_steps() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = task(&tmp.path().join("r"));
    write(&repo, "crates/gui/a.rs", "a");
    let tip = commit(&repo, "gui");
    let s = select(&repo, &base, &tip, &wf(&[])).await.unwrap();
    assert_eq!(names(&s), ["fmt", "gui"]);
    assert_eq!(s.run[1].because, ["crates/gui/a.rs"]);
    assert!(s.run[0].because.is_empty());
    assert!(s.unmatched.is_empty());
    assert_eq!(s.changed, 1);
}

#[tokio::test]
async fn unmatched_file_runs_every_step() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = task(&tmp.path().join("r"));
    write(&repo, "build.rs", "b");
    let tip = commit(&repo, "root");
    let s = select(&repo, &base, &tip, &wf(&[])).await.unwrap();
    assert_eq!(names(&s), ["fmt", "gui", "daemon"]);
    assert_eq!(s.unmatched, ["build.rs"]);
}

#[tokio::test]
async fn ignored_files_neither_pick_nor_fall_back() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = task(&tmp.path().join("r"));
    write(&repo, "docs/x.md", "m");
    let tip = commit(&repo, "docs");
    let s = select(&repo, &base, &tip, &wf(&["**/*.md"])).await.unwrap();
    assert_eq!(names(&s), ["fmt"]);
    assert!(s.unmatched.is_empty());
    assert_eq!(s.changed, 1);
}

#[tokio::test]
async fn ignored_files_do_not_pick_a_step() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = task(&tmp.path().join("r"));
    write(&repo, "crates/gui/README.md", "m");
    let tip = commit(&repo, "readme");
    let s = select(&repo, &base, &tip, &wf(&["**/*.md"])).await.unwrap();
    assert_eq!(names(&s), ["fmt"]);
    assert!(s.unmatched.is_empty());
}

#[tokio::test]
async fn rename_counts_both_sides() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = task(&tmp.path().join("r"));
    git(&repo, &["mv", "crates/gui/x.rs", "crates/daemon/x.rs"]);
    let tip = commit(&repo, "mv");
    let s = select(&repo, &base, &tip, &wf(&[])).await.unwrap();
    assert_eq!(names(&s), ["fmt", "gui", "daemon"]);
    assert!(s.unmatched.is_empty());
    assert_eq!(s.changed, 2);
}

const WITH_A: &str = "steps:\n  a:\n    run: x\n";
const WITH_B: &str = "steps:\n  b:\n    run: x\n";

/// A bare origin whose `branch` carries `file` (when given), and a clone of it.
fn origin_and_clone(tmp: &Path, branch: &str, file: Option<&str>) -> (PathBuf, PathBuf) {
    let seed = make_repo(&tmp.join("seed"));
    git(&seed, &["checkout", "-q", "-B", branch]);
    if let Some(text) = file {
        write(&seed, ".fleet/workflow.yml", text);
        commit(&seed, "wf");
    }
    let origin = tmp.join("origin.git");
    git(
        tmp,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            origin.to_str().unwrap(),
        ],
    );
    git(
        &seed,
        &[
            "push",
            "-q",
            origin.to_str().unwrap(),
            &format!("{branch}:{branch}"),
        ],
    );
    let clone = tmp.join("clone");
    git(
        tmp,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    (origin, clone)
}

async fn reads_remote_not_local(branch: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let (origin, clone) = origin_and_clone(tmp.path(), branch, Some(WITH_A));
    git(&clone, &["checkout", "-q", branch]);
    write(&clone, ".fleet/workflow.yml", WITH_B);
    let local = commit(&clone, "local");
    // Origin moves on after the clone, and the clone's remote-tracking ref is pointed at its own commit, so only a
    // real fetch can make `origin/<branch>` right.
    let other = tmp.path().join("other");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    git(&other, &["checkout", "-q", branch]);
    write(&other, ".fleet/workflow.yml", "steps:\n  c:\n    run: x\n");
    commit(&other, "origin moves");
    git(&other, &["push", "-q", "origin", branch]);
    git(
        &clone,
        &[
            "update-ref",
            &format!("refs/remotes/origin/{branch}"),
            &local,
        ],
    );

    let (w, sha) = fetch(&clone, branch, &[]).await.unwrap();
    assert_eq!(w.steps.len(), 1);
    assert_eq!(w.steps[0].name, "c");
    assert_eq!(sha, git(&origin, &["rev-parse", branch]));
    assert_ne!(sha, local);
}

#[tokio::test]
async fn fetch_reads_the_remote_base_not_the_branch() {
    reads_remote_not_local("main").await;
}

#[tokio::test]
async fn fetch_handles_a_slash_branch() {
    reads_remote_not_local("release/1.0").await;
}

#[tokio::test]
async fn fetch_reports_missing_and_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, clone) = origin_and_clone(tmp.path(), "main", None);
    match fetch(&clone, "main", &[]).await {
        Err(WorkflowError::Missing { base }) => assert_eq!(base, "main"),
        other => panic!("{other:?}"),
    }
    let tmp = tempfile::tempdir().unwrap();
    let (_, clone) = origin_and_clone(tmp.path(), "main", Some("steps: {}\n"));
    assert!(matches!(
        fetch(&clone, "main", &[]).await,
        Err(WorkflowError::Invalid { .. })
    ));
}

#[tokio::test]
async fn fetch_fails_on_an_unreachable_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, clone) = origin_and_clone(tmp.path(), "main", Some(WITH_A));
    git(
        &clone,
        &["remote", "set-url", "origin", "file:///nonexistent"],
    );
    match fetch(&clone, "main", &[]).await {
        Err(WorkflowError::Fetch(msg)) => {
            assert!(
                msg.starts_with("fatal:") || msg.starts_with("error:"),
                "{msg}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn fetch_gives_up_on_a_stalled_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, clone) = origin_and_clone(tmp.path(), "main", Some(WITH_A));
    // An ssh remote whose transport never answers: git waits on it until killed.
    git(
        &clone,
        &["remote", "set-url", "origin", "ssh://stalled.invalid/x.git"],
    );
    // It records its shell's pid, to show it was ended with git.
    let pid_file = tmp.path().join("transport.pid");
    let transport = format!(
        "echo $$ > '{}'; sleep 30; true",
        pid_file.to_string_lossy().replace('\\', "/")
    );
    git(&clone, &["config", "core.sshCommand", &transport]);
    // Run inside an agent's shell, the test inherits its persona's `GIT_ALLOW_PROTOCOL=https:file`: ssh has to be let
    // back in, or git refuses the transport before it can stall.
    let ssh = [("GIT_ALLOW_PROTOCOL".to_string(), "ssh".to_string())];
    let started = std::time::Instant::now();
    let got = fetch_within(&clone, "main", &ssh, std::time::Duration::from_millis(500)).await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    match got {
        Err(e @ WorkflowError::FetchTimedOut) => {
            assert_eq!(e.to_string(), "cannot fetch the base branch: timed out")
        }
        other => panic!("{other:?}"),
    }
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let alive = || {
        std::process::Command::new(fleet_fs::posix_shell().expect("a POSIX shell"))
            .args(["-c", &format!("kill -0 {}", pid.trim())])
            .output()
            .unwrap()
            .status
            .success()
    };
    let gone_by = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while alive() && std::time::Instant::now() < gone_by {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!alive(), "the stalled transport outlived the fetch");
}

#[tokio::test]
async fn no_paths_and_no_ignore_means_nothing_fell_back() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = task(&tmp.path().join("r"));
    write(&repo, "build.rs", "b");
    let tip = commit(&repo, "root");
    let w = Workflow {
        ignore: vec![],
        steps: vec![step("fmt", &[]), step("lint", &[])],
        pr: Default::default(),
        env: Default::default(),
    };
    let s = select(&repo, &base, &tip, &w).await.unwrap();
    assert_eq!(names(&s), ["fmt", "lint"]);
    assert!(s.unmatched.is_empty());
    assert_eq!(s.changed, 1);
}

/// This repo's own gate: a mistake in it would only surface when a fleet task first ran it.
#[test]
fn this_repos_workflow_parses_with_the_steps_ci_runs() {
    let wf = parse(include_str!("../../../.fleet/workflow.yml")).unwrap();
    let names: Vec<_> = wf.steps.iter().map(|s| s.name.as_str()).collect();
    for step in ["fmt", "clippy", "daemon", "gui", "tauri", "web"] {
        assert!(
            names.contains(&step),
            "{step} is missing from .fleet/workflow.yml: {names:?}"
        );
    }
}
