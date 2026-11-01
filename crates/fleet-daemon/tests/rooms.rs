mod common;

use common::*;
use fleet_api::{
    AgentId, AgentKind, EventKind, MemberRef, Post, PostKind, PrState, Room, RoomKind, Status,
    TaskPr, TaskState,
};
use fleet_daemon::agents::AgentService;
use fleet_daemon::session::SessionHost;
use fleet_daemon::supervisor::Supervisor;
use fleet_daemon::{mcp, rooms};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

async fn conversation(fx: &Fx) -> AgentId {
    Supervisor::new(fx.svc.clone())
        .new_conversation(PERSONA, "what is going on?")
        .await
        .unwrap()
}

/// A started orchestrator task; `origin` is the conversation that started it, as `start_task` records.
async fn orchestrator(fx: &Fx, origin: Option<&AgentId>) -> AgentId {
    let a = spawn_ready(fx).await;
    let origin = origin.cloned();
    fx.store
        .update_task(&a.id, |t| {
            t.kind = AgentKind::Orchestrator;
            t.origin = origin;
        })
        .await
        .unwrap();
    rooms::sync_task(&fx.store, 10, &a.id).await.unwrap();
    a.id
}

async fn members(fx: &Fx, room: &str) -> Vec<MemberRef> {
    fx.store
        .room_members(room)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.left_at.is_none())
        .map(|m| MemberRef::from_key(&m.member))
        .collect()
}

async fn room_of(fx: &Fx, kind: RoomKind) -> Vec<Room> {
    rooms::list(&fx.store)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == kind)
        .collect()
}

fn pr(state: PrState) -> Vec<TaskPr> {
    vec![TaskPr {
        url: "https://example.test/pr/1".into(),
        repo: "core".into(),
        state,
        merged_at: None,
    }]
}

