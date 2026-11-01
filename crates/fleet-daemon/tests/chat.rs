mod common;

use common::net::*;
use common::*;
use fleet_api::{
    AgentId, AskKind, AskOption, AskQuestion, BlockedReason, CallStatus, ChatAsk, ChatBody,
    ChatItem, ChatPage, MarkerKind, PromptFrom, Status,
};
use fleet_daemon::agents::{PROMPT_FILE, PROMPT_POINTER};
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::json;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct Server {
    addr: SocketAddr,
    fx: Fx,
}

async fn start() -> Server {
    start_holding(None).await
}

/// `hold`: how long a PermissionRequest hook is held before it answers nothing (the daemon's own when None).
async fn start_holding(hold: Option<Duration>) -> Server {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
    if let Some(hold) = hold {
        state = state.with_ask_hold(hold);
    }
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    Server { addr, fx }
}

fn fixture_lines(name: &str) -> Vec<String> {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/transcripts/{name}.jsonl",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .lines()
    .map(str::to_string)
    .collect()
}

/// `tools.jsonl` up to and including its first `tool_use`, so it ends on a running call.
fn up_to_first_call() -> Vec<String> {
    let lines = fixture_lines("tools");
    let end = lines
        .iter()
        .position(|l| l.contains(r#""type":"tool_use""#))
        .unwrap();
    lines[..=end].to_vec()
}

fn prompt_line(text: &str) -> String {
    json!({"type": "user", "timestamp": "2026-11-01T03:27:24.604Z", "message": {"role": "user", "content": text}})
        .to_string()
}

/// Where the persona's Claude keeps `session`'s transcript: the fake Claude's project folder.
fn transcript(srv: &Server, session: &str) -> PathBuf {
    let dir = srv
        .fx
        .svc
        .config()
        .personas_dir
        .join(PERSONA)
        .join("claude/projects/fake-cwd");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{session}.jsonl"))
}

fn append(path: &PathBuf, lines: &[String]) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

/// Records `session` for the agent and writes `lines` as its transcript (none: no file).
async fn session(srv: &Server, id: &AgentId, session: &str, at: i64, lines: &[String]) {
    if !lines.is_empty() {
        append(&transcript(srv, session), lines);
    }
    srv.fx
        .store
        .add_claude_session(id, session, at)
        .await
        .unwrap();
}

/// The Claude session the agent was launched with.
async fn own(srv: &Server, id: &AgentId) -> String {
    srv.fx.store.claude_session(id).await.unwrap().unwrap()
}

fn chat_path(id: &AgentId) -> String {
    format!("/api/agents/{}/chat", id.0)
}

/// The next `kind` event's data. `ask` and `items` events come and go beside the one wanted, so they are skipped.
async fn next_of(sse: &mut SseClient, kind: &str) -> String {
    loop {
        let ev = sse.next().await;
        if ev.event.as_deref() == Some(kind) {
            return ev.data;
        }
        assert!(
            matches!(ev.event.as_deref(), Some("ask" | "items")),
            "wanted {kind}: {ev:?}"
        );
    }
}

async fn page_of(sse: &mut SseClient) -> ChatPage {
    serde_json::from_str(&next_of(sse, "page").await).unwrap()
}

async fn items_of(sse: &mut SseClient) -> Vec<ChatItem> {
    serde_json::from_str(&next_of(sse, "items").await).unwrap()
}

fn calls(items: &[ChatItem]) -> Vec<(u64, CallStatus, Option<&str>)> {
    items
        .iter()
        .filter_map(|i| match &i.body {
            ChatBody::Call { status, result, .. } => Some((i.seq, *status, result.as_deref())),
            _ => None,
        })
        .collect()
}

fn is_marker(i: &ChatItem, kind: MarkerKind) -> bool {
    i.body == ChatBody::Marker { kind }
}

#[tokio::test]
async fn streams_the_latest_page_then_new_items() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let head = up_to_first_call();
    let s1 = own(&srv, &a.id).await;
    session(&srv, &a.id, &s1, 10, &head).await;
    let mut sse = SseClient::connect(srv.addr, &chat_path(&a.id), &[]).await;
    let page = page_of(&mut sse).await;
    assert_eq!(page.earlier, 0);
    assert!(matches!(page.items[0].body, ChatBody::Prompt { .. }));
    let running = calls(&page.items);
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].1, CallStatus::Running);
    let seqs: Vec<u64> = page.items.iter().map(|i| i.seq).collect();
    assert_eq!(seqs, (0..page.items.len() as u64).collect::<Vec<_>>());

    let rest = fixture_lines("tools")[head.len()..].to_vec();
    append(&transcript(&srv, &s1), &rest);
    let mut fresh = Vec::new();
    while !fresh.iter().any(|i: &ChatItem| i.seq == running[0].0) {
        fresh.extend(items_of(&mut sse).await);
    }
    let updated = fresh.iter().find(|i| i.seq == running[0].0).unwrap();
    assert_eq!(calls(std::slice::from_ref(updated))[0].1, CallStatus::Done);
    assert!(fresh.iter().any(|i| i.seq >= page.items.len() as u64));
}

