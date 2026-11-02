mod common;

use common::net::*;
use common::*;
use fleet_api::{AgentId, AgentKind, PostKind, PrState, Task, TaskPr, TaskState};
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::store::RepoRow;
use fleet_daemon::supervisor::Supervisor;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;

struct Server {
    addr: SocketAddr,
    fx: Fx,
}

async fn start() -> Server {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    Server { addr, fx }
}

async fn conversation(srv: &Server) -> AgentId {
    Supervisor::new(srv.fx.svc.clone())
        .new_conversation(PERSONA, "what is going on?")
        .await
        .unwrap()
}

async fn rpc(srv: &Server, id: &AgentId, body: &Value) -> Value {
    let token = srv.fx.store.agent_token(id).await.unwrap().unwrap();
    let (status, text) = http(
        srv.addr,
        "POST",
        &format!("/internal/mcp/{}", id.0),
        &[("X-Fleet-Token", &token)],
        Some(body),
    )
    .await;
    assert_eq!(status, 200, "{text}");
    serde_json::from_str(&text).unwrap()
}

/// The tool's text and whether it was an error.
async fn call(srv: &Server, id: &AgentId, name: &str, args: Value) -> (String, bool) {
    let v = rpc(
        srv,
        id,
        &json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}),
    )
    .await;
    let r = &v["result"];
    (
        r["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no text in {v}"))
            .to_string(),
        r["isError"].as_bool().unwrap(),
    )
}

async fn tool_names(srv: &Server, id: &AgentId) -> Vec<String> {
    let v = rpc(
        srv,
        id,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn gh_dir(srv: &Server) -> PathBuf {
    srv.fx._tmp.path().join("personas").join(PERSONA).join("gh")
}

const SUPERVISOR_TOOLS: [&str; 6] = [
    "fleet_overview",
    "file_issue",
    "write_doc",
    "start_task",
    "merge_pr",
    "archive_task",
];

#[tokio::test]
async fn only_a_supervisor_is_offered_the_supervisor_tools() {
    let srv = start().await;
    let solo = spawn_ready(&srv.fx).await;
    let names = tool_names(&srv, &solo.id).await;
    for t in SUPERVISOR_TOOLS {
        assert!(
            !names.contains(&t.to_string()),
            "{t} offered to a solo agent"
        );
        let (text, err) = call(&srv, &solo.id, t, json!({})).await;
        assert!(err && text.contains("not available"), "{t}: {text}");
    }
    let sup = conversation(&srv).await;
    let names = tool_names(&srv, &sup).await;
    for t in SUPERVISOR_TOOLS.into_iter().chain(["set_title"]) {
        assert!(names.contains(&t.to_string()), "{t} missing: {names:?}");
    }
    assert!(!names.contains(&"open_pr".to_string()));
}

#[tokio::test]
async fn overview_lists_the_personas_tasks_across_products() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let b = spawn_ready(&srv.fx).await;
    let c = spawn_ready(&srv.fx).await;
    let store = &srv.fx.store;
    store
        .update_task(&a.id, |t| t.title = Some("Build X".into()))
        .await
        .unwrap();
    store
        .update_task(&b.id, |t| {
            t.title = Some("Fix Y".into());
            t.product = "other".into();
            t.prs = vec![TaskPr {
                url: "https://example.test/pr/9".into(),
                repo: "core".into(),
                state: PrState::Open,
                merged_at: None,
                ci: fleet_api::CiState::Unknown,
                failing_checks: vec![],
                can_merge: false,
                merge_method: None,
            }];
        })
        .await
        .unwrap();
    store
        .update_task(&c.id, |t| {
            t.title = Some("Old Z".into());
            t.state = TaskState::Archived;
        })
        .await
        .unwrap();
    let sup = conversation(&srv).await;
    let (text, err) = call(&srv, &sup, "fleet_overview", json!({})).await;
    assert!(!err, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(
        lines[0].starts_with("p · Build X · ") && lines[0].ends_with(" · steps 0/0 · PR none"),
        "{text}"
    );
    assert!(
        lines[1].starts_with("other · Fix Y · ") && lines[1].ends_with(" · steps 0/0 · PR open"),
        "{text}"
    );
}

/// Points `core`'s origin at a GitHub repo; nothing fetches it, `file_issue` only reads the URL.
fn github_origin(srv: &Server) {
    git(
        &srv.fx.repo,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/core.git",
        ],
    );
}

fn issue_posts(srv: &Server) -> Vec<common::fake_forge::Recorded> {
    srv.fx
        .forge
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path_and_query == "/repos/acme/core/issues")
        .collect()
}

