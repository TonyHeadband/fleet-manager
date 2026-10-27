mod common;

use common::net::*;
use common::*;
use fleet_api::{
    EffectiveSettings, EffectiveValue, Product, ProductList, ProductOverride, SettingSource,
};
use fleet_daemon::config::LimitValues;
use fleet_daemon::fleet::defs::Defs;
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::overrides::{effective, resolve_model};
use serde_json::{json, Value};
use std::net::SocketAddr;

async fn start_on(fx: &Fx) -> SocketAddr {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(
        fx.svc.clone(),
        fx.store.clone(),
        fx.tmux.clone(),
        addr.port(),
    );
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    addr
}

async fn start() -> (SocketAddr, Fx) {
    let fx = fixture().await;
    (start_on(&fx).await, fx)
}

fn entry(section: &str, key: &str, value: &str) -> ProductOverride {
    ProductOverride {
        section: section.into(),
        key: key.into(),
        value: value.into(),
        error: None,
    }
}

async fn put(addr: SocketAddr, entries: &[ProductOverride]) -> (u16, Value) {
    let body = json!({ "overrides": entries });
    json_of(
        addr,
        "PUT",
        &format!("/api/products/{PRODUCT}/overrides"),
        Some(&body),
    )
    .await
}

async fn product(addr: SocketAddr) -> Product {
    let (s, v) = json_of(addr, "GET", "/api/products", None).await;
    assert_eq!(s, 200, "{v}");
    let list: ProductList = serde_json::from_value(v).unwrap();
    list.products.into_iter().find(|p| p.id == PRODUCT).unwrap()
}

async fn effective_of(addr: SocketAddr, query: &str) -> EffectiveSettings {
    let (s, v) = json_of(
        addr,
        "GET",
        &format!("/api/settings/effective{query}"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

fn row<'a>(e: &'a EffectiveSettings, section: &str, key: &str) -> &'a EffectiveValue {
    e.values
        .iter()
        .find(|v| v.section == section && v.key == key)
        .unwrap_or_else(|| panic!("no {section}.{key} in {e:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn put_and_read_overrides() {
    let (addr, _fx) = start().await;
    let entries = [
        entry("models", "rust", "opus"),
        entry("limits", "max_steps", "30"),
    ];
    let (s, v) = put(addr, &entries).await;
    assert_eq!(s, 200, "{v}");
    let p: Product = serde_json::from_value(v).unwrap();
    // Stored in section/key order, so a read is stable whatever order the client sent.
    assert_eq!(
        p.overrides,
        vec![
            entry("limits", "max_steps", "30"),
            entry("models", "rust", "opus")
        ]
    );
    assert_eq!(product(addr).await.overrides, p.overrides);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_entries_refuse_the_whole_list() {
    let (addr, _fx) = start().await;
    let good = [entry("limits", "max_steps", "30")];
    assert_eq!(put(addr, &good).await.0, 200);
    for (bad, named) in [
        (entry("colours", "x", "1"), "colours.x"),
        (entry("limits", "max_loops", "3"), "limits.max_loops"),
        (entry("limits", "max_steps", "0"), "limits.max_steps"),
        (entry("limits", "max_steps", "abc"), "limits.max_steps"),
        (entry("models", "nobody", "opus"), "models.nobody"),
        (entry("models", "rust", ""), "models.rust"),
        (entry("models", "rust", "sonnet --x"), "models.rust"),
    ] {
        let (s, v) = put(addr, &[entry("models", "rust", "opus"), bad]).await;
        assert_eq!(s, 400, "{v}");
        assert!(v["error"].as_str().unwrap().contains(named), "{v}");
        assert_eq!(product(addr).await.overrides, good);
    }
    let twice = [
        entry("models", "rust", "opus"),
        entry("models", "rust", "haiku"),
    ];
    let (s, v) = put(addr, &twice).await;
    assert_eq!(s, 400, "{v}");
    assert!(v["error"].as_str().unwrap().contains("models.rust"), "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_product_is_404() {
    let (addr, _fx) = start().await;
    let (s, _) = json_of(
        addr,
        "PUT",
        "/api/products/nope/overrides",
        Some(&json!({ "overrides": [] })),
    )
    .await;
    assert_eq!(s, 404);
    let (s, _) = json_of(addr, "GET", "/api/settings/effective?product=nope", None).await;
    assert_eq!(s, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn orphan_override_is_listed_invalid() {
    let (addr, fx) = start().await;
    fx.store
        .set_product_overrides(PRODUCT, &[entry("models", "gone", "opus")])
        .await
        .unwrap();
    let listed = product(addr).await.overrides;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].section.as_str(), listed[0].key.as_str()),
        ("models", "gone")
    );
    let error = listed[0].error.as_deref().expect("flagged");
    assert!(error.contains("gone"), "{error}");

    let e = effective_of(addr, &format!("?product={PRODUCT}")).await;
    assert!(!e.values.iter().any(|v| v.key == "gone"), "{e:?}");

    let defs = Defs::new(fx.state_dir.join("no-user-defs"));
    let stored = fx.store.product_overrides(PRODUCT).await.unwrap();
    assert!(!effective(&LimitValues::default(), &defs, &stored)
        .values
        .iter()
        .any(|v| v.key == "gone"));
    // The pure resolver still honours the entry: the definition may return.
    assert_eq!(
        resolve_model("gone", None, &stored).as_deref(),
        Some("opus")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn effective_reports_sources() {
    let fx = fixture_config(|c| c.limits.max_steps = 30).await;
    let addr = start_on(&fx).await;
    assert_eq!(
        put(addr, &[entry("limits", "max_parallel_specialists", "5")])
            .await
            .0,
        200
    );
    let e = effective_of(addr, &format!("?product={PRODUCT}")).await;
    let steps = row(&e, "limits", "max_steps");
    assert_eq!(
        (steps.value.as_deref(), steps.source),
        (Some("30"), SettingSource::Global)
    );
    let par = row(&e, "limits", "max_parallel_specialists");
    assert_eq!(
        (par.value.as_deref(), par.source),
        (Some("5"), SettingSource::Product)
    );
    let versions = row(&e, "limits", "max_breakdown_versions");
    assert_eq!(
        (versions.value.as_deref(), versions.source),
        (Some("8"), SettingSource::Default)
    );
    let rust = row(&e, "models", "rust");
    assert_eq!(
        (rust.value.as_deref(), rust.source),
        (Some("sonnet"), SettingSource::Definition)
    );

    // Without a product the override is not applied.
    let bare = effective_of(addr, "").await;
    assert_eq!(
        row(&bare, "limits", "max_parallel_specialists").source,
        SettingSource::Default
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn model_override_wins_over_the_definition() {
    let (addr, _fx) = start().await;
    assert_eq!(put(addr, &[entry("models", "rust", "opus")]).await.0, 200);
    let e = effective_of(addr, &format!("?product={PRODUCT}")).await;
    let rust = row(&e, "models", "rust");
    assert_eq!(
        (rust.value.as_deref(), rust.source),
        (Some("opus"), SettingSource::Product)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_product_drops_its_overrides() {
    let (addr, fx) = start().await;
    assert_eq!(
        put(addr, &[entry("limits", "max_steps", "30")]).await.0,
        200
    );
    let (s, v) = json_of(addr, "DELETE", &format!("/api/products/{PRODUCT}"), None).await;
    assert_eq!(s, 204, "{v}");
    assert!(fx
        .store
        .product_overrides(PRODUCT)
        .await
        .unwrap()
        .is_empty());
}