#[tokio::test]
async fn joins_sessions_with_a_new_session_marker() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let s1 = own(&srv, &a.id).await;
    session(&srv, &a.id, &s1, 10, &up_to_first_call()).await;
    let mut sse = SseClient::connect(srv.addr, &chat_path(&a.id), &[]).await;
    let page = page_of(&mut sse).await;
    let (call_seq, status, _) = calls(&page.items)[0];
    assert_eq!(status, CallStatus::Running);

    session(&srv, &a.id, "s-2", 2_000, &fixture_lines("clear-after")).await;
    let mut fresh = Vec::new();
    while !fresh
        .iter()
        .any(|i: &ChatItem| matches!(i.body, ChatBody::Prompt { .. }))
    {
        fresh.extend(items_of(&mut sse).await);
    }
    // The first session's open call ended with it.
    assert_eq!(
        calls(&fresh)[0],
        (call_seq, CallStatus::Failed, Some("interrupted"))
    );
    let marker = fresh
        .iter()
        .position(|i| is_marker(i, MarkerKind::NewSession))
        .expect("a new-session marker");
    assert_eq!(fresh[marker].at, 2_000);
    assert_eq!(fresh[marker].seq, page.items.len() as u64);
    assert!(
        matches!(&fresh[marker + 1].body, ChatBody::Prompt { text, .. } if text.starts_with("Read notes.txt")),
        "{fresh:?}"
    );

    // A fresh stream builds the same thing at once.
    let mut again = SseClient::connect(srv.addr, &chat_path(&a.id), &[]).await;
    let whole = page_of(&mut again).await.items;
    let markers: Vec<_> = whole
        .iter()
        .filter(|i| is_marker(i, MarkerKind::NewSession))
        .collect();
    assert_eq!(markers.len(), 1);
    assert_eq!(
        calls(&whole)[0],
        (call_seq, CallStatus::Failed, Some("interrupted"))
    );
}

#[tokio::test]
async fn a_stopped_agent_gets_its_whole_session_then_ended() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let s1 = own(&srv, &a.id).await;
    session(&srv, &a.id, &s1, 10, &up_to_first_call()).await;
    srv.fx.svc.stop(&a.id).await.unwrap();
    let since = srv.fx.svc.agent(&a.id).await.unwrap().status_since;
    // The stream ends by itself, so the plain request returns with the whole body.
    let (status, body) = http(srv.addr, "GET", &chat_path(&a.id), &[], None).await;
    assert_eq!(status, 200);
    let events: Vec<&str> = body.lines().filter(|l| l.starts_with("event:")).collect();
    assert_eq!(events, ["event: page", "event: ended"], "{body}");
    let data: Vec<&str> = body
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .collect();
    let page: ChatPage = serde_json::from_str(data[0]).unwrap();
    let last = page.items.last().unwrap();
    assert!(is_marker(last, MarkerKind::SessionEnded), "{last:?}");
    assert_eq!(last.at, since);
    assert_eq!(calls(&page.items)[0].1, CallStatus::Failed);
    assert_eq!(calls(&page.items)[0].2, Some("interrupted"));
    assert_eq!(serde_json::from_str::<i64>(data[1]).unwrap(), since);
}

