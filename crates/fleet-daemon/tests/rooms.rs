mod common;

use common::net::*;
use common::*;
use fleet_api::{
    AgentId, AgentKind, Approval, DecideBreakdown, EventKind, MemberRef, Post, PostKind, PrState,
    Room, RoomKind, Status, StepDraft, TaskEvent, TaskEventKind, TaskPr, TaskState,
};
use fleet_daemon::agents::AgentService;
use fleet_daemon::agents::ServiceError;
use fleet_daemon::fleet::breakdown::BreakdownAnswer;
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::rooms::posts::NO_ANSWER_YET;
use fleet_daemon::session::SessionHost;
use fleet_daemon::supervisor::Supervisor;
use fleet_daemon::{mcp, rooms};
use serde_json::{json, Value};
use std::net::SocketAddr;
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

/// A repo the task's branch changed, as the merge watcher marks it.
fn changed(repo: &str) -> fleet_api::Workspace {
    fleet_api::Workspace {
        repo: repo.into(),
        branch: "fleet/x".into(),
        worktree: String::new(),
        base: String::new(),
        from_branch: None,
        has_changes: true,
        owned: false,
        changed: true,
    }
}

/// The task changed `core` and has one PR there in `state`.
async fn set_prs(fx: &Fx, id: &AgentId, state: PrState) {
    fx.store
        .update_task(id, |t| {
            t.repos = vec![changed("core")];
            t.prs = pr(state);
        })
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
async fn a_pr_closed_unmerged_does_not_end_the_task() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let task = orchestrator(&fx, Some(&sup)).await;
    set_prs(&fx, &task, PrState::Closed).await;
    rooms::sync(&fx.store, 20).await.unwrap();
    let op = &room_of(&fx, RoomKind::Operation).await[0];
    assert_eq!(op.archived_at, None);
    assert!(members(&fx, &op.id).await.contains(&MemberRef::Agent(task)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_with_one_repo_merged_and_another_without_a_pr_stays_in_its_rooms() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let task = orchestrator(&fx, Some(&sup)).await;
    fx.store
        .update_task(&task, |t| {
            t.repos = vec![changed("core"), changed("web")];
            t.prs = pr(PrState::Merged);
        })
        .await
        .unwrap();
    rooms::sync(&fx.store, 20).await.unwrap();
    let op = &room_of(&fx, RoomKind::Operation).await[0];
    assert_eq!(op.archived_at, None);
    assert!(members(&fx, &op.id)
        .await
        .contains(&MemberRef::Agent(task.clone())));

    // Done by hand ends it.
    fx.store
        .update_task(&task, |t| t.done_by_hand = true)
        .await
        .unwrap();
    rooms::sync(&fx.store, 30).await.unwrap();
    assert_eq!(
        room_of(&fx, RoomKind::Operation).await[0].archived_at,
        Some(30)
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
    direct_from(fx, Some(origin)).await
}

/// `direct`, or with no origin a hand-made task, in its product's room only.
async fn direct_from(fx: &Fx, origin: Option<&AgentId>) -> AgentId {
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
    let origin = origin.cloned();
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

    let (read, failed) = tool(&fx.svc, &sup, "room_read", json!({ "room": op_room(&sup) })).await;
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
    let read = tool(&fx.svc, &sup, "room_read", json!({ "room": op_room(&sup) }))
        .await
        .0;
    assert!(read.starts_with(&format!("{one} ")), "{read}");
    assert_eq!(
        tool(&fx.svc, &sup, "room_read", json!({ "room": op_room(&sup) }))
            .await
            .0,
        "nothing new"
    );

    let two = post(&a, json!({ "kind": "announcement", "text": "two" })).await;
    let read = tool(&fx.svc, &sup, "room_read", json!({ "room": op_room(&sup) }))
        .await
        .0;
    assert_eq!(read, format!("{two} {} → all: [announcement] two", a.0));

    // Its own posts are not new to the sender, and the supervisor's reads move only its own place.
    let room = op_room(&sup);
    let three = post(
        &sup,
        json!({ "kind": "message", "text": "rebase when free", "to": a.0, "room": room }),
    )
    .await;
    let read = tool(&fx.svc, &a, "room_read", json!({ "room": op_room(&sup) }))
        .await
        .0;
    assert_eq!(
        read,
        format!("{three} supervisor → {}: [message] rebase when free", a.0)
    );
    assert_eq!(
        tool(&fx.svc, &a, "room_read", json!({ "room": op_room(&sup) }))
            .await
            .0,
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

    let read = tool(&fx.svc, &sup, "room_read", json!({ "room": op_room(&sup) }))
        .await
        .0;
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
async fn an_answer_to_an_ended_task_tells_nobody() {
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
    fx.store
        .update_task(&a, |t| t.done_by_hand = true)
        .await
        .unwrap();
    set_status(&fx, &a, Status::Review).await;

    assert!(svc
        .room_answer(&MemberRef::Owner, ask, "port 7070")
        .await
        .unwrap());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!screen(&fx.sessions, &a).contains(NOTICE));
    assert!(fx.store.task(&a).await.unwrap().unwrap().inbox.is_empty());
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

fn product_room() -> String {
    format!("product:{PRODUCT}")
}

/// The posts in `room`, oldest first.
async fn all_posts(fx: &Fx, room: &str) -> Vec<Post> {
    fx.store.posts_after(room, "nobody", 0).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_operation_tasks_orchestrator_reads_the_daemons_migration_announcement() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    rooms::announce::migrations_taken(&fx.store, PRODUCT, "Other task", &["017_x.sql".into()])
        .await
        .unwrap();
    let taken = all_posts(&fx, &product_room()).await[0].id;
    let hello = posted_id(
        &tool(
            &fx.svc,
            &sup,
            "room_post",
            json!({ "kind": "announcement", "text": "hello", "room": op_room(&sup) }),
        )
        .await,
    );

    // With no room named, every room it is in, each headed with its id and kind.
    let read = tool(&fx.svc, &a, "room_read", json!({})).await;
    assert_eq!(
        read,
        (
            format!(
                "# {} (operation)\n{hello} supervisor → all: [announcement] hello\n\
                 # {} (product)\n{taken} daemon → all: [announcement] migration 017_x.sql taken by Other task",
                op_room(&sup),
                product_room()
            ),
            false
        )
    );
    assert_eq!(
        tool(&fx.svc, &a, "room_read", json!({})).await.0,
        format!(
            "# {} (operation)\nnothing new\n# {} (product)\nnothing new",
            op_room(&sup),
            product_room()
        ),
        "each room's reads are tracked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn posting_with_no_room_uses_the_operation_room_else_the_product_room() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let hand_made = direct_from(&fx, None).await;
    let one = posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "announcement", "text": "one" }),
        )
        .await,
    );
    let two = posted_id(
        &tool(
            &fx.svc,
            &hand_made,
            "room_post",
            json!({ "kind": "announcement", "text": "two" }),
        )
        .await,
    );
    assert_eq!(
        fx.store.post(one).await.unwrap().unwrap().room,
        op_room(&sup)
    );
    assert_eq!(
        fx.store.post(two).await.unwrap().unwrap().room,
        product_room()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_supervisor_in_no_room_is_told_so() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let (text, failed) = tool(&fx.svc, &sup, "room_read", json!({})).await;
    assert!(failed && text.contains("you are in no room"), "{text}");
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
    // A supervisor goes by `supervisor` in every room.
    let second = format!("{NOTICE} new message from supervisor in {product}; call room_read.");

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
    let second = format!("{NOTICE} new message from supervisor in {product}; call room_read.");
    assert!(pane_contains(&fx, &a, &second).await);
}

/// The conversation's session, once its TUI is up.
async fn ready_conversation(fx: &Fx, sup: &AgentId) -> String {
    let session = agent(fx, sup).await.session;
    wait_screen_in(&fx.sessions, &session, |s| s.contains("ready")).await;
    session
}

/// The conversation's session, at its input box after its first turn.
async fn idle_conversation(fx: &Fx, sup: &AgentId) -> String {
    let session = ready_conversation(fx, sup).await;
    reach(fx, sup, Status::Review).await;
    session
}

/// `sup` reads `room`, which makes it a reader there.
async fn look_in(fx: &Fx, sup: &AgentId, room: &str) {
    let (text, failed) = tool(&fx.svc, sup, "room_read", json!({ "room": room })).await;
    assert!(!failed, "{text}");
}

/// Two conversations of the persona that both looked in the product room; the second is the most recently used.
async fn two_readers(fx: &Fx) -> ((AgentId, String), (AgentId, String)) {
    rooms::product_room(&fx.store, PRODUCT).await.unwrap();
    let old = conversation(fx).await;
    let recent = conversation(fx).await;
    let sessions = (
        ready_conversation(fx, &old).await,
        ready_conversation(fx, &recent).await,
    );
    look_in(fx, &old, &product_room()).await;
    look_in(fx, &recent, &product_room()).await;
    fx.clock.advance(1);
    fx.store
        .touch_conversation(&recent, fx.clock.now_for_test())
        .await
        .unwrap();
    // After the touch: a conversation used since its last Review has a turn pending.
    reach(fx, &old, Status::Review).await;
    reach(fx, &recent, Status::Review).await;
    ((old, sessions.0), (recent, sessions.1))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broadcast_in_a_product_room_wakes_only_the_most_recent_conversation() {
    let fx = fixture().await;
    let ((_, old), (_, recent)) = two_readers(&fx).await;
    let a = direct_from(&fx, None).await;
    posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "message", "text": "rebase on main" }),
        )
        .await,
    );
    let line = notice(&fx, "message", &a, &product_room()).await;
    wait_screen_in(&fx.sessions, &recent, |s| s.contains(&line)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!fx.sessions.screen_text(&old).unwrap().contains(NOTICE));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_operation_room_broadcast_wakes_only_its_own_conversation() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let other = conversation(&fx).await;
    let own = idle_conversation(&fx, &sup).await;
    let theirs = idle_conversation(&fx, &other).await;
    let a = direct(&fx, &sup).await;
    look_in(&fx, &other, &op_room(&sup)).await;
    posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "message", "text": "step 2 is done" }),
        )
        .await,
    );
    let line = notice(&fx, "message", &a, &op_room(&sup)).await;
    wait_screen_in(&fx.sessions, &own, |s| s.contains(&line)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!fx.sessions.screen_text(&theirs).unwrap().contains(NOTICE));
}

