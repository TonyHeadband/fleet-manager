mod common;

use common::net::*;
use common::*;
use fleet_api::{
    Actor, AgentId, AgentKind, Approval, BlockedReason, DecideBreakdown, DefKind, ProductOverride,
    SettingSource, SpawnRequest, Status, StepDraft,
};
use fleet_client::http::FleetClient;
use fleet_daemon::agents::{AgentService, RequestGate, ServiceError, SpawnError, PROMPT_FILE};
use fleet_daemon::fleet::breakdown::{BreakdownAnswer, BreakdownError};
use fleet_daemon::fleet::defs::{Defs, FleetError};
use fleet_daemon::fleet::workflow::WorkflowError;
use fleet_daemon::hooks_cfg::ORCHESTRATOR_TOOLS;
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

fn request(
    fx: &Fx,
    kind: AgentKind,
    role: Option<&str>,
    repos: &[&str],
    roster: &[&str],
) -> SpawnRequest {
    SpawnRequest {
        actor: Actor("local".into()),
        product: PRODUCT.into(),
        release: None,
        repos: repos.iter().map(|r| pick(fx, r)).collect(),
        prompt: "fix the login bug".into(),
        slug: None,
        persona: PERSONA.into(),
        kind,
        role: role.map(String::from),
        roster: roster.iter().map(|r| r.to_string()).collect(),
    }
}

fn orchestrator(fx: &Fx, role: &str) -> SpawnRequest {
    request(fx, AgentKind::Orchestrator, Some(role), &["core"], &[])
}

async fn set_fleet(fx: &Fx, roster: &[&str]) {
    let roster: Vec<String> = roster.iter().map(|r| r.to_string()).collect();
    fx.store
        .set_repo_roster(PRODUCT, "core", &roster)
        .await
        .unwrap();
}

/// `fx` made ready for an orchestrated task: a roster, and an origin whose `main` has a workflow.
async fn ready_for_fleet(fx: &Fx) {
    add_origin_with_workflow(fx, &one_step("true"));
    set_fleet(fx, &["rust"]).await;
}

/// A fixture whose `core` repo is ready for an orchestrated task.
async fn fleet_fixture() -> Fx {
    let fx = fixture().await;
    ready_for_fleet(&fx).await;
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
    assert!(entries(&fx.tasks_root).is_empty());
    assert!(fx.store.snapshot().await.unwrap().agents.is_empty());
    assert!(fx.store.tasks().await.unwrap().is_empty());
    assert_eq!(git(&fx.repo, &["branch", "--list", "fleet/*"]), "");
}

#[tokio::test]
async fn orchestrator_spawn_layout() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    assert_eq!(a.kind, AgentKind::Orchestrator);
    assert_eq!(a.role.as_deref(), Some("direct"));

    let cwd = PathBuf::from(&a.workdir);
    let task = cwd.parent().unwrap().to_path_buf();
    assert_eq!(cwd.file_name().unwrap(), "orchestrator");
    assert_eq!(task.parent().unwrap(), fx.tasks_root);
    let name = task.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        pane_contains(
            &fx,
            &a.id,
            &format!("cwd={}", cwd.canonicalize().unwrap().display())
        )
        .await
    );

    // The task folder and its parents hold nothing Claude Code would load for every agent under it.
    assert_eq!(entries(&task), ["internal", "orchestrator"]);
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

    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.kind, AgentKind::Orchestrator);
    assert_eq!(p.role.as_deref(), Some("direct"));
    assert_eq!(p.roster, ["rust"]);
    assert_eq!(p.workdir, task.to_string_lossy());
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
            &fx,
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
    let p = fx.store.task(&b.id).await.unwrap().unwrap();
    assert_eq!(p.roster, ["rust", "reviewer"]);
    let md = std::fs::read_to_string(cwd.join("CLAUDE.md")).unwrap();
    assert!(md.contains("- reviewer: "), "{md}");
}

#[tokio::test]
async fn solo_spawn_is_unchanged() {
    let fx = fleet_fixture().await;
    let a = fx
        .svc
        .spawn(request(&fx, AgentKind::Solo, None, &["core"], &[]))
        .await
        .unwrap();
    let task = PathBuf::from(&a.workdir);
    assert_eq!(task.parent().unwrap(), fx.tasks_root);
    assert!(task.join("CLAUDE.md").is_file());
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.kind, AgentKind::Solo);
    assert!(p.roster.is_empty());
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
async fn orchestrator_needs_one_repo() {
    let fx = fleet_fixture().await;
    let other = make_repo(&fx._tmp.path().join("other"));
    let other = other.to_string_lossy().into_owned();
    for repos in [&[][..], &["core", other.as_str()][..]] {
        let err = fx
            .svc
            .spawn(request(
                &fx,
                AgentKind::Orchestrator,
                Some("direct"),
                repos,
                &[],
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, SpawnError::OneRepo), "{err:?}");
        assert_eq!(err.to_string(), "an orchestrated task has exactly one repo");
        assert_nothing_created(&fx).await;
    }
}

/// Nothing a refused spawn could have left, `assert_nothing_created`, plus no tmux session.
async fn assert_nothing_started(fx: &Fx) {
    assert_nothing_created(fx).await;
    assert!(fx.tmux.list_sessions("").await.unwrap().is_empty());
}

#[tokio::test]
async fn orchestrator_needs_a_workflow() {
    let fx = fixture().await;
    add_origin(&fx);
    set_fleet(&fx, &["rust"]).await;
    let err = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap_err();
    assert!(
        matches!(&err, SpawnError::Workflow { repo, error: WorkflowError::Missing { base } } if repo == "core" && base == "main"),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "core has no .fleet/workflow.yml on main. It lists the checks the gate runs before a pull request, so \
         orchestrated tasks need it."
    );
    assert_nothing_started(&fx).await;

    // An invalid file is refused the same way, with the parse error.
    push_workflow(&fx, "steps: {}\n");
    let err = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "core: .fleet/workflow.yml on main is invalid: `steps` is empty"
    );
    assert_nothing_started(&fx).await;
}

#[tokio::test]
async fn a_typed_worktree_is_checked_on_its_own_branch() {
    let fx = fleet_fixture().await;
    // `feature` starts before the workflow commit, so its origin branch has no file while `origin/main` has one.
    let wt = fx._tmp.path().join("feature-wt");
    git(
        &fx.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
            "main~1",
        ],
    );
    git(&fx.repo, &["push", "-q", "origin", "feature"]);
    let typed = wt.to_string_lossy().into_owned();
    let err = fx
        .svc
        .spawn(request(
            &fx,
            AgentKind::Orchestrator,
            Some("direct"),
            &[typed.as_str()],
            &[],
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, SpawnError::Workflow { repo, error: WorkflowError::Missing { base } } if repo == "core" && base == "feature"),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "core has no .fleet/workflow.yml on feature. It lists the checks the gate runs before a pull request, so \
         orchestrated tasks need it."
    );
    assert_nothing_started(&fx).await;
}

#[tokio::test]
async fn orchestrator_creation_fetches_first() {
    let fx = fixture().await;
    let bare = add_origin(&fx);
    set_fleet(&fx, &["rust"]).await;
    // Resolved once, and `origin/main` fetched without the file.
    let err = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap_err();
    assert!(matches!(err, SpawnError::Workflow { .. }), "{err:?}");

    // Pushed from elsewhere: only a fetch at creation sees it.
    let other = fx._tmp.path().join("elsewhere");
    git(
        fx._tmp.path(),
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    std::fs::create_dir_all(other.join(".fleet")).unwrap();
    std::fs::write(other.join(".fleet/workflow.yml"), one_step("true")).unwrap();
    git(&other, &["add", "."]);
    git(
        &other,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "workflow",
        ],
    );
    git(&other, &["push", "-q", "origin", "main"]);

    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    assert_eq!(a.kind, AgentKind::Orchestrator);
}

#[tokio::test]
async fn orchestrator_refused_when_origin_unreachable() {
    let fx = fixture().await;
    git(
        &fx.repo,
        &["remote", "add", "origin", "file:///nonexistent"],
    );
    set_fleet(&fx, &["rust"]).await;
    let err = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap_err();
    assert!(
        matches!(&err, SpawnError::Workflow { repo, error: WorkflowError::Fetch(_) } if repo == "core"),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("core: cannot fetch the base branch: "),
        "{err}"
    );
    assert_nothing_started(&fx).await;
}

#[tokio::test]
async fn solo_needs_no_workflow() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(request(&fx, AgentKind::Solo, None, &["core"], &[]))
        .await
        .unwrap();
    assert_eq!(a.kind, AgentKind::Solo);
}

#[tokio::test]
async fn orchestrator_needs_a_roster() {
    let fx = fleet_fixture().await;
    set_fleet(&fx, &[]).await;
    let err = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap_err();
    assert!(matches!(err, SpawnError::NoRoster), "{err:?}");
    assert_eq!(
        err.to_string(),
        "pick at least one specialist for this task"
    );
    assert_nothing_created(&fx).await;
    // A roster sent with the request is enough.
    fx.svc
        .spawn(request(
            &fx,
            AgentKind::Orchestrator,
            Some("direct"),
            &["core"],
            &["rust"],
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn roster_names_are_deduped_in_order() {
    let fx = fleet_fixture().await;
    let a = fx
        .svc
        .spawn(request(
            &fx,
            AgentKind::Orchestrator,
            Some("direct"),
            &["core"],
            &["reviewer", "rust", "reviewer"],
        ))
        .await
        .unwrap();
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.roster, ["reviewer", "rust"]);
    let md = std::fs::read_to_string(PathBuf::from(&a.workdir).join("CLAUDE.md")).unwrap();
    assert_eq!(md.matches("- reviewer: ").count(), 1, "{md}");
}

#[tokio::test]
async fn unknown_roster_name_is_refused() {
    let fx = fleet_fixture().await;
    let err = fx
        .svc
        .spawn(request(
            &fx,
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
    set_fleet(&fx, &["cobol"]).await;
    let err = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap_err();
    assert!(matches!(err, SpawnError::Fleet(_)), "{err:?}");
    assert_nothing_created(&fx).await;

    set_fleet(&fx, &["rust"]).await;
    let err = fx
        .svc
        .spawn(orchestrator(&fx, "freestyle"))
        .await
        .unwrap_err();
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
        .spawn(request(&fx, AgentKind::Orchestrator, None, &["core"], &[]))
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
    let mut body = serde_json::to_value(request(
        &fx,
        AgentKind::Specialist,
        Some("rust"),
        &["core"],
        &[],
    ))
    .unwrap();
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
    let handle = tokio::spawn(async move { svc.propose_breakdown(&held, drafts).await });
    assert!(
        wait_until(|| async {
            agent(fx, id).await.blocked_reason == Some(BlockedReason::Breakdown)
        })
        .await,
        "the proposal never reached the card"
    );
    handle
}

fn approve() -> DecideBreakdown {
    DecideBreakdown {
        approve: true,
        comment: None,
        version: None,
    }
}

#[tokio::test]
async fn propose_then_approve() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let handle = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;

    let card = agent(&fx, &a.id).await;
    assert_eq!(card.status, Status::NeedsYou);
    assert_eq!(card.activity, "breakdown v1 ready");
    let md = std::fs::read_to_string(breakdown_md(&a)).unwrap();
    assert!(md.contains("Version 1, proposed"), "{md}");
    assert!(md.contains("### 2. ui"), "{md}");
    // Nothing Claude Code loads went next to the fleet agents' folders.
    assert!(!Path::new(&a.workdir).join("BREAKDOWN.md").exists());
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns.len(), 1);
    assert_eq!(p.breakdowns[0].approval, Approval::Proposed);

    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answer.unwrap(), BreakdownAnswer::Approved);
    let card = agent(&fx, &a.id).await;
    assert_eq!((card.status, card.blocked_reason), (Status::Working, None));
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].approval, Approval::Approved);
    assert_eq!(p.breakdowns[0].decided_at, Some(fx.clock.now_for_test()));
    let md = std::fs::read_to_string(breakdown_md(&a)).unwrap();
    assert!(md.contains("Version 1, approved"), "{md}");

    let err = fx.svc.decide_breakdown(&a.id, approve()).await.unwrap_err();
    assert!(matches!(err, ServiceError::NoPendingBreakdown), "{err:?}");
}