#[tokio::test]
async fn history_pages_backwards() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let lines: Vec<String> = (0..250).map(|n| prompt_line(&format!("p{n}"))).collect();
    let s1 = own(&srv, &a.id).await;
    session(&srv, &a.id, &s1, 10, &lines).await;
    let history = format!("/api/agents/{}/chat/history", a.id.0);
    let seqs = |p: &ChatPage| p.items.iter().map(|i| i.seq).collect::<Vec<_>>();

    let (status, v) = json_of(srv.addr, "GET", &history, None).await;
    assert_eq!(status, 200, "{v}");
    let latest: ChatPage = serde_json::from_value(v).unwrap();
    assert_eq!(seqs(&latest), (50..250).collect::<Vec<_>>());
    assert_eq!(latest.earlier, 50);

    let (_, v) = json_of(
        srv.addr,
        "GET",
        &format!("{history}?before=50&limit=30"),
        None,
    )
    .await;
    let older: ChatPage = serde_json::from_value(v).unwrap();
    assert_eq!(seqs(&older), (20..50).collect::<Vec<_>>());
    assert_eq!(older.earlier, 20);

    let (_, v) = json_of(srv.addr, "GET", &format!("{history}?before=20"), None).await;
    let first: ChatPage = serde_json::from_value(v).unwrap();
    assert_eq!(seqs(&first), (0..20).collect::<Vec<_>>());
    assert_eq!(first.earlier, 0);

    // The stream opens on the same latest page.
    let mut sse = SseClient::connect(srv.addr, &chat_path(&a.id), &[]).await;
    assert_eq!(page_of(&mut sse).await, latest);
}

#[tokio::test]
async fn no_transcript_yet_is_an_empty_page() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let s1 = own(&srv, &a.id).await;
    std::fs::remove_file(transcript(&srv, &s1)).unwrap();
    session(&srv, &a.id, &s1, 10, &[]).await;
    let mut sse = SseClient::connect(srv.addr, &chat_path(&a.id), &[]).await;
    assert_eq!(
        page_of(&mut sse).await,
        ChatPage {
            items: vec![],
            earlier: 0
        }
    );
    append(&transcript(&srv, &s1), &[prompt_line(PROMPT_POINTER)]);
    let items = items_of(&mut sse).await;
    let prompt_md = std::fs::read_to_string(PathBuf::from(&a.workdir).join(PROMPT_FILE)).unwrap();
    assert_eq!(
        items[0].body,
        ChatBody::Prompt {
            text: prompt_md,
            from: PromptFrom::Owner
        }
    );
}

#[tokio::test]
async fn a_new_agents_chat_knows_its_session_before_any_hook() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let s1 = own(&srv, &a.id).await;
    let sessions = srv.fx.store.claude_sessions(&a.id).await.unwrap();
    assert_eq!(sessions, [(s1.clone(), srv.fx.clock.now_for_test())]);
    append(&transcript(&srv, &s1), &[prompt_line("hello")]);
    let mut sse = SseClient::connect(srv.addr, &chat_path(&a.id), &[]).await;
    let page = page_of(&mut sse).await;
    assert_eq!(
        page.items.iter().map(|i| &i.body).collect::<Vec<_>>(),
        [&ChatBody::Prompt {
            text: "hello".into(),
            from: PromptFrom::Owner
        }]
    );
}

#[tokio::test]
async fn unknown_agent_is_404() {
    let srv = start().await;
    let (s, _) = http(srv.addr, "GET", "/api/agents/ghost/chat", &[], None).await;
    assert_eq!(s, 404);
    let (s, _) = http(srv.addr, "GET", "/api/agents/ghost/chat/history", &[], None).await;
    assert_eq!(s, 404);
}

/// The next `ask` event. Bounded here: the stream's keep-alives would keep `SseClient::next` waiting forever.
async fn ask_of(sse: &mut SseClient) -> Option<ChatAsk> {
    let data = tokio::time::timeout(Duration::from_secs(20), next_of(sse, "ask"))
        .await
        .expect("no ask event within 20s");
    serde_json::from_str(&data).unwrap()
}

