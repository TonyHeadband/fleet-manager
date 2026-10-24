mod common;

use common::*;
use fleet_api::{Actor, AgentId, BlockedReason, SpawnRequest, Status};
use fleet_api::EventKind;
use fleet_daemon::agents::{unapplied_repo_settings, ServiceError, SpawnError};
use fleet_daemon::ingest::ingest;
use fleet_daemon::repos::RepoError;
use serde_json::json;
use std::path::PathBuf;

fn req(prompt: &str) -> SpawnRequest {
    req_with(prompt, &["core"])
}

fn req_with(prompt: &str, repos: &[&str]) -> SpawnRequest {
    SpawnRequest { actor: Actor("local".into()), repos: repos.iter().map(|r| r.to_string()).collect(), prompt: prompt.into(), task_slug: None }
}

fn hook(name: &str) -> serde_json::Value {
    json!({"hook_event_name": name})
}

async fn send_hook(fx: &Fx, id: &AgentId, name: &str) {
    let token = fx.store.agent_token(id).await.unwrap().unwrap();
    ingest(&fx.store, id, Some(&token), hook(name), fx.clock.now_for_test()).await.unwrap();
}

#[tokio::test]
async fn spawn_creates_worktree_session_and_queued_card() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("fix the tests")).await.unwrap();
    assert_eq!(a.status, Status::Queued);
    let ws = &a.workspaces;
    assert_eq!((ws.len(), ws[0].repo.as_str(), ws[0].branch.as_str()), (1, "core", "fleet/fix-the-tests"));
    assert!(PathBuf::from(&ws[0].worktree).is_dir());
    assert!(fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
    assert_eq!(fx.store.snapshot().await.unwrap().agents.len(), 1);
    assert!(fx.state_dir.join("agents").join(&a.id.0).join("settings.json").is_file());
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    assert!(pane_contains(&fx, &a.id, &format!("token={token}")).await, "FLEET_TOKEN not in the agent's environment");
    assert_eq!(token.len(), 64);
}

#[tokio::test]
async fn spawned_agent_carries_the_configured_owner_and_no_blocked_reason() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    assert_eq!((a.owner.as_str(), a.blocked_reason), ("antonio", None));
}

#[tokio::test]
async fn claude_is_started_with_the_generated_settings_file() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    let settings = fx.state_dir.join("agents").join(&a.id.0).join("settings.json");
    assert!(pane_contains(&fx, &a.id, "arg=--settings").await);
    assert!(pane_contains(&fx, &a.id, &format!("arg={}", settings.display())).await);
}

#[tokio::test]
async fn spawn_unknown_project_leaves_nothing_behind() {
    let fx = fixture().await;
    let mut r = req("x");
    r.repos = vec!["nope".into()];
    assert!(matches!(fx.svc.spawn(r).await, Err(SpawnError::Repo(_))));
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(fx.tmux.list_sessions("").await.unwrap().is_empty());
    assert!(walk_is_empty(&fx.projects_root));
}

#[tokio::test]
async fn empty_and_oversized_prompts_are_rejected() {
    let fx = fixture().await;
    assert!(matches!(fx.svc.spawn(req("   ")).await, Err(SpawnError::EmptyPrompt)));
    assert!(matches!(fx.svc.spawn(req(&"x".repeat(20_001))).await, Err(SpawnError::PromptTooLong)));
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn spawn_failure_after_the_worktree_cleans_everything_up() {
    // A tmux socket name that is far too long makes `tmux new-session` fail after the worktree exists.
    let fx = fixture_with(fake_claude(), Some("x".repeat(300))).await;
    assert!(matches!(fx.svc.spawn(req("fix it")).await, Err(SpawnError::Tmux(_))));
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(git(&fx.repo, &["branch", "--list", "fleet/*"]).is_empty(), "branch left behind");
    assert!(!git(&fx.repo, &["worktree", "list"]).contains("fix-it"), "worktree left behind");
    let wt_root = fx.projects_root.clone();
    assert!(!wt_root.exists() || walk_is_empty(&wt_root), "project dir or worktree left behind");
    assert!(!wt_root.join("fix-it").exists(), "project dir left behind");
    let agents = fx.state_dir.join("agents");
    assert!(!agents.exists() || std::fs::read_dir(agents).unwrap().next().is_none(), "settings left behind");
}

fn walk_is_empty(p: &std::path::Path) -> bool {
    std::fs::read_dir(p).unwrap().flatten().all(|e| e.path().is_dir() && walk_is_empty(&e.path()))
}

#[tokio::test]
async fn prompt_reaches_the_agent_as_one_literal_argument_after_double_dash() {
    let fx = fixture().await;
    let marker = fx.state_dir.join("pwned");
    let prompt = format!("--help $(touch {}); 'a\" `id`", marker.display());
    let a = fx.svc.spawn(req(&prompt)).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "arg=--\n").await || pane_contains(&fx, &a.id, "arg=--").await);
    assert!(pane_contains(&fx, &a.id, &format!("arg={prompt}")).await);
    assert!(!marker.exists(), "prompt was run by a shell");
}

