mod common;

use common::fake_forge::Recorded;
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
use std::path::Path;
use std::time::{Duration, Instant};

const GHE_URL: &str = "https://ghe.example.com/acme/core.git/";
const LIB_URL: &str = "https://ghe.example.com/acme/lib";
const MERGED_AT: &str = "2026-10-26T10:00:00Z";

async fn serve(fx: Fx) -> (SocketAddr, Fx) {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (addr, fx)
}

async fn call(addr: SocketAddr, method: &str, path: &str, body: Value) -> (u16, Value) {
    json_of(addr, method, path, Some(&body)).await
}

fn reply_merged(fx: &Fx) {
    let prs = [rest_pr("core", 6, "CLOSED"), rest_pr("core", 7, "MERGED")];
    reply_for(fx, "core", &prs);
}

/// The PR-list requests the fake forge got.
fn pr_lists(fx: &Fx) -> Vec<Recorded> {
    fx.forge
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET" && r.path_and_query.contains("/pulls"))
        .collect()
}

async fn set_core_url(fx: &Fx, url: Option<&str>) {
    fx.store
        .upsert_repo(&RepoRow {
            product: PRODUCT.into(),
            name: "core".into(),
            path: Some(fx.repo.clone()),
            url: url.map(Into::into),
        })
        .await
        .unwrap();
}

/// Puts a commit on a new branch `branch` of the checkout at `dir`, without touching its working tree, and returns the
/// commit the branch started from.
fn branch_with_commit(dir: &Path, branch: &str) -> String {
    let base = git(dir, &["rev-parse", "HEAD"]);
    let tip = git(
        dir,
        &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "work"],
    );
    git(dir, &["branch", branch, &tip]);
    base
}

/// A stopped task record on `repo`, branch `fleet/<id>`, with a commit on that branch in the repo's checkout.
async fn task_on(fx: &Fx, id: &str, repo: &str) -> AgentId {
    task_on_repos(fx, id, &[repo]).await
}

