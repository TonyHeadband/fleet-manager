//! fleet-client against the real daemon router (the daemon crate owns this test so the dependency points one way).
mod common;

use common::net::http;
use common::*;
use fleet_api::{
    default_ui_schema, ActionDef, Actor, Agent, AgentId, AgentKind, DefKind, NewProduct,
    NewRelease, ProductOverride, ProductUpdate, ReleaseUpdate, RepoUpsert, SettingSource,
    SpawnRequest,
};
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
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
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

/// A solo task in the fixture's product, under its persona.
fn solo(fx: &Fx, repos: &[&str], prompt: &str) -> SpawnRequest {
    SpawnRequest {
        actor: Actor("local".into()),
        product: PRODUCT.into(),
        release: None,
        repos: repos.iter().map(|r| pick(fx, r)).collect(),
        prompt: prompt.into(),
        slug: None,
        persona: PERSONA.into(),
        kind: AgentKind::Solo,
        role: None,
    }
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
    assert_eq!(blocking!(c, c.repos_for(PRODUCT)).unwrap().len(), 1);
    let products = blocking!(c, c.products()).unwrap();
    assert_eq!(
        products.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        [PRODUCT]
    );
    assert!(blocking!(c, c.repos_for("nobody")).unwrap().is_empty());
    assert_eq!(blocking!(c, c.ui_schema()).unwrap(), default_ui_schema());

    let agent: Agent = blocking!(c, c.spawn(&solo(&fx, &["core"], "fix it")))
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
    let err = blocking!(c, c.spawn(&solo(&fx, &["nope"], "x"))).unwrap_err();
    assert!(err.message.contains("nope"), "{err}");
    let ghost = common::agent_fixture(&fx);
    let stop = action("stop");
    let err = blocking!(c, c.run(&stop, &ghost, None)).unwrap_err();
    assert!(err.message.contains("no such agent"), "{err}");
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
    let (url, fx) = start().await;
    let (tx, rx) = mpsc::channel();
    let _h = spawn_event_stream(&url, tx, wake());
    let next = || {
        rx.recv_timeout(Duration::from_secs(10))
            .expect("no message in 10s")
    };
    assert!(matches!(next(), StreamMsg::Connected));
    assert!(matches!(next(), StreamMsg::Snapshot(s) if s.seq == 0));
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&solo(&fx, &["core"], "x")))
        .unwrap()
        .agent;
    assert!(
        matches!(next(), StreamMsg::Event(e) if matches!(&e.kind, fleet_api::EventKind::AgentCreated { agent: a } if a.id == agent.id))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pane_stream_shows_the_agents_terminal() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&solo(&fx, &["core"], "x")))
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
    let agent = blocking!(c, c.spawn(&solo(&fx, &["core"], "x")))
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
async fn client_spawns_a_task_with_no_repos() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&solo(&fx, &[], "think first")))
        .unwrap()
        .agent;
    assert!(agent.workspaces.is_empty());
    assert_eq!(blocking!(c, c.snapshot()).unwrap().agents.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn client_adds_and_lists_repos() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&solo(&fx, &[], "x"))).unwrap().agent;
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
    assert!(err.message.contains("no such agent"), "{err}");
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
    let agent = blocking!(c, c.spawn(&solo(&fx, &[], "x"))).unwrap().agent;
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
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let first = blocking!(c, c.spawn(&solo(&fx, &["core"], "x")))
        .unwrap()
        .agent;
    let (a, stop) = (first.clone(), action("stop"));
    assert_eq!(blocking!(c, c.run(&stop, &a, None)).unwrap(), None);

    let second = blocking!(c, c.spawn(&solo(&fx, &["core"], "y")))
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
    let agent = blocking!(c, c.spawn(&solo(&fx, &["core"], "x")))
        .unwrap()
        .agent;
    // A file the agent did not create keeps the task folder; the report must say so.
    std::fs::write(PathBuf::from(&agent.workdir).join("notes.txt"), "mine").unwrap();
    let id = agent.id.0.clone();
    let nothing = fleet_api::ArchivePlan {
        items: vec![],
        total: 0,
    };
    let err = blocking!(c, c.archive(&id, &nothing)).unwrap_err();
    assert!(err.message.contains("stop it first"), "{err}");
    blocking!(c, c.stop(&id)).unwrap();
    let plan = blocking!(c, c.archive_plan(&id)).unwrap();
    assert!(plan.items.last().unwrap().stays);
    // Work done after the plan was confirmed: the daemon removes nothing and says the plan is stale.
    let wt = PathBuf::from(&agent.workspaces[0].worktree);
    std::fs::write(wt.join("late.txt"), "new").unwrap();
    let err = blocking!(c, c.archive(&id, &plan)).unwrap_err();
    assert_eq!(err.message, fleet_api::PLAN_CHANGED);
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

    let list = blocking!(c, c.tasks()).unwrap();
    assert_eq!(list.tasks.len(), 1);
    assert_eq!(list.tasks[0].state, fleet_api::TaskState::Archived);
    drop(fx);
}

