mod common;

use common::*;
use fleet_api::EventKind;
use fleet_api::{
    Actor, Agent, AgentId, ArchiveReport, BlockedReason, SpawnRequest, Status, StopCause, TaskState,
};
use fleet_daemon::agents::{
    ServiceError, SpawnError, CLAUDE_MD_HEADER, PROMPT_FILE, PROMPT_POINTER,
};
use fleet_daemon::ingest::{ingest, IngestOutcome};
use fleet_daemon::personas::PersonaError;
use fleet_daemon::repos::RepoError;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn req(fx: &Fx, prompt: &str) -> SpawnRequest {
    req_with(fx, prompt, &["core"])
}

fn req_with(fx: &Fx, prompt: &str, repos: &[&str]) -> SpawnRequest {
    SpawnRequest {
        actor: Actor("local".into()),
        product: PRODUCT.into(),
        release: None,
        repos: repos.iter().map(|r| pick(fx, r)).collect(),
        prompt: prompt.into(),
        slug: None,
        persona: PERSONA.into(),
        kind: fleet_api::AgentKind::Solo,
        role: None,
        roster: vec![],
    }
}

fn hook(name: &str) -> serde_json::Value {
    json!({"hook_event_name": name})
}

async fn send_hook(fx: &Fx, id: &AgentId, name: &str) {
    let token = fx.store.agent_token(id).await.unwrap().unwrap();
    ingest(
        &fx.store,
        id,
        Some(&token),
        hook(name),
        fx.clock.now_for_test(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn spawn_creates_worktree_session_and_queued_card() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "fix the tests")).await.unwrap();
    assert_eq!(a.status, Status::Queued);
    let ws = &a.workspaces;
    assert_eq!(
        (ws.len(), ws[0].repo.as_str(), ws[0].branch.as_str()),
        (1, "core", "fleet/fix-the-tests")
    );
    assert!(PathBuf::from(&ws[0].worktree).is_dir());
    assert!(fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
    assert_eq!(fx.store.snapshot().await.unwrap().agents.len(), 1);
    assert!(fx
        .state_dir
        .join("agents")
        .join(&a.id.0)
        .join("settings.json")
        .is_file());
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    assert!(
        pane_contains(&fx, &a.id, &format!("token={token}")).await,
        "FLEET_TOKEN not in the agent's environment"
    );
    assert_eq!(token.len(), 64);
}

#[tokio::test]
async fn spawned_agent_carries_the_configured_owner_and_no_blocked_reason() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    assert_eq!((a.owner.as_str(), a.blocked_reason), ("antonio", None));
}

#[tokio::test]
async fn claude_is_started_with_the_generated_settings_file() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    let settings = fx
        .state_dir
        .join("agents")
        .join(&a.id.0)
        .join("settings.json");
    assert!(pane_contains(&fx, &a.id, "arg=--settings").await);
    assert!(pane_contains(&fx, &a.id, &format!("arg={}", settings.display())).await);
}

#[tokio::test]
async fn spawn_unknown_task_leaves_nothing_behind() {
    let fx = fixture().await;
    let mut r = req(&fx, "x");
    r.repos = vec![pick(&fx, "nope")];
    assert!(matches!(fx.svc.spawn(r).await, Err(SpawnError::Repo(_))));
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(fx.tmux.list_sessions("").await.unwrap().is_empty());
    assert!(walk_is_empty(&fx.tasks_root));
}

#[tokio::test]
async fn empty_and_oversized_prompts_are_rejected() {
    let fx = fixture().await;
    assert!(matches!(
        fx.svc.spawn(req(&fx, "   ")).await,
        Err(SpawnError::EmptyPrompt)
    ));
    assert!(matches!(
        fx.svc.spawn(req(&fx, &"x".repeat(20_001))).await,
        Err(SpawnError::PromptTooLong)
    ));
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn spawn_failure_after_the_worktree_cleans_everything_up() {
    // A tmux socket name that is far too long makes `tmux new-session` fail after the worktree exists.
    let fx = fixture_with(fake_claude(), Some("x".repeat(300))).await;
    assert!(matches!(
        fx.svc.spawn(req(&fx, "fix it")).await,
        Err(SpawnError::Tmux(_))
    ));
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(
        git(&fx.repo, &["branch", "--list", "fleet/*"]).is_empty(),
        "branch left behind"
    );
    assert!(
        !git(&fx.repo, &["worktree", "list"]).contains("fix-it"),
        "worktree left behind"
    );
    let wt_root = fx.tasks_root.clone();
    assert!(
        !wt_root.exists() || walk_is_empty(&wt_root),
        "task dir or worktree left behind"
    );
    assert!(!wt_root.join("fix-it").exists(), "task dir left behind");
    let agents = fx.state_dir.join("agents");
    assert!(
        !agents.exists() || std::fs::read_dir(agents).unwrap().next().is_none(),
        "settings left behind"
    );
    // The record stored with the card is never deleted: it says the task ended, and why.
    let records = fx.store.tasks().await.unwrap();
    assert_eq!(records.len(), 1, "{records:?}");
    let p = &records[0];
    assert_eq!(p.state, TaskState::Archived);
    assert_eq!(p.archived_at, Some(fx.clock.now_for_test()));
    let last = &p.log.last().unwrap().text;
    assert!(last.starts_with("spawn failed: "), "{last}");
    assert_eq!(
        p.archive,
        Some(ArchiveReport {
            removed: vec![],
            freed: 0,
            kept: None
        })
    );
}

fn walk_is_empty(p: &std::path::Path) -> bool {
    std::fs::read_dir(p)
        .unwrap()
        .flatten()
        .all(|e| e.path().is_dir() && walk_is_empty(&e.path()))
}

#[tokio::test]
async fn the_prompt_is_in_a_private_file_and_never_on_the_command_line() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture().await;
    let marker = fx.state_dir.join("pwned");
    // Made at run time, so no other process on the machine (this test's own source, say) can contain it.
    let secret: String = format!(
        "{:x}{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
    .chars()
    .rev()
    .collect();
    let prompt = format!("--help $(touch {}); 'a\" `id` {secret}", marker.display());
    let a = fx.svc.spawn(req(&fx, &prompt)).await.unwrap();
    let file = PathBuf::from(&a.workdir).join(PROMPT_FILE);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("{prompt}\n")
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600,
        "readable by others"
    );
    assert!(pane_contains(&fx, &a.id, &format!("arg={PROMPT_POINTER}")).await);
    // `ps` shows the command line: the prompt must not be on it.
    let ps = std::process::Command::new("ps")
        .args(["-eo", "args"])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&ps.stdout).contains(&secret),
        "the prompt is visible in ps"
    );
    assert!(!marker.exists(), "prompt was run by a shell");
    fx.svc.stop(&a.id).await.unwrap();
    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    fx.svc.archive(&a.id, &plan.items).await.unwrap();
    assert!(
        !PathBuf::from(&a.workdir).exists(),
        "PROMPT.md kept the task folder alive"
    );
}

#[tokio::test]
async fn two_spawns_with_the_same_prompt_get_distinct_branches_and_sessions() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "same")).await.unwrap();
    let b = fx.svc.spawn(req(&fx, "same")).await.unwrap();
    assert_ne!(
        (&a.workdir, &a.workspaces[0].branch, &a.tmux_session),
        (&b.workdir, &b.workspaces[0].branch, &b.tmux_session)
    );
    assert_eq!(a.tmux_session, session_name(&a.id));
}

