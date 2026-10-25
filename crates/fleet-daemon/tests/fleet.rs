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

/// An orchestrator whose `two_steps` breakdown is approved, with its project folder and name.
async fn approved_project(fx: &Fx) -> (fleet_api::Agent, PathBuf, String) {
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    let task = propose_in_background(fx, &fx.svc, &a.id, two_steps()).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(task.await.unwrap().unwrap(), BreakdownAnswer::Approved);
    let project = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    let name = project.file_name().unwrap().to_string_lossy().into_owned();
    (a, project, name)
}

fn fleet_refusal(err: &ServiceError) -> &str {
    match err {
        ServiceError::Fleet(text) => text,
        other => panic!("not a fleet refusal: {other:?}"),
    }
}

/// Nothing a refused specialist spawn could have left: no fleet folder, card or step branch.
async fn assert_no_specialist(fx: &Fx, project: &Path, name: &str) {
    assert!(!project.join("fleet").exists());
    let agents = fx.store.snapshot().await.unwrap().agents;
    assert!(
        agents.iter().all(|a| a.kind != AgentKind::Specialist),
        "{agents:?}"
    );
    assert_eq!(
        git(&fx.repo, &["branch", "--list", "fleet/*"]),
        format!("+ fleet/{name}")
    );
}

#[tokio::test]
async fn spawn_refused_before_approval() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    let project = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    let name = project.file_name().unwrap().to_string_lossy().into_owned();
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "no breakdown is approved yet");

    let task = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "breakdown v1 is waiting for approval");
    assert_no_specialist(&fx, &project, &name).await;
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert!(p.breakdowns[0]
        .steps
        .iter()
        .all(|s| s.state == fleet_api::StepState::Pending));
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn spawn_refused_while_new_version_pending() {
    let fx = fleet_fixture().await;
    let (a, project, name) = approved_project(&fx).await;
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    let task = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "breakdown v2 is waiting for approval");
    assert_no_specialist(&fx, &project, &name).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn spawn_refused_after_changes_requested() {
    let fx = fleet_fixture().await;
    let (a, project, name) = approved_project(&fx).await;
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    let task = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    fx.svc
        .decide_breakdown(
            &a.id,
            DecideBreakdown {
                approve: false,
                comment: Some("no".into()),
            },
        )
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    // v1 is approved and its step 1 Pending, but v1 is no longer the latest word.
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "breakdown v2 needs a new proposal");
    assert_no_specialist(&fx, &project, &name).await;
}

#[tokio::test]
async fn spawn_respects_dependencies() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let err = fx.svc.spawn_specialist(&a.id, 2).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "step 2 waits for step 1");
    let err = fx.svc.spawn_specialist(&a.id, 9).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "step 9 is not in breakdown v1");
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let err = fx.svc.spawn_specialist(&a.id, 2).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "step 2 waits for step 1");
}

