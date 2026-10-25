mod common;

use common::net::*;
use common::*;
use fleet_api::{
    Actor, AgentId, AgentKind, Approval, BlockedReason, DecideBreakdown, DefKind, RepoFleet,
    SpawnRequest, Status, StepDraft,
};
use fleet_client::http::FleetClient;
use fleet_daemon::agents::{AgentService, ServiceError, SpawnError, PROMPT_FILE};
use fleet_daemon::fleet::breakdown::{BreakdownAnswer, BreakdownError};
use fleet_daemon::fleet::defs::{Defs, FleetError};
use fleet_daemon::hooks_cfg::ORCHESTRATOR_TOOLS;
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

fn request(kind: AgentKind, role: Option<&str>, repos: &[&str], roster: &[&str]) -> SpawnRequest {
    SpawnRequest {
        actor: Actor("local".into()),
        repos: repos.iter().map(|r| r.to_string()).collect(),
        prompt: "fix the login bug".into(),
        task_slug: None,
        persona: PERSONA.into(),
        kind,
        role: role.map(String::from),
        roster: roster.iter().map(|r| r.to_string()).collect(),
    }
}

fn orchestrator(role: &str) -> SpawnRequest {
    request(AgentKind::Orchestrator, Some(role), &["core"], &[])
}

async fn set_fleet(fx: &Fx, roster: &[&str], test_command: Option<&str>) {
    fx.store
        .set_repo_fleet(
            PERSONA,
            "core",
            &RepoFleet {
                roster: roster.iter().map(|r| r.to_string()).collect(),
                test_command: test_command.map(String::from),
            },
        )
        .await
        .unwrap();
}

/// A fixture whose `core` repo is ready for an orchestrated project.
async fn fleet_fixture() -> Fx {
    let fx = fixture().await;
    set_fleet(&fx, &["rust"], Some("true")).await;
    fx
}