#[tokio::test]
async fn a_decision_on_a_version_not_pending_is_refused() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let handle = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    let seen = |version| DecideBreakdown {
        version: Some(version),
        ..approve()
    };
    let err = fx.svc.decide_breakdown(&a.id, seen(2)).await.unwrap_err();
    assert!(
        matches!(
            err,
            ServiceError::StaleBreakdown {
                pending: 1,
                seen: 2
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "breakdown v1 is pending, not v2; review it first"
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(
        p.breakdowns[0].approval,
        Approval::Proposed,
        "nothing decided"
    );

    fx.svc.decide_breakdown(&a.id, seen(1)).await.unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answer.unwrap(), BreakdownAnswer::Approved);
}

#[tokio::test]
async fn changes_requested_returns_comment() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let handle = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
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
                version: None,
            },
        )
        .await
        .unwrap();
    let answer = handle.await.unwrap().unwrap();
    assert_eq!(
        answer,
        BreakdownAnswer::ChangesRequested("split the api".into())
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].approval, Approval::ChangesRequested);
    assert_eq!(p.breakdowns[0].comment.as_deref(), Some("split the api"));
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);

    // The next proposal is version 2 and keeps the numbers.
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1] = step("api part two", "rust", &[1]);
    let handle = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    let v2 = &p.breakdowns[1];
    assert_eq!(v2.version, 2);
    assert_eq!(v2.steps.iter().map(|s| s.n).collect::<Vec<_>>(), [1, 3]);
    assert_eq!(agent(&fx, &a.id).await.activity, "breakdown v2 ready");
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(handle.await.unwrap().unwrap(), BreakdownAnswer::Approved);
}

#[tokio::test]
async fn a_bad_proposal_changes_nothing() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
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
        .task(&a.id)
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
        .spawn(request(&fx, AgentKind::Solo, None, &["core"], &[]))
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

/// Nothing about a breakdown decision was typed into the agent's pane.
async fn assert_nothing_typed(fx: &Fx, id: &AgentId) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pane = fx.tmux.capture(&session_name(id)).await.unwrap();
    for typed in [
        "fleet-manager: breakdown",
        "breakdown v1 approved",
        "changes requested",
    ] {
        assert!(!pane.contains(typed), "{pane}");
    }
}

#[tokio::test]
async fn decision_after_hold_goes_to_the_inbox() {
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
    let a = svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let err = svc.propose_breakdown(&a.id, two_steps()).await.unwrap_err();
    assert!(matches!(err, ServiceError::BreakdownNoAnswerYet), "{err:?}");
    assert!(err.to_string().contains("wait_for_fleet"), "{err}");
    // The proposal outlives the call: it is still Antonio's to decide.
    let card = agent(&fx, &a.id).await;
    assert_eq!(
        (card.status, card.blocked_reason),
        (Status::NeedsYou, Some(BlockedReason::Breakdown))
    );
    svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);
    assert_nothing_typed(&fx, &a.id).await;
    assert_eq!(
        svc.wait_for_fleet_lines(&a.id).await.unwrap(),
        ["breakdown v1 approved"]
    );

    let handle = tokio::spawn({
        let (svc, id) = (svc.clone(), a.id.clone());
        let mut drafts = two_steps();
        drafts[0].n = Some(1);
        drafts[1].n = Some(2);
        async move { svc.propose_breakdown(&id, drafts).await }
    });
    assert!(handle.await.unwrap().is_err(), "the hold runs out");
    svc.decide_breakdown(
        &a.id,
        DecideBreakdown {
            approve: false,
            comment: Some("smaller".into()),
            version: None,
        },
    )
    .await
    .unwrap();
    assert_nothing_typed(&fx, &a.id).await;
    assert_eq!(
        svc.wait_for_fleet_lines(&a.id).await.unwrap(),
        ["breakdown v2: changes requested: smaller"]
    );
}

/// A lead whose turn ended makes no further call, so the inbox alone would never reach it: it gets one typed line.
#[tokio::test]
async fn idle_lead_is_nudged_to_read_the_decision() {
    let fx = fleet_fixture().await;
    let (a, b) = pending_after_restart(&fx).await;
    let token = fx.store.agent_token(&b.id).await.unwrap().unwrap();
    fleet_daemon::ingest::ingest(
        &fx.store,
        &b.id,
        Some(&token),
        json!({"hook_event_name": "Stop"}),
        fx.clock.now_for_test(),
    )
    .await
    .unwrap();
    assert_eq!(agent(&fx, &b.id).await.status, Status::Review, "stored");

    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    let line = "fleet-manager: breakdown v1 decided; call wait_for_fleet";
    assert!(pane_contains(&fx, &b.id, line).await);
    let pane = fx.tmux.capture(&session_name(&b.id)).await.unwrap();
    assert_eq!(pane.matches(line).count(), 1, "{pane}");
    let inbox = fx.store.task(&a.id).await.unwrap().unwrap().inbox;
    assert_eq!(inbox.len(), 1, "{inbox:?}");
    assert_eq!(inbox[0].kind, fleet_api::FleetEventKind::BreakdownDecided);
    assert_eq!(inbox[0].detail, "breakdown v1 approved");
}

/// A session that ended on its own is Stopped before its held call is dropped: the decision must not go to that
/// call, where nobody reads it, but to the inbox a restarted lead reads.
#[tokio::test]
async fn decision_for_an_exited_lead_goes_to_the_inbox() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let handle = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    fleet_daemon::ingest::ingest(
        &fx.store,
        &a.id,
        Some(&token),
        json!({"hook_event_name": "SessionEnd"}),
        fx.clock.now_for_test(),
    )
    .await
    .unwrap();
    assert_eq!(agent(&fx, &a.id).await.status, Status::Stopped, "stored");

    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    let inbox = fx.store.task(&a.id).await.unwrap().unwrap().inbox;
    assert_eq!(inbox.len(), 1, "{inbox:?}");
    assert_eq!(inbox[0].kind, fleet_api::FleetEventKind::BreakdownDecided);
    handle.abort();
}

#[tokio::test]
async fn stop_ends_a_held_breakdown() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let handle = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    fx.svc.stop(&a.id).await.unwrap();
    let r = tokio::time::timeout(Duration::from_secs(10), handle)
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
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();

    let id = a.id.0.clone();
    let view = tokio::task::block_in_place(|| c.fleet_view(&id)).unwrap();
    assert_eq!(view.task, a.id);
    assert!(view.breakdowns.is_empty() && view.gate_runs.is_empty());
    let err = tokio::task::block_in_place(|| c.decide_breakdown(&id, &approve())).unwrap_err();
    assert_eq!(err.0, ServiceError::NoPendingBreakdown.to_string());
    let (status, _) = json_of(
        addr,
        "POST",
        &format!("/api/tasks/{id}/breakdown/decide"),
        Some(&json!({"approve": true})),
    )
    .await;
    assert_eq!(status, 409);

    let handle = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    let view = tokio::task::block_in_place(|| c.fleet_view(&id)).unwrap();
    assert_eq!(view.breakdowns.len(), 1);
    assert_eq!(view.breakdowns[0].steps.len(), 2);
    tokio::task::block_in_place(|| c.decide_breakdown(&id, &approve())).unwrap();
    assert_eq!(handle.await.unwrap().unwrap(), BreakdownAnswer::Approved);

    let (status, _) = json_of(addr, "GET", "/api/tasks/nope/fleet", None).await;
    assert_eq!(status, 404);
}

/// An orchestrator whose `two_steps` breakdown is approved, with its task folder and name.
async fn approved_task(fx: &Fx) -> (fleet_api::Agent, PathBuf, String) {
    let a = fx.svc.spawn(orchestrator(fx, "direct")).await.unwrap();
    let handle = propose_in_background(fx, &fx.svc, &a.id, two_steps()).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(handle.await.unwrap().unwrap(), BreakdownAnswer::Approved);
    let task = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    let name = task.file_name().unwrap().to_string_lossy().into_owned();
    (a, task, name)
}

fn fleet_refusal(err: &ServiceError) -> &str {
    match err {
        ServiceError::Fleet(text) => text,
        other => panic!("not a fleet refusal: {other:?}"),
    }
}

/// Nothing a refused specialist spawn could have left: no fleet folder, card or step branch.
async fn assert_no_specialist(fx: &Fx, task: &Path, name: &str) {
    assert!(!task.join("fleet").exists());
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
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let task = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    let name = task.file_name().unwrap().to_string_lossy().into_owned();
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "no breakdown is approved yet");

    let handle = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "breakdown v1 is waiting for approval");
    assert_no_specialist(&fx, &task, &name).await;
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert!(p.breakdowns[0]
        .steps
        .iter()
        .all(|s| s.state == fleet_api::StepState::Pending));
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn spawn_refused_while_new_version_pending() {
    let fx = fleet_fixture().await;
    let (a, task, name) = approved_task(&fx).await;
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    let handle = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "breakdown v2 is waiting for approval");
    assert_no_specialist(&fx, &task, &name).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn spawn_refused_after_changes_requested() {
    let fx = fleet_fixture().await;
    let (a, task, name) = approved_task(&fx).await;
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    let handle = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    fx.svc
        .decide_breakdown(
            &a.id,
            DecideBreakdown {
                approve: false,
                comment: Some("no".into()),
                version: None,
            },
        )
        .await
        .unwrap();
    handle.await.unwrap().unwrap();
    // v1 is approved and its step 1 Pending, but v1 is no longer the latest word.
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "breakdown v2 needs a new proposal");
    assert_no_specialist(&fx, &task, &name).await;
}

