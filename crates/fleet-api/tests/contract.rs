use fleet_api::{
    default_ui_schema, eval, openapi, Actor, Agent, AgentId, BlockedReason, Condition, DiffReport,
    Event, EventKind, Project, RepoInfo, RepoList, RepoSource, SpawnRequest, SpawnResponse, Status,
    UiSchema, Workspace, WorkspaceDiff,
};
use serde_json::json;
use std::path::PathBuf;
use ts_rs::TS;

fn agent(status: Status, paused: bool) -> Agent {
    Agent {
        id: AgentId("a1".into()),
        task: "fix tests".into(),
        workdir: "/w/fix-tests".into(),
        workspaces: vec![workspace("agent-core", false)],
        tmux_session: "fleet-a1".into(),
        status,
        activity: "running tests".into(),
        status_since: 1_700_000_000,
        has_changes: false,
        paused,
        owner: "antonio".into(),
        blocked_reason: None,
        repo_request: None,
        stop_cause: None,
        disk_bytes: None,
    }
}

fn workspace(repo: &str, has_changes: bool) -> Workspace {
    Workspace {
        repo: repo.into(),
        branch: "fleet/fix-tests".into(),
        worktree: format!("/w/fix-tests/{repo}"),
        base: "abc123".into(),
        from_branch: None,
        has_changes,
        owned: false,
    }
}

fn visible(schema: &UiSchema, a: &Agent) -> Vec<String> {
    schema
        .actions
        .iter()
        .filter(|act| eval(&act.when, a))
        .map(|act| act.id.clone())
        .collect()
}

#[test]
fn status_serializes_snake_case() {
    assert_eq!(
        serde_json::to_value(Status::NeedsYou).unwrap(),
        json!("needs_you")
    );
}

#[test]
fn event_round_trips_json() {
    let ev = Event {
        seq: 7,
        kind: EventKind::StatusChanged {
            id: AgentId("a1".into()),
            status: Status::Review,
            activity: "done".into(),
            since: 10,
            has_changes: true,
            paused: false,
            blocked_reason: Some(BlockedReason::Question),
            repo_request: Some(fleet_api::RepoRequest {
                repo: "tools".into(),
                why: "w".into(),
            }),
            stop_cause: None,
            disk_bytes: None,
        },
    };
    let text = serde_json::to_string(&ev).unwrap();
    assert!(text.contains("\"type\":\"status_changed\""), "{text}");
    assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), ev);
}

#[test]
fn openapi_lists_all_routes() {
    let doc = serde_json::to_value(openapi()).unwrap();
    let paths = doc["paths"].as_object().expect("paths object");
    for p in [
        "/api/agents",
        "/api/agents/{id}/stop",
        "/api/agents/{id}/answer",
        "/api/agents/{id}/pause",
        "/api/agents/{id}/resume",
        "/api/agents/{id}/terminal",
        "/api/agents/{id}/diff",
        "/api/events",
        "/api/repos",
        "/api/ui-schema",
    ] {
        assert!(
            paths.contains_key(p),
            "missing {p}; have {:?}",
            paths.keys()
        );
    }
    assert!(
        !paths.contains_key("/api/projects"),
        "the old route must be gone"
    );
}

#[test]
fn default_ui_schema_covers_every_status_in_exactly_one_column() {
    let schema = default_ui_schema();
    for s in Status::ALL {
        let n = schema
            .columns
            .iter()
            .filter(|c| c.statuses.contains(&s))
            .count();
        assert_eq!(n, 1, "{s:?} appears in {n} columns");
    }
    let titles: Vec<&str> = schema.columns.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Queued", "Working", "Needs you", "Talking", "Review"]
    );
}

#[test]
fn eval_conditions() {
    let working = agent(Status::Working, false);
    assert!(eval(&Condition::Always, &working));
    assert!(eval(
        &Condition::StatusIn {
            statuses: vec![Status::Working]
        },
        &working
    ));
    assert!(!eval(
        &Condition::StatusIn {
            statuses: vec![Status::Review]
        },
        &working
    ));
    assert!(eval(
        &Condition::All {
            conditions: vec![
                Condition::Paused { value: false },
                Condition::StatusIn {
                    statuses: vec![Status::Working]
                }
            ]
        },
        &working
    ));
    assert!(!eval(&Condition::Paused { value: true }, &working));

    let schema = default_ui_schema();
    let needs = visible(&schema, &agent(Status::NeedsYou, false));
    assert!(needs.contains(&"answer".to_string()) && needs.contains(&"stop".to_string()));
    assert!(!visible(&schema, &working).contains(&"answer".to_string()));
    let paused = visible(&schema, &agent(Status::Review, true));
    assert!(paused.contains(&"resume".to_string()) && !paused.contains(&"pause".to_string()));
}