/// Reads `ask` events until one satisfies `pred`.
async fn ask_until(
    sse: &mut SseClient,
    pred: impl Fn(&Option<ChatAsk>) -> bool,
) -> Option<ChatAsk> {
    loop {
        let ask = ask_of(sse).await;
        if pred(&ask) {
            return ask;
        }
    }
}

fn hook_path(id: &AgentId) -> String {
    format!("/internal/hooks/{}", id.0)
}

/// Sends a hook the way Claude does, on its own connection; returns that connection, the response not yet read.
async fn send_hook(srv: &Server, id: &AgentId, body: &serde_json::Value) -> TcpStream {
    let token = srv.fx.store.agent_token(id).await.unwrap().unwrap();
    let mut s = TcpStream::connect(srv.addr).await.unwrap();
    let text = request_text(
        srv.addr,
        "POST",
        &hook_path(id),
        &[("X-Fleet-Token", &token)],
        Some(&body.to_string()),
    );
    s.write_all(text.as_bytes()).await.unwrap();
    s
}

/// The status and JSON body of a hook's response.
async fn hook_reply(mut s: TcpStream) -> (u16, serde_json::Value) {
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(20), s.read_to_string(&mut raw))
        .await
        .expect("the hook never answered")
        .unwrap();
    let status = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = raw.split_once("\r\n\r\n").unwrap().1;
    (status, serde_json::from_str(body).unwrap())
}

async fn hook(srv: &Server, id: &AgentId, body: serde_json::Value) -> (u16, serde_json::Value) {
    hook_reply(send_hook(srv, id, &body).await).await
}

fn permission_request(
    tool: &str,
    input: serde_json::Value,
    suggestions: serde_json::Value,
) -> serde_json::Value {
    json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": tool,
        "tool_input": input,
        "permission_suggestions": suggestions,
    })
}

fn python_request() -> serde_json::Value {
    permission_request(
        "Bash",
        json!({"command": "python3 -c \"print(42)\"", "description": "Print 42"}),
        json!([{"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "python3 -c \"print(42)\""}],
                "behavior": "allow", "destination": "localSettings"}]),
    )
}

fn questions_input() -> serde_json::Value {
    json!({"questions": [
        {"question": "Which color do you like?", "header": "Color", "multiSelect": false, "options": [
            {"label": "Red", "description": "Choose red."},
            {"label": "Green", "description": "Choose green."},
            {"label": "Blue", "description": "Choose blue."}]},
        {"question": "Which fruits do you like?", "header": "Fruits", "multiSelect": true, "options": [
            {"label": "Apple", "description": "Choose apple."},
            {"label": "Banana", "description": "Choose banana."},
            {"label": "Cherry", "description": "Choose cherry."}]}
    ]})
}

fn ask_path(id: &AgentId, ask: &str) -> String {
    format!("/api/agents/{}/asks/{ask}", id.0)
}

async fn answer(srv: &Server, id: &AgentId, ask: &str, body: serde_json::Value) -> u16 {
    http(srv.addr, "POST", &ask_path(id, ask), &[], Some(&body))
        .await
        .0
}

fn option(id: &str, label: &str) -> AskOption {
    AskOption {
        id: id.into(),
        label: label.into(),
        description: None,
    }
}

/// A question's option, with the description Claude gave it.
fn described(id: &str, label: &str) -> AskOption {
    AskOption {
        description: Some(format!("Choose {}.", label.to_lowercase())),
        ..option(id, label)
    }
}

/// Opens the agent's chat and reads past its first page and its opening `ask`.
async fn open_chat(srv: &Server, id: &AgentId) -> (SseClient, Option<ChatAsk>) {
    let mut sse = SseClient::connect(srv.addr, &chat_path(id), &[]).await;
    page_of(&mut sse).await;
    let ask = ask_of(&mut sse).await;
    (sse, ask)
}