#[tokio::test]
async fn spawn_respects_dependencies() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
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
    let (a, task, name) = approved_task(&fx).await;
    // The orchestrator has merged something since the task started: the step starts from that.
    let lead_wt = Path::new(&a.workdir).join("core");
    std::fs::write(lead_wt.join("merged.txt"), "x").unwrap();
    git(&lead_wt, &["add", "."]);
    git(&lead_wt, &["commit", "-q", "-m", "merged"]);
    let tip = git(&lead_wt, &["rev-parse", "HEAD"]);

    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let folder = task.join("fleet").join("1-api");
    let branch = format!("fleet/{name}--1-api");
    assert_eq!(s.kind, AgentKind::Specialist);
    assert_eq!(s.task.as_ref(), Some(&a.id));
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

    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    let step = &p.breakdowns[0].steps[0];
    assert_eq!(step.state, fleet_api::StepState::Running);
    assert_eq!(step.agent.as_ref(), Some(&s.id));
    assert_eq!(step.branch.as_deref(), Some(branch.as_str()));
    assert!(p.log.iter().any(|l| l.text == "rust started for step 1"));
    let md = std::fs::read_to_string(task.join("BREAKDOWN.md")).unwrap();
    assert!(md.contains(&format!("| running | {} |", s.id.0)), "{md}");

    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "step 1 is running");
    // Only the task's orchestrator starts its specialists.
    let err = fx.svc.spawn_specialist(&s.id, 2).await.unwrap_err();
    assert!(matches!(err, ServiceError::Fleet(_)), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_spawns_start_a_step_once() {
    let fx = fleet_fixture().await;
    let (a, task, _) = approved_task(&fx).await;
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
    assert_eq!(entries(&task.join("fleet")), ["1-api"]);
}

#[tokio::test]
async fn a_failed_specialist_spawn_leaves_nothing() {
    let fx = fleet_fixture().await;
    let (a, task, name) = approved_task(&fx).await;
    // A leftover branch with the step's name makes the worktree fail after the folder exists.
    git(&fx.repo, &["branch", &format!("fleet/{name}--1-api")]);
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(entries(&task.join("fleet")), Vec::<String>::new());
    let agents = fx.store.snapshot().await.unwrap().agents;
    assert!(agents.iter().all(|x| x.kind != AgentKind::Specialist));
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(
        p.breakdowns[0].steps[0].state,
        fleet_api::StepState::Pending
    );
    assert_eq!(p.breakdowns[0].steps[0].agent, None);
}

#[tokio::test]
async fn finish_refused_when_dirty() {
    let fx = fleet_fixture().await;
    let (a, task, _) = approved_task(&fx).await;
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
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    let step = &p.breakdowns[0].steps[0];
    assert_eq!(step.state, fleet_api::StepState::Done);
    assert_eq!(step.report.as_deref(), Some("built it"));
    assert_eq!(p.inbox.len(), 1, "{:?}", p.inbox);
    assert_eq!(p.inbox[0].kind, fleet_api::FleetEventKind::Finished);
    assert_eq!(p.inbox[0].step, 1);
    let md = std::fs::read_to_string(task.join("BREAKDOWN.md")).unwrap();
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
    let (a, task, _) = approved_task(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let wt = PathBuf::from(&s.workspaces[0].worktree);
    std::fs::write(wt.join("a.rs"), "").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "a"]);
    fx.svc.finish_step(&s.id, "done").await.unwrap();
    fx.svc.spawn_specialist(&a.id, 2).await.unwrap();

    assert_eq!(
        entries(&task),
        [
            "BREAKDOWN.md",
            "fleet",
            "internal",
            "internal-snapshots",
            "orchestrator"
        ]
        .map(String::from)
    );
    assert_eq!(entries(&task.join("fleet")), ["1-api", "2-ui"]);
    for dir in [task.clone(), task.join("fleet")] {
        assert!(!dir.join("CLAUDE.md").exists(), "{}", dir.display());
        assert!(!dir.join(".claude").exists(), "{}", dir.display());
    }
}

#[tokio::test]
async fn step_starts_from_the_task_branch_not_the_leads_head() {
    let fx = fleet_fixture().await;
    let (a, _, name) = approved_task(&fx).await;
    let lead_wt = Path::new(&a.workdir).join("core");
    let old = git(&lead_wt, &["rev-parse", "HEAD"]);
    std::fs::write(lead_wt.join("merged.txt"), "x").unwrap();
    git(&lead_wt, &["add", "."]);
    git(&lead_wt, &["commit", "-q", "-m", "merged"]);
    let tip = git(&lead_wt, &["rev-parse", "HEAD"]);
    // The orchestrator looks at an older commit; the task branch still holds the merge.
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
    let (a, task, name) = approved_task(&fx).await;
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
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    let step = &p.breakdowns[0].steps[0];
    assert_eq!(step.state, fleet_api::StepState::Pending);
    assert_eq!((step.agent.as_ref(), step.branch.as_ref()), (None, None));
    assert!(p
        .log
        .iter()
        .any(|l| l.text.starts_with("step 1 could not start")));
    let md = std::fs::read_to_string(task.join("BREAKDOWN.md")).unwrap();
    assert!(!md.contains("| running |"), "{md}");
    assert_eq!(entries(&task.join("fleet")), Vec::<String>::new());
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
async fn finish_with(svc: &Arc<AgentService>, s: &fleet_api::Agent, report: &str) {
    let wt = PathBuf::from(&s.workspaces[0].worktree);
    std::fs::write(wt.join("f.rs"), report).unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "work"]);
    svc.finish_step(&s.id, report).await.unwrap();
}

#[tokio::test]
async fn wait_returns_queued_events_at_once() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    finish_with(&fx.svc, &s, "built it").await;
    let events = fx.svc.wait_for_fleet(&a.id).await.unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, fleet_api::FleetEventKind::Finished);
    assert_eq!(events[0].step, 1);
    // Taken, not copied.
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert!(p.inbox.is_empty());
}

#[tokio::test]
async fn wait_wakes_on_event() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
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
    let (a, _, _) = approved_task(&fx).await;
    assert!(svc.wait_for_fleet(&a.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn wait_ends_when_the_orchestrator_stops() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
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
    let (a, _, _) = approved_task(&fx).await;
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
    let (a, _, _) = approved_task(&fx).await;
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
    let (a, _, _) = approved_task(&fx).await;
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
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(
        p.breakdowns[0].steps[0].state,
        fleet_api::StepState::Running
    );
}

#[tokio::test]
async fn inbox_survives_daemon_restart() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
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
    let (a, _, _) = approved_task(&fx).await;
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
    let (a, _, _) = approved_task(&fx).await;
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

/// An orchestrated task whose two steps are done and whose branch adds `src/lib.rs`, with `core`'s origin a bare
/// repo whose `main` has `workflow`, so `open_pr` really fetches and pushes. Returns the lead, its worktree, the
/// task branch and the bare origin.
async fn done_task(fx: &Fx, workflow: &str) -> (fleet_api::Agent, PathBuf, String, PathBuf) {
    let (a, _, _) = approved_task(fx).await;
    finish_steps(fx, &a).await;
    let bare = push_workflow(fx, workflow);
    let (wt, branch) = commit_lib(&a, "src/lib.rs");
    (a, wt, branch, bare)
}

async fn finish_steps(fx: &Fx, a: &fleet_api::Agent) {
    for n in [1, 2] {
        let s = fx.svc.spawn_specialist(&a.id, n).await.unwrap();
        finish_with(&fx.svc, &s, &format!("step {n}")).await;
    }
}

/// Commits an empty `file` in the lead's repo worktree; returns the worktree and its branch.
fn commit_lib(a: &fleet_api::Agent, file: &str) -> (PathBuf, String) {
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    std::fs::create_dir_all(wt.join(file).parent().unwrap()).unwrap();
    std::fs::write(wt.join(file), "").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "merged"]);
    (wt, a.workspaces[0].branch.clone())
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
    let fx = fleet_fixture().await;
    let (a, _, branch, bare) = done_task(&fx, &one_step("echo boom; exit 3")).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    let run = gate_failure(&err);
    assert_eq!(run.base, "main");
    assert_eq!(run.steps.len(), 1);
    assert_eq!(run.steps[0].exit_code, Some(3));
    assert!(
        run.steps[0].output_tail.contains("boom"),
        "{}",
        run.steps[0].output_tail
    );
    assert_eq!(run.steps[0].command, "echo boom; exit 3");
    assert_eq!(run.pr_url, None);
    assert_eq!(git(&bare, &["branch", "--list", &branch]), "");
    assert!(!gh_calls(&fx).await.contains("pr "));
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.gate_runs, vec![run.clone()]);
    assert_ne!(agent(&fx, &a.id).await.status, Status::Review);
}

#[tokio::test]
async fn gate_timeout() {
    let fx = fixture_config(|c| c.test_timeout_secs = 1).await;
    ready_for_fleet(&fx).await;
    let pid_file = fx._tmp.path().join("sleeper.pid");
    // A grandchild of the shell: only killing the whole group ends it.
    let cmd = format!("sleep 30 & echo $! > {}; wait", pid_file.display());
    let (a, _, branch, bare) = done_task(&fx, &one_step(&cmd)).await;
    let started = std::time::Instant::now();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(gate_failure(&err).steps[0].exit_code, None);
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
    let fx = fleet_fixture().await;
    let (a, _, _, _) = done_task(&fx, &one_step("seq 1 1000; exit 1")).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    let lines: Vec<&str> = gate_failure(&err).steps[0].output_tail.lines().collect();
    assert_eq!(lines.len(), 200);
    assert_eq!(lines.first(), Some(&"801"));
    assert_eq!(lines.last(), Some(&"1000"));
}

#[tokio::test]
async fn dirty_worktree_refused() {
    let fx = fleet_fixture().await;
    let marker = fx._tmp.path().join("ran");
    let (a, wt, branch, _) =
        done_task(&fx, &one_step(&format!("touch {}", marker.display()))).await;

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

    // The tests run on the task branch, not on whatever the lead has checked out.
    git(&wt, &["checkout", "-q", "--detach"]);
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), format!("check out {branch} first"));

    assert!(!marker.exists());
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert!(p.gate_runs.is_empty());
}

#[tokio::test]
async fn open_steps_refused() {
    let fx = fixture().await;
    let marker = fx._tmp.path().join("ran");
    add_origin_with_workflow(&fx, &one_step(&format!("touch {}", marker.display())));
    set_fleet(&fx, &["rust"]).await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "no breakdown is approved yet");

    let handle = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "breakdown v1 is waiting for approval");
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    handle.await.unwrap().unwrap();
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
    let (a, _, branch, bare) = done_task(&fx, &one_step("true")).await;
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
            "pr create --head {branch} --base main --title=Add the api --body=both steps"
        )),
        "{calls}"
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert!(
        p.log.iter().any(|l| l.text == format!("PR opened: {url}")),
        "{:?}",
        p.log
    );
    assert_eq!(p.gate_runs.len(), 1);
    assert!(p.gate_runs[0].passed());
    assert_eq!(p.gate_runs[0].pr_url.as_deref(), Some(url.as_str()));
    // Recorded on the task, open until the merge watcher hears otherwise.
    assert_eq!(
        p.prs,
        vec![fleet_api::TaskPr {
            url: url.clone(),
            repo: p.repos[0].repo.clone(),
            state: fleet_api::PrState::Open,
            merged_at: None,
        }]
    );
    let lead = agent(&fx, &a.id).await;
    assert_eq!(
        (lead.status, lead.activity.as_str()),
        (Status::Review, "PR open")
    );
}

#[tokio::test]
async fn push_failure_keeps_the_run() {
    let fx = fleet_fixture().await;
    let (a, wt, _, _) = done_task(&fx, &one_step("true")).await;
    // The fetch has to work: only the push loses its remote, while the tests run.
    git(
        &wt,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            "file:///nowhere.git",
        ],
    );
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    let text = fleet_refusal(&err);
    assert!(text.starts_with("push failed: "), "{text}");
    assert!(!gh_calls(&fx).await.contains("pr "));
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.gate_runs.len(), 1);
    assert!(p.gate_runs[0].passed());
    assert_eq!(p.gate_runs[0].pr_url, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_breakdown_holds_still_while_the_gate_runs() {
    let fx = fleet_fixture().await;
    let marker = fx._tmp.path().join("started");
    let cmd = format!("touch {}; sleep 2; exit 1", marker.display());
    let (a, _, _, _) = done_task(&fx, &one_step(&cmd)).await;
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
    assert_eq!(gate_failure(&err).steps[0].exit_code, Some(1));
    // Over: a fix step can be proposed.
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    drafts.push(step("fix", "rust", &[]));
    let handle = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    handle.abort();
}

#[tokio::test]
async fn a_cancelled_open_pr_ends_the_tests() {
    let fx = fleet_fixture().await;
    let pid_file = fx._tmp.path().join("sleeper.pid");
    let cmd = format!("sleep 30 & echo $! > {}; wait", pid_file.display());
    let (a, _, _, _) = done_task(&fx, &one_step(&cmd)).await;
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
    let fx = fleet_fixture().await;
    let cmd = "git -c commit.gpgsign=false commit -q --allow-empty -m sneaked";
    let (a, _, branch, bare) = done_task(&fx, &one_step(cmd)).await;
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
    let fx = fleet_fixture().await;
    let cmd = "git push -q origin HEAD:refs/heads/sneaky; true";
    let (a, wt, branch, bare) = done_task(&fx, &one_step(cmd)).await;
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
    let (a, _, _, _) = done_task(&fx, &one_step("true")).await;
    std::fs::write(gh_dir(&fx).await.join("pr-empty"), "").unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        "push failed: the branch is pushed, but gh pr create printed no URL"
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.gate_runs[0].pr_url, None);
    assert_ne!(agent(&fx, &a.id).await.status, Status::Review);
}