#[tokio::test]
async fn a_repos_deny_and_ask_rules_reach_the_agents_settings() {
    // Claude starts one level above the checkout, so it never reads a repo's own settings (measured); the daemon
    // carries the deny and ask rules over itself.
    let fx = fixture().await;
    let clean = fx.svc.spawn_reporting(req(&fx, "clean")).await.unwrap();
    assert!(clean.warnings.is_empty());

    std::fs::create_dir_all(fx.repo.join(".claude")).unwrap();
    std::fs::write(fx.repo.join(".claude/settings.json"), r#"{"permissions": {"allow": ["Bash(npm:*)"], "deny": ["Read(./.env)"], "ask": ["Bash(git push *)"]}}"#).unwrap();
    std::fs::write(fx.repo.join(".mcp.json"), "{}").unwrap();
    git(&fx.repo, &["add", "."]);
    git(&fx.repo, &["commit", "-q", "-m", "claude config"]);
    let with = fx
        .svc
        .spawn_reporting(req(&fx, "with settings"))
        .await
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            fx.state_dir
                .join("agents")
                .join(&with.agent.id.0)
                .join("settings.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(settings["permissions"]["deny"], json!(["Read(core/.env)"]));
    assert_eq!(settings["permissions"]["ask"], json!(["Bash(git push *)"]));
    assert!(
        !settings["permissions"]["allow"].to_string().contains("npm"),
        "a repo's allow rule loosened the agent"
    );
    // The user hears what was left out.
    assert_eq!(with.warnings.len(), 2, "{:?}", with.warnings);
    assert!(
        with.warnings[0].starts_with("core: .claude/settings.json allow rules do not apply"),
        "{:?}",
        with.warnings
    );
    assert!(
        with.warnings[1].starts_with("core: .mcp.json is not loaded"),
        "{:?}",
        with.warnings
    );
}

#[tokio::test]
async fn a_repo_added_later_brings_its_rules_into_the_settings() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    std::fs::create_dir_all(tools.join(".claude")).unwrap();
    std::fs::write(
        tools.join(".claude/settings.json"),
        r#"{"permissions": {"deny": ["Edit(/vendor/**)"]}}"#,
    )
    .unwrap();
    git(&tools, &["add", "."]);
    git(&tools, &["commit", "-q", "-m", "rules"]);
    let a = fx
        .svc
        .spawn(req_with(&fx, "grow", &["core"]))
        .await
        .unwrap();
    fx.svc
        .attach_repo(&a.id, tools.to_str().unwrap())
        .await
        .unwrap();
    let text = std::fs::read_to_string(
        fx.state_dir
            .join("agents")
            .join(&a.id.0)
            .join("settings.json"),
    )
    .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        settings["permissions"]["deny"],
        json!(["Edit(tools/vendor/**)"])
    );
}

#[tokio::test]
async fn a_failed_fetch_at_spawn_comes_back_as_a_warning() {
    let fx = fixture().await;
    let seed = make_repo(&fx._tmp.path().join("up-seed"));
    let bare = fx._tmp.path().join("up.git");
    git(
        fx._tmp.path(),
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let url = format!("file://{}", bare.display());
    let first = fx
        .svc
        .spawn_reporting(req_with(&fx, "first", &[&url]))
        .await
        .unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    // The remote goes away; the base clone is still remembered, so the next spawn starts from it after warning.
    std::fs::remove_dir_all(&bare).unwrap();
    let second = fx
        .svc
        .spawn_reporting(req_with(&fx, "second", &["up"]))
        .await
        .unwrap();
    assert_eq!(second.agent.workspaces.len(), 1);
    assert_eq!(second.warnings.len(), 1, "{:?}", second.warnings);
    assert!(
        second.warnings[0].starts_with("could not fetch up:"),
        "{:?}",
        second.warnings
    );
}

#[tokio::test]
async fn spawn_fails_clearly_when_claude_is_missing_and_leaves_nothing() {
    for missing in ["fleet-no-such-claude", "/nonexistent/claude"] {
        let fx = fixture_with(vec![missing.into()], None).await;
        let err = fx.svc.spawn(req(&fx, "x")).await.unwrap_err();
        assert!(
            matches!(&err, SpawnError::ClaudeNotFound(p) if p == missing),
            "{err:?}"
        );
        assert!(err.to_string().contains("claude_command"), "{err}");
        assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
        assert_eq!(
            std::fs::read_dir(&fx.tasks_root).unwrap().count(),
            0,
            "a task folder was made"
        );
        assert_eq!(git(&fx.repo, &["branch", "--list", "fleet/*"]), "");
    }
}

#[tokio::test]
async fn a_claude_found_on_path_starts() {
    // `sh` stands in for a program named without a path.
    let fx = fixture_with(vec!["sh".into(), "-c".into(), "sleep 30".into()], None).await;
    fx.svc.spawn(req(&fx, "on path")).await.unwrap();
}

#[tokio::test]
async fn same_slug_twice_gets_distinct_task_dirs_and_branches() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "same thing")).await.unwrap();
    let b = fx.svc.spawn(req(&fx, "same thing")).await.unwrap();
    let name = |w: &str| {
        PathBuf::from(w)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(
        (name(&a.workdir).as_str(), name(&b.workdir).as_str()),
        ("same-thing", "same-thing-2")
    );
    assert_eq!(
        (
            a.workspaces[0].branch.as_str(),
            b.workspaces[0].branch.as_str()
        ),
        ("fleet/same-thing", "fleet/same-thing-2")
    );
}

#[tokio::test]
async fn spawn_creates_task_dir_with_one_workspace() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "fix the tests")).await.unwrap();
    let workdir = PathBuf::from(&a.workdir);
    assert_eq!(workdir, fx.tasks_root.join("fix-the-tests"));
    assert!(
        !workdir.join(".git").exists(),
        "the task directory is a plain directory, not a repo"
    );
    assert_eq!(
        PathBuf::from(&a.workspaces[0].worktree),
        workdir.join("core")
    );
    assert!(workdir.join("core").join("README").is_file());
    assert_eq!(a.workspaces[0].base, git(&fx.repo, &["rev-parse", "main"]));
    // claude runs in the task directory, one level above the checkout.
    let cwd = std::fs::canonicalize(&workdir).unwrap();
    assert!(pane_contains(&fx, &a.id, &format!("cwd={}", cwd.display())).await);
}

#[tokio::test]
async fn task_claude_md_tells_the_agent_where_the_repos_are() {
    // The agent starts one level above the checkout; without this it writes files next to the repo, not in it.
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "without")).await.unwrap();
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(
        text.contains("- `core/` (branch `fleet/without`)"),
        "{text}"
    );
    assert!(
        text.contains("Do not create files directly in this folder"),
        "{text}"
    );
    assert!(
        !text.contains('@'),
        "no import when the repo has no CLAUDE.md: {text}"
    );
}

fn agent_dir(fx: &Fx) -> PathBuf {
    fx.state_dir.parent().unwrap().join("agent")
}

#[tokio::test]
async fn instructions_lead_claude_md() {
    let fx = fixture().await;
    std::fs::create_dir_all(agent_dir(&fx)).unwrap();
    std::fs::write(
        agent_dir(&fx).join("instructions.md"),
        "Always write tests.\n",
    )
    .unwrap();
    let a = fx.svc.spawn(req(&fx, "with")).await.unwrap();
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(
        text.starts_with("# Instructions for every agent\n\nAlways write tests.\n"),
        "{text}"
    );
    assert!(text.contains(CLAUDE_MD_HEADER), "{text}");
}

#[tokio::test]
async fn invalid_file_fails_spawn() {
    let fx = fixture().await;
    std::fs::create_dir_all(agent_dir(&fx)).unwrap();
    std::fs::write(
        agent_dir(&fx).join("settings.json"),
        r#"{"env": {"GH_HOST": "x"}}"#,
    )
    .unwrap();
    let err = fx.svc.spawn(req(&fx, "bad")).await.unwrap_err();
    assert!(matches!(err, SpawnError::AgentConfig(_)), "{err}");
    assert!(err.to_string().contains("GH_HOST"), "{err}");
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert_eq!(std::fs::read_dir(&fx.tasks_root).unwrap().count(), 0);
}

#[tokio::test]
async fn invalid_file_fails_attach_and_changes_nothing() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with(&fx, "none", &[])).await.unwrap();
    std::fs::create_dir_all(agent_dir(&fx)).unwrap();
    std::fs::write(
        agent_dir(&fx).join("settings.json"),
        r#"{"env": {"GIT_DIR": "x"}}"#,
    )
    .unwrap();
    let err = fx.svc.attach_repo(&a.id, "core").await.unwrap_err();
    assert!(matches!(err, ServiceError::AgentConfig(_)), "{err}");
    assert!(err.to_string().contains("GIT_DIR"), "{err}");
    assert!(agent(&fx, &a.id).await.workspaces.is_empty());
}

#[tokio::test]
async fn task_claude_md_imports_the_repos_own_claude_md_when_it_has_one() {
    let fx = fixture().await;
    std::fs::write(fx.repo.join("CLAUDE.md"), "be nice").unwrap();
    git(&fx.repo, &["add", "."]);
    git(&fx.repo, &["commit", "-q", "-m", "docs"]);
    let a = fx.svc.spawn(req(&fx, "with it")).await.unwrap();
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(text.lines().any(|l| l == "@core/CLAUDE.md"), "{text}");
    assert!(text.contains("- `core/`"), "{text}");
}

