mod common;

use common::net::*;
use common::*;
use fleet_api::{
    parse_date, task_outcome, Actor, AgentId, AgentKind, PrState, Release, SpawnRequest, Task,
    TaskOutcome, TaskPr,
};
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::store::RepoRow;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const GHE_URL: &str = "https://ghe.example.com/acme/core.git/";
const MERGED_AT: &str = "2026-10-26T10:00:00Z";

async fn serve(fx: Fx) -> (SocketAddr, Fx) {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(
        fx.svc.clone(),
        fx.store.clone(),
        fx.tmux.clone(),
        addr.port(),
    );
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (addr, fx)
}

async fn call(addr: SocketAddr, method: &str, path: &str, body: Value) -> (u16, Value) {
    json_of(addr, method, path, Some(&body)).await
}

/// The persona's gh config dir: where the fake gh reads its replies and logs its argv.
async fn gh_dir(fx: &Fx) -> PathBuf {
    let env = fixture_env(fx).await;
    let dir = env
        .vars
        .iter()
        .find(|(k, _)| k == "GH_CONFIG_DIR")
        .map(|(_, v)| PathBuf::from(v))
        .expect("GH_CONFIG_DIR");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn reply_merged(dir: &Path) {
    let reply = json!([
        { "url": "https://ghe.example.com/acme/core/pull/6", "state": "CLOSED", "mergedAt": null },
        { "url": "https://ghe.example.com/acme/core/pull/7", "state": "MERGED", "mergedAt": MERGED_AT },
    ]);
    std::fs::write(dir.join("pr-list.json"), reply.to_string()).unwrap();
}

fn argv_log(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("argv.log")).unwrap_or_default()
}

async fn set_core_url(fx: &Fx, url: Option<&str>) {
    fx.store
        .upsert_repo(&RepoRow {
            product: PRODUCT.into(),
            name: "core".into(),
            path: Some(fx.repo.clone()),
            url: url.map(Into::into),
            roster: vec![],
        })
        .await
        .unwrap();
}

/// A stopped task record on `repo`, branch `fleet/<id>`.
async fn task_on(fx: &Fx, id: &str, repo: &str) -> AgentId {
    let t: Task = serde_json::from_value(json!({
        "id": id, "product": PRODUCT, "release": null, "name": id, "goal": "g",
        "workdir": fx.tasks_root.join(id).to_string_lossy(),
        "repos": [{
            "repo": repo, "branch": format!("fleet/{id}"), "worktree": "/nowhere", "base": "abc",
            "has_changes": false
        }],
        "state": "stopped", "agents_used": 1,
        "times": {"queued": 0, "working": 0, "needs_you": 0, "review": 0},
        "log": [], "persona": PERSONA
    }))
    .unwrap();
    fx.store.insert_task(&t).await.unwrap();
    t.id
}

/// A task on `core` with a GitHub Enterprise URL.
async fn github_task(fx: &Fx, id: &str) -> AgentId {
    set_core_url(fx, Some(GHE_URL)).await;
    task_on(fx, id, "core").await
}

async fn path_only_task(fx: &Fx, id: &str) -> AgentId {
    let lib = make_repo(&fx._tmp.path().join("lib"));
    add_path_repo(fx, "lib", &lib).await;
    task_on(fx, id, "lib").await
}

