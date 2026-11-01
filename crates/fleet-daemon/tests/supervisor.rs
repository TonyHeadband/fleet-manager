mod common;

use common::net::*;
use common::*;
use fleet_api::{Agent, AgentId, Status};
use fleet_daemon::agents::PROMPT_FILE;
use fleet_daemon::hooks_cfg::rule_path;
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::supervisor::{conversation_folder, folder, Supervisor};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

async fn start(fx: &Fx, text: &str) -> Agent {
    let id = Supervisor::new(fx.svc.clone())
        .new_conversation(PERSONA, text)
        .await
        .unwrap();
    agent(fx, &id).await
}

fn settings(fx: &Fx, id: &AgentId) -> Value {
    let path = fx
        .state_dir
        .join("agents")
        .join(&id.0)
        .join("settings.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn edit_rules(p: &Path) -> [String; 2] {
    let p = rule_path(&p.to_string_lossy());
    [format!("Edit(/{p}/**)"), format!("Write(/{p}/**)")]
}

#[tokio::test]
async fn a_new_conversation_starts_a_supervisor_with_the_text_as_its_prompt() {
    let fx = fixture().await;
    let a = start(&fx, "what is left for the release?").await;
    let screen = wait_screen_in(&fx.sessions, &a.session, |s| s.contains("ready")).await;
    for line in [
        "arg=--permission-mode",
        "arg=auto",
        "arg=--effort",
        "arg=high",
        "arg=--session-id",
    ] {
        assert!(screen.contains(line), "{line} missing:\n{screen}");
    }
    assert!(!screen.contains("arg=--model"), "{screen}");
    let prompt = std::fs::read_to_string(Path::new(&a.workdir).join(PROMPT_FILE)).unwrap();
    assert_eq!(prompt.trim_end(), "what is left for the release?");
    assert_eq!(a.kind, fleet_api::AgentKind::Supervisor);
    assert_eq!(a.product, "");
    assert_eq!(a.persona, PERSONA);
    let rows = fx.store.conversations(PERSONA).await.unwrap();
    assert_eq!(rows.iter().map(|r| &r.id).collect::<Vec<_>>(), [&a.id]);
    let listed = Supervisor::new(fx.svc.clone()).list(PERSONA).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, a.id);
    assert!(listed[0].live);
    assert!(listed[0].tasks.is_empty());

    let solo = spawn_ready(&fx).await;
    let origin = a.id.clone();
    fx.store
        .update_task(&solo.id, |t| t.origin = Some(origin))
        .await
        .unwrap();
    let listed = Supervisor::new(fx.svc.clone()).list(PERSONA).await.unwrap();
    assert_eq!(
        listed[0].tasks,
        [fleet_api::TaskMark {
            id: solo.id.clone(),
            title: "do the thing".into(),
            status: agent(&fx, &solo.id).await.status,
        }]
    );
}

#[tokio::test]
async fn a_second_conversation_writes_its_own_prompt() {
    let fx = fixture().await;
    let first = start(&fx, "first").await;
    let second = start(&fx, "second").await;
    assert_ne!(first.workdir, second.workdir);
    for (a, text) in [(&first, "first"), (&second, "second")] {
        let prompt = std::fs::read_to_string(Path::new(&a.workdir).join(PROMPT_FILE)).unwrap();
        assert_eq!(prompt.trim_end(), text);
    }
    assert_eq!(fx.store.conversations(PERSONA).await.unwrap().len(), 2);
}

#[tokio::test]
async fn the_supervisor_runs_in_its_own_folder_in_the_persona_folder_with_the_persona_config() {
    let fx = fixture().await;
    let a = start(&fx, "hello").await;
    assert_eq!(
        folder(fx.svc.config(), PERSONA),
        fx.state_dir.join("supervisor").join(PERSONA)
    );
    let dir = folder(fx.svc.config(), PERSONA).join(&a.id.0);
    assert_eq!(conversation_folder(fx.svc.config(), PERSONA, &a.id), dir);
    assert_eq!(Path::new(&a.workdir), dir);
    let cwd = fleet_fs::canonical(&dir).unwrap();
    assert!(pane_contains(&fx, &a.id, &format!("cwd={}", cwd.display())).await);
    let claude_dir = fixture_env(&fx).await.claude_dir().unwrap();
    assert!(
        pane_contains(
            &fx,
            &a.id,
            &format!("claude_config={}", claude_dir.display())
        )
        .await
    );
    let md = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
    for needle in ["mcp__fleet__", "write_doc", "set_title", "start"] {
        assert!(md.contains(needle), "{needle} missing from CLAUDE.md");
    }
}

