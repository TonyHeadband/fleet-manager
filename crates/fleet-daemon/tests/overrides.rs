mod common;

use common::net::*;
use common::*;
use fleet_api::{
    EffectiveSettings, EffectiveValue, Effort, PermissionMode, Product, ProductList,
    ProductOverride, SettingSource,
};
use fleet_daemon::config::{GateBuild, LimitValues, PrSettings};
use fleet_daemon::fleet::defs::Defs;
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::overrides::{
    annotate, effective, resolve_effort, resolve_limits, resolve_mode, resolve_model, resolve_pr,
    validate,
};
use serde_json::{json, Value};
use std::net::SocketAddr;

async fn start_on(fx: &Fx) -> SocketAddr {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
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
        (entry("models", "rust", "opus;"), "models.rust"),
        (entry("models", "rust", "opus\u{1}"), "models.rust"),
        (entry("models", "rust", "opus\0"), "models.rust"),
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
    assert!(!effective(
        &LimitValues::default(),
        PermissionMode::Auto,
        Effort::Medium,
        &defs,
        &stored
    )
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

fn global_pr() -> PrSettings {
    PrSettings {
        style: "conventional".into(),
        attribution: "global".into(),
        draft: false,
        labels: vec!["fleet".into()],
        gate_build: GateBuild::Shared,
    }
}

#[test]
fn pr_override_wins_over_global() {
    let entries = [
        entry("pr", "style", "plain"),
        entry("pr", "attribution", "Made with Fleet Manager"),
        entry("pr", "draft", "true"),
        entry("pr", "labels", " bot , ,ui,"),
    ];
    let (pr, source) = resolve_pr(&global_pr(), &entries);
    assert_eq!(
        pr,
        PrSettings {
            style: "plain".into(),
            attribution: "Made with Fleet Manager".into(),
            draft: true,
            labels: vec!["bot".into(), "ui".into()],
            gate_build: GateBuild::Shared,
        }
    );
    assert_eq!(source, SettingSource::Product);

    let (pr, source) = resolve_pr(&global_pr(), &[]);
    assert_eq!(pr, global_pr());
    assert_eq!(source, SettingSource::Global);
    let (_, source) = resolve_pr(&PrSettings::default(), &[]);
    assert_eq!(source, SettingSource::Default);
    // The style's source only: a draft override does not make the style a product's.
    let (_, source) = resolve_pr(&global_pr(), &[entry("pr", "draft", "true")]);
    assert_eq!(source, SettingSource::Global);
}

#[test]
fn unknown_pr_key_is_refused() {
    let defs = Defs::new(std::env::temp_dir().join("no-user-defs-pr"));
    for (bad, named) in [
        (entry("pr", "colour", "x"), "pr.colour"),
        (entry("pr", "draft", "maybe"), "pr.draft"),
        (entry("pr", "attribution", "a\nb"), "pr.attribution"),
        (entry("pr", "style", "nope"), "pr.style"),
    ] {
        let err = validate(&[bad], &defs).unwrap_err();
        assert!(err.contains(named), "{err}");
    }
    validate(
        &[
            entry("pr", "style", "conventional-semver-pre1.0"),
            entry("pr", "draft", "false"),
            entry("pr", "labels", "a,b"),
            entry("pr", "attribution", "Made with Fleet Manager"),
        ],
        &defs,
    )
    .unwrap();
}

#[test]
fn pr_style_override_naming_a_missing_style() {
    let defs = Defs::new(std::env::temp_dir().join("no-user-defs-pr"));
    let listed = annotate(vec![entry("pr", "style", "gone")], &defs);
    assert_eq!(
        listed[0].error.as_deref(),
        Some("pr.style: no PR style named `gone`")
    );
    let listed = annotate(vec![entry("pr", "style", "plain")], &defs);
    assert_eq!(listed[0].error, None);
}

#[test]
fn max_pr_attempts_is_read_overridden_and_refused_at_zero() {
    let defs = Defs::new(std::env::temp_dir().join("no-user-defs-pr"));
    let mut global = LimitValues::default();
    let (limits, sources) = resolve_limits(&global, &[]);
    assert_eq!(limits.max_pr_attempts, 3);
    assert_eq!(sources[3], SettingSource::Default);

    global.max_pr_attempts = 5;
    let (limits, sources) = resolve_limits(&global, &[]);
    assert_eq!(
        (limits.max_pr_attempts, sources[3]),
        (5, SettingSource::Global)
    );

    let over = [entry("limits", "max_pr_attempts", "2")];
    let (limits, sources) = resolve_limits(&global, &over);
    assert_eq!(
        (limits.max_pr_attempts, sources[3]),
        (2, SettingSource::Product)
    );

    let err = validate(&[entry("limits", "max_pr_attempts", "0")], &defs).unwrap_err();
    assert!(err.contains("limits.max_pr_attempts"), "{err}");
}

#[test]
fn an_override_wins_over_the_definition_and_global() {
    let entries = [
        entry("modes", "rust", "manual"),
        entry("efforts", "rust", "max"),
    ];
    assert_eq!(
        resolve_mode(
            "rust",
            Some(PermissionMode::Auto),
            PermissionMode::AcceptEdits,
            &entries
        ),
        (PermissionMode::Manual, SettingSource::Product)
    );
    assert_eq!(
        resolve_effort("rust", Some(Effort::Low), Effort::High, &entries),
        (Effort::Max, SettingSource::Product)
    );
    assert_eq!(
        resolve_mode(
            "rust",
            Some(PermissionMode::Auto),
            PermissionMode::AcceptEdits,
            &[]
        ),
        (PermissionMode::Auto, SettingSource::Definition)
    );
    assert_eq!(
        resolve_effort("rust", None, Effort::High, &[]),
        (Effort::High, SettingSource::Global)
    );
    // An entry that does not parse is ignored, as for models.
    assert_eq!(
        resolve_mode(
            "rust",
            None,
            PermissionMode::Auto,
            &[entry("modes", "rust", "plan")]
        ),
        (PermissionMode::Auto, SettingSource::Global)
    );
}

#[test]
fn a_bad_mode_override_is_an_entry_error() {
    let defs = Defs::new(std::env::temp_dir().join("no-user-defs-modes"));
    let err = validate(&[entry("modes", "rust", "plan")], &defs).unwrap_err();
    assert_eq!(
        err,
        "modes.rust: mode plan is not allowed (auto, acceptEdits, manual, dontAsk)"
    );
    let err = validate(&[entry("efforts", "rust", "huge")], &defs).unwrap_err();
    assert_eq!(
        err,
        "efforts.rust: effort huge is not allowed (low, medium, high, xhigh, max)"
    );
    validate(
        &[
            entry("modes", "rust", "dontAsk"),
            entry("efforts", "rust", "xhigh"),
        ],
        &defs,
    )
    .unwrap();
}

#[test]
fn unknown_definition_key_is_an_error() {
    let defs = Defs::new(std::env::temp_dir().join("no-user-defs-keys"));
    for bad in [
        entry("modes", "nobody", "auto"),
        entry("efforts", "nobody", "low"),
    ] {
        let err = validate(std::slice::from_ref(&bad), &defs).unwrap_err();
        assert!(err.contains(&format!("{}.nobody", bad.section)), "{err}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn effective_lists_mode_and_effort_rows_with_sources() {
    let (addr, _fx) = start().await;
    assert_eq!(
        put(
            addr,
            &[
                entry("modes", "rust", "manual"),
                entry("efforts", "rust", "max")
            ]
        )
        .await
        .0,
        200
    );
    let e = effective_of(addr, &format!("?product={PRODUCT}")).await;
    let pair = |section, key| {
        let r = row(&e, section, key);
        (r.value.clone().unwrap(), r.source)
    };
    assert_eq!(
        pair("modes", "rust"),
        ("manual".into(), SettingSource::Product)
    );
    assert_eq!(
        pair("efforts", "rust"),
        ("max".into(), SettingSource::Product)
    );
    assert_eq!(
        pair("modes", "docs"),
        ("auto".into(), SettingSource::Definition)
    );
    assert_eq!(
        pair("efforts", "reviewer"),
        ("high".into(), SettingSource::Definition)
    );
}

#[test]
fn pr_gate_build_override_is_validated_and_wins_over_global() {
    let defs = Defs::new(std::env::temp_dir().join("no-user-defs-pr"));
    validate(&[entry("pr", "gate_build", "clean")], &defs).unwrap();
    let err = validate(&[entry("pr", "gate_build", "x")], &defs).unwrap_err();
    assert!(
        err.contains("pr.gate_build") && err.contains("must be shared or clean"),
        "{err}"
    );
    let (pr, _) = resolve_pr(&global_pr(), &[entry("pr", "gate_build", "clean")]);
    assert_eq!(pr.gate_build, GateBuild::Clean);
    let mut global = global_pr();
    global.gate_build = GateBuild::Clean;
    let (pr, _) = resolve_pr(&global, &[entry("pr", "gate_build", "shared")]);
    assert_eq!(pr.gate_build, GateBuild::Shared);
    let (pr, _) = resolve_pr(&global, &[]);
    assert_eq!(pr.gate_build, GateBuild::Clean);
}

#[test]
fn concurrent_tasks_resolve_product_over_global() {
    let over = [entry("limits", "max_concurrent_tasks", "2")];
    let mut global = LimitValues::default();
    global.max_concurrent_tasks = Some(4);
    let (limits, sources) = resolve_limits(&global, &over);
    assert_eq!(
        (limits.max_concurrent_tasks, sources[4]),
        (Some(2), SettingSource::Product)
    );
    let (limits, sources) = resolve_limits(&global, &[]);
    assert_eq!(
        (limits.max_concurrent_tasks, sources[4]),
        (Some(4), SettingSource::Global)
    );
    let (limits, sources) = resolve_limits(&LimitValues::default(), &[]);
    assert_eq!(
        (limits.max_concurrent_tasks, sources[4]),
        (None, SettingSource::Default)
    );
}

#[tokio::test]
async fn effective_lists_concurrent_tasks_without_a_cap() {
    let fx = fixture_config(|_| {}).await;
    let addr = start_on(&fx).await;
    let e = effective_of(addr, &format!("?product={PRODUCT}")).await;
    let cap = row(&e, "limits", "max_concurrent_tasks");
    assert_eq!(
        (cap.value.as_deref(), cap.source),
        (None, SettingSource::Default)
    );
}

#[test]
fn product_cannot_override_active_products() {
    let defs = Defs::new(std::env::temp_dir().join("no-user-defs-active"));
    let err = validate(&[entry("limits", "max_active_products", "1")], &defs).unwrap_err();
    assert!(err.contains("unknown limit"), "{err}");
}
