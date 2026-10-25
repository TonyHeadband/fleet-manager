mod common;

use common::*;
use fleet_api::{
    Actor, Agent, AgentId, ArchiveReport, EventKind, OrphanKind, ProjectState, SpawnRequest, Status,
};
use fleet_daemon::agents::{AgentService, ServiceError, CLAUDE_MD_HEADER, PROMPT_FILE};
use fleet_daemon::git;
use fleet_daemon::ingest::{authenticate, ingest, IngestError};
use serde_json::json;
use std::path::{Path, PathBuf};

fn req_with(prompt: &str, repos: &[&str]) -> SpawnRequest {
    SpawnRequest {
        actor: Actor("local".into()),
        repos: repos.iter().map(|r| r.to_string()).collect(),
        prompt: prompt.into(),
        task_slug: None,
    }
}

async fn stopped(fx: &Fx, prompt: &str, repos: &[&str]) -> Agent {
    let a = fx.svc.spawn(req_with(prompt, repos)).await.unwrap();
    fx.svc.stop(&a.id).await.unwrap();
    a
}

/// Archives against a plan made just now, as a user confirming at once would.
async fn archive_now(fx: &Fx, id: &AgentId) -> Result<ArchiveReport, ServiceError> {
    let plan = fx.svc.archive_plan(id).await?;
    fx.svc.archive(id, &plan.items).await
}

async fn archive_orphan_now(fx: &Fx, id: &str) -> Result<ArchiveReport, ServiceError> {
    let plan = fx.svc.orphan_plan(id).await?;
    fx.svc.archive_orphan(id, &plan.items).await
}

async fn find_opt(svc: &AgentService, id: &AgentId) -> Option<Agent> {
    svc.agent(id).await.ok()
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[tokio::test]
async fn archive_refuses_a_running_project() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("busy", &["core"])).await.unwrap();
    assert!(matches!(
        fx.svc.archive(&a.id, &[]).await,
        Err(ServiceError::NotStopped)
    ));
    assert!(matches!(
        fx.svc.archive_plan(&a.id).await,
        Err(ServiceError::NotStopped)
    ));
    assert!(PathBuf::from(&a.workspaces[0].worktree).is_dir());
    assert!(PathBuf::from(&a.workdir).join(PROMPT_FILE).is_file());
    assert!(fx.state_dir.join("agents").join(&a.id.0).is_dir());
    assert_ne!(
        find_opt(&fx.svc, &a.id).await.unwrap().status,
        Status::Stopped
    );
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!((p.state, p.archive), (ProjectState::Running, None));
}