#[tokio::test]
async fn two_spawns_with_the_same_prompt_get_distinct_branches_and_sessions() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("same")).await.unwrap();
    let b = fx.svc.spawn(req("same")).await.unwrap();
    assert_ne!((&a.workdir, &a.workspaces[0].branch, &a.tmux_session), (&b.workdir, &b.workspaces[0].branch, &b.tmux_session));
    assert_eq!(a.tmux_session, session_name(&a.id));
}

#[tokio::test]
async fn repo_settings_that_claude_will_not_load_are_reported() {
    // Claude starts one level above the checkout, so a repo's own settings never apply (measured).
    let fx = fixture().await;
    let clean = fx.svc.spawn_reporting(req("clean")).await.unwrap();
    assert!(unapplied_repo_settings(&PathBuf::from(&clean.agent.workspaces[0].worktree)).is_empty());
    assert!(clean.warnings.is_empty());

    std::fs::create_dir_all(fx.repo.join(".claude")).unwrap();
    std::fs::write(fx.repo.join(".claude/settings.json"), "{}").unwrap();
    std::fs::write(fx.repo.join(".mcp.json"), "{}").unwrap();
    git(&fx.repo, &["add", "."]);
    git(&fx.repo, &["commit", "-q", "-m", "claude config"]);
    let with = fx.svc.spawn_reporting(req("with settings")).await.unwrap();
    assert_eq!(
        unapplied_repo_settings(&PathBuf::from(&with.agent.workspaces[0].worktree)),
        [".claude/settings.json", ".mcp.json"]
    );
    // The user hears about it too, not only daemon.log.
    assert_eq!(with.warnings.len(), 2, "{:?}", with.warnings);
    assert!(with.warnings[0].starts_with("core: .claude/settings.json is not loaded"), "{:?}", with.warnings);
}

#[tokio::test]
async fn a_failed_fetch_at_spawn_comes_back_as_a_warning() {
    let fx = fixture().await;
    let seed = make_repo(&fx._tmp.path().join("up-seed"));
    let bare = fx._tmp.path().join("up.git");
    git(fx._tmp.path(), &["clone", "-q", "--bare", seed.to_str().unwrap(), bare.to_str().unwrap()]);
    let url = format!("file://{}", bare.display());
    let first = fx.svc.spawn_reporting(req_with("first", &[&url])).await.unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    // The remote goes away; the base clone is still remembered, so the next spawn starts from it after warning.
    std::fs::remove_dir_all(&bare).unwrap();
    let second = fx.svc.spawn_reporting(req_with("second", &["up"])).await.unwrap();
    assert_eq!(second.agent.workspaces.len(), 1);
    assert_eq!(second.warnings.len(), 1, "{:?}", second.warnings);
    assert!(second.warnings[0].starts_with("could not fetch up:"), "{:?}", second.warnings);
}

#[tokio::test]
async fn same_slug_twice_gets_distinct_project_dirs_and_branches() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("same thing")).await.unwrap();
    let b = fx.svc.spawn(req("same thing")).await.unwrap();
    let name = |w: &str| PathBuf::from(w).file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!((name(&a.workdir).as_str(), name(&b.workdir).as_str()), ("same-thing", "same-thing-2"));
    assert_eq!((a.workspaces[0].branch.as_str(), b.workspaces[0].branch.as_str()), ("fleet/same-thing", "fleet/same-thing-2"));
}

#[tokio::test]
async fn spawn_creates_project_dir_with_one_workspace() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("fix the tests")).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    assert_eq!(workdir, fx.projects_root.join("fix-the-tests"));
    assert!(!workdir.join(".git").exists(), "the project directory is a plain directory, not a repo");
    assert_eq!(PathBuf::from(&a.workspaces[0].worktree), workdir.join("core"));
    assert!(workdir.join("core").join("README").is_file());
    assert_eq!(a.workspaces[0].base, git(&fx.repo, &["rev-parse", "main"]));
    // claude runs in the project directory, one level above the checkout.
    let cwd = std::fs::canonicalize(&workdir).unwrap();
    assert!(pane_contains(&fx, &a.id, &format!("cwd={}", cwd.display())).await);
}

