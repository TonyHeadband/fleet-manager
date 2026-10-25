use fleet_api::{Agent, AgentId, BlockedReason, Status, Workspace};
use fleet_daemon::hooks_cfg::{write_settings, SettingsError};
use fleet_daemon::ingest::{ingest, IngestError, IngestOutcome};
use fleet_daemon::store::Store;
use serde_json::{json, Value};
use std::path::PathBuf;

fn agent(id: &str) -> Agent {
    Agent {
        id: AgentId(id.into()),
        task: "t".into(),
        workdir: "/w".into(),
        workspaces: vec![Workspace {
            repo: "core".into(),
            branch: "fleet/t".into(),
            worktree: "/w/core".into(),
            base: "abc".into(),
            from_branch: None,
            has_changes: false,
            owned: false,
        }],
        tmux_session: "fleet-x".into(),
        status: Status::Queued,
        activity: "".into(),
        status_since: 1,
        has_changes: false,
        paused: false,
        owner: "antonio".into(),
        blocked_reason: None,
        repo_request: None,
        stop_cause: None,
        disk_bytes: None,
    }
}

async fn store_with(id: &str, token: &str) -> Store {
    let s = Store::open(":memory:".as_ref()).await.unwrap();
    s.insert_agent(&agent(id), token).await.unwrap();
    s
}

fn fixture(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hooks")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

async fn status_of(s: &Store, id: &str) -> Agent {
    s.snapshot()
        .await
        .unwrap()
        .agents
        .into_iter()
        .find(|a| a.id.0 == id)
        .unwrap()
}

#[test]
fn settings_file_has_a_hook_for_each_mapped_event() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_settings(
        dir.path(),
        "http://127.0.0.1:7878",
        &AgentId("a1-b2".into()),
        &Default::default(),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let hooks = v["hooks"].as_object().unwrap();
    let mut names: Vec<&str> = hooks.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Notification",
            "PermissionDenied",
            "PermissionRequest",
            "PostToolUse",
            "PostToolUseFailure",
            "PreToolUse",
            "SessionEnd",
            "SessionStart",
            "Stop",
            "StopFailure",
            "SubagentStart",
            "UserPromptSubmit"
        ]
    );
    let url = "http://127.0.0.1:7878/internal/hooks/a1-b2";
    for (name, groups) in hooks {
        let h = &groups[0]["hooks"][0];
        if name == "SessionStart" || name == "SessionEnd" {
            assert_eq!(h["type"], "command", "{name}");
            let cmd = h["command"].as_str().unwrap();
            assert!(
                cmd.contains(url) && cmd.contains("X-Fleet-Token: $FLEET_TOKEN"),
                "{cmd}"
            );
        } else {
            assert_eq!(h["type"], "http", "{name}");
            assert_eq!(h["url"], url);
            assert_eq!(h["headers"]["X-Fleet-Token"], "$FLEET_TOKEN");
            assert_eq!(h["allowedEnvVars"], json!(["FLEET_TOKEN"]));
        }
    }
}

#[test]
fn the_settings_pre_allow_only_the_fleet_tools() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_settings(
        dir.path(),
        "http://127.0.0.1:7878",
        &AgentId("a1".into()),
        &Default::default(),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        v["permissions"],
        json!({"allow": ["mcp__fleet__list_repos", "mcp__fleet__request_repo"]})
    );
    assert!(
        v["hooks"].as_object().is_some_and(|h| !h.is_empty()),
        "permissions must not replace the hooks"
    );
}

#[test]
fn settings_rejects_ids_and_urls_that_could_break_out_of_the_curl_command() {
    let dir = tempfile::tempdir().unwrap();
    for bad in ["a;b", "a b", "a'b", "$(x)", "a/b", "", "a\nb"] {
        let err = write_settings(
            dir.path(),
            "http://127.0.0.1:7878",
            &AgentId(bad.into()),
            &Default::default(),
        )
        .unwrap_err();
        assert!(matches!(err, SettingsError::BadAgentId(_)), "{bad:?}");
    }
    for bad in [
        "http://evil.example:80",
        "http://127.0.0.1:7878/x;y",
        "https://127.0.0.1:7878",
        "http://127.0.0.1",
    ] {
        let err = write_settings(dir.path(), bad, &AgentId("a1".into()), &Default::default())
            .unwrap_err();
        assert!(matches!(err, SettingsError::BadBaseUrl(_)), "{bad:?}");
    }
    assert!(write_settings(
        dir.path(),
        "http://[::1]:7878",
        &AgentId("a1".into()),
        &Default::default()
    )
    .is_ok());
}