#[tokio::test]
async fn files_the_tests_leave_do_not_block_the_pr() {
    let fx = fleet_fixture().await;
    let (a, wt, branch, bare) = done_task(&fx, &one_step("echo built > build.log")).await;
    let tested = git(&wt, &["rev-parse", "HEAD"]);
    let url = fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    assert_eq!(url, "https://example.test/pr/1");
    assert!(wt.join("build.log").exists());
    assert_eq!(git(&bare, &["rev-parse", &branch]), tested);
}

/// The task's log lines that start with `prefix`.
async fn log_lines(fx: &Fx, id: &AgentId, prefix: &str) -> Vec<String> {
    let p = fx.store.task(id).await.unwrap().unwrap();
    p.log
        .into_iter()
        .map(|l| l.text)
        .filter(|t| t.starts_with(prefix))
        .collect()
}

#[tokio::test]
async fn gate_runs_the_selected_steps_in_order() {
    let fx = fleet_fixture().await;
    let wf = "steps:\n  a:\n    run: \"true\"\n  b:\n    run: exit 0\n    paths: [nothing/**]\n  c:\n    run: \"true\"\n    paths: [src/**]\n";
    let (a, _, branch, bare) = done_task(&fx, wf).await;
    fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    let run = &p.gate_runs[0];
    let names: Vec<&str> = run.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["a", "c"]);
    assert!(run.steps.iter().all(|s| s.exit_code == Some(0)));
    assert_eq!(run.steps[0].because, Vec::<String>::new());
    assert_eq!(run.steps[1].because, ["src/lib.rs"]);
    assert!(run.unmatched.is_empty());
    assert_eq!(git(&bare, &["branch", "--list", &branch]), branch);
    let lines = log_lines(&fx, &a.id, "tests ran").await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("tests ran on main: a ok, c ok (") && lines[0].ends_with("s)"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn gate_stops_at_the_first_failure() {
    let fx = fleet_fixture().await;
    let wf = "steps:\n  a:\n    run: exit 3\n  b:\n    run: touch ran-b\n";
    let (a, wt, branch, bare) = done_task(&fx, wf).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    let text = err.to_string();
    assert!(
        text.lines().next().unwrap().contains("step a") && text.contains("exit 3"),
        "{text}"
    );
    assert_eq!(gate_failure(&err).steps.len(), 1);
    assert!(!wt.join("ran-b").exists());
    assert_eq!(git(&bare, &["branch", "--list", &branch]), "");
    assert!(!gh_calls(&fx).await.contains("pr "));
}

#[tokio::test]
async fn gate_ignores_the_branchs_workflow() {
    let fx = fleet_fixture().await;
    let wf = "steps:\n  test:\n    run: exit 1\n  lib:\n    run: \"true\"\n    paths: [src/**]\n";
    let (a, wt, branch, bare) = done_task(&fx, wf).await;
    std::fs::create_dir_all(wt.join(".fleet")).unwrap();
    std::fs::write(
        wt.join(".fleet/workflow.yml"),
        "ignore: [\"**\"]\nsteps:\n  test:\n    run: \"true\"\n",
    )
    .unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "weaken the gate"]);
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    let run = gate_failure(&err);
    assert_eq!(run.steps[0].command, "exit 1");
    assert_eq!(run.unmatched, [".fleet/workflow.yml"]);
    assert!(
        err.to_string()
            .ends_with("\nran every step: no step covers .fleet/workflow.yml"),
        "{err}"
    );
    let lines = log_lines(&fx, &a.id, "tests ran").await;
    assert!(
        lines[0].ends_with("; every step: unmatched .fleet/workflow.yml"),
        "{lines:?}"
    );
    assert_eq!(git(&bare, &["branch", "--list", &branch]), "");
}

#[tokio::test]
async fn gate_budget_spans_steps() {
    let fx = fixture_config(|c| c.test_timeout_secs = 2).await;
    ready_for_fleet(&fx).await;
    let wf = "steps:\n  a:\n    run: sleep 1.5\n  b:\n    run: sleep 5\n";
    let (a, _, branch, bare) = done_task(&fx, wf).await;
    let started = std::time::Instant::now();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    let run = gate_failure(&err);
    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.steps[0].exit_code, Some(0));
    assert_eq!(run.steps[1].exit_code, None);
    assert!(err.to_string().contains("timed out after"), "{err}");
    assert_eq!(git(&bare, &["branch", "--list", &branch]), "");
}

#[tokio::test]
async fn gate_refuses_without_the_file() {
    let fx = fleet_fixture().await;
    let (a, _, _, _) = done_task(&fx, &one_step("true")).await;
    // Removed from the base after the task started: the gate reads the remote's base, as it is now.
    git(&fx.repo, &["rm", "-q", ".fleet/workflow.yml"]);
    git(&fx.repo, &["commit", "-q", "-m", "drop the workflow"]);
    git(&fx.repo, &["push", "-q", "origin", "main"]);
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        "no .fleet/workflow.yml on main; fix it on main and call open_pr again"
    );

    std::fs::create_dir_all(fx.repo.join(".fleet")).unwrap();
    std::fs::write(fx.repo.join(".fleet/workflow.yml"), "steps: {}\n").unwrap();
    git(&fx.repo, &["add", "."]);
    git(&fx.repo, &["commit", "-q", "-m", "empty workflow"]);
    git(&fx.repo, &["push", "-q", "origin", "main"]);
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        ".fleet/workflow.yml on main is invalid: `steps` is empty; fix it on main and call open_pr again"
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert!(p.gate_runs.is_empty());
}

#[tokio::test]
async fn gate_refuses_with_no_changes() {
    let fx = fleet_fixture().await;
    let marker = fx._tmp.path().join("ran");
    let (a, wt, _, _) = done_task(&fx, &one_step(&format!("touch {}", marker.display()))).await;
    git(&wt, &["reset", "-q", "--hard", "HEAD~1"]);
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "nothing changed against main");
    assert!(!marker.exists());
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert!(p.gate_runs.is_empty());
}

#[tokio::test]
async fn gate_refuses_when_no_step_runs() {
    let fx = fleet_fixture().await;
    // `done_task` changes only `src/lib.rs`, which is ignored, and the one step watches other paths.
    let wf = "ignore: [src/**]\nsteps:\n  web:\n    run: \"true\"\n    paths: [web/**]\n";
    let (a, _, _, _) = done_task(&fx, wf).await;
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        "no workflow step runs for these changes (every changed file is ignored); add a step without `paths` on main"
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert!(p.gate_runs.is_empty());
}

/// An orchestrator whose breakdown of two independent steps is approved, so both can run at once.
async fn approved_parallel(fx: &Fx) -> fleet_api::Agent {
    let a = fx.svc.spawn(orchestrator(fx, "direct")).await.unwrap();
    let drafts = vec![step("api", "rust", &[]), step("ui", "rust", &[])];
    let handle = propose_in_background(fx, &fx.svc, &a.id, drafts).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(handle.await.unwrap().unwrap(), BreakdownAnswer::Approved);
    a
}

#[tokio::test]
async fn stop_task_stops_everything() {
    let fx = fleet_fixture().await;
    let a = approved_parallel(&fx).await;
    let s1 = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let s2 = fx.svc.spawn_specialist(&a.id, 2).await.unwrap();

    assert_eq!(fx.svc.stop_task(&a.id).await.unwrap(), 3);
    for id in [&a.id, &s1.id, &s2.id] {
        let x = agent(&fx, id).await;
        assert_eq!(x.status, Status::Stopped, "{x:?}");
        assert!(!fx.tmux.has_session(&x.tmux_session).await.unwrap());
    }
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, fleet_api::TaskState::Stopped);
    assert_eq!(p.stop_cause, Some(fleet_api::StopCause::Stopped));
    // Idempotent: nothing is left to stop.
    assert_eq!(fx.svc.stop_task(&a.id).await.unwrap(), 0);
    assert!(matches!(
        fx.svc.stop_task(&AgentId("nope".into())).await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn stopping_specialist_fails_step_only() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    fx.svc.stop(&s.id).await.unwrap();

    assert_eq!(agent(&fx, &a.id).await.status, Status::Working);
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, fleet_api::TaskState::Running);
    assert_eq!(p.breakdowns[0].steps[0].state, fleet_api::StepState::Failed);
    // The orchestrator's card keeps its own screen; the specialist's stop is logged on the task.
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
    let (a, task, _) = approved_task(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    crash(&fx, &a.id).await;
    assert_ne!(agent(&fx, &s.id).await.status, Status::Stopped);

    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    assert_ne!(b.id, a.id);
    assert_eq!(b.kind, AgentKind::Orchestrator);
    assert_eq!(b.task.as_ref(), Some(&a.id));
    assert_eq!(b.role.as_deref(), Some("direct"));
    // The same folder and worktree, not a new one.
    assert_eq!(b.workdir, a.workdir);
    assert_eq!(b.workspaces[0].worktree, a.workspaces[0].worktree);
    assert_eq!(b.workspaces[0].branch, a.workspaces[0].branch);
    assert_eq!(
        entries(&task),
        [
            "BREAKDOWN.md",
            "fleet",
            "internal",
            "internal-snapshots",
            "orchestrator"
        ]
    );
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

    let p = fx.store.task(&a.id).await.unwrap().unwrap();
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
    assert_eq!(s2.task.as_ref(), Some(&a.id));
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
    let (a, _, _) = approved_task(&fx).await;
    let err = fx.svc.restart_orchestrator(&a.id).await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "the orchestrator is still running");
    assert_eq!(fx.store.task_agents(&a.id).await.unwrap().len(), 1);

    let solo = fx
        .svc
        .spawn(request(&fx, AgentKind::Solo, None, &["core"], &[]))
        .await
        .unwrap();
    fx.svc.stop(&solo.id).await.unwrap();
    let err = fx.svc.restart_orchestrator(&solo.id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Fleet(_)), "{err:?}");
    assert_eq!(fx.store.task_agents(&solo.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn restart_into_a_stopped_task_runs_it_again() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    assert_eq!(fx.svc.stop_task(&a.id).await.unwrap(), 2);
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, fleet_api::TaskState::Stopped);
    assert!(p.stopped_at.is_some());

    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.state, fleet_api::TaskState::Running);
    assert_eq!((p.stopped_at, p.stop_cause), (None, None));
    assert_eq!(p.lead.as_ref(), Some(&b.id));
    assert_eq!(
        p.log.last().unwrap().text,
        "orchestrator restarted; task running again"
    );
    // The step the stop failed is in the prompt, for the new orchestrator to deal with.
    let prompt = std::fs::read_to_string(Path::new(&b.workdir).join(PROMPT_FILE)).unwrap();
    assert!(prompt.contains("| failed |"), "{prompt}");
    assert_eq!(agent(&fx, &s.id).await.status, Status::Stopped);
    // Stopping it again ends the new orchestrator only.
    assert_eq!(fx.svc.stop_task(&a.id).await.unwrap(), 1);
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
    let (a, _, _) = approved_task(&fx).await;
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let id = a.id.0.clone();

    let (status, body) = json_of(addr, "POST", &format!("/api/tasks/{id}/restart"), None).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(tokio::task::block_in_place(|| c.stop_task(&id)).unwrap(), 2);
    let (status, body) = json_of(addr, "POST", &format!("/api/tasks/{id}/stop"), None).await;
    assert_eq!((status, body), (200, json!({ "stopped": 0 })));

    let b = tokio::task::block_in_place(|| c.restart_orchestrator(&id)).unwrap();
    assert_eq!(b.task.as_ref(), Some(&a.id));
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.lead.as_ref(), Some(&b.id));
    let (status, _) = json_of(addr, "POST", "/api/tasks/nope/stop", None).await;
    assert_eq!(status, 404);
}