#[tokio::test]
async fn file_issue_posts_to_the_forge_as_the_persona() {
    let srv = start().await;
    github_origin(&srv);
    let sup = conversation(&srv).await;
    let (text, err) = call(
        &srv,
        &sup,
        "file_issue",
        json!({"product": PRODUCT, "repo": "core", "title": "It breaks", "body": "Steps: 1", "labels": ["kind/bug", "ui"]}),
    )
    .await;
    assert!(!err, "{text}");
    assert!(
        text.starts_with("https://github.com/acme/core/issues/"),
        "{text}"
    );
    let posts = issue_posts(&srv);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].auth, "Bearer ghp_fake");
    assert_eq!(
        posts[0].body,
        json!({"title": "It breaks", "body": "Steps: 1", "labels": ["kind/bug", "ui"]})
    );
    let calls = srv.fx.store.forge_calls(10).await.unwrap();
    let row = calls.iter().find(|c| c.op == "create_issue").unwrap();
    assert_eq!(
        (row.agent.as_ref(), row.repo.as_str(), row.ok),
        (Some(&sup), "github.com/acme/core", true)
    );
    let (text, err) = call(
        &srv,
        &sup,
        "file_issue",
        json!({"product": PRODUCT, "repo": "nope", "title": "t", "body": "b"}),
    )
    .await;
    assert!(err && text.contains("nope"), "{text}");
}

#[tokio::test]
async fn file_issue_works_for_a_pat_only_persona() {
    let srv = start().await;
    github_origin(&srv);
    let store = &srv.fx.store;
    let mut row = store.persona(PERSONA).await.unwrap().unwrap();
    row.github_auth = fleet_api::GithubAuth::Pat;
    store.update_persona(&row).await.unwrap();
    std::fs::remove_dir_all(gh_dir(&srv)).unwrap();
    let sup = conversation(&srv).await;
    let (text, err) = call(
        &srv,
        &sup,
        "file_issue",
        json!({"product": PRODUCT, "repo": "core", "title": "t", "body": "b"}),
    )
    .await;
    assert!(!err, "{text}");
    assert_eq!(issue_posts(&srv).len(), 1);
}

#[tokio::test]
async fn file_issue_refuses_a_repo_with_no_forge_origin() {
    let srv = start().await;
    let sup = conversation(&srv).await;
    let (text, err) = call(
        &srv,
        &sup,
        "file_issue",
        json!({"product": PRODUCT, "repo": "core", "title": "t", "body": "b"}),
    )
    .await;
    assert!(err && text.contains("no GitHub owner and repo"), "{text}");
    assert!(issue_posts(&srv).is_empty());
}

#[tokio::test]
async fn write_doc_commits_on_main_without_pushing() {
    let srv = start().await;
    let sup = conversation(&srv).await;
    let (text, err) = call(
        &srv,
        &sup,
        "write_doc",
        json!({"product": PRODUCT, "path": "specs/2026-11-01-thing.md", "content": "# Thing\n", "message": "spec: thing"}),
    )
    .await;
    assert!(!err, "{text}");
    let repo = srv.fx.svc.internal().repo_dir(PRODUCT);
    let sha = git(&repo, &["rev-parse", "--short", "main"]);
    assert_eq!(text, format!("committed {sha} specs/2026-11-01-thing.md"));
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%s", "main"]),
        "spec: thing"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("specs/2026-11-01-thing.md")).unwrap(),
        "# Thing\n"
    );
    assert_eq!(git(&repo, &["remote"]), "");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
}

