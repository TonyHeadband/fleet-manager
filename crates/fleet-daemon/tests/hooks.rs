use fleet_api::{Agent, AgentId, AgentSettings, BlockedReason, Status, Workspace};
use fleet_daemon::fleet::defs::ToolRules;
use fleet_daemon::hooks_cfg::{
    write_settings, RoleSettings, SettingsError, ORCHESTRATOR_TOOLS, SPECIALIST_TOOLS,
};
use fleet_daemon::ingest::{ingest, IngestError, IngestOutcome};
use fleet_daemon::repo_rules::RepoRules;
use fleet_daemon::store::Store;
use serde_json::{json, Value};
use std::path::PathBuf;

fn agent(id: &str) -> Agent {
    Agent {
        id: AgentId(id.into()),
        goal: "t".into(),
        title: None,
        workdir: "/w".into(),
        workspaces: vec![Workspace {
            repo: "core".into(),
            branch: "fleet/t".into(),
            worktree: "/w/core".into(),
            base: "abc".into(),
            from_branch: None,
            has_changes: false,
            owned: false,
            changed: false,
        }],
        session: "fleet-x".into(),
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
        persona: "p".into(),
        product: "p".into(),
        kind: fleet_api::AgentKind::Solo,
        task: None,
        role: None,
        step: None,
        model: None,
        attach: None,
        mode: None,
        effort: None,
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
        &Default::default(),
        &Default::default(),
        std::path::Path::new("/w/task"),
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
        &Default::default(),
        &Default::default(),
        std::path::Path::new("/w/task"),
        &Default::default(),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        v["permissions"],
        json!({
            "allow": ["mcp__fleet__list_repos", "mcp__fleet__request_repo"],
            "deny": ["Read(~/.claude/**)"],
        })
    );
    assert!(
        v["hooks"].as_object().is_some_and(|h| !h.is_empty()),
        "permissions must not replace the hooks"
    );
}

#[test]
fn settings_merge_keeps_fleet_allow() {
    let dir = tempfile::tempdir().unwrap();
    let repo_rules = RepoRules {
        deny: vec!["Bash(rm:*)".into()],
        ..Default::default()
    };
    let fleet = AgentSettings {
        allow: vec!["Read".into()],
        deny: vec!["WebFetch".into()],
        env: [("EDITOR".to_string(), "vim".to_string())].into(),
    };
    let path = write_settings(
        dir.path(),
        "http://127.0.0.1:7878",
        &AgentId("a1".into()),
        &repo_rules,
        &fleet,
        &Default::default(),
        std::path::Path::new("/w/task"),
        &Default::default(),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        v["permissions"],
        json!({
            "allow": ["mcp__fleet__list_repos", "mcp__fleet__request_repo", "Read"],
            "deny": ["Bash(rm:*)", "WebFetch", "Read(~/.claude/**)"],
        })
    );
    assert_eq!(v["env"], json!({"EDITOR": "vim"}));
    assert!(v["hooks"].as_object().is_some_and(|h| !h.is_empty()));
}

/// The deny set every orchestrator and fleet agent gets, as found in the fleet spike.
const SPIKE_DENY: [&str; 11] = [
    "Agent",
    "Task",
    "Bash(git push:*)",
    "Bash(git * push *)",
    "Bash(git * push)",
    "Bash(gh pr create:*)",
    "Bash(gh * pr create *)",
    "Bash(gh * pr create)",
    "Bash(gh api:*)",
    "Bash(gh * api *)",
    "Bash(gh * api)",
];

