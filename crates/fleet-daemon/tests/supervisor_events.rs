mod common;

use common::*;
use fleet_api::{AgentId, EventKind, Status, TaskEvent, TaskEventKind};
use fleet_daemon::supervisor::{route, start_event_watcher, Supervisor};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Executor};
use std::time::Duration;

async fn conversation(fx: &Fx) -> AgentId {
    Supervisor::new(fx.svc.clone())
        .new_conversation(PERSONA, "what is going on?")
        .await
        .unwrap()
}

async fn needs_you(fx: &Fx, id: &AgentId, activity: &str) {
    fx.store
        .update_agent(id, |a| {
            a.status = Status::NeedsYou;
            a.activity = activity.into();
        })
        .await
        .unwrap();
}

async fn events(fx: &Fx, conversation: &AgentId) -> Vec<TaskEvent> {
    fx.store.task_events(conversation, 0).await.unwrap()
}

async fn origin(fx: &Fx, task: &AgentId, conversation: &AgentId) {
    let c = conversation.clone();
    fx.store
        .update_task(task, |t| t.origin = Some(c))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn needs_you_goes_to_the_origin_conversation() {
    let fx = fixture().await;
    start_event_watcher(fx.svc.clone());
    let started_it = conversation(&fx).await;
    let latest = conversation(&fx).await;
    let a = spawn_ready(&fx).await;
    origin(&fx, &a.id, &started_it).await;
    fx.store
        .update_task(&a.id, |t| t.title = Some("Build X".into()))
        .await
        .unwrap();

    // A supervisor needing you is not a task event.
    needs_you(&fx, &latest, "asking").await;
    needs_you(&fx, &a.id, "approve the plan").await;

    assert!(wait_until(|| async { !events(&fx, &started_it).await.is_empty() }).await);
    let got = events(&fx, &started_it).await;
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].kind, TaskEventKind::NeedsYou);
    assert_eq!(got[0].text, "needs you · approve the plan");
    assert_eq!(got[0].task, a.id);
    assert_eq!(got[0].task_title, "Build X");
    assert_eq!(got[0].persona, PERSONA);
    assert_eq!(got[0].conversation, Some(started_it));
    assert!(events(&fx, &latest).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_without_origin_goes_to_the_latest_conversation() {
    let fx = fixture().await;
    let _old = conversation(&fx).await;
    let latest = conversation(&fx).await;
    fx.store
        .touch_conversation(&latest, i64::MAX / 2)
        .await
        .unwrap();
    let a = spawn_ready(&fx).await;
    route(
        &fx.svc,
        &a.id,
        TaskEventKind::ChecksPassed,
        "checks passed".into(),
    )
    .await
    .unwrap();
    assert_eq!(events(&fx, &latest).await.len(), 1);

    // An origin that no longer exists falls back the same way.
    origin(&fx, &a.id, &AgentId("gone".into())).await;
    route_again(&fx, &a.id).await;
    assert_eq!(events(&fx, &latest).await.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_start_leaves_no_conversation_to_catch_events() {
    let (fx, db) = fixture_on_file().await;
    let live = conversation(&fx).await;
    let a = spawn_ready(&fx).await;
    // The next launch fails after the conversation's agent and row are stored.
    let mut conn = SqliteConnectOptions::new()
        .filename(&db)
        .connect()
        .await
        .unwrap();
    conn.execute(
        "CREATE TRIGGER no_launch BEFORE INSERT ON claude_sessions BEGIN SELECT RAISE(ABORT, 'disk full'); END",
    )
    .await
    .unwrap();
    let failed = Supervisor::new(fx.svc.clone())
        .new_conversation(PERSONA, "and now?")
        .await;
    assert!(failed.is_err(), "{failed:?}");
    let rows = fx.store.conversations(PERSONA).await.unwrap();
    assert_eq!(
        rows.iter().map(|c| &c.id).collect::<Vec<_>>(),
        vec![&live],
        "the failed start's row stays"
    );

    route_again(&fx, &a.id).await;
    assert_eq!(events(&fx, &live).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fallback_skips_a_conversation_whose_agent_is_gone() {
    let fx = fixture().await;
    let live = conversation(&fx).await;
    let gone = conversation(&fx).await;
    fx.store
        .touch_conversation(&gone, i64::MAX / 2)
        .await
        .unwrap();
    fx.store.remove_agent(&gone).await.unwrap();
    let a = spawn_ready(&fx).await;
    route_again(&fx, &a.id).await;
    assert_eq!(events(&fx, &live).await.len(), 1);
    assert!(events(&fx, &gone).await.is_empty());
}

async fn route_again(fx: &Fx, task: &AgentId) {
    route(
        &fx.svc,
        task,
        TaskEventKind::ChecksPassed,
        "checks passed".into(),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_persona_without_conversations_still_broadcasts() {
    let fx = fixture().await;
    let a = spawn_ready(&fx).await;
    let mut live = fx.store.subscribe();
    route_again(&fx, &a.id).await;
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), live.recv())
            .await
            .unwrap()
            .unwrap();
        if let EventKind::SupervisorEvent { event } = ev.kind {
            assert_eq!(event.conversation, None);
            assert_eq!(event.task, a.id);
            assert!(event.id > 0);
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_specialist_event_goes_to_its_task() {
    let fx = fixture().await;
    start_event_watcher(fx.svc.clone());
    let conv = conversation(&fx).await;
    let lead = spawn_ready(&fx).await;
    let specialist = spawn_ready(&fx).await;
    origin(&fx, &lead.id, &conv).await;
    let task = lead.id.clone();
    fx.store
        .update_agent(&specialist.id, |a| a.task = Some(task))
        .await
        .unwrap();

    needs_you(&fx, &specialist.id, "stuck").await;

    assert!(wait_until(|| async { !events(&fx, &conv).await.is_empty() }).await);
    assert_eq!(events(&fx, &conv).await[0].task, lead.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn needs_you_is_sent_once_per_transition() {
    let fx = fixture().await;
    start_event_watcher(fx.svc.clone());
    let conv = conversation(&fx).await;
    let a = spawn_ready(&fx).await;
    origin(&fx, &a.id, &conv).await;

    needs_you(&fx, &a.id, "one").await;
    needs_you(&fx, &a.id, "two").await;
    set_status(&fx, &a.id, Status::Working).await;
    needs_you(&fx, &a.id, "three").await;

    assert!(wait_until(|| async { events(&fx, &conv).await.len() >= 2 }).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let texts: Vec<String> = events(&fx, &conv)
        .await
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(texts, ["needs you · one", "needs you · three"]);
}
