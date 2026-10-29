mod common;

use common::*;
use fleet_api::{Actor, AgentId, FootprintItem, SpawnRequest, TaskState};
use fleet_daemon::footprint::{dir_bytes, measure};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn put(path: PathBuf, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
}

fn s(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// Polls until `f` returns Some, for at most 10 s: the measurer runs in the background.
async fn eventually<T, F: std::future::Future<Output = Option<T>>>(mut f: impl FnMut() -> F) -> T {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for a measurement");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[test]
fn measure_splits_build_output_and_daemon_files() {
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("proj");
    let agent_dir = tmp.path().join("agent");
    put(wd.join("core/src/a.rs"), 10);
    put(wd.join("core/target/x"), 1000);
    put(wd.join("CLAUDE.md"), 5);
    put(agent_dir.join("settings.json"), 7);
    let f = measure(
        &wd,
        &[("core".into(), wd.join("core"))],
        &agent_dir,
        &[],
        42,
    );
    assert_eq!(
        f.items[0],
        FootprintItem {
            label: "core worktree".into(),
            path: s(wd.join("core")),
            bytes: 1010,
            build_bytes: Some(1000)
        }
    );
    assert_eq!(f.items[1].label, "daemon files");
    assert_eq!(f.items[1].bytes, 12);
    assert_eq!(f.items.len(), 2);
    assert_eq!((f.total, f.measured_at), (1022, 42));
}

#[test]
fn measure_lists_other_files_in_the_task_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("proj");
    put(wd.join("core/a"), 10);
    put(wd.join("notes/n.txt"), 30);
    put(wd.join("PROMPT.md"), 4);
    let f = measure(
        &wd,
        &[("core".into(), wd.join("core"))],
        &tmp.path().join("none"),
        &[],
        1,
    );
    let labels: Vec<_> = f
        .items
        .iter()
        .map(|i| (i.label.as_str(), i.bytes))
        .collect();
    assert_eq!(
        labels,
        [
            ("core worktree", 10),
            ("daemon files", 4),
            ("other files", 30)
        ]
    );
    assert_eq!(f.items[0].build_bytes, None);
}

// Creating a symlink needs a privilege on Windows (Developer Mode); sub-project 2 decides how to test it there.
#[cfg(unix)]
#[test]
fn measure_does_not_follow_symlinks_out_of_the_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("proj");
    put(tmp.path().join("big/blob"), 5000);
    put(wd.join("core/a"), 10);
    std::os::unix::fs::symlink(tmp.path().join("big"), wd.join("core/link")).unwrap();
    // A link back to the folder itself must not loop either.
    std::os::unix::fs::symlink(&wd, wd.join("core/self")).unwrap();
    let f = measure(
        &wd,
        &[("core".into(), wd.join("core"))],
        &tmp.path().join("none"),
        &[],
        1,
    );
    assert_eq!(f.items[0].bytes, 10);
    assert_eq!(f.total, 10);
}

#[test]
fn dir_bytes_is_zero_for_a_missing_path() {
    assert_eq!(dir_bytes(Path::new("/nonexistent/fleet-footprint")), 0);
}

#[tokio::test]
async fn stop_triggers_a_measurement_that_lands_on_the_card() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&fx, "core")],
            prompt: "weigh me".into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    fx.svc.stop(&a.id).await.unwrap();
    let id = a.id.clone();
    let (disk, fp) = eventually(|| async {
        let agent = fx.svc.agent(&id).await.unwrap();
        let p = fx.store.task(&id).await.unwrap().unwrap();
        agent.disk_bytes.zip(p.footprint)
    })
    .await;
    assert!(disk > 0);
    assert_eq!(fp.total, disk);
    assert!(fp.items.iter().any(|i| i.label == "core worktree"));
}

#[tokio::test]
async fn a_crash_triggers_a_measurement() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&fx, "core")],
            prompt: "crash me".into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    fx.sessions.kill(&session_name(&a.id)).unwrap();
    fx.svc.reconcile().await.unwrap();
    let id = a.id.clone();
    let disk = eventually(|| async { fx.svc.agent(&id).await.unwrap().disk_bytes }).await;
    assert!(disk > 0);
}

