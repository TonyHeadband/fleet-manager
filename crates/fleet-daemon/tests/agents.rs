mod common;

use common::*;
use fleet_api::{Actor, AgentId, BlockedReason, SpawnRequest, Status};
use fleet_api::EventKind;
use fleet_daemon::agents::{unapplied_repo_settings, ServiceError, SpawnError};
use fleet_daemon::ingest::ingest;
use serde_json::json;
use std::path::PathBuf;

fn req(prompt: &str) -> SpawnRequest {
    SpawnRequest { actor: Actor("local".into()), repo: "core".into(), prompt: prompt.into(), task_slug: None }
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
    r.repo = "nope".into();
    assert!(matches!(fx.svc.spawn(r).await, Err(SpawnError::UnknownRepo(_))));
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(fx.tmux.list_sessions("").await.unwrap().is_empty());
    assert!(!fx.state_dir.join("worktrees").exists());
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
    let wt_root = fx.state_dir.join("worktrees");
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
    let clean = fx.svc.spawn(req("clean")).await.unwrap();
    assert!(unapplied_repo_settings(&PathBuf::from(&clean.workspaces[0].worktree)).is_empty());

    std::fs::create_dir_all(fx.repo.join(".claude")).unwrap();
    std::fs::write(fx.repo.join(".claude/settings.json"), "{}").unwrap();
    std::fs::write(fx.repo.join(".mcp.json"), "{}").unwrap();
    git(&fx.repo, &["add", "."]);
    git(&fx.repo, &["commit", "-q", "-m", "claude config"]);
    let with = fx.svc.spawn(req("with settings")).await.unwrap();
    assert_eq!(
        unapplied_repo_settings(&PathBuf::from(&with.workspaces[0].worktree)),
        [".claude/settings.json", ".mcp.json"]
    );
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
    assert_eq!(workdir, fx.state_dir.join("worktrees").join("fix-the-tests"));
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
async fn reconcile_after_restart_keeps_workspaces() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("x")).await.unwrap();
    let second = fleet_daemon::agents::AgentService::new(fx.svc.config().clone(), fx.store.clone(), fx.tmux.clone(), fx.svc.options().clone());
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
async fn kill_removes_session_worktree_branch_settings_and_card() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req("clean me")).await.unwrap();
    fx.svc.kill(&a.id).await.unwrap();
    assert!(!fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
    assert!(!PathBuf::from(&a.workspaces[0].worktree).exists());
    assert!(!PathBuf::from(&a.workdir).exists(), "project directory left behind");
    assert!(git(&fx.repo, &["branch", "--list", "fleet/*"]).is_empty());
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
