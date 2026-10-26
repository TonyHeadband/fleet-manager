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
    assert_eq!(o.kind, OrphanKind::TaskFolder);
    assert_eq!(o.id, format!("task_folder:{}", folder.display()));
    assert_eq!(o.path, folder.to_string_lossy());
    assert_eq!(o.worktrees, [folder.join("core").to_string_lossy()]);
    assert_eq!(o.bytes, None);
}

#[test]
fn a_known_task_under_an_old_root_is_not_an_orphan_of_the_new_root() {
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
fn an_archived_tasks_kept_folder_is_not_reported_again() {
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
fn a_worktree_of_a_known_task_is_not_an_orphan() {
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
            slug: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
            roster: vec![],
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
    let lost = fx.tasks_root.join("lost");
    std::fs::create_dir_all(&lost).unwrap();
    std::fs::write(lost.join("f"), vec![b'x'; 321]).unwrap();
    fx.svc.rescan_orphans().await;
    let o = fx.svc.orphans();
    assert_eq!(o.len(), 1, "{o:?}");
    assert!(eventually_orphans(&fx, |o| o[0].bytes == Some(321)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_being_spawned_is_not_an_orphan() {
    let fx = fixture().await;
    let gate = SpawnGate::install(&fx.repo, fx._tmp.path());
    let svc = fx.svc.clone();
    let spawn = tokio::spawn(async move {
        svc.spawn(SpawnRequest {
            actor: Actor("local".into()),
            repos: vec!["core".into()],
            prompt: "in flight".into(),
            slug: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
            roster: vec![],
        })
        .await
    });
    // Held inside `git worktree add`: the folder exists and no record does yet.
    assert!(
        gate.wait_reached().await,
        "the spawn never reached the gate"
    );
    let workdir = fx.tasks_root.join("in-flight");
    assert!(workdir.is_dir());
    assert!(fx.store.tasks().await.unwrap().is_empty());
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

#[tokio::test]
async fn fleet_worktrees_are_not_orphans() {
    let fx = fixture().await;
    add_origin_with_workflow(&fx, &one_step("true"));
    fx.store
        .set_repo_fleet(
            PERSONA,
            "core",
            &fleet_api::RepoFleet {
                roster: vec!["rust".into()],
            },
        )
        .await
        .unwrap();
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            repos: vec!["core".into()],
            prompt: "a fleet".into(),
            slug: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Orchestrator,
            role: Some("direct".into()),
            roster: vec![],
        })
        .await
        .unwrap();
    let (svc, id) = (fx.svc.clone(), a.id.clone());
    let held = tokio::spawn(async move {
        svc.propose_breakdown(
            &id,
            vec![fleet_api::StepDraft {
                n: None,
                title: "api".into(),
                specialist: "rust".into(),
                brief: "build the api".into(),
                depends_on: vec![],
            }],
        )
        .await
    });
    assert!(
        wait_until(|| async {
            fx.store
                .task(&a.id)
                .await
                .unwrap()
                .is_some_and(|p| !p.breakdowns.is_empty())
        })
        .await
    );
    fx.svc
        .decide_breakdown(
            &a.id,
            fleet_api::DecideBreakdown {
                approve: true,
                comment: None,
                version: None,
            },
        )
        .await
        .unwrap();
    held.await.unwrap().unwrap();
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let task = PathBuf::from(&a.workdir).parent().unwrap().to_path_buf();
    assert!(task.join("fleet/1-api/core").is_dir());

    fx.svc.rescan_orphans().await;
    assert!(fx.svc.orphans().is_empty(), "{:?}", fx.svc.orphans());
    // Across a restart and with every agent stopped, still nothing is an orphan.
    fx.tmux.kill(&session_name(&a.id)).await.unwrap();
    fx.svc.reconcile().await.unwrap();
    fx.svc.restart_orchestrator(&a.id).await.unwrap();
    fx.svc.stop_task(&a.id).await.unwrap();
    fx.svc.rescan_orphans().await;
    assert!(fx.svc.orphans().is_empty(), "{:?}", fx.svc.orphans());
}

#[tokio::test]
async fn a_tmux_error_still_reports_folder_orphans() {
    // A socket name that is far too long makes every tmux call fail with something other than "no server".
    let fx = fixture_with(fake_claude(), Some("x".repeat(300))).await;
    let lost = fx.tasks_root.join("lost");
    std::fs::create_dir_all(&lost).unwrap();
    fx.svc.rescan_orphans().await;
    let o = fx.svc.orphans();
    assert_eq!(o.len(), 1, "{o:?}");
    assert_eq!(
        (o[0].kind, o[0].path.as_str()),
        (OrphanKind::TaskFolder, &*lost.to_string_lossy())
    );
}

#[tokio::test]
async fn a_gone_worktree_orphan_has_no_size() {
    let fx = fixture().await;
    // Sized first (folders come before worktrees), so once it has a size the worktree's turn is over too.
    let lost = fx.tasks_root.join("lost");
    std::fs::create_dir_all(&lost).unwrap();
    std::fs::write(lost.join("f"), b"12345").unwrap();
    let wt = fx.tasks_root.join("gone/core");
    add_worktree(&fx.repo, &wt, "fleet/gone");
    std::fs::remove_dir_all(fx.tasks_root.join("gone")).unwrap();

    fx.svc.rescan_orphans().await;
    assert!(
        eventually_orphans(&fx, |o| o
            .iter()
            .any(|o| o.kind == OrphanKind::TaskFolder && o.bytes == Some(5)))
        .await
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let o = fx.svc.orphans();
    let gone = o
        .iter()
        .find(|o| o.kind == OrphanKind::Worktree)
        .expect("worktree orphan");
    assert_eq!(gone.bytes, None, "{o:?}");
}

/// A service on `fx`'s store whose tmux holds the next `list-sessions` (the scan's last step) until `open` exists.
struct ScanGate {
    armed: PathBuf,
    reached: PathBuf,
    open: PathBuf,
    svc: std::sync::Arc<fleet_daemon::agents::AgentService>,
}

impl ScanGate {
    fn new(fx: &Fx) -> ScanGate {
        use std::os::unix::fs::PermissionsExt;
        let dir = fx._tmp.path();
        let (armed, reached, open) = (
            dir.join("scan-armed"),
            dir.join("scan-reached"),
            dir.join("scan-open"),
        );
        let program = dir.join("gated-tmux");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\ncase \"$*\" in *list-sessions*)\n  if [ -e '{a}' ]; then rm -f '{a}'; touch '{r}'; i=0\n    while [ ! -e '{o}' ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i+1)); done\n  fi;;\nesac\nexec tmux \"$@\"\n",
                a = armed.display(),
                r = reached.display(),
                o = open.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let svc = std::sync::Arc::new(fleet_daemon::agents::AgentService::new(
            fx.svc.config().clone(),
            fx.store.clone(),
            fx.repos.clone(),
            fx.tmux.clone().with_program(program.to_string_lossy()),
            fx.svc.options().clone(),
        ));
        ScanGate {
            armed,
            reached,
            open,
            svc,
        }
    }

    fn arm(&self) {
        std::fs::write(&self.armed, b"").unwrap();
    }

    async fn wait_reached(&self) -> bool {
        wait_until(|| async { self.reached.exists() }).await
    }

    fn open(&self) {
        let _ = std::fs::write(&self.open, b"");
    }
}

impl Drop for ScanGate {
    fn drop(&mut self) {
        self.open();
    }
}

fn has_folder(o: &[fleet_api::Orphan], p: &Path) -> bool {
    o.iter().any(|o| o.path == p.to_string_lossy())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_older_scan_never_overwrites_a_newer_one() {
    let fx = fixture().await;
    let gate = ScanGate::new(&fx);
    std::fs::create_dir_all(fx.tasks_root.join("a")).unwrap();
    gate.arm();
    let svc = gate.svc.clone();
    let older = tokio::spawn(async move { svc.rescan_orphans().await });
    assert!(gate.wait_reached().await, "the scan never reached the gate");
    // The older scan has read the folders already; this one is only on disk for the newer scan.
    let b = fx.tasks_root.join("b");
    std::fs::create_dir_all(&b).unwrap();
    let svc = gate.svc.clone();
    let newer = tokio::spawn(async move { svc.rescan_orphans().await });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    gate.open();
    older.await.unwrap();
    newer.await.unwrap();
    assert!(
        has_folder(&gate.svc.orphans(), &b),
        "{:?}",
        gate.svc.orphans()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refresh_during_a_scan_queues_another() {
    let fx = fixture().await;
    let gate = ScanGate::new(&fx);
    gate.arm();
    gate.svc.request_rescan();
    assert!(gate.wait_reached().await, "the scan never reached the gate");
    let b = fx.tasks_root.join("b");
    std::fs::create_dir_all(&b).unwrap();
    gate.svc.request_rescan();
    gate.svc.request_rescan();
    gate.open();
    let svc = gate.svc.clone();
    assert!(
        wait_until(|| async { svc.rescans_run() == 2 && has_folder(&svc.orphans(), &b) }).await,
        "rescans: {}, orphans: {:?}",
        svc.rescans_run(),
        svc.orphans()
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(svc.rescans_run(), 2);
}