#[tokio::test]
async fn a_permission_is_held_until_the_owner_decides() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, first) = open_chat(&srv, &a.id).await;
    assert_eq!(first, None);

    let mut held = tokio::spawn({
        let conn = send_hook(&srv, &a.id, &python_request()).await;
        hook_reply(conn)
    });
    let ask = ask_until(&mut sse, Option::is_some).await.unwrap();
    assert_eq!(
        ask.kind,
        AskKind::Permission {
            tool: "Bash".into(),
            input: json!({"command": "python3 -c \"print(42)\"", "description": "Print 42"}),
            options: vec![
                option("allow", "Allow once"),
                option("allow_session", "Allow for this session"),
                option("deny", "Deny"),
            ],
        }
    );
    assert_eq!(ask.answered, None);
    wait_agent(&srv.fx.svc, &a.id, |x| x.status == Status::NeedsYou).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut held)
            .await
            .is_err(),
        "the hook answered before the owner did"
    );

    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "allow_session"})).await,
        204
    );
    let (status, decision) = held.await.unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        decision,
        json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {
            "behavior": "allow",
            "updatedPermissions": [{"type": "addRules",
                "rules": [{"toolName": "Bash", "ruleContent": "python3 -c \"print(42)\""}],
                "behavior": "allow", "destination": "session"}]}}})
    );
    let answered = ask_of(&mut sse).await.unwrap();
    assert_eq!(answered.id, ask.id);
    assert_eq!(answered.answered.as_deref(), Some("Allow for this session"));

    // The tool runs, and the prompt is gone.
    hook(
        &srv,
        &a.id,
        json!({"hook_event_name": "PostToolUse", "tool_name": "Bash"}),
    )
    .await;
    assert_eq!(ask_of(&mut sse).await, None);
}

#[tokio::test]
async fn allow_once_and_deny_answer_with_their_decisions() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    // No suggested rules: no session option.
    let request = permission_request("Bash", json!({"command": "ls"}), json!([]));
    for (choice, decision) in [
        ("allow", json!({"behavior": "allow"})),
        (
            "deny",
            json!({"behavior": "deny", "message": "The owner denied this."}),
        ),
    ] {
        let held = tokio::spawn({
            let conn = send_hook(&srv, &a.id, &request).await;
            hook_reply(conn)
        });
        let ask = ask_until(&mut sse, |a| {
            a.as_ref().is_some_and(|a| a.answered.is_none())
        })
        .await
        .unwrap();
        let AskKind::Permission { options, .. } = &ask.kind else {
            panic!("{ask:?}");
        };
        assert_eq!(
            options,
            &[option("allow", "Allow once"), option("deny", "Deny")]
        );
        assert_eq!(
            answer(&srv, &a.id, &ask.id, json!({"choice": choice})).await,
            204
        );
        assert_eq!(
            held.await.unwrap().1,
            json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}})
        );
        hook(
            &srv,
            &a.id,
            json!({"hook_event_name": "PostToolUse", "tool_name": "Bash"}),
        )
        .await;
    }
}

#[tokio::test]
async fn the_second_answer_gets_409() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    let held = tokio::spawn({
        let conn = send_hook(&srv, &a.id, &python_request()).await;
        hook_reply(conn)
    });
    let ask = ask_until(&mut sse, Option::is_some).await.unwrap();
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "nope"})).await,
        400
    );
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "deny"})).await,
        204
    );
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "allow"})).await,
        409
    );
    held.await.unwrap();
    hook(
        &srv,
        &a.id,
        json!({"hook_event_name": "PostToolUse", "tool_name": "Bash"}),
    )
    .await;
    // Still 409 once the ask has cleared, not 404.
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "allow"})).await,
        409
    );
    assert_eq!(
        answer(&srv, &a.id, "ghost", json!({"choice": "allow"})).await,
        404
    );
    assert_eq!(
        http(
            srv.addr,
            "POST",
            "/api/agents/ghost/asks/x",
            &[],
            Some(&json!({"choice": "allow"}))
        )
        .await
        .0,
        404
    );
}

#[tokio::test]
async fn a_dropped_hook_falls_back_to_the_screen() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    let conn = send_hook(&srv, &a.id, &python_request()).await;
    let ask = ask_until(&mut sse, Option::is_some).await.unwrap();
    wait_agent(&srv.fx.svc, &a.id, |x| x.status == Status::NeedsYou).await;
    drop(conn);
    let fallback = ask_until(&mut sse, |a| a.as_ref() != Some(&ask))
        .await
        .unwrap();
    let AskKind::Screen { text } = &fallback.kind else {
        panic!("{fallback:?}");
    };
    assert!(text.contains("ready"), "{text}");
    assert_eq!(agent(&srv.fx, &a.id).await.status, Status::NeedsYou);
    // The reply box answers it, not this endpoint.
    assert_eq!(
        answer(&srv, &a.id, &fallback.id, json!({"choice": "allow"})).await,
        400
    );
    // The old ask is gone.
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "allow"})).await,
        404
    );
}