#[tokio::test]
async fn ingest_applies_an_event_and_updates_the_card() {
    let s = store_with("a1", "tok").await;
    let out = ingest(
        &s,
        &AgentId("a1".into()),
        Some("tok"),
        fixture("SessionStart-1.json"),
        100,
    )
    .await
    .unwrap();
    assert!(matches!(out, IngestOutcome::Applied(_)));
    let a = status_of(&s, "a1").await;
    assert_eq!((a.status, a.status_since), (Status::Working, 100));
}

#[tokio::test]
async fn wrong_missing_or_empty_token_is_rejected_and_state_unchanged() {
    let s = store_with("a1", "tok").await;
    let before = s.snapshot().await.unwrap();
    for t in [Some("nope"), None, Some("")] {
        let err = ingest(
            &s,
            &AgentId("a1".into()),
            t,
            fixture("SessionStart-1.json"),
            5,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, IngestError::BadToken), "{t:?}");
    }
    assert_eq!(s.snapshot().await.unwrap(), before);
}

#[tokio::test]
async fn unknown_agent_is_rejected() {
    let s = store_with("a1", "tok").await;
    let err = ingest(
        &s,
        &AgentId("zz".into()),
        Some("tok"),
        fixture("Stop-1.json"),
        5,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, IngestError::UnknownAgent));
}

#[tokio::test]
async fn malformed_body_is_rejected() {
    let s = store_with("a1", "tok").await;
    let err = ingest(&s, &AgentId("a1".into()), Some("tok"), json!({"x": 1}), 5)
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::BadBody(_)));
}

#[tokio::test]
async fn replaying_the_recorded_session_ends_in_review_with_since_tracking() {
    let s = store_with("a1", "tok").await;
    let id = AgentId("a1".into());
    let steps = [
        ("SessionStart-1.json", 10, Status::Working, 10),
        ("UserPromptSubmit-1.json", 11, Status::Working, 10),
        ("PreToolUse-1.json", 12, Status::Working, 10),
        ("PermissionRequest-1.json", 20, Status::NeedsYou, 20),
        ("Notification-1.json", 26, Status::NeedsYou, 20),
        ("PostToolUse-1.json", 30, Status::Working, 30),
        ("Stop-1.json", 31, Status::Review, 31),
    ];
    for (file, now, want, since) in steps {
        ingest(&s, &id, Some("tok"), fixture(file), now)
            .await
            .unwrap();
        let a = status_of(&s, "a1").await;
        assert_eq!((a.status, a.status_since), (want, since), "after {file}");
    }
}

#[tokio::test]
async fn blocked_reason_is_set_by_a_permission_request_and_cleared_by_the_next_tool_event() {
    let s = store_with("a1", "tok").await;
    let id = AgentId("a1".into());
    ingest(&s, &id, Some("tok"), fixture("PermissionRequest-1.json"), 5)
        .await
        .unwrap();
    let a = status_of(&s, "a1").await;
    assert_eq!(
        (a.status, a.blocked_reason),
        (Status::NeedsYou, Some(BlockedReason::Permission))
    );
    ingest(&s, &id, Some("tok"), fixture("PostToolUse-1.json"), 6)
        .await
        .unwrap();
    let a = status_of(&s, "a1").await;
    assert_eq!((a.status, a.blocked_reason), (Status::Working, None));
}

#[tokio::test]
async fn duplicate_event_is_ignored_and_emits_nothing() {
    let s = store_with("a1", "tok").await;
    let id = AgentId("a1".into());
    ingest(&s, &id, Some("tok"), fixture("PreToolUse-1.json"), 5)
        .await
        .unwrap();
    let seq = s.snapshot().await.unwrap().seq;
    let again = ingest(&s, &id, Some("tok"), fixture("PreToolUse-1.json"), 6)
        .await
        .unwrap();
    assert!(matches!(again, IngestOutcome::Ignored));
    assert_eq!(s.snapshot().await.unwrap().seq, seq);
}

#[tokio::test]
async fn events_after_session_end_never_revive_the_agent() {
    let s = store_with("a1", "tok").await;
    let id = AgentId("a1".into());
    ingest(&s, &id, Some("tok"), fixture("SessionEnd-3.json"), 5)
        .await
        .unwrap();
    assert_eq!(status_of(&s, "a1").await.status, Status::Stopped);
    for f in [
        "SessionStart-1.json",
        "UserPromptSubmit-1.json",
        "Stop-1.json",
    ] {
        let out = ingest(&s, &id, Some("tok"), fixture(f), 9).await.unwrap();
        assert!(matches!(out, IngestOutcome::Ignored), "{f}");
    }
    assert_eq!(status_of(&s, "a1").await.status, Status::Stopped);
}

#[tokio::test]
async fn known_agent_with_a_different_token_is_bad_token() {
    let s = store_with("a1", "tok").await;
    assert!(matches!(
        ingest(
            &s,
            &AgentId("a1".into()),
            Some("tok2"),
            fixture("Stop-1.json"),
            1
        )
        .await,
        Err(IngestError::BadToken)
    ));
}