#[tokio::test(flavor = "multi_thread")]
async fn to_supervisor_in_a_product_room_reaches_the_most_recent_conversation() {
    let fx = fixture().await;
    let ((_, old), (recent, session)) = two_readers(&fx).await;
    let a = direct_from(&fx, None).await;
    let id = posted_id(
        &tool(
            &fx.svc,
            &a,
            "room_post",
            json!({ "kind": "message", "text": "who owns 017?", "to": "supervisor" }),
        )
        .await,
    );
    assert_eq!(
        fx.store.post(id).await.unwrap().unwrap().to,
        Some(MemberRef::Agent(recent))
    );
    let line = notice(&fx, "message", &a, &product_room()).await;
    wait_screen_in(&fx.sessions, &session, |s| s.contains(&line)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!fx.sessions.screen_text(&old).unwrap().contains(NOTICE));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rooms_supervisor_is_listed_once_and_readers_never() {
    let fx = fixture().await;
    let ((old, _), (recent, _)) = two_readers(&fx).await;
    let a = direct_from(&fx, None).await;
    let rooms = rooms::list(&fx.store).await.unwrap();
    let product = rooms.iter().find(|r| r.id == product_room()).unwrap();
    let got: Vec<(String, String)> = product
        .members
        .iter()
        .map(|m| (m.id.0.clone(), m.name.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            (recent.0.clone(), "supervisor".to_string()),
            (a.0.clone(), task_name(&fx, &a).await)
        ]
    );
    // A reader is not someone to address.
    let refused = fx
        .svc
        .owner_post(
            &product_room(),
            PostKind::Message,
            Some(&MemberRef::Agent(old.clone())),
            "hello",
        )
        .await;
    assert!(
        matches!(&refused, Err(ServiceError::Refused(t)) if t.contains("is not in this room")),
        "{refused:?}"
    );
}

// The breakdown as an ask.

fn draft(title: &str, depends_on: &[u32]) -> StepDraft {
    StepDraft {
        n: None,
        title: title.into(),
        specialist: "rust".into(),
        brief: format!("build {title}"),
        depends_on: depends_on.to_vec(),
        release_note: false,
        mode: None,
        effort: None,
    }
}

fn two_drafts() -> Vec<StepDraft> {
    // Independent, so a later version can restate them as new steps.
    vec![draft("api", &[]), draft("ui", &[])]
}

/// Starts a held `propose_breakdown` and waits until its ask is in `room`; returns the call and the ask.
async fn propose(
    fx: &Fx,
    a: &AgentId,
    room: &str,
) -> (
    tokio::task::JoinHandle<Result<BreakdownAnswer, ServiceError>>,
    Post,
) {
    let before = asks(fx, room).await.len();
    let (svc, held) = (fx.svc.clone(), a.clone());
    let handle = tokio::spawn(async move { svc.propose_breakdown(&held, two_drafts()).await });
    if !wait_until(|| async { asks(fx, room).await.len() > before }).await {
        let refused = tokio::time::timeout(Duration::from_secs(1), handle).await;
        panic!("no ask for the proposal: {refused:?}");
    }
    (handle, asks(fx, room).await.pop().unwrap())
}

async fn answer_to(fx: &Fx, ask: i64, room: &str) -> Post {
    fx.store
        .posts_after(room, "nobody", 0)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.reply_to == Some(ask))
        .unwrap_or_else(|| panic!("ask {ask} has no answer"))
}