#[tokio::test]
async fn archive_of_an_unknown_agent_is_not_found() {
    let fx = fixture().await;
    let ghost = AgentId("ghost".into());
    assert!(matches!(
        fx.svc.archive(&ghost, &[]).await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        fx.svc.archive_plan(&ghost).await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn the_plan_counts_uncommitted_files() {
    let fx = fixture().await;
    let a = stopped(&fx, "dirty", &["core"]).await;
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    std::fs::write(wt.join("one.txt"), "1").unwrap();
    std::fs::create_dir(wt.join("sub")).unwrap();
    std::fs::write(wt.join("sub/two.txt"), "22").unwrap();
    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    let paths: Vec<&str> = plan.items.iter().map(|i| i.path.as_str()).collect();
    let daemon_dir = fx.state_dir.join("agents").join(&a.id.0);
    let workdir = PathBuf::from(&a.workdir);
    // Everything archive deletes is named on its own, so the user sees each path before confirming.
    assert_eq!(
        paths,
        [
            s(&wt),
            s(&daemon_dir),
            s(&workdir.join("CLAUDE.md")),
            s(&workdir.join(PROMPT_FILE)),
            a.workdir.clone()
        ]
    );
    assert_eq!(plan.items[0].uncommitted_files, 2);
    for i in &plan.items[..4] {
        assert!(i.bytes > 0, "{i:?}");
    }
    assert_eq!(
        plan.items[4].bytes, 0,
        "the folder holds nothing but what goes"
    );
    let stays: Vec<bool> = plan.items.iter().map(|i| i.stays).collect();
    assert_eq!(
        stays,
        [false, false, false, false, true],
        "only the folder stays"
    );
    assert_eq!(
        plan.total,
        plan.items[..4].iter().map(|i| i.bytes).sum::<u64>()
    );
    // Making the plan removes nothing.
    assert!(wt.join("sub/two.txt").is_file());
    assert!(daemon_dir.is_dir());
}

#[tokio::test]
async fn the_plan_counts_ignored_files_but_not_build_output() {
    let fx = fixture().await;
    let a = stopped(&fx, "ignored", &["core"]).await;
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    std::fs::write(wt.join(".gitignore"), ".env\ntarget/\n").unwrap();
    common::git(&wt, &["add", ".gitignore"]);
    common::git(&wt, &["commit", "-q", "-m", "ignore"]);
    std::fs::write(wt.join(".env"), "SECRET=1").unwrap();
    std::fs::create_dir(wt.join("target")).unwrap();
    std::fs::write(wt.join("target/x"), "build").unwrap();
    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    assert_eq!(plan.items[0].path, s(&wt));
    assert_eq!(plan.items[0].uncommitted_files, 1);
}

#[tokio::test]
async fn archive_removes_the_worktree_keeps_the_branch_and_the_record() {
    let fx = fixture().await;
    let a = stopped(&fx, "fix login", &["core"]).await;
    let (wt, base) = (a.workspaces[0].worktree.clone(), fx.repo.clone());
    std::fs::write(Path::new(&wt).join("done.txt"), "work").unwrap();
    git(Path::new(&wt), &["add", "done.txt"]);
    git(Path::new(&wt), &["commit", "-q", "-m", "the agent's work"]);
    std::fs::write(Path::new(&wt).join("draft.txt"), "lost").unwrap();
    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    assert_eq!(plan.items[0].uncommitted_files, 1);
    let mut events = fx.store.subscribe();

    let r = fx.svc.archive(&a.id, &plan.items).await.unwrap();

    assert!(!Path::new(&wt).exists());
    assert!(git::branch_exists(&base, "fleet/fix-login").await.unwrap());
    assert_eq!(
        git(&base, &["log", "-1", "--format=%s", "fleet/fix-login"]),
        "the agent's work",
        "the branch keeps the agent's commits"
    );
    assert!(
        !git(&base, &["worktree", "list"]).contains("fix-login"),
        "worktree left registered"
    );
    assert!(!Path::new(&a.workdir).exists());
    assert!(!fx.state_dir.join("agents").join(&a.id.0).exists());
    assert!(find_opt(&fx.svc, &a.id).await.is_none());
    assert!(matches!(
        events.try_recv().unwrap().kind,
        EventKind::AgentRemoved { id } if id == a.id
    ));
    assert_eq!(r.kept, None);
    assert_eq!(r.freed, plan.total);
    assert!(r.removed.contains(&wt));
    assert!(r.removed.contains(&a.workdir));
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, ProjectState::Archived);
    assert_eq!(p.archived_at, Some(fx.clock.now_for_test()));
    assert_eq!(p.archive.unwrap(), r);
    let fp = p.footprint.expect("the plan's measurement");
    assert_eq!(fp.total, plan.total);
    assert!(p.log.last().unwrap().text.starts_with("archived, "));
    assert!(
        p.log.last().unwrap().text.ends_with(" freed"),
        "{:?}",
        p.log.last()
    );
}

#[tokio::test]
async fn archive_refuses_a_plan_the_folder_no_longer_matches() {
    let fx = fixture().await;
    let a = stopped(&fx, "moving", &["core"]).await;
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    let confirmed = fx.svc.archive_plan(&a.id).await.unwrap();
    // The user confirmed losing nothing; this file would be lost unseen.
    std::fs::write(wt.join("late.txt"), "new work").unwrap();

    assert!(matches!(
        fx.svc.archive(&a.id, &confirmed.items).await,
        Err(ServiceError::PlanChanged)
    ));
    assert!(wt.join("late.txt").is_file(), "nothing is removed");
    assert!(PathBuf::from(&a.workdir).join(PROMPT_FILE).is_file());
    assert!(fx.state_dir.join("agents").join(&a.id.0).is_dir());
    assert!(find_opt(&fx.svc, &a.id).await.is_some(), "the card stays");
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!((p.state, p.archive), (ProjectState::Stopped, None));
    assert!(
        p.log.iter().all(|l| !l.text.starts_with("archive failed")),
        "nothing failed: {:?}",
        p.log
    );

    // A repo attached since the plan is a path the user never saw go.
    let fresh = fx.svc.archive_plan(&a.id).await.unwrap();
    assert_eq!(fresh.items[0].uncommitted_files, 1);
    let tools = make_repo(&fx._tmp.path().join("tools"));
    fx.svc
        .attach_repo(&a.id, tools.to_str().unwrap())
        .await
        .unwrap();
    assert!(matches!(
        fx.svc.archive(&a.id, &fresh.items).await,
        Err(ServiceError::PlanChanged)
    ));
    assert!(wt.is_dir());

    // Sizes alone may move: the plan still matches when only bytes differ.
    let mut again = fx.svc.archive_plan(&a.id).await.unwrap();
    for i in &mut again.items {
        i.bytes += 1;
    }
    let r = fx.svc.archive(&a.id, &again.items).await.unwrap();
    assert_eq!(r.kept, None);
    assert!(!wt.exists());
}

#[tokio::test]
async fn archive_keeps_a_folder_holding_the_users_files() {
    let fx = fixture().await;
    let a = stopped(&fx, "notes", &["core"]).await;
    let workdir = PathBuf::from(&a.workdir);
    std::fs::write(workdir.join("notes.txt"), "mine").unwrap();
    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    assert_eq!(plan.items.last().unwrap().path, a.workdir);
    assert!(plan.items.last().unwrap().stays);
    assert_eq!(
        plan.items.last().unwrap().bytes,
        4,
        "notes.txt stays, and the plan says so"
    );
    let r = fx.svc.archive(&a.id, &plan.items).await.unwrap();
    let kept = format!("kept {}: it contains notes.txt", workdir.display());
    assert_eq!(r.kept.as_deref(), Some(kept.as_str()));
    assert_eq!(entries(&workdir), ["notes.txt"]);
    assert_eq!(
        std::fs::read_to_string(workdir.join("notes.txt")).unwrap(),
        "mine"
    );
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.log.last().unwrap().text, format!("archived, {kept}"));
}

#[tokio::test]
async fn archive_never_deletes_through_a_symlink() {
    let fx = fixture().await;
    let outside = fx._tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious"), "keep").unwrap();
    let a = stopped(&fx, "links", &["core"]).await;
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    let workdir = PathBuf::from(&a.workdir);
    // As a `node_modules` link would: inside the worktree, pointing out of the project.
    std::os::unix::fs::symlink(&outside, wt.join("node_modules")).unwrap();
    std::os::unix::fs::symlink(&outside, workdir.join("shared")).unwrap();
    // Locked, so git refuses and the by-hand fallback runs over the link too.
    git(&fx.repo, &["worktree", "lock", &s(&wt)]);

    let r = archive_now(&fx, &a.id).await.unwrap();

    assert!(!wt.exists());
    assert_eq!(
        std::fs::read_to_string(outside.join("precious")).unwrap(),
        "keep"
    );
    assert_eq!(entries(&outside), ["precious"]);
    // The link in the folder is not the daemon's: it keeps the folder, the target is untouched.
    assert_eq!(entries(&workdir), ["shared"]);
    assert!(r.kept.unwrap().contains("it contains shared"));
}

#[tokio::test]
async fn archive_keeps_a_worktree_that_is_a_symlink() {
    let fx = fixture().await;
    let outside = fx._tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious"), "keep").unwrap();
    let a = stopped(&fx, "swapped", &["core"]).await;
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    std::fs::rename(&wt, fx._tmp.path().join("moved")).unwrap();
    std::os::unix::fs::symlink(&outside, &wt).unwrap();

    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    assert!(
        plan.items.iter().all(|i| i.path != s(&wt)),
        "a link is not listed as going: {plan:?}"
    );
    let r = fx.svc.archive(&a.id, &plan.items).await.unwrap();

    assert_eq!(entries(&outside), ["precious"]);
    assert!(std::fs::symlink_metadata(&wt)
        .unwrap()
        .file_type()
        .is_symlink());
    let kept = r.kept.unwrap();
    assert!(
        kept.contains(&format!("kept {}: it is a symbolic link", wt.display())),
        "{kept}"
    );
    assert!(!r.removed.contains(&s(&wt)));
}

#[tokio::test]
async fn archive_refuses_a_project_folder_that_is_a_symlink() {
    let fx = fixture().await;
    let a = stopped(&fx, "relinked", &["core"]).await;
    let workdir = PathBuf::from(&a.workdir);
    let elsewhere = fx._tmp.path().join("elsewhere");
    std::fs::rename(&workdir, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &workdir).unwrap();
    let before = entries(&elsewhere);

    assert!(matches!(
        fx.svc.archive(&a.id, &[]).await,
        Err(ServiceError::Removal(_))
    ));
    assert_eq!(entries(&elsewhere), before);
    assert!(elsewhere.join("core/README").is_file());
    assert!(find_opt(&fx.svc, &a.id).await.is_some());
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, ProjectState::Stopped);
    assert!(p.log.last().unwrap().text.starts_with("archive failed: "));
}