#[tokio::test]
async fn has_changes_is_any_workspace_and_emits_workspaces_changed() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    send_hook(&fx, &a.id, "Stop").await;
    let mut rx = fx.store.subscribe();
    std::fs::write(
        PathBuf::from(&a.workspaces[0].worktree).join("new.txt"),
        "x",
    )
    .unwrap();
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
    let a = fx.svc.spawn(req_with(&fx, "new tool", &[])).await.unwrap();
    let dir = PathBuf::from(&a.workdir).join("fresh");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    fx.svc.reconcile().await.unwrap();
    send_hook(&fx, &a.id, "Stop").await;
    fx.svc.reconcile().await.unwrap();
    assert!(
        !agent(&fx, &a.id).await.has_changes,
        "an empty, unborn repo is no work yet"
    );
    std::fs::write(dir.join("README"), "hi").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert!(
        after.workspaces[0].owned && after.workspaces[0].has_changes && after.has_changes,
        "{:?}",
        after.workspaces
    );
}

#[tokio::test]
async fn reconcile_after_restart_keeps_workspaces() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    let second = fleet_daemon::agents::AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        fx.svc.options().clone(),
    );
    second.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.workspaces, a.workspaces);
}

#[tokio::test]
async fn pause_sends_escape_and_marks_paused_then_resume_sends_continue() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    fx.svc.pause(&a.id).await.unwrap();
    assert!(agent(&fx, &a.id).await.paused);
    assert!(
        pane_contains(&fx, &a.id, "^[").await,
        "no Escape reached the pane"
    );
    fx.svc.resume(&a.id).await.unwrap();
    assert!(!agent(&fx, &a.id).await.paused);
    assert!(pane_contains(&fx, &a.id, "continue").await);
}

#[tokio::test]
async fn answer_types_the_text_literally() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    fx.svc.answer(&a.id, "yes; $(rm -rf x) -n").await.unwrap();
    assert!(pane_contains(&fx, &a.id, "yes; $(rm -rf x) -n").await);
}

#[tokio::test]
async fn controls_on_unknown_or_dead_agents_are_errors() {
    let fx = fixture().await;
    let ghost = AgentId("0123456789abcdef".into());
    assert!(matches!(
        fx.svc.answer(&ghost, "x").await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        fx.svc.pause(&ghost).await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        fx.svc.stop(&ghost).await,
        Err(ServiceError::NotFound)
    ));
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    send_hook(&fx, &a.id, "SessionEnd").await;
    assert!(matches!(
        fx.svc.answer(&a.id, "x").await,
        Err(ServiceError::NotRunning)
    ));
}

#[tokio::test]
async fn stop_ends_the_session_and_keeps_everything_on_disk() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "keep me")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    fx.svc.stop(&a.id).await.unwrap();
    assert!(!fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
    assert!(PathBuf::from(&a.workspaces[0].worktree).is_dir());
    assert!(PathBuf::from(&a.workdir).join("CLAUDE.md").is_file());
    let after = agent(&fx, &a.id).await;
    assert_eq!(
        (after.status, after.stop_cause, after.activity.as_str()),
        (Status::Stopped, Some(StopCause::Stopped), "stopped by you")
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    // "ready" is printed by the fake claude, so it is on the pane the stop saved.
    assert!(p.last_screen.unwrap().contains("ready"));
    assert_eq!(p.log.last().unwrap().text, "stopped by you");
    assert!(git(&fx.repo, &["branch", "--list", "fleet/*"]).contains("fleet/keep-me"));
}

/// A tmux that stops at `kill-session` until the test opens the gate, so the test can act between a stop's write
/// and its kill. Opened on drop, and the wrapper gives up after 20 s on its own, so a failing test hangs nothing.
struct KillGate {
    program: PathBuf,
    reached: PathBuf,
    open: PathBuf,
}

impl KillGate {
    fn install(dir: &std::path::Path) -> KillGate {
        use std::os::unix::fs::PermissionsExt;
        let gate = KillGate {
            program: dir.join("gated-tmux"),
            reached: dir.join("kill-reached"),
            open: dir.join("kill-open"),
        };
        std::fs::write(
            &gate.program,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do\n  if [ \"$a\" = kill-session ]; then\n    touch '{}'\n    i=0\n    while [ ! -e '{}' ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i+1)); done\n  fi\ndone\nexec tmux \"$@\"\n",
                gate.reached.display(),
                gate.open.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&gate.program, std::fs::Permissions::from_mode(0o755)).unwrap();
        gate
    }
}

impl Drop for KillGate {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.open, b"");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_end_racing_a_stop_keeps_the_cause_stopped() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "race")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let gate = KillGate::install(fx._tmp.path());
    let svc = Arc::new(fleet_daemon::agents::AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone().with_program(gate.program.to_string_lossy()),
        fx.svc.options().clone(),
    ));
    let id = a.id.clone();
    let stop = tokio::spawn(async move { svc.stop(&id).await });
    assert!(
        wait_until(|| async { gate.reached.exists() }).await,
        "the stop never reached the kill"
    );
    // What the dying claude sends: it must not get to name the cause.
    send_hook(&fx, &a.id, "SessionEnd").await;
    drop(gate);
    stop.await.unwrap().unwrap();

    let after = agent(&fx, &a.id).await;
    assert_eq!(
        (after.status, after.stop_cause, after.activity.as_str()),
        (Status::Stopped, Some(StopCause::Stopped), "stopped by you")
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.stop_cause),
        (TaskState::Stopped, Some(StopCause::Stopped))
    );
    let log: Vec<&str> = p.log.iter().map(|l| l.text.as_str()).collect();
    assert!(!log.contains(&"agent exited"), "{log:?}");
    assert_eq!(log.last(), Some(&"stopped by you"));
    assert!(p.last_screen.unwrap().contains("ready"));
    assert!(!fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
}

/// A tmux whose `kill-session` fails while the file `kill_fails_flag(dir)` exists (created here). With `end_first`
/// it ends the session for real before failing, as a session that exits on its own between a stop's check and its
/// kill does ("can't find session").
fn failing_kill(dir: &std::path::Path, end_first: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let program = dir.join(if end_first { "gone-tmux" } else { "stuck-tmux" });
    let flag = kill_fails_flag(dir);
    std::fs::write(&flag, b"").unwrap();
    let end = if end_first { "    tmux \"$@\"\n" } else { "" };
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do\n  if [ \"$a\" = kill-session ] && [ -e '{}' ]; then\n{end}    echo 'kill failed' >&2\n    exit 1\n  fi\ndone\nexec tmux \"$@\"\n",
            flag.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    program
}

/// While this file exists, the `failing_kill` wrapper's kills fail.
fn kill_fails_flag(dir: &std::path::Path) -> PathBuf {
    dir.join("kill-fails")
}

/// A service on `fx`'s store and tmux server whose kills fail (see `failing_kill`).
fn with_failing_kill(fx: &Fx, end_first: bool) -> Arc<fleet_daemon::agents::AgentService> {
    let program = failing_kill(fx._tmp.path(), end_first);
    Arc::new(fleet_daemon::agents::AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone().with_program(program.to_string_lossy()),
        fx.svc.options().clone(),
    ))
}

/// Starts a held `request_repo` call and waits until it is pending.
async fn held_request(
    fx: &Fx,
    svc: &Arc<fleet_daemon::agents::AgentService>,
    id: &AgentId,
) -> tokio::task::JoinHandle<Result<fleet_daemon::agents::RepoAnswer, ServiceError>> {
    let (svc2, id2) = (svc.clone(), id.clone());
    let handle = tokio::spawn(async move { svc2.request_repo(&id2, "elsewhere", "w").await });
    assert!(
        wait_until(|| async { agent(fx, id).await.repo_request.is_some() }).await,
        "the request never became pending"
    );
    handle
}