async fn set_prs(fx: &Fx, id: &AgentId, state: PrState) {
    fx.store
        .update_task(id, |t| t.prs = pr(state))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_planned_task_joins_its_product_and_operation_rooms() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let task = orchestrator(&fx, Some(&sup)).await;

    let product = room_of(&fx, RoomKind::Product).await;
    assert_eq!(product.len(), 1);
    assert_eq!(product[0].product.as_deref(), Some(PRODUCT));
    let product_name = fx.store.product(PRODUCT).await.unwrap().unwrap().name;
    assert_eq!(product[0].title, product_name);
    assert_eq!(
        members(&fx, &product[0].id).await,
        vec![MemberRef::Agent(task.clone())]
    );

    let op = room_of(&fx, RoomKind::Operation).await;
    assert_eq!(op.len(), 1);
    assert_eq!(op[0].conversation, Some(sup.clone()));
    assert_eq!(op[0].persona.as_deref(), Some(PERSONA));
    assert_eq!(op[0].title, "Untitled");
    let mut got = members(&fx, &op[0].id).await;
    got.sort_by_key(|m| m.key().to_string());
    let mut want = vec![MemberRef::Agent(sup), MemberRef::Agent(task)];
    want.sort_by_key(|m| m.key().to_string());
    assert_eq!(got, want);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_operation_room_is_titled_after_its_conversation() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    fx.store
        .update_agent(&sup, |a| a.title = Some("Ship the thing".into()))
        .await
        .unwrap();
    orchestrator(&fx, Some(&sup)).await;
    assert_eq!(
        room_of(&fx, RoomKind::Operation).await[0].title,
        "Ship the thing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hand_made_task_joins_only_its_product_room() {
    let fx = fixture().await;
    let task = orchestrator(&fx, None).await;
    let product = room_of(&fx, RoomKind::Product).await;
    assert_eq!(product.len(), 1);
    assert_eq!(
        members(&fx, &product[0].id).await,
        vec![MemberRef::Agent(task)]
    );
    assert!(room_of(&fx, RoomKind::Operation).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_solo_task_joins_no_room() {
    let fx = fixture().await;
    spawn_ready(&fx).await;
    rooms::sync(&fx.store, 10).await.unwrap();
    assert!(rooms::list(&fx.store).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_product_room_is_made_once() {
    let fx = fixture().await;
    let a = rooms::product_room(&fx.store, PRODUCT).await.unwrap();
    let b = rooms::product_room(&fx.store, PRODUCT).await.unwrap();
    assert_eq!(a, b);
    assert_eq!(room_of(&fx, RoomKind::Product).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_operation_room_archives_when_every_task_has_ended() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let one = orchestrator(&fx, Some(&sup)).await;
    let two = orchestrator(&fx, Some(&sup)).await;
    assert_eq!(room_of(&fx, RoomKind::Operation).await.len(), 1);

    set_prs(&fx, &one, PrState::Merged).await;
    rooms::sync(&fx.store, 20).await.unwrap();
    let op = &room_of(&fx, RoomKind::Operation).await[0];
    assert_eq!(op.archived_at, None, "one task is still running");
    assert!(!members(&fx, &op.id)
        .await
        .contains(&MemberRef::Agent(one.clone())));
    assert!(members(&fx, &op.id)
        .await
        .contains(&MemberRef::Agent(two.clone())));

    fx.store
        .update_task(&two, |t| t.state = TaskState::Archived)
        .await
        .unwrap();
    rooms::sync(&fx.store, 30).await.unwrap();
    let op = &room_of(&fx, RoomKind::Operation).await[0];
    assert_eq!(op.archived_at, Some(30));
    assert_eq!(members(&fx, &op.id).await, vec![MemberRef::Agent(sup)]);
    // Neither task is in the product room any more, which stays.
    let product = &room_of(&fx, RoomKind::Product).await[0];
    assert!(members(&fx, &product.id).await.is_empty());
    assert_eq!(product.archived_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_pr_ends_the_task_too() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let task = orchestrator(&fx, Some(&sup)).await;
    set_prs(&fx, &task, PrState::Closed).await;
    rooms::sync(&fx.store, 20).await.unwrap();
    assert_eq!(
        room_of(&fx, RoomKind::Operation).await[0].archived_at,
        Some(20)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_open_pr_at_review_keeps_the_room_open() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let task = orchestrator(&fx, Some(&sup)).await;
    set_prs(&fx, &task, PrState::Open).await;
    fx.store
        .update_agent(&task, |a| a.status = fleet_api::Status::Review)
        .await
        .unwrap();
    rooms::sync(&fx.store, 20).await.unwrap();
    let op = &room_of(&fx, RoomKind::Operation).await[0];
    assert_eq!(op.archived_at, None);
    assert!(members(&fx, &op.id).await.contains(&MemberRef::Agent(task)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_task_from_the_same_conversation_reopens_the_archived_room() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let first = orchestrator(&fx, Some(&sup)).await;
    set_prs(&fx, &first, PrState::Merged).await;
    rooms::sync(&fx.store, 20).await.unwrap();
    let before = room_of(&fx, RoomKind::Operation).await;
    assert_eq!(before[0].archived_at, Some(20));

    let second = orchestrator(&fx, Some(&sup)).await;
    let after = room_of(&fx, RoomKind::Operation).await;
    assert_eq!(after.len(), 1, "no second room");
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[0].archived_at, None);
    assert!(members(&fx, &after[0].id)
        .await
        .contains(&MemberRef::Agent(second)));
}

#[tokio::test(flavor = "multi_thread")]
async fn joining_and_leaving_are_repeatable() {
    let fx = fixture().await;
    let room = rooms::product_room(&fx.store, PRODUCT).await.unwrap();
    let who = MemberRef::Agent(AgentId("a1".into()));
    rooms::join(&fx.store, &room, &who, 1).await.unwrap();
    rooms::join(&fx.store, &room, &who, 2).await.unwrap();
    assert_eq!(members(&fx, &room).await, vec![who.clone()]);
    rooms::leave(&fx.store, &room, &who, 3).await.unwrap();
    rooms::leave(&fx.store, &room, &who, 4).await.unwrap();
    assert!(members(&fx, &room).await.is_empty());
    rooms::join(&fx.store, &room, &who, 5).await.unwrap();
    assert_eq!(members(&fx, &room).await, vec![who]);
}

#[test]
fn members_have_one_wire_and_store_form() {
    assert_eq!(
        serde_json::to_string(&MemberRef::Owner).unwrap(),
        "\"owner\""
    );
    assert_eq!(
        serde_json::to_string(&MemberRef::Daemon).unwrap(),
        "\"daemon\""
    );
    let agent = MemberRef::Agent(AgentId("x".into()));
    assert_eq!(serde_json::to_string(&agent).unwrap(), "{\"agent\":\"x\"}");
    for m in [MemberRef::Owner, MemberRef::Daemon, agent] {
        assert_eq!(MemberRef::from_key(m.key()), m);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_watcher_follows_a_task_to_its_end() {
    let fx = fixture().await;
    rooms::start_room_watcher(fx.svc.clone());
    let sup = conversation(&fx).await;
    let task = orchestrator(&fx, Some(&sup)).await;
    set_prs(&fx, &task, PrState::Merged).await;
    // A task event is what the merge checker emits when a PR merges.
    fleet_daemon::supervisor::route(
        &fx.svc,
        &task,
        fleet_api::TaskEventKind::PrMerged,
        "merged".into(),
    )
    .await
    .unwrap();
    assert!(
        wait_until(|| async {
            room_of(&fx, RoomKind::Operation).await[0]
                .archived_at
                .is_some()
        })
        .await
    );
}

// Posting, reading, asking and answering.

/// A running `direct` orchestrator on `core`, started by `origin`, in its rooms and at its input box.
async fn direct(fx: &Fx, origin: &AgentId) -> AgentId {
    if !fx.repo.join(".fleet").exists() {
        add_origin_with_workflow(fx, &one_step("true"));
    }
    let a = fx
        .svc
        .spawn(fleet_api::SpawnRequest {
            plan: None,
            actor: fleet_api::Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(fx, "core")],
            prompt: "do the thing".into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: AgentKind::Orchestrator,
            role: Some("direct".into()),
        })
        .await
        .unwrap();
    assert!(pane_contains(fx, &a.id, "ready").await);
    let origin = Some(origin.clone());
    fx.store
        .update_task(&a.id, |t| t.origin = origin)
        .await
        .unwrap();
    rooms::sync_task(&fx.store, 10, &a.id).await.unwrap();
    set_status(fx, &a.id, Status::Working).await;
    a.id
}

/// Calls the fleet MCP tool `name` as `caller`; returns its text and whether it is an error.
async fn tool(
    svc: &Arc<AgentService>,
    caller: &AgentId,
    name: &str,
    args: Value,
) -> (String, bool) {
    let r = mcp::handle(
        svc,
        caller,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": args },
        }),
    )
    .await;
    let result = &r["result"];
    (
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{r}"))
            .to_string(),
        result["isError"].as_bool().unwrap(),
    )
}

fn posted_id(reply: &(String, bool)) -> i64 {
    assert!(!reply.1, "{reply:?}");
    reply
        .0
        .strip_prefix("posted ")
        .unwrap_or_else(|| panic!("{reply:?}"))
        .parse()
        .unwrap()
}

fn op_room(sup: &AgentId) -> String {
    format!("operation:{}", sup.0)
}

/// The asks stored in `room`, oldest first.
async fn asks(fx: &Fx, room: &str) -> Vec<Post> {
    fx.store
        .posts_after(room, "nobody", 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|p| p.kind == PostKind::Ask)
        .collect()
}

/// Waits until `room` holds an ask; returns its id.
async fn wait_ask(fx: &Fx, room: &str) -> i64 {
    assert!(wait_until(|| async { !asks(fx, room).await.is_empty() }).await);
    asks(fx, room).await[0].id
}

/// `caller`'s `room_ask`, held on its own task.
fn ask_in_background(
    svc: &Arc<AgentService>,
    caller: &AgentId,
    to: &str,
    text: &str,
) -> tokio::task::JoinHandle<(String, bool)> {
    let (svc, caller) = (svc.clone(), caller.clone());
    let args = json!({ "to": to, "text": text });
    tokio::spawn(async move { tool(&svc, &caller, "room_ask", args).await })
}

/// A service on the fixture's store and sessions whose held calls give up after `hold`.
fn with_hold(fx: &Fx, hold: Duration) -> Arc<AgentService> {
    let mut opts = fx.svc.options().clone();
    opts.repo_request_hold = hold;
    Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.sessions.clone(),
        opts,
    ))
}

const NOTICE: &str = "Fleet Manager:";

fn screen(sessions: &SessionHost, id: &AgentId) -> String {
    sessions.screen_text(&session_name(id)).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn announcements_wake_nobody_and_show_in_room_read() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let mut live = fx.store.subscribe();

    let id = posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "announcement", "text": "migration 017 is mine" }),
        )
        .await,
    );

    let ev = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let EventKind::RoomPost { post } = live.recv().await.unwrap().kind {
                return post;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        (ev.id, ev.room.as_str(), &ev.sender, ev.kind, ev.to.as_ref()),
        (
            id,
            op_room(&sup).as_str(),
            &MemberRef::Agent(a.clone()),
            PostKind::Announcement,
            None
        )
    );

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!screen(&fx.sessions, &sup).contains("migration 017"));
    assert!(!screen(&fx.sessions, &a).contains(NOTICE));

    let (read, failed) = tool(&fx.svc, &sup, "room_read", json!({})).await;
    assert!(!failed, "{read}");
    assert_eq!(
        read,
        format!("{id} {} → all: [announcement] migration 017 is mine", a.0)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn room_read_returns_only_what_is_new_for_that_member() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let post = |who: &AgentId, args: Value| {
        let (svc, who) = (fx.svc.clone(), who.clone());
        async move { posted_id(&tool(&svc, &who, "room_post", args).await) }
    };

    let one = post(&a, json!({ "kind": "announcement", "text": "one" })).await;
    let read = tool(&fx.svc, &sup, "room_read", json!({})).await.0;
    assert!(read.starts_with(&format!("{one} ")), "{read}");
    assert_eq!(
        tool(&fx.svc, &sup, "room_read", json!({})).await.0,
        "nothing new"
    );

    let two = post(&a, json!({ "kind": "announcement", "text": "two" })).await;
    let read = tool(&fx.svc, &sup, "room_read", json!({})).await.0;
    assert_eq!(read, format!("{two} {} → all: [announcement] two", a.0));

    // Its own posts are not new to the sender, and the supervisor's reads move only its own place.
    let room = op_room(&sup);
    let three = post(
        &sup,
        json!({ "kind": "message", "text": "rebase when free", "to": a.0, "room": room }),
    )
    .await;
    let read = tool(&fx.svc, &a, "room_read", json!({})).await.0;
    assert_eq!(
        read,
        format!("{three} supervisor → {}: [message] rebase when free", a.0)
    );
    assert_eq!(
        tool(&fx.svc, &a, "room_read", json!({})).await.0,
        "nothing new"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ask_holds_until_answered_and_returns_the_answer() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let held = ask_in_background(&fx.svc, &a, "supervisor", "can I change the schema?");
    let ask = wait_ask(&fx, &op_room(&sup)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!held.is_finished(), "the ask holds its caller");

    let read = tool(&fx.svc, &sup, "room_read", json!({})).await.0;
    assert_eq!(
        read,
        format!("{ask} {} → supervisor: [ask] can I change the schema?", a.0)
    );
    let answered = tool(
        &fx.svc,
        &sup,
        "room_answer",
        json!({ "ask": ask, "text": "yes, it is yours" }),
    )
    .await;
    assert_eq!(answered, ("answered".to_string(), false));
    let reply = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, ("yes, it is yours".to_string(), false));
    assert!(fx
        .store
        .post(ask)
        .await
        .unwrap()
        .unwrap()
        .answered_at
        .is_some());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !screen(&fx.sessions, &a).contains(NOTICE),
        "the held call had it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_answers_resolve_once() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let held = ask_in_background(&fx.svc, &a, "owner", "ship it?");
    let ask = wait_ask(&fx, &op_room(&sup)).await;

    let (owner, supervisor) = tokio::join!(
        fx.svc.room_answer(&MemberRef::Owner, ask, "owner says yes"),
        tool(
            &fx.svc,
            &sup,
            "room_answer",
            json!({ "ask": ask, "text": "supervisor says yes" })
        ),
    );
    let owner_won = owner.unwrap();
    let supervisor_won = supervisor == ("answered".to_string(), false);
    assert!(owner_won != supervisor_won, "{owner_won} {supervisor:?}");
    if owner_won {
        assert_eq!(supervisor, ("already answered".to_string(), false));
    }
    let winner = if owner_won {
        "owner says yes"
    } else {
        "supervisor says yes"
    };
    let reply = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, (winner.to_string(), false));

    // A late answer changes nothing.
    assert!(!fx
        .svc
        .room_answer(&MemberRef::Owner, ask, "again")
        .await
        .unwrap());
    let answers: Vec<Post> = fx
        .store
        .posts_after(&op_room(&sup), "nobody", 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|p| p.kind == PostKind::Answer)
        .collect();
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert_eq!(
        (answers[0].reply_to, answers[0].text.as_str()),
        (Some(ask), winner)
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!screen(&fx.sessions, &a).contains(NOTICE));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_supervisor_cannot_ask() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    direct(&fx, &sup).await;
    let reply = tool(
        &fx.svc,
        &sup,
        "room_ask",
        json!({ "to": "owner", "text": "may I?" }),
    )
    .await;
    assert_eq!(
        reply,
        ("the supervisor sends messages, not asks".to_string(), true)
    );
    assert!(asks(&fx, &op_room(&sup)).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ask_survives_a_daemon_restart_and_its_answer_arrives_as_a_notice() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let held = ask_in_background(&fx.svc, &a, "owner", "which port?");
    let ask = wait_ask(&fx, &op_room(&sup)).await;

    let (svc, sessions) = restart(&fx).await;
    held.abort();
    svc.recover().await.unwrap();
    wait_screen_in(&sessions, &session_name(&a), |s| s.contains("ready")).await;
    assert_eq!(
        fx.store.post(ask).await.unwrap().unwrap().answered_at,
        None,
        "still open"
    );

    let answered = tool(
        &svc,
        &sup,
        "room_answer",
        json!({ "ask": ask, "text": "port 7070" }),
    )
    .await;
    assert_eq!(answered, ("answered".to_string(), false));
    let line = format!("{NOTICE} supervisor answered your ask {ask}: port 7070");
    wait_screen_in(&sessions, &session_name(&a), |s| s.contains(&line)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pane = screen(&sessions, &a);
    assert_eq!(pane.matches(&line).count(), 1, "{pane}");
    sessions.shutdown_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_ask_times_out_to_no_answer_yet_and_stays_open() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let svc = with_hold(&fx, Duration::from_millis(300));
    let reply = tool(
        &svc,
        &a,
        "room_ask",
        json!({ "to": "owner", "text": "which port?" }),
    )
    .await;
    assert_eq!(
        reply,
        ("no answer yet; call room_read later".to_string(), false)
    );
    let ask = wait_ask(&fx, &op_room(&sup)).await;
    assert_eq!(fx.store.post(ask).await.unwrap().unwrap().answered_at, None);

    // With the call gone, the answer is typed as a notice instead.
    assert!(svc
        .room_answer(&MemberRef::Owner, ask, "port 7070")
        .await
        .unwrap());
    let line = format!("{NOTICE} owner answered your ask {ask}: port 7070");
    assert!(pane_contains(&fx, &a, &line).await);
}

#[test]
fn the_room_tools_go_to_orchestrators_and_supervisors() {
    let names = |kind| -> Vec<String> {
        mcp::tools(kind)
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .filter(|n| n.starts_with("room_"))
            .map(String::from)
            .collect()
    };
    assert_eq!(
        names(AgentKind::Orchestrator),
        ["room_post", "room_read", "room_ask", "room_answer"]
    );
    assert_eq!(
        names(AgentKind::Supervisor),
        ["room_post", "room_read", "room_answer"]
    );
    assert!(names(AgentKind::Solo).is_empty());
    assert!(names(AgentKind::Specialist).is_empty());
}
