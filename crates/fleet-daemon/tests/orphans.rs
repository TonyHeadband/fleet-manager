mod common;

use common::*;
use fleet_api::{Actor, OrphanKind, SpawnRequest};
use fleet_daemon::orphans::scan;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn put(path: PathBuf) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
}

fn set<T: std::hash::Hash + Eq>(items: impl IntoIterator<Item = T>) -> HashSet<T> {
    items.into_iter().collect()
}

/// A worktree of `base` at `path` on a new branch.
fn add_worktree(base: &Path, path: &Path, branch: &str) {
    git(
        base,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            &path.to_string_lossy(),
        ],
    );
}

#[test]
fn a_folder_with_no_record_is_an_orphan_with_its_worktrees() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("projects");
    let base = make_repo(&tmp.path().join("core"));
    let folder = root.join("lost");
    put(folder.join("CLAUDE.md"));
    add_worktree(&base, &folder.join("core"), "fleet/lost");
    put(root.join("known/CLAUDE.md"));
    // A file at the root is not a project folder.
    put(root.join("notes.txt"));

    let found = scan(
        &root,
        &tmp.path().join("agents"),
        &[base],
        &set([root.join("known")]),
        &HashSet::new(),
    );
    assert_eq!(found.len(), 1, "{found:?}");
    let o = &found[0];
    assert_eq!(o.kind, OrphanKind::ProjectFolder);
    assert_eq!(o.id, format!("project_folder:{}", folder.display()));
    assert_eq!(o.path, folder.to_string_lossy());
    assert_eq!(o.worktrees, [folder.join("core").to_string_lossy()]);
    assert_eq!(o.bytes, None);
}

#[test]
fn a_known_project_under_an_old_root_is_not_an_orphan_of_the_new_root() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("old/root");
    let new = tmp.path().join("new/root");
    put(old.join("p1/CLAUDE.md"));
    std::fs::create_dir_all(&new).unwrap();
    let known = set([old.join("p1")]);
    let agents = tmp.path().join("agents");
    assert!(scan(&new, &agents, &[], &known, &HashSet::new()).is_empty());
    assert!(scan(&old, &agents, &[], &known, &HashSet::new()).is_empty());
}

#[test]
fn an_archived_projects_kept_folder_is_not_reported_again() {
    // An archive that kept the folder leaves a record with that workdir; "known" covers archived records.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("projects");
    let base = make_repo(&tmp.path().join("core"));
    let kept = root.join("kept");
    put(kept.join("mine/notes.txt"));
    add_worktree(&base, &kept.join("core"), "fleet/kept");
    let found = scan(
        &root,
        &tmp.path().join("agents"),
        &[base],
        &set([kept]),
        &HashSet::new(),
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_registered_worktree_whose_folder_is_gone_is_an_orphan() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("projects");
    let base = make_repo(&tmp.path().join("core"));
    let wt = root.join("gone/core");
    add_worktree(&base, &wt, "fleet/gone");
    std::fs::remove_dir_all(root.join("gone")).unwrap();
    // A worktree outside the projects root is the user's own.
    let mine = tmp.path().join("mine");
    add_worktree(&base, &mine, "mine");

    let found = scan(
        &root,
        &tmp.path().join("agents"),
        &[base],
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, OrphanKind::Worktree);
    assert_eq!(found[0].id, format!("worktree:{}", wt.display()));
    assert_eq!(found[0].path, wt.to_string_lossy());
}

#[test]
fn a_worktree_of_a_known_project_is_not_an_orphan() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("projects");
    let base = make_repo(&tmp.path().join("core"));
    add_worktree(&base, &root.join("live/core"), "fleet/live");
    let found = scan(
        &root,
        &tmp.path().join("agents"),
        &[base],
        &set([root.join("live")]),
        &HashSet::new(),
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_agent_folder_with_no_record_is_an_orphan() {
    let tmp = tempfile::tempdir().unwrap();
    let agents = tmp.path().join("agents");
    put(agents.join("dead1/settings.json"));
    put(agents.join("live1/settings.json"));
    let found = scan(
        &tmp.path().join("projects"),
        &agents,
        &[],
        &HashSet::new(),
        &set(["live1".to_string()]),
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, OrphanKind::AgentFolder);
    assert_eq!(
        found[0].id,
        format!("agent_folder:{}", agents.join("dead1").display())
    );
}

async fn eventually_orphans(fx: &Fx, want: impl Fn(&[fleet_api::Orphan]) -> bool) -> bool {
    wait_until(|| async { want(&fx.svc.orphans()) }).await
}

#[tokio::test]
async fn rescan_finds_an_orphan_session_and_end_session_kills_it() {
    let fx = fixture().await;
    fx.tmux
        .new_session(
            "fleet-stray",
            &fx.state_dir,
            &[],
            &["sleep".into(), "60".into()],
        )
        .await
        .unwrap();
    fx.svc.rescan_orphans().await;
    let found = fx.svc.orphans();
    let o = found
        .iter()
        .find(|o| o.kind == OrphanKind::Session)
        .expect("session orphan");
    assert_eq!(
        (o.id.as_str(), o.path.as_str()),
        ("session:fleet-stray", "fleet-stray")
    );

    fx.svc.end_session("fleet-stray").await.unwrap();
    assert!(!fx.tmux.has_session("fleet-stray").await.unwrap());
    assert!(eventually_orphans(&fx, |o| o.is_empty()).await);
}

#[tokio::test]
async fn end_session_refuses_a_session_that_belongs_to_an_agent() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            repos: vec!["core".into()],
            prompt: "stay alive".into(),
            task_slug: None,
            persona: PERSONA.into(),
        })
        .await
        .unwrap();
    let session = session_name(&a.id);
    assert!(fx.svc.end_session(&session).await.is_err());
    assert!(fx.tmux.has_session(&session).await.unwrap());
    // Neither is a session that does not exist an orphan.
    assert!(fx.svc.end_session("fleet-nothere").await.is_err());
}

#[tokio::test]
async fn rescan_sizes_orphan_folders_in_the_background() {
    let fx = fixture().await;
    let lost = fx.projects_root.join("lost");
    std::fs::create_dir_all(&lost).unwrap();
    std::fs::write(lost.join("f"), vec![b'x'; 321]).unwrap();
    fx.svc.rescan_orphans().await;
    let o = fx.svc.orphans();
    assert_eq!(o.len(), 1, "{o:?}");
    assert!(eventually_orphans(&fx, |o| o[0].bytes == Some(321)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_being_spawned_is_not_an_orphan() {
    let fx = fixture().await;
    let gate = SpawnGate::install(&fx.repo, fx._tmp.path());
    let svc = fx.svc.clone();
    let spawn = tokio::spawn(async move {
        svc.spawn(SpawnRequest {
            actor: Actor("local".into()),
            repos: vec!["core".into()],
            prompt: "in flight".into(),
            task_slug: None,
            persona: PERSONA.into(),
        })
        .await
    });
    // Held inside `git worktree add`: the folder exists and no record does yet.
    assert!(
        gate.wait_reached().await,
        "the spawn never reached the gate"
    );
    let workdir = fx.projects_root.join("in-flight");
    assert!(workdir.is_dir());
    assert!(fx.store.projects().await.unwrap().is_empty());
    fx.svc.rescan_orphans().await;
    let found = fx.svc.orphans();
    assert!(
        found.is_empty(),
        "a spawning project listed as an orphan: {found:?}"
    );
    gate.open();
    let a = spawn.await.unwrap().unwrap();
    assert_eq!(std::path::PathBuf::from(&a.workdir), workdir);
}