/// Gives a directory back its write permission when the test ends, even on a panic, so the tempdir can be removed.
struct Writable(PathBuf);

impl Drop for Writable {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

#[tokio::test]
async fn a_failed_archive_leaves_the_record_unarchived() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture().await;
    let a = stopped(&fx, "stuck", &["core"]).await;
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    let ro = wt.join("ro");
    std::fs::create_dir(&ro).unwrap();
    std::fs::write(ro.join("f"), "x").unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    let restore = Writable(ro.clone());
    let log_len = fx.store.project(&a.id).await.unwrap().unwrap().log.len();

    let err = archive_now(&fx, &a.id).await.unwrap_err();

    assert!(matches!(err, ServiceError::Removal(_)), "{err}");
    let card = find_opt(&fx.svc, &a.id).await.expect("the card stays");
    assert_eq!(card.status, Status::Stopped);
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.archive, p.archived_at),
        (ProjectState::Stopped, None, None)
    );
    assert_eq!(p.log.len(), log_len + 1);
    assert_eq!(p.log.last().unwrap().text, format!("archive failed: {err}"));
    assert!(err.to_string().contains(&s(&wt)), "{err}");
    // Nothing after the failing worktree was touched, and the next plan shows what is left.
    assert!(fx.state_dir.join("agents").join(&a.id.0).is_dir());
    assert!(PathBuf::from(&a.workdir).join(PROMPT_FILE).is_file());
    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    assert_eq!(plan.items[0].path, s(&wt));

    drop(restore);
    let r = fx.svc.archive(&a.id, &plan.items).await.unwrap();
    assert_eq!(r.kept, None);
    assert!(!PathBuf::from(&a.workdir).exists());
    assert_eq!(
        fx.store.project(&a.id).await.unwrap().unwrap().state,
        ProjectState::Archived
    );
}

