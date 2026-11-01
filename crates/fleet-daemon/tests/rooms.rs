mod common;

use common::*;
use fleet_api::{AgentId, AgentKind, MemberRef, PrState, Room, RoomKind, TaskPr, TaskState};
use fleet_daemon::rooms;
use fleet_daemon::supervisor::Supervisor;

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