/// The bookkeeping every stop does, whether or not its kill worked.
async fn assert_stopped_by_you(fx: &Fx, id: &AgentId) {
    let after = agent(fx, id).await;
    assert_eq!(
        (after.status, after.stop_cause, after.activity.as_str()),
        (Status::Stopped, Some(StopCause::Stopped), "stopped by you")
    );
    let p = fx.store.task(id).await.unwrap().unwrap();
    assert_eq!(p.log.last().unwrap().text, "stopped by you");
    assert!(p.last_screen.unwrap().contains("ready"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_whose_kill_finds_the_session_gone_finishes() {
    let fx = fixture().await;
    let svc = with_failing_kill(&fx, true);
    let a = svc.spawn(req(&fx, "gone")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let held = held_request(&fx, &svc, &a.id).await;

    svc.stop(&a.id).await.unwrap();

    assert_stopped_by_you(&fx, &a.id).await;
    let r = tokio::time::timeout(std::time::Duration::from_secs(5), held)
        .await
        .expect("the held request outlived the stop")
        .unwrap();
    assert!(matches!(r, Err(ServiceError::RequestWithdrawn)), "{r:?}");
    assert!(!fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_whose_kill_fails_on_a_live_session_records_it_and_blocks_archive() {
    let fx = fixture().await;
    let svc = with_failing_kill(&fx, false);
    let a = svc.spawn(req(&fx, "stuck")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let held = held_request(&fx, &svc, &a.id).await;

    let r = svc.stop(&a.id).await;
    assert!(matches!(r, Err(ServiceError::Tmux(_))), "{r:?}");

    assert_stopped_by_you(&fx, &a.id).await;
    let r = tokio::time::timeout(std::time::Duration::from_secs(5), held)
        .await
        .expect("the held request outlived the stop")
        .unwrap();
    assert!(matches!(r, Err(ServiceError::RequestWithdrawn)), "{r:?}");
    assert!(fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
    // Archive would delete the worktrees under the claude still running in them.
    assert!(matches!(
        fx.svc.archive_plan(&a.id).await,
        Err(ServiceError::SessionStillLive)
    ));
    assert!(matches!(
        fx.svc.archive(&a.id, &[]).await,
        Err(ServiceError::SessionStillLive)
    ));
    assert!(PathBuf::from(&a.workspaces[0].worktree).is_dir());

    // A second Stop, once tmux can kill again, ends the session it left: no other way out exists.
    std::fs::remove_file(kill_fails_flag(fx._tmp.path())).unwrap();
    let log_len = fx.store.task(&a.id).await.unwrap().unwrap().log.len();
    svc.stop(&a.id).await.unwrap();
    assert!(!fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    let added: Vec<&str> = p.log[log_len..].iter().map(|l| l.text.as_str()).collect();
    assert_eq!(added, ["session ended on retry"]);
    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    fx.svc.archive(&a.id, &plan.items).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_clears_the_pending_repo_request() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "asks")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let held = held_request(&fx, &fx.svc, &a.id).await;

    fx.svc.stop(&a.id).await.unwrap();

    let after = agent(&fx, &a.id).await;
    assert_eq!(
        (after.status, after.repo_request, after.blocked_reason),
        (Status::Stopped, None, None)
    );
    let _ = held.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_whose_persona_is_deleted_meanwhile_fails_and_cleans_up() {
    let fx = fixture().await;
    let gate = SpawnGate::install(&fx.repo, fx._tmp.path());
    let svc = fx.svc.clone();
    let r = req(&fx, "orphaned");
    let spawn = tokio::spawn(async move { svc.spawn(r).await });
    // Held inside `git worktree add`: past the login check, before the record is stored.
    assert!(
        gate.wait_reached().await,
        "the spawn never reached the gate"
    );
    fx.svc.personas().delete(PERSONA).await.unwrap();
    gate.open();

    let r = spawn.await.unwrap();
    assert!(
        matches!(&r, Err(SpawnError::Persona(PersonaError::NotFound(p))) if p == PERSONA),
        "{r:?}"
    );
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(fx.store.tasks().await.unwrap().is_empty());
    assert!(!fx.tasks_root.join("orphaned").exists());
    assert!(git(&fx.repo, &["branch", "--list", "fleet/*"]).is_empty());
    assert!(!git(&fx.repo, &["worktree", "list"]).contains("orphaned"));
}

/// A spawn held inside `git worktree add`, with `lose` run before it may go on: the product or release it checked
/// at the start is gone by the time its record is stored.
async fn spawn_losing(
    fx: &Fx,
    r: SpawnRequest,
    lose: impl std::future::Future<Output = ()>,
) -> Result<Agent, SpawnError> {
    let gate = SpawnGate::install(&fx.repo, fx._tmp.path());
    let svc = fx.svc.clone();
    let spawn = tokio::spawn(async move { svc.spawn(r).await });
    assert!(
        gate.wait_reached().await,
        "the spawn never reached the gate"
    );
    lose.await;
    gate.open();
    spawn.await.unwrap()
}

async fn assert_nothing_left(fx: &Fx, name: &str) {
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(fx.store.tasks().await.unwrap().is_empty());
    assert!(!fx.tasks_root.join(name).exists());
    assert!(git(&fx.repo, &["branch", "--list", "fleet/*"]).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_whose_product_is_deleted_meanwhile_fails_and_cleans_up() {
    let fx = fixture().await;
    let store = fx.store.clone();
    let r = spawn_losing(&fx, req(&fx, "gone"), async move {
        assert_eq!(store.delete_product_if_unused(PRODUCT).await.unwrap(), 0);
    })
    .await;
    assert!(
        matches!(&r, Err(SpawnError::Repo(RepoError::UnknownProduct(p))) if p == PRODUCT),
        "{r:?}"
    );
    assert_nothing_left(&fx, "gone").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_whose_release_is_deleted_meanwhile_fails_and_cleans_up() {
    let fx = fixture().await;
    let release = fx
        .store
        .insert_release(&fleet_daemon::store::ReleaseRow {
            id: 0,
            product: PRODUCT.into(),
            version: "1.0".into(),
            start: "2026-11-01".into(),
            target: "2026-12-01".into(),
            released_at: None,
        })
        .await
        .unwrap();
    let mut r = req(&fx, "lost");
    r.release = Some(release);
    let store = fx.store.clone();
    let r = spawn_losing(&fx, r, async move {
        store.delete_release(release).await.unwrap();
    })
    .await;
    assert!(
        matches!(&r, Err(SpawnError::ReleaseNotInProduct { release: x, .. }) if *x == release),
        "{r:?}"
    );
    assert_nothing_left(&fx, "lost").await;
}

#[tokio::test]
async fn a_stopped_card_is_never_paused() {
    let fx = fixture().await;
    let pause = |id: AgentId| {
        let store = fx.store.clone();
        async move { store.update_agent(&id, |x| x.paused = true).await.unwrap() }
    };
    let by_you = fx.svc.spawn(req(&fx, "one")).await.unwrap();
    pause(by_you.id.clone()).await;
    fx.svc.stop(&by_you.id).await.unwrap();

    let exited = fx.svc.spawn(req(&fx, "two")).await.unwrap();
    pause(exited.id.clone()).await;
    send_hook(&fx, &exited.id, "SessionEnd").await;

    let crashed = fx.svc.spawn(req(&fx, "three")).await.unwrap();
    pause(crashed.id.clone()).await;
    fx.tmux.kill(&session_name(&crashed.id)).await.unwrap();
    fx.svc.reconcile().await.unwrap();

    for (id, cause) in [
        (&by_you.id, StopCause::Stopped),
        (&exited.id, StopCause::Exited),
        (&crashed.id, StopCause::Crashed),
    ] {
        let a = agent(&fx, id).await;
        assert_eq!(
            (a.status, a.stop_cause, a.paused),
            (Status::Stopped, Some(cause), false)
        );
    }
}

#[tokio::test]
async fn stopping_twice_is_a_no_op() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    fx.svc.stop(&a.id).await.unwrap();
    // The first stop's measurement lands in the background; it must not be taken for a write by the second.
    assert!(
        wait_until(|| async { agent(&fx, &a.id).await.disk_bytes.is_some() }).await,
        "the stop was never measured"
    );
    let before = (
        fx.store.snapshot().await.unwrap().seq,
        fx.store.task(&a.id).await.unwrap().unwrap().log.len(),
    );
    fx.clock.advance(60);
    fx.svc.stop(&a.id).await.unwrap();
    let after = (
        fx.store.snapshot().await.unwrap().seq,
        fx.store.task(&a.id).await.unwrap().unwrap().log.len(),
    );
    assert_eq!(
        before, after,
        "the second stop wrote an event or a log line"
    );
}

#[tokio::test]
async fn a_stopped_agent_stays_stopped_across_a_daemon_restart() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    fx.svc.stop(&a.id).await.unwrap();
    let second = fleet_daemon::agents::AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        fx.svc.options().clone(),
    );
    second.reconcile().await.unwrap();
    second.mark_unverified().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert_eq!(
        (after.status, after.stop_cause, after.activity.as_str()),
        (Status::Stopped, Some(StopCause::Stopped), "stopped by you")
    );
}

#[tokio::test]
async fn a_lost_session_is_stopped_as_crashed_with_its_last_activity() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    let pre = json!({"hook_event_name": "PreToolUse", "tool_name": "Bash"});
    ingest(&fx.store, &a.id, Some(&token), pre, fx.clock.now_for_test())
        .await
        .unwrap();
    assert_eq!(agent(&fx, &a.id).await.activity, "running Bash");
    fx.tmux.kill(&session_name(&a.id)).await.unwrap();
    fx.svc.reconcile().await.unwrap();
    let a = agent(&fx, &a.id).await;
    assert_eq!(a.status, Status::Stopped);
    assert_eq!(a.stop_cause, Some(StopCause::Crashed));
    assert_eq!(a.activity, "crashed while running Bash");
}

#[tokio::test]
async fn a_stored_dead_agent_is_read_as_stopped_with_its_cause() {
    let fx = fixture().await;
    let template = agent(&fx, &fx.svc.spawn(req(&fx, "x")).await.unwrap().id).await;
    // Built from JSON the way an old row is stored: "dead" and no stop_cause key.
    let legacy = |id: &str, activity: &str| -> Agent {
        let mut v = serde_json::to_value(&template).unwrap();
        v["id"] = json!(id);
        v["status"] = json!("dead");
        v["activity"] = json!(activity);
        v.as_object_mut().unwrap().remove("stop_cause");
        serde_json::from_value(v).unwrap()
    };
    for (id, activity) in [("ended", "session ended"), ("lost", "session lost")] {
        fx.store
            .insert_agent(&legacy(id, activity), "tok")
            .await
            .unwrap();
    }
    fx.svc.adopt_legacy_dead().await.unwrap();
    let ended = agent(&fx, &AgentId("ended".into())).await;
    let lost = agent(&fx, &AgentId("lost".into())).await;
    assert_eq!(
        (ended.status, ended.stop_cause),
        (Status::Stopped, Some(StopCause::Exited))
    );
    assert_eq!(ended.activity, "session ended");
    assert_eq!(
        (lost.status, lost.stop_cause),
        (Status::Stopped, Some(StopCause::Crashed))
    );
    assert_eq!(lost.activity, "crashed while session lost");
    // A second run finds nothing left to adopt.
    fx.svc.adopt_legacy_dead().await.unwrap();
    assert_eq!(
        agent(&fx, &AgentId("lost".into())).await.activity,
        "crashed while session lost"
    );
}

#[tokio::test]
async fn a_hook_for_a_stopped_agent_changes_nothing() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    send_hook(&fx, &a.id, "SessionEnd").await;
    let stopped = agent(&fx, &a.id).await;
    assert_eq!(stopped.stop_cause, Some(StopCause::Exited));
    assert_eq!(stopped.activity, "agent exited");
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    let out = ingest(
        &fx.store,
        &a.id,
        Some(&token),
        hook("SessionStart"),
        fx.clock.now_for_test(),
    )
    .await
    .unwrap();
    assert!(matches!(out, IngestOutcome::Ignored));
    assert_eq!(agent(&fx, &a.id).await, stopped);
}

#[tokio::test]
async fn session_that_exits_immediately_is_dead_after_reconcile() {
    let fx = fixture_with(vec!["true".into()], None).await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    assert!(
        wait_until(|| async { !fx.tmux.has_session(&session_name(&a.id)).await.unwrap() }).await
    );
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Stopped);
}

#[tokio::test]
async fn a_restarted_service_adopts_live_sessions_and_kills_none() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
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
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    fx.clock.advance(19);
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Queued);
    fx.clock.advance(2);
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert_eq!(
        (after.status, after.activity.as_str()),
        (Status::NeedsYou, "waiting at startup prompt")
    );
    assert_eq!(after.blocked_reason, Some(BlockedReason::Startup));
}

#[tokio::test]
async fn session_start_cancels_the_startup_rule() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    send_hook(&fx, &a.id, "SessionStart").await;
    fx.clock.advance(60);
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);
}

#[tokio::test]
async fn startup_prompt_then_session_start_returns_to_working() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    fx.clock.advance(30);
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::NeedsYou);
    send_hook(&fx, &a.id, "SessionStart").await;
    let after = agent(&fx, &a.id).await;
    assert_eq!(
        (after.status, after.blocked_reason),
        (Status::Working, None)
    );
}

