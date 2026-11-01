mod common;

use common::net::*;
use common::*;
use fleet_api::{AgentId, PrState, TaskPr, TaskState};
use fleet_daemon::http::{bind_loopback, router, AppState};
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

const SUPERVISOR_TOOLS: [&str; 4] = ["fleet_overview", "file_issue", "write_doc", "start_task"];

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

#[tokio::test]
async fn file_issue_runs_gh_as_the_persona() {
    let srv = start().await;
    let sup = conversation(&srv).await;
    let (text, err) = call(
        &srv,
        &sup,
        "file_issue",
        json!({"product": PRODUCT, "repo": "core", "title": "It breaks", "body": "Steps: 1", "labels": ["kind/bug", "ui"]}),
    )
    .await;
    assert!(!err, "{text}");
    assert_eq!(text, "https://example.test/issue/1");
    let log = std::fs::read_to_string(gh_dir(&srv).join("argv.log")).unwrap();
    let line = log.lines().find(|l| l.starts_with("issue create")).unwrap();
    for part in [
        "--title It breaks",
        "--body Steps: 1",
        "--label kind/bug",
        "--label ui",
    ] {
        assert!(line.contains(part), "{part} missing: {line}");
    }
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

#[tokio::test]
async fn start_task_records_the_conversation_as_origin() {
    let srv = start().await;
    let sup = conversation(&srv).await;
    let (text, err) = call(
        &srv,
        &sup,
        "start_task",
        json!({"product": PRODUCT, "repos": ["core"], "prompt": "do it", "title": "Do It", "kind": "solo"}),
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
        json!({"product": "nope", "repos": [], "prompt": "x", "kind": "solo"}),
    )
    .await;
    assert!(err && text.contains("nope"), "{text}");
    let (text, err) = call(
        &srv,
        &sup,
        "start_task",
        json!({"product": PRODUCT, "repos": ["core"], "prompt": "x", "kind": "weird"}),
    )
    .await;
    assert!(err && text.contains("kind"), "{text}");
}

#[tokio::test]
async fn start_task_with_a_plan_names_it_in_the_prompt() {
    let srv = start().await;
    let sup = conversation(&srv).await;
    let (text, err) = call(
        &srv,
        &sup,
        "start_task",
        json!({"product": PRODUCT, "repos": ["core"], "prompt": "do it", "kind": "solo", "plan": "plans/2026-11-01-x.md"}),
    )
    .await;
    assert!(!err, "{text}");
    let id = AgentId(text.split(' ').nth(1).unwrap().to_string());
    let a = agent(&srv.fx, &id).await;
    let prompt =
        std::fs::read_to_string(PathBuf::from(&a.workdir).join(fleet_daemon::agents::PROMPT_FILE))
            .unwrap();
    assert!(
        prompt.contains("do it")
            && prompt
                .trim_end()
                .ends_with("The plan is plans/2026-11-01-x.md in the internal repo."),
        "{prompt}"
    );
}
