use fleet_api::{
    default_ui_schema, eval, openapi, rollup, task_id, Actor, Agent, AgentConfig,
    AgentConfigUpdate, AgentId, AgentKind, Approval, ArchiveOrphanRequest, ArchiveRequest,
    BlockedReason, Condition, DecideBreakdown, DefFile, DefInfo, DefKind, DiffReport, EnvLine,
    EnvSource, Event, EventKind, FieldKind, FleetEvent, FleetEventKind, FleetView, NewPersona,
    PersonaList, PersonaUpdate, PlanItem, PrRules, Product, ProductList, Release, RepoInfo,
    RepoList, RepoPick, SpawnRequest, SpawnResponse, Status, StepDraft, StopCause,
    StopTaskResponse, Task, Tone, ToolError, ToolFix, ToolId, ToolList, UiSchema, Versioning,
    WorkflowState, Workspace, WorkspaceDiff,
};
use serde_json::json;
use std::path::PathBuf;
use ts_rs::TS;

fn agent(status: Status, paused: bool) -> Agent {
    Agent {
        id: AgentId("a1".into()),
        goal: "fix tests".into(),
        title: None,
        workdir: "/w/fix-tests".into(),
        workspaces: vec![workspace("agent-core", false)],
        session: "fleet-a1".into(),
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
        persona: "p".into(),
        product: "app".into(),
        kind: AgentKind::Solo,
        task: None,
        role: None,
        step: None,
        model: None,
        mode: None,
        effort: None,
        attach: None,
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
        changed: false,
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
fn pr_style_def_kind_serializes_snake_case() {
    assert_eq!(
        serde_json::to_value(DefKind::PrStyle).unwrap(),
        json!("pr_style")
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
        "/api/personas",
        "/api/personas/{id}",
        "/api/personas/{id}/check",
        "/api/personas/{id}/token",
        "/api/agent-config",
        "/api/personas/{id}/definitions",
        "/api/personas/{id}/definitions/{kind}/{name}",
        "/api/personas/{id}/definitions/{kind}/{name}/move",
        "/api/personas/{id}/nicknames/{name}",
        "/api/products",
        "/api/products/{id}",
        "/api/products/{id}/repos",
        "/api/products/{id}/repos/{name}",
        "/api/products/{id}/hidden-specialists",
        "/api/products/{id}/releases",
        "/api/products/{id}/overrides",
        "/api/settings/effective",
        "/api/releases/{id}",
        "/api/tasks/{id}/release",
        "/api/tasks/{id}/done",
        "/api/tools",
        "/api/tools/recheck",
        "/api/version",
    ] {
        assert!(
            paths.contains_key(p),
            "missing {p}; have {:?}",
            paths.keys()
        );
    }
}

#[test]
fn the_openapi_document_lists_the_supervisor_routes() {
    let doc = serde_json::to_value(openapi()).unwrap();
    let paths = doc["paths"].as_object().expect("paths object");
    for (p, method) in [
        ("/api/supervisor/{persona}/conversations", "get"),
        ("/api/supervisor/conversations", "post"),
        ("/api/supervisor/conversations/{id}/message", "post"),
    ] {
        assert!(
            paths.get(p).is_some_and(|o| o.get(method).is_some()),
            "missing {method} {p}; have {:?}",
            paths.keys()
        );
    }
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    for s in [
        "Conversation",
        "TaskMark",
        "NewConversation",
        "ConversationMessage",
    ] {
        assert!(schemas.contains_key(s), "missing schema {s}");
    }
}

#[test]
fn the_openapi_document_lists_the_task_routes() {
    let doc = serde_json::to_value(openapi()).unwrap();
    let paths = doc["paths"].as_object().expect("paths object");
    for (p, method) in [
        ("/api/tasks", "get"),
        ("/api/tasks/refresh", "post"),
        ("/api/tasks/{id}/archive-plan", "get"),
        ("/api/tasks/{id}/archive", "post"),
        ("/api/orphans/plan", "post"),
        ("/api/orphans/archive", "post"),
        ("/api/tasks/{id}/fleet", "get"),
        ("/api/tasks/{id}/breakdown/decide", "post"),
    ] {
        assert!(
            paths.get(p).is_some_and(|o| o.get(method).is_some()),
            "missing {method} {p}; have {:?}",
            paths.keys()
        );
    }
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("TaskList"));
    assert!(schemas.contains_key("OrphanRequest"));
    // Both archives carry the plan the user confirmed.
    for (p, body) in [
        ("/api/tasks/{id}/archive", "ArchiveRequest"),
        ("/api/orphans/archive", "ArchiveOrphanRequest"),
    ] {
        assert!(schemas.contains_key(body), "missing schema {body}");
        let schema = &paths[p]["post"]["requestBody"]["content"]["application/json"]["schema"];
        assert_eq!(
            schema["$ref"],
            format!("#/components/schemas/{body}"),
            "{p}"
        );
    }
}

#[test]
fn an_archive_request_carries_the_confirmed_items() {
    let item = PlanItem {
        path: "/p/x/core".into(),
        bytes: 10,
        uncommitted_files: 2,
        stays: false,
    };
    let v = serde_json::to_value(ArchiveOrphanRequest {
        id: "task_folder:/p/x".into(),
        items: vec![item.clone()],
    })
    .unwrap();
    assert_eq!(
        v,
        json!({"id": "task_folder:/p/x", "items": [{"path": "/p/x/core", "bytes": 10, "uncommitted_files": 2, "stays": false}]})
    );
    let r: ArchiveRequest = serde_json::from_value(json!({"items": [v["items"][0]]})).unwrap();
    assert_eq!(r.items, [item]);
    // A body without the plan is refused, never read as "nothing confirmed".
    assert!(serde_json::from_value::<ArchiveRequest>(json!({})).is_err());
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
        // Nothing produces Talking yet, so it has no column until agent-to-agent messaging brings one back.
        let want = usize::from(s != Status::Talking);
        assert_eq!(n, want, "{s:?} appears in {n} columns");
    }
    let titles: Vec<&str> = schema.columns.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Stopped", "Queued", "Working", "Needs you", "Review"]
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
    assert!(visible(&schema, &working).contains(&"answer".to_string()));
    assert!(!visible(&schema, &agent(Status::Stopped, false)).contains(&"answer".to_string()));
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
fn the_board_starts_with_stopped_and_has_no_talking() {
    let schema = default_ui_schema();
    let ids: Vec<_> = schema.columns.iter().map(|c| c.id.clone()).collect();
    assert_eq!(ids, ["stopped", "queued", "working", "needs_you", "review"]);
    let stopped = &schema.columns[0];
    assert!(stopped.dim);
    assert_eq!(
        (
            stopped.statuses.as_slice(),
            stopped.empty_hint.as_str(),
            stopped.tone
        ),
        (&[Status::Stopped][..], "Nothing stopped", Tone::Muted)
    );
    assert!(schema.columns[1..].iter().all(|c| !c.dim));
    assert_eq!(schema.columns[4].statuses, [Status::Review]);
}

#[test]
fn stop_replaces_kill_and_hides_on_stopped_cards() {
    let schema = default_ui_schema();
    assert!(schema.actions.iter().all(|a| a.id != "kill"));
    let stop = schema
        .actions
        .iter()
        .find(|a| a.id == "stop")
        .expect("stop action");
    assert_eq!(
        (
            stop.label.as_str(),
            stop.method.as_str(),
            stop.path.as_str(),
            stop.destructive
        ),
        ("Stop", "POST", "/api/agents/{id}/stop", true)
    );
    assert!(!eval(&stop.when, &agent(Status::Stopped, false)));
    for s in [
        Status::Queued,
        Status::Working,
        Status::NeedsYou,
        Status::Review,
    ] {
        assert!(eval(&stop.when, &agent(s, false)), "{s:?}");
    }
}

#[test]
fn resume_is_not_offered_on_talking() {
    assert!(
        !visible(&default_ui_schema(), &agent(Status::Talking, true))
            .contains(&"resume".to_string())
    );
}

#[test]
fn a_stopped_card_shows_its_cause_and_size_and_no_ended_badge() {
    let schema = default_ui_schema();
    let field = |key: &str| schema.card_fields.iter().find(|f| f.key == key);
    assert!(
        schema.card_fields.iter().all(|f| f.label != "Ended"),
        "the old Ended badge is gone"
    );
    let cause = field("stop_cause").expect("stop_cause field");
    assert_eq!(cause.kind, FieldKind::Badge);
    for (raw, label, tone) in [
        ("stopped", "Stopped", Tone::Caution),
        ("exited", "Exited", Tone::Active),
        ("crashed", "Crashed", Tone::Danger),
        ("interrupted", "Interrupted", Tone::Caution),
    ] {
        assert_eq!(cause.value_labels[raw], label);
        assert_eq!(cause.value_tones[raw], tone);
    }
    let size = field("disk_bytes").expect("disk_bytes field");
    assert_eq!(size.kind, FieldKind::Bytes);
    let mut stopped = agent(Status::Stopped, false);
    stopped.stop_cause = Some(StopCause::Crashed);
    for f in [cause, size] {
        assert!(eval(&f.show_when, &stopped), "{}", f.key);
        assert!(
            !eval(&f.show_when, &agent(Status::Working, false)),
            "{}",
            f.key
        );
    }
}

#[test]
fn ui_schema_without_the_new_fields_still_parses() {
    // A daemon from before the Stopped column sends neither `dim` nor `value_tones`.
    let col: fleet_api::ColumnDef = serde_json::from_value(json!({
        "id": "q", "title": "Q", "statuses": ["queued"], "empty_hint": "", "tone": "neutral"
    }))
    .unwrap();
    assert!(!col.dim);
    let f: fleet_api::FieldDef = serde_json::from_value(json!({
        "key": "paused", "label": "Paused", "kind": "badge", "show_when": {"type": "always"}
    }))
    .unwrap();
    assert!(f.value_tones.is_empty());
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
                product: "app".into(),
                name: "core".into(),
                path: Some("/r/core".into()),
                url: None,
                workflow: WorkflowState::Found {
                    base: "main".into(),
                    steps: vec!["fmt".into()],
                },
                env: vec![EnvLine {
                    name: "CARGO_TARGET_DIR".into(),
                    value: "{task}/target".into(),
                    source: EnvSource::Repo,
                    gate: true,
                    overridden: false,
                }],
            },
            RepoInfo {
                product: "app".into(),
                name: "portal".into(),
                path: Some("/r/portal".into()),
                url: Some("https://h/x/portal.git".into()),
                workflow: WorkflowState::Missing {
                    base: "main".into(),
                },
                env: vec![],
            },
        ],
    };
    let v = json!({"repos": [
        {"product": "app", "name": "core", "path": "/r/core", "url": null,
         "workflow": {"state": "found", "base": "main", "steps": ["fmt"]},
         "env": [{"name": "CARGO_TARGET_DIR", "value": "{task}/target", "source": "repo",
                  "gate": true, "overridden": false}]},
        {"product": "app", "name": "portal", "path": "/r/portal", "url": "https://h/x/portal.git",
         "workflow": {"state": "missing", "base": "main"}, "env": []},
    ]});
    assert_eq!(serde_json::to_value(&p).unwrap(), v);
    assert_eq!(serde_json::from_value::<RepoList>(v).unwrap(), p);
}