fn decision(approve: bool, comment: Option<&str>) -> DecideBreakdown {
    DecideBreakdown {
        approve,
        comment: comment.map(String::from),
        version: None,
    }
}

async fn held_answer(
    handle: tokio::task::JoinHandle<Result<BreakdownAnswer, ServiceError>>,
) -> BreakdownAnswer {
    tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn proposing_a_breakdown_posts_an_ask_to_the_supervisor() {
    let fx = fixture().await;
    rooms::start_delivery_watcher(fx.svc.clone());
    let sup = conversation(&fx).await;
    reach(&fx, &sup, Status::Review).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);
    let (_held, ask) = propose(&fx, &a, &room).await;
    assert_eq!(ask.sender, MemberRef::Agent(a.clone()));
    assert_eq!(ask.to, Some(MemberRef::Agent(sup.clone())));
    assert_eq!(ask.breakdown_version, Some(1));
    assert_eq!(ask.text, "breakdown v1: 2 steps — api, …");
    assert_eq!(ask.answered_at, None);
    // The owner hears of it from the card's Needs you; nothing wakes or types into the supervisor.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !screen(&fx.sessions, &sup).contains(NOTICE),
        "{}",
        screen(&fx.sessions, &sup)
    );
    assert!(events_of(&fx, &sup, TaskEventKind::RoomAsk)
        .await
        .is_empty());
    assert_eq!(
        fx.store
            .task(&a)
            .await
            .unwrap()
            .unwrap()
            .inbox
            .iter()
            .filter(|e| e.kind == fleet_api::FleetEventKind::RoomPost)
            .count(),
        0
    );

    // A hand-made task has no operation room: its ask goes to the owner in the product's room.
    let b = direct_from(&fx, None).await;
    let product = format!("product:{PRODUCT}");
    let (_held, ask) = propose(&fx, &b, &product).await;
    assert_eq!(ask.sender, MemberRef::Agent(b));
    assert_eq!(ask.to, Some(MemberRef::Owner));
    assert_eq!(ask.breakdown_version, Some(1));
}