#[tokio::test]
async fn a_hook_for_an_archived_agent_is_refused() {
    let fx = fixture().await;
    let a = stopped(&fx, "done", &["core"]).await;
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    archive_now(&fx, &a.id).await.unwrap();
    let r = ingest(
        &fx.store,
        &a.id,
        Some(&token),
        json!({"hook_event_name": "SessionStart"}),
        fx.clock.now_for_test(),
    )
    .await;
    assert!(matches!(r, Err(IngestError::UnknownAgent)), "{r:?}");
    // The MCP route authenticates the same way.
    assert!(matches!(
        authenticate(&fx.store, &a.id, Some(&token)).await,
        Err(IngestError::UnknownAgent)
    ));
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, ProjectState::Archived);
    assert!(find_opt(&fx.svc, &a.id).await.is_none());
}

// Ported from the Kill tests removed with Kill (e53c905): the same removal, now behind Archive.

#[tokio::test]
async fn archive_keeps_an_owned_repo_and_reports_it() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("keep it", &["core"])).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    make_repo(&workdir.join("newtool"));
    fx.svc.reconcile().await.unwrap();
    assert!(agent(&fx, &a.id).await.workspaces.iter().any(|w| w.owned));
    fx.svc.stop(&a.id).await.unwrap();
    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    assert!(
        plan.items.iter().all(|i| !i.path.ends_with("/newtool")),
        "an owned repo is not listed as going: {plan:?}"
    );
    assert!(
        plan.items.last().unwrap().bytes > 0,
        "the owned repo stays in the folder"
    );
    let report = fx.svc.archive(&a.id, &plan.items).await.unwrap();
    assert_eq!(
        report.kept,
        Some(format!("kept {}: it contains newtool", workdir.display()))
    );
    assert!(
        workdir.join("newtool/README").is_file(),
        "owned repo destroyed"
    );
    assert_eq!(
        git(&workdir.join("newtool"), &["log", "--oneline"])
            .lines()
            .count(),
        1
    );
    assert_eq!(
        entries(&workdir),
        ["newtool"],
        "the daemon's own files and the worktree must go"
    );
    assert_eq!(
        git(&fx.repo, &["branch", "--list", "fleet/*"]),
        "fleet/keep-it"
    );
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn archive_keeps_files_it_did_not_create_even_before_reconcile() {
    let fx = fixture().await;
    let a = stopped(&fx, "stray", &[]).await;
    let workdir = PathBuf::from(&a.workdir);
    make_repo(&workdir.join("young"));
    std::fs::write(workdir.join("notes.txt"), "mine").unwrap();
    let report = archive_now(&fx, &a.id).await.unwrap();
    assert_eq!(
        report.kept,
        Some(format!(
            "kept {}: it contains notes.txt, young",
            workdir.display()
        ))
    );
    assert_eq!(entries(&workdir), ["notes.txt", "young"]);
}