#[test]
fn condition_wire_shape_is_internally_tagged() {
    let c = Condition::All {
        conditions: vec![Condition::Paused { value: false }, Condition::Always],
    };
    assert_eq!(
        serde_json::to_value(&c).unwrap(),
        json!({"type": "all", "conditions": [{"type": "paused", "value": false}, {"type": "always"}]})
    );
}

#[test]
fn blocked_reason_is_snake_case_and_serialised_as_null_when_absent() {
    assert_eq!(
        serde_json::to_value(BlockedReason::Permission).unwrap(),
        json!("permission")
    );
    for (r, s) in [
        (BlockedReason::Question, "question"),
        (BlockedReason::Error, "error"),
        (BlockedReason::Startup, "startup"),
    ] {
        assert_eq!(serde_json::to_value(r).unwrap(), json!(s));
    }
    let a = serde_json::to_value(agent(Status::Working, false)).unwrap();
    assert_eq!(a["blocked_reason"], json!(null));
    assert_eq!(a["owner"], json!("antonio"));
}

#[test]
fn diff_report_round_trips() {
    let r = DiffReport {
        workspaces: vec![WorkspaceDiff {
            repo: "core".into(),
            diff: "+hi\n".into(),
            truncated: false,
        }],
    };
    assert_eq!(
        serde_json::to_value(&r).unwrap(),
        json!({"workspaces": [{"repo": "core", "diff": "+hi\n", "truncated": false}]})
    );
}

#[test]
fn every_card_field_is_an_agent_field_or_a_known_derived_one() {
    // A field naming something that is not on Agent renders nothing (it happened to `branch` in A1).
    let keys: Vec<String> = serde_json::to_value(agent(Status::Working, false))
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    for f in default_ui_schema().card_fields {
        let derived = ["repos", "branch"].contains(&f.key.as_str());
        assert!(
            derived || keys.contains(&f.key),
            "card field {:?} is not an Agent field",
            f.key
        );
    }
}

#[test]
fn pause_is_not_offered_on_needs_you_cards_because_escape_would_deny_the_request() {
    let schema = default_ui_schema();
    assert!(!visible(&schema, &agent(Status::NeedsYou, false)).contains(&"pause".to_string()));
    for s in [Status::Working, Status::Review] {
        assert!(
            visible(&schema, &agent(s, false)).contains(&"pause".to_string()),
            "{s:?}"
        );
    }
}

#[test]
fn resume_is_not_offered_on_dead_cards() {
    let schema = default_ui_schema();
    assert!(!visible(&schema, &agent(Status::Stopped, true)).contains(&"resume".to_string()));
    assert!(visible(&schema, &agent(Status::Working, true)).contains(&"resume".to_string()));
}

#[test]
fn ui_schema_shows_a_changes_badge_only_when_the_agent_has_changes() {
    let schema = default_ui_schema();
    let f = schema
        .card_fields
        .iter()
        .find(|f| f.key == "has_changes")
        .expect("has_changes field");
    let mut a = agent(Status::Review, false);
    assert!(!eval(&f.show_when, &a));
    a.has_changes = true;
    assert!(eval(&f.show_when, &a));
}

#[test]
fn ui_schema_shows_the_blocked_reason_only_on_needs_you_cards() {
    let schema = default_ui_schema();
    let f = schema
        .card_fields
        .iter()
        .find(|f| f.key == "blocked_reason")
        .expect("blocked_reason field");
    assert!(eval(&f.show_when, &agent(Status::NeedsYou, false)));
    assert!(!eval(&f.show_when, &agent(Status::Working, false)));
}

#[test]
fn repo_list_round_trips() {
    let p = RepoList {
        repos: vec![
            RepoInfo {
                name: "core".into(),
                source: RepoSource::Path,
                location: "/r/core".into(),
            },
            RepoInfo {
                name: "portal".into(),
                source: RepoSource::Url,
                location: "git@h:x/portal.git".into(),
            },
        ],
    };
    assert_eq!(
        serde_json::to_value(&p).unwrap(),
        json!({"repos": [
            {"name": "core", "source": "path", "location": "/r/core"},
            {"name": "portal", "source": "url", "location": "git@h:x/portal.git"},
        ]})
    );
}

#[test]
fn agent_round_trips_with_workspaces() {
    let mut a = agent(Status::Working, false);
    a.workspaces.push(workspace("portal", true));
    let v = serde_json::to_value(&a).unwrap();
    assert_eq!(v["workspaces"][0]["repo"], "agent-core");
    assert_eq!(v["workspaces"][1]["has_changes"], true);
    assert_eq!(v["workdir"], "/w/fix-tests");
    for gone in ["project", "branch", "worktree"] {
        assert!(v.get(gone).is_none(), "Agent still has `{gone}`");
    }
    assert_eq!(serde_json::from_value::<Agent>(v).unwrap(), a);
}