#[tokio::test]
async fn specialist_gets_own_folder_branch_and_skills() {
    let fx = fleet_fixture().await;
    let (a, project, name) = approved_project(&fx).await;
    // The orchestrator has merged something since the project started: the step starts from that.
    let lead_wt = Path::new(&a.workdir).join("core");
    std::fs::write(lead_wt.join("merged.txt"), "x").unwrap();
    git(&lead_wt, &["add", "."]);
    git(&lead_wt, &["commit", "-q", "-m", "merged"]);
    let tip = git(&lead_wt, &["rev-parse", "HEAD"]);

    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let folder = project.join("fleet").join("1-api");
    let branch = format!("fleet/{name}--1-api");
    assert_eq!(s.kind, AgentKind::Specialist);
    assert_eq!(s.project.as_ref(), Some(&a.id));
    assert_eq!(s.role.as_deref(), Some("rust"));
    assert_eq!(s.step, Some(1));
    assert_eq!(s.persona, a.persona);
    assert_eq!(s.workdir, folder.to_string_lossy());
    assert_eq!(s.workspaces.len(), 1);
    assert_eq!(s.workspaces[0].branch, branch);
    assert_eq!(
        s.workspaces[0].from_branch.as_deref(),
        Some(format!("fleet/{name}").as_str())
    );
    let wt = folder.join("core");
    assert_eq!(s.workspaces[0].worktree, wt.to_string_lossy());
    assert_eq!(git(&wt, &["branch", "--show-current"]), branch);
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), tip);
    assert!(
        pane_contains(
            &fx,
            &s.id,
            &format!("cwd={}", folder.canonicalize().unwrap().display())
        )
        .await
    );
    assert!(
        pane_contains(
            &fx,
            &s.id,
            "git_config_count=3 git_config_2=remote.origin.pushurl=fleet-push-blocked:///"
        )
        .await
    );

    assert_eq!(
        entries(&folder),
        [".claude", "CLAUDE.md", PROMPT_FILE, "core"].map(String::from)
    );
    assert_eq!(
        skills_in(&folder),
        [
            "systematic-debugging",
            "test-driven-development",
            "verification-before-completion"
        ]
    );
    let prompt = std::fs::read_to_string(folder.join(PROMPT_FILE)).unwrap();
    assert!(prompt.contains("build api"), "{prompt}");
    assert!(prompt.contains("fix the login bug"), "{prompt}");
    let md = std::fs::read_to_string(folder.join("CLAUDE.md")).unwrap();
    let rust = Defs::new(fx.state_dir.join("no-user-defs"))
        .specialist("rust")
        .unwrap();
    assert!(md.contains(rust.prompt.trim()), "{md}");
    assert!(md.contains("`core/` (branch"), "{md}");
    assert!(!md.contains("request_repo"), "{md}");

    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(
            fx.state_dir
                .join("agents")
                .join(&s.id.0)
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
    assert!(
        allow.contains(&json!("mcp__fleet__finish_step")),
        "{allow:?}"
    );
    assert!(!allow.contains(&json!("mcp__fleet__spawn_specialist")));

    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    let step = &p.breakdowns[0].steps[0];
    assert_eq!(step.state, fleet_api::StepState::Running);
    assert_eq!(step.agent.as_ref(), Some(&s.id));
    assert_eq!(step.branch.as_deref(), Some(branch.as_str()));
    assert!(p.log.iter().any(|l| l.text == "rust started for step 1"));
    let md = std::fs::read_to_string(project.join("BREAKDOWN.md")).unwrap();
    assert!(md.contains(&format!("| running | {} |", s.id.0)), "{md}");

    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "step 1 is running");
    // Only the project's orchestrator starts its specialists.
    let err = fx.svc.spawn_specialist(&s.id, 2).await.unwrap_err();
    assert!(matches!(err, ServiceError::Fleet(_)), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_spawns_start_a_step_once() {
    let fx = fleet_fixture().await;
    let (a, project, _) = approved_project(&fx).await;
    let calls: Vec<_> = (0..2)
        .map(|_| {
            let (svc, id) = (fx.svc.clone(), a.id.clone());
            tokio::spawn(async move { svc.spawn_specialist(&id, 1).await })
        })
        .collect();
    let mut ok = 0;
    for c in calls {
        match c.await.unwrap() {
            Ok(_) => ok += 1,
            Err(e) => assert_eq!(fleet_refusal(&e), "step 1 is running"),
        }
    }
    assert_eq!(ok, 1);
    assert_eq!(entries(&project.join("fleet")), ["1-api"]);
}

#[tokio::test]
async fn a_failed_specialist_spawn_leaves_nothing() {
    let fx = fleet_fixture().await;
    let (a, project, name) = approved_project(&fx).await;
    // A leftover branch with the step's name makes the worktree fail after the folder exists.
    git(&fx.repo, &["branch", &format!("fleet/{name}--1-api")]);
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(entries(&project.join("fleet")), Vec::<String>::new());
    let agents = fx.store.snapshot().await.unwrap().agents;
    assert!(agents.iter().all(|x| x.kind != AgentKind::Specialist));
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(
        p.breakdowns[0].steps[0].state,
        fleet_api::StepState::Pending
    );
    assert_eq!(p.breakdowns[0].steps[0].agent, None);
}

#[tokio::test]
async fn finish_refused_when_dirty() {
    let fx = fleet_fixture().await;
    let (a, project, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    assert!(pane_contains(&fx, &s.id, "ready").await);
    let wt = PathBuf::from(&s.workspaces[0].worktree);
    std::fs::write(wt.join("new.rs"), "fn main() {}").unwrap();
    let err = fx.svc.finish_step(&s.id, "built it").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "commit your changes first");
    assert_eq!(agent(&fx, &s.id).await.status, Status::Queued);

    // Only a specialist finishes a step.
    let err = fx.svc.finish_step(&a.id, "built it").await.unwrap_err();
    assert!(matches!(err, ServiceError::Fleet(_)), "{err:?}");

    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "api"]);
    fx.svc.finish_step(&s.id, "built it").await.unwrap();
    let card = agent(&fx, &s.id).await;
    assert_eq!(card.status, Status::Stopped);
    assert_eq!(card.stop_cause, Some(fleet_api::StopCause::Finished));
    assert_eq!(card.activity, "finished step 1");
    assert!(!fx.tmux.has_session(&s.tmux_session).await.unwrap());
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    let step = &p.breakdowns[0].steps[0];
    assert_eq!(step.state, fleet_api::StepState::Done);
    assert_eq!(step.report.as_deref(), Some("built it"));
    assert_eq!(p.inbox.len(), 1, "{:?}", p.inbox);
    assert_eq!(p.inbox[0].kind, fleet_api::FleetEventKind::Finished);
    assert_eq!(p.inbox[0].step, 1);
    let md = std::fs::read_to_string(project.join("BREAKDOWN.md")).unwrap();
    assert!(md.contains("| done |"), "{md}");
    // The lead is still running.
    assert_ne!(agent(&fx, &a.id).await.status, Status::Stopped);

    let err = fx.svc.finish_step(&s.id, "again").await.unwrap_err();
    assert!(matches!(err, ServiceError::NotRunning), "{err:?}");
    // Step 2 is unblocked now.
    fx.svc.spawn_specialist(&a.id, 2).await.unwrap();
}