#[tokio::test]
async fn archive_of_a_plain_project_removes_the_folder() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let a = stopped(&fx, "plain", &["core", tools.to_str().unwrap()]).await;
    let report = archive_now(&fx, &a.id).await.unwrap();
    assert_eq!(report.kept, None);
    assert!(!PathBuf::from(&a.workdir).exists());
    assert!(entries(&fx.projects_root).is_empty());
    assert_eq!(git(&tools, &["branch", "--list", "fleet/*"]), "fleet/plain");
    assert_eq!(
        git(&fx.repo, &["branch", "--list", "fleet/*"]),
        "fleet/plain"
    );
    assert!(
        !git(&tools, &["worktree", "list"]).contains("plain"),
        "worktree left registered"
    );
    assert!(!fx.state_dir.join("agents").join(&a.id.0).exists());
}

#[tokio::test]
async fn archive_finds_the_base_of_a_repo_that_is_no_longer_remembered() {
    let fx = fixture().await;
    let a = stopped(&fx, "forgotten", &["core"]).await;
    fx.store
        .update_agent(&a.id, |x| x.workspaces[0].repo = "gone".into())
        .await
        .unwrap();
    // The worktree's own .git file names its base, so git removes it as usual.
    assert_eq!(archive_now(&fx, &a.id).await.unwrap().kept, None);
    assert!(!PathBuf::from(&a.workdir).exists());
    assert!(
        !git(&fx.repo, &["worktree", "list"]).contains("forgotten"),
        "worktree left registered"
    );
    assert_eq!(
        git(&fx.repo, &["branch", "--list", "fleet/*"]),
        "fleet/forgotten"
    );
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn archive_removes_a_worktree_git_refuses_to() {
    let fx = fixture().await;
    let a = stopped(&fx, "locked", &["core"]).await;
    let wt = &a.workspaces[0].worktree;
    // A locked worktree makes `git worktree remove --force` fail.
    git(&fx.repo, &["worktree", "lock", wt]);
    assert_eq!(archive_now(&fx, &a.id).await.unwrap().kept, None);
    assert!(!PathBuf::from(wt).exists());
    assert!(!PathBuf::from(&a.workdir).exists());
    assert_eq!(
        git(&fx.repo, &["branch", "--list", "fleet/*"]),
        "fleet/locked",
        "the branch holds the agent's commits"
    );
    assert!(
        !git(&fx.repo, &["worktree", "list"]).contains("locked"),
        "worktree left registered"
    );
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn a_new_project_avoids_a_name_whose_branch_survived_an_archive() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let old = stopped(&fx, "again", &[tools.to_str().unwrap()]).await;
    archive_now(&fx, &old.id).await.unwrap();
    // `tools` is remembered but not requested: its surviving `fleet/again` still blocks the name.
    let a = fx.svc.spawn(req_with("again", &[])).await.unwrap();
    assert_eq!(PathBuf::from(&a.workdir), fx.projects_root.join("again-2"));
    let w = fx.svc.attach_repo(&a.id, "tools").await.unwrap();
    assert_eq!(w.branch, "fleet/again-2");
}

/// Was `client_kill_reports_what_was_kept`; the client side comes with the archive routes.
#[tokio::test]
async fn archive_reports_what_was_kept() {
    let fx = fixture().await;
    let first = stopped(&fx, "x", &["core"]).await;
    std::fs::write(PathBuf::from(&first.workdir).join("notes.txt"), "mine").unwrap();
    let kept = archive_now(&fx, &first.id)
        .await
        .unwrap()
        .kept
        .expect("kept");
    assert!(
        kept.contains("kept") && kept.contains("notes.txt"),
        "{kept}"
    );
    let p = fx.store.project(&first.id).await.unwrap().unwrap();
    assert_eq!(p.log.last().unwrap().text, format!("archived, {kept}"));

    let second = stopped(&fx, "y", &["core"]).await;
    let r: ArchiveReport = archive_now(&fx, &second.id).await.unwrap();
    assert_eq!(r.kept, None);
    let p = fx.store.project(&second.id).await.unwrap().unwrap();
    assert_eq!(
        p.log.last().unwrap().text,
        format!("archived, {} freed", fleet_api::format_bytes(r.freed))
    );
}

// Orphans.

/// A folder the daemon made and lost the record of: its CLAUDE.md, PROMPT.md and a worktree of `core`.
fn lost_project(fx: &Fx, name: &str) -> PathBuf {
    let folder = fx.projects_root.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("CLAUDE.md"),
        format!("{CLAUDE_MD_HEADER}\nmore\n"),
    )
    .unwrap();
    std::fs::write(folder.join(PROMPT_FILE), "the task\n").unwrap();
    git(
        &fx.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            &format!("fleet/{name}"),
            &s(&folder.join("core")),
        ],
    );
    folder
}