fn skills_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join(".claude/skills"))
        .map(|e| {
            e.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Nothing a refused spawn could have left: no folder, card, record or branch.
async fn assert_nothing_created(fx: &Fx) {
    assert!(entries(&fx.projects_root).is_empty());
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(fx.store.projects().await.unwrap().is_empty());
    assert_eq!(git(&fx.repo, &["branch", "--list", "fleet/*"]), "");
}

#[tokio::test]
async fn orchestrator_spawn_layout() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    assert_eq!(a.kind, AgentKind::Orchestrator);
    assert_eq!(a.role.as_deref(), Some("direct"));

    let cwd = PathBuf::from(&a.workdir);
    let project = cwd.parent().unwrap().to_path_buf();
    assert_eq!(cwd.file_name().unwrap(), "orchestrator");
    assert_eq!(project.parent().unwrap(), fx.projects_root);
    let name = project.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        pane_contains(
            &fx,
            &a.id,
            &format!("cwd={}", cwd.canonicalize().unwrap().display())
        )
        .await
    );

    // The project folder and its parents hold nothing Claude Code would load for every agent under it.
    assert_eq!(entries(&project), ["orchestrator"]);
    assert_eq!(
        entries(&cwd),
        [".claude", "CLAUDE.md", PROMPT_FILE, "core"].map(String::from)
    );
    assert_eq!(a.workspaces.len(), 1);
    assert_eq!(a.workspaces[0].worktree, cwd.join("core").to_string_lossy());
    assert_eq!(a.workspaces[0].branch, format!("fleet/{name}"));
    assert_eq!(
        git(&cwd.join("core"), &["branch", "--show-current"]),
        format!("fleet/{name}")
    );

    let md = std::fs::read_to_string(cwd.join("CLAUDE.md")).unwrap();
    let def = Defs::new(fx.state_dir.join("no-user-defs"))
        .orchestrator("direct")
        .unwrap();
    assert!(md.contains(def.prompt.trim()), "{md}");
    let rust = Defs::new(fx.state_dir.join("no-user-defs"))
        .specialist("rust")
        .unwrap();
    assert!(md.contains(&format!("- rust: {}", rust.summary)), "{md}");
    assert!(md.contains("`core/` (branch"), "{md}");
    assert!(!md.contains("request_repo"), "{md}");
    assert_eq!(
        std::fs::read_to_string(cwd.join(PROMPT_FILE))
            .unwrap()
            .trim(),
        "fix the login bug"
    );
    assert!(skills_in(&cwd).is_empty());

    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.kind, AgentKind::Orchestrator);
    assert_eq!(p.role.as_deref(), Some("direct"));
    assert_eq!(p.roster, ["rust"]);
    assert_eq!(p.test_command.as_deref(), Some("true"));
    assert_eq!(p.workdir, project.to_string_lossy());
    assert_eq!(p.name, name);

    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(
            fx.state_dir
                .join("agents")
                .join(&a.id.0)
                .join("settings.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let deny = settings["permissions"]["deny"].as_array().unwrap();
    for rule in ["Agent", "Task", "Bash(git push:*)", "Bash(gh pr create:*)"] {
        assert!(deny.contains(&json!(rule)), "{rule} not in {deny:?}");
    }
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    for tool in ORCHESTRATOR_TOOLS {
        assert!(allow.contains(&json!(tool)), "{tool} not in {allow:?}");
    }
    // The env backstop for pushes the deny rules cannot see is on the agent's own process.
    assert!(
        pane_contains(
            &fx,
            &a.id,
            "git_config_count=3 git_config_2=remote.origin.pushurl=fleet-push-blocked:///"
        )
        .await
    );

    // The sent roster replaces the repo's; the structured type brings its skills.
    let b = fx
        .svc
        .spawn(request(
            AgentKind::Orchestrator,
            Some("structured"),
            &["core"],
            &["rust", "reviewer"],
        ))
        .await
        .unwrap();
    let cwd = PathBuf::from(&b.workdir);
    assert_eq!(skills_in(&cwd), ["brainstorming", "writing-plans"]);
    assert!(cwd.join(".claude/skills/brainstorming/SKILL.md").is_file());
    let p = fx.store.project(&b.id).await.unwrap().unwrap();
    assert_eq!(p.roster, ["rust", "reviewer"]);
    let md = std::fs::read_to_string(cwd.join("CLAUDE.md")).unwrap();
    assert!(md.contains("- reviewer: "), "{md}");
}

#[tokio::test]
async fn solo_spawn_is_unchanged() {
    let fx = fleet_fixture().await;
    let a = fx
        .svc
        .spawn(request(AgentKind::Solo, None, &["core"], &[]))
        .await
        .unwrap();
    let project = PathBuf::from(&a.workdir);
    assert_eq!(project.parent().unwrap(), fx.projects_root);
    assert!(project.join("CLAUDE.md").is_file());
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.kind, AgentKind::Solo);
    assert!(p.roster.is_empty() && p.test_command.is_none());
    assert!(pane_contains(&fx, &a.id, "git_config_count=2 git_config_2=unset").await);
    let settings = std::fs::read_to_string(
        fx.state_dir
            .join("agents")
            .join(&a.id.0)
            .join("settings.json"),
    )
    .unwrap();
    assert!(!settings.contains("git push"), "{settings}");
}

#[tokio::test]
async fn orchestrator_needs_one_repo_and_test_command() {
    let fx = fleet_fixture().await;
    let other = make_repo(&fx._tmp.path().join("other"));
    let other = other.to_string_lossy().into_owned();
    for repos in [&[][..], &["core", other.as_str()][..]] {
        let err = fx
            .svc
            .spawn(request(AgentKind::Orchestrator, Some("direct"), repos, &[]))
            .await
            .unwrap_err();
        assert!(matches!(err, SpawnError::OneRepo), "{err:?}");
        assert_eq!(
            err.to_string(),
            "an orchestrated project has exactly one repo"
        );
        assert_nothing_created(&fx).await;
    }

    set_fleet(&fx, &["rust"], None).await;
    let err = fx.svc.spawn(orchestrator("direct")).await.unwrap_err();
    assert!(
        matches!(&err, SpawnError::NoTestCommand(r) if r == "core"),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "set a test command for core first (Management → Personas)"
    );
    assert_nothing_created(&fx).await;
}