#[tokio::test]
async fn measure_all_skips_archived_tasks_and_missing_folders() {
    let fx = fixture().await;
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&fx, "core")],
            prompt: "one".into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    let b = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&fx, "core")],
            prompt: "two".into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    fx.store
        .update_task(&b.id, |p| p.state = TaskState::Archived)
        .await
        .unwrap();
    fx.svc.request_measure_all();
    let id: AgentId = a.id.clone();
    eventually(|| async { fx.store.task(&id).await.unwrap().unwrap().footprint }).await;
    // One queue, one job at a time: once this later request has landed, the All job has passed b.
    fx.clock.advance(10);
    let later = fx.clock.now_for_test();
    fx.svc.request_measure(&a.id);
    eventually(|| async {
        let fp = fx.store.task(&id).await.unwrap().unwrap().footprint?;
        (fp.measured_at == later).then_some(())
    })
    .await;
    assert!(fx
        .store
        .task(&b.id)
        .await
        .unwrap()
        .unwrap()
        .footprint
        .is_none());
}

#[test]
fn a_nested_worktree_is_not_counted_in_other_files() {
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("proj");
    put(wd.join("sub/core/a"), 10);
    put(wd.join("sub/n.txt"), 30);
    let f = measure(
        &wd,
        &[("core".into(), wd.join("sub/core"))],
        &tmp.path().join("none"),
        &[],
        1,
    );
    let labels: Vec<_> = f
        .items
        .iter()
        .map(|i| (i.label.as_str(), i.bytes))
        .collect();
    assert_eq!(
        labels,
        [
            ("core worktree", 10),
            ("daemon files", 0),
            ("other files", 30)
        ]
    );
}

// Creating a symlink needs a privilege on Windows (Developer Mode); sub-project 2 decides how to test it there.
#[cfg(unix)]
#[test]
fn a_symlinked_build_dir_is_not_a_build_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("proj");
    put(wd.join("core/a"), 10);
    put(tmp.path().join("elsewhere/x"), 500);
    std::os::unix::fs::symlink(tmp.path().join("elsewhere"), wd.join("core/target")).unwrap();
    let f = measure(
        &wd,
        &[("core".into(), wd.join("core"))],
        &tmp.path().join("none"),
        &[],
        1,
    );
    assert_eq!(f.items[0].build_bytes, None);
}

#[test]
fn shared_build_output_is_its_own_item() {
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("proj");
    put(wd.join("core/a"), 10);
    put(wd.join("target/debug/x"), 1000);
    let f = measure(
        &wd,
        &[("core".into(), wd.join("core"))],
        &tmp.path().join("none"),
        &[wd.join("target")],
        1,
    );
    assert_eq!(
        f.items[1],
        FootprintItem {
            label: "build output".into(),
            path: s(wd.join("target")),
            bytes: 1000,
            build_bytes: Some(1000)
        }
    );
    assert!(f.items.iter().all(|i| i.label != "other files"), "{f:?}");
    assert_eq!(f.total, 1010);
}

// Creating a symlink needs a privilege on Windows (Developer Mode); sub-project 2 decides how to test it there.
#[cfg(unix)]
#[test]
fn a_linked_build_dir_is_not_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("proj");
    put(wd.join("core/a"), 10);
    put(tmp.path().join("elsewhere/x"), 500);
    std::os::unix::fs::symlink(tmp.path().join("elsewhere"), wd.join("target")).unwrap();
    let f = measure(
        &wd,
        &[("core".into(), wd.join("core"))],
        &tmp.path().join("none"),
        &[wd.join("target")],
        1,
    );
    assert!(f.items.iter().all(|i| i.label != "build output"), "{f:?}");
    assert_eq!(f.total, 10);
}

// Creating a symlink needs a privilege on Windows (Developer Mode); sub-project 2 decides how to test it there.
#[cfg(unix)]
#[test]
fn build_output_inside_a_worktree_is_not_weighed_twice() {
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("proj");
    put(wd.join("core/target/x"), 500);
    std::os::unix::fs::symlink(wd.join("core"), wd.join("out")).unwrap();
    let f = measure(
        &wd,
        &[("core".into(), wd.join("core"))],
        &tmp.path().join("none"),
        &[wd.join("core/target"), wd.join("out/target")],
        1,
    );
    assert!(f.items.iter().all(|i| i.label != "build output"), "{f:?}");
    assert_eq!(f.total, 500);
}