#[tokio::test]
async fn settings_deny_gh_push_and_edits_outside_its_folder() {
    let fx = fixture().await;
    let a = start(&fx, "hello").await;
    let s = settings(&fx, &a.id);
    let deny = strings(&s["permissions"]["deny"]);
    let mut want: Vec<String> = [
        "Bash(gh:*)",
        "Bash(gh *)",
        "Bash(git push:*)",
        "Bash(git * push *)",
        "Bash(git * push)",
    ]
    .map(String::from)
    .to_vec();
    let internal = fx.svc.internal().root().to_path_buf();
    for p in [
        fx.state_dir.join("repos"),
        fx.tasks_root.clone(),
        internal.clone(),
    ] {
        want.extend(edit_rules(&p));
    }
    for rule in &want {
        assert!(deny.contains(rule), "{rule} missing from {deny:?}");
    }
    // Subagents are answered by the PreToolUse hook, not denied outright.
    assert!(
        !deny.iter().any(|r| r == "Agent" || r == "Task"),
        "{deny:?}"
    );
    let allow = strings(&s["permissions"]["allow"]);
    for tool in fleet_daemon::hooks_cfg::SUPERVISOR_TOOLS {
        assert!(
            allow.iter().any(|r| r == tool),
            "{tool} missing from {allow:?}"
        );
    }
    // The subagent tools get a second PreToolUse hook that fails closed; the first, for every tool, stays as for
    // any agent.
    let pre = s["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2, "{pre:?}");
    assert_eq!(pre[0]["matcher"], "");
    assert!(pre[0]["hooks"][0].get("onFailure").is_none(), "{pre:?}");
    assert_eq!(pre[1]["matcher"], "Agent|Task");
    let guard = &pre[1]["hooks"][0];
    assert_eq!(guard["type"], "http");
    assert_eq!(guard["onFailure"], "block");
    assert_eq!(guard["url"], pre[0]["hooks"][0]["url"]);
    let clone = fx.repos.clone_dir(PERSONA, PRODUCT, "core");
    assert_eq!(
        strings(&s["permissions"]["additionalDirectories"]),
        [
            clone.to_string_lossy().into_owned(),
            internal.to_string_lossy().into_owned()
        ]
    );
}

struct Server {
    addr: SocketAddr,
    fx: Fx,
}

async fn serve(fx: Fx) -> Server {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    Server { addr, fx }
}

async fn pre_tool_use(srv: &Server, id: &AgentId, tool: &str, input: Value) -> Value {
    let token = srv.fx.store.agent_token(id).await.unwrap().unwrap();
    let body = json!({
        "session_id": "s",
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": input,
        "tool_use_id": "toolu_1",
    });
    let (status, v) = json_of_with(
        srv.addr,
        &format!("/internal/hooks/{}", id.0),
        &token,
        &body,
    )
    .await;
    assert_eq!(status, 200, "{v}");
    v
}

async fn json_of_with(addr: SocketAddr, path: &str, token: &str, body: &Value) -> (u16, Value) {
    let (status, text) = http(addr, "POST", path, &[("X-Fleet-Token", token)], Some(body)).await;
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_supervisor_subagent_must_be_explore_or_plan() {
    let fx = fixture().await;
    let sup = start(&fx, "hello").await;
    let mut orch = sup.clone();
    orch.id = AgentId("orch1".into());
    orch.session = "fleet-orch1".into();
    orch.kind = fleet_api::AgentKind::Orchestrator;
    orch.product = PRODUCT.into();
    fx.store.insert_agent(&orch, "orch-token").await.unwrap();
    let srv = serve(fx).await;

    let denied = json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": "the supervisor's subagents are read-only (Explore, Plan); start a task to change anything",
    }});
    for tool in ["Agent", "Task"] {
        let v = pre_tool_use(
            &srv,
            &sup.id,
            tool,
            json!({"subagent_type": "general-purpose", "prompt": "x"}),
        )
        .await;
        assert_eq!(v, denied, "{tool}");
        let v = pre_tool_use(&srv, &sup.id, tool, json!({"prompt": "x"})).await;
        assert_eq!(v, denied, "{tool} with no type");
        for ok in ["Explore", "Plan"] {
            let v = pre_tool_use(
                &srv,
                &sup.id,
                tool,
                json!({"subagent_type": ok, "prompt": "x"}),
            )
            .await;
            assert_eq!(v, json!({}), "{tool} {ok}");
        }
    }
    let v = pre_tool_use(&srv, &sup.id, "Bash", json!({"command": "ls"})).await;
    assert_eq!(v, json!({}));
    let v = pre_tool_use(
        &srv,
        &orch.id,
        "Agent",
        json!({"subagent_type": "general-purpose"}),
    )
    .await;
    assert_eq!(v, json!({}));
}

