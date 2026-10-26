mod common;
use common::*;
use fleet_api::{Actor, AgentId, SpawnRequest};
use fleet_daemon::first_run::seed_first_run;
use fleet_daemon::hooks_cfg::write_settings;
use serde_json::{json, Value};

fn read(p: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn seeds_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    seed_first_run(dir.path()).unwrap();
    assert_eq!(
        read(&dir.path().join(".claude.json")),
        json!({"hasCompletedOnboarding": true, "hasSeenAutoDefaultNotice": true, "fullscreenUpsellSeenCount": 3})
    );
}

#[test]
fn adds_only_missing_keys() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(".claude.json");
    std::fs::write(
        &p,
        r#"{"fullscreenUpsellSeenCount":5,"hasCompletedOnboarding":false,"theme":"dark"}"#,
    )
    .unwrap();
    seed_first_run(dir.path()).unwrap();
    let v = read(&p);
    assert_eq!(v["fullscreenUpsellSeenCount"], 5);
    assert_eq!(v["hasCompletedOnboarding"], false);
    assert_eq!(v["hasSeenAutoDefaultNotice"], true);
    assert_eq!(v["theme"], "dark");
    std::fs::write(&p, r#"{"fullscreenUpsellSeenCount":1}"#).unwrap();
    seed_first_run(dir.path()).unwrap();
    assert_eq!(read(&p)["fullscreenUpsellSeenCount"], 3);
}

#[test]
fn leaves_an_unparsable_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(".claude.json");
    std::fs::write(&p, "{ not json").unwrap();
    seed_first_run(dir.path()).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ not json");
}

#[test]
fn agent_settings_set_the_classic_renderer() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_settings(
        dir.path(),
        "http://127.0.0.1:7878",
        &AgentId("a1".into()),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(read(&path)["tui"], "default");
}

#[tokio::test]
async fn spawn_seeds_the_persona_config() {
    let fx = fixture().await;
    let req = SpawnRequest {
        actor: Actor("local".into()),
        repos: vec!["core".into()],
        prompt: "x".into(),
        task_slug: None,
        persona: PERSONA.into(),
        kind: fleet_api::AgentKind::Solo,
        role: None,
        roster: vec![],
    };
    fx.svc.spawn(req).await.unwrap();
    let f = fx
        ._tmp
        .path()
        .join("personas")
        .join(PERSONA)
        .join("claude/.claude.json");
    assert_eq!(read(&f)["hasCompletedOnboarding"], true);
}