async fn task_on_repos(fx: &Fx, id: &str, repos: &[&str]) -> AgentId {
    let branch = format!("fleet/{id}");
    let mut workspaces = Vec::new();
    for repo in repos {
        let row = fx.store.repo(PRODUCT, repo).await.unwrap().unwrap();
        let base = branch_with_commit(&row.path.unwrap(), &branch);
        workspaces.push(json!({
            "repo": repo, "branch": branch, "worktree": "/nowhere", "base": base, "has_changes": false
        }));
    }
    let t: Task = serde_json::from_value(json!({
        "id": id, "product": PRODUCT, "release": null, "name": id, "goal": "g",
        "workdir": fx.tasks_root.join(id).to_string_lossy(),
        "repos": workspaces,
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
    let id = github_task(&fx, "t1").await;
    let local = path_only_task(&fx, "t2").await;
    reply_merged(&fx);

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
    assert!(t.repos[0].changed);
    assert!(!t.done_by_hand);
    assert_eq!(task_outcome(&t), TaskOutcome::Merged);
    let lists = pr_lists(&fx);
    assert_eq!(lists.len(), 1, "{lists:?}");
    assert!(
        lists[0]
            .path_and_query
            .starts_with("/repos/acme/core/pulls?head=acme%3Afleet%2Ft1&state=all"),
        "{}",
        lists[0].path_and_query
    );
    // A task with no GitHub repo is never asked about.
    assert!(record(&fx, &local).await.prs.is_empty());

    // A done task is not checked again.
    fx.svc.check_merges().await;
    assert_eq!(pr_lists(&fx).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn gh_failure_changes_nothing() {
    let fx = fixture().await;
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
    reply_merged(&fx);
    fx.forge.fail_next_on("GET", "/pulls", 502, "Bad Gateway");

    fx.svc.check_merges().await;

    assert_eq!(pr_lists(&fx).len(), 1);
    let t = record(&fx, &id).await;
    assert_eq!(t.prs, vec![open]);
    assert_eq!(t.done_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_changes_nothing() {
    let fx = fixture_options(|_| {}, |o| o.merge_check_timeout = Duration::from_secs(1)).await;
    let id = github_task(&fx, "t1").await;
    reply_merged(&fx);
    fx.forge.delay_next(Duration::from_secs(5));

    let started = Instant::now();
    fx.svc.check_merges().await;

    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(pr_lists(&fx).len(), 1);
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
    let id = github_task(&fx, "t1").await;
    reply_merged(&fx);

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
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&fx, "core")],
            prompt: "work".into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    set_core_url(&fx, Some(GHE_URL)).await;
    // A long period: only the run on start and the stop below check.
    fx.svc.start_merge_watcher(Duration::from_secs(3600));
    assert!(wait_until(|| async { fx.svc.merge_checks_run() >= 1 }).await);
    assert!(record(&fx, &a.id).await.done_at.is_none());
    reply_merged(&fx);
    commit_in(&record(&fx, &a.id).await.repos[0].worktree);

    fx.svc.stop(&a.id).await.unwrap();

    assert!(
        wait_until(|| async { record(&fx, &a.id).await.done_at.is_some() }).await,
        "the stop did not check the task's PRs"
    );
    let branch = &record(&fx, &a.id).await.repos[0].branch;
    assert!(
        pr_lists(&fx).iter().any(|r| r
            .path_and_query
            .contains(&format!("head=acme%3A{}&", branch.replace('/', "%2F")))),
        "{:?}",
        pr_lists(&fx)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_watcher_checks_again_on_its_timer() {
    let fx = fixture().await;
    let id = github_task(&fx, "t1").await;

    fx.svc.start_merge_watcher(Duration::from_millis(200));
    assert!(wait_until(|| async { fx.svc.merge_checks_run() >= 1 }).await);
    reply_merged(&fx);

    assert!(
        wait_until(|| async { record(&fx, &id).await.done_at.is_some() }).await,
        "the timer did not check again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn logged_out_persona_changes_nothing() {
    let fx = fixture().await;
    let id = github_task(&fx, "t1").await;
    reply_merged(&fx);
    let persona = fx.svc.personas().get(PERSONA).await.unwrap();
    std::fs::remove_file(token_file(&persona)).unwrap();

    fx.svc.check_merges().await;

    assert!(pr_lists(&fx).is_empty());
    let t = record(&fx, &id).await;
    assert!(t.prs.is_empty());
    assert_eq!(t.done_at, None);
}

/// Seeing a merge needs only the GitHub login: a lapsed Claude login does not stop it.
#[tokio::test(flavor = "multi_thread")]
async fn merge_seen_with_claude_logged_out() {
    let fx = fixture().await;
    let id = github_task(&fx, "t1").await;
    reply_merged(&fx);
    let claude = fixture_env(&fx).await.claude_dir().unwrap();
    std::fs::remove_file(claude.join(".fake-login")).unwrap();

    fx.svc.check_merges().await;

    let t = record(&fx, &id).await;
    assert_eq!(t.done_at, Some(merged_at()));
    assert_eq!(task_outcome(&t), TaskOutcome::Merged);
}

/// A commit in the worktree at `path`, so the task's branch changed its repo.
fn commit_in(path: &str) {
    let wt = Path::new(path);
    std::fs::write(wt.join("CHANGE"), "x").unwrap();
    git(wt, &["add", "CHANGE"]);
    git(wt, &["commit", "-q", "-m", "change"]);
}

fn pr(repo: &str, n: u32, state: PrState, merged_at: Option<i64>) -> TaskPr {
    TaskPr {
        url: format!("https://ghe.example.com/acme/{repo}/pull/{n}"),
        repo: repo.into(),
        state,
        merged_at,
    }
}

/// A PR as the REST API lists it: a merged one is closed with a `merged_at`.
fn rest_pr(repo: &str, n: u32, state: &str) -> Value {
    let merged_at = (state == "MERGED").then_some(MERGED_AT);
    json!({
        "html_url": format!("https://ghe.example.com/acme/{repo}/pull/{n}"),
        "number": n,
        "state": if state == "OPEN" { "open" } else { "closed" },
        "merged_at": merged_at,
    })
}

fn reply_for(fx: &Fx, repo: &str, prs: &[Value]) {
    fx.forge.set_prs("acme", repo, prs.to_vec());
}

fn merged_at() -> i64 {
    parse_date("2026-10-26").unwrap() * 86_400 + 10 * 3600
}

/// `core` and a second repo `lib`, both with a GitHub Enterprise URL.
async fn two_github_repos(fx: &Fx) {
    set_core_url(fx, Some(GHE_URL)).await;
    let lib = make_repo(&fx._tmp.path().join("lib"));
    fx.store
        .upsert_repo(&RepoRow {
            product: PRODUCT.into(),
            name: "lib".into(),
            path: Some(lib),
            url: Some(LIB_URL.into()),
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_repo_without_pr_keeps_task_open() {
    let fx = fixture().await;
    two_github_repos(&fx).await;
    let id = task_on_repos(&fx, "t1", &["core", "lib"]).await;
    reply_for(&fx, "core", &[rest_pr("core", 7, "MERGED")]);

    fx.svc.check_merges().await;

    let t = record(&fx, &id).await;
    assert_eq!(
        t.prs,
        vec![pr("core", 7, PrState::Merged, Some(merged_at()))]
    );
    assert!(t.repos.iter().all(|w| w.changed), "{:?}", t.repos);
    assert_eq!(t.done_at, None);
    assert_eq!(task_outcome(&t), TaskOutcome::Open);
    assert!(pr_lists(&fx).iter().any(|r| r
        .path_and_query
        .starts_with("/repos/acme/lib/pulls?head=acme%3Afleet%2Ft1&")));

    // Once the other changed repo's PR merges too, the task is done.
    reply_for(&fx, "lib", &[rest_pr("lib", 3, "MERGED")]);
    fx.svc.check_merges().await;
    let t = record(&fx, &id).await;
    assert_eq!(t.done_at, Some(merged_at()));
    assert_eq!(task_outcome(&t), TaskOutcome::Merged);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_repo_failing_changes_nothing() {
    let fx = fixture().await;
    two_github_repos(&fx).await;
    let id = task_on_repos(&fx, "t1", &["core", "lib"]).await;
    let seen = vec![pr("lib", 3, PrState::Open, None)];
    let kept = seen.clone();
    fx.store.update_task(&id, |t| t.prs = kept).await.unwrap();
    reply_for(&fx, "core", &[rest_pr("core", 7, "MERGED")]);
    reply_for(&fx, "lib", &[rest_pr("lib", 3, "MERGED")]);
    fx.forge
        .fail_next_on("GET", "/repos/acme/lib/pulls", 502, "Bad Gateway");

    fx.svc.check_merges().await;

    assert!(pr_lists(&fx)
        .iter()
        .any(|r| r.path_and_query.starts_with("/repos/acme/lib/")));
    let t = record(&fx, &id).await;
    assert_eq!(t.prs, seen);
    assert!(t.repos.iter().all(|w| !w.changed), "{:?}", t.repos);
    assert_eq!(t.done_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_reply_keeps_a_newly_opened_pr() {
    let fx = fixture_options(|_| {}, |_| {}).await;
    let id = github_task(&fx, "t1").await;
    reply_for(&fx, "core", &[rest_pr("core", 7, "MERGED")]);

    // The fake logs a request on arrival and answers after the delay: a PR can be recorded in between.
    fx.forge.delay_next(Duration::from_millis(1500));
    let svc = fx.svc.clone();
    let check = tokio::spawn(async move { svc.check_merges().await });
    assert!(wait_until(|| async { !pr_lists(&fx).is_empty() }).await);
    // What `open_pr` records while the check waits for GitHub.
    let opened = pr("core", 8, PrState::Open, None);
    let recorded = opened.clone();
    fx.store
        .update_task(&id, |t| t.prs = vec![recorded])
        .await
        .unwrap();
    check.await.unwrap();

    let t = record(&fx, &id).await;
    assert_eq!(t.prs, vec![opened.clone()]);
    assert_eq!(t.done_at, None);

    // The task was queued again: that check starts from the PR just recorded, and GitHub now lists it.
    reply_for(
        &fx,
        "core",
        &[rest_pr("core", 7, "MERGED"), rest_pr("core", 8, "OPEN")],
    );
    assert!(wait_until(|| async { fx.svc.merge_checks_run() >= 1 }).await);
    let t = record(&fx, &id).await;
    assert_eq!(
        t.prs,
        vec![pr("core", 7, PrState::Merged, Some(merged_at())), opened]
    );
    assert_eq!(t.done_at, None);
}

/// A stopped solo task on `core` with a commit on its branch; `core` has a GitHub Enterprise URL.
async fn stopped_github_task(fx: &Fx) -> AgentId {
    stopped_github_task_changing(fx, true).await
}

/// A stopped solo task on `core`, with a commit on its branch when `change`; `core` has a GitHub Enterprise URL.
async fn stopped_github_task_changing(fx: &Fx, change: bool) -> AgentId {
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(fx, "core")],
            prompt: "work".into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    set_core_url(fx, Some(GHE_URL)).await;
    if change {
        commit_in(&record(fx, &a.id).await.repos[0].worktree);
    }
    fx.svc.stop(&a.id).await.unwrap();
    a.id
}

const OPEN_PR_REFUSAL: &str =
    "https://ghe.example.com/acme/core/pull/8 is still open; merge or close it before archiving";

#[tokio::test(flavor = "multi_thread")]
async fn archive_refused_with_open_pr() {
    let (addr, fx) = serve(fixture().await).await;
    let id = stopped_github_task(&fx).await;
    // Never recorded: the route's own check finds it.
    reply_for(&fx, "core", &[rest_pr("core", 8, "OPEN")]);

    let (s, v) = json_of(
        addr,
        "GET",
        &format!("/api/tasks/{}/archive-plan", id.0),
        None,
    )
    .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["error"], OPEN_PR_REFUSAL);

    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/tasks/{}/archive", id.0),
        json!({ "items": [] }),
    )
    .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["error"], OPEN_PR_REFUSAL);
    let t = record(&fx, &id).await;
    assert_eq!(t.state, fleet_api::TaskState::Stopped);
    assert!(
        t.log.iter().all(|l| !l.text.starts_with("archive failed")),
        "{:?}",
        t.log
    );
    assert_eq!(t.prs, vec![pr("core", 8, PrState::Open, None)]);

    // GitHub unreachable: the recorded open PR still refuses.
    fx.forge.fail_next_on("GET", "/pulls", 502, "Bad Gateway");
    let (s, v) = json_of(
        addr,
        "GET",
        &format!("/api/tasks/{}/archive-plan", id.0),
        None,
    )
    .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["error"], OPEN_PR_REFUSAL);
}

/// A solo agent may have opened a PR the record never saw: with GitHub unreachable and commits to lose, archive waits.
#[tokio::test(flavor = "multi_thread")]
async fn archive_refused_when_github_unreachable() {
    let (addr, fx) = serve(fixture().await).await;
    let id = stopped_github_task(&fx).await;
    let name = record(&fx, &id).await.name;
    let refusal = format!(
        "could not ask GitHub whether {name} has an open PR: GitHub said: Bad Gateway; try again"
    );

    fx.forge.fail_next_on("GET", "/pulls", 502, "Bad Gateway");
    let (s, v) = json_of(
        addr,
        "GET",
        &format!("/api/tasks/{}/archive-plan", id.0),
        None,
    )
    .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["error"], refusal);

    fx.forge.fail_next_on("GET", "/pulls", 502, "Bad Gateway");
    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/tasks/{}/archive", id.0),
        json!({ "items": [] }),
    )
    .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["error"], refusal);
    let t = record(&fx, &id).await;
    assert_eq!(t.state, fleet_api::TaskState::Stopped);
    assert!(
        t.log.iter().all(|l| !l.text.starts_with("archive failed")),
        "{:?}",
        t.log
    );
}

/// With nothing on the branch there is no PR to lose: archive goes ahead without GitHub.
#[tokio::test(flavor = "multi_thread")]
async fn archive_allowed_when_github_unreachable_and_nothing_changed() {
    let (addr, fx) = serve(fixture().await).await;
    let id = stopped_github_task_changing(&fx, false).await;
    fx.forge.fail_next_on("GET", "/pulls", 502, "Bad Gateway");

    let (s, plan) = json_of(
        addr,
        "GET",
        &format!("/api/tasks/{}/archive-plan", id.0),
        None,
    )
    .await;
    assert_eq!(s, 200, "{plan}");
    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/tasks/{}/archive", id.0),
        json!({ "items": plan["items"] }),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(record(&fx, &id).await.state, fleet_api::TaskState::Archived);
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_allowed_after_pr_closed() {
    let (addr, fx) = serve(fixture().await).await;
    let id = stopped_github_task(&fx).await;
    fx.store
        .update_task(&id, |t| t.prs = vec![pr("core", 8, PrState::Open, None)])
        .await
        .unwrap();
    reply_for(&fx, "core", &[rest_pr("core", 8, "CLOSED")]);

    let (s, plan) = json_of(
        addr,
        "GET",
        &format!("/api/tasks/{}/archive-plan", id.0),
        None,
    )
    .await;
    assert_eq!(s, 200, "{plan}");
    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/tasks/{}/archive", id.0),
        json!({ "items": plan["items"] }),
    )
    .await;
    assert_eq!(s, 200, "{v}");

    let t = record(&fx, &id).await;
    assert_eq!(t.state, fleet_api::TaskState::Archived);
    assert_eq!(t.prs, vec![pr("core", 8, PrState::Closed, None)]);
    assert_eq!(task_outcome(&t), TaskOutcome::Dropped);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleted_branch_with_merged_pr_finishes() {
    let fx = fixture().await;
    let id = github_task(&fx, "t1").await;
    // A merged branch deleted by hand: no checkout can count its commits any more.
    git(&fx.repo, &["branch", "-D", "fleet/t1"]);
    reply_merged(&fx);

    fx.svc.check_merges().await;

    let t = record(&fx, &id).await;
    assert!(t.repos[0].changed);
    assert_eq!(t.done_at, Some(merged_at()));
    assert_eq!(task_outcome(&t), TaskOutcome::Merged);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_checkout_changes_nothing() {
    let fx = fixture().await;
    let id = github_task(&fx, "t1").await;
    // No local path and no clone: git has nowhere to look.
    fx.store
        .upsert_repo(&RepoRow {
            product: PRODUCT.into(),
            name: "core".into(),
            path: None,
            url: Some(GHE_URL.into()),
        })
        .await
        .unwrap();
    reply_merged(&fx);

    fx.svc.check_merges().await;

    let t = record(&fx, &id).await;
    assert!(t.prs.is_empty());
    assert!(!t.repos[0].changed);
    assert_eq!(t.done_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_of_running_task_is_not_stopped() {
    let (addr, fx) = serve(fixture().await).await;
    let a = fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&fx, "core")],
            prompt: "work".into(),
            slug: None,
            title: None,
            persona: PERSONA.into(),
            kind: AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    set_core_url(&fx, Some(GHE_URL)).await;
    reply_for(&fx, "core", &[rest_pr("core", 8, "OPEN")]);

    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/tasks/{}/archive", a.id.0),
        json!({ "items": [] }),
    )
    .await;

    assert_eq!(s, 409, "{v}");
    assert_eq!(v["error"], "the task is not stopped; stop it first");
    assert!(pr_lists(&fx).is_empty());
    fx.svc.stop(&a.id).await.unwrap();
}

/// The `forge_calls` rows of the PR lists (the fixture's persona check left its own).
async fn watcher_calls(fx: &Fx) -> Vec<fleet_daemon::store::ForgeCallRow> {
    let mut rows = fx.store.forge_calls(50).await.unwrap();
    rows.retain(|r| r.op == "prs_of");
    rows
}

#[tokio::test(flavor = "multi_thread")]
async fn a_watch_check_records_one_call_per_repo() {
    let fx = fixture().await;
    two_github_repos(&fx).await;
    let id = task_on_repos(&fx, "t1", &["core", "lib"]).await;
    reply_for(&fx, "core", &[rest_pr("core", 7, "MERGED")]);
    reply_for(&fx, "lib", &[rest_pr("lib", 3, "MERGED")]);

    fx.svc.check_merges().await;

    let mut rows = watcher_calls(&fx).await;
    rows.sort_by(|a, b| a.repo.cmp(&b.repo));
    let seen: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.repo.as_str(),
                r.op.as_str(),
                r.ok,
                r.task.clone(),
                r.agent.clone(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            (
                "ghe.example.com/acme/core",
                "prs_of",
                true,
                Some(id.clone()),
                None
            ),
            ("ghe.example.com/acme/lib", "prs_of", true, Some(id), None),
        ]
    );
    assert!(rows.iter().all(|r| r.persona == PERSONA));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hung_forge_fails_the_check_after_the_timeout() {
    let (addr, fx) = serve(
        fixture_options(
            |_| {},
            |o| o.merge_check_timeout = Duration::from_millis(300),
        )
        .await,
    )
    .await;
    let id = stopped_github_task(&fx).await;
    fx.forge.delay_next(Duration::from_secs(3));

    let started = Instant::now();
    let (s, v) = json_of(
        addr,
        "GET",
        &format!("/api/tasks/{}/archive-plan", id.0),
        None,
    )
    .await;

    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(s, 409, "{v}");
    let error = v["error"].as_str().unwrap();
    assert!(error.contains("timed out"), "{error}");
    let rows = watcher_calls(&fx).await;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_merged_becomes_an_event_once() {
    let fx = fixture().await;
    let conv = fleet_daemon::supervisor::Supervisor::new(fx.svc.clone())
        .new_conversation(PERSONA, "hello")
        .await
        .unwrap();
    let id = github_task(&fx, "t1").await;
    let c = conv.clone();
    fx.store
        .update_task(&id, |t| t.origin = Some(c))
        .await
        .unwrap();
    reply_merged(&fx);

    fx.svc.check_merges().await;
    fx.svc.check_merges().await;

    let got = fx.store.task_events(&conv, 0).await.unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].kind, fleet_api::TaskEventKind::PrMerged);
    assert_eq!(
        got[0].text,
        "PR merged · https://ghe.example.com/acme/core/pull/7"
    );
    assert_eq!(got[0].task, id);
}