#[tokio::test]
async fn no_inheritance() {
    let fx = fleet_fixture().await;
    let (a, project, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let wt = PathBuf::from(&s.workspaces[0].worktree);
    std::fs::write(wt.join("a.rs"), "").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "a"]);
    fx.svc.finish_step(&s.id, "done").await.unwrap();
    fx.svc.spawn_specialist(&a.id, 2).await.unwrap();

    assert_eq!(
        entries(&project),
        ["BREAKDOWN.md", "fleet", "orchestrator"].map(String::from)
    );
    assert_eq!(entries(&project.join("fleet")), ["1-api", "2-ui"]);
    for dir in [project.clone(), project.join("fleet")] {
        assert!(!dir.join("CLAUDE.md").exists(), "{}", dir.display());
        assert!(!dir.join(".claude").exists(), "{}", dir.display());
    }
}

#[tokio::test]
async fn step_starts_from_the_project_branch_not_the_leads_head() {
    let fx = fleet_fixture().await;
    let (a, _, name) = approved_project(&fx).await;
    let lead_wt = Path::new(&a.workdir).join("core");
    let old = git(&lead_wt, &["rev-parse", "HEAD"]);
    std::fs::write(lead_wt.join("merged.txt"), "x").unwrap();
    git(&lead_wt, &["add", "."]);
    git(&lead_wt, &["commit", "-q", "-m", "merged"]);
    let tip = git(&lead_wt, &["rev-parse", "HEAD"]);
    // The orchestrator looks at an older commit; the project branch still holds the merge.
    git(&lead_wt, &["checkout", "-q", "--detach", &old]);

    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let wt = PathBuf::from(&s.workspaces[0].worktree);
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), tip);
    assert_eq!(s.workspaces[0].base, tip);
    assert_eq!(
        s.workspaces[0].from_branch.as_deref(),
        Some(format!("fleet/{name}").as_str())
    );
}

#[tokio::test]
async fn a_launch_failure_after_the_card_is_stored_undoes_everything() {
    let fx = fleet_fixture().await;
    let (a, project, name) = approved_project(&fx).await;
    // The same daemon state behind a tmux whose socket path is too long: the session start fails after the card.
    let broken = Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fleet_daemon::tmux::Tmux::new(Some("x".repeat(300))),
        fx.svc.options().clone(),
    ));
    let before = std::fs::read_dir(fx.state_dir.join("agents"))
        .unwrap()
        .count();
    let err = broken.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert!(matches!(err, ServiceError::Tmux(_)), "{err:?}");

    let agents = fx.store.snapshot().await.unwrap().agents;
    assert!(agents.iter().all(|x| x.kind != AgentKind::Specialist));
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    let step = &p.breakdowns[0].steps[0];
    assert_eq!(step.state, fleet_api::StepState::Pending);
    assert_eq!((step.agent.as_ref(), step.branch.as_ref()), (None, None));
    assert!(p
        .log
        .iter()
        .any(|l| l.text.starts_with("step 1 could not start")));
    let md = std::fs::read_to_string(project.join("BREAKDOWN.md")).unwrap();
    assert!(!md.contains("| running |"), "{md}");
    assert_eq!(entries(&project.join("fleet")), Vec::<String>::new());
    assert_eq!(
        git(&fx.repo, &["branch", "--list", &format!("fleet/{name}--*")]),
        ""
    );
    assert_eq!(
        std::fs::read_dir(fx.state_dir.join("agents"))
            .unwrap()
            .count(),
        before
    );
    // Nothing is left that would stop the step from starting.
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
}

/// Commits something in the specialist's worktree and finishes its step.
async fn finish_with(svc: &AgentService, s: &fleet_api::Agent, report: &str) {
    let wt = PathBuf::from(&s.workspaces[0].worktree);
    std::fs::write(wt.join("f.rs"), report).unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "work"]);
    svc.finish_step(&s.id, report).await.unwrap();
}

#[tokio::test]
async fn wait_returns_queued_events_at_once() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    finish_with(&fx.svc, &s, "built it").await;
    let events = fx.svc.wait_for_fleet(&a.id).await.unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, fleet_api::FleetEventKind::Finished);
    assert_eq!(events[0].step, 1);
    // Taken, not copied.
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert!(p.inbox.is_empty());
}

#[tokio::test]
async fn wait_wakes_on_event() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let (svc, id) = (fx.svc.clone(), a.id.clone());
    let waiter = tokio::spawn(async move { svc.wait_for_fleet(&id).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!waiter.is_finished());
    finish_with(&fx.svc, &s, "built it").await;
    let started = std::time::Instant::now();
    let events = tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("woke within a second")
        .unwrap()
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, fleet_api::FleetEventKind::Finished);
}