/// The supervisor's message to `a` in its operation room; returns the notice `a` is typed for it.
async fn message_to(fx: &Fx, sup: &AgentId, a: &AgentId) -> String {
    let room = op_room(sup);
    posted_id(
        &tool(
            &fx.svc,
            sup,
            "room_post",
            json!({ "kind": "message", "text": "rebase when free", "to": a.0, "room": room }),
        )
        .await,
    );
    format!("{NOTICE} new message from supervisor in {room}; call room_read.")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_an_orchestrator_held_in_room_ask_waits_for_review() {
    let fx = fixture().await;
    rooms::start_delivery_watcher(fx.svc.clone());
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    // Whatever its status says, a held call is a running turn.
    set_status(&fx, &a, Status::Review).await;
    let held = ask_in_background(&fx.svc, &a, "owner", "which port?");
    let ask = wait_ask(&fx, &op_room(&sup)).await;

    let line = message_to(&fx, &sup, &a).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !screen(&fx.sessions, &a).contains(NOTICE),
        "typed into the held turn"
    );

    assert!(fx
        .svc
        .room_answer(&MemberRef::Owner, ask, "port 7070")
        .await
        .unwrap());
    let reply = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, ("port 7070".to_string(), false));
    set_status(&fx, &a, Status::Working).await;
    set_status(&fx, &a, Status::Review).await;
    assert!(pane_contains(&fx, &a, &line).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_an_orchestrator_held_in_propose_breakdown_waits_for_review() {
    let fx = fixture().await;
    rooms::start_delivery_watcher(fx.svc.clone());
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let (held, _) = propose(&fx, &a, &op_room(&sup)).await;
    let card = agent(&fx, &a).await;
    assert_eq!(
        (card.status, card.blocked_reason),
        (Status::NeedsYou, Some(fleet_api::BlockedReason::Breakdown))
    );

    let line = message_to(&fx, &sup, &a).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !screen(&fx.sessions, &a).contains(NOTICE),
        "typed into the held turn"
    );

    fx.svc
        .decide_breakdown(&a, decision(true, None))
        .await
        .unwrap();
    assert_eq!(held_answer(held).await, BreakdownAnswer::Approved);
    set_status(&fx, &a, Status::Working).await;
    set_status(&fx, &a, Status::Review).await;
    assert!(pane_contains(&fx, &a, &line).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_in_the_panel_answers_the_ask() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);
    let (held, ask) = propose(&fx, &a, &room).await;

    fx.svc
        .decide_breakdown(&a, decision(true, None))
        .await
        .unwrap();
    assert_eq!(held_answer(held).await, BreakdownAnswer::Approved);
    let q = fx.store.post(ask.id).await.unwrap().unwrap();
    assert!(q.answered_at.is_some());
    let answer = answer_to(&fx, ask.id, &room).await;
    assert_eq!(answer.sender, MemberRef::Owner);
    assert_eq!(answer.text, "approved in the panel");

    // Decided once: the room and the panel both find it decided.
    assert!(!fx.svc.room_approve(ask.id).await.unwrap());
    let err = fx
        .svc
        .decide_breakdown(&a, decision(true, None))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NoPendingBreakdown), "{err:?}");
    let answers = fx
        .store
        .posts_after(&room, "nobody", 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|p| p.reply_to == Some(ask.id))
        .count();
    assert_eq!(answers, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_the_ask_approves_the_breakdown() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);

    // Any other text asks for changes, with the text as the comment.
    let (held, ask) = propose(&fx, &a, &room).await;
    assert!(fx
        .svc
        .room_answer(&MemberRef::Owner, ask.id, "split the api step")
        .await
        .unwrap());
    assert_eq!(
        held_answer(held).await,
        BreakdownAnswer::ChangesRequested("split the api step".into())
    );
    let p = fx.store.task(&a).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].approval, Approval::ChangesRequested);
    assert_eq!(
        p.breakdowns[0].comment.as_deref(),
        Some("split the api step")
    );

    // Text never approves, not even `approve`.
    let (held, ask) = propose(&fx, &a, &room).await;
    assert!(fx
        .svc
        .room_answer(&MemberRef::Owner, ask.id, "approve")
        .await
        .unwrap());
    assert_eq!(
        held_answer(held).await,
        BreakdownAnswer::ChangesRequested("approve".into())
    );

    let (held, ask) = propose(&fx, &a, &room).await;
    assert_eq!(ask.breakdown_version, Some(3));
    assert!(fx.svc.room_approve(ask.id).await.unwrap());
    assert_eq!(held_answer(held).await, BreakdownAnswer::Approved);
    let p = fx.store.task(&a).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[2].approval, Approval::Approved);
    assert!(fx
        .store
        .post(ask.id)
        .await
        .unwrap()
        .unwrap()
        .answered_at
        .is_some());
    assert_eq!(
        answer_to(&fx, ask.id, &room).await.text,
        "approved in the room"
    );
    // Neither path leaves the other pending.
    let err = fx
        .svc
        .decide_breakdown(&a, decision(true, None))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NoPendingBreakdown), "{err:?}");
    assert!(!fx.svc.room_approve(ask.id).await.unwrap());
    assert!(!fx
        .svc
        .room_answer(&MemberRef::Owner, ask.id, "more")
        .await
        .unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_supervisor_cannot_approve_a_breakdown() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let other = direct(&fx, &sup).await;
    let room = op_room(&sup);
    let (held, ask) = propose(&fx, &a, &room).await;

    let reply = tool(
        &fx.svc,
        &sup,
        "room_answer",
        json!({ "ask": ask.id, "text": "approve" }),
    )
    .await;
    assert_eq!(
        reply,
        (
            "breakdown approval waits for the owner until autopilot".into(),
            true
        )
    );
    // Nor may another orchestrator in the room.
    let reply = tool(
        &fx.svc,
        &other,
        "room_answer",
        json!({ "ask": ask.id, "text": "approve" }),
    )
    .await;
    assert_eq!(
        reply,
        ("breakdown approval waits for the owner".into(), true)
    );
    let p = fx.store.task(&a).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[0].approval, Approval::Proposed);
    assert_eq!(
        fx.store.post(ask.id).await.unwrap().unwrap().answered_at,
        None
    );
    assert!(!held.is_finished());
    held.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_breakdown_version_ask_is_refused() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);
    let (held, v1) = propose(&fx, &a, &room).await;
    fx.svc
        .decide_breakdown(&a, decision(false, Some("fewer steps")))
        .await
        .unwrap();
    held_answer(held).await;
    assert_eq!(answer_to(&fx, v1.id, &room).await.text, "fewer steps");

    let (held, v2) = propose(&fx, &a, &room).await;
    let err = fx.svc.room_approve(v1.id).await.unwrap_err();
    assert_eq!(err.to_string(), "breakdown v1 is stale: v2 is the latest");
    let p = fx.store.task(&a).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[1].approval, Approval::Proposed);
    assert_eq!(
        fx.store.post(v2.id).await.unwrap().unwrap().answered_at,
        None
    );
    held.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_version_supersedes_an_ask_left_open() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);
    let (held, v1) = propose(&fx, &a, &room).await;
    held.abort();
    // A decision that did not reach the room, as one made before rooms existed.
    fx.store
        .update_task(&a, |p| {
            p.breakdowns[0].approval = Approval::ChangesRequested;
        })
        .await
        .unwrap();

    let (held, _v2) = propose(&fx, &a, &room).await;
    let answer = answer_to(&fx, v1.id, &room).await;
    assert_eq!(answer.text, "superseded by v2");
    assert_eq!(answer.sender, MemberRef::Daemon);
    let err = fx.svc.room_approve(v1.id).await.unwrap_err();
    assert_eq!(err.to_string(), "breakdown v1 is stale: v2 is the latest");
    held.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_breakdown_ask_is_not_reposted() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);
    let (held, ask) = propose(&fx, &a, &room).await;

    fx.clock.advance(31 * 60);
    rooms::repost_stale_asks(&fx.svc).await.unwrap();
    assert!(events_of(&fx, &sup, TaskEventKind::RoomMessage)
        .await
        .is_empty());
    assert_eq!(
        fx.store.post(ask.id).await.unwrap().unwrap().answered_at,
        None
    );
    held.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn asks_of_ended_tasks_or_archived_rooms_are_not_reposted() {
    let fx = fixture().await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let svc = with_hold(&fx, Duration::from_millis(200));
    // An ask in the product room from a task that then ends.
    let product = format!("product:{PRODUCT}");
    let reply = tool(
        &svc,
        &a,
        "room_ask",
        json!({ "to": "owner", "text": "which port?", "room": product }),
    )
    .await;
    assert_eq!(reply.0, NO_ANSWER_YET);
    // An ask in a room that is then archived, from a task still running.
    let b = direct(&fx, &sup).await;
    let reply = tool(
        &svc,
        &b,
        "room_ask",
        json!({ "to": "owner", "text": "which db?" }),
    )
    .await;
    assert_eq!(reply.0, NO_ANSWER_YET);
    set_prs(&fx, &a, PrState::Merged).await;
    rooms::sync(&fx.store, 20).await.unwrap();
    fx.store
        .set_room_archived(&op_room(&sup), Some(20))
        .await
        .unwrap();

    fx.clock.advance(31 * 60);
    rooms::repost_stale_asks(&fx.svc).await.unwrap();
    assert!(events_of(&fx, &sup, TaskEventKind::RoomMessage)
        .await
        .is_empty());
}

