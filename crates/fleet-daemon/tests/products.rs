mod common;

use common::net::*;
use common::*;
use fleet_api::{
    Actor, AgentKind, Product, ProductList, Release, RepoInfo, SpawnRequest, Task, TaskState,
};
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::{json, Value};
use std::net::SocketAddr;

async fn start() -> (SocketAddr, Fx) {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (addr, fx)
}

async fn call(addr: SocketAddr, method: &str, path: &str, body: Value) -> (u16, Value) {
    json_of(addr, method, path, Some(&body)).await
}

fn error_of(v: &Value) -> &str {
    v["error"]
        .as_str()
        .unwrap_or_else(|| panic!("no error in {v}"))
}

async fn new_product(addr: SocketAddr, name: &str) -> Product {
    let (s, v) = call(addr, "POST", "/api/products", json!({ "name": name })).await;
    assert_eq!(s, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

async fn new_release(addr: SocketAddr, product: &str, version: &str) -> Release {
    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/products/{product}/releases"),
        json!({ "version": version, "start": "2026-11-01", "target": "2026-12-01" }),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

async fn spawn_in_fixture_product(fx: &Fx, prompt: &str) -> fleet_api::Agent {
    fx.svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(fx, "core")],
            prompt: prompt.into(),
            slug: None,
            persona: PERSONA.into(),
            kind: AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn create_list_rename_delete_product() {
    let (addr, _fx) = start().await;
    let (s, v) = call(
        addr,
        "POST",
        "/api/products",
        json!({ "name": "My App", "default_persona": PERSONA }),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let p: Product = serde_json::from_value(v).unwrap();
    assert_eq!(
        (p.id.as_str(), p.name.as_str(), p.default_persona.as_deref()),
        ("my-app", "My App", Some(PERSONA))
    );
    assert_eq!((p.repos.len(), p.releases.len(), p.tasks), (0, 0, 0));

    // Another name with the same slug gets the next free id.
    assert_eq!(new_product(addr, "my app").await.id, "my-app-2");

    let (_, v) = json_of(addr, "GET", "/api/products", None).await;
    let list: ProductList = serde_json::from_value(v).unwrap();
    let ids: Vec<&str> = list.products.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, [PRODUCT, "my-app", "my-app-2"]);

    let (s, v) = call(
        addr,
        "PATCH",
        "/api/products/my-app",
        json!({ "name": "Renamed" }),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let p: Product = serde_json::from_value(v).unwrap();
    // The id never changes, and a field left out stays.
    assert_eq!(
        (p.id.as_str(), p.name.as_str(), p.default_persona.as_deref()),
        ("my-app", "Renamed", Some(PERSONA))
    );
    let (_, v) = call(
        addr,
        "PATCH",
        "/api/products/my-app",
        json!({ "default_persona": null }),
    )
    .await;
    assert_eq!(v["default_persona"], Value::Null);
    let (s, _) = call(addr, "PATCH", "/api/products/nope", json!({ "name": "x" })).await;
    assert_eq!(s, 404);

    let (s, v) = call(addr, "POST", "/api/products", json!({ "name": "  " })).await;
    assert_eq!(s, 400, "{v}");
    assert!(!error_of(&v).contains('\n'));
    let (s, _) = call(
        addr,
        "POST",
        "/api/products",
        json!({ "name": "Q", "default_persona": "ghost" }),
    )
    .await;
    assert_eq!(s, 400);

    assert_eq!(
        http(addr, "DELETE", "/api/products/my-app", &[], None)
            .await
            .0,
        204
    );
    assert_eq!(
        http(addr, "DELETE", "/api/products/my-app", &[], None)
            .await
            .0,
        404
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_product_name_is_409() {
    let (addr, _fx) = start().await;
    new_product(addr, "Alpha").await;
    let (s, v) = call(addr, "POST", "/api/products", json!({ "name": "Alpha" })).await;
    assert_eq!(s, 409, "{v}");
    let other = new_product(addr, "Beta").await;
    let (s, _) = call(
        addr,
        "PATCH",
        &format!("/api/products/{}", other.id),
        json!({ "name": "Alpha" }),
    )
    .await;
    assert_eq!(s, 409);
}

#[tokio::test(flavor = "multi_thread")]
async fn product_with_tasks_cannot_be_deleted() {
    let (addr, fx) = start().await;
    let a = spawn_in_fixture_product(&fx, "work").await;
    let (s, v) = json_of(addr, "DELETE", &format!("/api/products/{PRODUCT}"), None).await;
    assert_eq!(s, 409, "{v}");
    assert!(error_of(&v).contains("task"), "{v}");
    let (_, v) = json_of(addr, "GET", "/api/products", None).await;
    assert_eq!(v["products"][0]["tasks"], 1);
    fx.svc.stop(&a.id).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_needs_path_or_url() {
    let (addr, _fx) = start().await;
    let (s, v) = call(
        addr,
        "PUT",
        &format!("/api/products/{PRODUCT}/repos"),
        json!({ "name": "x" }),
    )
    .await;
    assert_eq!(s, 400, "{v}");
    let (s, _) = call(
        addr,
        "PUT",
        "/api/products/nope/repos",
        json!({ "name": "x", "url": "https://github.com/o/r.git" }),
    )
    .await;
    assert_eq!(s, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_url_is_refused() {
    let (addr, fx) = start().await;
    let uri = format!("/api/products/{PRODUCT}/repos");
    for url in [
        "git@github.com:o/r.git",
        "ssh://git@github.com/o/r.git",
        "http://github.com/o/r.git",
        "https://user:secret@github.com/o/r.git",
        "https://token@github.com/o/r.git",
    ] {
        let (s, v) = call(addr, "PUT", &uri, json!({ "name": "r", "url": url })).await;
        assert_eq!(s, 400, "{url}: {v}");
        assert!(!v.to_string().contains("secret"), "{v}");
    }
    assert!(fx.store.repo(PRODUCT, "r").await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn repos_are_added_filled_in_and_removed() {
    let (addr, fx) = start().await;
    let uri = format!("/api/products/{PRODUCT}/repos");
    let dir = make_repo(&fx._tmp.path().join("extra"));
    let path = dir.to_string_lossy().into_owned();

    let (s, v) = call(addr, "PUT", &uri, json!({ "name": "extra", "path": path })).await;
    assert_eq!(s, 200, "{v}");
    let info: RepoInfo = serde_json::from_value(v).unwrap();
    assert_eq!((info.product.as_str(), info.url), (PRODUCT, None));

    // A path that is a subfolder, or no repo at all, is no toplevel.
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    for bad in [dir.join("sub"), fx._tmp.path().join("missing")] {
        let (s, v) = call(addr, "PUT", &uri, json!({ "name": "bad", "path": bad })).await;
        assert_eq!(s, 400, "{v}");
    }

    // A URL may be added to a path-only repo; the path stays.
    let url = "https://github.com/o/extra.git";
    let (s, v) = call(addr, "PUT", &uri, json!({ "name": "extra", "url": url })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (v["path"].as_str(), v["url"].as_str()),
        (Some(path.as_str()), Some(url))
    );

    // A path may be added to a URL-only repo.
    let (s, _) = call(
        addr,
        "PUT",
        &uri,
        json!({ "name": "remote", "url": "https://github.com/o/remote.git" }),
    )
    .await;
    assert_eq!(s, 200);
    let other = make_repo(&fx._tmp.path().join("remote-local"));
    let (s, v) = call(
        addr,
        "PUT",
        &uri,
        json!({ "name": "remote", "path": other }),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["path"].is_string() && v["url"].is_string(), "{v}");

    // Saying the same URL again is no change.
    let (s, _) = call(addr, "PUT", &uri, json!({ "name": "extra", "url": url })).await;
    assert_eq!(s, 200);

    assert_eq!(
        http(addr, "DELETE", &format!("{uri}/extra"), &[], None)
            .await
            .0,
        204
    );
    assert_eq!(
        http(addr, "DELETE", &format!("{uri}/extra"), &[], None)
            .await
            .0,
        404
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_a_repos_url_is_409() {
    let (addr, fx) = start().await;
    let uri = format!("/api/products/{PRODUCT}/repos");
    let first = "https://github.com/o/r.git";
    let (s, _) = call(addr, "PUT", &uri, json!({ "name": "r", "url": first })).await;
    assert_eq!(s, 200);
    let (s, v) = call(
        addr,
        "PUT",
        &uri,
        json!({ "name": "r", "url": "https://github.com/o/other.git" }),
    )
    .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(
        error_of(&v),
        "r already has a URL; remove the repo and add it again to change it"
    );
    let row = fx.store.repo(PRODUCT, "r").await.unwrap().unwrap();
    assert_eq!(row.url.as_deref(), Some(first));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_a_running_task_uses_cannot_be_removed() {
    let (addr, fx) = start().await;
    let a = spawn_in_fixture_product(&fx, "work").await;
    let uri = format!("/api/products/{PRODUCT}/repos/core");
    assert_eq!(http(addr, "DELETE", &uri, &[], None).await.0, 409);
    fx.svc.stop(&a.id).await.unwrap();
    assert_eq!(http(addr, "DELETE", &uri, &[], None).await.0, 204);
}

#[tokio::test(flavor = "multi_thread")]
async fn release_dates_are_validated() {
    let (addr, _fx) = start().await;
    let uri = format!("/api/products/{PRODUCT}/releases");
    let make = |version: &str, start: &str, target: &str| json!({ "version": version, "start": start, "target": target });
    let (s, v) = call(addr, "POST", &uri, make("1", "2026-02-30", "2026-03-01")).await;
    assert_eq!(s, 400, "{v}");
    assert!(!error_of(&v).contains('\n'));
    let (s, _) = call(addr, "POST", &uri, make("1", "2026-11-02", "2026-11-01")).await;
    assert_eq!(s, 400);
    let (s, _) = call(addr, "POST", &uri, make("1", "soon", "2026-11-01")).await;
    assert_eq!(s, 400);
    let (s, v) = call(addr, "POST", &uri, make("1", "2026-11-02", "2026-11-02")).await;
    assert_eq!(s, 200, "{v}");
    let r: Release = serde_json::from_value(v).unwrap();
    assert_eq!((r.product.as_str(), r.released_at), (PRODUCT, None));

    // A patch is held to the same rules, against the dates it leaves alone.
    let (s, _) = call(
        addr,
        "PATCH",
        &format!("/api/releases/{}", r.id),
        json!({ "target": "2026-11-01" }),
    )
    .await;
    assert_eq!(s, 400);
    let (s, _) = call(
        addr,
        "PATCH",
        "/api/releases/9999",
        json!({ "target": "2026-11-03" }),
    )
    .await;
    assert_eq!(s, 404);
    let (s, _) = call(
        addr,
        "POST",
        "/api/products/nope/releases",
        make("1", "2026-11-01", "2026-11-02"),
    )
    .await;
    assert_eq!(s, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_version_is_409() {
    let (addr, _fx) = start().await;
    new_release(addr, PRODUCT, "1.0").await;
    let (s, v) = call(
        addr,
        "POST",
        &format!("/api/products/{PRODUCT}/releases"),
        json!({ "version": "1.0", "start": "2026-11-01", "target": "2026-12-01" }),
    )
    .await;
    assert_eq!(s, 409, "{v}");
    // Another product may use the version.
    let other = new_product(addr, "Other").await;
    new_release(addr, &other.id, "1.0").await;
    let two = new_release(addr, PRODUCT, "2.0").await;
    let (s, _) = call(
        addr,
        "PATCH",
        &format!("/api/releases/{}", two.id),
        json!({ "version": "1.0" }),
    )
    .await;
    assert_eq!(s, 409);
}

#[tokio::test(flavor = "multi_thread")]
async fn task_cannot_join_another_products_release() {
    let (addr, fx) = start().await;
    let other = new_product(addr, "Other").await;
    let theirs = new_release(addr, &other.id, "9.0").await;
    let ours = new_release(addr, PRODUCT, "1.0").await;
    let a = spawn_in_fixture_product(&fx, "work").await;
    let uri = format!("/api/tasks/{}/release", a.id.0);

    let (s, v) = call(addr, "PUT", &uri, json!({ "release": theirs.id })).await;
    assert_eq!(s, 400, "{v}");
    let (s, _) = call(addr, "PUT", &uri, json!({ "release": 9999 })).await;
    assert_eq!(s, 400);
    let (s, _) = call(
        addr,
        "PUT",
        "/api/tasks/ghost/release",
        json!({ "release": ours.id }),
    )
    .await;
    assert_eq!(s, 404);

    let (s, v) = call(addr, "PUT", &uri, json!({ "release": ours.id })).await;
    assert_eq!(s, 200, "{v}");
    let t: Task = serde_json::from_value(v).unwrap();
    assert_eq!(t.release, Some(ours.id));

    // Deleting the release sends its tasks back to the backlog.
    assert_eq!(
        http(
            addr,
            "DELETE",
            &format!("/api/releases/{}", ours.id),
            &[],
            None
        )
        .await
        .0,
        204
    );
    assert_eq!(fx.store.task(&a.id).await.unwrap().unwrap().release, None);
    assert_eq!(
        http(
            addr,
            "DELETE",
            &format!("/api/releases/{}", ours.id),
            &[],
            None
        )
        .await
        .0,
        404
    );

    let (s, v) = call(addr, "PUT", &uri, json!({ "release": null })).await;
    assert_eq!(s, 200, "{v}");
    fx.svc.stop(&a.id).await.unwrap();
}

/// Tasks join a release only until it is released; one already in it may still go back to the backlog.
#[tokio::test(flavor = "multi_thread")]
async fn released_release_takes_no_new_task() {
    let (addr, fx) = start().await;
    let r = new_release(addr, PRODUCT, "1.0").await;
    let a = spawn_in_fixture_product(&fx, "work").await;
    let uri = format!("/api/tasks/{}/release", a.id.0);
    let (s, v) = call(addr, "PUT", &uri, json!({ "release": r.id })).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = call(
        addr,
        "PATCH",
        &format!("/api/releases/{}", r.id),
        json!({ "released": true }),
    )
    .await;
    assert_eq!(s, 200, "{v}");

    let b = spawn_in_fixture_product(&fx, "more").await;
    let (s, v) = call(
        addr,
        "PUT",
        &format!("/api/tasks/{}/release", b.id.0),
        json!({ "release": r.id }),
    )
    .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(error_of(&v), "release 1.0 is already released");
    assert_eq!(fx.store.task(&b.id).await.unwrap().unwrap().release, None);

    let body = json!({
        "actor": "local", "product": PRODUCT, "release": r.id, "repos": [pick(&fx, "core")],
        "prompt": "late", "slug": null, "persona": PERSONA
    });
    let (s, v) = call(addr, "POST", "/api/agents", body).await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(error_of(&v), "release 1.0 is already released");

    // Already in it: back to the backlog is fine.
    let (s, v) = call(addr, "PUT", &uri, json!({ "release": null })).await;
    assert_eq!(s, 200, "{v}");
    fx.svc.stop(&a.id).await.unwrap();
    fx.svc.stop(&b.id).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_released_sets_released_at() {
    let (addr, fx) = start().await;
    let r = new_release(addr, PRODUCT, "1.0").await;
    let uri = format!("/api/releases/{}", r.id);
    fx.clock.advance(500);
    let now = fx.clock.now_for_test();

    let (s, v) = call(addr, "PATCH", &uri, json!({ "released": true })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["released_at"], now);
    // Marking it again does not move the stamp.
    fx.clock.advance(60);
    let (_, v) = call(addr, "PATCH", &uri, json!({ "released": true })).await;
    assert_eq!(v["released_at"], now);
    // A patch without `released` leaves it.
    let (_, v) = call(addr, "PATCH", &uri, json!({ "version": "1.0.1" })).await;
    assert_eq!(
        (v["version"].as_str(), v["released_at"].as_i64()),
        (Some("1.0.1"), Some(now))
    );

    let (_, v) = call(addr, "PATCH", &uri, json!({ "released": false })).await;
    assert_eq!(v["released_at"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_a_product_creates_its_internal_repo() {
    let (addr, fx) = start().await;
    let p = new_product(addr, "Fresh").await;
    let repo = fx.state_dir.join("internal").join(&p.id);
    assert!(repo.join(".git").exists());
    assert_eq!(git(&repo, &["branch", "--show-current"]), "main");
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_product_deletes_its_internal_repo() {
    let (addr, fx) = start().await;
    let p = new_product(addr, "Doomed").await;
    let repo = fx.state_dir.join("internal").join(&p.id);
    assert!(repo.exists());
    let (s, v) = json_of(addr, "DELETE", &format!("/api/products/{}", p.id), None).await;
    assert_eq!(s, 204, "{v}");
    assert!(!repo.exists());
}

fn bare_remote(dir: &std::path::Path) -> String {
    let bare = dir.join("internal.git");
    git(
        dir,
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    format!("file://{}", bare.display())
}

async fn set_internal_url(addr: SocketAddr, url: Value) -> (u16, Value) {
    call(
        addr,
        "PATCH",
        &format!("/api/products/{PRODUCT}"),
        json!({ "default_persona": PERSONA, "internal_url": url }),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn setting_internal_url_clones_and_shows_it() {
    let (addr, fx) = start().await;
    let remotes = tempfile::tempdir().unwrap();
    let url = bare_remote(remotes.path());

    let (s, v) = set_internal_url(addr, json!(url)).await;

    assert_eq!(s, 200, "{v}");
    assert_eq!(v["internal_url"], url);
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.internal_url.as_deref(), Some(url.as_str()));
    let remote = fx.svc.internal().remote_state(PRODUCT).await.unwrap();
    assert_eq!(remote.url.as_deref(), Some(url.as_str()));

    let (s, v) = set_internal_url(addr, Value::Null).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["internal_url"], Value::Null);
    assert_eq!(
        fx.svc.internal().remote_state(PRODUCT).await.unwrap().url,
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn internal_url_on_create_clones_before_storing() {
    let (addr, fx) = start().await;
    let remotes = tempfile::tempdir().unwrap();
    let url = bare_remote(remotes.path());

    let (s, v) = call(
        addr,
        "POST",
        "/api/products",
        json!({ "name": "Notes", "default_persona": PERSONA, "internal_url": url }),
    )
    .await;

    assert_eq!(s, 200, "{v}");
    assert_eq!(v["internal_url"], url);
    let remote = fx.svc.internal().remote_state("notes").await.unwrap();
    assert_eq!(remote.url.as_deref(), Some(url.as_str()));

    let (s, v) = call(
        addr,
        "POST",
        "/api/products",
        json!({ "name": "Bare", "internal_url": url }),
    )
    .await;
    assert_eq!(s, 400, "{v}");
    assert!(fx.store.product("bare").await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn internal_url_needs_a_default_persona() {
    let (addr, fx) = start().await;
    let remotes = tempfile::tempdir().unwrap();
    let url = bare_remote(remotes.path());

    let (s, v) = call(
        addr,
        "PATCH",
        &format!("/api/products/{PRODUCT}"),
        json!({ "internal_url": url }),
    )
    .await;

    assert_eq!(s, 400, "{v}");
    assert_eq!(error_of(&v), "an internal repo URL needs a default persona");
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.internal_url, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn internal_url_refused_while_a_task_has_notes() {
    let (addr, fx) = start().await;
    let remotes = tempfile::tempdir().unwrap();
    let url = bare_remote(remotes.path());
    let a = spawn_in_fixture_product(&fx, "notes").await;
    let name = fx.store.task(&a.id).await.unwrap().unwrap().name;
    fx.svc
        .internal()
        .attach_task(PRODUCT, &name, &remotes.path().join("notes"), None)
        .await
        .unwrap();

    let (s, v) = set_internal_url(addr, json!(url)).await;

    assert_eq!(s, 409, "{v}");
    assert!(error_of(&v).contains(&name), "{v}");
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.internal_url, None);

    fx.store
        .archive_task(&a.id, |t| t.state = TaskState::Archived)
        .await
        .unwrap();
    let (s, v) = set_internal_url(addr, json!(url)).await;
    assert_eq!(s, 200, "{v}");

    fx.store
        .archive_task(&a.id, |t| t.state = TaskState::Running)
        .await
        .unwrap();
    fx.svc
        .internal()
        .attach_task(PRODUCT, &name, &remotes.path().join("again"), None)
        .await
        .unwrap();
    let (s, v) = set_internal_url(addr, Value::Null).await;
    assert_eq!(s, 409, "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn clearing_default_persona_refused_while_internal_url_set() {
    let (addr, fx) = start().await;
    let remotes = tempfile::tempdir().unwrap();
    let url = bare_remote(remotes.path());
    let (s, v) = set_internal_url(addr, json!(url)).await;
    assert_eq!(s, 200, "{v}");

    let (s, v) = call(
        addr,
        "PATCH",
        &format!("/api/products/{PRODUCT}"),
        json!({ "default_persona": null }),
    )
    .await;

    assert_eq!(s, 400, "{v}");
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.default_persona.as_deref(), Some(PERSONA));
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_default_persona_checks_its_host_while_internal_url_set() {
    let (addr, fx) = start().await;
    let other_host = ready_persona(&fx.svc, "Enterprise", "ghe.example").await;
    let same_host = ready_persona(&fx.svc, "Second", "github.com").await;
    // Stored as a connect would leave it; nothing here reaches github.com.
    let mut row = fx.store.product(PRODUCT).await.unwrap().unwrap();
    row.default_persona = Some(PERSONA.into());
    row.internal_url = Some("https://github.com/o/notes.git".into());
    fx.store.update_product(&row).await.unwrap();
    let path = format!("/api/products/{PRODUCT}");
    let set_persona = |id: String| call(addr, "PATCH", &path, json!({ "default_persona": id }));

    let (s, v) = set_persona(other_host).await;
    assert_eq!(s, 400, "{v}");
    assert!(error_of(&v).contains("ghe.example"), "{v}");
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.default_persona.as_deref(), Some(PERSONA));

    let (s, v) = set_persona(same_host.clone()).await;
    assert_eq!(s, 200, "{v}");
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.default_persona, Some(same_host));
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_clone_stores_nothing() {
    let (addr, fx) = start().await;
    let remotes = tempfile::tempdir().unwrap();
    let missing = format!("file://{}", remotes.path().join("nope.git").display());
    let before = fx.svc.internal().ensure(PRODUCT).await.unwrap();

    let (s, v) = set_internal_url(addr, json!(missing)).await;

    assert_eq!(s, 400, "{v}");
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!((stored.internal_url, stored.default_persona), (None, None));
    assert!(before.is_dir());
    assert_eq!(
        fx.svc.internal().remote_state(PRODUCT).await.unwrap().url,
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_internal_url_is_refused() {
    let (addr, fx) = start().await;

    let (s, v) = set_internal_url(addr, json!("git@github.com:o/r.git")).await;

    assert_eq!(s, 400, "{v}");
    assert!(error_of(&v).contains("SSH"), "{v}");
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.internal_url, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn credentialed_internal_url_is_refused_and_never_shown() {
    let (addr, fx) = start().await;
    let secret = "https://u:secret@github.com/o/r.git";

    let (s, v) = set_internal_url(addr, json!(secret)).await;

    assert_eq!(s, 400, "{v}");
    assert!(!v.to_string().contains("secret"), "{v}");
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.internal_url, None);
    let (_, v) = json_of(addr, "GET", "/api/products", None).await;
    assert!(!v.to_string().contains("secret"), "{v}");

    let (s, v) = call(
        addr,
        "POST",
        "/api/products",
        json!({ "name": "Leaky", "default_persona": PERSONA, "internal_url": secret }),
    )
    .await;
    assert_eq!(s, 400, "{v}");
    assert!(!v.to_string().contains("secret"), "{v}");
    assert!(fx.store.product("leaky").await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_rename_leaves_the_internal_repo_alone() {
    let (addr, fx) = start().await;
    new_product(addr, "Taken").await;
    let remotes = tempfile::tempdir().unwrap();
    let url = bare_remote(remotes.path());

    let (s, v) = call(
        addr,
        "PATCH",
        &format!("/api/products/{PRODUCT}"),
        json!({ "name": "Taken", "default_persona": PERSONA, "internal_url": url }),
    )
    .await;

    assert_eq!(s, 409, "{v}");
    assert_eq!(
        fx.svc.internal().remote_state(PRODUCT).await.unwrap().url,
        None
    );
    let stored = fx.store.product(PRODUCT).await.unwrap().unwrap();
    assert_eq!(stored.internal_url, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_clone_on_create_stores_no_product() {
    let (addr, fx) = start().await;
    let remotes = tempfile::tempdir().unwrap();
    let missing = format!("file://{}", remotes.path().join("nope.git").display());

    let (s, v) = call(
        addr,
        "POST",
        "/api/products",
        json!({ "name": "Ghost", "default_persona": PERSONA, "internal_url": missing }),
    )
    .await;

    assert_eq!(s, 400, "{v}");
    assert!(fx.store.product("ghost").await.unwrap().is_none());
}