#[tokio::test]
async fn project_claude_md_tells_the_agent_where_the_repos_are() {
    // The agent starts one level above the checkout; without this it writes files next to the repo, not in it.
    let fx = fixture().await;
    let a = fx.svc.spawn(req("without")).await.unwrap();
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(text.contains("- `core/` (branch `fleet/without`)"), "{text}");
    assert!(text.contains("Do not create files directly in this folder"), "{text}");
    assert!(!text.contains('@'), "no import when the repo has no CLAUDE.md: {text}");
}

#[tokio::test]
async fn project_claude_md_imports_the_repos_own_claude_md_when_it_has_one() {
    let fx = fixture().await;
    std::fs::write(fx.repo.join("CLAUDE.md"), "be nice").unwrap();
    git(&fx.repo, &["add", "."]);
    git(&fx.repo, &["commit", "-q", "-m", "docs"]);
    let a = fx.svc.spawn(req("with it")).await.unwrap();
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(text.lines().any(|l| l == "@core/CLAUDE.md"), "{text}");
    assert!(text.contains("- `core/`"), "{text}");
}

#[tokio::test]
async fn has_changes_is_any_workspace_and_emits_workspaces_changed() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    send_hook(&fx, &a.id, "Stop").await;
    let mut rx = fx.store.subscribe();
    std::fs::write(PathBuf::from(&a.workspaces[0].worktree).join("new.txt"), "x").unwrap();
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert!(after.workspaces[0].has_changes && after.has_changes);
    let mut kinds = vec![];
    while let Ok(e) = rx.try_recv() {
        kinds.push(match e.kind {
            EventKind::WorkspacesChanged { .. } => "workspaces",
            EventKind::StatusChanged { .. } => "status",
            _ => "other",
        });
    }
    assert_eq!(kinds, ["status", "workspaces"]);
}

#[tokio::test]
async fn committed_work_in_an_owned_repo_counts_as_changes() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("new tool", &[])).await.unwrap();
    let dir = PathBuf::from(&a.workdir).join("fresh");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fx.svc.reconcile().await.unwrap();
    send_hook(&fx, &a.id, "Stop").await;
    fx.svc.reconcile().await.unwrap();
    assert!(!agent(&fx, &a.id).await.has_changes, "an empty, unborn repo is no work yet");
    std::fs::write(dir.join("README"), "hi").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert!(after.workspaces[0].owned && after.workspaces[0].has_changes && after.has_changes, "{:?}", after.workspaces);
}

#[tokio::test]
async fn reconcile_after_restart_keeps_workspaces() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    let second = fleet_daemon::agents::AgentService::new(fx.svc.config().clone(), fx.store.clone(), fx.repos.clone(), fx.tmux.clone(), fx.svc.options().clone());
    second.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.workspaces, a.workspaces);
}

#[tokio::test]
async fn pause_sends_escape_and_marks_paused_then_resume_sends_continue() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    fx.svc.pause(&a.id).await.unwrap();
    assert!(agent(&fx, &a.id).await.paused);
    assert!(pane_contains(&fx, &a.id, "^[").await, "no Escape reached the pane");
    fx.svc.resume(&a.id).await.unwrap();
    assert!(!agent(&fx, &a.id).await.paused);
    assert!(pane_contains(&fx, &a.id, "continue").await);
}

#[tokio::test]
async fn answer_types_the_text_literally() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    fx.svc.answer(&a.id, "yes; $(rm -rf x) -n").await.unwrap();
    assert!(pane_contains(&fx, &a.id, "yes; $(rm -rf x) -n").await);
}

#[tokio::test]
async fn controls_on_unknown_or_dead_agents_are_errors() {
    let fx = fixture().await;
    let ghost = AgentId("0123456789abcdef".into());
    assert!(matches!(fx.svc.answer(&ghost, "x").await, Err(ServiceError::NotFound)));
    assert!(matches!(fx.svc.pause(&ghost).await, Err(ServiceError::NotFound)));
    assert!(matches!(fx.svc.kill(&ghost).await, Err(ServiceError::NotFound)));
    let a = fx.svc.spawn(req("x")).await.unwrap();
    send_hook(&fx, &a.id, "SessionEnd").await;
    assert!(matches!(fx.svc.answer(&a.id, "x").await, Err(ServiceError::NotRunning)));
}

