mod common;

use common::*;
use fleet_api::{Actor, AgentId, Footprint, FootprintItem, SpawnRequest, TaskState};
use std::path::PathBuf;

fn put(path: PathBuf, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
}

fn item(label: &str, bytes: u64, build: Option<u64>) -> FootprintItem {
    FootprintItem {
        label: label.into(),
        path: String::new(),
        bytes,
        build_bytes: build,
    }
}

async fn task(fx: &Fx, prompt: &str, items: Vec<FootprintItem>) -> AgentId {
    let a = fx
        .svc
        .spawn(SpawnRequest {
            plan: None,
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(fx, "core")],
            prompt: prompt.into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    let total = items.iter().map(|i| i.bytes).sum();
    fx.store
        .update_task(&a.id, |p| {
            p.footprint = Some(Footprint {
                measured_at: 1,
                items,
                total,
            })
        })
        .await
        .unwrap();
    a.id
}

#[tokio::test]
async fn each_kind_lands_in_its_bucket() {
    let fx = fixture().await;
    task(
        &fx,
        "live",
        vec![
            item("core worktree", 1010, Some(1000)),
            item("build output", 500, Some(500)),
            item("internal", 7, None),
            item("other files", 3, None),
            // The daemon's files for the task are weighed in the state folder, not here.
            item("daemon files", 90_000, None),
        ],
    )
    .await;
    put(fx.state_dir.join("repos/core/pack"), 4000);
    put(fx.state_dir.join("transcripts/t1/a1/chat.json"), 300);
    put(fx.state_dir.join("notes.log"), 20);
    put(fx.state_dir.join("agents/x/settings.json"), 5);

    let s = fx.svc.storage().await;
    assert_eq!(s.worktrees_bytes, 10 + 7 + 3);
    assert_eq!(s.build_bytes, 1500);
    assert_eq!(s.repos_bytes, 4000);
    assert_eq!(s.transcripts_bytes, 300);
    // The state folder also holds whatever the fixture wrote (a database file, personas); at least ours is in it.
    assert!(s.daemon_bytes >= 25, "{s:?}");
}

#[tokio::test]
async fn an_archived_task_is_not_counted() {
    let fx = fixture().await;
    let live = task(&fx, "live", vec![item("core worktree", 100, None)]).await;
    let gone = task(&fx, "gone", vec![item("core worktree", 9000, Some(8000))]).await;
    fx.store
        .update_task(&gone, |p| p.state = TaskState::Archived)
        .await
        .unwrap();
    let s = fx.svc.storage().await;
    assert_eq!((s.worktrees_bytes, s.build_bytes), (100, 0), "{live:?}");
}

#[tokio::test]
async fn the_tasks_folder_is_not_daemon_files() {
    let fx = fixture().await;
    put(fx.state_dir.join("repos/core/pack"), 40);
    put(fx.state_dir.join("transcripts/t/c"), 30);
    let before = fx.svc.storage().await.daemon_bytes;
    put(fx.tasks_root.join("task/f"), 100_000);
    assert_eq!(fx.svc.storage().await.daemon_bytes, before);
}

#[tokio::test]
async fn free_and_total_space_of_the_tasks_volume_are_reported() {
    let fx = fixture().await;
    let s = fx.svc.storage().await;
    assert!(s.disk_total_bytes > 0, "{s:?}");
    assert!(s.disk_free_bytes > 0, "{s:?}");
    assert!(s.disk_free_bytes <= s.disk_total_bytes, "{s:?}");
}