#[tokio::test]
async fn orchestrator_needs_a_roster() {
    let fx = fleet_fixture().await;
    set_fleet(&fx, &[], Some("true")).await;
    let err = fx.svc.spawn(orchestrator("direct")).await.unwrap_err();
    assert!(matches!(err, SpawnError::NoRoster), "{err:?}");
    assert_eq!(
        err.to_string(),
        "pick at least one specialist for this project"
    );
    assert_nothing_created(&fx).await;
    // A roster sent with the request is enough.
    fx.svc
        .spawn(request(
            AgentKind::Orchestrator,
            Some("direct"),
            &["core"],
            &["rust"],
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn unknown_roster_name_is_refused() {
    let fx = fleet_fixture().await;
    let err = fx
        .svc
        .spawn(request(
            AgentKind::Orchestrator,
            Some("direct"),
            &["core"],
            &["rust", "cobol"],
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, SpawnError::Fleet(FleetError::NotFound { kind: DefKind::Specialist, name }) if name == "cobol"),
        "{err:?}"
    );
    assert_nothing_created(&fx).await;

    // The repo's own roster is checked the same way when none is sent.
    set_fleet(&fx, &["cobol"], Some("true")).await;
    let err = fx.svc.spawn(orchestrator("direct")).await.unwrap_err();
    assert!(matches!(err, SpawnError::Fleet(_)), "{err:?}");
    assert_nothing_created(&fx).await;

    set_fleet(&fx, &["rust"], Some("true")).await;
    let err = fx.svc.spawn(orchestrator("freestyle")).await.unwrap_err();
    assert!(
        matches!(
            &err,
            SpawnError::Fleet(FleetError::NotFound {
                kind: DefKind::Orchestrator,
                ..
            })
        ),
        "{err:?}"
    );
    let err = fx
        .svc
        .spawn(request(AgentKind::Orchestrator, None, &["core"], &[]))
        .await
        .unwrap_err();
    assert!(matches!(err, SpawnError::Fleet(_)), "{err:?}");
    assert_nothing_created(&fx).await;
}

#[tokio::test]
async fn a_specialist_cannot_be_spawned_over_http() {
    let fx = fleet_fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(
        fx.svc.clone(),
        fx.store.clone(),
        fx.tmux.clone(),
        addr.port(),
    );
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    let mut body =
        serde_json::to_value(request(AgentKind::Specialist, Some("rust"), &["core"], &[])).unwrap();
    body["actor"] = json!("local");
    let (status, v) = json_of(addr, "POST", "/api/agents", Some(&body)).await;
    assert_eq!(status, 400, "{v}");
    assert_nothing_created(&fx).await;
}

/// `git` with exactly `vars` (plus `PATH`, to find git itself): none of the test process's own git setup applies.
fn git_with(dir: &Path, vars: &[(String, String)], args: &[&str]) -> std::process::Output {
    Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs(vars.iter().map(|(k, v)| (k, v)))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

#[tokio::test]
async fn a_fleet_agents_env_blocks_push_to_origin() {
    let fx = fixture().await;
    let persona = fixture_env(&fx).await;
    let tmp = fx._tmp.path();
    let bare = tmp.join("origin.git");
    git(tmp, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    let work = make_repo(&tmp.join("work"));
    git(
        &work,
        &[
            "remote",
            "add",
            "origin",
            &format!("file://{}", bare.display()),
        ],
    );
    git(&work, &["branch", "fleet/x"]);
    git(&work, &["branch", "solo/x"]);

    for kind in [AgentKind::Orchestrator, AgentKind::Specialist] {
        let vars = fleet_daemon::fleet::ops::agent_vars(&persona, kind);
        let out = git_with(&work, &vars, &["push", "origin", "fleet/x"]);
        assert!(
            !out.status.success(),
            "{kind:?} pushed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(git(&bare, &["branch", "--list", "fleet/x"]), "");
    }

    // The same push under a solo agent's env goes through, so the block is what stopped it.
    let vars = fleet_daemon::fleet::ops::agent_vars(&persona, AgentKind::Solo);
    assert_eq!(vars, persona.vars);
    let out = git_with(&work, &vars, &["push", "origin", "solo/x"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(git(&bare, &["branch", "--list", "solo/x"]), "solo/x");
}

fn step(title: &str, specialist: &str, depends_on: &[u32]) -> StepDraft {
    StepDraft {
        n: None,
        title: title.into(),
        specialist: specialist.into(),
        brief: format!("build {title}"),
        depends_on: depends_on.to_vec(),
    }
}

fn two_steps() -> Vec<StepDraft> {
    vec![step("api", "rust", &[]), step("ui", "rust", &[1])]
}

fn breakdown_md(a: &fleet_api::Agent) -> PathBuf {
    Path::new(&a.workdir).parent().unwrap().join("BREAKDOWN.md")
}

/// Starts a held `propose_breakdown` in the background and waits until the card shows it.
async fn propose_in_background(
    fx: &Fx,
    svc: &Arc<AgentService>,
    id: &AgentId,
    drafts: Vec<StepDraft>,
) -> tokio::task::JoinHandle<Result<BreakdownAnswer, ServiceError>> {
    let (svc, held) = (svc.clone(), id.clone());
    let task = tokio::spawn(async move { svc.propose_breakdown(&held, drafts).await });
    assert!(
        wait_until(|| async {
            agent(fx, id).await.blocked_reason == Some(BlockedReason::Breakdown)
        })
        .await,
        "the proposal never reached the card"
    );
    task
}

fn approve() -> DecideBreakdown {
    DecideBreakdown {
        approve: true,
        comment: None,
    }
}

#[tokio::test]
async fn propose_then_approve() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    let task = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;

    let card = agent(&fx, &a.id).await;
    assert_eq!(card.status, Status::NeedsYou);
    assert_eq!(card.activity, "breakdown v1 ready");
    let md = std::fs::read_to_string(breakdown_md(&a)).unwrap();
    assert!(md.contains("Version 1, proposed"), "{md}");
    assert!(md.contains("### 2. ui"), "{md}");
    // Nothing Claude Code loads went next to the fleet agents' folders.
    assert!(!Path::new(&a.workdir).join("BREAKDOWN.md").exists());
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns.len(), 1);
    assert_eq!(p.breakdowns[0].approval, Approval::Proposed);

    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answer.unwrap(), BreakdownAnswer::Approved);
    let card = agent(&fx, &a.id).await;
    assert_eq!((card.status, card.blocked_reason), (Status::Working, None));
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].approval, Approval::Approved);
    assert_eq!(p.breakdowns[0].decided_at, Some(fx.clock.now_for_test()));
    let md = std::fs::read_to_string(breakdown_md(&a)).unwrap();
    assert!(md.contains("Version 1, approved"), "{md}");

    let err = fx.svc.decide_breakdown(&a.id, approve()).await.unwrap_err();
    assert!(matches!(err, ServiceError::NoPendingBreakdown), "{err:?}");
}

#[tokio::test]
async fn changes_requested_returns_comment() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    let task = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    // A second proposal while one waits is refused, and leaves the first alone.
    let err = fx
        .svc
        .propose_breakdown(&a.id, two_steps())
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::BreakdownPending), "{err:?}");
    fx.svc
        .decide_breakdown(
            &a.id,
            DecideBreakdown {
                approve: false,
                comment: Some("split\nthe api".into()),
            },
        )
        .await
        .unwrap();
    let answer = task.await.unwrap().unwrap();
    assert_eq!(
        answer,
        BreakdownAnswer::ChangesRequested("split the api".into())
    );
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].approval, Approval::ChangesRequested);
    assert_eq!(p.breakdowns[0].comment.as_deref(), Some("split the api"));
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);

    // The next proposal is version 2 and keeps the numbers.
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1] = step("api part two", "rust", &[1]);
    let task = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    let v2 = &p.breakdowns[1];
    assert_eq!(v2.version, 2);
    assert_eq!(v2.steps.iter().map(|s| s.n).collect::<Vec<_>>(), [1, 3]);
    assert_eq!(agent(&fx, &a.id).await.activity, "breakdown v2 ready");
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(task.await.unwrap().unwrap(), BreakdownAnswer::Approved);
}