#[tokio::test]
async fn mark_unverified_flags_agents_whose_events_may_have_been_missed() {
    let fx = fixture().await;
    let working = fx.svc.spawn(req(&fx, "one")).await.unwrap();
    send_hook(&fx, &working.id, "UserPromptSubmit").await;
    let done = fx.svc.spawn(req(&fx, "two")).await.unwrap();
    send_hook(&fx, &done.id, "Stop").await;
    fx.svc.mark_unverified().await.unwrap();
    let w = agent(&fx, &working.id).await;
    assert_eq!(
        (w.status, w.activity.as_str()),
        (Status::Working, "status unverified since daemon restart")
    );
    let d = agent(&fx, &done.id).await;
    assert_eq!(
        (d.status, d.activity.as_str()),
        (Status::Review, "finished"),
        "a finished agent is not in doubt"
    );
    // The next real event replaces the doubt with fact.
    send_hook(&fx, &working.id, "PreToolUse").await;
    assert_eq!(agent(&fx, &working.id).await.activity, "running tool");
}

#[tokio::test]
async fn reconcile_does_not_overwrite_a_status_set_after_its_snapshot() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    let stale = vec![agent(&fx, &a.id).await]; // read while still Queued
    fx.clock.advance(30);
    send_hook(&fx, &a.id, "SessionStart").await; // lands after the snapshot
    fx.svc.reconcile_with(stale).await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);
}

#[tokio::test]
async fn one_vanished_agent_does_not_stop_reconcile_for_the_others() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    fx.tmux.kill(&session_name(&a.id)).await.unwrap();
    let mut ghost = agent(&fx, &a.id).await;
    ghost.id = AgentId("ghost-not-in-the-store".into()); // killed between snapshot and write
    ghost.status = Status::Working;
    let real = agent(&fx, &a.id).await;
    fx.svc.reconcile_with(vec![ghost, real]).await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Stopped);
}

#[tokio::test]
async fn concurrent_spawns_of_the_same_prompt_get_distinct_task_dirs() {
    let fx = fixture().await;
    let (a, b) = tokio::join!(
        fx.svc.spawn(req(&fx, "same")),
        fx.svc.spawn(req(&fx, "same"))
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.workdir, b.workdir);
    assert_ne!(a.workspaces[0].branch, b.workspaces[0].branch);
}

#[tokio::test]
async fn diff_covers_every_workspace_and_unknown_agents_are_not_found() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    std::fs::write(
        PathBuf::from(&a.workspaces[0].worktree).join("new.txt"),
        "hello\n",
    )
    .unwrap();
    let report = fx.svc.diff(&a.id).await.unwrap();
    assert_eq!(report.workspaces.len(), 1);
    assert_eq!(report.workspaces[0].repo, "core");
    assert!(
        report.workspaces[0].diff.contains("+hello"),
        "{}",
        report.workspaces[0].diff
    );
    assert!(matches!(
        fx.svc.diff(&AgentId("zz".into())).await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn reconcile_refreshes_has_changes_for_agents_in_review() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "x")).await.unwrap();
    send_hook(&fx, &a.id, "Stop").await;
    fx.svc.reconcile().await.unwrap();
    assert!(!agent(&fx, &a.id).await.has_changes);
    std::fs::write(
        PathBuf::from(&a.workspaces[0].worktree).join("new.txt"),
        "x",
    )
    .unwrap();
    fx.svc.reconcile().await.unwrap();
    assert!(agent(&fx, &a.id).await.has_changes);
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn spawn_with_no_repos_makes_an_empty_task_folder() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(req_with(&fx, "just think", &[]))
        .await
        .unwrap();
    let workdir = PathBuf::from(&a.workdir);
    assert_eq!(workdir, fx.tasks_root.join("just-think"));
    assert!(a.workspaces.is_empty());
    assert_eq!(entries(&workdir), ["CLAUDE.md", "PROMPT.md"]);
    let text = std::fs::read_to_string(workdir.join("CLAUDE.md")).unwrap();
    assert!(text.contains("This task has no repos yet."), "{text}");
    assert!(
        text.contains("To start a new repo, create a subfolder and run `git init` in it."),
        "{text}"
    );
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
    let a = fx
        .svc
        .spawn(req_with(
            &fx,
            "two of them",
            &["core", tools.to_str().unwrap()],
        ))
        .await
        .unwrap();
    let workdir = PathBuf::from(&a.workdir);
    let names: Vec<_> = a
        .workspaces
        .iter()
        .map(|w| (w.repo.as_str(), w.branch.as_str(), w.owned))
        .collect();
    assert_eq!(
        names,
        [
            ("core", "fleet/two-of-them", false),
            ("tools", "fleet/two-of-them", false)
        ]
    );
    for w in &a.workspaces {
        assert_eq!(PathBuf::from(&w.worktree), workdir.join(&w.repo));
        assert!(workdir.join(&w.repo).join("README").is_file());
    }
    assert!(!git(
        &tools,
        &[
            "rev-parse",
            "--verify",
            "-q",
            "refs/heads/fleet/two-of-them"
        ]
    )
    .is_empty());
    let text = std::fs::read_to_string(workdir.join("CLAUDE.md")).unwrap();
    assert!(
        text.contains("- `core/` (branch `fleet/two-of-them`)")
            && text.contains("- `tools/` (branch `fleet/two-of-them`)"),
        "{text}"
    );
    assert!(
        text.contains("request_repo") && text.contains("git init"),
        "{text}"
    );
}