#[tokio::test]
async fn wait_times_out_empty() {
    let fx = fleet_fixture().await;
    let mut opts = fx.svc.options().clone();
    opts.repo_request_hold = Duration::from_millis(300);
    let svc = AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        opts,
    );
    let (a, _, _) = approved_project(&fx).await;
    assert!(svc.wait_for_fleet(&a.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn wait_ends_when_the_orchestrator_stops() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let (svc, id) = (fx.svc.clone(), a.id.clone());
    let waiter = tokio::spawn(async move { svc.wait_for_fleet(&id).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    fx.svc.stop(&a.id).await.unwrap();
    let err = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("wait ended")
        .unwrap()
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotRunning), "{err:?}");
}

#[tokio::test]
async fn only_the_orchestrator_follows_the_fleet() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    for who in [&s.id] {
        let err = fx.svc.wait_for_fleet(who).await.unwrap_err();
        assert!(matches!(err, ServiceError::NotAnOrchestrator), "{err:?}");
        let err = fx.svc.fleet_status(who).await.unwrap_err();
        assert!(matches!(err, ServiceError::NotAnOrchestrator), "{err:?}");
    }
}

#[tokio::test]
async fn fleet_status_lists_steps_with_live_status() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let st = fx.svc.fleet_status(&a.id).await.unwrap();
    assert_eq!((st.version, st.approval), (1, Approval::Approved));
    assert_eq!(
        st.steps,
        vec![
            (
                1,
                fleet_api::StepState::Running,
                Some(agent(&fx, &s.id).await.status)
            ),
            (2, fleet_api::StepState::Pending, None),
        ]
    );
}

#[tokio::test]
async fn idle_without_finish_is_reported() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let token = fx.store.agent_token(&s.id).await.unwrap().unwrap();
    fleet_daemon::ingest::ingest(
        &fx.store,
        &s.id,
        Some(&token),
        json!({"hook_event_name": "Stop"}),
        fx.clock.now_for_test(),
    )
    .await
    .unwrap();
    assert_eq!(agent(&fx, &s.id).await.status, Status::Review);
    let events = fx.svc.wait_for_fleet(&a.id).await.unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, fleet_api::FleetEventKind::IdleWithoutFinish);
    assert_eq!(events[0].step, 1);
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(
        p.breakdowns[0].steps[0].state,
        fleet_api::StepState::Running
    );
}

#[tokio::test]
async fn inbox_survives_daemon_restart() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    finish_with(&fx.svc, &s, "built it").await;
    let rebuilt = AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        fx.svc.options().clone(),
    );
    let events = rebuilt.wait_for_fleet(&a.id).await.unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, fleet_api::FleetEventKind::Finished);
}

#[tokio::test]
async fn a_dropped_wait_leaves_the_events_queued() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    finish_with(&fx.svc, &s, "built it").await;
    // Dropped at the first point it has to wait on the store; where exactly is up to the scheduler, but the ack is
    // the last await, so nothing short of completion may consume the events.
    let first = tokio::time::timeout(Duration::ZERO, fx.svc.wait_for_fleet_lines(&a.id)).await;
    let lines = match first {
        Ok(done) => done.unwrap(),
        Err(_) => fx.svc.wait_for_fleet_lines(&a.id).await.unwrap(),
    };
    assert_eq!(lines, ["step 1 (rust): finished: built it"]);
}

#[tokio::test]
async fn event_lines_use_the_mcp_formats() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    finish_with(&fx.svc, &s, "built it\nwith detail").await;
    let s2 = fx.svc.spawn_specialist(&a.id, 2).await.unwrap();
    let token = fx.store.agent_token(&s2.id).await.unwrap().unwrap();
    fleet_daemon::ingest::ingest(
        &fx.store,
        &s2.id,
        Some(&token),
        json!({"hook_event_name": "Stop"}),
        fx.clock.now_for_test(),
    )
    .await
    .unwrap();
    let lines = fx.svc.wait_for_fleet_lines(&a.id).await.unwrap();
    assert_eq!(
        lines,
        [
            "step 1 (rust): finished: built it",
            "step 2: ended its turn without finish_step",
        ]
    );
}

