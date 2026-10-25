//! fleet-client against the real daemon router (the daemon crate owns this test so the dependency points one way).
mod common;

use common::net::http;
use common::*;
use fleet_api::{default_ui_schema, ActionDef, Agent, AgentId, AgentKind, DefKind, RepoFleet};
use fleet_client::http::FleetClient;
use fleet_client::stream::{spawn_event_stream, spawn_pane_stream, PaneMsg, StreamMsg};
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;

async fn start() -> (String, Fx) {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(
        fx.svc.clone(),
        fx.store.clone(),
        fx.tmux.clone(),
        addr.port(),
    );
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (format!("http://{addr}"), fx)
}

/// The client is blocking by design (it runs on background threads in the GUI), so tests call it from a
/// blocking section of the multi-thread runtime.
macro_rules! blocking {
    ($c:ident, $body:expr) => {{
        tokio::task::block_in_place(|| $body)
    }};
}

fn action(id: &str) -> ActionDef {
    default_ui_schema()
        .actions
        .into_iter()
        .find(|a| a.id == id)
        .unwrap()
}

fn wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

#[tokio::test(flavor = "multi_thread")]
async fn client_round_trips_against_the_real_daemon() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let names: Vec<String> = blocking!(c, c.repos())
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(names, ["core"]);
    assert_eq!(blocking!(c, c.repos_for(PERSONA)).unwrap().len(), 1);
    assert!(blocking!(c, c.repos_for("nobody")).unwrap().is_empty());
    assert_eq!(blocking!(c, c.ui_schema()).unwrap(), default_ui_schema());

    let agent: Agent = blocking!(
        c,
        c.spawn(
            PERSONA,
            &["core".to_string()],
            "fix it",
            AgentKind::Solo,
            None,
            &[]
        )
    )
    .unwrap()
    .agent;
    assert_eq!(blocking!(c, c.snapshot()).unwrap().agents.len(), 1);
    assert!(pane_contains(&fx, &agent.id, "ready").await);

    let (a, ans) = (agent.clone(), action("answer"));
    blocking!(c, c.run(&ans, &a, Some("$(x) \"q\" {a}"))).unwrap();
    assert!(
        pane_contains(&fx, &agent.id, "{a}").await,
        "answer text did not reach the agent"
    );

    let (a, pause, resume) = (agent.clone(), action("pause"), action("resume"));
    blocking!(c, c.run(&pause, &a, None)).unwrap();
    assert!(common::agent(&fx, &agent.id).await.paused);
    blocking!(c, c.run(&resume, &a, None)).unwrap();
    assert!(!common::agent(&fx, &agent.id).await.paused);

    std::fs::write(
        PathBuf::from(&agent.workspaces[0].worktree).join("new.txt"),
        "hello\n",
    )
    .unwrap();
    let id = agent.id.0.clone();
    let report = blocking!(c, c.diff(&id)).unwrap();
    assert_eq!(report.workspaces[0].repo, "core");
    assert!(report.workspaces[0].diff.contains("+hello"));

    let (a, stop) = (agent.clone(), action("stop"));
    blocking!(c, c.run(&stop, &a, None)).unwrap();
    assert_eq!(
        blocking!(c, c.snapshot()).unwrap().agents[0].status,
        fleet_api::Status::Stopped
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_carry_the_daemons_message() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let err = blocking!(
        c,
        c.spawn(
            PERSONA,
            &["nope".to_string()],
            "x",
            AgentKind::Solo,
            None,
            &[]
        )
    )
    .unwrap_err();
    assert!(err.0.contains("nope"), "{err}");
    let ghost = common::agent_fixture(&fx);
    let stop = action("stop");
    let err = blocking!(c, c.run(&stop, &ghost, None)).unwrap_err();
    assert!(err.0.contains("no such agent"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_daemon_is_an_error_not_a_panic_or_a_hang() {
    let c = Arc::new(FleetClient::new("http://127.0.0.1:1"));
    let started = std::time::Instant::now();
    assert!(blocking!(c, c.snapshot()).is_err());
    assert!(started.elapsed() < Duration::from_secs(12));
}

#[tokio::test(flavor = "multi_thread")]
async fn event_stream_sends_connected_then_snapshot_then_events() {
    let (url, _fx) = start().await;
    let (tx, rx) = mpsc::channel();
    let _h = spawn_event_stream(&url, tx, wake());
    let next = || {
        rx.recv_timeout(Duration::from_secs(10))
            .expect("no message in 10s")
    };
    assert!(matches!(next(), StreamMsg::Connected));
    assert!(matches!(next(), StreamMsg::Snapshot(s) if s.seq == 0));
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(
        c,
        c.spawn(
            PERSONA,
            &["core".to_string()],
            "x",
            AgentKind::Solo,
            None,
            &[]
        )
    )
    .unwrap()
    .agent;
    assert!(
        matches!(next(), StreamMsg::Event(e) if matches!(&e.kind, fleet_api::EventKind::AgentCreated { agent: a } if a.id == agent.id))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pane_stream_shows_the_agents_terminal() {
    let (url, _fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(
        c,
        c.spawn(
            PERSONA,
            &["core".to_string()],
            "x",
            AgentKind::Solo,
            None,
            &[]
        )
    )
    .unwrap()
    .agent;
    let (tx, rx) = mpsc::channel();
    let _h = spawn_pane_stream(&url, &agent.id.0, tx, wake());
    let mut seen = false;
    for _ in 0..20 {
        if let Ok(PaneMsg::Text(t)) = rx.recv_timeout(Duration::from_secs(10)) {
            if t.contains("ready") {
                seen = true;
                break;
            }
        }
    }
    assert!(seen, "pane text never arrived");
}

#[tokio::test(flavor = "multi_thread")]
async fn pane_stream_of_a_stopped_agent_ends_without_reconnecting() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(
        c,
        c.spawn(
            PERSONA,
            &["core".to_string()],
            "x",
            AgentKind::Solo,
            None,
            &[]
        )
    )
    .unwrap()
    .agent;
    assert!(pane_contains(&fx, &agent.id, "ready").await);
    fx.svc.stop(&agent.id).await.unwrap();
    let (tx, rx) = mpsc::channel();
    let _h = spawn_pane_stream(&url, &agent.id.0, tx, wake());
    let next = || rx.recv_timeout(Duration::from_secs(10));
    assert!(matches!(next(), Ok(PaneMsg::Text(t)) if t.contains("ready")));
    assert!(matches!(next(), Ok(PaneMsg::Closed)));
    // The stream's thread ends on `closed`, dropping its sender; a reconnecting one would send the saved screen
    // again after its 1 s backoff instead.
    assert!(
        matches!(
            rx.recv_timeout(Duration::from_millis(2500)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ),
        "the stream kept going after closed"
    );
}

#[allow(dead_code)]
fn _unused(_: AgentId) {}

#[tokio::test(flavor = "multi_thread")]
async fn client_spawns_a_project_with_no_repos() {
    let (url, _fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(
        c,
        c.spawn(PERSONA, &[], "think first", AgentKind::Solo, None, &[])
    )
    .unwrap()
    .agent;
    assert!(agent.workspaces.is_empty());
    assert_eq!(blocking!(c, c.snapshot()).unwrap().agents.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn client_adds_and_lists_repos() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(PERSONA, &[], "x", AgentKind::Solo, None, &[]))
        .unwrap()
        .agent;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let path = tools.to_str().unwrap().to_string();
    let id = agent.id.0.clone();
    let ws = blocking!(c, c.add_repo(&id, &path)).unwrap();
    assert_eq!(ws.repo, "tools");
    let names: Vec<String> = blocking!(c, c.repos())
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert!(
        names.contains(&"tools".to_string()) && names.contains(&"core".to_string()),
        "{names:?}"
    );
    let err = blocking!(c, c.add_repo("ghost", &path)).unwrap_err();
    assert!(err.0.contains("no such agent"), "{err}");
}

/// Holds an agent's `request_repo` call open in the background, as Claude Code does.
fn request_repo_held(
    url: &str,
    id: &str,
    token: &str,
    repo: &str,
) -> tokio::task::JoinHandle<Value> {
    let body = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": "request_repo", "arguments": {"repo": repo, "why": "need it"}}});
    let addr: std::net::SocketAddr = url.trim_start_matches("http://").parse().unwrap();
    let (path, token) = (format!("/internal/mcp/{id}"), token.to_string());
    tokio::spawn(async move {
        let (status, text) = http(
            addr,
            "POST",
            &path,
            &[("X-Fleet-Token", &token)],
            Some(&body),
        )
        .await;
        assert_eq!(status, 200, "{text}");
        serde_json::from_str(&text).unwrap()
    })
}

async fn wait_pending(fx: &Fx, id: &AgentId) {
    assert!(
        wait_until(|| async { common::agent(fx, id).await.repo_request.is_some() }).await,
        "request never became pending"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn client_grant_and_refuse_round_trip() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(PERSONA, &[], "x", AgentKind::Solo, None, &[]))
        .unwrap()
        .agent;
    let token = fx.store.agent_token(&agent.id).await.unwrap().unwrap();
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let path = tools.to_str().unwrap().to_string();
    let id = agent.id.0.clone();

    let held = request_repo_held(&url, &id, &token, &path);
    wait_pending(&fx, &agent.id).await;
    let ws = blocking!(c, c.grant_repo(&id, &path)).unwrap();
    assert_eq!(ws.repo, "tools");
    let reply = held.await.unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    // Nothing pending any more.
    assert!(blocking!(c, c.grant_repo(&id, &path)).is_err());

    let other = make_repo(&fx._tmp.path().join("other"));
    let other = other.to_str().unwrap().to_string();
    let held = request_repo_held(&url, &id, &token, &other);
    wait_pending(&fx, &agent.id).await;
    blocking!(c, c.refuse_repo(&id, "not this one")).unwrap();
    let reply = held.await.unwrap();
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not this one"),
        "{reply}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn client_stop_and_the_stop_action_both_stop_the_agent() {
    let (url, _fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let core = ["core".to_string()];
    let first = blocking!(c, c.spawn(PERSONA, &core, "x", AgentKind::Solo, None, &[]))
        .unwrap()
        .agent;
    let (a, stop) = (first.clone(), action("stop"));
    assert_eq!(blocking!(c, c.run(&stop, &a, None)).unwrap(), None);

    let second = blocking!(c, c.spawn(PERSONA, &core, "y", AgentKind::Solo, None, &[]))
        .unwrap()
        .agent;
    let id = second.id.0.clone();
    blocking!(c, c.stop(&id)).unwrap();
    let agents = blocking!(c, c.snapshot()).unwrap().agents;
    assert_eq!(agents.len(), 2, "stopping keeps the card");
    assert!(agents
        .iter()
        .all(|a| a.status == fleet_api::Status::Stopped));
}

#[tokio::test(flavor = "multi_thread")]
async fn client_archive_reports_what_was_kept() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let core = ["core".to_string()];
    let agent = blocking!(c, c.spawn(PERSONA, &core, "x", AgentKind::Solo, None, &[]))
        .unwrap()
        .agent;
    // A file the agent did not create keeps the project folder; the report must say so.
    std::fs::write(PathBuf::from(&agent.workdir).join("notes.txt"), "mine").unwrap();
    let id = agent.id.0.clone();
    let nothing = fleet_api::ArchivePlan {
        items: vec![],
        total: 0,
    };
    let err = blocking!(c, c.archive(&id, &nothing)).unwrap_err();
    assert!(err.0.contains("stop it first"), "{err}");
    blocking!(c, c.stop(&id)).unwrap();
    let plan = blocking!(c, c.archive_plan(&id)).unwrap();
    assert!(plan.items.last().unwrap().stays);
    // Work done after the plan was confirmed: the daemon removes nothing and says the plan is stale.
    let wt = PathBuf::from(&agent.workspaces[0].worktree);
    std::fs::write(wt.join("late.txt"), "new").unwrap();
    let err = blocking!(c, c.archive(&id, &plan)).unwrap_err();
    assert_eq!(err.0, fleet_api::PLAN_CHANGED);
    assert!(wt.join("late.txt").is_file());
    std::fs::remove_file(wt.join("late.txt")).unwrap();
    let kept = blocking!(c, c.archive(&id, &plan))
        .unwrap()
        .kept
        .expect("kept message");
    assert!(
        kept.contains("kept") && kept.contains("notes.txt"),
        "{kept}"
    );

    let list = blocking!(c, c.projects()).unwrap();
    assert_eq!(list.projects.len(), 1);
    assert_eq!(list.projects[0].state, fleet_api::ProjectState::Archived);
    drop(fx);
}

#[tokio::test(flavor = "multi_thread")]
async fn client_orphan_calls_round_trip() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let folder = fx.projects_root.join("lost");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("CLAUDE.md"),
        format!("{}\nmore\n", fleet_daemon::agents::CLAUDE_MD_HEADER),
    )
    .unwrap();
    fx.tmux
        .new_session(
            "fleet-stray",
            &fx.state_dir,
            &[],
            &["sleep".into(), "60".into()],
        )
        .await
        .unwrap();
    blocking!(c, c.refresh_projects()).unwrap();
    let want = format!("project_folder:{}", folder.display());
    let c2 = c.clone();
    assert!(
        wait_until(|| {
            let c = c2.clone();
            let want = want.clone();
            async move {
                tokio::task::block_in_place(|| c.projects())
                    .map(|l| {
                        l.orphans.iter().any(|o| o.id == want)
                            && l.orphans.iter().any(|o| o.id == "session:fleet-stray")
                    })
                    .unwrap_or(false)
            }
        })
        .await
    );
    let plan = blocking!(c, c.orphan_plan(&want)).unwrap();
    blocking!(c, c.end_session("session:fleet-stray")).unwrap();
    assert!(!fx.tmux.has_session("fleet-stray").await.unwrap());
    blocking!(c, c.archive_orphan(&want, &plan)).unwrap();
    assert!(!folder.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn personas_round_trip() {
    use fleet_api::{GithubAuth, NewPersona, PersonaUpdate};
    let (url, _fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let new = NewPersona {
        name: "Home".into(),
        host: "github.com".into(),
        commit_name: "T".into(),
        commit_email: "t@e".into(),
        github_auth: GithubAuth::Gh,
    };
    let p = blocking!(c, c.create_persona(&new)).unwrap();
    assert_eq!((p.id.as_str(), p.ready), ("home", false));
    let up = PersonaUpdate {
        name: Some("Renamed".into()),
        commit_name: None,
        commit_email: None,
        github_auth: None,
    };
    assert_eq!(
        blocking!(c, c.update_persona("home", &up)).unwrap().name,
        "Renamed"
    );
    let checked = blocking!(c, c.check_persona("home")).unwrap();
    assert!(!checked.claude.unwrap().ok);
    let list = blocking!(c, c.personas()).unwrap();
    // The fixture's own persona and this one.
    assert_eq!(list.personas.len(), 2);
    assert!(list.gh_command.ends_with("tests/bin/gh"));
    blocking!(c, c.delete_persona("home")).unwrap();
    let ids: Vec<String> = blocking!(c, c.personas())
        .unwrap()
        .personas
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, [PERSONA]);
    assert!(blocking!(c, c.delete_persona("home")).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn client_saves_and_reads_the_agent_config() {
    let (url, _fx) = start().await;
    let c = FleetClient::new(&url);
    assert_eq!(blocking!(c, c.agent_config()).unwrap(), Default::default());
    let settings = fleet_api::AgentSettings {
        allow: vec!["Read".into()],
        deny: vec![],
        env: [("A".to_string(), "b".to_string())].into(),
    };
    let want = fleet_api::AgentConfig {
        instructions: "hi".into(),
        settings: settings.clone(),
        settings_error: None,
    };
    let update = fleet_api::AgentConfigUpdate {
        instructions: Some("hi".into()),
        settings: Some(settings),
    };
    assert_eq!(blocking!(c, c.save_agent_config(&update)).unwrap(), want);
    assert_eq!(blocking!(c, c.agent_config()).unwrap(), want);
    let mut bad = update.clone();
    bad.settings
        .as_mut()
        .unwrap()
        .env
        .insert("GH_TOKEN".into(), "x".into());
    let err = blocking!(c, c.save_agent_config(&bad)).unwrap_err();
    assert!(err.to_string().contains("GH_TOKEN"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn client_reads_saves_and_resets_definitions() {
    let (url, _fx) = start().await;
    let c = FleetClient::new(&url);
    assert_eq!(blocking!(c, c.definitions()).unwrap().len(), 12);
    let f = blocking!(c, c.definition(DefKind::Orchestrator, "direct")).unwrap();
    assert!(f.builtin && f.text.contains("propose_breakdown"), "{f:?}");

    let text = "---\nname: direct\nsummary: Mine\n---\nWork.\n";
    let info = blocking!(c, c.save_definition(DefKind::Orchestrator, "direct", text)).unwrap();
    assert!(info.overridden && info.summary == "Mine", "{info:?}");
    let f = blocking!(c, c.definition(DefKind::Orchestrator, "direct")).unwrap();
    assert_eq!((f.text.as_str(), f.builtin), (text, false));

    let err = blocking!(
        c,
        c.save_definition(DefKind::Orchestrator, "direct", "---\nname: x\n---\n")
    )
    .unwrap_err();
    assert!(err.to_string().contains("orchestrators/direct.md"), "{err}");

    blocking!(c, c.reset_definition(DefKind::Orchestrator, "direct")).unwrap();
    assert!(
        blocking!(c, c.definition(DefKind::Orchestrator, "direct"))
            .unwrap()
            .builtin
    );
    assert!(blocking!(c, c.reset_definition(DefKind::Orchestrator, "direct")).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn client_saves_a_repo_fleet() {
    let (url, _fx) = start().await;
    let c = FleetClient::new(&url);
    let f = RepoFleet {
        roster: vec!["rust".into()],
        test_command: Some("cargo test".into()),
    };
    let info = blocking!(c, c.set_repo_fleet(PERSONA, "core", &f)).unwrap();
    assert_eq!(
        (info.roster, info.test_command),
        (f.roster.clone(), f.test_command.clone())
    );
    let bad = RepoFleet {
        roster: vec!["ghost".into()],
        test_command: None,
    };
    let err = blocking!(c, c.set_repo_fleet(PERSONA, "core", &bad)).unwrap_err();
    assert!(err.to_string().contains("ghost"), "{err}");
}