#[tokio::test]
async fn kill_removes_session_worktree_settings_and_card_but_keeps_the_branch() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("clean me")).await.unwrap();
    assert_eq!(fx.svc.kill(&a.id).await.unwrap().kept, None);
    assert!(!fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
    assert!(!PathBuf::from(&a.workspaces[0].worktree).exists());
    assert!(!PathBuf::from(&a.workdir).exists(), "project directory left behind");
    assert_eq!(git(&fx.repo, &["branch", "--list", "fleet/*"]), "fleet/clean-me", "the branch holds the agent's commits");
    assert!(!git(&fx.repo, &["worktree", "list"]).contains("clean-me"), "worktree left registered");
    assert!(!fx.state_dir.join("agents").join(&a.id.0).exists());
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn reconcile_marks_dead_when_the_session_vanished() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    send_hook(&fx, &a.id, "UserPromptSubmit").await;
    fx.tmux.kill(&session_name(&a.id)).await.unwrap();
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert_eq!((after.status, after.activity.as_str()), (Status::Dead, "session lost"));
}

#[tokio::test]
async fn session_that_exits_immediately_is_dead_after_reconcile() {
    let fx = fixture_with(vec!["/nonexistent/claude".into()], None).await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    assert!(wait_until(|| async { !fx.tmux.has_session(&session_name(&a.id)).await.unwrap() }).await);
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Dead);
}

#[tokio::test]
async fn a_restarted_service_adopts_live_sessions_and_kills_none() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    send_hook(&fx, &a.id, "UserPromptSubmit").await;
    let second = fleet_daemon::agents::AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        fx.svc.options().clone(),
    );
    second.reconcile().await.unwrap();
    assert!(fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);
}

#[tokio::test]
async fn silent_start_becomes_needs_you_after_the_grace_period() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    fx.clock.advance(19);
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Queued);
    fx.clock.advance(2);
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert_eq!((after.status, after.activity.as_str()), (Status::NeedsYou, "waiting at startup prompt"));
    assert_eq!(after.blocked_reason, Some(BlockedReason::Startup));
}

#[tokio::test]
async fn session_start_cancels_the_startup_rule() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    send_hook(&fx, &a.id, "SessionStart").await;
    fx.clock.advance(60);
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);
}

#[tokio::test]
async fn startup_prompt_then_session_start_returns_to_working() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    fx.clock.advance(30);
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::NeedsYou);
    send_hook(&fx, &a.id, "SessionStart").await;
    let after = agent(&fx, &a.id).await;
    assert_eq!((after.status, after.blocked_reason), (Status::Working, None));
}

#[tokio::test]
async fn mark_unverified_flags_agents_whose_events_may_have_been_missed() {
    let fx = fixture().await;
    let working = fx.svc.spawn(req("one")).await.unwrap();
    send_hook(&fx, &working.id, "UserPromptSubmit").await;
    let done = fx.svc.spawn(req("two")).await.unwrap();
    send_hook(&fx, &done.id, "Stop").await;
    fx.svc.mark_unverified().await.unwrap();
    let w = agent(&fx, &working.id).await;
    assert_eq!((w.status, w.activity.as_str()), (Status::Working, "status unverified since daemon restart"));
    let d = agent(&fx, &done.id).await;
    assert_eq!((d.status, d.activity.as_str()), (Status::Review, "finished"), "a finished agent is not in doubt");
    // The next real event replaces the doubt with fact.
    send_hook(&fx, &working.id, "PreToolUse").await;
    assert_eq!(agent(&fx, &working.id).await.activity, "running tool");
}

#[tokio::test]
async fn reconcile_does_not_overwrite_a_status_set_after_its_snapshot() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    let stale = vec![agent(&fx, &a.id).await]; // read while still Queued
    fx.clock.advance(30);
    send_hook(&fx, &a.id, "SessionStart").await; // lands after the snapshot
    fx.svc.reconcile_with(stale).await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);
}

#[tokio::test]
async fn one_vanished_agent_does_not_stop_reconcile_for_the_others() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    fx.tmux.kill(&session_name(&a.id)).await.unwrap();
    let mut ghost = agent(&fx, &a.id).await;
    ghost.id = AgentId("ghost-not-in-the-store".into()); // killed between snapshot and write
    ghost.status = Status::Working;
    let real = agent(&fx, &a.id).await;
    fx.svc.reconcile_with(vec![ghost, real]).await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Dead);
}