#[tokio::test]
async fn write_doc_refuses_paths_outside_specs_and_plans() {
    let srv = start().await;
    let sup = conversation(&srv).await;
    for path in [
        "README.md",
        "docs/x.md",
        "specs/x.txt",
        "specs/../README.md",
        "plans/a/../../x.md",
        "/specs/x.md",
        "specs/sub/x.md",
    ] {
        let (text, err) = call(
            &srv,
            &sup,
            "write_doc",
            json!({"product": PRODUCT, "path": path, "content": "x", "message": "m"}),
        )
        .await;
        assert!(
            err && text == "path must be specs/<name>.md or plans/<name>.md",
            "{path}: {text}"
        );
    }
}

/// `core` ready for the direct orchestrator `start_task` starts: an origin whose `main` has a workflow.
fn ready_for_direct(srv: &Server) {
    add_origin_with_workflow(&srv.fx, &one_step("true"));
}

#[tokio::test]
async fn start_task_records_the_conversation_as_origin() {
    let srv = start().await;
    ready_for_direct(&srv);
    let sup = conversation(&srv).await;
    let (text, err) = call(
        &srv,
        &sup,
        "start_task",
        json!({"product": PRODUCT, "repos": ["core"], "prompt": "do it", "title": "Do It"}),
    )
    .await;
    assert!(!err, "{text}");
    assert!(
        text.starts_with("started ") && text.ends_with(" Do It"),
        "{text}"
    );
    let id = AgentId(text.split(' ').nth(1).unwrap().to_string());
    let task = srv.fx.store.task(&id).await.unwrap().unwrap();
    assert_eq!(task.origin, Some(sup.clone()));
    assert_eq!(task.persona, PERSONA);
    assert_eq!(task.product, PRODUCT);
    let listed = Supervisor::new(srv.fx.svc.clone())
        .list(PERSONA)
        .await
        .unwrap();
    assert_eq!(listed[0].tasks.len(), 1);
    let (text, err) = call(
        &srv,
        &sup,
        "start_task",
        json!({"product": "nope", "repos": [], "prompt": "x"}),
    )
    .await;
    assert!(err && text.contains("nope"), "{text}");
}

#[tokio::test]
async fn start_task_always_starts_a_direct_orchestrator_with_the_plan_field() {
    let srv = start().await;
    ready_for_direct(&srv);
    let sup = conversation(&srv).await;
    // An old call's `kind` and `role` are ignored.
    let (text, err) = call(
        &srv,
        &sup,
        "start_task",
        json!({"product": PRODUCT, "repos": ["core"], "prompt": "do it", "kind": "solo",
               "role": "structured", "plan": "plans/2026-11-01-x.md"}),
    )
    .await;
    assert!(!err, "{text}");
    let id = AgentId(text.split(' ').nth(1).unwrap().to_string());
    let task = srv.fx.store.task(&id).await.unwrap().unwrap();
    assert_eq!(
        (task.kind, task.role.as_deref(), task.plan.as_deref()),
        (
            AgentKind::Orchestrator,
            Some("direct"),
            Some("plans/2026-11-01-x.md")
        )
    );
    let a = agent(&srv.fx, &id).await;
    let prompt =
        std::fs::read_to_string(PathBuf::from(&a.workdir).join(fleet_daemon::agents::PROMPT_FILE))
            .unwrap();
    assert!(
        prompt.contains("do it") && !prompt.contains("The plan is"),
        "{prompt}"
    );
}