#[tokio::test]
async fn spawn_with_two_checkouts_of_one_repo_keeps_the_first() {
    let fx = fixture().await;
    let base = make_repo(&fx._tmp.path().join("dev").join("foo"));
    let feat = fx._tmp.path().join("dev").join("foo-feature");
    git(
        &base,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat",
            feat.to_str().unwrap(),
        ],
    );
    std::fs::write(feat.join("extra"), "x").unwrap();
    git(&feat, &["add", "."]);
    git(&feat, &["commit", "-q", "-m", "extra"]);

    let a = fx
        .svc
        .spawn(req_with(
            &fx,
            "both",
            &[base.to_str().unwrap(), feat.to_str().unwrap()],
        ))
        .await
        .unwrap();
    assert_eq!(a.workspaces.len(), 1);
    assert_eq!(a.workspaces[0].base, git(&base, &["rev-parse", "HEAD"]));
    assert!(!PathBuf::from(&a.workspaces[0].worktree)
        .join("extra")
        .exists());
}

#[tokio::test]
async fn spawn_records_from_branch_on_the_workspace() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(req_with(&fx, "record it", &["core"]))
        .await
        .unwrap();
    let branch = git(&fx.repo, &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(
        a.workspaces[0].from_branch.as_deref(),
        Some(branch.as_str())
    );
}

#[tokio::test]
async fn spawn_with_a_bad_repo_creates_nothing() {
    let fx = fixture().await;
    let err = fx
        .svc
        .spawn(req_with(&fx, "bad one", &["core", "not a repo at all"]))
        .await
        .unwrap_err();
    assert!(matches!(err, SpawnError::Repo(_)), "{err:?}");
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(
        entries(&fx.tasks_root).is_empty(),
        "task folder left behind"
    );
    assert!(
        git(&fx.repo, &["branch", "--list", "fleet/*"]).is_empty(),
        "branch left behind"
    );
    assert!(fx.tmux.list_sessions("").await.unwrap().is_empty());
}

#[tokio::test]
async fn second_spawn_with_the_same_goal_gets_slug_2() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with(&fx, "same goal", &[])).await.unwrap();
    let b = fx
        .svc
        .spawn(req_with(&fx, "same goal", &["core"]))
        .await
        .unwrap();
    assert_eq!(PathBuf::from(&a.workdir), fx.tasks_root.join("same-goal"));
    assert_eq!(PathBuf::from(&b.workdir), fx.tasks_root.join("same-goal-2"));
    assert_eq!(b.workspaces[0].branch, "fleet/same-goal-2");
}

#[tokio::test]
async fn a_left_over_branch_also_moves_the_slug_on() {
    let fx = fixture().await;
    git(&fx.repo, &["branch", "fleet/taken"]);
    let a = fx
        .svc
        .spawn(req_with(&fx, "taken", &["core"]))
        .await
        .unwrap();
    assert_eq!(PathBuf::from(&a.workdir), fx.tasks_root.join("taken-2"));
}

#[tokio::test]
async fn attach_repo_adds_a_workspace_and_tells_the_agent() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let a = fx
        .svc
        .spawn(req_with(&fx, "grow", &["core"]))
        .await
        .unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    // A working agent reads the line as text; see the dialog tests below for when nothing is typed.
    send_hook(&fx, &a.id, "UserPromptSubmit").await;
    let mut rx = fx.store.subscribe();
    let w = fx
        .svc
        .attach_repo(&a.id, tools.to_str().unwrap())
        .await
        .unwrap();
    let workdir = PathBuf::from(&a.workdir);
    assert_eq!(
        (w.repo.as_str(), w.branch.as_str(), w.owned),
        ("tools", "fleet/grow", false)
    );
    assert_eq!(PathBuf::from(&w.worktree), workdir.join("tools"));
    assert!(workdir.join("tools/README").is_file());
    assert_eq!(agent(&fx, &a.id).await.workspaces.len(), 2);
    let mut changed = false;
    while let Ok(e) = rx.try_recv() {
        changed |= matches!(e.kind, EventKind::WorkspacesChanged { ref workspaces, .. } if workspaces.len() == 2);
    }
    assert!(changed, "no WorkspacesChanged");
    let text = std::fs::read_to_string(workdir.join("CLAUDE.md")).unwrap();
    assert!(
        text.contains("- `tools/` (branch `fleet/grow`)") && text.contains("- `core/`"),
        "{text}"
    );
    assert!(pane_contains(&fx, &a.id, "repo tools is now available").await);
    assert!(
        fx.store.repo(PERSONA, "tools").await.unwrap().is_some(),
        "an attached repo is remembered"
    );
}

#[tokio::test]
async fn attach_to_a_task_with_no_repos_uses_the_folder_branch() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(req_with(&fx, "empty start", &[]))
        .await
        .unwrap();
    let w = fx.svc.attach_repo(&a.id, "core").await.unwrap();
    assert_eq!(w.branch, "fleet/empty-start");
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(
        !text.contains("no repos yet") && text.contains("- `core/`"),
        "{text}"
    );
}

#[tokio::test]
async fn attaching_a_repo_twice_is_a_no_op() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(req_with(&fx, "twice", &["core"]))
        .await
        .unwrap();
    let w = fx.svc.attach_repo(&a.id, "core").await.unwrap();
    assert_eq!(w, a.workspaces[0]);
    let w2 = fx
        .svc
        .attach_repo(&a.id, fx.repo.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(w2, a.workspaces[0]);
    assert_eq!(agent(&fx, &a.id).await.workspaces, a.workspaces);
}

#[tokio::test]
async fn attaching_a_bad_repo_is_a_repo_error() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(req_with(&fx, "bad attach", &[]))
        .await
        .unwrap();
    assert!(matches!(
        fx.svc.attach_repo(&a.id, "no such thing").await,
        Err(ServiceError::Repo(_))
    ));
    assert!(matches!(
        fx.svc.attach_repo(&AgentId("zz".into()), "core").await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn an_agent_initialised_repo_is_adopted_as_owned() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(req_with(&fx, "new tool", &["core"]))
        .await
        .unwrap();
    let workdir = PathBuf::from(&a.workdir);
    make_repo(&workdir.join("newtool"));
    std::fs::create_dir_all(workdir.join("notes")).unwrap(); // a plain folder is not a repo
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert_eq!(after.workspaces.len(), 2, "{:?}", after.workspaces);
    let w = &after.workspaces[1];
    assert_eq!(
        (w.repo.as_str(), w.branch.as_str(), w.owned),
        ("newtool", "main", true)
    );
    assert_eq!(w.base, "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
    assert_eq!(PathBuf::from(&w.worktree), workdir.join("newtool"));
    let row = fx
        .store
        .repo(PRODUCT, "newtool")
        .await
        .unwrap()
        .expect("owned repo added to the product");
    assert_eq!(
        row.path,
        Some(std::fs::canonicalize(workdir.join("newtool")).unwrap())
    );
    let text = std::fs::read_to_string(workdir.join("CLAUDE.md")).unwrap();
    assert!(text.contains("- `newtool/` (branch `main`)"), "{text}");
    // Its diff shows everything, since its base is the empty tree.
    let d = fx.svc.diff(&a.id).await.unwrap();
    assert!(
        d.workspaces[1].diff.contains("+hi"),
        "{}",
        d.workspaces[1].diff
    );
    // Idempotent.
    fx.svc.reconcile().await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.workspaces.len(), 2);
}

#[tokio::test]
async fn an_owned_repo_is_never_offered_to_another_task() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(req_with(&fx, "new tool", &["core"]))
        .await
        .unwrap();
    make_repo(&PathBuf::from(&a.workdir).join("newtool"));
    fx.svc.reconcile().await.unwrap();
    let names: Vec<_> = fx
        .svc
        .remembered_repos(None)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(names, ["core"]);
    let b = fx.svc.spawn(req_with(&fx, "other", &[])).await.unwrap();
    assert!(matches!(
        fx.svc.attach_repo(&b.id, "newtool").await,
        Err(ServiceError::Repo(RepoError::InsideTasks(_)))
    ));
    // Its own task still answers from its workspaces.
    assert!(fx.svc.attach_repo(&a.id, "newtool").await.unwrap().owned);
}