#[tokio::test]
async fn archiving_an_orphan_folder_creates_an_archived_record() {
    let fx = fixture().await;
    let folder = lost_project(&fx, "lost");
    std::fs::write(folder.join("core/draft.txt"), "x").unwrap();
    fx.svc.rescan_orphans().await;
    let id = format!("project_folder:{}", folder.display());
    assert!(fx.svc.orphans().iter().any(|o| o.id == id));

    let plan = fx.svc.orphan_plan(&id).await.unwrap();
    let paths: Vec<&str> = plan.items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            s(&folder.join("core")),
            s(&folder.join("CLAUDE.md")),
            s(&folder.join(PROMPT_FILE)),
            s(&folder)
        ]
    );
    assert_eq!(plan.items[0].uncommitted_files, 1);
    assert!(plan.items[1].bytes > 0 && plan.items[2].bytes > 0);
    assert_eq!(plan.items[3].bytes, 0);
    let stays: Vec<bool> = plan.items.iter().map(|i| i.stays).collect();
    assert_eq!(stays, [false, false, false, true]);

    let r = fx.svc.archive_orphan(&id, &plan.items).await.unwrap();

    assert_eq!(r.kept, None);
    assert_eq!(r.freed, plan.total);
    assert!(!folder.exists());
    assert!(git::branch_exists(&fx.repo, "fleet/lost").await.unwrap());
    assert!(!git(&fx.repo, &["worktree", "list"]).contains("lost"));
    let records = fx.store.projects().await.unwrap();
    assert_eq!(records.len(), 1);
    let p = &records[0];
    assert_eq!(
        (p.name.as_str(), p.workdir.as_str(), p.state),
        ("lost", folder.to_str().unwrap(), ProjectState::Archived)
    );
    assert_eq!(p.id.0.len(), 32);
    assert_eq!(p.archive.as_ref(), Some(&r));
    let log: Vec<&str> = p.log.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(log[0], "found as orphan");
    assert!(log[1].starts_with("archived, ") && log[1].ends_with(" freed"));
    assert!(p.footprint.is_some());
    assert!(fx.svc.orphans().is_empty(), "{:?}", fx.svc.orphans());
}

#[tokio::test]
async fn an_orphan_archive_refuses_a_plan_the_folder_no_longer_matches() {
    let fx = fixture().await;
    let folder = lost_project(&fx, "drifting");
    fx.svc.rescan_orphans().await;
    let id = format!("project_folder:{}", folder.display());
    let confirmed = fx.svc.orphan_plan(&id).await.unwrap();
    std::fs::write(folder.join("core/late.txt"), "new work").unwrap();
    assert!(matches!(
        fx.svc.archive_orphan(&id, &confirmed.items).await,
        Err(ServiceError::PlanChanged)
    ));
    assert!(folder.join("core/late.txt").is_file());
    assert!(folder.join("CLAUDE.md").is_file());
    assert!(fx.store.projects().await.unwrap().is_empty(), "no record");
    assert!(fx.svc.orphans().iter().any(|o| o.id == id));
}