#[tokio::test]
async fn a_supervisor_is_not_a_task() {
    let fx = fixture().await;
    let before = fx.store.tasks().await.unwrap();
    let a = start(&fx, "hello").await;
    assert_eq!(a.task, None);
    assert_eq!(fx.store.tasks().await.unwrap(), before);
    set_status(&fx, &a.id, Status::Stopped).await;
    assert_eq!(fx.store.tasks().await.unwrap(), before);
    fx.store.backfill_tasks().await.unwrap();
    assert_eq!(fx.store.tasks().await.unwrap(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_supervisor_does_not_trigger_a_merge_check() {
    let fx = fixture().await;
    let solo = spawn_ready(&fx).await;
    let sup = start(&fx, "hello").await;
    // A long period: only the run on start, and the stops below, could check.
    fx.svc.start_merge_watcher(Duration::from_secs(3600));
    assert!(wait_until(|| async { fx.svc.merge_checks_run() >= 1 }).await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let base = fx.svc.merge_checks_run();

    set_status(&fx, &sup.id, Status::Stopped).await;
    // The watcher reads events in order, so once the solo stop below is checked the supervisor's was seen.
    set_status(&fx, &solo.id, Status::Stopped).await;
    assert!(wait_until(|| async { fx.svc.merge_checks_run() > base }).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fx.svc.merge_checks_run(), base + 1);
}

#[tokio::test]
async fn other_agents_get_no_subagent_guard() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    let pre = settings(&fx, &a.id)["hooks"]["PreToolUse"].clone();
    assert_eq!(pre.as_array().unwrap().len(), 1, "{pre}");
    assert!(pre[0]["hooks"][0].get("onFailure").is_none(), "{pre}");
}

#[tokio::test]
async fn a_rebase_keeps_the_subagent_guard() {
    let fx = fixture().await;
    let a = start(&fx, "hello").await;
    let dir = fx.state_dir.join("agents").join(&a.id.0);
    let token = fx.store.agent_token(&a.id).await.unwrap().unwrap();
    fleet_daemon::hooks_cfg::rebase(&dir, "http://127.0.0.1:9999", &a.id, &token).unwrap();
    let pre = settings(&fx, &a.id)["hooks"]["PreToolUse"].clone();
    assert_eq!(pre[1]["matcher"], "Agent|Task", "{pre}");
    assert_eq!(pre[1]["hooks"][0]["onFailure"], "block");
    assert_eq!(
        pre[1]["hooks"][0]["url"],
        format!("http://127.0.0.1:9999/internal/hooks/{}", a.id.0)
    );
}

#[tokio::test]
async fn the_conversation_folder_is_trusted_for_its_persona() {
    let fx = fixture().await;
    let a = start(&fx, "hello").await;
    let claude_dir = fixture_env(&fx).await.claude_dir().unwrap();
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(claude_dir.join(".claude.json")).unwrap())
            .unwrap();
    let key =
        fleet_daemon::first_run::trust_key(&fleet_fs::canonical(Path::new(&a.workdir)).unwrap());
    assert_eq!(
        config["projects"][&key]["hasTrustDialogAccepted"], true,
        "{config}"
    );
    // The first-run answers are still there.
    assert_eq!(config["hasCompletedOnboarding"], true);
}

fn message_path(id: &AgentId) -> String {
    format!("/api/supervisor/conversations/{}/message", id.0)
}

async fn send(srv: &Server, id: &AgentId, text: &str) -> u16 {
    json_of(
        srv.addr,
        "POST",
        &message_path(id),
        Some(&json!({ "text": text })),
    )
    .await
    .0
}

/// A fixture whose fake Claude takes a while to come up and drops what is typed before, as the real TUI does.
async fn slow_start() -> Fx {
    fixture_options(
        |_| {},
        |o| o.extra_env = vec![("FAKE_CLAUDE_READY_AFTER_MS".into(), "800".into())],
    )
    .await
}

/// A conversation at its prompt, stopped as the reaper stops it: its process is gone.
async fn exited(fx: &Fx) -> Agent {
    let a = start(fx, "hello").await;
    wait_screen_in(&fx.sessions, &a.session, |s| s.contains("ready")).await;
    fx.svc.stop(&a.id).await.unwrap();
    assert!(wait_until(|| async { !fx.sessions.alive(&a.session) }).await);
    a
}

fn count(screen: &str, needle: &str) -> usize {
    screen.matches(needle).count()
}

#[tokio::test]
async fn a_message_to_a_live_conversation_is_typed() {
    let fx = fixture().await;
    let a = start(&fx, "hello").await;
    wait_screen_in(&fx.sessions, &a.session, |s| s.contains("ready")).await;
    let before = fx.store.conversations(PERSONA).await.unwrap()[0].last_used_at;
    fx.clock.advance(30);
    let srv = serve(fx).await;
    assert_eq!(send(&srv, &a.id, "  and the docs?  ").await, 204);
    let screen = wait_screen_in(&srv.fx.sessions, &a.session, |s| s.contains("entry<<")).await;
    assert_eq!(count(&screen, "entry<<and the docs?>>"), 1, "{screen}");
    let row = &srv.fx.store.conversations(PERSONA).await.unwrap()[0];
    assert_eq!(row.last_used_at, before + 30);
}

#[tokio::test]
async fn a_message_to_an_exited_conversation_resumes_then_types_once() {
    let fx = slow_start().await;
    let a = exited(&fx).await;
    let claude_session = fx.store.claude_session(&a.id).await.unwrap().unwrap();
    let srv = serve(fx).await;
    assert_eq!(send(&srv, &a.id, "are we there yet?").await, 204);
    let screen = wait_screen_in(&srv.fx.sessions, &a.session, |s| {
        s.contains("entry<<are we there yet?>>")
    })
    .await;
    assert!(screen.contains("arg=--resume"), "{screen}");
    assert!(
        screen.contains(&format!("arg={claude_session}")),
        "{screen}"
    );
    assert!(!screen.contains("arg=--session-id"), "{screen}");
    // Typed once, and only into the ready TUI: nothing of it echoes before `ready`.
    let ready = screen.find("ready").unwrap();
    let first = screen.find("are we there yet?").unwrap();
    assert!(first > ready, "{screen}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let screen = srv.fx.sessions.screen_text(&a.session).unwrap();
    assert_eq!(count(&screen, "entry<<"), 1, "{screen}");
    let now = agent(&srv.fx, &a.id).await;
    assert_ne!(now.status, Status::Stopped);
    let listed = Supervisor::new(srv.fx.svc.clone())
        .list(PERSONA)
        .await
        .unwrap();
    assert!(listed[0].live);
}

#[tokio::test]
async fn two_messages_while_resuming_arrive_in_order() {
    let fx = slow_start().await;
    let a = exited(&fx).await;
    let srv = serve(fx).await;
    let (addr, path) = (srv.addr, message_path(&a.id));
    let first = tokio::spawn({
        let path = path.clone();
        async move {
            json_of(addr, "POST", &path, Some(&json!({"text": "one"})))
                .await
                .0
        }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let second = tokio::spawn(async move {
        json_of(addr, "POST", &path, Some(&json!({"text": "two"})))
            .await
            .0
    });
    assert_eq!(first.await.unwrap(), 204);
    assert_eq!(second.await.unwrap(), 204);
    let screen = wait_screen_in(&srv.fx.sessions, &a.session, |s| s.contains("entry<<two>>")).await;
    let one = screen.find("entry<<one>>").expect(&screen);
    let two = screen.find("entry<<two>>").unwrap();
    assert!(one < two, "{screen}");
    assert_eq!(count(&screen, "entry<<"), 2, "{screen}");
    // One process: the second message did not start another.
    assert_eq!(count(&screen, "arg=--resume"), 1, "{screen}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_reaper_stops_only_idle_review_after_ten_minutes() {
    let fx = fixture().await;
    let mut cases = Vec::new();
    for (status, idle, stopped) in [
        (Status::Working, 660, false),
        (Status::NeedsYou, 660, false),
        (Status::Talking, 660, false),
        (Status::Queued, 660, false),
        (Status::Review, 540, false),
        (Status::Review, 600, true),
    ] {
        cases.push((start(&fx, "hello").await, status, idle, stopped));
    }
    // Typed into a minute ago, still in Review until Claude's hook says otherwise.
    let typed = start(&fx, "hello").await;
    fx.clock.advance(660);
    let now = fx.clock.0.load(std::sync::atomic::Ordering::SeqCst);
    for (a, status, idle, _) in &cases {
        let (status, idle) = (*status, *idle);
        fx.store
            .update_agent(&a.id, |x| {
                x.status = status;
                x.status_since = now - idle;
            })
            .await
            .unwrap();
    }
    fx.store
        .update_agent(&typed.id, |x| {
            x.status = Status::Review;
            x.status_since = now - 660;
        })
        .await
        .unwrap();
    fx.store
        .touch_conversation(&typed.id, now - 60)
        .await
        .unwrap();
    // Only supervisors: a task idle in Review for longer is left alone.
    let solo = spawn_ready(&fx).await;
    fx.store
        .update_agent(&solo.id, |x| {
            x.status = Status::Review;
            x.status_since = now - 3600;
        })
        .await
        .unwrap();

    let reaper =
        fleet_daemon::supervisor::start_idle_reaper(fx.svc.clone(), Duration::from_millis(50));
    assert_eq!(
        fleet_daemon::supervisor::IDLE_EXIT,
        Duration::from_secs(600)
    );
    let idle = cases.last().unwrap().0.id.clone();
    assert!(wait_until(|| async { agent(&fx, &idle).await.status == Status::Stopped }).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    for (a, status, idle, stopped) in &cases {
        let now = agent(&fx, &a.id).await;
        assert_eq!(
            now.status == Status::Stopped,
            *stopped,
            "{status:?} idle {idle} s: {now:?}"
        );
    }
    assert!(!fx.sessions.alive(&cases.last().unwrap().0.session));
    assert_eq!(agent(&fx, &typed.id).await.status, Status::Review);
    assert_eq!(agent(&fx, &solo.id).await.status, Status::Review);
    // Last: an abort in the middle of a store call drops the in-memory connection, and the store reopens empty
    // (#475), which would fail the reads above.
    reaper.abort();
}

#[tokio::test]
async fn conversations_list_their_tasks_with_status() {
    let fx = fixture().await;
    let srv = serve(fx).await;
    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/supervisor/conversations",
        Some(&json!({"persona": PERSONA, "text": "plan the release"})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let made: fleet_api::Conversation = serde_json::from_value(v).unwrap();
    assert_eq!(made.persona, PERSONA);
    assert!(made.live);
    assert!(made.tasks.is_empty());

    let solo = spawn_ready(&srv.fx).await;
    let origin = made.id.clone();
    srv.fx
        .store
        .update_task(&solo.id, |t| t.origin = Some(origin))
        .await
        .unwrap();
    // Another task, started by no conversation, is not listed.
    spawn_ready(&srv.fx).await;
    set_status(&srv.fx, &solo.id, Status::NeedsYou).await;

    let (status, v) = json_of(
        srv.addr,
        "GET",
        &format!("/api/supervisor/{PERSONA}/conversations"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let listed: Vec<fleet_api::Conversation> = serde_json::from_value(v).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, made.id);
    assert_eq!(
        listed[0].tasks,
        [fleet_api::TaskMark {
            id: solo.id.clone(),
            title: "do the thing".into(),
            status: Status::NeedsYou,
        }]
    );
    let (_, v) = json_of(
        srv.addr,
        "GET",
        "/api/supervisor/nobody/conversations",
        None,
    )
    .await;
    assert_eq!(v, json!([]));
}

#[tokio::test]
async fn blank_text_is_400_and_unknown_id_is_404() {
    let fx = fixture().await;
    let a = start(&fx, "hello").await;
    let solo = spawn_ready(&fx).await;
    let srv = serve(fx).await;
    assert_eq!(send(&srv, &a.id, " \n ").await, 400);
    assert_eq!(send(&srv, &AgentId("nope".into()), "hi").await, 404);
    // A task's agent is no conversation.
    assert_eq!(send(&srv, &solo.id, "hi").await, 404);
    let (status, _) = json_of(
        srv.addr,
        "POST",
        "/api/supervisor/conversations",
        Some(&json!({"persona": PERSONA, "text": "  "})),
    )
    .await;
    assert_eq!(status, 400);
}