#[tokio::test]
async fn a_bad_proposal_changes_nothing() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    let before = agent(&fx, &a.id).await;
    let err = fx
        .svc
        .propose_breakdown(&a.id, vec![step("api", "go", &[])])
        .await
        .unwrap_err();
    assert!(
        matches!(
            &err,
            ServiceError::Breakdown(BreakdownError::NotInRoster { step: 1, .. })
        ),
        "{err:?}"
    );
    let after = agent(&fx, &a.id).await;
    assert_eq!(
        (after.status, after.blocked_reason, after.activity),
        (before.status, before.blocked_reason, before.activity)
    );
    assert!(fx
        .store
        .project(&a.id)
        .await
        .unwrap()
        .unwrap()
        .breakdowns
        .is_empty());
    assert!(!breakdown_md(&a).exists());
    let err = fx.svc.decide_breakdown(&a.id, approve()).await.unwrap_err();
    assert!(matches!(err, ServiceError::NoPendingBreakdown), "{err:?}");

    // Only an orchestrator proposes.
    let solo = fx
        .svc
        .spawn(request(AgentKind::Solo, None, &["core"], &[]))
        .await
        .unwrap();
    let err = fx
        .svc
        .propose_breakdown(&solo.id, two_steps())
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotAnOrchestrator), "{err:?}");
    assert_eq!(agent(&fx, &solo.id).await.blocked_reason, None);
}