#[tokio::test]
async fn a_hook_held_too_long_answers_nothing() {
    let srv = start_holding(Some(Duration::from_millis(300))).await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    let (status, body) = hook(&srv, &a.id, python_request()).await;
    assert_eq!((status, body), (200, json!({})));
    let ask = ask_until(&mut sse, |a| {
        a.as_ref()
            .is_some_and(|a| matches!(a.kind, AskKind::Screen { .. }))
    })
    .await;
    assert!(ask.is_some());
}

#[tokio::test]
async fn a_question_is_answered_through_its_hook() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    hook(&srv, &a.id, json!({"hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion", "tool_input": questions_input()})).await;
    let held = tokio::spawn({
        let conn = send_hook(
            &srv,
            &a.id,
            &permission_request("AskUserQuestion", questions_input(), json!([])),
        )
        .await;
        hook_reply(conn)
    });
    let ask = ask_until(&mut sse, Option::is_some).await.unwrap();
    assert_eq!(
        ask.kind,
        AskKind::Question {
            questions: vec![
                AskQuestion {
                    header: "Color".into(),
                    question: "Which color do you like?".into(),
                    options: vec![
                        described("0.0", "Red"),
                        described("0.1", "Green"),
                        described("0.2", "Blue")
                    ],
                    multi_select: false,
                },
                AskQuestion {
                    header: "Fruits".into(),
                    question: "Which fruits do you like?".into(),
                    options: vec![
                        described("1.0", "Apple"),
                        described("1.1", "Banana"),
                        described("1.2", "Cherry")
                    ],
                    multi_select: true,
                },
            ]
        }
    );
    // One color only.
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "0.0,0.1,1.0"})).await,
        400
    );
    // Every question needs an answer.
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "0.1"})).await,
        400
    );
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "0.1,1.2,1.0"})).await,
        204
    );
    let mut input = questions_input();
    input["answers"] =
        json!({"Which color do you like?": "Green", "Which fruits do you like?": "Apple, Cherry"});
    assert_eq!(
        held.await.unwrap().1,
        json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest",
            "decision": {"behavior": "allow", "updatedInput": input}}})
    );
    let answered = ask_of(&mut sse).await.unwrap();
    assert_eq!(answered.answered.as_deref(), Some("Green; Apple, Cherry"));
    hook(
        &srv,
        &a.id,
        json!({"hook_event_name": "PostToolUse", "tool_name": "AskUserQuestion"}),
    )
    .await;
    assert_eq!(ask_of(&mut sse).await, None);
}

#[tokio::test]
async fn a_question_takes_free_text() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    let held = tokio::spawn({
        let conn = send_hook(
            &srv,
            &a.id,
            &permission_request("AskUserQuestion", questions_input(), json!([])),
        )
        .await;
        hook_reply(conn)
    });
    let ask = ask_until(&mut sse, Option::is_some).await.unwrap();
    assert_eq!(
        answer(
            &srv,
            &a.id,
            &ask.id,
            json!({"choice": "1.1", "text": "Purple"})
        )
        .await,
        204
    );
    let decision = held.await.unwrap().1;
    assert_eq!(
        decision["hookSpecificOutput"]["decision"]["updatedInput"]["answers"],
        json!({"Which color do you like?": "Purple", "Which fruits do you like?": "Banana"})
    );
}

#[tokio::test]
async fn needs_you_with_no_hook_data_shows_the_screen() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    srv.fx
        .store
        .update_agent(&a.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::Question);
        })
        .await
        .unwrap();
    let (mut sse, ask) = open_chat(&srv, &a.id).await;
    let ask = ask.unwrap();
    let AskKind::Screen { text } = &ask.kind else {
        panic!("{ask:?}");
    };
    assert!(text.contains("ready") && !text.contains("zz"), "{text}");
    // The screen follows the terminal.
    srv.fx
        .sessions
        .write_text_async(&a.session, "zz")
        .await
        .unwrap();
    ask_until(&mut sse, |a| {
        matches!(a, Some(ChatAsk { kind: AskKind::Screen { text }, .. }) if text.contains("zz"))
    })
    .await;
    // Back to work: no prompt.
    set_status(&srv.fx, &a.id, Status::Working).await;
    hook(&srv, &a.id, json!({"hook_event_name": "UserPromptSubmit"})).await;
    assert_eq!(ask_until(&mut sse, Option::is_none).await, None);
}