/// An orchestrated project whose two steps are done, with `core`'s origin a bare repo so `open_pr` really pushes.
/// Returns the lead, its worktree, the project branch and the bare origin.
async fn done_project(fx: &Fx) -> (fleet_api::Agent, PathBuf, String, PathBuf) {
    let (a, _, _) = approved_project(fx).await;
    for n in [1, 2] {
        let s = fx.svc.spawn_specialist(&a.id, n).await.unwrap();
        finish_with(&fx.svc, &s, &format!("step {n}")).await;
    }
    // Only now: a `file://` origin is outside the persona's host rule, which a spawn checks.
    let tmp = fx._tmp.path();
    let bare = tmp.join("origin.git");
    git(tmp, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    git(
        &fx.repo,
        &[
            "remote",
            "add",
            "origin",
            &format!("file://{}", bare.display()),
        ],
    );
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    let branch = a.workspaces[0].branch.clone();
    (a, wt, branch, bare)
}

/// The fixture persona's gh config dir, where the fake gh logs its calls.
async fn gh_dir(fx: &Fx) -> PathBuf {
    let env = fixture_env(fx).await;
    let dir = &env
        .vars
        .iter()
        .find(|(k, _)| k == "GH_CONFIG_DIR")
        .unwrap()
        .1;
    PathBuf::from(dir)
}

/// Every call the fake gh got under the fixture's persona, one per line.
async fn gh_calls(fx: &Fx) -> String {
    std::fs::read_to_string(gh_dir(fx).await.join("argv.log")).unwrap_or_default()
}

fn gate_failure(err: &ServiceError) -> &fleet_api::GateRun {
    match err {
        ServiceError::GateFailed(run) => run,
        other => panic!("not a gate failure: {other:?}"),
    }
}

#[tokio::test]
async fn gate_failure_pushes_nothing() {
    let fx = fixture().await;
    set_fleet(&fx, &["rust"], Some("echo boom; exit 3")).await;
    let (a, _, branch, bare) = done_project(&fx).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    let run = gate_failure(&err);
    assert_eq!(run.exit_code, Some(3));
    assert!(run.output_tail.contains("boom"), "{}", run.output_tail);
    assert_eq!(run.command, "echo boom; exit 3");
    assert_eq!(run.pr_url, None);
    assert_eq!(git(&bare, &["branch", "--list", &branch]), "");
    assert!(!gh_calls(&fx).await.contains("pr "));
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.gate_runs, vec![run.clone()]);
    assert_ne!(agent(&fx, &a.id).await.status, Status::Review);
}

#[tokio::test]
async fn gate_timeout() {
    let fx = fixture_config(|c| c.test_timeout_secs = 1).await;
    let pid_file = fx._tmp.path().join("sleeper.pid");
    // A grandchild of the shell: only killing the whole group ends it.
    let cmd = format!("sleep 30 & echo $! > {}; wait", pid_file.display());
    set_fleet(&fx, &["rust"], Some(&cmd)).await;
    let (a, _, branch, bare) = done_project(&fx).await;
    let started = std::time::Instant::now();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(gate_failure(&err).exit_code, None);
    assert_eq!(git(&bare, &["branch", "--list", &branch]), "");
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let alive = || {
        Command::new("kill")
            .args(["-0", pid.trim()])
            .output()
            .unwrap()
            .status
            .success()
    };
    assert!(
        wait_until(|| async { !alive() }).await,
        "the sleeper outlived the gate"
    );
}

#[tokio::test]
async fn output_is_tailed() {
    let fx = fixture().await;
    set_fleet(&fx, &["rust"], Some("seq 1 1000; exit 1")).await;
    let (a, _, _, _) = done_project(&fx).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    let lines: Vec<&str> = gate_failure(&err).output_tail.lines().collect();
    assert_eq!(lines.len(), 200);
    assert_eq!(lines.first(), Some(&"801"));
    assert_eq!(lines.last(), Some(&"1000"));
}

#[tokio::test]
async fn dirty_worktree_refused() {
    let fx = fixture().await;
    let marker = fx._tmp.path().join("ran");
    set_fleet(&fx, &["rust"], Some(&format!("touch {}", marker.display()))).await;
    let (a, wt, branch, _) = done_project(&fx).await;

    std::fs::write(wt.join("loose.rs"), "").unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "commit or abort the merge first");
    std::fs::remove_file(wt.join("loose.rs")).unwrap();

    // A merge whose conflicts are resolved but not committed leaves a clean status and a MERGE_HEAD.
    let git_dir = PathBuf::from(git(&wt, &["rev-parse", "--absolute-git-dir"]));
    let head = git(&wt, &["rev-parse", "HEAD"]);
    std::fs::write(git_dir.join("MERGE_HEAD"), format!("{head}\n")).unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "commit or abort the merge first");
    std::fs::remove_file(git_dir.join("MERGE_HEAD")).unwrap();

    // The tests run on the project branch, not on whatever the lead has checked out.
    git(&wt, &["checkout", "-q", "--detach"]);
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), format!("check out {branch} first"));

    assert!(!marker.exists());
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert!(p.gate_runs.is_empty());
}

#[tokio::test]
async fn open_steps_refused() {
    let fx = fixture().await;
    let marker = fx._tmp.path().join("ran");
    set_fleet(&fx, &["rust"], Some(&format!("touch {}", marker.display()))).await;
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "no breakdown is approved yet");

    let task = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "breakdown v1 is waiting for approval");
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    task.await.unwrap().unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "step 1 is pending");
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "step 1 is running");

    // Only the orchestrator opens the PR.
    let err = fx.svc.open_pr(&s.id, "t", "b").await.unwrap_err();
    assert!(matches!(err, ServiceError::NotAnOrchestrator), "{err:?}");
    assert!(!marker.exists());
}