#[test]
fn product_list_round_trips() {
    let p = ProductList {
        products: vec![Product {
            id: "app".into(),
            name: "App".into(),
            default_persona: Some("home".into()),
            internal_url: None,
            repos: vec![],
            releases: vec![Release {
                id: 3,
                product: "app".into(),
                version: "v1.0".into(),
                start: "2026-11-01".into(),
                target: "2026-12-15".into(),
                released_at: None,
            }],
            tasks: 2,
            overrides: vec![],
            hidden_specialists: vec!["docs".into()],
            stale_hidden_specialists: vec![],
        }],
    };
    let v = json!({"products": [{
        "id": "app", "name": "App", "default_persona": "home", "internal_url": null,
        "repos": [], "tasks": 2, "overrides": [], "hidden_specialists": ["docs"], "stale_hidden_specialists": [],
        "releases": [{"id": 3, "product": "app", "version": "v1.0", "start": "2026-11-01",
                      "target": "2026-12-15", "released_at": null}]
    }]});
    assert_eq!(serde_json::to_value(&p).unwrap(), v);
    assert_eq!(serde_json::from_value::<ProductList>(v).unwrap(), p);
}

#[test]
fn unreadable_workflow_wire_shape() {
    let w = WorkflowState::Unreadable {
        base: "main".into(),
        error: "no origin".into(),
    };
    let v = json!({"state": "unreadable", "base": "main", "error": "no origin"});
    assert_eq!(serde_json::to_value(&w).unwrap(), v);
    assert_eq!(serde_json::from_value::<WorkflowState>(v).unwrap(), w);
}