#[tokio::test]
async fn decision_after_hold_is_typed() {
    let fx = fleet_fixture().await;
    let mut opts = fx.svc.options().clone();
    opts.repo_request_hold = Duration::from_millis(300);
    let svc = Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        opts,
    ));
    let a = svc.spawn(orchestrator("direct")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let err = svc.propose_breakdown(&a.id, two_steps()).await.unwrap_err();
    assert!(matches!(err, ServiceError::NoAnswerYet), "{err:?}");
    // The proposal outlives the call: it is still Antonio's to decide.
    let card = agent(&fx, &a.id).await;
    assert_eq!(
        (card.status, card.blocked_reason),
        (Status::NeedsYou, Some(BlockedReason::Breakdown))
    );
    svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "fleet-manager: breakdown v1 approved").await);
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);

    let task = tokio::spawn({
        let (svc, id) = (svc.clone(), a.id.clone());
        let mut drafts = two_steps();
        drafts[0].n = Some(1);
        drafts[1].n = Some(2);
        async move { svc.propose_breakdown(&id, drafts).await }
    });
    assert!(task.await.unwrap().is_err(), "the hold runs out");
    svc.decide_breakdown(
        &a.id,
        DecideBreakdown {
            approve: false,
            comment: Some("smaller".into()),
        },
    )
    .await
    .unwrap();
    assert!(
        pane_contains(
            &fx,
            &a.id,
            "fleet-manager: breakdown v2 changes requested: smaller"
        )
        .await
    );
}

#[tokio::test]
async fn stop_ends_a_held_breakdown() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    let task = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    fx.svc.stop(&a.id).await.unwrap();
    let r = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    assert!(r.is_err(), "{r:?}");
    // Still Antonio's to decide; nothing is typed into a stopped session.
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn fleet_view_and_decision_over_http() {
    let fx = fleet_fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(
        fx.svc.clone(),
        fx.store.clone(),
        fx.tmux.clone(),
        addr.port(),
    );
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    let c = FleetClient::new(&format!("http://{addr}"));
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();

    let id = a.id.0.clone();
    let view = tokio::task::block_in_place(|| c.fleet_view(&id)).unwrap();
    assert_eq!(view.project, a.id);
    assert!(view.breakdowns.is_empty() && view.gate_runs.is_empty());
    let err = tokio::task::block_in_place(|| c.decide_breakdown(&id, &approve())).unwrap_err();
    assert_eq!(err.0, ServiceError::NoPendingBreakdown.to_string());
    let (status, _) = json_of(
        addr,
        "POST",
        &format!("/api/projects/{id}/breakdown/decide"),
        Some(&json!({"approve": true})),
    )
    .await;
    assert_eq!(status, 409);

    let task = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    let view = tokio::task::block_in_place(|| c.fleet_view(&id)).unwrap();
    assert_eq!(view.breakdowns.len(), 1);
    assert_eq!(view.breakdowns[0].steps.len(), 2);
    tokio::task::block_in_place(|| c.decide_breakdown(&id, &approve())).unwrap();
    assert_eq!(task.await.unwrap().unwrap(), BreakdownAnswer::Approved);

    let (status, _) = json_of(addr, "GET", "/api/projects/nope/fleet", None).await;
    assert_eq!(status, 404);
}