#[tokio::test]
async fn an_unborn_agent_repo_is_adopted_on_its_branch() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with(&fx, "unborn", &[])).await.unwrap();
    let dir = PathBuf::from(&a.workdir).join("fresh");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "trunk"]);
    fx.svc.reconcile().await.unwrap();
    let after = agent(&fx, &a.id).await;
    assert_eq!(
        after
            .workspaces
            .iter()
            .map(|w| (w.repo.as_str(), w.branch.as_str(), w.owned))
            .collect::<Vec<_>>(),
        [("fresh", "trunk", true)]
    );
}

#[tokio::test]
async fn concurrent_attaches_to_one_task_both_land() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let a = fx.svc.spawn(req_with(&fx, "both", &[])).await.unwrap();
    let (x, y) = tokio::join!(
        fx.svc.attach_repo(&a.id, "core"),
        fx.svc.attach_repo(&a.id, tools.to_str().unwrap())
    );
    x.unwrap();
    y.unwrap();
    let mut names: Vec<_> = agent(&fx, &a.id)
        .await
        .workspaces
        .into_iter()
        .map(|w| w.repo)
        .collect();
    names.sort();
    assert_eq!(names, ["core", "tools"]);
}

#[tokio::test]
async fn attach_with_an_existing_branch_is_a_clear_conflict() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req_with(&fx, "clash", &[])).await.unwrap();
    // Remembered only after the name was picked, so the picker could not see its branch.
    let tools = make_repo(&fx._tmp.path().join("tools"));
    git(&tools, &["branch", "fleet/clash"]);
    let before = git(&tools, &["rev-parse", "fleet/clash"]);
    let err = fx
        .svc
        .attach_repo(&a.id, tools.to_str().unwrap())
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "branch fleet/clash already exists in tools; delete or rename it, then try again"
    );
    assert!(
        matches!(
            err,
            ServiceError::Repo(fleet_daemon::repos::RepoError::BranchExists { .. })
        ),
        "{err:?}"
    );
    assert_eq!(
        git(&tools, &["rev-parse", "fleet/clash"]),
        before,
        "the old branch must never be reused or moved"
    );
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
    let a = fx
        .svc
        .spawn(req_with(&fx, "asks", &["core"]))
        .await
        .unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    // Queued: the startup trust dialog may be on screen.
    fx.svc
        .attach_repo(&a.id, tools.to_str().unwrap())
        .await
        .unwrap();
    send_hook(&fx, &a.id, "UserPromptSubmit").await;
    fx.store
        .update_agent(&a.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::Permission);
        })
        .await
        .unwrap();
    fx.svc
        .attach_repo(&a.id, other.to_str().unwrap())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let pane = fx.tmux.capture(&session_name(&a.id)).await.unwrap();
    assert!(!pane.contains("fleet-manager:"), "{pane}");
    let text = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(
        text.contains("- `tools/`") && text.contains("- `other/`"),
        "{text}"
    );
}

async fn log_texts(fx: &Fx, id: &AgentId) -> Vec<String> {
    let p = fx.store.task(id).await.unwrap().unwrap();
    p.log.into_iter().map(|l| l.text).collect()
}

async fn ask_for_repo(fx: &Fx, id: &AgentId, repo: &str) {
    let request = fleet_api::RepoRequest {
        repo: repo.into(),
        why: "needed".into(),
    };
    fx.store
        .update_agent(id, |a| {
            a.status = Status::NeedsYou;
            a.blocked_reason = Some(BlockedReason::RepoAccess);
            a.repo_request = Some(request);
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn spawn_and_repo_changes_are_logged_on_the_task() {
    let fx = fixture().await;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let other = make_repo(&fx._tmp.path().join("other"));
    let a = fx.svc.spawn(req(&fx, "grow")).await.unwrap();
    fx.svc
        .attach_repo(&a.id, tools.to_str().unwrap())
        .await
        .unwrap();
    ask_for_repo(&fx, &a.id, "other").await;
    fx.svc
        .grant_repo(&a.id, other.to_str().unwrap())
        .await
        .unwrap();
    ask_for_repo(&fx, &a.id, "third").await;
    fx.svc.refuse_repo(&a.id, "not this one").await.unwrap();
    assert_eq!(
        log_texts(&fx, &a.id).await,
        [
            "started with core",
            "repo tools added",
            "repo other granted",
            "repo request refused: not this one"
        ]
    );
    assert_eq!(fx.store.task(&a.id).await.unwrap().unwrap().repos.len(), 3);
}

#[tokio::test]
async fn mark_unverified_logs_on_the_task() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "one")).await.unwrap();
    send_hook(&fx, &a.id, "UserPromptSubmit").await;
    fx.svc.mark_unverified().await.unwrap();
    assert_eq!(
        log_texts(&fx, &a.id).await.last().unwrap(),
        "daemon restarted, status unverified"
    );
}

#[tokio::test]
async fn spawn_runs_with_persona_env() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "persona env")).await.unwrap();
    assert_eq!(a.persona, PERSONA);
    let claude_dir = fx._tmp.path().join("personas").join(PERSONA).join("claude");
    for needle in [
        format!("claude_config={}", claude_dir.display()),
        "gh_host=github.com".to_string(),
        format!("author={COMMIT_EMAIL}"),
    ] {
        assert!(pane_contains(&fx, &a.id, &needle).await, "{needle}");
    }
    assert_eq!(
        fx.store.task(&a.id).await.unwrap().unwrap().persona,
        PERSONA
    );
}

#[tokio::test]
async fn the_agent_gets_no_ambient_token() {
    let sock = format!("fleetambient-{}", std::process::id());
    let fx = fixture_with(fake_claude(), Some(sock.clone())).await;
    // A tmux server started from a shell that had tokens keeps them in its global environment, which every new
    // session inherits.
    let tmux = |args: &[&str]| {
        let ok = std::process::Command::new("tmux")
            .args(["-L", &sock])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "tmux {args:?}");
    };
    tmux(&["new-session", "-d", "-s", "keep"]);
    tmux(&["set-environment", "-g", "GH_TOKEN", "leak"]);
    tmux(&["set-environment", "-g", "ANTHROPIC_API_KEY", "leak"]);
    tmux(&[
        "set-environment",
        "-g",
        "GIT_CONFIG_PARAMETERS",
        "'credential.helper'='leak'",
    ]);
    // A path nothing runs: the real desktop askpass must never be reachable from a test.
    tmux(&[
        "set-environment",
        "-g",
        "SSH_ASKPASS",
        "/nonexistent/askpass",
    ]);
    tmux(&[
        "set-environment",
        "-g",
        "GIT_ASKPASS",
        "/nonexistent/askpass",
    ]);
    tmux(&[
        "set-environment",
        "-g",
        "SSH_AUTH_SOCK",
        "/nonexistent/agent.sock",
    ]);
    let a = fx.svc.spawn(req(&fx, "no tokens")).await.unwrap();
    assert!(
        pane_contains(
            &fx,
            &a.id,
            "gh_token=unset anthropic_key=unset git_params=unset ssh_askpass=unset ssh_auth_sock=unset git_askpass=[]"
        )
        .await,
        "{:?}",
        fx.tmux.capture(&session_name(&a.id)).await
    );

    // A window the user opens in the agent's session gets none of them either.
    let out = fx._tmp.path().join("window-env");
    let session = session_name(&a.id);
    tmux(&[
        "new-window",
        "-t",
        &format!("={session}:"),
        &format!("env > {}.tmp && mv {0}.tmp {0}", out.display()),
    ]);
    assert!(wait_until(|| async { out.exists() }).await);
    let env = std::fs::read_to_string(&out).unwrap();
    for k in fleet_api::ambient_vars() {
        assert!(!env.contains(&format!("{k}=")), "{k} reached a new window");
    }
    // And attaching from a terminal that has them does not copy them back in.
    let updated = std::process::Command::new("tmux")
        .args(["-L", &sock, "show-options", "-v", "-t"])
        .arg(format!("={session}:"))
        .arg("update-environment")
        .output()
        .unwrap();
    let updated = String::from_utf8_lossy(&updated.stdout);
    assert!(updated.contains("DISPLAY"), "{updated}");
    for k in ["SSH_AUTH_SOCK", "SSH_ASKPASS"] {
        assert!(!updated.contains(k), "{k} in {updated}");
    }
}

