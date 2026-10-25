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