/// A service on the fixture's state whose `propose_breakdown` and `finish_step` stop at `gate`, holding calls
/// `hold` long.
fn gated(fx: &Fx, gate: &Arc<RequestGate>, hold: Duration) -> Arc<AgentService> {
    let mut opts = fx.svc.options().clone();
    opts.fleet_gate = Some(gate.clone());
    opts.repo_request_hold = hold;
    Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        opts,
    ))
}

#[tokio::test]
async fn a_failed_step_does_not_block_the_pr() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let handle = propose_in_background(&fx, &fx.svc, &a.id, vec![step("api", "rust", &[])]).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    handle.await.unwrap().unwrap();
    let s1 = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    fx.svc.stop(&s1.id).await.unwrap();
    // The retry is a new step, carried with the failed one, and approved like any other.
    let mut failed = step("api", "rust", &[]);
    failed.n = Some(1);
    let drafts = vec![failed, step("api again", "rust", &[])];
    let handle = propose_in_background(&fx, &fx.svc, &a.id, drafts).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    handle.await.unwrap().unwrap();
    let s2 = fx.svc.spawn_specialist(&a.id, 2).await.unwrap();
    finish_with(&fx.svc, &s2, "built it").await;
    let bare = push_workflow(&fx, &one_step("true"));
    let wt = PathBuf::from(&a.workspaces[0].worktree);
    std::fs::write(wt.join("api.rs"), "").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "merged"]);

    let url = fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    assert_eq!(url, "https://example.test/pr/1");
    let branch = &a.workspaces[0].branch;
    assert_eq!(git(&bare, &["branch", "--list", branch]), *branch);
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    let steps = &p.breakdowns[1].steps;
    assert_eq!(steps[0].state, fleet_api::StepState::Failed);
    assert!(
        p.log
            .iter()
            .any(|l| l.text.starts_with("tests ran on main: test ok (")
                && l.text.ends_with("; failed steps: 1")),
        "{:?}",
        p.log
    );
}

#[tokio::test]
async fn a_pending_or_running_step_still_blocks_the_pr_after_a_failure() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
    let s1 = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    fx.svc.stop(&s1.id).await.unwrap();
    let err = fx.svc.open_pr(&a.id, "t", "b").await.unwrap_err();
    assert_eq!(fleet_refusal(&err), "step 2 is pending");
}

#[tokio::test]
async fn a_cancelled_spawn_ends_whole_or_not_at_all() {
    let fx = fleet_fixture().await;
    let (a, task, _) = approved_task(&fx).await;
    let gate = SpawnGate::install(&fx.repo, fx._tmp.path());
    let (svc, id) = (fx.svc.clone(), a.id.clone());
    let call = tokio::spawn(async move { svc.spawn_specialist(&id, 1).await });
    // Cancelled mid-launch, as a client that hangs up cancels the MCP route: the folder exists, nothing else.
    assert!(gate.wait_reached().await);
    call.abort();
    let _ = call.await;
    gate.open();

    let settled = || async {
        let p = fx.store.task(&a.id).await.unwrap().unwrap();
        let step = p.breakdowns[0].steps[0].clone();
        let agents = fx.store.task_agents(&a.id).await.unwrap();
        let card = agents
            .iter()
            .find(|x| x.kind == AgentKind::Specialist)
            .cloned();
        match (step.state, card) {
            (fleet_api::StepState::Running, Some(c)) => {
                step.agent.as_ref() == Some(&c.id)
                    && fx.tmux.has_session(&c.tmux_session).await.unwrap()
            }
            (fleet_api::StepState::Pending, None) => {
                !task.join("fleet").exists() || entries(&task.join("fleet")).is_empty()
            }
            _ => false,
        }
    };
    assert!(
        wait_until(settled).await,
        "the step is neither fully started nor fully undone"
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    if p.breakdowns[0].steps[0].state == fleet_api::StepState::Pending {
        fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    } else {
        let agents = fx.store.task_agents(&a.id).await.unwrap();
        let s = agents
            .iter()
            .find(|x| x.kind == AgentKind::Specialist)
            .unwrap();
        finish_with(&fx.svc, s, "built it").await;
        fx.svc.spawn_specialist(&a.id, 2).await.unwrap();
    }
}

#[tokio::test]
async fn a_cancelled_finish_step_still_finishes() {
    let fx = fleet_fixture().await;
    let gate = Arc::new(RequestGate::default());
    let svc = gated(&fx, &gate, Duration::from_secs(60));
    let (a, _, _) = approved_task(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let wt = PathBuf::from(&s.workspaces[0].worktree);
    std::fs::write(wt.join("f.rs"), "x").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "work"]);
    let (svc2, id) = (svc.clone(), s.id.clone());
    let call = tokio::spawn(async move { svc2.finish_step(&id, "built it").await });
    tokio::time::timeout(Duration::from_secs(10), gate.reached.notified())
        .await
        .expect("finish_step never reached the gap");
    call.abort();
    let _ = call.await;
    gate.open.notify_one();
    assert!(
        wait_until(|| async {
            let p = fx.store.task(&a.id).await.unwrap().unwrap();
            let step = &p.breakdowns[0].steps[0];
            step.state == fleet_api::StepState::Done && step.report.as_deref() == Some("built it")
        })
        .await,
        "the agent stopped as Finished but its step was never marked done"
    );
}

#[tokio::test]
async fn a_permission_prompt_replaces_the_breakdown_pin() {
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
    let a = svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let err = svc.propose_breakdown(&a.id, two_steps()).await.unwrap_err();
    assert!(matches!(err, ServiceError::BreakdownNoAnswerYet), "{err:?}");

    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    fleet_daemon::ingest::ingest(
        &fx.store,
        &a.id,
        Some(&token),
        json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash"}),
        fx.clock.now_for_test(),
    )
    .await
    .unwrap();
    let card = agent(&fx, &a.id).await;
    assert_eq!(
        (card.status, card.blocked_reason),
        (Status::NeedsYou, Some(BlockedReason::Permission))
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(
        p.breakdowns[0].approval,
        Approval::Proposed,
        "still pending"
    );

    // The decision is stored, but the prompt keeps the card, and nothing is typed into it.
    svc.decide_breakdown(&a.id, approve()).await.unwrap();
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].approval, Approval::Approved);
    let card = agent(&fx, &a.id).await;
    assert_eq!(
        (card.status, card.blocked_reason),
        (Status::NeedsYou, Some(BlockedReason::Permission))
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pane = fx.tmux.capture(&session_name(&a.id)).await.unwrap();
    assert!(!pane.contains("fleet-manager: breakdown"), "{pane}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_step_that_fails_during_a_proposal_stays_failed() {
    let fx = fleet_fixture().await;
    let gate = Arc::new(RequestGate::default());
    let svc = gated(&fx, &gate, Duration::from_secs(60));
    let (a, _, _) = approved_task(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    drafts.push(step("docs", "rust", &[]));
    let (svc2, id) = (svc.clone(), a.id.clone());
    let handle = tokio::spawn(async move { svc2.propose_breakdown(&id, drafts).await });
    tokio::time::timeout(Duration::from_secs(10), gate.reached.notified())
        .await
        .expect("the proposal never reached the gap");
    // The specialist crashes between the proposal's read and its write.
    let now = fx.clock.now_for_test();
    fx.store
        .update_agent(&s.id, |x| {
            x.status = Status::Stopped;
            x.stop_cause = Some(fleet_api::StopCause::Crashed);
            x.activity = "crashed".into();
            x.status_since = now;
        })
        .await
        .unwrap();
    gate.open.notify_one();
    assert!(
        wait_until(|| async {
            fx.store
                .task(&a.id)
                .await
                .unwrap()
                .unwrap()
                .breakdowns
                .len()
                == 2
        })
        .await
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].steps[0].state, fleet_api::StepState::Failed);
    assert_eq!(
        p.breakdowns[1].steps[0].state,
        fleet_api::StepState::Failed,
        "the new version brings the step back as running"
    );
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_line_carries_the_report() {
    let fx = fleet_fixture().await;
    let gate = Arc::new(RequestGate::default());
    let svc = gated(&fx, &gate, Duration::from_secs(60));
    let (a, _, _) = approved_task(&fx).await;
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let wt = PathBuf::from(&s.workspaces[0].worktree);
    std::fs::write(wt.join("f.rs"), "x").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "work"]);
    let (svc2, id) = (svc.clone(), s.id.clone());
    let finish = tokio::spawn(async move { svc2.finish_step(&id, "built it").await });
    tokio::time::timeout(Duration::from_secs(10), gate.reached.notified())
        .await
        .expect("finish_step never reached the gap");
    // The agent is stopped and its event queued; the step is not marked done yet.
    // On the same service: the daemon has one, and its lock is what orders the two.
    let (svc3, lead) = (svc.clone(), a.id.clone());
    let wait = tokio::spawn(async move { svc3.wait_for_fleet_lines(&lead).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    gate.open.notify_one();
    finish.await.unwrap().unwrap();
    let lines = wait.await.unwrap().unwrap();
    assert_eq!(lines, ["step 1 (rust): finished: built it"]);
}

#[tokio::test]
async fn open_pr_refuses_to_push_for_a_lead_that_stopped() {
    let fx = fleet_fixture().await;
    let marker = fx._tmp.path().join("started");
    let cmd = format!("touch {}; sleep 1", marker.display());
    let (a, _, branch, bare) = done_task(&fx, &one_step(&cmd)).await;
    let (svc, id) = (fx.svc.clone(), a.id.clone());
    let call = tokio::spawn(async move { svc.open_pr(&id, "t", "b").await });
    assert!(wait_until(|| async { marker.exists() }).await);
    fx.svc.stop(&a.id).await.unwrap();
    let err = call.await.unwrap().unwrap_err();
    assert!(matches!(err, ServiceError::NotRunning), "{err:?}");
    assert_eq!(git(&bare, &["branch", "--list", &branch]), "");
    assert!(!gh_calls(&fx).await.contains("pr "));
}

/// A service whose breakdown hold runs out quickly, served over HTTP so hooks go through the real route.
async fn short_hold_server(fx: &Fx) -> (Arc<AgentService>, std::net::SocketAddr) {
    let mut opts = fx.svc.options().clone();
    opts.repo_request_hold = Duration::from_millis(300);
    let svc = Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        opts,
    ));
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(svc.clone(), fx.store.clone(), fx.tmux.clone(), addr.port());
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (svc, addr)
}

async fn post_hook(fx: &Fx, addr: std::net::SocketAddr, id: &AgentId, body: Value) {
    let token = fx.store.agent_token(id).await.unwrap().unwrap();
    let (status, text) = http(
        addr,
        "POST",
        &format!("/internal/hooks/{}", id.0),
        &[("X-Fleet-Token", &token)],
        Some(&body),
    )
    .await;
    assert_eq!(status, 200, "{text}");
}

fn permission_request() -> Value {
    json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash"})
}

fn post_tool_use() -> Value {
    json!({"hook_event_name": "PostToolUse", "tool_name": "Bash"})
}

#[tokio::test(flavor = "multi_thread")]
async fn decision_during_a_live_prompt_waits_in_the_inbox() {
    let fx = fleet_fixture().await;
    let (svc, addr) = short_hold_server(&fx).await;
    let a = svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let err = svc.propose_breakdown(&a.id, two_steps()).await.unwrap_err();
    assert!(matches!(err, ServiceError::BreakdownNoAnswerYet), "{err:?}");
    post_hook(&fx, addr, &a.id, permission_request()).await;

    svc.decide_breakdown(&a.id, approve()).await.unwrap();
    // Answering the prompt hands the card back, and still nothing is typed: the decision waits in the inbox.
    post_hook(&fx, addr, &a.id, post_tool_use()).await;
    assert_nothing_typed(&fx, &a.id).await;
    assert_eq!(
        svc.wait_for_fleet_lines(&a.id).await.unwrap(),
        ["breakdown v1 approved"]
    );
}