#[tokio::test]
async fn concurrent_spawns_of_the_same_prompt_get_distinct_project_dirs() {
    let fx = fixture().await;
    let (a, b) = tokio::join!(fx.svc.spawn(req("same")), fx.svc.spawn(req("same")));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.workdir, b.workdir);
    assert_ne!(a.workspaces[0].branch, b.workspaces[0].branch);
}

#[tokio::test]
async fn diff_covers_every_workspace_and_unknown_agents_are_not_found() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    std::fs::write(PathBuf::from(&a.workspaces[0].worktree).join("new.txt"), "hello\n").unwrap();
    let report = fx.svc.diff(&a.id).await.unwrap();
    assert_eq!(report.workspaces.len(), 1);
    assert_eq!(report.workspaces[0].repo, "core");
    assert!(report.workspaces[0].diff.contains("+hello"), "{}", report.workspaces[0].diff);
    assert!(matches!(fx.svc.diff(&AgentId("zz".into())).await, Err(ServiceError::NotFound)));
}

#[tokio::test]
async fn reconcile_refreshes_has_changes_for_agents_in_review() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    send_hook(&fx, &a.id, "Stop").await;
    fx.svc.reconcile().await.unwrap();
    assert!(!agent(&fx, &a.id).await.has_changes);
    std::fs::write(PathBuf::from(&a.workspaces[0].worktree).join("new.txt"), "x").unwrap();
    fx.svc.reconcile().await.unwrap();
    assert!(agent(&fx, &a.id).await.has_changes);
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[tokio::test]
async fn spawn_with_no_repos_makes_an_empty_project_folder() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("just think", &[])).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    assert_eq!(workdir, fx.projects_root.join("just-think"));
    assert!(a.workspaces.is_empty());
    assert_eq!(entries(&workdir), ["CLAUDE.md"]);
    let text = std::fs::read_to_string(workdir.join("CLAUDE.md")).unwrap();
    assert!(text.contains("This project has no repos yet."), "{text}");
    assert!(text.contains("To start a new repo, create a subfolder and run `git init` in it."), "{text}");
    assert!(
        text.contains("To work in an existing repo, call the `request_repo` tool (fleet MCP server); never clone or copy repos yourself."),
        "{text}"
    );
    let cwd = std::fs::canonicalize(&workdir).unwrap();
    assert!(pane_contains(&fx, &a.id, &format!("cwd={}", cwd.display())).await);
}

#[tokio::test]
async fn spawn_with_two_repos_makes_two_worktrees_on_one_branch() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let a = fx.svc.spawn(req_with("two of them", &["core", tools.to_str().unwrap()])).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    let names: Vec<_> = a.workspaces.iter().map(|w| (w.repo.as_str(), w.branch.as_str(), w.owned)).collect();
    assert_eq!(names, [("core", "fleet/two-of-them", false), ("tools", "fleet/two-of-them", false)]);
    for w in &a.workspaces {
        assert_eq!(PathBuf::from(&w.worktree), workdir.join(&w.repo));
        assert!(workdir.join(&w.repo).join("README").is_file());
    }
    assert!(!git(&tools, &["rev-parse", "--verify", "-q", "refs/heads/fleet/two-of-them"]).is_empty());
    let text = std::fs::read_to_string(workdir.join("CLAUDE.md")).unwrap();
    assert!(text.contains("- `core/` (branch `fleet/two-of-them`)") && text.contains("- `tools/` (branch `fleet/two-of-them`)"), "{text}");
    assert!(text.contains("request_repo") && text.contains("git init"), "{text}");
}

#[tokio::test]
async fn spawn_with_two_checkouts_of_one_repo_keeps_the_first() {
    let fx = fixture().await;
    let base = make_repo(&fx._tmp.path().join("dev").join("foo"));
    let feat = fx._tmp.path().join("dev").join("foo-feature");
    git(&base, &["worktree", "add", "-q", "-b", "feat", feat.to_str().unwrap()]);
    std::fs::write(feat.join("extra"), "x").unwrap();
    git(&feat, &["add", "."]);
    git(&feat, &["commit", "-q", "-m", "extra"]);

    let a = fx.svc.spawn(req_with("both", &[base.to_str().unwrap(), feat.to_str().unwrap()])).await.unwrap();
    assert_eq!(a.workspaces.len(), 1);
    assert_eq!(a.workspaces[0].base, git(&base, &["rev-parse", "HEAD"]));
    assert!(!PathBuf::from(&a.workspaces[0].worktree).join("extra").exists());
}