#[test]
fn workspaces_changed_event_round_trips() {
    let ev = Event {
        seq: 9,
        kind: EventKind::WorkspacesChanged {
            id: AgentId("a1".into()),
            workspaces: vec![workspace("agent-core", true)],
        },
    };
    let text = serde_json::to_string(&ev).unwrap();
    assert!(text.contains("\"type\":\"workspaces_changed\""), "{text}");
    assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), ev);
}

#[test]
fn spawn_request_uses_repos() {
    let r = SpawnRequest {
        actor: Actor("local".into()),
        repos: vec!["core".into()],
        prompt: "x".into(),
        task_slug: None,
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["repos"], json!(["core"]));
    assert!(v.get("project").is_none());
}

#[test]
fn default_ui_schema_has_repos_field_not_project() {
    let schema = default_ui_schema();
    assert!(schema.card_fields.iter().any(|f| f.key == "repos"));
    assert!(!schema.card_fields.iter().any(|f| f.key == "project"));
}

#[test]
fn v0_1_agent_json_without_new_fields_still_parses() {
    let v = json!({
        "id": "a1", "task": "t", "workdir": "/w",
        "workspaces": [{"repo": "core", "branch": "b", "worktree": "/w/core", "base": "x", "has_changes": false}],
        "tmux_session": "fleet-a1", "status": "working", "activity": "", "status_since": 1,
        "has_changes": false, "paused": false, "owner": "me", "blocked_reason": null
    });
    let a: Agent = serde_json::from_value(v).unwrap();
    assert!(!a.workspaces[0].owned);
    assert_eq!(a.repo_request, None);
}

fn blocked(reason: BlockedReason) -> Agent {
    let mut a = agent(Status::NeedsYou, false);
    a.blocked_reason = Some(reason);
    a
}

#[test]
fn answer_is_hidden_for_a_repo_access_request() {
    let a = blocked(BlockedReason::RepoAccess);
    assert!(!visible(&default_ui_schema(), &a).contains(&"answer".to_string()));
}

#[test]
fn answer_shows_for_a_permission_request() {
    let a = blocked(BlockedReason::Permission);
    assert!(visible(&default_ui_schema(), &a).contains(&"answer".to_string()));
}

#[test]
fn blocked_reason_labels_cover_every_reason() {
    let schema = default_ui_schema();
    let def = schema
        .card_fields
        .iter()
        .find(|f| f.key == "blocked_reason")
        .unwrap();
    for r in [
        BlockedReason::Permission,
        BlockedReason::Question,
        BlockedReason::Error,
        BlockedReason::Startup,
        BlockedReason::RepoAccess,
    ] {
        // Exhaustive match: adding a variant breaks this until the list above and the labels follow.
        match r {
            BlockedReason::Permission
            | BlockedReason::Question
            | BlockedReason::Error
            | BlockedReason::Startup
            | BlockedReason::RepoAccess => {}
        }
        let name = serde_json::to_value(r).unwrap();
        let name = name.as_str().unwrap();
        assert!(def.value_labels.contains_key(name), "no label for {name}");
    }
    assert_eq!(def.value_labels["permission"], "Permission needed");
}

#[test]
fn spawn_request_without_repos_is_valid() {
    let r: SpawnRequest =
        serde_json::from_value(json!({"actor": "local", "prompt": "x", "task_slug": null}))
            .unwrap();
    assert!(r.repos.is_empty());
}

#[test]
fn actor_is_a_plain_string_on_the_wire() {
    assert_eq!(
        serde_json::to_value(Actor("local".into())).unwrap(),
        json!("local")
    );
}

#[test]
fn ts_bindings_export() {
    let cfg = ts_rs::Config::from_env();
    Agent::export_all(&cfg).unwrap();
    UiSchema::export_all(&cfg).unwrap();
    Event::export_all(&cfg).unwrap();
    RepoList::export_all(&cfg).unwrap();
    DiffReport::export_all(&cfg).unwrap();
    Workspace::export_all(&cfg).unwrap();
    Project::export_all(&cfg).unwrap();
    SpawnResponse::export_all(&cfg).unwrap();
    // The web tests read this file, so the UI is always tested against the daemon's real default schema.
    let json = serde_json::to_string_pretty(&default_ui_schema()).unwrap();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/api/generated/default-ui-schema.json");
    std::fs::write(&out, json + "\n").unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/src/api/generated");
    for f in [
        "Agent.ts",
        "Status.ts",
        "Event.ts",
        "UiSchema.ts",
        "RepoList.ts",
        "Workspace.ts",
        "DiffReport.ts",
        "SpawnResponse.ts",
        "default-ui-schema.json",
    ] {
        assert!(dir.join(f).exists(), "missing {f}");
    }
}

#[test]
fn dead_still_parses_as_stopped() {
    assert_eq!(
        serde_json::from_str::<Status>("\"dead\"").unwrap(),
        Status::Stopped
    );
    assert_eq!(
        serde_json::to_string(&Status::Stopped).unwrap(),
        "\"stopped\""
    );
}