// The owner's rooms API.

struct Server {
    addr: SocketAddr,
}

async fn serve(fx: &Fx) -> Server {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    Server { addr }
}

async fn listed(srv: &Server, persona: &str) -> Vec<Room> {
    let (status, v) = json_of(
        srv.addr,
        "GET",
        &format!("/api/rooms?persona={persona}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

async fn read_posts(srv: &Server, room: &str, query: &str) -> Vec<Post> {
    let (status, v) = json_of(
        srv.addr,
        "GET",
        &format!("/api/rooms/{room}/posts{query}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

async fn post_as_owner(srv: &Server, room: &str, body: Value) -> (u16, Value) {
    json_of(
        srv.addr,
        "POST",
        &format!("/api/rooms/{room}/posts"),
        Some(&body),
    )
    .await
}

async fn answer_as_owner(srv: &Server, ask: i64, body: Value) -> (u16, Value) {
    json_of(
        srv.addr,
        "POST",
        &format!("/api/rooms/asks/{ask}/answer"),
        Some(&body),
    )
    .await
}

async fn post_as(fx: &Fx, a: &AgentId, text: &str, room: Option<&str>) -> i64 {
    let mut args = json!({ "kind": "message", "text": text });
    if let Some(r) = room {
        args["room"] = json!(r);
    }
    posted_id(&tool(&fx.svc, a, "room_post", args).await)
}

/// What the rooms list calls a task: its title, else its name.
async fn task_name(fx: &Fx, a: &AgentId) -> String {
    let t = fx.store.task(a).await.unwrap().unwrap();
    t.title.filter(|t| !t.is_empty()).unwrap_or(t.name)
}

#[tokio::test(flavor = "multi_thread")]
async fn rooms_list_counts_the_owners_unread() {
    let fx = fixture().await;
    let srv = serve(&fx).await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    // An archived operation room of this persona, and a live one of another persona.
    let old = rooms::operation_room(&fx.store, &AgentId("old".into()), PERSONA)
        .await
        .unwrap();
    fx.store.set_room_archived(&old, Some(5)).await.unwrap();
    rooms::operation_room(&fx.store, &AgentId("theirs".into()), "someone-else")
        .await
        .unwrap();

    let product = format!("product:{PRODUCT}");
    // Before any post the product room is not listed.
    let ids: Vec<String> = listed(&srv, PERSONA)
        .await
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![op_room(&sup), old.clone()]);

    post_as(&fx, &a, "one", None).await;
    post_as(&fx, &a, "two", None).await;
    post_as(&fx, &a, "in the product room", Some(&product)).await;
    // The owner's own post is not unread to them.
    let (status, v) = post_as_owner(
        &srv,
        &op_room(&sup),
        json!({ "kind": "announcement", "text": "mine" }),
    )
    .await;
    assert_eq!(status, 200, "{v}");

    let rooms = listed(&srv, PERSONA).await;
    let got: Vec<(String, u32)> = rooms.iter().map(|r| (r.id.clone(), r.unread)).collect();
    // Live operation rooms, then archived ones, then product rooms with posts. The owner has no member row in the
    // product room and every post there counts.
    assert_eq!(
        got,
        vec![(op_room(&sup), 2), (old.clone(), 0), (product.clone(), 1)]
    );
    assert!(fx
        .store
        .room_members(&product)
        .await
        .unwrap()
        .iter()
        .all(|m| m.member != "owner"));
    let theirs: Vec<String> = listed(&srv, "someone-else")
        .await
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(theirs, vec!["operation:theirs".to_string(), product]);
}

#[tokio::test(flavor = "multi_thread")]
async fn rooms_list_their_present_agent_members_by_name() {
    let fx = fixture().await;
    let srv = serve(&fx).await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let gone = direct(&fx, &sup).await;
    let room = op_room(&sup);
    fx.store.remove_member(&room, &gone.0, 7).await.unwrap();
    // The owner gains a member row by reading; it is still no recipient.
    let (status, v) = post_as_owner(
        &srv,
        &room,
        json!({ "kind": "announcement", "text": "hello" }),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    read_posts(&srv, &room, "").await;
    assert!(fx
        .store
        .room_members(&room)
        .await
        .unwrap()
        .iter()
        .any(|m| m.member == "owner"));

    let listed = listed(&srv, PERSONA).await;
    let r = listed.iter().find(|r| r.id == room).unwrap();
    let got: Vec<(String, String)> = r
        .members
        .iter()
        .map(|m| (m.id.0.clone(), m.name.clone()))
        .collect();
    // Nobody has posted: the members come from the room, named as their posts would be.
    assert_eq!(
        got,
        vec![
            (sup.0.clone(), "supervisor".to_string()),
            (a.0.clone(), task_name(&fx, &a).await)
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owners_post_with_no_recipient_is_to_everyone() {
    let fx = fixture().await;
    let srv = serve(&fx).await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let (status, v) = post_as_owner(
        &srv,
        &op_room(&sup),
        json!({ "kind": "message", "to": null, "text": "everyone" }),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let p: Post = serde_json::from_value(v).unwrap();
    assert_eq!(p.to, None);
    // A message to everyone wakes every agent in the room: the orchestrator gets it in its inbox.
    assert!(
        wait_until(|| async {
            fx.store
                .task(&a)
                .await
                .unwrap()
                .unwrap()
                .inbox
                .iter()
                .any(|e| e.kind == fleet_api::FleetEventKind::RoomPost)
        })
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_a_room_clears_the_owners_unread() {
    let fx = fixture().await;
    let srv = serve(&fx).await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let product = format!("product:{PRODUCT}");
    let first = post_as(&fx, &a, "first", Some(&product)).await;
    rooms::announce::landed(&fx.store, PRODUCT, "the thing", "https://example.test/pr/1")
        .await
        .unwrap();
    let (status, v) = post_as_owner(
        &srv,
        &product,
        json!({ "kind": "message", "text": "noted" }),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let unread = |srv| async move {
        listed(srv, PERSONA)
            .await
            .into_iter()
            .find(|r| r.id == format!("product:{PRODUCT}"))
            .unwrap()
            .unread
    };
    assert_eq!(unread(&srv).await, 2);

    // Newest last, every sender named.
    let posts = read_posts(&srv, &product, "").await;
    let got: Vec<(&str, &str)> = posts
        .iter()
        .map(|p| (p.sender_name.as_str(), p.text.as_str()))
        .collect();
    let name = task_name(&fx, &a).await;
    assert_eq!(
        got,
        vec![
            (name.as_str(), "first"),
            (
                "Fleet Manager",
                "landed: the thing · https://example.test/pr/1"
            ),
            ("owner", "noted"),
        ]
    );
    assert_eq!(posts[0].id, first);
    assert_eq!(unread(&srv).await, 0);

    // Pages: the latest `limit`, and those before an id.
    let latest = read_posts(&srv, &product, "?limit=1").await;
    assert_eq!(
        latest.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(),
        vec!["noted"]
    );
    let older = read_posts(&srv, &product, &format!("?before={}", latest[0].id)).await;
    assert_eq!(older.len(), 2);
    assert_eq!(older[0].id, first);

    post_as(&fx, &a, "again", Some(&product)).await;
    assert_eq!(unread(&srv).await, 1);
    let (status, _) = json_of(srv.addr, "GET", "/api/rooms/nowhere/posts", None).await;
    assert_eq!(status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_posts_messages_and_announcements_not_asks() {
    let fx = fixture().await;
    let srv = serve(&fx).await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);

    let (status, v) = post_as_owner(
        &srv,
        &room,
        json!({ "kind": "message", "to": { "agent": a.0 }, "text": "rebase first" }),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let p: Post = serde_json::from_value(v).unwrap();
    assert_eq!(
        (
            &p.sender,
            p.sender_name.as_str(),
            p.to.as_ref(),
            p.kind,
            p.text.as_str()
        ),
        (
            &MemberRef::Owner,
            "owner",
            Some(&MemberRef::Agent(a.clone())),
            PostKind::Message,
            "rebase first"
        )
    );
    // A message wakes its recipient: the working orchestrator gets it in its inbox.
    assert!(
        wait_until(|| async {
            fx.store
                .task(&a)
                .await
                .unwrap()
                .unwrap()
                .inbox
                .iter()
                .any(|e| e.kind == fleet_api::FleetEventKind::RoomPost)
        })
        .await
    );

    let (status, v) = post_as_owner(
        &srv,
        &room,
        json!({ "kind": "announcement", "text": "freeze at five" }),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["to"], Value::Null);

    for body in [
        json!({ "kind": "ask", "text": "which port?" }),
        json!({ "kind": "answer", "text": "this one" }),
        json!({ "kind": "message", "text": "  " }),
        json!({ "kind": "message", "to": { "agent": "nobody" }, "text": "hi" }),
        // Agent ids are checked as ids: neither names the owner or everyone.
        json!({ "kind": "message", "to": { "agent": "owner" }, "text": "hi" }),
        json!({ "kind": "message", "to": { "agent": "all" }, "text": "hi" }),
    ] {
        let (status, v) = post_as_owner(&srv, &room, body.clone()).await;
        assert_eq!(status, 400, "{body} → {v}");
    }
    let (status, _) =
        post_as_owner(&srv, "nowhere", json!({ "kind": "message", "text": "hi" })).await;
    assert_eq!(status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn answering_twice_is_409() {
    let fx = fixture().await;
    let srv = serve(&fx).await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);
    let held = ask_in_background(&fx.svc, &a, "owner", "which port?");
    let ask = wait_ask(&fx, &room).await;

    // An ordinary ask is answered with text: `approve` alone, or blank text, is no answer.
    for body in [
        json!({ "approve": true }),
        json!({ "text": "  " }),
        json!({}),
    ] {
        let (status, v) = answer_as_owner(&srv, ask, body.clone()).await;
        assert_eq!(status, 400, "{body} → {v}");
    }
    assert_eq!(fx.store.post(ask).await.unwrap().unwrap().answered_at, None);
    // With text, `approve` changes nothing: the text is the answer.
    let (status, v) = answer_as_owner(&srv, ask, json!({ "approve": true, "text": "8080" })).await;
    assert_eq!(status, 204, "{v}");
    assert_eq!(answer_to(&fx, ask, &room).await.text, "8080");
    let (reply, failed) = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap();
    assert!(!failed && reply.contains("8080"), "{reply}");
    assert_eq!(answer_to(&fx, ask, &room).await.sender, MemberRef::Owner);

    let (status, v) = answer_as_owner(&srv, ask, json!({ "text": "9090" })).await;
    assert_eq!(
        (status, v["error"].as_str()),
        (409, Some("already answered"))
    );
    let (status, _) = answer_as_owner(&srv, 999_999, json!({ "text": "x" })).await;
    assert_eq!(status, 404);
    let note = post_as(&fx, &a, "not an ask", None).await;
    let (status, _) = answer_as_owner(&srv, note, json!({ "text": "x" })).await;
    assert_eq!(status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_approves_a_breakdown_from_the_room() {
    let fx = fixture().await;
    let srv = serve(&fx).await;
    let sup = conversation(&fx).await;
    let a = direct(&fx, &sup).await;
    let room = op_room(&sup);

    // Text asks for changes, through the same decision as the panel's.
    let (held, v1) = propose(&fx, &a, &room).await;
    // Approving drops a comment, so approve with text is refused and leaves the version undecided.
    let (status, v) = answer_as_owner(
        &srv,
        v1.id,
        json!({ "approve": true, "text": "but split it" }),
    )
    .await;
    assert_eq!(status, 400, "{v}");
    assert_eq!(
        fx.store.post(v1.id).await.unwrap().unwrap().answered_at,
        None
    );
    assert_eq!(
        fx.store.task(&a).await.unwrap().unwrap().breakdowns[0].approval,
        Approval::Proposed
    );
    let (status, v) = answer_as_owner(&srv, v1.id, json!({ "text": "split the api step" })).await;
    assert_eq!(status, 204, "{v}");
    assert_eq!(
        held_answer(held).await,
        BreakdownAnswer::ChangesRequested("split the api step".into())
    );

    // Only `approve: true` approves: the text `approve` requests changes.
    let (held, v2) = propose(&fx, &a, &room).await;
    let (status, v) = answer_as_owner(&srv, v2.id, json!({ "text": "approve" })).await;
    assert_eq!(status, 204, "{v}");
    assert_eq!(
        held_answer(held).await,
        BreakdownAnswer::ChangesRequested("approve".into())
    );

    let (held, v2) = propose(&fx, &a, &room).await;
    let (status, v) = answer_as_owner(&srv, v2.id, json!({ "approve": true })).await;
    assert_eq!(status, 204, "{v}");
    assert_eq!(held_answer(held).await, BreakdownAnswer::Approved);
    let p = fx.store.task(&a).await.unwrap().unwrap();
    assert_eq!(p.breakdowns[2].approval, Approval::Approved);
    assert_eq!(
        answer_to(&fx, v2.id, &room).await.text,
        "approved in the room"
    );
    // The panel finds it decided.
    let err = fx
        .svc
        .decide_breakdown(&a, decision(true, None))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NoPendingBreakdown), "{err:?}");

    let (status, v) = answer_as_owner(&srv, v2.id, json!({ "approve": true })).await;
    assert_eq!(
        (status, v["error"].as_str()),
        (409, Some("already answered"))
    );
    let (status, v) = answer_as_owner(&srv, v1.id, json!({ "approve": true })).await;
    assert_eq!(
        (status, v["error"].as_str()),
        (409, Some("breakdown v1 is stale: v3 is the latest"))
    );
}