/// Lead A proposes `two_steps` and is held; the task is stopped and restarted. Returns A and the new lead B, with
/// v1 still Proposed.
async fn pending_after_restart(fx: &Fx) -> (fleet_api::Agent, fleet_api::Agent) {
    let a = fx.svc.spawn(orchestrator(fx, "direct")).await.unwrap();
    let handle = propose_in_background(fx, &fx.svc, &a.id, two_steps()).await;
    fx.svc.stop_task(&a.id).await.unwrap();
    let held = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    assert!(held.is_err(), "{held:?}");
    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    assert!(pane_contains(fx, &b.id, "ready").await);
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.lead.as_ref(), Some(&b.id));
    assert_eq!(p.breakdowns.last().unwrap().approval, Approval::Proposed);
    (a, b)
}

/// B's `wait_for_fleet`, started before the decision so the decision has to wake it.
fn wait_in_background(
    fx: &Fx,
    b: &AgentId,
) -> tokio::task::JoinHandle<Result<Vec<fleet_api::FleetEvent>, ServiceError>> {
    let (svc, id) = (fx.svc.clone(), b.clone());
    tokio::spawn(async move { svc.wait_for_fleet(&id).await })
}

async fn decided_event(
    handle: tokio::task::JoinHandle<Result<Vec<fleet_api::FleetEvent>, ServiceError>>,
) -> fleet_api::FleetEvent {
    let events = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("the decision never woke wait_for_fleet")
        .unwrap()
        .unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    events.into_iter().next().unwrap()
}

#[tokio::test]
async fn restart_delivers_an_approval_through_wait_for_fleet() {
    let fx = fleet_fixture().await;
    let (a, b) = pending_after_restart(&fx).await;
    let handle = wait_in_background(&fx, &b.id);
    tokio::time::sleep(Duration::from_millis(200)).await;

    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    let ev = decided_event(handle).await;
    assert_eq!(ev.kind, fleet_api::FleetEventKind::BreakdownDecided);
    assert_eq!((ev.step, &ev.agent), (0, &a.id));
    assert_eq!(ev.detail, "breakdown v1 approved");
    let s = fx.svc.spawn_specialist(&b.id, 1).await.unwrap();
    assert_eq!(s.task.as_ref(), Some(&a.id));
}

#[tokio::test]
async fn changes_requested_reaches_the_new_lead() {
    let fx = fleet_fixture().await;
    let (a, b) = pending_after_restart(&fx).await;
    let handle = wait_in_background(&fx, &b.id);
    tokio::time::sleep(Duration::from_millis(200)).await;

    fx.svc
        .decide_breakdown(
            &a.id,
            DecideBreakdown {
                approve: false,
                comment: Some("smaller".into()),
                version: None,
            },
        )
        .await
        .unwrap();
    let ev = decided_event(handle).await;
    assert_eq!(ev.kind, fleet_api::FleetEventKind::BreakdownDecided);
    assert_eq!(ev.detail, "breakdown v1: changes requested: smaller");
    // B may now propose v2.
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    let handle = propose_in_background(&fx, &fx.svc, &b.id, drafts).await;
    handle.abort();
}

#[tokio::test]
async fn decision_while_mid_turn_waits_in_the_inbox() {
    let fx = fleet_fixture().await;
    let (a, b) = pending_after_restart(&fx).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_nothing_typed(&fx, &b.id).await;

    let events = tokio::time::timeout(Duration::from_secs(10), fx.svc.wait_for_fleet(&b.id))
        .await
        .expect("the decision is not in the inbox")
        .unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].detail, "breakdown v1 approved");
    assert_eq!(
        fx.store.task(&a.id).await.unwrap().unwrap().inbox,
        [],
        "acked"
    );
}

/// The lead's card as `/api/agents` serves it.
async fn listed(addr: std::net::SocketAddr, id: &AgentId) -> Value {
    let (status, body) = json_of(addr, "GET", "/api/agents", None).await;
    assert_eq!(status, 200, "{body}");
    body["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == id.0)
        .cloned()
        .unwrap()
}

/// The status and blocked reason the last `StatusChanged` the store logged for `id` carries.
async fn last_status_event(fx: &Fx, id: &AgentId) -> (Status, Option<BlockedReason>) {
    fx.store
        .events_since(0)
        .await
        .unwrap()
        .unwrap()
        .into_iter()
        .rev()
        .find_map(|e| match e.kind {
            fleet_api::EventKind::StatusChanged {
                id: x,
                status,
                blocked_reason,
                ..
            } if &x == id => Some((status, blocked_reason)),
            _ => None,
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_breakdown_shows_needs_you_after_restart() {
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
    let (a, b) = pending_after_restart(&fx).await;

    post_hook(&fx, addr, &b.id, post_tool_use()).await;
    assert_eq!(agent(&fx, &b.id).await.status, Status::Working, "stored");
    let card = listed(addr, &b.id).await;
    assert_eq!(
        (&card["status"], &card["blocked_reason"]),
        (&json!("needs_you"), &json!("breakdown")),
        "{card}"
    );
    // The stream says the same: the hook's StatusChanged, and a fresh snapshot.
    assert_eq!(
        last_status_event(&fx, &b.id).await,
        (Status::NeedsYou, Some(BlockedReason::Breakdown))
    );
    let mut sse = SseClient::connect(addr, "/api/events", &[]).await;
    let snap = sse.next().await;
    assert_eq!(snap.event.as_deref(), Some("snapshot"));
    let snap: Value = serde_json::from_str(&snap.data).unwrap();
    let card = snap["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == b.id.0)
        .unwrap();
    assert_eq!(card["status"], "needs_you", "{card}");
    // The stopped lead A is left as it is.
    assert_eq!(listed(addr, &a.id).await["status"], "stopped");

    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    let card = listed(addr, &b.id).await;
    assert_eq!(
        (&card["status"], &card["blocked_reason"]),
        (&json!("working"), &Value::Null),
        "{card}"
    );
    assert_eq!(last_status_event(&fx, &b.id).await, (Status::Working, None));
}

#[tokio::test]
async fn takeover_prompt_says_a_version_is_pending() {
    let fx = fleet_fixture().await;
    let (_, b) = pending_after_restart(&fx).await;
    let prompt = std::fs::read_to_string(Path::new(&b.workdir).join(PROMPT_FILE)).unwrap();
    assert!(
        prompt.contains(
            "Breakdown v1 is waiting for the person's decision. Do not propose again: call wait_for_fleet; the \
             decision arrives there."
        ),
        "{prompt}"
    );

    // Nothing pending once it is decided: a later restart's prompt does not say so.
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
    crash(&fx, &a.id).await;
    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    let prompt = std::fs::read_to_string(Path::new(&b.workdir).join(PROMPT_FILE)).unwrap();
    assert!(
        !prompt.contains("waiting for the person's decision"),
        "{prompt}"
    );
}

/// The pane once the fake `claude` has printed its argv.
async fn argv_of(fx: &Fx, id: &AgentId) -> String {
    assert!(pane_contains(fx, id, "ready").await);
    fx.tmux.capture(&session_name(id)).await.unwrap()
}

async fn set_models(fx: &Fx, models: &[(&str, &str)]) {
    let entries: Vec<fleet_api::ProductOverride> = models
        .iter()
        .map(|(k, v)| fleet_api::ProductOverride {
            section: "models".into(),
            key: k.to_string(),
            value: v.to_string(),
            error: None,
        })
        .collect();
    fx.store
        .set_product_overrides(PRODUCT, &entries)
        .await
        .unwrap();
}

#[tokio::test]
async fn specialist_runs_on_its_definition_model() {
    let fx = fleet_fixture().await;
    let (a, _, _) = approved_task(&fx).await;
    assert_eq!(a.model.as_deref(), Some("opus"));
    assert!(argv_of(&fx, &a.id)
        .await
        .contains("arg=--model\narg=opus\narg=--\n"));
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    assert_eq!(s.model.as_deref(), Some("sonnet"));
    assert!(argv_of(&fx, &s.id)
        .await
        .contains("arg=--model\narg=sonnet\n"));
    assert_eq!(agent(&fx, &s.id).await.model.as_deref(), Some("sonnet"));
}

#[tokio::test]
async fn product_override_wins() {
    let fx = fleet_fixture().await;
    set_models(&fx, &[("rust", "opus"), ("direct", "haiku")]).await;
    let (a, _, _) = approved_task(&fx).await;
    assert_eq!(a.model.as_deref(), Some("haiku"));
    assert!(argv_of(&fx, &a.id)
        .await
        .contains("arg=--model\narg=haiku\n"));
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    assert_eq!(s.model.as_deref(), Some("opus"));
    assert!(argv_of(&fx, &s.id)
        .await
        .contains("arg=--model\narg=opus\n"));
}

#[tokio::test]
async fn no_model_means_no_flag() {
    let fx = fleet_fixture().await;
    let dir = fx._tmp.path().join("fleet/specialists");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plain.md"),
        "---\nname: plain\nsummary: No model of its own.\n---\nBody\n",
    )
    .unwrap();
    set_fleet(&fx, &["plain"]).await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let handle = propose_in_background(&fx, &fx.svc, &a.id, vec![step("only", "plain", &[])]).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(handle.await.unwrap().unwrap(), BreakdownAnswer::Approved);
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    assert_eq!(s.model, None);
    assert!(!argv_of(&fx, &s.id).await.contains("--model"));
    assert_eq!(agent(&fx, &s.id).await.model, None);
}

#[tokio::test]
async fn solo_has_no_model() {
    let fx = fleet_fixture().await;
    set_models(&fx, &[("rust", "opus")]).await;
    let a = fx
        .svc
        .spawn(request(&fx, AgentKind::Solo, None, &["core"], &[]))
        .await
        .unwrap();
    assert_eq!(a.model, None);
    assert!(!argv_of(&fx, &a.id).await.contains("--model"));
    assert_eq!(agent(&fx, &a.id).await.model, None);
}

#[tokio::test]
async fn restart_resolves_the_model_again() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "structured")).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("opus"));
    set_models(&fx, &[("structured", "sonnet")]).await;
    crash(&fx, &a.id).await;
    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    assert_eq!(b.model.as_deref(), Some("sonnet"));
    assert!(argv_of(&fx, &b.id)
        .await
        .contains("arg=--model\narg=sonnet\n"));
    assert_eq!(agent(&fx, &b.id).await.model.as_deref(), Some("sonnet"));
}

/// The product's overrides replaced by these `limits` entries, as the Management page's PUT does.
async fn set_limits(fx: &Fx, limits: &[(&str, u32)]) {
    let entries: Vec<ProductOverride> = limits
        .iter()
        .map(|(key, value)| ProductOverride {
            section: "limits".into(),
            key: (*key).into(),
            value: value.to_string(),
            error: None,
        })
        .collect();
    fx.store
        .set_product_overrides(PRODUCT, &entries)
        .await
        .unwrap();
}

fn independent() -> StepDraft {
    step("api", "rust", &[])
}

fn changes(comment: &str) -> DecideBreakdown {
    DecideBreakdown {
        approve: false,
        comment: Some(comment.into()),
        version: None,
    }
}

/// An orchestrator whose breakdown of `count` independent steps is approved.
async fn approved_independent(fx: &Fx, count: usize) -> fleet_api::Agent {
    let a = fx.svc.spawn(orchestrator(fx, "direct")).await.unwrap();
    let drafts = (0..count)
        .map(|i| step(&format!("step {i}"), "rust", &[]))
        .collect();
    let handle = propose_in_background(fx, &fx.svc, &a.id, drafts).await;
    fx.svc.decide_breakdown(&a.id, approve()).await.unwrap();
    assert_eq!(handle.await.unwrap().unwrap(), BreakdownAnswer::Approved);
    a
}

