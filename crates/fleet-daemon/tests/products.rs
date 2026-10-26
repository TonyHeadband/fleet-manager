mod common;

use common::net::*;
use common::*;
use fleet_api::{Actor, AgentKind, Product, ProductList, Release, RepoInfo, SpawnRequest, Task};
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::{json, Value};
use std::net::SocketAddr;

async fn start() -> (SocketAddr, Fx) {
    let fx = fixture().await;
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
            roster: vec![],
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

    // A URL may be added to a path-only repo; the roster and path stay.
    call(
        addr,
        "PUT",
        &format!("{uri}/extra/fleet"),
        json!({ "roster": [] }),
    )
    .await;
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