#[tokio::test(flavor = "multi_thread")]
async fn client_orphan_calls_round_trip() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let folder = fx.tasks_root.join("lost");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("CLAUDE.md"),
        format!("{}\nmore\n", fleet_daemon::agents::CLAUDE_MD_HEADER),
    )
    .unwrap();
    blocking!(c, c.refresh_tasks()).unwrap();
    let want = format!("task_folder:{}", folder.display());
    let c2 = c.clone();
    assert!(
        wait_until(|| {
            let c = c2.clone();
            let want = want.clone();
            async move {
                tokio::task::block_in_place(|| c.tasks())
                    .map(|l| l.orphans.iter().any(|o| o.id == want))
                    .unwrap_or(false)
            }
        })
        .await
    );
    let plan = blocking!(c, c.orphan_plan(&want)).unwrap();
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
        nicknames: None,
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
    assert_eq!(list.gh_command, env!("CARGO_BIN_EXE_fake-gh"));
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
    assert_eq!(blocking!(c, c.definitions()).unwrap().len(), 17);
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
async fn client_saves_hidden_specialists() {
    let (url, _fx) = start().await;
    let c = FleetClient::new(&url);
    let hidden = vec!["reviewer".to_string(), "docs".into(), "reviewer".into()];
    let p = blocking!(c, c.put_hidden_specialists(PRODUCT, hidden.clone())).unwrap();
    assert_eq!(p.hidden_specialists, ["reviewer", "docs"]);
    let bad = vec!["ghost".to_string()];
    let err = blocking!(c, c.put_hidden_specialists(PRODUCT, bad.clone())).unwrap_err();
    assert!(err.to_string().contains("ghost"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn client_manages_products_releases_and_repos() {
    let (url, fx) = start().await;
    let c = FleetClient::new(&url);
    let p = blocking!(
        c,
        c.create_product(&NewProduct {
            name: "Shop".into(),
            default_persona: Some(PERSONA.into()),
            internal_url: None,
        })
    )
    .unwrap();
    assert_eq!(p.id, "shop");
    let p = blocking!(
        c,
        c.update_product(
            "shop",
            &ProductUpdate {
                default_persona: Some(None),
                internal_url: None,
                ..Default::default()
            }
        )
    )
    .unwrap();
    assert_eq!(p.default_persona, None);

    let repo = RepoUpsert {
        name: "api".into(),
        path: None,
        url: Some("https://github.com/o/api.git".into()),
    };
    let info = blocking!(c, c.upsert_repo("shop", &repo)).unwrap();
    assert_eq!(info.url.as_deref(), repo.url.as_deref());
    assert_eq!(blocking!(c, c.repos_for("shop")).unwrap().len(), 1);

    let r = blocking!(
        c,
        c.create_release(
            "shop",
            &NewRelease {
                version: "1.0".into(),
                start: "2026-11-01".into(),
                target: "2026-12-01".into(),
            }
        )
    )
    .unwrap();
    let r = blocking!(
        c,
        c.update_release(
            r.id,
            &ReleaseUpdate {
                released: Some(true),
                ..Default::default()
            }
        )
    )
    .unwrap();
    assert!(r.released_at.is_some());

    // The fixture's product has the task, so a release of `shop` is another product's.
    let a = fx.svc.spawn(solo(&fx, &["core"], "work")).await.unwrap();
    let err = blocking!(c, c.set_task_release(&a.id.0, Some(r.id))).unwrap_err();
    assert!(err.to_string().contains("not a release"), "{err}");
    let t = blocking!(c, c.set_task_release(&a.id.0, None)).unwrap();
    assert_eq!(t.release, None);

    blocking!(c, c.delete_release(r.id)).unwrap();
    blocking!(c, c.delete_repo("shop", "api")).unwrap();
    blocking!(c, c.delete_product("shop")).unwrap();
    assert!(blocking!(c, c.delete_product(PRODUCT)).is_err());
    assert_eq!(blocking!(c, c.products()).unwrap().len(), 1);
    fx.svc.stop(&a.id).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn client_puts_overrides_and_reads_effective() {
    let (url, _fx) = start().await;
    let c = FleetClient::new(&url);
    let entry = |section: &str, key: &str, value: &str| ProductOverride {
        section: section.into(),
        key: key.into(),
        value: value.into(),
        error: None,
    };
    let p = blocking!(
        c,
        c.put_product_overrides(
            PRODUCT,
            vec![
                entry("limits", "max_steps", "30"),
                entry("models", "rust", "opus")
            ]
        )
    )
    .unwrap();
    assert_eq!(p.overrides.len(), 2);

    let e = blocking!(c, c.effective_settings(Some(PRODUCT))).unwrap();
    let steps = e.values.iter().find(|v| v.key == "max_steps").unwrap();
    assert_eq!(
        (steps.value.as_deref(), steps.source),
        (Some("30"), SettingSource::Product)
    );
    let bare = blocking!(c, c.effective_settings(None)).unwrap();
    let steps = bare.values.iter().find(|v| v.key == "max_steps").unwrap();
    assert_eq!(steps.source, SettingSource::Default);

    let err = blocking!(
        c,
        c.put_product_overrides(PRODUCT, vec![entry("limits", "max_steps", "0")])
    )
    .unwrap_err();
    assert!(err.to_string().contains("limits.max_steps"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn client_reads_internal_resources() {
    let (base, fx) = start().await;
    let dir = fx.svc.internal().ensure(PRODUCT).await.unwrap();
    std::fs::create_dir_all(dir.join("notes")).unwrap();
    std::fs::write(dir.join("notes/a b.md"), "alpha").unwrap();
    for args in [
        vec!["add", "."],
        vec![
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@localhost",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "add note",
        ],
    ] {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let c = FleetClient::new(&base);
    blocking!(c, {
        assert_eq!(c.internal_branches(PRODUCT).unwrap().branches, ["main"]);
        let mut want: Vec<&str> = fleet_daemon::internal::SKELETON
            .iter()
            .map(|(path, _)| *path)
            .chain(["notes/a b.md"])
            .collect();
        want.sort();
        assert_eq!(c.internal_files(PRODUCT, "main").unwrap(), want);
        assert_eq!(
            c.internal_file(PRODUCT, "main", "notes/a b.md").unwrap(),
            "alpha"
        );
        let log = c
            .internal_history(PRODUCT, "main", Some("notes/a b.md"))
            .unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].subject, "add note");
        assert_eq!(c.internal_history(PRODUCT, "main", None).unwrap().len(), 2);
        assert!(c.internal_files("nope", "main").is_err());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn fleet_attach_connects_and_gets_the_screen_first() {
    let (url, fx) = start().await;
    let a = spawn_ready(&fx).await;
    let first = blocking!(c, {
        let mut ws = fleet_client::attach::connect(&url, &a.id.0).unwrap();
        loop {
            match ws.read().unwrap() {
                tokio_tungstenite::tungstenite::Message::Binary(b) => break b,
                tokio_tungstenite::tungstenite::Message::Text(t) => {
                    panic!("text before the screen: {t}")
                }
                _ => {}
            }
        }
    });
    let mut screen = vt100::Parser::new(
        std::num::NonZeroU16::new(40).unwrap(),
        std::num::NonZeroU16::new(120).unwrap(),
        0,
    );
    screen.process(&first);
    assert!(
        screen.screen().contents().contains("ready"),
        "{}",
        screen.screen().contents()
    );

    let err = blocking!(c, fleet_client::attach::connect(&url, "nope"))
        .err()
        .expect("attaching to no agent fails");
    assert!(err.contains("no such agent"), "{err}");
    assert!(
        !err.contains('{'),
        "the daemon's message, not its JSON: {err}"
    );
}