#[tokio::test]
async fn gate_pass_pushes_and_opens_pr() {
    let fx = fleet_fixture().await;
    let (a, _, branch, bare) = done_project(&fx).await;
    let url = fx
        .svc
        .open_pr(&a.id, "Add the api", "both steps")
        .await
        .unwrap();
    assert_eq!(url, "https://example.test/pr/1");
    assert_eq!(git(&bare, &["branch", "--list", &branch]), branch);
    let calls = gh_calls(&fx).await;
    assert!(
        calls.contains(&format!(
            "pr create --head {branch} --base main --title Add the api --body both steps"
        )),
        "{calls}"
    );
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert!(
        p.log.iter().any(|l| l.text == format!("PR opened: {url}")),
        "{:?}",
        p.log
    );
    assert_eq!(p.gate_runs.len(), 1);
    assert_eq!(p.gate_runs[0].exit_code, Some(0));
    assert_eq!(p.gate_runs[0].pr_url.as_deref(), Some(url.as_str()));
    let lead = agent(&fx, &a.id).await;
    assert_eq!(
        (lead.status, lead.activity.as_str()),
        (Status::Review, "PR open")
    );
}

#[tokio::test]
async fn push_failure_keeps_the_run() {
    let fx = fleet_fixture().await;
    let (a, _, _, bare) = done_project(&fx).await;
    std::fs::remove_dir_all(&bare).unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    let text = fleet_refusal(&err);
    assert!(text.starts_with("push failed: "), "{text}");
    assert!(!gh_calls(&fx).await.contains("pr "));
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.gate_runs.len(), 1);
    assert_eq!(p.gate_runs[0].exit_code, Some(0));
    assert_eq!(p.gate_runs[0].pr_url, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_breakdown_holds_still_while_the_gate_runs() {
    let fx = fixture().await;
    let marker = fx._tmp.path().join("started");
    let cmd = format!("touch {}; sleep 2; exit 1", marker.display());
    set_fleet(&fx, &["rust"], Some(&cmd)).await;
    let (a, _, _, _) = done_project(&fx).await;
    let (svc, id) = (fx.svc.clone(), a.id.clone());
    let gate = tokio::spawn(async move { svc.open_pr(&id, "t", "b").await });
    assert!(wait_until(|| async { marker.exists() }).await);

    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "open_pr is already running");
    let err = fx
        .svc
        .propose_breakdown(&a.id, vec![step("fix", "rust", &[])])
        .await
        .unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        "the tests before the PR are running; wait for open_pr"
    );

    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        "the tests before the PR are running; wait for open_pr"
    );

    let err = gate.await.unwrap().unwrap_err();
    assert_eq!(gate_failure(&err).exit_code, Some(1));
    // Over: a fix step can be proposed.
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    drafts.push(step("fix", "rust", &[]));
    let task = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    task.abort();
}

#[tokio::test]
async fn a_cancelled_open_pr_ends_the_tests() {
    let fx = fixture().await;
    let pid_file = fx._tmp.path().join("sleeper.pid");
    let cmd = format!("sleep 30 & echo $! > {}; wait", pid_file.display());
    set_fleet(&fx, &["rust"], Some(&cmd)).await;
    let (a, _, _, _) = done_project(&fx).await;
    let (svc, id) = (fx.svc.clone(), a.id.clone());
    let call = tokio::spawn(async move { svc.open_pr(&id, "t", "b").await });
    assert!(
        wait_until(|| async {
            std::fs::read_to_string(&pid_file).is_ok_and(|p| !p.trim().is_empty())
        })
        .await
    );
    call.abort();
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let alive = || {
        Command::new("kill")
            .args(["-0", pid.trim()])
            .output()
            .unwrap()
            .status
            .success()
    };
    assert!(
        wait_until(|| async { !alive() }).await,
        "the sleeper outlived the cancelled call"
    );
}

#[tokio::test]
async fn a_branch_that_moves_during_the_tests_is_not_pushed() {
    let fx = fixture().await;
    let cmd = "git -c commit.gpgsign=false commit -q --allow-empty -m sneaked";
    set_fleet(&fx, &["rust"], Some(cmd)).await;
    let (a, _, branch, bare) = done_project(&fx).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        "the branch moved while the tests ran; call open_pr again"
    );
    assert_eq!(git(&bare, &["branch", "--list", &branch]), "");
    assert!(!gh_calls(&fx).await.contains("pr "));
}

#[tokio::test]
async fn the_tests_cannot_push() {
    let fx = fixture().await;
    let cmd = "git push -q origin HEAD:refs/heads/sneaky; true";
    set_fleet(&fx, &["rust"], Some(cmd)).await;
    let (a, wt, branch, bare) = done_project(&fx).await;
    let tested = git(&wt, &["rev-parse", "HEAD"]);
    fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    assert_eq!(git(&bare, &["branch", "--list", "sneaky"]), "");
    // The pushed commit is the tested one, and the branch follows origin as `push -u` would leave it.
    assert_eq!(git(&bare, &["rev-parse", &branch]), tested);
    assert_eq!(
        git(&wt, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        format!("origin/{branch}")
    );
}

#[tokio::test]
async fn gh_without_a_url_is_a_failure() {
    let fx = fleet_fixture().await;
    let (a, _, _, _) = done_project(&fx).await;
    std::fs::write(gh_dir(&fx).await.join("pr-empty"), "").unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        "push failed: the branch is pushed, but gh pr create printed no URL"
    );
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.gate_runs[0].pr_url, None);
    assert_ne!(agent(&fx, &a.id).await.status, Status::Review);
}