#[test]
fn orchestrator_settings_deny_subagents_and_push() {
    assert_eq!(
        ORCHESTRATOR_TOOLS,
        [
            "mcp__fleet__propose_breakdown",
            "mcp__fleet__spawn_specialist",
            "mcp__fleet__wait_for_fleet",
            "mcp__fleet__fleet_status",
            "mcp__fleet__open_pr",
        ]
    );
    assert_eq!(SPECIALIST_TOOLS, ["mcp__fleet__finish_step"]);
    let dir = tempfile::tempdir().unwrap();
    // A definition that tries to allow a push still gets the fixed denies, and adds its own.
    let rules = ToolRules {
        allow: vec!["Bash(git push:*)".into()],
        deny: vec!["WebFetch".into()],
    };
    let path = write_settings(
        dir.path(),
        "http://127.0.0.1:7878",
        &AgentId("a1".into()),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        std::path::Path::new("/w/task"),
        &RoleSettings::orchestrator(&rules),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let deny: Vec<&str> = v["permissions"]["deny"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    let mut want = SPIKE_DENY.to_vec();
    want.push("WebFetch");
    want.push("Read(~/.claude/**)");
    assert_eq!(deny, want);
    for rule in ["Bash(bash -c:*)", "Bash(sh -c:*)", "Bash(git * push:*)"] {
        assert!(!deny.contains(&rule), "{rule}");
    }
    let allow = v["permissions"]["allow"].as_array().unwrap();
    for tool in ORCHESTRATOR_TOOLS {
        assert!(allow.contains(&json!(tool)), "{tool} not in {allow:?}");
    }
    assert!(!allow.contains(&json!(SPECIALIST_TOOLS[0])));

    let role = RoleSettings::specialist(&ToolRules::default());
    assert_eq!(role.deny, SPIKE_DENY);
    assert_eq!(role.allow, SPECIALIST_TOOLS);
    assert_eq!(RoleSettings::default().deny, Vec::<String>::new());
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
            &Default::default(),
            &Default::default(),
            std::path::Path::new("/w/task"),
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
        let err = write_settings(
            dir.path(),
            bad,
            &AgentId("a1".into()),
            &Default::default(),
            &Default::default(),
            &Default::default(),
            std::path::Path::new("/w/task"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(matches!(err, SettingsError::BadBaseUrl(_)), "{bad:?}");
    }
    assert!(write_settings(
        dir.path(),
        "http://[::1]:7878",
        &AgentId("a1".into()),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        std::path::Path::new("/w/task"),
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

fn env_of(fleet: &[(&str, &str)], repo: &[(&str, &str)]) -> Value {
    let map = |kv: &[(&str, &str)]| -> std::collections::BTreeMap<String, String> {
        kv.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let dir = tempfile::tempdir().unwrap();
    let fleet = AgentSettings {
        env: map(fleet),
        ..Default::default()
    };
    let path = write_settings(
        dir.path(),
        "http://127.0.0.1:7878",
        &AgentId("a1".into()),
        &Default::default(),
        &fleet,
        &map(repo),
        std::path::Path::new("/w/task"),
        &Default::default(),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["env"].clone()
}

#[test]
fn repo_env_wins_over_the_fleet_env() {
    assert_eq!(
        env_of(&[("A", "1"), ("B", "2")], &[("B", "3")]),
        json!({"A": "1", "B": "3"})
    );
}

#[test]
fn fleet_env_alone_is_unchanged_without_repo_env() {
    assert_eq!(
        env_of(&[("A", "1"), ("B", "2")], &[]),
        json!({"A": "1", "B": "2"})
    );
    assert!(env_of(&[], &[]).is_null());
}

#[test]
fn settings_exclude_every_folder_above_the_task_and_deny_the_owners_claude_dir() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_settings(
        dir.path(),
        "http://127.0.0.1:7878",
        &AgentId("a1".into()),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        std::path::Path::new("/home/u/fleet-tasks/t1"),
        &Default::default(),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let ex: Vec<&str> = v["claudeMdExcludes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    for dir in ["/home/u/fleet-tasks", "/home/u", "/home", ""] {
        for leaf in [
            "CLAUDE.md",
            "CLAUDE.local.md",
            ".claude/CLAUDE.md",
            ".claude/rules/**",
        ] {
            let rule = format!("{dir}/{leaf}");
            assert!(ex.contains(&rule.as_str()), "{rule} missing in {ex:?}");
        }
    }
    assert!(
        ex.iter().all(|e| !e.starts_with("/home/u/fleet-tasks/t1")),
        "{ex:?}"
    );
    assert!(v["permissions"]["deny"]
        .as_array()
        .unwrap()
        .contains(&json!("Read(~/.claude/**)")));
}