#[tokio::test]
async fn start_task_offers_no_kind_or_role() {
    let srv = start().await;
    let sup = conversation(&srv).await;
    let v = rpc(
        &srv,
        &sup,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let tool = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "start_task")
        .unwrap()
        .clone();
    let props = &tool["inputSchema"]["properties"];
    assert!(
        props.get("kind").is_none() && props.get("role").is_none(),
        "{tool}"
    );
    assert!(props.get("plan").is_some(), "{tool}");
    assert!(!tool["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("kind")));
}

// ---- merge_pr and archive_task: an ask to the owner, and the action only on their yes ----

const GHE_URL: &str = "https://ghe.example.com/acme/core.git/";
const PRODUCT_ROOM: &str = "product:p";

/// A stopped task `t1` on `core` (a GitHub Enterprise repo) with an open PR #7 whose checks are green but for
/// `windows`, which has this conclusion.
async fn mergeable_task(srv: &Server, windows: &str) -> AgentId {
    let fx = &srv.fx;
    fx.store
        .upsert_repo(&RepoRow {
            product: PRODUCT.into(),
            name: "core".into(),
            path: Some(fx.repo.clone()),
            url: Some(GHE_URL.into()),
        })
        .await
        .unwrap();
    let base = git(&fx.repo, &["rev-parse", "HEAD"]);
    let tip = git(
        &fx.repo,
        &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "work"],
    );
    git(&fx.repo, &["branch", "fleet/t1", &tip]);
    let t: Task = serde_json::from_value(json!({
        "id": "t1", "product": PRODUCT, "release": null, "name": "t1", "goal": "g", "title": "Ship it",
        "workdir": fx.tasks_root.join("t1").to_string_lossy(),
        "repos": [{"repo": "core", "branch": "fleet/t1", "worktree": "/nowhere", "base": base, "has_changes": false}],
        "state": "stopped", "agents_used": 1,
        "times": {"queued": 0, "working": 0, "needs_you": 0, "review": 0},
        "log": [], "persona": PERSONA
    }))
    .unwrap();
    fx.store.insert_task(&t).await.unwrap();
    set_windows(srv, windows);
    t.id
}

fn set_windows(srv: &Server, windows: &str) {
    let forge = &srv.fx.forge;
    forge.set_prs(
        "acme",
        "core",
        vec![
            json!({"html_url": "https://ghe.example.com/acme/core/pull/7", "number": 7,
                    "state": "open", "merged_at": null}),
        ],
    );
    forge.set_pr_detail(
        "acme",
        "core",
        7,
        json!({"mergeable": true, "mergeable_state": "clean",
               "head": {"sha": "sha1"}, "base": {"ref": "main"}}),
    );
    forge.set_check_runs(
        "acme",
        "core",
        "sha1",
        json!({"check_runs": [
            {"name": "linux", "status": "completed", "conclusion": "success"},
            {"name": "windows", "status": "completed", "conclusion": windows},
        ]}),
    );
    forge.set_statuses("acme", "core", "sha1", json!({"statuses": []}));
    forge.set_repo_settings(
        "acme",
        "core",
        json!({"allow_rebase_merge": true, "allow_squash_merge": true, "allow_merge_commit": true}),
    );
}

/// The ask a tool posted, from its reply `… ask <id> …`.
async fn ask_of(srv: &Server, reply: &str) -> fleet_api::Post {
    let id: i64 = reply
        .split("ask ")
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no ask id in {reply:?}"));
    srv.fx.store.post(id).await.unwrap().unwrap()
}

async fn owner_answers(srv: &Server, ask: i64, text: &str) -> u16 {
    json_of(
        srv.addr,
        "POST",
        &format!("/api/rooms/asks/{ask}/answer"),
        Some(&json!({"text": text})),
    )
    .await
    .0
}

async fn room_texts(srv: &Server) -> Vec<String> {
    srv.fx
        .store
        .room_posts(PRODUCT_ROOM, None, 50)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.text)
        .collect()
}