#[test]
fn agent_round_trips_with_workspaces() {
    let mut a = agent(Status::Working, false);
    a.workspaces.push(workspace("portal", true));
    let v = serde_json::to_value(&a).unwrap();
    assert_eq!(v["workspaces"][0]["repo"], "agent-core");
    assert_eq!(v["workspaces"][1]["has_changes"], true);
    assert_eq!(v["workdir"], "/w/fix-tests");
    for gone in ["branch", "worktree"] {
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
        plan: None,
        actor: Actor("local".into()),
        product: "app".into(),
        release: Some(3),
        repos: vec![
            RepoPick {
                repo: "core".into(),
                local: None,
            },
            RepoPick {
                repo: "tools".into(),
                local: Some("/r/tools".into()),
            },
        ],
        prompt: "x".into(),
        slug: None,
        title: None,
        persona: String::new(),
        kind: AgentKind::Solo,
        role: None,
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(
        v["repos"],
        json!([{"repo": "core", "local": null}, {"repo": "tools", "local": "/r/tools"}])
    );
    assert_eq!(
        (v["product"].clone(), v["release"].clone()),
        (json!("app"), json!(3))
    );
    assert!(v.get("task").is_none());
    // `local` may be left out.
    let picked: RepoPick = serde_json::from_value(json!({"repo": "core"})).unwrap();
    assert_eq!(picked.local, None);
}

#[test]
fn default_ui_schema_has_repos_field_not_task() {
    let schema = default_ui_schema();
    assert!(schema.card_fields.iter().any(|f| f.key == "repos"));
    assert!(!schema.card_fields.iter().any(|f| f.key == "task"));
}

#[test]
fn v0_1_agent_json_without_new_fields_still_parses() {
    let v = json!({
        "id": "a1", "goal": "t", "workdir": "/w",
        "workspaces": [{"repo": "core", "branch": "b", "worktree": "/w/core", "base": "x", "has_changes": false}],
        "session": "fleet-a1", "status": "working", "activity": "", "status_since": 1,
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
fn answer_is_offered_while_the_session_is_live() {
    let schema = default_ui_schema();
    for status in [Status::Working, Status::NeedsYou, Status::Review] {
        assert!(visible(&schema, &agent(status, false)).contains(&"answer".to_string()));
    }
    assert!(!visible(&schema, &agent(Status::Stopped, false)).contains(&"answer".to_string()));
    let mut working = agent(Status::Working, false);
    working.blocked_reason = Some(BlockedReason::RepoAccess);
    assert!(!visible(&schema, &working).contains(&"answer".to_string()));
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
        BlockedReason::Breakdown,
    ] {
        // Exhaustive match: adding a variant breaks this until the list above and the labels follow.
        match r {
            BlockedReason::Permission
            | BlockedReason::Question
            | BlockedReason::Error
            | BlockedReason::Startup
            | BlockedReason::RepoAccess
            | BlockedReason::Breakdown => {}
        }
        let name = serde_json::to_value(r).unwrap();
        let name = name.as_str().unwrap();
        assert!(def.value_labels.contains_key(name), "no label for {name}");
    }
    assert_eq!(def.value_labels["permission"], "Permission needed");
}

#[test]
fn spawn_request_without_repos_is_valid() {
    let r: SpawnRequest = serde_json::from_value(
        json!({"actor": "local", "product": "app", "prompt": "x", "slug": null}),
    )
    .unwrap();
    assert!(r.repos.is_empty());
    assert_eq!(r.release, None);
}

#[test]
fn spawn_request_needs_a_product_and_picks() {
    // An older client: no product, and repos as bare names. Refused, never started somewhere unintended.
    assert!(
        serde_json::from_value::<SpawnRequest>(json!({"actor": "local", "prompt": "x"})).is_err()
    );
    assert!(serde_json::from_value::<SpawnRequest>(
        json!({"actor": "local", "product": "app", "prompt": "x", "repos": ["core"]})
    )
    .is_err());
}

fn gh_too_old() -> ToolError {
    ToolError::TooOld {
        tool: ToolId::Gh,
        found: "2.30.0".into(),
        needed: "2.40".into(),
        fix: ToolFix {
            install: Some("winget install GitHub.cli".into()),
            link: "https://cli.github.com".into(),
            manager: Some("winget".into()),
        },
    }
}

#[test]
fn tool_error_round_trips_with_its_kind_tag() {
    let err = gh_too_old();
    let v = serde_json::to_value(&err).unwrap();
    assert_eq!(v["kind"], "too-old");
    assert_eq!(v["tool"], "gh");
    assert_eq!(serde_json::from_value::<ToolError>(v).unwrap(), err);
}

#[test]
fn tool_error_messages_name_the_tool() {
    let fix = ToolFix {
        install: None,
        link: "https://claude.com/claude-code".into(),
        manager: None,
    };
    let missing = ToolError::NotInstalled {
        tool: ToolId::Claude,
        fix,
    };
    assert_eq!(missing.to_string(), "Claude Code is not installed");
    assert_eq!(
        gh_too_old().to_string(),
        "GitHub CLI (gh) 2.30.0 is too old: 2.40 or newer is needed"
    );
    let login = ToolError::NotLoggedIn {
        tool: ToolId::Gh,
        persona: Some("work".into()),
    };
    assert_eq!(
        login.to_string(),
        "GitHub CLI (gh) is not logged in for work"
    );
    assert!(login.fix().is_none());
    assert_eq!(gh_too_old().tool(), ToolId::Gh);
}

#[test]
fn missing_required_lists_only_required_tools_with_errors() {
    let status = |tool, required, error| fleet_api::ToolStatus {
        tool,
        required,
        version: None,
        error,
    };
    let list = ToolList {
        tools: vec![
            status(ToolId::Git, true, None),
            status(ToolId::Gh, true, Some(gh_too_old())),
            status(ToolId::Claude, false, Some(gh_too_old())),
        ],
        checked_at: 0,
        path: None,
    };
    let missing: Vec<_> = list.missing_required().map(|s| s.tool).collect();
    assert_eq!(missing, vec![ToolId::Gh]);
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
    Task::export_all(&cfg).unwrap();
    SpawnResponse::export_all(&cfg).unwrap();
    ArchiveRequest::export_all(&cfg).unwrap();
    ArchiveOrphanRequest::export_all(&cfg).unwrap();
    SpawnRequest::export_all(&cfg).unwrap();
    PersonaList::export_all(&cfg).unwrap();
    NewPersona::export_all(&cfg).unwrap();
    PersonaUpdate::export_all(&cfg).unwrap();
    AgentConfig::export_all(&cfg).unwrap();
    AgentConfigUpdate::export_all(&cfg).unwrap();
    FleetView::export_all(&cfg).unwrap();
    FleetEvent::export_all(&cfg).unwrap();
    DefInfo::export_all(&cfg).unwrap();
    DefFile::export_all(&cfg).unwrap();
    fleet_api::MoveDefinition::export_all(&cfg).unwrap();
    DecideBreakdown::export_all(&cfg).unwrap();
    StopTaskResponse::export_all(&cfg).unwrap();
    ProductList::export_all(&cfg).unwrap();
    ToolList::export_all(&cfg).unwrap();
    fleet_api::VersionInfo::export_all(&cfg).unwrap();
    PrRules::export_all(&cfg).unwrap();
    fleet_api::Conversation::export_all(&cfg).unwrap();
    fleet_api::Room::export_all(&cfg).unwrap();
    fleet_api::MemberRef::export_all(&cfg).unwrap();
    fleet_api::Post::export_all(&cfg).unwrap();
    fleet_api::NewConversation::export_all(&cfg).unwrap();
    fleet_api::ConversationMessage::export_all(&cfg).unwrap();
    fleet_api::TaskEvent::export_all(&cfg).unwrap();
    fleet_api::TaskEventKind::export_all(&cfg).unwrap();
    Versioning::export_all(&cfg).unwrap();
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
        "ArchiveRequest.ts",
        "ArchiveOrphanRequest.ts",
        "default-ui-schema.json",
        "Conversation.ts",
        "TaskMark.ts",
        "NewConversation.ts",
        "ConversationMessage.ts",
        "TaskEvent.ts",
        "TaskEventKind.ts",
        "Room.ts",
        "RoomKind.ts",
        "MemberRef.ts",
        "Post.ts",
        "PostKind.ts",
    ] {
        assert!(dir.join(f).exists(), "missing {f}");
    }
}

/// Every tag of the form vocabulary this build knows, sorted. The web's `isKnown` is checked against the file this
/// writes, so a new kind cannot drift between the two UIs.
#[test]
fn form_vocabulary_export() {
    use fleet_api::{ChoiceSource, DefaultValue, FormItem, InputDef, InputKind, Rule};
    // Exhaustive matches: a new variant fails to compile here until it is listed below too.
    let _ = |i: FormItem| match i {
        FormItem::Field(_) | FormItem::Group { .. } | FormItem::Unknown => (),
    };
    let _ = |k: InputKind| match k {
        InputKind::Text
        | InputKind::Date
        | InputKind::Multiline
        | InputKind::Markdown
        | InputKind::Choice
        | InputKind::Unknown => (),
    };
    let _ = |s: ChoiceSource| match s {
        ChoiceSource::ProductRepos | ChoiceSource::Unknown => (),
    };
    let _ = |d: DefaultValue| match d {
        DefaultValue::Today | DefaultValue::Unknown => (),
    };
    let _ = |r: Rule| match r {
        Rule::NotBlank { .. } | Rule::Date | Rule::NotBefore { .. } | Rule::Unknown => (),
    };
    let input = InputDef {
        key: "k".into(),
        label: "K".into(),
        kind: InputKind::Text,
        placeholder: None,
        default: None,
        rules: vec![],
        help: None,
        source: None,
    };
    // A unit variant serializes as its tag; one with fields as `{"type": tag, ..}`.
    fn tags<T: serde::Serialize>(values: impl IntoIterator<Item = T>) -> Vec<String> {
        let mut tags: Vec<String> = values
            .into_iter()
            .map(|v| match serde_json::to_value(v).unwrap() {
                serde_json::Value::String(s) => s,
                v => v["type"].as_str().unwrap().to_string(),
            })
            .collect();
        tags.sort();
        tags
    }
    let vocabulary = json!({
        "items": tags([FormItem::Field(input), FormItem::Group { fields: vec![] }]),
        "kinds": tags([
            InputKind::Text,
            InputKind::Date,
            InputKind::Multiline,
            InputKind::Markdown,
            InputKind::Choice,
        ]),
        "sources": tags([ChoiceSource::ProductRepos]),
        "defaults": tags([DefaultValue::Today]),
        "rules": tags([
            Rule::NotBlank { message: None },
            Rule::Date,
            Rule::NotBefore { field: "k".into(), message: None },
        ]),
    });
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/api/generated/form-vocabulary.json");
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&vocabulary).unwrap() + "\n",
    )
    .unwrap();
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

#[test]
fn agent_without_persona_deserialises() {
    let v = json!({
        "id": "a1", "goal": "t", "workdir": "/w",
        "workspaces": [],
        "session": "fleet-a1", "status": "working", "activity": "", "status_since": 1,
        "has_changes": false, "paused": false, "owner": "me", "blocked_reason": null
    });
    let a: Agent = serde_json::from_value(v).unwrap();
    assert_eq!(a.persona, "");
}

#[test]
fn spawn_request_without_persona_deserialises() {
    let v = json!({"actor": "local", "product": "app", "prompt": "x"});
    let r: SpawnRequest = serde_json::from_value(v).unwrap();
    assert_eq!(r.persona, "");
}

fn old_agent_value() -> serde_json::Value {
    json!({
        "id": "a1", "goal": "t", "workdir": "/w",
        "workspaces": [],
        "session": "fleet-a1", "status": "working", "activity": "", "status_since": 1,
        "has_changes": false, "paused": false, "owner": "me", "blocked_reason": null
    })
}

#[test]
fn old_agent_json_is_solo() {
    let a: Agent = serde_json::from_value(old_agent_value()).unwrap();
    assert_eq!(a.kind, AgentKind::Solo);
    assert_eq!(a.task, None);
    assert_eq!(a.role, None);
    assert_eq!(a.step, None);
    assert_eq!(task_id(&a), &a.id);
}

#[test]
fn task_id_prefers_the_task_field() {
    let mut a: Agent = serde_json::from_value(old_agent_value()).unwrap();
    a.task = Some(AgentId("p1".into()));
    assert_eq!(task_id(&a), &AgentId("p1".into()));
}

#[test]
fn rollup_order() {
    use Status::*;
    assert_eq!(rollup([Review, NeedsYou, Working]), NeedsYou);
    assert_eq!(rollup([Review, Working]), Working);
    assert_eq!(rollup([Talking, Queued]), Working);
    assert_eq!(rollup([Queued, Review]), Queued);
    assert_eq!(rollup([Review, Stopped]), Review);
    assert_eq!(rollup([]), Stopped);
}

#[test]
fn old_task_json_loads() {
    let v = json!({
        "id": "p1", "product": "app", "name": "n", "goal": "t", "workdir": "/w", "repos": [],
        "state": "running", "agents_used": 1,
        "times": {"queued": 0, "working": 0, "needs_you": 0, "review": 0},
        "log": [],
        // Written by a build that had a per-repo test command: ignored now.
        "test_command": "cargo test",
        // Written by a build that typed breakdown decisions into the lead's session: ignored now.
        "pending_line": "fleet-manager: breakdown v1 approved"
    });
    let p: Task = serde_json::from_value(v).unwrap();
    assert_eq!(p.kind, AgentKind::Solo);
    assert!(p.breakdowns.is_empty() && p.inbox.is_empty() && p.gate_runs.is_empty());
    assert!(p.specialists.is_empty() && p.lead.is_none());
    assert!(p.pr_rules.is_none());

    // A record from before the roster went away: its roster is what the orchestrator was shown.
    let mut v = serde_json::to_value(&p).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("specialists");
    o.insert("roster".into(), json!(["rust"]));
    let old: Task = serde_json::from_value(v).unwrap();
    assert_eq!(old.specialists, ["rust"]);
}

#[test]
fn pr_rules_round_trip() {
    let rules = PrRules {
        style: "conventional-semver-pre1.0".into(),
        title: Some("^(feat|fix)".into()),
        commits: Some("^(feat|fix)".into()),
        sections: vec!["Summary".into(), "Test plan".into()],
        versioning: Versioning::SemverPre1,
        prose: "Be brief.".into(),
        release_notes: Some("unreleased.md".into()),
        breaking: vec!["BREAKING CHANGE".into()],
        attribution: "Co-Authored-By: x".into(),
        draft: true,
        labels: vec!["bot".into()],
    };
    let v = serde_json::to_value(&rules).unwrap();
    assert_eq!(v["versioning"], json!("semver-pre1.0"));
    assert_eq!(serde_json::from_value::<PrRules>(v).unwrap(), rules);
    let empty: PrRules = serde_json::from_value(json!({})).unwrap();
    assert_eq!(empty.versioning, Versioning::None);
    assert_eq!(empty.commits, None);
}

#[test]
fn fleet_wire_names() {
    assert_eq!(
        serde_json::to_value(AgentKind::Orchestrator).unwrap(),
        json!("orchestrator")
    );
    assert_eq!(
        serde_json::to_value(AgentKind::Supervisor).unwrap(),
        json!("supervisor")
    );
    assert_eq!(
        serde_json::from_value::<AgentKind>(json!("supervisor")).unwrap(),
        AgentKind::Supervisor
    );
    assert_eq!(
        serde_json::to_value(BlockedReason::Breakdown).unwrap(),
        json!("breakdown")
    );
    assert_eq!(
        serde_json::to_value(StopCause::Finished).unwrap(),
        json!("finished")
    );
    assert_eq!(
        serde_json::to_value(Approval::ChangesRequested).unwrap(),
        json!("changes_requested")
    );
    assert_eq!(
        serde_json::to_value(FleetEventKind::IdleWithoutFinish).unwrap(),
        json!("idle_without_finish")
    );
    assert_eq!(
        serde_json::to_value(FleetEventKind::BreakdownDecided).unwrap(),
        json!("breakdown_decided")
    );
    let d: StepDraft = serde_json::from_value(
        json!({"title": "t", "specialist": "s", "brief": "b", "depends_on": []}),
    )
    .unwrap();
    assert_eq!(d.n, None);
    assert!(!d.release_note);
    let r: SpawnRequest =
        serde_json::from_value(json!({"actor": "local", "product": "app", "prompt": "x"})).unwrap();
    assert_eq!(r.kind, AgentKind::Solo);
    assert!(r.role.is_none());
}

#[test]
fn lists_from_an_older_daemon_have_no_paths() {
    let tools: ToolList = serde_json::from_value(json!({ "tools": [], "checked_at": 1 })).unwrap();
    assert_eq!(tools.path, None);
    let personas: PersonaList = serde_json::from_value(json!({
        "personas": [],
        "claude_command": ["npx", "claude"],
        "gh_command": "gh",
        "tasks_root": "/t",
    }))
    .unwrap();
    assert_eq!(
        (personas.claude_path.as_deref(), personas.gh_path.as_deref()),
        (None, None)
    );
    assert_eq!(personas.claude_login_command(), vec!["npx", "claude"]);
    assert_eq!(personas.gh_login_program(), "gh");
    let found = PersonaList {
        claude_path: Some("/opt/npx".into()),
        gh_path: Some("/opt/gh".into()),
        ..personas
    };
    assert_eq!(found.claude_login_command(), vec!["/opt/npx", "claude"]);
    assert_eq!(found.gh_login_program(), "/opt/gh");
}
