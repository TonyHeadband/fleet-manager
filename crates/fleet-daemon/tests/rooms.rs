mod common;

use common::*;
use fleet_api::{
    AgentId, AgentKind, EventKind, MemberRef, Post, PostKind, PrState, Room, RoomKind, Status,
    TaskEvent, TaskEventKind, TaskPr, TaskState,
};
use fleet_daemon::agents::AgentService;
use fleet_daemon::rooms::posts::NO_ANSWER_YET;
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

    // At its input box, not mid-turn: only then is a notice typed.
    set_status(&fx, &a, Status::Review).await;
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
    set_status(&fx, &a, Status::Review).await;
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

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_while_the_asker_is_working_types_nothing() {
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
    assert_eq!(reply.0, NO_ANSWER_YET);
    let ask = wait_ask(&fx, &op_room(&sup)).await;
    assert_eq!(agent(&fx, &a).await.status, Status::Working);

    assert!(svc
        .room_answer(&MemberRef::Owner, ask, "port 7070")
        .await
        .unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !screen(&fx.sessions, &a).contains(NOTICE),
        "typed into a running turn"
    );
    // It is not lost: the answer waits in the room.
    let read = tool(&svc, &a, "room_read", json!({})).await.0;
    assert!(read.contains("[answer] port 7070"), "{read}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_supervisor_reaches_every_room_of_its_persona() {
    let fx = fixture().await;
    let me = conversation(&fx).await;
    let other = conversation(&fx).await;
    let a = direct(&fx, &other).await;
    let theirs = op_room(&other);
    let product = format!("product:{PRODUCT}");

    let said = posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "announcement", "text": "migration 017 is mine" }),
        )
        .await,
    );
    let read = tool(&fx.svc, &me, "room_read", json!({ "room": theirs })).await;
    assert_eq!(
        read,
        (
            format!("{said} {} → all: [announcement] migration 017 is mine", a.0),
            false
        )
    );
    assert_eq!(
        tool(&fx.svc, &me, "room_read", json!({ "room": theirs }))
            .await
            .0,
        "nothing new",
        "its reads are tracked once it is a member"
    );

    let hello = posted_id(
        &tool(
            &fx.svc,
            &me,
            "room_post",
            json!({ "kind": "announcement", "text": "hello product", "room": product }),
        )
        .await,
    );
    let read = tool(&fx.svc, &a, "room_read", json!({ "room": product }))
        .await
        .0;
    assert_eq!(
        read,
        format!("{hello} {} → all: [announcement] hello product", me.0)
    );
    assert_eq!(
        tool(&fx.svc, &me, "room_read", json!({ "room": product }))
            .await
            .0,
        "nothing new"
    );

    // Looking in neither keeps it a member past a sync nor holds the operation room open.
    rooms::sync(&fx.store, 20).await.unwrap();
    assert!(members(&fx, &product)
        .await
        .contains(&MemberRef::Agent(me.clone())));
    assert!(members(&fx, &theirs)
        .await
        .contains(&MemberRef::Agent(me.clone())));
    set_prs(&fx, &a, PrState::Merged).await;
    rooms::sync(&fx.store, 30).await.unwrap();
    assert_eq!(
        fx.store.room(&theirs).await.unwrap().unwrap().archived_at,
        Some(30)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rooms_of_another_persona_or_task_are_refused() {
    let fx = fixture().await;
    let me = conversation(&fx).await;
    let a = direct(&fx, &me).await;
    let foreign = rooms::operation_room(&fx.store, &AgentId("elsewhere".into()), "someone-else")
        .await
        .unwrap();
    let (text, failed) = tool(&fx.svc, &me, "room_read", json!({ "room": foreign })).await;
    assert!(
        failed && text.contains("not one of your persona's"),
        "{text}"
    );
    assert!(!members(&fx, &foreign)
        .await
        .contains(&MemberRef::Agent(me.clone())));

    // An orchestrator stays in its own rooms.
    let (text, failed) = tool(&fx.svc, &a, "room_read", json!({ "room": foreign })).await;
    assert!(failed && text.contains("not in room"), "{text}");
}

// Waking the recipient, and surfacing posts to the owner.

/// The notice a recipient of a `kind` post from the task `from` in `room` is typed.
async fn notice(fx: &Fx, kind: &str, from: &AgentId, room: &str) -> String {
    let t = fx.store.task(from).await.unwrap().unwrap();
    let name = t.title.filter(|t| !t.is_empty()).unwrap_or(t.name);
    format!("{NOTICE} new {kind} from {name} in {room}; call room_read.")
}

async fn events_of(fx: &Fx, conversation: &AgentId, kind: TaskEventKind) -> Vec<TaskEvent> {
    fx.store
        .task_events(conversation, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == kind)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_reaches_a_waiting_orchestrator_through_wait_for_fleet() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let waiting = {
        let (svc, a) = (fx.svc.clone(), a.clone());
        tokio::spawn(async move { tool(&svc, &a, "wait_for_fleet", json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!waiting.is_finished());

    let room = op_room(&sup);
    posted_id(
        &tool(
            &fx.svc,
            &sup,
            "room_post",
            json!({ "kind": "message", "text": "rebase when free", "to": a.0, "room": room }),
        )
        .await,
    );
    let (text, failed) = tokio::time::timeout(Duration::from_secs(10), waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(!failed, "{text}");
    assert_eq!(
        text,
        format!("new message from supervisor in {room}; call room_read.")
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !screen(&fx.sessions, &a).contains(NOTICE),
        "nothing typed into a running turn"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_a_working_orchestrator_waits_for_review() {
    let fx = fixture().await;
    rooms::start_delivery_watcher(fx.svc.clone());
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);
    let say = |text: &str| {
        let (svc, sup, room, a) = (fx.svc.clone(), sup.clone(), room.clone(), a.clone());
        let args = json!({ "kind": "message", "text": text, "to": a.0, "room": room });
        async move { posted_id(&tool(&svc, &sup, "room_post", args).await) }
    };
    // Working, not merely Queued: the notice waits because of the turn, not because the screen is starting.
    assert_eq!(agent(&fx, &a).await.status, Status::Working);
    say("rebase when free").await;
    say("and run the migrations").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !screen(&fx.sessions, &a).contains(NOTICE),
        "typed into a running turn"
    );

    set_status(&fx, &a, Status::Review).await;
    let line = format!("{NOTICE} new message from supervisor in {room}; call room_read.");
    assert!(pane_contains(&fx, &a, &line).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pane = screen(&fx.sessions, &a);
    assert_eq!(pane.matches(&line).count(), 1, "one notice: {pane}");
    assert_eq!(pane.matches(NOTICE).count(), 1, "one notice: {pane}");

    // Delivered: the next idle turn brings nothing again.
    set_status(&fx, &a, Status::Working).await;
    set_status(&fx, &a, Status::Review).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(screen(&fx.sessions, &a).matches(NOTICE).count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_an_idle_orchestrator_is_typed_at_once() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    set_status(&fx, &a, Status::Review).await;
    let room = op_room(&sup);
    posted_id(
        &tool(
            &fx.svc,
            &sup,
            "room_post",
            json!({ "kind": "message", "text": "rebase when free", "to": a.0, "room": room }),
        )
        .await,
    );
    let line = format!("{NOTICE} new message from supervisor in {room}; call room_read.");
    assert!(pane_contains(&fx, &a, &line).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_an_exited_supervisor_resumes_it() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let session = agent(&fx, &sup).await.session;
    wait_screen_in(&fx.sessions, &session, |s| s.contains("ready")).await;
    fx.svc.stop(&sup).await.unwrap();
    assert!(wait_until(|| async { !fx.sessions.alive(&session) }).await);

    posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "message", "text": "step 2 is done", "to": "supervisor" }),
        )
        .await,
    );
    let line = notice(&fx, "message", &a, &op_room(&sup)).await;
    let pane = wait_screen_in(&fx.sessions, &session, |s| s.contains(&line)).await;
    assert!(pane.contains("arg=--resume"), "{pane}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    // A message is pasted, then echoed as one entry.
    let pane = fx.sessions.screen_text(&session).unwrap();
    assert_eq!(
        pane.matches(&format!("entry<<{line}>>")).count(),
        1,
        "{pane}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_a_working_supervisor_waits_for_review() {
    let fx = fixture().await;
    rooms::start_delivery_watcher(fx.svc.clone());
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let session = agent(&fx, &sup).await.session;
    wait_screen_in(&fx.sessions, &session, |s| s.contains("ready")).await;
    reach(&fx, &sup, Status::Working).await;

    posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "message", "text": "step 2 is done", "to": "supervisor" }),
        )
        .await,
    );
    let line = notice(&fx, "message", &a, &op_room(&sup)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!fx.sessions.screen_text(&session).unwrap().contains(NOTICE));

    reach(&fx, &sup, Status::Review).await;
    wait_screen_in(&fx.sessions, &session, |s| s.contains(&line)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // A message is pasted, then echoed as one entry.
    let pane = fx.sessions.screen_text(&session).unwrap();
    assert_eq!(
        pane.matches(&format!("entry<<{line}>>")).count(),
        1,
        "{pane}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ask_to_the_supervisor_reaches_the_owner_as_a_chat_row() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    // Its first prompt's turn is over.
    reach(&fx, &sup, Status::Review).await;
    let held = ask_in_background(&fx.svc, &a, "supervisor", "can I change the schema?");
    wait_ask(&fx, &op_room(&sup)).await;

    let name = {
        let t = fx.store.task(&a).await.unwrap().unwrap();
        t.title.filter(|t| !t.is_empty()).unwrap_or(t.name)
    };
    assert!(
        wait_until(|| async {
            !events_of(&fx, &sup, TaskEventKind::RoomAsk)
                .await
                .is_empty()
        })
        .await
    );
    let rows = events_of(&fx, &sup, TaskEventKind::RoomAsk).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].task, a);
    assert_eq!(
        rows[0].text,
        format!("{name} in Untitled: can I change the schema?")
    );
    // The supervisor it went to hears of it too.
    let line = notice(&fx, "ask", &a, &op_room(&sup)).await;
    assert!(pane_contains(&fx, &sup, &line).await);
    held.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_the_owner_is_a_chat_row() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "message", "text": "PR is up", "to": "owner" }),
        )
        .await,
    );
    assert!(
        wait_until(|| async {
            !events_of(&fx, &sup, TaskEventKind::RoomMessage)
                .await
                .is_empty()
        })
        .await
    );
    let rows = events_of(&fx, &sup, TaskEventKind::RoomMessage).await;
    assert!(rows[0].text.ends_with(" in Untitled: PR is up"), "{rows:?}");
    // An announcement reaches nobody, the owner included.
    posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "announcement", "text": "fyi" }),
        )
        .await,
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fx.store.task_events(&sup, 0).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_ask_is_reposted_once_after_thirty_minutes() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let svc = with_hold(&fx, Duration::from_millis(200));
    let reply = tool(
        &svc,
        &a,
        "room_ask",
        json!({ "to": "owner", "text": "which port?" }),
    )
    .await;
    assert_eq!(reply.0, NO_ANSWER_YET);
    // An ask already answered is never reposted.
    let answered = tool(
        &svc,
        &a,
        "room_ask",
        json!({ "to": "owner", "text": "which db?" }),
    )
    .await;
    assert_eq!(answered.0, NO_ANSWER_YET);
    let room = op_room(&sup);
    let second = asks(&fx, &room).await[1].id;
    assert!(fx
        .svc
        .room_answer(&MemberRef::Owner, second, "sqlite")
        .await
        .unwrap());

    let reaper =
        fleet_daemon::supervisor::start_idle_reaper(fx.svc.clone(), Duration::from_millis(50));
    fx.clock.advance(29 * 60);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(events_of(&fx, &sup, TaskEventKind::RoomMessage)
        .await
        .is_empty());

    fx.clock.advance(60);
    assert!(
        wait_until(|| async {
            !events_of(&fx, &sup, TaskEventKind::RoomMessage)
                .await
                .is_empty()
        })
        .await
    );
    fx.clock.advance(3600);
    tokio::time::sleep(Duration::from_millis(300)).await;
    reaper.abort();
    let rows = events_of(&fx, &sup, TaskEventKind::RoomMessage).await;
    assert_eq!(rows.len(), 1, "once: {rows:?}");
    assert!(rows[0].text.contains("which port?"), "{rows:?}");
    assert_eq!(rows[0].task, a);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_given_while_the_asker_works_is_typed_once_after_review() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let svc = with_hold(&fx, Duration::from_millis(300));
    rooms::start_delivery_watcher(svc.clone());
    let reply = tool(
        &svc,
        &a,
        "room_ask",
        json!({ "to": "owner", "text": "which port?" }),
    )
    .await;
    assert_eq!(reply.0, NO_ANSWER_YET);
    let ask = wait_ask(&fx, &op_room(&sup)).await;
    assert!(svc
        .room_answer(&MemberRef::Owner, ask, "port 7070")
        .await
        .unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!screen(&fx.sessions, &a).contains(NOTICE));

    set_status(&fx, &a, Status::Review).await;
    let line = format!("{NOTICE} owner answered your ask {ask}: port 7070");
    assert!(pane_contains(&fx, &a, &line).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pane = screen(&fx.sessions, &a);
    assert_eq!(pane.matches(&line).count(), 1, "{pane}");
    // `wait_for_fleet` has it too.
    let (text, _) = tool(&svc, &a, "wait_for_fleet", json!({})).await;
    assert_eq!(text, format!("owner answered your ask {ask}: port 7070"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_notice_waits_for_the_turn_the_first_one_started() {
    let fx = fixture().await;
    rooms::start_delivery_watcher(fx.svc.clone());
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    set_status(&fx, &a, Status::Review).await;
    let op = op_room(&sup);
    let product = format!("product:{PRODUCT}");
    let say = |room: &str| {
        let (svc, sup, a) = (fx.svc.clone(), sup.clone(), a.clone());
        let args = json!({ "kind": "message", "text": "look", "to": a.0, "room": room });
        async move { posted_id(&tool(&svc, &sup, "room_post", args).await) }
    };
    let first = format!("{NOTICE} new message from supervisor in {op}; call room_read.");
    // Outside its own operation room the supervisor goes by its id.
    let second = format!(
        "{NOTICE} new message from {} in {product}; call room_read.",
        sup.0
    );

    say(&op).await;
    assert!(pane_contains(&fx, &a, &first).await);
    // Still Review: the hook has not reported the turn the first line started.
    say(&product).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !screen(&fx.sessions, &a).contains(&second),
        "typed into the turn the first notice started"
    );

    set_status(&fx, &a, Status::Working).await;
    set_status(&fx, &a, Status::Review).await;
    assert!(pane_contains(&fx, &a, &second).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pane = screen(&fx.sessions, &a);
    assert_eq!(pane.matches(&first).count(), 1, "{pane}");
    assert_eq!(pane.matches(&second).count(), 1, "{pane}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_supervisor_notice_never_lands_in_a_running_turn() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let session = agent(&fx, &sup).await.session;
    wait_screen_in(&fx.sessions, &session, |s| s.contains("ready")).await;
    set_status(&fx, &sup, Status::Working).await;
    let sent = Supervisor::new(fx.svc.clone())
        .message_if_idle(&sup, "Fleet Manager: new message")
        .await
        .unwrap();
    assert!(!sent, "busy");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!fx.sessions.screen_text(&session).unwrap().contains(NOTICE));

    reach(&fx, &sup, Status::Review).await;
    assert!(Supervisor::new(fx.svc.clone())
        .message_if_idle(&sup, "Fleet Manager: new message")
        .await
        .unwrap());
    wait_screen_in(&fx.sessions, &session, |s| {
        s.contains("entry<<Fleet Manager: new message>>")
    })
    .await;
}

/// Moves the agent to `status` a second later, as Claude's hook does: `status_since` is the new now.
async fn reach(fx: &Fx, id: &AgentId, status: Status) {
    fx.clock.advance(1);
    let now = fx.clock.now_for_test();
    fx.store
        .update_agent(id, |a| {
            a.status = status;
            a.status_since = now;
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_supervisor_notice_behind_the_owners_message_waits_for_the_next_review() {
    let fx = fixture().await;
    rooms::start_delivery_watcher(fx.svc.clone());
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let session = agent(&fx, &sup).await.session;
    wait_screen_in(&fx.sessions, &session, |s| s.contains("ready")).await;
    reach(&fx, &sup, Status::Review).await;
    // The owner's message is typed; Claude's hook has not reported its turn, so it still reads Review.
    Supervisor::new(fx.svc.clone())
        .message(&sup, "and the docs?")
        .await
        .unwrap();

    posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "message", "text": "step 2 is done", "to": "supervisor" }),
        )
        .await,
    );
    let line = notice(&fx, "message", &a, &op_room(&sup)).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        !fx.sessions.screen_text(&session).unwrap().contains(&line),
        "typed into the owner's turn"
    );

    reach(&fx, &sup, Status::Working).await;
    reach(&fx, &sup, Status::Review).await;
    wait_screen_in(&fx.sessions, &session, |s| {
        s.contains(&format!("entry<<{line}>>"))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pane = fx.sessions.screen_text(&session).unwrap();
    assert_eq!(
        pane.matches(&format!("entry<<{line}>>")).count(),
        1,
        "{pane}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mark_left_by_missed_events_does_not_hold_notices_back() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    reach(&fx, &a, Status::Review).await;
    let op = op_room(&sup);
    let product = format!("product:{PRODUCT}");
    let say = |room: &str| {
        let (svc, sup, a) = (fx.svc.clone(), sup.clone(), a.clone());
        let args = json!({ "kind": "message", "text": "look", "to": a.0, "room": room });
        async move { posted_id(&tool(&svc, &sup, "room_post", args).await) }
    };
    say(&op).await;
    let first = format!("{NOTICE} new message from supervisor in {op}; call room_read.");
    assert!(pane_contains(&fx, &a, &first).await);
    // No watcher saw the turn: it went Working and back to Review unobserved.
    reach(&fx, &a, Status::Working).await;
    reach(&fx, &a, Status::Review).await;
    say(&product).await;
    let second = format!(
        "{NOTICE} new message from {} in {product}; call room_read.",
        sup.0
    );
    assert!(pane_contains(&fx, &a, &second).await);
}