#[tokio::test]
async fn an_orphan_folder_keeps_a_claude_md_the_daemon_did_not_write() {
    let fx = fixture().await;
    let folder = fx.projects_root.join("mine");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("CLAUDE.md"), "# my own notes\n").unwrap();
    std::fs::write(folder.join(PROMPT_FILE), "mine too\n").unwrap();
    fx.svc.rescan_orphans().await;
    let id = format!("project_folder:{}", folder.display());
    let plan = fx.svc.orphan_plan(&id).await.unwrap();
    assert_eq!(plan.total, 0, "{plan:?}");
    let r = fx.svc.archive_orphan(&id, &plan.items).await.unwrap();
    assert_eq!(entries(&folder), ["CLAUDE.md", PROMPT_FILE]);
    assert!(r.kept.unwrap().contains("it contains CLAUDE.md, PROMPT.md"));
}

#[tokio::test]
async fn archiving_an_orphan_agent_folder_removes_it() {
    let fx = fixture().await;
    let dir = fx.state_dir.join("agents").join("deadbeef");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.json"), "{}").unwrap();
    fx.svc.rescan_orphans().await;
    let id = format!("agent_folder:{}", dir.display());
    let plan = fx.svc.orphan_plan(&id).await.unwrap();
    assert_eq!(plan.items.len(), 1);
    assert!(!plan.items[0].stays);
    assert_eq!(
        (plan.items[0].path.as_str(), plan.total),
        (dir.to_str().unwrap(), 2)
    );
    let r = fx.svc.archive_orphan(&id, &plan.items).await.unwrap();
    assert_eq!((r.freed, r.kept), (2, None));
    assert!(!dir.exists());
    assert!(fx.state_dir.join("agents").is_dir());
}

#[tokio::test]
async fn a_session_orphan_is_ended_not_archived() {
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
    assert!(matches!(
        fx.svc.archive_orphan("session:fleet-stray", &[]).await,
        Err(ServiceError::SessionNotArchived)
    ));
    assert!(fx.tmux.has_session("fleet-stray").await.unwrap());
}

#[tokio::test]
async fn only_an_orphan_from_the_last_scan_is_archived() {
    let fx = fixture().await;
    let folder = lost_project(&fx, "unseen");
    // Not scanned yet: the user has not been shown it.
    let id = format!("project_folder:{}", folder.display());
    assert!(matches!(
        fx.svc.archive_orphan(&id, &[]).await,
        Err(ServiceError::NotAnOrphan)
    ));
    fx.svc.rescan_orphans().await;
    let sneaky = format!("project_folder:{}/..", folder.display());
    assert!(matches!(
        fx.svc.archive_orphan(&sneaky, &[]).await,
        Err(ServiceError::NotAnOrphan)
    ));
    assert!(folder.join("core/README").is_file());
}