async fn record(fx: &Fx, id: &AgentId) -> Task {
    fx.store.task(id).await.unwrap().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn merged_pr_marks_task_done() {
    let fx = fixture().await;
    let dir = gh_dir(&fx).await;
    let id = github_task(&fx, "t1").await;
    let local = path_only_task(&fx, "t2").await;
    reply_merged(&dir);

    fx.svc.check_merges().await;

    let t = record(&fx, &id).await;
    let merged_at = parse_date("2026-10-26").unwrap() * 86_400 + 10 * 3600;
    assert_eq!(
        t.prs,
        vec![
            TaskPr {
                url: "https://ghe.example.com/acme/core/pull/6".into(),
                repo: "core".into(),
                state: PrState::Closed,
                merged_at: None,
            },
            TaskPr {
                url: "https://ghe.example.com/acme/core/pull/7".into(),
                repo: "core".into(),
                state: PrState::Merged,
                merged_at: Some(merged_at),
            },
        ]
    );
    assert_eq!(t.done_at, Some(merged_at));
    assert!(!t.done_by_hand);
    assert_eq!(task_outcome(&t), TaskOutcome::Merged);
    let log = argv_log(&dir);
    assert!(
        log.contains(
            "pr list -R ghe.example.com/acme/core --head fleet/t1 --state all --json url,state,mergedAt"
        ),
        "{log}"
    );
    // A task with no GitHub repo is never asked about.
    assert!(!log.contains("fleet/t2"), "{log}");
    assert!(record(&fx, &local).await.prs.is_empty());

    // A done task is not checked again.
    std::fs::remove_file(dir.join("argv.log")).unwrap();
    fx.svc.check_merges().await;
    assert!(!argv_log(&dir).contains("pr list"));
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_failure_changes_nothing() {
    let fx = fixture().await;
    let dir = gh_dir(&fx).await;
    let id = github_task(&fx, "t1").await;
    let open = TaskPr {
        url: "https://ghe.example.com/acme/core/pull/7".into(),
        repo: "core".into(),
        state: PrState::Open,
        merged_at: None,
    };
    let seen = open.clone();
    fx.store
        .update_task(&id, |t| t.prs = vec![seen])
        .await
        .unwrap();
    reply_merged(&dir);
    std::fs::write(dir.join("pr-fail"), "").unwrap();

    fx.svc.check_merges().await;

    assert!(argv_log(&dir).contains("pr list"));
    let t = record(&fx, &id).await;
    assert_eq!(t.prs, vec![open]);
    assert_eq!(t.done_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_changes_nothing() {
    // Only the PR query hangs: the persona's login check runs through the same program.
    let wrapper_dir = tempfile::tempdir().unwrap();
    let wrapper = wrapper_dir.path().join("gh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n[ \"$1 $2\" = \"pr list\" ] && export FAKE_GH_SLEEP=30\nexec {} \"$@\"\n",
            fake_gh()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let gh = wrapper.to_string_lossy().into_owned();
    let fx = fixture_options(
        |c| c.gh_command = gh,
        |o| o.merge_check_timeout = Duration::from_secs(1),
    )
    .await;
    let dir = gh_dir(&fx).await;
    let id = github_task(&fx, "t1").await;
    reply_merged(&dir);

    let started = Instant::now();
    fx.svc.check_merges().await;

    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(argv_log(&dir).contains("pr list"));
    let t = record(&fx, &id).await;
    assert!(t.prs.is_empty());
    assert_eq!(t.done_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_done_refused_with_url_repo() {
    let (addr, fx) = serve(fixture().await).await;
    let id = github_task(&fx, "t1").await;

    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/tasks/{}/done", id.0),
        json!({}),
    )
    .await;

    assert_eq!(s, 409, "{v}");
    assert_eq!(
        v["error"],
        "this task has a GitHub repo; it is done when its PR merges"
    );
    let t = record(&fx, &id).await;
    assert!(!t.done_by_hand && t.done_at.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_done_allowed_path_only() {
    let (addr, fx) = serve(fixture().await).await;
    let id = path_only_task(&fx, "t1").await;

    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/tasks/{}/done", id.0),
        json!({}),
    )
    .await;

    assert_eq!(s, 200, "{v}");
    let t: Task = serde_json::from_value(v).unwrap();
    assert!(t.done_by_hand);
    assert_eq!(t.done_at, Some(fx.clock.now_for_test()));
    assert_eq!(task_outcome(&t), TaskOutcome::DoneByHand);
    assert_eq!(record(&fx, &id).await, t);

    let (s, _) = call(addr, "POST", "/api/tasks/ghost/done", json!({})).await;
    assert_eq!(s, 404);

    // The client's call reaches the same route.
    let again = path_only_task_named(&fx, "t3").await;
    let client = fleet_client::http::FleetClient::new(&format!("http://{addr}"));
    let c = tokio::task::spawn_blocking(move || client.mark_done(&again.0))
        .await
        .unwrap()
        .unwrap();
    assert!(c.done_by_hand);
}

/// A second path-only task on the `lib` repo `path_only_task` added.
async fn path_only_task_named(fx: &Fx, id: &str) -> AgentId {
    task_on(fx, id, "lib").await
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_triggers_a_check() {
    let (addr, fx) = serve(fixture().await).await;
    let dir = gh_dir(&fx).await;
    let id = github_task(&fx, "t1").await;
    reply_merged(&dir);

    let (s, _) = http(addr, "POST", "/api/tasks/refresh", &[], None).await;

    assert_eq!(s, 202);
    assert!(
        wait_until(|| async { record(&fx, &id).await.done_at.is_some() }).await,
        "refresh did not check the task's PRs"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn finished_task_cannot_change_release() {
    let (addr, fx) = serve(fixture().await).await;
    let id = path_only_task(&fx, "t1").await;
    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/products/{PRODUCT}/releases"),
        json!({ "version": "1.0", "start": "2026-11-01", "target": "2026-12-01" }),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let r: Release = serde_json::from_value(v).unwrap();
    let uri = format!("/api/tasks/{}/release", id.0);
    let (s, v) = call(addr, "PUT", &uri, json!({ "release": r.id })).await;
    assert_eq!(s, 200, "{v}");
    fx.svc.mark_done(&id).await.unwrap();

    let (s, v) = call(addr, "PUT", &uri, json!({ "release": null })).await;

    assert_eq!(s, 409, "{v}");
    assert_eq!(
        v["error"],
        "a finished task stays in the release it finished in"
    );
    assert_eq!(record(&fx, &id).await.release, Some(r.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_an_agent_triggers_a_check() {
    let fx = fixture().await;
    let dir = gh_dir(&fx).await;
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&fx, "core")],
            prompt: "work".into(),
            slug: None,
            persona: PERSONA.into(),
            kind: AgentKind::Solo,
            role: None,
            roster: vec![],
        })
        .await
        .unwrap();
    set_core_url(&fx, Some(GHE_URL)).await;
    // A long period: only the run on start and the stop below check.
    fx.svc.start_merge_watcher(Duration::from_secs(3600));
    assert!(wait_until(|| async { fx.svc.merge_checks_run() >= 1 }).await);
    assert!(record(&fx, &a.id).await.done_at.is_none());
    reply_merged(&dir);

    fx.svc.stop(&a.id).await.unwrap();

    assert!(
        wait_until(|| async { record(&fx, &a.id).await.done_at.is_some() }).await,
        "the stop did not check the task's PRs"
    );
    let branch = &record(&fx, &a.id).await.repos[0].branch;
    assert!(argv_log(&dir).contains(&format!("--head {branch} ")));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_watcher_checks_again_on_its_timer() {
    let fx = fixture().await;
    let dir = gh_dir(&fx).await;
    let id = github_task(&fx, "t1").await;

    fx.svc.start_merge_watcher(Duration::from_millis(200));
    assert!(wait_until(|| async { fx.svc.merge_checks_run() >= 1 }).await);
    reply_merged(&dir);

    assert!(
        wait_until(|| async { record(&fx, &id).await.done_at.is_some() }).await,
        "the timer did not check again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn logged_out_persona_changes_nothing() {
    let fx = fixture().await;
    let dir = gh_dir(&fx).await;
    let id = github_task(&fx, "t1").await;
    reply_merged(&dir);
    std::fs::remove_file(dir.join("hosts.yml")).unwrap();

    fx.svc.check_merges().await;

    assert!(!argv_log(&dir).contains("pr list"));
    let t = record(&fx, &id).await;
    assert!(t.prs.is_empty());
    assert_eq!(t.done_at, None);
}