#[tokio::test]
async fn spawn_records_from_branch_on_the_workspace() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("record it", &["core"])).await.unwrap();
    let branch = git(&fx.repo, &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(a.workspaces[0].from_branch.as_deref(), Some(branch.as_str()));
}

#[tokio::test]
async fn spawn_with_a_bad_repo_creates_nothing() {
    let fx = fixture().await;
    let err = fx.svc.spawn(req_with("bad one", &["core", "not a repo at all"])).await.unwrap_err();
    assert!(matches!(err, SpawnError::Repo(_)), "{err:?}");
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(entries(&fx.projects_root).is_empty(), "project folder left behind");
    assert!(git(&fx.repo, &["branch", "--list", "fleet/*"]).is_empty(), "branch left behind");
    assert!(fx.tmux.list_sessions("").await.unwrap().is_empty());
}

#[tokio::test]
async fn second_spawn_with_the_same_goal_gets_slug_2() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("same goal", &[])).await.unwrap();
    let b = fx.svc.spawn(req_with("same goal", &["core"])).await.unwrap();
    assert_eq!(PathBuf::from(&a.workdir), fx.projects_root.join("same-goal"));
    assert_eq!(PathBuf::from(&b.workdir), fx.projects_root.join("same-goal-2"));
    assert_eq!(b.workspaces[0].branch, "fleet/same-goal-2");
}

#[tokio::test]
async fn a_left_over_branch_also_moves_the_slug_on() {
    let fx = fixture().await;
    git(&fx.repo, &["branch", "fleet/taken"]);
    let a = fx.svc.spawn(req_with("taken", &["core"])).await.unwrap();
    assert_eq!(PathBuf::from(&a.workdir), fx.projects_root.join("taken-2"));
}

#[tokio::test]
async fn attach_repo_adds_a_workspace_and_tells_the_agent() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let a = fx.svc.spawn(req_with("grow", &["core"])).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    // A working agent reads the line as text; see the dialog tests below for when nothing is typed.
    send_hook(&fx, &a.id, "UserPromptSubmit").await;
    let mut rx = fx.store.subscribe();
    let w = fx.svc.attach_repo(&a.id, tools.to_str().unwrap()).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    assert_eq!((w.repo.as_str(), w.branch.as_str(), w.owned), ("tools", "fleet/grow", false));
    assert_eq!(PathBuf::from(&w.worktree), workdir.join("tools"));
    assert!(workdir.join("tools/README").is_file());
    assert_eq!(agent(&fx, &a.id).await.workspaces.len(), 2);
    let mut changed = false;
    while let Ok(e) = rx.try_recv() {
        changed |= matches!(e.kind, EventKind::WorkspacesChanged { ref workspaces, .. } if workspaces.len() == 2);
    }
    assert!(changed, "no WorkspacesChanged");
    let text = std::fs::read_to_string(workdir.join("CLAUDE.md")).unwrap();
    assert!(text.contains("- `tools/` (branch `fleet/grow`)") && text.contains("- `core/`"), "{text}");
    assert!(pane_contains(&fx, &a.id, "repo tools is now available").await);
    assert!(fx.store.repo("tools").await.unwrap().is_some(), "an attached repo is remembered");
}

#[tokio::test]
async fn attach_to_a_project_with_no_repos_uses_the_folder_branch() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("empty start", &[])).await.unwrap();
    let w = fx.svc.attach_repo(&a.id, "core").await.unwrap();
    assert_eq!(w.branch, "fleet/empty-start");
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(!text.contains("no repos yet") && text.contains("- `core/`"), "{text}");
}

#[tokio::test]
async fn attaching_a_repo_twice_is_a_no_op() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("twice", &["core"])).await.unwrap();
    let w = fx.svc.attach_repo(&a.id, "core").await.unwrap();
    assert_eq!(w, a.workspaces[0]);
    let w2 = fx.svc.attach_repo(&a.id, fx.repo.to_str().unwrap()).await.unwrap();
    assert_eq!(w2, a.workspaces[0]);
    assert_eq!(agent(&fx, &a.id).await.workspaces, a.workspaces);
}