#[tokio::test]
async fn files_the_tests_leave_do_not_block_the_pr() {
    let fx = fixture().await;
    set_fleet(&fx, &["rust"], Some("echo built > build.log")).await;
    let (a, wt, branch, bare) = done_project(&fx).await;
    let tested = git(&wt, &["rev-parse", "HEAD"]);
    let url = fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    assert_eq!(url, "https://example.test/pr/1");
    assert!(wt.join("build.log").exists());
    assert_eq!(git(&bare, &["rev-parse", &branch]), tested);
}

/// An orchestrator whose breakdown of two independent steps is approved, so both can run at once.
async fn approved_parallel(fx: &Fx) -> fleet_api::Agent {
    let a = fx.svc.spawn(orchestrator("direct")).await.unwrap();
    let drafts = vec![step("api", "rust", &[]), step("ui", "rust", &[])];
    let task = propose_in_background(fx, &fx.svc, &a.id, drafts).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(task.await.unwrap().unwrap(), BreakdownAnswer::Approved);
    a
}

#[tokio::test]
async fn stop_project_stops_everything() {
    let fx = fleet_fixture().await;
    let a = approved_parallel(&fx).await;
    let s1 = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let s2 = fx.svc.spawn_specialist(&a.id, 2).await.unwrap();

    assert_eq!(fx.svc.stop_project(&a.id).await.unwrap(), 3);
    for id in [&a.id, &s1.id, &s2.id] {
        let x = agent(&fx, id).await;
        assert_eq!(x.status, Status::Stopped, "{x:?}");
        assert!(!fx.tmux.has_session(&x.tmux_session).await.unwrap());
    }
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, fleet_api::ProjectState::Stopped);
    assert_eq!(p.stop_cause, Some(fleet_api::StopCause::Stopped));
    // Idempotent: nothing is left to stop.
    assert_eq!(fx.svc.stop_project(&a.id).await.unwrap(), 0);
    assert!(matches!(
        fx.svc.stop_project(&AgentId("nope".into())).await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn stopping_specialist_fails_step_only() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    fx.svc.stop(&s.id).await.unwrap();

    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, fleet_api::ProjectState::Running);
    assert_eq!(p.breakdowns[0].steps[0].state, fleet_api::StepState::Failed);
    // The orchestrator's card keeps its own screen; the specialist's stop is logged on the project.
    assert!(p.last_screen.is_none());
    assert!(
        p.log
            .iter()
            .any(|l| l.text == "step 1 (rust) stopped by you"),
        "{:?}",
        p.log
    );
    let events = fx.svc.wait_for_fleet(&a.id).await.unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, fleet_api::FleetEventKind::Failed);
    assert_eq!(events[0].step, 1);
}

/// Kills the agent's session behind the daemon's back and lets reconcile find it, as a crash would.
async fn crash(fx: &Fx, id: &AgentId) {
    fx.tmux.kill(&session_name(id)).await.unwrap();
    fx.svc.reconcile().await.unwrap();
    let x = agent(fx, id).await;
    assert_eq!(
        (x.status, x.stop_cause),
        (Status::Stopped, Some(fleet_api::StopCause::Crashed))
    );
}

#[tokio::test]
async fn restart_keeps_fleet_linked() {
    let fx = fleet_fixture().await;
    let (a, project, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    crash(&fx, &a.id).await;
    assert_ne!(agent(&fx, &s.id).await.status, Status::Stopped);

    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    assert_ne!(b.id, a.id);
    assert_eq!(b.kind, AgentKind::Orchestrator);
    assert_eq!(b.project.as_ref(), Some(&a.id));
    assert_eq!(b.role.as_deref(), Some("direct"));
    // The same folder and worktree, not a new one.
    assert_eq!(b.workdir, a.workdir);
    assert_eq!(b.workspaces[0].worktree, a.workspaces[0].worktree);
    assert_eq!(b.workspaces[0].branch, a.workspaces[0].branch);
    assert_eq!(entries(&project), ["BREAKDOWN.md", "fleet", "orchestrator"]);
    assert!(
        pane_contains(
            &fx,
            &b.id,
            &format!(
                "cwd={}",
                Path::new(&a.workdir).canonicalize().unwrap().display()
            )
        )
        .await
    );
    // Settings and MCP config are the new agent's, with its own token.
    let settings = fx.state_dir.join("agents").join(&b.id.0);
    assert!(settings.join("settings.json").is_file());
    let mcp = std::fs::read_to_string(settings.join("mcp.json")).unwrap();
    assert!(mcp.contains(&b.id.0), "{mcp}");
    let token = fx.store.agent_token(&b.id).await.unwrap().unwrap();
    assert!(pane_contains(&fx, &b.id, &format!("token={token}")).await);
    assert!(Path::new(&b.workdir).join("CLAUDE.md").is_file());

    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.lead.as_ref(), Some(&b.id));
    assert_eq!(p.agents_used, 3);
    assert!(p
        .log
        .iter()
        .any(|l| l.text.starts_with("orchestrator restarted")));
    let prompt = std::fs::read_to_string(Path::new(&b.workdir).join(PROMPT_FILE)).unwrap();
    assert!(prompt.contains("fix the login bug"), "{prompt}");
    assert!(prompt.contains("taking over"), "{prompt}");
    assert!(prompt.contains("| # | Step |"), "{prompt}");
    assert!(prompt.contains("| running |"), "{prompt}");

    // The old session's calls are refused; the new one follows the fleet.
    assert!(fx.svc.wait_for_fleet(&a.id).await.is_err());
    finish_with(&fx.svc, &s, "built it").await;
    let events = fx.svc.wait_for_fleet(&b.id).await.unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, fleet_api::FleetEventKind::Finished);
    // And it may start the next step.
    let s2 = fx.svc.spawn_specialist(&b.id, 2).await.unwrap();
    assert_eq!(s2.project.as_ref(), Some(&a.id));
}

