mod common;

use common::net::*;
use common::*;
use fleet_api::{AgentId, CallStatus, ChatBody, ChatItem, ChatPage, MarkerKind, PromptFrom};
use fleet_daemon::agents::{PROMPT_FILE, PROMPT_POINTER};
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::json;
use std::io::Write;
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

async fn next_of(sse: &mut SseClient, kind: &str) -> String {
    let ev = sse.next().await;
    assert_eq!(ev.event.as_deref(), Some(kind), "{ev:?}");
    ev.data
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