#[tokio::test]
async fn attaching_a_bad_repo_is_a_repo_error() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("bad attach", &[])).await.unwrap();
    assert!(matches!(fx.svc.attach_repo(&a.id, "no such thing").await, Err(ServiceError::Repo(_))));
    assert!(matches!(fx.svc.attach_repo(&AgentId("zz".into()), "core").await, Err(ServiceError::NotFound)));
}

#[tokio::test]
async fn an_agent_initialised_repo_is_adopted_as_owned() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("new tool", &["core"])).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    make_repo(&workdir.join("newtool"));
    std::fs::create_dir_all(workdir.join("notes")).unwrap(); // a plain folder is not a repo
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert_eq!(after.workspaces.len(), 2, "{:?}", after.workspaces);
    let w = &after.workspaces[1];
    assert_eq!((w.repo.as_str(), w.branch.as_str(), w.owned), ("newtool", "main", true));
    assert_eq!(w.base, "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
    assert_eq!(PathBuf::from(&w.worktree), workdir.join("newtool"));
    let row = fx.store.repo("newtool").await.unwrap().expect("owned repo remembered");
    assert_eq!(row.base, std::fs::canonicalize(workdir.join("newtool")).unwrap());
    let text = std::fs::read_to_string(workdir.join("CLAUDE.md")).unwrap();
    assert!(text.contains("- `newtool/` (branch `main`)"), "{text}");
    // Its diff shows everything, since its base is the empty tree.
    let d = fx.svc.diff(&a.id).await.unwrap();
    assert!(d.workspaces[1].diff.contains("+hi"), "{}", d.workspaces[1].diff);
    // Idempotent.
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.workspaces.len(), 2);
}

#[tokio::test]
async fn an_owned_repo_is_never_offered_to_another_project() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("new tool", &["core"])).await.unwrap();
    make_repo(&PathBuf::from(&a.workdir).join("newtool"));
    fx.svc.reconcile().await.unwrap();
    let names: Vec<_> = fx.svc.remembered_repos().await.unwrap().into_iter().map(|r| r.name).collect();
    assert_eq!(names, ["core"]);
    let b = fx.svc.spawn(req_with("other", &[])).await.unwrap();
    assert!(matches!(fx.svc.attach_repo(&b.id, "newtool").await, Err(ServiceError::Repo(RepoError::InsideProjects(_)))));
    // Its own project still answers from its workspaces.
    assert!(fx.svc.attach_repo(&a.id, "newtool").await.unwrap().owned);
}

#[tokio::test]
async fn an_unborn_agent_repo_is_adopted_on_its_branch() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("unborn", &[])).await.unwrap();
    let dir = PathBuf::from(&a.workdir).join("fresh");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "trunk"]);
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert_eq!(after.workspaces.iter().map(|w| (w.repo.as_str(), w.branch.as_str(), w.owned)).collect::<Vec<_>>(), [("fresh", "trunk", true)]);
}

#[tokio::test]
async fn kill_keeps_an_owned_repo_and_reports_it() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("keep it", &["core"])).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    make_repo(&workdir.join("newtool"));
    fx.svc.reconcile().await.unwrap();
    assert!(agent(&fx, &a.id).await.workspaces.iter().any(|w| w.owned));
    let report = fx.svc.kill(&a.id).await.unwrap();
    assert_eq!(report.kept, Some(format!("kept {}: it contains newtool", workdir.display())));
    assert!(workdir.join("newtool/README").is_file(), "owned repo destroyed");
    assert_eq!(git(&workdir.join("newtool"), &["log", "--oneline"]).lines().count(), 1);
    assert_eq!(entries(&workdir), ["newtool"], "the daemon's own files and the worktree must go");
    assert_eq!(git(&fx.repo, &["branch", "--list", "fleet/*"]), "fleet/keep-it");
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn kill_keeps_files_it_did_not_create_even_before_reconcile() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("stray", &[])).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    make_repo(&workdir.join("young"));
    std::fs::write(workdir.join("notes.txt"), "mine").unwrap();
    let report = fx.svc.kill(&a.id).await.unwrap();
    assert_eq!(report.kept, Some(format!("kept {}: it contains notes.txt, young", workdir.display())));
    assert_eq!(entries(&workdir), ["notes.txt", "young"]);
}