#[tokio::test]
async fn archiving_an_orphan_rechecks_it_against_the_records_first() {
    let fx = fixture().await;
    let folder = lost_project(&fx, "claimed");
    fx.svc.rescan_orphans().await;
    let id = format!("project_folder:{}", folder.display());
    assert!(fx.svc.orphans().iter().any(|o| o.id == id));
    // A record names the folder after the scan: the cached row is stale.
    let mut a = agent_fixture(&fx);
    a.id = AgentId("claimer".into());
    a.workdir = s(&folder);
    a.workspaces.clear();
    fx.store.insert_agent(&a, "token").await.unwrap();

    assert!(matches!(
        fx.svc.orphan_plan(&id).await,
        Err(ServiceError::NotAnOrphan)
    ));
    assert!(matches!(
        fx.svc.archive_orphan(&id, &[]).await,
        Err(ServiceError::NotAnOrphan)
    ));
    assert!(folder.join("core/README").is_file());
    assert!(folder.join("CLAUDE.md").is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn archiving_an_orphan_spares_a_folder_a_spawn_has_taken() {
    let fx = fixture().await;
    // A stray folder is scanned, then goes; a spawn then takes its name while the cached row still names it.
    let stray = fx.projects_root.join("taken");
    std::fs::create_dir_all(&stray).unwrap();
    fx.svc.rescan_orphans().await;
    let id = format!("project_folder:{}", stray.display());
    assert!(fx.svc.orphans().iter().any(|o| o.id == id));
    std::fs::remove_dir(&stray).unwrap();

    let gate = SpawnGate::install(&fx.repo, fx._tmp.path());
    let svc = fx.svc.clone();
    let spawn = tokio::spawn(async move { svc.spawn(req_with("taken", &["core"])).await });
    assert!(
        gate.wait_reached().await,
        "the spawn never reached the gate"
    );
    assert!(stray.is_dir(), "the spawn made the folder");
    assert!(
        fx.store.projects().await.unwrap().is_empty(),
        "no record yet"
    );

    assert!(matches!(
        fx.svc.archive_orphan(&id, &[]).await,
        Err(ServiceError::NotAnOrphan)
    ));
    assert!(stray.is_dir());
    gate.open();
    let a = spawn.await.unwrap().unwrap();
    assert_eq!(PathBuf::from(&a.workdir), stray);
    assert!(PathBuf::from(&a.workspaces[0].worktree)
        .join("README")
        .is_file());
}

#[tokio::test]
async fn an_orphan_worktree_whose_folder_is_gone_is_pruned() {
    let fx = fixture().await;
    let folder = lost_project(&fx, "vanished");
    std::fs::remove_dir_all(&folder).unwrap();
    fx.svc.rescan_orphans().await;
    let wt = folder.join("core");
    let id = format!("worktree:{}", wt.display());
    let found = fx.svc.orphans();
    assert!(
        found
            .iter()
            .any(|o| o.id == id && o.kind == OrphanKind::Worktree),
        "{found:?}"
    );
    let r = archive_orphan_now(&fx, &id).await.unwrap();
    assert_eq!((r.freed, r.kept), (0, None));
    assert!(!git(&fx.repo, &["worktree", "list"]).contains("vanished"));
    assert!(git::branch_exists(&fx.repo, "fleet/vanished")
        .await
        .unwrap());
    assert!(fx.svc.orphans().is_empty(), "{:?}", fx.svc.orphans());
}

#[tokio::test]
async fn an_orphan_folder_with_the_users_files_frees_only_what_went() {
    let fx = fixture().await;
    let folder = lost_project(&fx, "mixed");
    std::fs::write(folder.join("notes.txt"), vec![b'x'; 5000]).unwrap();
    fx.svc.rescan_orphans().await;
    let id = format!("project_folder:{}", folder.display());
    let plan = fx.svc.orphan_plan(&id).await.unwrap();
    assert_eq!(plan.items.last().unwrap().bytes, 5000);
    assert!(plan.items.last().unwrap().stays);
    let r = fx.svc.archive_orphan(&id, &plan.items).await.unwrap();
    assert_eq!(r.freed, plan.total, "the kept notes are not freed");
    assert_eq!(entries(&folder), ["notes.txt"]);
    let kept = format!("kept {}: it contains notes.txt", folder.display());
    assert_eq!(r.kept.as_deref(), Some(kept.as_str()));
}

#[tokio::test]
async fn archive_keeps_a_repository_that_replaced_a_worktree() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("recloned", &["core"])).await.unwrap();
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    // The agent drops its worktree and makes a repository of its own in the same folder.
    git(&fx.repo, &["worktree", "remove", "--force", &s(&wt)]);
    make_repo(&wt);
    git(&wt, &["checkout", "-q", "-b", "only-here"]);
    std::fs::write(wt.join("work.txt"), "mine").unwrap();
    git(&wt, &["add", "work.txt"]);
    git(&wt, &["commit", "-q", "-m", "exists nowhere else"]);
    fx.svc.stop(&a.id).await.unwrap();

    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    let item = plan
        .items
        .iter()
        .find(|i| i.path == s(&wt))
        .expect("the repository is listed");
    assert!(item.stays, "{plan:?}");
    assert_eq!(item.uncommitted_files, 0);
    assert_eq!(
        plan.total,
        plan.items
            .iter()
            .filter(|i| !i.stays)
            .map(|i| i.bytes)
            .sum::<u64>()
    );

    let r = fx.svc.archive(&a.id, &plan.items).await.unwrap();

    assert!(wt.join(".git").is_dir());
    assert_eq!(
        git(&wt, &["log", "-1", "--format=%s", "only-here"]),
        "exists nowhere else"
    );
    assert!(!r.removed.contains(&s(&wt)));
    let kept = r.kept.expect("kept");
    assert!(
        kept.contains(&format!(
            "kept {}: it is a repository, not a worktree",
            wt.display()
        )),
        "{kept}"
    );
    assert_eq!(r.freed, plan.total, "the repository is not freed");
}