fn assert_limit_reached(err: &ServiceError, key: &str, value: u32, source: SettingSource) {
    match err {
        ServiceError::LimitReached {
            key: k,
            value: v,
            source: s,
        } => assert_eq!((*k, *v, *s), (key, value, source)),
        other => panic!("not a limit refusal: {other:?}"),
    }
}

#[tokio::test]
async fn version_cap_refuses_the_next_proposal() {
    let fx = fleet_fixture().await;
    set_limits(&fx, &[("max_breakdown_versions", 2)]).await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    for round in 1..=2 {
        let handle = propose_in_background(&fx, &fx.svc, &a.id, vec![independent()]).await;
        fx.svc
            .decide_breakdown(&a.id, changes(&format!("round {round}")))
            .await
            .unwrap();
        handle.await.unwrap().unwrap();
    }
    let err = fx
        .svc
        .propose_breakdown(&a.id, vec![independent()])
        .await
        .unwrap_err();
    assert_limit_reached(&err, "max_breakdown_versions", 2, SettingSource::Product);
    assert_eq!(
        err.to_string(),
        "limit reached: max_breakdown_versions = 2 (product), counted over the whole task. Stop and ask the \
         person; they can raise it for this product in Management → Products."
    );
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns.len(), 2);
    let card = agent(&fx, &a.id).await;
    assert_eq!((card.status, card.blocked_reason), (Status::Working, None));
}

/// A service over the same store and tmux whose daemon config carries these global limits, as after a restart.
fn with_global_limits(
    fx: &Fx,
    set: impl FnOnce(&mut fleet_daemon::config::LimitValues),
) -> Arc<AgentService> {
    let mut cfg = fx.svc.config().clone();
    set(&mut cfg.limits);
    Arc::new(AgentService::new(
        cfg,
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        fx.svc.options().clone(),
    ))
}

#[tokio::test]
async fn version_cap_from_the_global_config() {
    let fx = fleet_fixture().await;
    let svc = with_global_limits(&fx, |l| l.max_breakdown_versions = 1);
    let a = svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let handle = propose_in_background(&fx, &svc, &a.id, vec![independent()]).await;
    svc.decide_breakdown(&a.id, changes("again")).await.unwrap();
    handle.await.unwrap().unwrap();
    let err = svc
        .propose_breakdown(&a.id, vec![independent()])
        .await
        .unwrap_err();
    assert_limit_reached(&err, "max_breakdown_versions", 1, SettingSource::Global);
    assert!(err.to_string().contains("(global)"), "{err}");
}

#[tokio::test]
async fn parallel_cap_from_the_global_config() {
    let fx = fleet_fixture().await;
    let a = approved_independent(&fx, 2).await;
    let svc = with_global_limits(&fx, |l| l.max_parallel_specialists = 1);
    svc.spawn_specialist(&a.id, 1).await.unwrap();
    let err = svc.spawn_specialist(&a.id, 2).await.unwrap_err();
    assert!(
        matches!(
            err,
            ServiceError::TooManyRunning {
                running: 1,
                value: 1,
                source: SettingSource::Global
            }
        ),
        "{err:?}"
    );
    assert!(err.to_string().contains("global"), "{err}");
}

#[tokio::test]
async fn step_cap_counts_the_whole_task() {
    let fx = fleet_fixture().await;
    set_limits(&fx, &[("max_steps", 3)]).await;
    let (a, _, _) = approved_task(&fx).await;
    for n in [1, 2] {
        let s = fx.svc.spawn_specialist(&a.id, n).await.unwrap();
        finish_with(&fx.svc, &s, "built it").await;
    }
    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    drafts.push(step("docs", "rust", &[]));
    drafts.push(step("polish", "rust", &[]));
    let err = fx.svc.propose_breakdown(&a.id, drafts).await.unwrap_err();
    assert_limit_reached(&err, "max_steps", 3, SettingSource::Product);
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns.len(), 1);
}

#[tokio::test]
async fn parallel_cap_says_wait() {
    let fx = fleet_fixture().await;
    set_limits(&fx, &[("max_parallel_specialists", 1)]).await;
    let a = approved_independent(&fx, 2).await;
    let first = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let err = fx.svc.spawn_specialist(&a.id, 2).await.unwrap_err();
    assert!(
        matches!(
            err,
            ServiceError::TooManyRunning {
                running: 1,
                value: 1,
                source: SettingSource::Product
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "1 specialist is running (max_parallel_specialists = 1, product). Call wait_for_fleet and spawn this \
         step when one finishes."
    );
    finish_with(&fx.svc, &first, "built it").await;
    fx.svc.spawn_specialist(&a.id, 2).await.unwrap();
}

#[tokio::test]
async fn raised_override_unblocks_at_once() {
    let fx = fleet_fixture().await;
    set_limits(&fx, &[("max_parallel_specialists", 1)]).await;
    let a = approved_independent(&fx, 2).await;
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    assert!(fx.svc.spawn_specialist(&a.id, 2).await.is_err());
    set_limits(&fx, &[("max_parallel_specialists", 2)]).await;
    fx.svc.spawn_specialist(&a.id, 2).await.unwrap();
}

#[tokio::test]
async fn lowered_limit_stops_nothing_running() {
    let fx = fleet_fixture().await;
    set_limits(&fx, &[("max_parallel_specialists", 2)]).await;
    let a = approved_independent(&fx, 3).await;
    let running = [
        fx.svc.spawn_specialist(&a.id, 1).await.unwrap(),
        fx.svc.spawn_specialist(&a.id, 2).await.unwrap(),
    ];
    set_limits(&fx, &[("max_parallel_specialists", 1)]).await;
    for s in &running {
        assert_ne!(agent(&fx, &s.id).await.status, Status::Stopped);
    }
    let err = fx.svc.spawn_specialist(&a.id, 3).await.unwrap_err();
    assert!(
        err.to_string()
            .starts_with("2 specialists are running (max_parallel_specialists = 1, product)"),
        "{err}"
    );
    assert!(
        matches!(
            err,
            ServiceError::TooManyRunning {
                running: 2,
                value: 1,
                ..
            }
        ),
        "{err:?}"
    );
    for s in &running {
        assert_ne!(agent(&fx, &s.id).await.status, Status::Stopped);
    }
}

#[tokio::test]
async fn restart_keeps_a_held_proposal_and_reads_new_limits() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let held = propose_in_background(&fx, &fx.svc, &a.id, two_steps()).await;
    // The old daemon's held call dies with it.
    held.abort();
    let mut cfg = fx.svc.config().clone();
    cfg.limits.max_steps = 2;
    let rebuilt = Arc::new(AgentService::new(
        cfg,
        fx.store.clone(),
        fx.repos.clone(),
        fx.tmux.clone(),
        fx.svc.options().clone(),
    ));
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(p.breakdowns.last().unwrap().approval, Approval::Proposed);

    rebuilt.decide_breakdown(&a.id, approve()).await.unwrap();
    let events = rebuilt.wait_for_fleet(&a.id).await.unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, fleet_api::FleetEventKind::BreakdownDecided);

    let mut drafts = two_steps();
    drafts[0].n = Some(1);
    drafts[1].n = Some(2);
    drafts.push(step("docs", "rust", &[]));
    let err = rebuilt.propose_breakdown(&a.id, drafts).await.unwrap_err();
    assert_limit_reached(&err, "max_steps", 2, SettingSource::Global);
}

fn internal_repo(fx: &Fx) -> PathBuf {
    fx.state_dir.join("internal").join(PRODUCT)
}

fn task_name_of(a: &fleet_api::Agent) -> String {
    Path::new(&a.workdir)
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[tokio::test]
async fn orchestrator_gets_internal_worktree() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "structured")).await.unwrap();
    let name = task_name_of(&a);
    let task = Path::new(&a.workdir).parent().unwrap();
    let internal = task.join("internal");
    assert_eq!(
        git(&internal, &["branch", "--show-current"]),
        format!("task/{name}")
    );
    let md = std::fs::read_to_string(Path::new(&a.workdir).join("CLAUDE.md")).unwrap();
    assert!(md.contains("## Internal resources"), "{md}");
    assert!(md.contains(&format!("`{}`", internal.display())), "{md}");
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
    let dirs = settings["permissions"]["additionalDirectories"]
        .as_array()
        .unwrap();
    assert!(
        dirs.contains(&json!(internal.to_string_lossy())),
        "{dirs:?}"
    );
    // Nothing of it lands in the repo's own checkout.
    assert_eq!(
        git(&Path::new(&a.workdir).join("core"), &["status", "--short"]),
        ""
    );
}

#[tokio::test]
async fn solo_task_has_no_internal_worktree() {
    let fx = fleet_fixture().await;
    let a = fx
        .svc
        .spawn(request(&fx, AgentKind::Solo, None, &["core"], &[]))
        .await
        .unwrap();
    assert!(!Path::new(&a.workdir).join("internal").exists());
    assert!(!internal_repo(&fx).exists());
    let settings = std::fs::read_to_string(
        fx.state_dir
            .join("agents")
            .join(&a.id.0)
            .join("settings.json"),
    )
    .unwrap();
    assert!(!settings.contains("additionalDirectories"), "{settings}");
}

#[tokio::test]
async fn repo_named_internal_does_not_clash() {
    let fx = fleet_fixture().await;
    add_path_repo(&fx, "internal", &fx.repo).await;
    set_roster(&fx, "internal").await;
    let mut req = orchestrator(&fx, "direct");
    req.repos = vec![fleet_api::RepoPick {
        repo: "internal".into(),
        local: Some(fx.repo.to_string_lossy().into_owned()),
    }];
    let a = fx.svc.spawn(req).await.unwrap();
    let cwd = Path::new(&a.workdir);
    let task = cwd.parent().unwrap();
    assert!(cwd.join("internal").join("README").is_file());
    assert!(task.join("internal").is_dir());
    assert_ne!(cwd.join("internal"), task.join("internal"));
    assert_eq!(
        git(&task.join("internal"), &["branch", "--show-current"]),
        format!("task/{}", task_name_of(&a))
    );
}

async fn set_roster(fx: &Fx, repo: &str) {
    fx.store
        .set_repo_roster(PRODUCT, repo, &["rust".to_string()])
        .await
        .unwrap();
}

#[tokio::test]
async fn first_task_of_old_product_creates_repo() {
    let fx = fleet_fixture().await;
    assert!(!internal_repo(&fx).exists());
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    assert!(internal_repo(&fx).join(".git").exists());
    assert!(Path::new(&a.workdir)
        .parent()
        .unwrap()
        .join("internal")
        .is_dir());
}

#[tokio::test]
async fn task_name_skips_existing_internal_branch() {
    let fx = fleet_fixture().await;
    fx.svc.internal().ensure(PRODUCT).await.unwrap();
    git(&internal_repo(&fx), &["branch", "task/x", "main"]);
    let mut req = orchestrator(&fx, "direct");
    req.slug = Some("x".into());
    let a = fx.svc.spawn(req).await.unwrap();
    assert_eq!(task_name_of(&a), "x-2");
}