#[tokio::test]
async fn kill_of_a_plain_project_removes_the_folder() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let a = fx.svc.spawn(req_with("plain", &["core", tools.to_str().unwrap()])).await.unwrap();
    let report = fx.svc.kill(&a.id).await.unwrap();
    assert_eq!(report.kept, None);
    assert!(!PathBuf::from(&a.workdir).exists());
    assert!(entries(&fx.projects_root).is_empty());
    assert_eq!(git(&tools, &["branch", "--list", "fleet/*"]), "fleet/plain");
    assert_eq!(git(&fx.repo, &["branch", "--list", "fleet/*"]), "fleet/plain");
    assert!(!git(&tools, &["worktree", "list"]).contains("plain"), "worktree left registered");
    assert!(!fx.state_dir.join("agents").join(&a.id.0).exists());
}

#[tokio::test]
async fn kill_skips_a_workspace_whose_repo_is_no_longer_remembered() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("forgotten", &["core"])).await.unwrap();
    fx.store.update_agent(&a.id, |x| x.workspaces[0].repo = "gone".into()).await.unwrap();
    let report = fx.svc.kill(&a.id).await.unwrap();
    // The worktree stays (its base is unknown), so the folder is kept and reported, never deleted.
    assert!(report.kept.unwrap().contains("core"));
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn concurrent_attaches_to_one_project_both_land() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let a = fx.svc.spawn(req_with("both", &[])).await.unwrap();
    let (x, y) = tokio::join!(fx.svc.attach_repo(&a.id, "core"), fx.svc.attach_repo(&a.id, tools.to_str().unwrap()));
    x.unwrap();
    y.unwrap();
    let mut names: Vec<_> = agent(&fx, &a.id).await.workspaces.into_iter().map(|w| w.repo).collect();
    names.sort();
    assert_eq!(names, ["core", "tools"]);
}

#[tokio::test]
async fn a_new_project_avoids_a_name_whose_branch_survived_a_kill() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let old = fx.svc.spawn(req_with("again", &[tools.to_str().unwrap()])).await.unwrap();
    fx.svc.kill(&old.id).await.unwrap();
    // `tools` is remembered but not requested: its surviving `fleet/again` still blocks the name.
    let a = fx.svc.spawn(req_with("again", &[])).await.unwrap();
    assert_eq!(PathBuf::from(&a.workdir), fx.projects_root.join("again-2"));
    let w = fx.svc.attach_repo(&a.id, "tools").await.unwrap();
    assert_eq!(w.branch, "fleet/again-2");
}

#[tokio::test]
async fn attach_with_an_existing_branch_is_a_clear_conflict() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with("clash", &[])).await.unwrap();
    // Remembered only after the name was picked, so the picker could not see its branch.
    let tools = make_repo(&fx._tmp.path().join("tools"));
    git(&tools, &["branch", "fleet/clash"]);
    let before = git(&tools, &["rev-parse", "fleet/clash"]);
    let err = fx.svc.attach_repo(&a.id, tools.to_str().unwrap()).await.unwrap_err();
    assert_eq!(err.to_string(), "branch fleet/clash already exists in tools; delete or rename it, then try again");
    assert!(matches!(err, ServiceError::Repo(fleet_daemon::repos::RepoError::BranchExists { .. })), "{err:?}");
    assert_eq!(git(&tools, &["rev-parse", "fleet/clash"]), before, "the old branch must never be reused or moved");
    assert!(!PathBuf::from(&a.workdir).join("tools").exists());
    assert!(agent(&fx, &a.id).await.workspaces.is_empty());
}

/// On a permission prompt Enter picks the highlighted "Yes", and at the startup trust dialog it picks "No, exit"; a
/// notice typed there would answer the dialog. CLAUDE.md and list_repos still tell the agent.
#[tokio::test]
async fn attach_types_nothing_while_a_dialog_may_be_up() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let other = make_repo(&fx._tmp.path().join("other"));
    let a = fx.svc.spawn(req_with("asks", &["core"])).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    // Queued: the startup trust dialog may be on screen.
    fx.svc.attach_repo(&a.id, tools.to_str().unwrap()).await.unwrap();
    send_hook(&fx, &a.id, "UserPromptSubmit").await;
    fx.store
        .update_agent(&a.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::Permission);
        })
        .await
        .unwrap();
    fx.svc.attach_repo(&a.id, other.to_str().unwrap()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let pane = fx.tmux.capture(&session_name(&a.id)).await.unwrap();
    assert!(!pane.contains("fleet-manager:"), "{pane}");
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(text.contains("- `tools/`") && text.contains("- `other/`"), "{text}");
}