async fn record(srv: &Server, id: &AgentId) -> Task {
    srv.fx.store.task(id).await.unwrap().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_pr_asks_the_owner_and_merges_only_on_their_yes() {
    let srv = start().await;
    let id = mergeable_task(&srv, "success").await;
    let sup = conversation(&srv).await;

    let (text, err) = call(&srv, &sup, "merge_pr", json!({"task": id.0})).await;
    assert!(!err, "{text}");
    let ask = ask_of(&srv, &text).await;
    assert_eq!(ask.kind, PostKind::Ask);
    assert_eq!(ask.to, Some(fleet_api::MemberRef::Owner));
    assert_eq!(ask.sender, fleet_api::MemberRef::Agent(sup.clone()));
    assert!(ask.text.starts_with("Merge PR #7?"), "{}", ask.text);
    assert!(ask.answered_at.is_none());
    assert!(
        srv.fx.forge.merge_requests().is_empty(),
        "nothing happens before the answer"
    );

    assert_eq!(owner_answers(&srv, ask.id, "yes").await, 204);

    let sent = srv.fx.forge.merge_requests();
    assert_eq!(sent.len(), 1, "the owner's yes merges");
    assert_eq!(sent[0].path_and_query, "/repos/acme/core/pulls/7/merge");
    assert!(room_texts(&srv)
        .await
        .iter()
        .any(|t| t.contains("merged PR #7")));
    assert_eq!(owner_answers(&srv, ask.id, "yes").await, 409);
    assert_eq!(srv.fx.forge.merge_requests().len(), 1, "an ask acts once");
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_pr_does_nothing_on_a_no_or_a_comment() {
    let srv = start().await;
    let id = mergeable_task(&srv, "success").await;
    let sup = conversation(&srv).await;
    for answer in ["no", "not before the release is cut"] {
        let (text, err) = call(&srv, &sup, "merge_pr", json!({"task": id.0})).await;
        assert!(!err, "{text}");
        let ask = ask_of(&srv, &text).await;
        assert_eq!(owner_answers(&srv, ask.id, answer).await, 204);
        let after = srv.fx.store.post(ask.id).await.unwrap().unwrap();
        assert!(
            after.answered_at.is_some(),
            "the ask is closed by the answer"
        );
    }
    assert!(srv.fx.forge.merge_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_supervisor_cannot_answer_its_own_merge_ask() {
    let srv = start().await;
    let id = mergeable_task(&srv, "success").await;
    let sup = conversation(&srv).await;
    let (text, _) = call(&srv, &sup, "merge_pr", json!({"task": id.0})).await;
    let ask = ask_of(&srv, &text).await;

    let (reply, err) = call(
        &srv,
        &sup,
        "room_answer",
        json!({"ask": ask.id, "text": "yes"}),
    )
    .await;

    assert!(err, "{reply}");
    assert!(srv.fx.forge.merge_requests().is_empty());
    let after = srv.fx.store.post(ask.id).await.unwrap().unwrap();
    assert!(after.answered_at.is_none(), "the ask is still open");
    // Another supervisor conversation of the persona is no more the owner.
    let other = conversation(&srv).await;
    let (_, err) = call(
        &srv,
        &other,
        "room_answer",
        json!({"ask": ask.id, "text": "yes"}),
    )
    .await;
    assert!(err);
    assert!(srv.fx.forge.merge_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_pr_refuses_a_red_pr_without_asking() {
    let srv = start().await;
    let id = mergeable_task(&srv, "failure").await;
    let sup = conversation(&srv).await;
    let (text, err) = call(&srv, &sup, "merge_pr", json!({"task": id.0})).await;
    assert!(err && text.contains("windows"), "{text}");
    assert!(room_texts(&srv).await.is_empty(), "no ask was posted");
    let (text, err) = call(&srv, &sup, "merge_pr", json!({"task": "nope"})).await;
    assert!(err, "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_yes_after_ci_turned_red_does_not_merge() {
    let srv = start().await;
    let id = mergeable_task(&srv, "success").await;
    let sup = conversation(&srv).await;
    let (text, err) = call(&srv, &sup, "merge_pr", json!({"task": id.0})).await;
    assert!(!err, "{text}");
    let ask = ask_of(&srv, &text).await;

    set_windows(&srv, "failure");
    assert_eq!(owner_answers(&srv, ask.id, "yes").await, 204);

    assert!(srv.fx.forge.merge_requests().is_empty());
    let texts = room_texts(&srv).await;
    assert!(
        texts
            .iter()
            .any(|t| t.contains("not merged") && t.contains("windows")),
        "{texts:?}"
    );
}

async fn stopped_task(srv: &Server) -> fleet_api::Agent {
    let a = spawn_ready(&srv.fx).await;
    srv.fx.svc.stop(&a.id).await.unwrap();
    a
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_task_asks_the_owner_and_archives_only_on_their_yes() {
    let srv = start().await;
    let a = stopped_task(&srv).await;
    let sup = conversation(&srv).await;

    let (text, err) = call(&srv, &sup, "archive_task", json!({"task": a.id.0})).await;
    assert!(!err, "{text}");
    let ask = ask_of(&srv, &text).await;
    assert_eq!(ask.to, Some(fleet_api::MemberRef::Owner));
    assert!(ask.text.starts_with("Archive task "), "{}", ask.text);
    assert!(
        ask.text.contains(" frees ") && ask.text.ends_with("nothing is uncommitted"),
        "{}",
        ask.text
    );
    assert_eq!(record(&srv, &a.id).await.state, TaskState::Stopped);
    assert!(std::path::Path::new(&a.workspaces[0].worktree).exists());

    assert_eq!(owner_answers(&srv, ask.id, "yes").await, 204);

    assert_eq!(record(&srv, &a.id).await.state, TaskState::Archived);
    assert!(!std::path::Path::new(&a.workspaces[0].worktree).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_task_does_nothing_on_a_no() {
    let srv = start().await;
    let a = stopped_task(&srv).await;
    let sup = conversation(&srv).await;
    let (text, _) = call(&srv, &sup, "archive_task", json!({"task": a.id.0})).await;
    let ask = ask_of(&srv, &text).await;
    assert_eq!(owner_answers(&srv, ask.id, "no, keep it").await, 204);
    assert_eq!(record(&srv, &a.id).await.state, TaskState::Stopped);
    assert!(std::path::Path::new(&a.workspaces[0].worktree).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_supervisor_cannot_answer_its_own_archive_ask() {
    let srv = start().await;
    let a = stopped_task(&srv).await;
    let sup = conversation(&srv).await;
    let (text, _) = call(&srv, &sup, "archive_task", json!({"task": a.id.0})).await;
    let ask = ask_of(&srv, &text).await;
    let (reply, err) = call(
        &srv,
        &sup,
        "room_answer",
        json!({"ask": ask.id, "text": "yes"}),
    )
    .await;
    assert!(err, "{reply}");
    assert_eq!(record(&srv, &a.id).await.state, TaskState::Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_task_refuses_a_running_task() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let sup = conversation(&srv).await;
    let (text, err) = call(&srv, &sup, "archive_task", json!({"task": a.id.0})).await;
    assert!(err, "{text}");
    assert!(room_texts(&srv).await.is_empty(), "no ask was posted");
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_task_refuses_when_work_would_be_lost() {
    let srv = start().await;
    let a = stopped_task(&srv).await;
    std::fs::write(
        std::path::Path::new(&a.workspaces[0].worktree).join("draft.txt"),
        "lost",
    )
    .unwrap();
    let sup = conversation(&srv).await;
    let (text, err) = call(&srv, &sup, "archive_task", json!({"task": a.id.0})).await;
    assert!(err && text.contains("uncommitted"), "{text}");
    assert!(room_texts(&srv).await.is_empty(), "no ask was posted");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_yes_after_work_appeared_does_not_archive() {
    let srv = start().await;
    let a = stopped_task(&srv).await;
    let sup = conversation(&srv).await;
    let (text, _) = call(&srv, &sup, "archive_task", json!({"task": a.id.0})).await;
    let ask = ask_of(&srv, &text).await;
    std::fs::write(
        std::path::Path::new(&a.workspaces[0].worktree).join("draft.txt"),
        "lost",
    )
    .unwrap();

    assert_eq!(owner_answers(&srv, ask.id, "yes").await, 204);

    assert_eq!(record(&srv, &a.id).await.state, TaskState::Stopped);
    assert!(std::path::Path::new(&a.workspaces[0].worktree)
        .join("draft.txt")
        .exists());
    let texts = room_texts(&srv).await;
    assert!(
        texts
            .iter()
            .any(|t| t.contains("not archived") && t.contains("uncommitted")),
        "{texts:?}"
    );
}