#[tokio::test]
async fn failed_orchestrator_spawn_leaves_no_internal_branch() {
    // tmux refuses the socket name, so the spawn fails at launch, after the worktree and the stored card.
    let fx = fixture_with(fake_claude(), Some("x".repeat(300))).await;
    ready_for_fleet(&fx).await;
    assert!(fx.svc.spawn(orchestrator(&fx, "direct")).await.is_err());
    assert!(entries(&fx.tasks_root).is_empty());
    let repo = internal_repo(&fx);
    assert_eq!(git(&repo, &["branch", "--list", "task/*"]), "");
    assert_eq!(
        git(&repo, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[tokio::test]
async fn restart_recreates_missing_internal_worktree() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let name = task_name_of(&a);
    let internal = Path::new(&a.workdir).parent().unwrap().join("internal");
    std::fs::write(internal.join("note.md"), "x").unwrap();
    git(&internal, &["add", "note.md"]);
    git(&internal, &["commit", "-q", "-m", "note"]);
    crash(&fx, &a.id).await;
    std::fs::remove_dir_all(&internal).unwrap();

    let b = fx.svc.restart_orchestrator(&a.id).await.unwrap();
    assert_eq!(
        git(&internal, &["branch", "--show-current"]),
        format!("task/{name}")
    );
    assert!(internal.join("note.md").is_file());
    let settings = std::fs::read_to_string(
        fx.state_dir
            .join("agents")
            .join(&b.id.0)
            .join("settings.json"),
    )
    .unwrap();
    assert!(
        settings.contains(&internal.to_string_lossy().into_owned()),
        "{settings}"
    );
}

#[tokio::test]
async fn archive_removes_internal_worktree_keeps_branch() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let name = task_name_of(&a);
    let task = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    let internal = task.join("internal");
    std::fs::write(internal.join("note.md"), "x").unwrap();
    git(&internal, &["add", "note.md"]);
    git(&internal, &["commit", "-q", "-m", "note"]);
    // A snapshot a specialist would have been given.
    let snap = task.join("internal-snapshots").join("1-api");
    fx.svc
        .internal()
        .snapshot(PRODUCT, &name, &snap)
        .await
        .unwrap();
    assert!(snap.join("note.md").is_file());
    fx.svc.stop(&a.id).await.unwrap();

    let plan = fx.svc.archive_plan(&a.id).await.unwrap();
    fx.svc.archive(&a.id, &plan.items).await.unwrap();

    assert!(
        !task.exists(),
        "{:?}",
        std::fs::read_dir(&task).map(|d| d.count())
    );
    let repo = internal_repo(&fx);
    assert_eq!(
        git(&repo, &["branch", "--list", &format!("task/{name}")]).trim(),
        format!("task/{name}")
    );
    assert_eq!(git(&repo, &["show", &format!("task/{name}:note.md")]), "x");
    assert!(!git(&repo, &["worktree", "list"]).contains("internal-snapshots"));
    assert_eq!(
        git(&repo, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[tokio::test]
async fn footprint_counts_internal_as_one_item() {
    let fx = fleet_fixture().await;
    let a = fx.svc.spawn(orchestrator(&fx, "direct")).await.unwrap();
    let task = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    let snap = task.join("internal-snapshots").join("1-api");
    fx.svc
        .internal()
        .snapshot(PRODUCT, &task_name_of(&a), &snap)
        .await
        .unwrap();
    let fp = fleet_daemon::footprint::measure(&task, &[], &fx.state_dir.join("agents"), 0);
    let items: Vec<_> = fp.items.iter().filter(|i| i.label == "internal").collect();
    assert_eq!(items.len(), 1, "{:?}", fp.items);
    assert!(items[0].bytes > 0);
    assert_eq!(items[0].path, task.join("internal").to_string_lossy());
    // Weighed once: the item is both folders and nothing of them is in "other files", which is the orchestrator's
    // folder alone.
    let dir = fleet_daemon::footprint::dir_bytes;
    assert_eq!(
        items[0].bytes,
        dir(&task.join("internal")) + dir(&task.join("internal-snapshots"))
    );
    let other = fp.items.iter().find(|i| i.label == "other files").unwrap();
    assert_eq!(other.bytes, dir(&task.join("orchestrator")));
}

/// Commits `file` with `body` in the task's internal worktree and returns the commit.
fn commit_note(task: &Path, file: &str, body: &str) -> String {
    let internal = task.join("internal");
    let path = internal.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    git(&internal, &["add", file]);
    git(&internal, &["commit", "-q", "-m", "note"]);
    git(&internal, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn specialist_gets_internal_snapshot() {
    let fx = fleet_fixture().await;
    let (a, task, name) = approved_task(&fx).await;
    let sha = commit_note(&task, &format!("tasks/{name}/plan.md"), "the plan");

    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let snap = task.join("internal-snapshots").join("1-api");
    assert_eq!(
        std::fs::read_to_string(snap.join("tasks").join(&name).join("plan.md")).unwrap(),
        "the plan"
    );
    assert_eq!(git(&snap, &["rev-parse", "HEAD"]), sha);
    assert_eq!(git(&snap, &["branch", "--show-current"]), "");

    let md = std::fs::read_to_string(Path::new(&s.workdir).join("CLAUDE.md")).unwrap();
    let section = format!(
        "## Internal resources\n\n\
`{}` is a read-only copy of this task's private notes (the design, the plan, decisions) as of commit\n\
`{sha}`. Read what your brief points you to here. Do not edit it, and never copy anything from it into a repo or\n\
a pull request.\n",
        snap.display()
    );
    assert!(md.contains(&section), "{md}");

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
    // Claude Code matches absolute paths with a leading `//`, and only Edit rules: Write(path) rules never match.
    let rule = format!("Edit(/{}/**)", snap.display());
    assert!(rule.starts_with("Edit(//"), "{rule}");
    let deny = settings["permissions"]["deny"].as_array().unwrap();
    assert!(deny.contains(&json!(rule)), "{rule} not in {deny:?}");
    let dirs = settings["permissions"]["additionalDirectories"]
        .as_array()
        .unwrap();
    assert_eq!(dirs, &[json!(snap.to_string_lossy())]);
    // The folder holds only what it did: the snapshot lives beside it, never inside the specialist's cwd.
    assert_eq!(
        entries(Path::new(&s.workdir)),
        [".claude", "CLAUDE.md", PROMPT_FILE, "core"].map(String::from)
    );
}

#[tokio::test]
async fn spawn_refused_while_internal_dirty() {
    let fx = fleet_fixture().await;
    let (a, task, name) = approved_task(&fx).await;
    let internal = task.join("internal");
    std::fs::write(internal.join("draft.md"), "wip").unwrap();
    let err = fx.svc.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert_eq!(
        fleet_refusal(&err),
        format!(
            "commit your internal resources first: git -C {} commit",
            internal.display()
        )
    );
    assert_no_specialist(&fx, &task, &name).await;
    assert!(!task.join("internal-snapshots").exists());
}

#[tokio::test]
async fn snapshot_unchanged_by_later_orchestrator_commits() {
    let fx = fleet_fixture().await;
    let (a, task, name) = approved_task(&fx).await;
    let sha = commit_note(&task, &format!("tasks/{name}/plan.md"), "v1");
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    let later = commit_note(&task, &format!("tasks/{name}/plan.md"), "v2");
    assert_ne!(sha, later);
    let snap = task.join("internal-snapshots").join("1-api");
    assert_eq!(git(&snap, &["rev-parse", "HEAD"]), sha);
    assert_eq!(
        std::fs::read_to_string(snap.join("tasks").join(&name).join("plan.md")).unwrap(),
        "v1"
    );
}

#[tokio::test]
async fn failed_specialist_start_removes_snapshot() {
    let fx = fleet_fixture().await;
    let (a, task, _) = approved_task(&fx).await;
    // A tmux whose socket path is too long: the launch fails after the snapshot was made.
    let broken = Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fleet_daemon::tmux::Tmux::new(Some("x".repeat(300))),
        fx.svc.options().clone(),
    ));
    broken.spawn_specialist(&a.id, 1).await.unwrap_err();
    assert!(!task.join("internal-snapshots").exists());
    assert_eq!(
        git(&internal_repo(&fx), &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        // The repo itself and the orchestrator's checkout.
        2
    );
    // Nothing is left that would stop the step from starting.
    fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
}

#[tokio::test]
async fn a_task_without_internal_notes_spawns_without_a_snapshot() {
    let fx = fleet_fixture().await;
    let (a, task, _) = approved_task(&fx).await;
    // A task made before the feature has no internal folder.
    let internal = task.join("internal");
    git(
        &internal_repo(&fx),
        &["worktree", "remove", "--force", &internal.to_string_lossy()],
    );
    let s = fx.svc.spawn_specialist(&a.id, 1).await.unwrap();
    assert!(!task.join("internal-snapshots").exists());
    let md = std::fs::read_to_string(Path::new(&s.workdir).join("CLAUDE.md")).unwrap();
    assert!(!md.contains("Internal resources"), "{md}");
}

#[tokio::test]
async fn open_pr_merges_internal_notes() {
    let fx = fleet_fixture().await;
    let (a, wt, branch, _) = done_task(&fx, &one_step("true")).await;
    let task = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    let file = "tasks/note/plan.md";
    commit_note(&task, file, "the plan");
    fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    assert_eq!(
        git(&internal_repo(&fx), &["show", &format!("main:{file}")]),
        "the plan"
    );
    assert_eq!(
        log_lines(&fx, &a.id, "internal notes").await,
        ["internal notes merged into the product's notes"]
    );
    // Nothing of the internal repo reaches the task branch or what the PR changes.
    let changed = git(&wt, &["diff", "--name-only", &format!("main...{branch}")]);
    assert_eq!(changed, "src/lib.rs");
    assert!(!wt.join(file).exists());
    let p = fx.store.task(&a.id).await.unwrap().unwrap();
    let gate_files = format!("{:?}", p.gate_runs);
    assert!(!gate_files.contains("plan.md"), "{gate_files}");
    assert!(!gh_calls(&fx).await.contains("plan.md"));
}

#[tokio::test]
async fn open_pr_without_internal_changes_logs_nothing() {
    let fx = fleet_fixture().await;
    let (a, _, _, _) = done_task(&fx, &one_step("true")).await;
    fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    assert!(log_lines(&fx, &a.id, "internal notes").await.is_empty());
}

#[tokio::test]
async fn open_pr_without_internal_folder_logs_nothing() {
    let fx = fleet_fixture().await;
    let (a, _, _, _) = done_task(&fx, &one_step("true")).await;
    let task = Path::new(&a.workdir).parent().unwrap();
    git(
        &internal_repo(&fx),
        &[
            "worktree",
            "remove",
            "--force",
            &task.join("internal").to_string_lossy(),
        ],
    );
    let url = fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    assert_eq!(url, "https://example.test/pr/1");
    assert!(log_lines(&fx, &a.id, "internal notes").await.is_empty());
}

#[tokio::test]
async fn open_pr_reports_internal_conflict() {
    let fx = fleet_fixture().await;
    let (a, _, _, _) = done_task(&fx, &one_step("true")).await;
    let task_a = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    let (b, _, _) = approved_task(&fx).await;
    finish_steps(&fx, &b).await;
    commit_lib(&b, "src/two.rs");
    let task_b = Path::new(&b.workdir).parent().unwrap().to_path_buf();
    commit_note(&task_a, "shared.md", "from one");
    commit_note(&task_b, "shared.md", "from two");

    fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    fx.svc.open_pr(&b.id, "t", "b").await.unwrap();
    assert_eq!(
        log_lines(&fx, &a.id, "internal notes").await,
        ["internal notes merged into the product's notes"]
    );
    assert_eq!(
        log_lines(&fx, &b.id, "internal notes").await,
        ["internal notes not merged: conflict in shared.md; the next pull request retries"]
    );
    let repo = internal_repo(&fx);
    assert_eq!(git(&repo, &["show", "main:shared.md"]), "from one");
    assert_eq!(git(&repo, &["status", "--short"]), "");
}

#[tokio::test]
async fn second_open_pr_merges_later_notes() {
    let fx = fleet_fixture().await;
    let (a, wt, _, _) = done_task(&fx, &one_step("true")).await;
    let task = Path::new(&a.workdir).parent().unwrap().to_path_buf();
    commit_note(&task, "one.md", "1");
    fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    commit_note(&task, "two.md", "2");
    std::fs::write(wt.join("src/more.rs"), "").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "more"]);
    fx.svc.open_pr(&a.id, "t", "b").await.unwrap();
    let repo = internal_repo(&fx);
    assert_eq!(git(&repo, &["show", "main:one.md"]), "1");
    assert_eq!(git(&repo, &["show", "main:two.md"]), "2");
    assert_eq!(log_lines(&fx, &a.id, "internal notes").await.len(), 2);
}