#[tokio::test]
async fn spawn_without_persona_is_refused() {
    let fx = fixture().await;
    let mut r = req(&fx, "no persona");
    r.persona = String::new();
    let err = fx.svc.spawn(r).await.unwrap_err();
    assert!(matches!(err, SpawnError::NoPersona), "{err:?}");
    assert_eq!(err.to_string(), "pick a persona for this task");
    assert_eq!(std::fs::read_dir(&fx.tasks_root).unwrap().count(), 0);
}

#[tokio::test]
async fn spawn_on_broken_login_creates_nothing() {
    let fx = fixture().await;
    let gh = fx._tmp.path().join("personas").join(PERSONA).join("gh");
    std::fs::remove_file(gh.join("hosts.yml")).unwrap();
    let err = fx.svc.spawn(req(&fx, "broken")).await.unwrap_err();
    assert!(
        matches!(
            &err,
            SpawnError::Persona(PersonaError::LoginBroken {
                which: "GitHub",
                ..
            })
        ),
        "{err:?}"
    );
    assert_eq!(std::fs::read_dir(&fx.tasks_root).unwrap().count(), 0);
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(fx.store.tasks().await.unwrap().is_empty());
    assert!(fx.tmux.list_sessions("fleet-").await.unwrap().is_empty());
}

#[tokio::test]
async fn attach_uses_agent_persona() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "attach elsewhere")).await.unwrap();
    let work = make_repo(&fx._tmp.path().join("work"));
    git(
        &work,
        &["remote", "add", "origin", "https://ghe.example.invalid/a/b"],
    );
    let err = fx
        .svc
        .attach_repo(&a.id, work.to_str().unwrap())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ServiceError::Repo(RepoError::WrongHost { persona, .. }) if persona == PERSONA),
        "{err:?}"
    );
    assert_eq!(agent(&fx, &a.id).await.workspaces.len(), 1);
}

#[tokio::test]
async fn a_record_without_a_persona_gets_no_repo() {
    let fx = fixture().await;
    let a = fx.svc.spawn(req(&fx, "old record")).await.unwrap();
    // As a record from before personas that no persona has adopted yet.
    fx.store
        .update_agent(&a.id, |x| x.persona = String::new())
        .await
        .unwrap();
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let err = fx
        .svc
        .attach_repo(&a.id, tools.to_str().unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NoPersona), "{err:?}");
}

#[tokio::test]
async fn a_failed_spawn_is_archived_after_its_cleanup_and_names_what_stayed() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: getuid(2) takes no arguments and cannot fail.
    if unsafe { libc::getuid() } == 0 {
        return; // root deletes the read-only folder below anyway
    }
    let fx = fixture_with(fake_claude(), Some("x".repeat(300))).await;
    // Run in the new worktree by `git worktree add`: leaves a folder in the task folder that cannot be emptied.
    let hook = fx.repo.join(".git/hooks/post-checkout");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\nmkdir -p ../stuck && touch ../stuck/f && chmod 500 ../stuck\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        fx.svc.spawn(req(&fx, "stuck")).await,
        Err(SpawnError::Tmux(_))
    ));
    let workdir = fx.tasks_root.join("stuck");
    let stuck = workdir.join("stuck");
    assert!(stuck.join("f").exists());
    let records = fx.store.tasks().await.unwrap();
    assert_eq!(records.len(), 1, "{records:?}");
    let p = &records[0];
    assert_eq!(p.state, TaskState::Archived);
    let kept = p
        .archive
        .as_ref()
        .and_then(|r| r.kept.clone())
        .unwrap_or_default();
    assert!(
        kept.contains(&workdir.to_string_lossy().into_owned()),
        "{kept:?}"
    );
    // The worktree itself went: only the folder git could not empty stayed.
    assert!(!workdir.join("core").exists());
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o700)).unwrap();
}

async fn add_product(fx: &Fx, id: &str) {
    fx.store
        .insert_product(&fleet_daemon::store::ProductRow {
            id: id.into(),
            name: id.into(),
            default_persona: None,
            created_at: 2,
        })
        .await
        .unwrap();
}

async fn add_release(fx: &Fx, product: &str, version: &str) -> i64 {
    fx.store
        .insert_release(&fleet_daemon::store::ReleaseRow {
            id: 0,
            product: product.into(),
            version: version.into(),
            start: "2026-11-01".into(),
            target: "2026-12-01".into(),
            released_at: None,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn spawn_into_unknown_product_is_400() {
    let fx = fixture().await;
    // With no repo too: the product is checked before any repo is.
    for repos in [&["core"][..], &[]] {
        let mut r = req_with(&fx, "x", repos);
        r.product = "nope".into();
        let err = fx.svc.spawn(r).await.unwrap_err();
        assert!(
            matches!(&err, SpawnError::Repo(RepoError::UnknownProduct(p)) if p == "nope"),
            "{err:?}"
        );
        assert_eq!(err.to_string(), "no such product: nope");
    }
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(walk_is_empty(&fx.tasks_root));
}

#[tokio::test]
async fn spawn_into_other_products_release_is_400() {
    let fx = fixture().await;
    add_product(&fx, "other").await;
    let theirs = add_release(&fx, "other", "v1").await;
    for release in [theirs, 9999] {
        let mut r = req(&fx, "x");
        r.release = Some(release);
        let err = fx.svc.spawn(r).await.unwrap_err();
        assert!(
            matches!(&err, SpawnError::ReleaseNotInProduct { release: got, product } if *got == release && product == PRODUCT),
            "{err:?}"
        );
    }
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(walk_is_empty(&fx.tasks_root));
}

#[tokio::test]
async fn agent_carries_its_product() {
    let fx = fixture().await;
    let v1 = add_release(&fx, PRODUCT, "v1").await;
    let mut r = req(&fx, "ship it");
    r.release = Some(v1);
    let a = fx.svc.spawn(r).await.unwrap();
    assert_eq!(a.product, PRODUCT);
    assert_eq!(agent(&fx, &a.id).await.product, PRODUCT);
    let t = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!((t.product.as_str(), t.release), (PRODUCT, Some(v1)));
    // Without a release the task is in the backlog.
    let b = fx.svc.spawn(req_with(&fx, "later", &[])).await.unwrap();
    assert_eq!(fx.store.task(&b.id).await.unwrap().unwrap().release, None);
}

#[tokio::test]
async fn wrong_host_url_is_refused_at_start() {
    let fx = fixture().await;
    fx.store
        .upsert_repo(&fleet_daemon::store::RepoRow {
            product: PRODUCT.into(),
            name: "lab".into(),
            path: None,
            url: Some("https://gitlab.example/o/lab.git".into()),
            roster: vec![],
        })
        .await
        .unwrap();
    let err = fx
        .svc
        .spawn(req_with(&fx, "x", &["lab"]))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, SpawnError::Repo(RepoError::WrongHost { host, persona, .. }) if host == "gitlab.example" && persona == PERSONA),
        "{err:?}"
    );
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(!fx.state_dir.join("repos").join(PERSONA).exists());
}

#[tokio::test]
async fn a_checkout_behind_its_url_warns_at_spawn() {
    let fx = fixture().await;
    let bare = add_origin(&fx);
    let url = format!("file://{}", bare.display());
    let other = fx._tmp.path().join("other");
    git(
        fx._tmp.path(),
        &["clone", "-q", &url, other.to_str().unwrap()],
    );
    std::fs::write(other.join("ahead"), "x").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-q", "-m", "ahead"]);
    git(&other, &["push", "-q", "origin", "main"]);
    let mut core = fx.store.repo(PRODUCT, "core").await.unwrap().unwrap();
    core.url = Some(url);
    fx.store.upsert_repo(&core).await.unwrap();
    let before = git(&fx.repo, &["for-each-ref", "refs/remotes/origin"]);

    let r = fx.svc.spawn_reporting(req(&fx, "behind")).await.unwrap();
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    assert!(
        r.warnings[0].starts_with("main in ") && r.warnings[0].contains("is behind"),
        "{:?}",
        r.warnings
    );
    // Read-only: the checkout's remote-tracking refs are as they were.
    assert_eq!(
        git(&fx.repo, &["for-each-ref", "refs/remotes/origin"]),
        before
    );
}