#[tokio::test]
async fn restart_prompt_carries_reports_and_the_inbox() {
    let fx = fleet_fixture().await;
    let a = approved_parallel(&fx).await;
    let s1 = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    finish_with(&fx.svc, &s1, "api is built").await;
    fx.svc.wait_for_fleet(&a.id).await.unwrap();
    let s2 = fx.svc.spawn_specialist(&a.id, 2).await.unwrap();
    crash(&fx, &a.id).await;
    // Happens while no orchestrator runs.
    fx.svc.stop(&s2.id).await.unwrap();

    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    let prompt = std::fs::read_to_string(Path::new(&b.workdir).join(PROMPT_FILE)).unwrap();
    assert!(prompt.contains("api is built"), "{prompt}");
    assert!(
        prompt.contains("step 2 (rust): stopped without finish_step"),
        "{prompt}"
    );
    // Not drained: the next wait gets it too.
    let lines = fx.svc.wait_for_fleet_lines(&b.id).await.unwrap();
    assert_eq!(
        lines,
        ["step 2 (rust): stopped without finish_step (stopped)"]
    );
}

#[tokio::test]
async fn restart_refused_while_running() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let err = fx.svc.restart_orchestrator(&a.id).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "the orchestrator is still running");
    assert_eq!(fx.store.project_agents(&a.id).await.unwrap().len(), 1);

    let solo = fx
        .svc
        .spawn(request(AgentKind::Solo, None, &["core"], &[]))
        .await
        .unwrap();
    fx.svc.stop(&solo.id).await.unwrap();
    let err = fx.svc.restart_orchestrator(&solo.id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Fleet(_)), "{err:?}");
    assert_eq!(fx.store.project_agents(&solo.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn restart_into_a_stopped_project_runs_it_again() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_project(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    assert_eq!(fx.svc.stop_project(&a.id).await.unwrap(), 2);
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, fleet_api::ProjectState::Stopped);
    assert!(p.stopped_at.is_some());

    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, fleet_api::ProjectState::Running);
    assert_eq!((p.stopped_at, p.stop_cause), (None, None));
    assert_eq!(p.lead.as_ref(), Some(&b.id));
    assert_eq!(
        p.log.last().unwrap().text,
        "orchestrator restarted; project running again"
    );
    // The step the stop failed is in the prompt, for the new orchestrator to deal with.
    let prompt = std::fs::read_to_string(Path::new(&b.workdir).join(PROMPT_FILE)).unwrap();
    assert!(prompt.contains("| failed |"), "{prompt}");
    assert_eq!(agent(&fx, &s.id).await.status, Status::Stopped);
    // Stopping it again ends the new orchestrator only.
    assert_eq!(fx.svc.stop_project(&a.id).await.unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_and_restart_over_http() {
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
    let (a, _, _) = approved_project(&fx).await;
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let id = a.id.0.clone();

    let (status, body) = json_of(addr, "POST", &format!("/api/projects/{id}/restart"), None).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        tokio::task::block_in_place(|| c.stop_project(&id)).unwrap(),
        2
    );
    let (status, body) = json_of(addr, "POST", &format!("/api/projects/{id}/stop"), None).await;
    assert_eq!((status, body), (200, json!({ "stopped": 0 })));

    let b = tokio::task::block_in_place(|| c.restart_orchestrator(&id)).unwrap();
    assert_eq!(b.project.as_ref(), Some(&a.id));
    let p = fx.store.project(&a.id).await.unwrap().unwrap();
    assert_eq!(p.lead.as_ref(), Some(&b.id));
    let (status, _) = json_of(addr, "POST", "/api/projects/nope/stop", None).await;
    assert_eq!(status, 404);
}