/// Holds `body` as Claude would, returning the hook's eventual reply.
async fn held(
    srv: &Server,
    id: &AgentId,
    body: serde_json::Value,
) -> tokio::task::JoinHandle<(u16, serde_json::Value)> {
    let conn = send_hook(srv, id, &body).await;
    tokio::spawn(hook_reply(conn))
}

async fn still_held(h: &mut tokio::task::JoinHandle<(u16, serde_json::Value)>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(300), h)
            .await
            .is_err(),
        "the held hook answered"
    );
}

#[tokio::test]
async fn another_hook_never_clears_an_unanswered_prompt() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    let mut h = held(&srv, &a.id, python_request()).await;
    let ask = ask_until(&mut sse, Option::is_some).await.unwrap();
    // A subagent or a parallel call keeps working while the main thread waits.
    for event in ["PreToolUse", "PostToolUse", "SubagentStart", "Stop"] {
        let (status, _) = hook(
            &srv,
            &a.id,
            json!({"hook_event_name": event, "tool_name": "Read"}),
        )
        .await;
        assert_eq!(status, 200, "{event}");
    }
    still_held(&mut h).await;
    let (_, now) = open_chat(&srv, &a.id).await;
    assert_eq!(now, Some(ask.clone()));
    assert_eq!(
        answer(&srv, &a.id, &ask.id, json!({"choice": "allow"})).await,
        204
    );
    assert_eq!(
        h.await.unwrap().1["hookSpecificOutput"]["decision"],
        json!({"behavior": "allow"})
    );
}

#[tokio::test]
async fn a_second_request_queues_behind_the_first() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    let first = held(&srv, &a.id, python_request()).await;
    let one = ask_until(&mut sse, Option::is_some).await.unwrap();
    let mut second = held(
        &srv,
        &a.id,
        permission_request("Bash", json!({"command": "ls"}), json!([])),
    )
    .await;
    still_held(&mut second).await;
    // The oldest unanswered prompt is the one shown.
    let (_, now) = open_chat(&srv, &a.id).await;
    assert_eq!(now.as_ref().map(|x| &x.id), Some(&one.id));

    assert_eq!(
        answer(&srv, &a.id, &one.id, json!({"choice": "allow"})).await,
        204
    );
    assert_eq!(
        first.await.unwrap().1["hookSpecificOutput"]["decision"],
        json!({"behavior": "allow"})
    );
    let answered = ask_of(&mut sse).await.unwrap();
    assert_eq!(
        (answered.id.as_str(), answered.answered.as_deref()),
        (one.id.as_str(), Some("Allow once"))
    );
    let two = ask_of(&mut sse).await.unwrap();
    assert_ne!(two.id, one.id);
    assert!(
        matches!(&two.kind, AskKind::Permission { input, .. } if input == &json!({"command": "ls"})),
        "{two:?}"
    );
    still_held(&mut second).await;
    assert_eq!(
        answer(&srv, &a.id, &two.id, json!({"choice": "deny"})).await,
        204
    );
    assert_eq!(
        second.await.unwrap().1["hookSpecificOutput"]["decision"]["behavior"],
        json!("deny")
    );
}

#[tokio::test]
async fn an_ended_agents_prompts_are_released() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let (mut sse, _) = open_chat(&srv, &a.id).await;
    let h = held(&srv, &a.id, python_request()).await;
    ask_until(&mut sse, Option::is_some).await;
    hook(&srv, &a.id, json!({"hook_event_name": "SessionEnd"})).await;
    let reply = tokio::time::timeout(Duration::from_secs(5), h)
        .await
        .expect("the hook stayed held")
        .unwrap();
    assert_eq!(reply, (200, json!({})));
}
