use fleet_api::{
    default_ui_schema, ActionDef, Agent, AgentId, ColumnDef, Status, Tone, UiSchema, Workspace,
};
use fleet_client::schema::{
    action_request, column_for, field_value, format_duration, visible_actions, FieldValue,
};
use serde_json::json;

fn ws(repo: &str, branch: &str) -> Workspace {
    Workspace {
        repo: repo.into(),
        branch: branch.into(),
        worktree: format!("/w/{repo}"),
        base: "abc".into(),
        from_branch: None,
        has_changes: false,
        owned: false,
    }
}

fn agent(status: Status, paused: bool) -> Agent {
    Agent {
        id: AgentId("abc".into()),
        goal: "fix the tests".into(),
        workdir: "/w/fix".into(),
        workspaces: vec![ws("core", "fleet/fix")],
        tmux_session: "fleet-abc".into(),
        status,
        activity: "running Bash".into(),
        status_since: 1000,
        has_changes: false,
        paused,
        owner: "antonio".into(),
        blocked_reason: None,
        repo_request: None,
        stop_cause: None,
        disk_bytes: None,
        persona: "p".into(),
        kind: fleet_api::AgentKind::Solo,
        task: None,
        role: None,
        step: None,
        attach: None,
    }
}

fn ids(schema: &UiSchema, a: &Agent) -> Vec<String> {
    visible_actions(schema, a)
        .iter()
        .map(|x| x.id.clone())
        .collect()
}

fn action<'a>(schema: &'a UiSchema, id: &str) -> &'a ActionDef {
    schema
        .actions
        .iter()
        .find(|a| a.id == id)
        .expect("action in the default schema")
}

#[test]
fn column_for_uses_schema_status_mapping() {
    let schema = default_ui_schema();
    assert_eq!(
        column_for(&schema, &agent(Status::Stopped, false))
            .unwrap()
            .id,
        "stopped"
    );
    assert_eq!(
        column_for(&schema, &agent(Status::NeedsYou, false))
            .unwrap()
            .id,
        "needs_you"
    );
    let custom = UiSchema {
        columns: vec![ColumnDef {
            id: "x".into(),
            title: "X".into(),
            statuses: vec![Status::Working, Status::Review],
            empty_hint: String::new(),
            tone: Tone::Neutral,
            dim: false,
        }],
        ..schema
    };
    assert_eq!(
        column_for(&custom, &agent(Status::Review, false))
            .unwrap()
            .id,
        "x"
    );
    assert!(column_for(&custom, &agent(Status::Queued, false)).is_none());
}

#[test]
fn visible_actions_follows_conditions() {
    let schema = default_ui_schema();
    assert_eq!(
        ids(&schema, &agent(Status::Working, false)),
        ["pause", "stop"]
    );
    // No Pause on a Needs you card: Escape would deny the pending request.
    assert_eq!(
        ids(&schema, &agent(Status::NeedsYou, false)),
        ["answer", "stop"]
    );
    assert_eq!(
        ids(&schema, &agent(Status::Review, true)),
        ["resume", "stop"]
    );
    // A stopped agent has nothing left to stop.
    assert!(ids(&schema, &agent(Status::Stopped, false)).is_empty());
    assert!(ids(&schema, &agent(Status::Stopped, true)).is_empty());
}

#[test]
fn action_request_builds_method_path_and_body() {
    let schema = default_ui_schema();
    let a = agent(Status::Working, false);
    let stop = action_request(action(&schema, "stop"), &a, None);
    assert_eq!(
        (stop.method.as_str(), stop.path.as_str(), stop.body),
        (
            "POST",
            "/api/agents/abc/stop",
            Some(json!({"actor": "local"}))
        )
    );
    let pause = action_request(action(&schema, "pause"), &a, None);
    assert_eq!(
        (pause.method.as_str(), pause.path.as_str()),
        ("POST", "/api/agents/abc/pause")
    );
    assert_eq!(pause.body, Some(json!({"actor": "local"})));
    let answer = action_request(action(&schema, "answer"), &a, Some("$(x) \"q\""));
    assert_eq!(answer.path, "/api/agents/abc/answer");
    assert_eq!(
        answer.body,
        Some(json!({"actor": "local", "text": "$(x) \"q\""}))
    );
}

#[test]
fn action_request_escapes_the_id() {
    let schema = default_ui_schema();
    let mut a = agent(Status::Working, false);
    a.id = AgentId("a/b?c d".into());
    assert_eq!(
        action_request(action(&schema, "stop"), &a, None).path,
        "/api/agents/a%2Fb%3Fc%20d/stop"
    );
}

#[test]
fn field_value_derives_repos_and_branch() {
    let mut a = agent(Status::Working, false);
    a.workspaces = vec![ws("core", "fleet/x"), ws("portal", "fleet/x")];
    assert_eq!(
        field_value(&a, "repos"),
        FieldValue::List(vec!["core".into(), "portal".into()])
    );
    assert_eq!(
        field_value(&a, "branch"),
        FieldValue::List(vec!["fleet/x".into()])
    );
}

#[test]
fn field_value_reads_plain_agent_fields() {
    let mut a = agent(Status::Working, false);
    a.paused = true;
    assert_eq!(
        field_value(&a, "goal"),
        FieldValue::Text("fix the tests".into())
    );
    assert_eq!(
        field_value(&a, "status"),
        FieldValue::Text("working".into())
    );
    assert_eq!(field_value(&a, "status_since"), FieldValue::Number(1000));
    assert_eq!(field_value(&a, "paused"), FieldValue::Flag);
}

#[test]
fn field_value_is_absent_for_empty_false_null_and_unknown() {
    let mut a = agent(Status::Working, false);
    a.activity = String::new();
    assert_eq!(field_value(&a, "activity"), FieldValue::Absent);
    assert_eq!(field_value(&a, "paused"), FieldValue::Absent);
    assert_eq!(field_value(&a, "blocked_reason"), FieldValue::Absent);
    assert_eq!(field_value(&a, "no_such_field"), FieldValue::Absent);
}

#[test]
fn format_duration_shows_two_largest_units() {
    let got: Vec<String> = [0, 5, 65, 3700, 90000, -4]
        .into_iter()
        .map(format_duration)
        .collect();
    assert_eq!(got, ["0s", "5s", "1m", "1h 1m", "1d 1h", "0s"]);
}

fn member(
    id: &str,
    kind: fleet_api::AgentKind,
    task: Option<&str>,
    status: Status,
    since: i64,
) -> Agent {
    let mut a = agent(status, false);
    a.id = AgentId(id.into());
    a.kind = kind;
    a.task = task.map(|p| AgentId(p.into()));
    a.status_since = since;
    a
}

fn ids_of(agents: &[&Agent]) -> Vec<String> {
    agents.iter().map(|a| a.id.0.clone()).collect()
}

#[test]
fn groups_and_rollup() {
    use fleet_api::AgentKind::{Orchestrator, Solo, Specialist};
    use fleet_client::schema::groups;

    // A fleet agent that needs you sends the whole task to Needs you.
    let agents = vec![
        member("o", Orchestrator, None, Status::Working, 10),
        member("f1", Specialist, Some("o"), Status::Working, 20),
        member("f2", Specialist, Some("o"), Status::NeedsYou, 30),
    ];
    let g = groups(&agents);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].lead.id.0, "o");
    assert_eq!(g[0].status, Status::NeedsYou);
    assert_eq!(ids_of(&g[0].fleet), ["f2", "f1"], "the stuck agent first");
    assert_eq!(g[0].stuck().map(|a| a.id.0.as_str()), Some("f2"));

    // Ended fleet agents do not count: Review once every live agent is done.
    let mut finished = member("f2", Specialist, Some("o"), Status::Stopped, 30);
    finished.stop_cause = Some(fleet_api::StopCause::Finished);
    let agents = vec![
        member("o", Orchestrator, None, Status::Review, 10),
        member("f1", Specialist, Some("o"), Status::Stopped, 20),
        finished,
    ];
    let g = groups(&agents);
    assert_eq!(g[0].status, Status::Review);
    assert!(!g[0].lead_down());

    // A restarted orchestrator leads; the old one is an ended row of the fleet.
    let agents = vec![
        member("o", Orchestrator, None, Status::Stopped, 10),
        member("f1", Specialist, Some("o"), Status::Working, 20),
        member("o2", Orchestrator, Some("o"), Status::Queued, 40),
    ];
    let g = groups(&agents);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].lead.id.0, "o2");
    assert_eq!(ids_of(&g[0].fleet), ["o", "f1"]);
    assert_eq!(g[0].status, Status::Working);

    // A crashed orchestrator with live fleet goes to Needs you.
    let mut crashed = member("o", Orchestrator, None, Status::Stopped, 10);
    crashed.stop_cause = Some(fleet_api::StopCause::Crashed);
    let agents = vec![
        crashed,
        member("f1", Specialist, Some("o"), Status::Working, 20),
    ];
    let g = groups(&agents);
    assert_eq!(g[0].status, Status::NeedsYou);
    assert!(g[0].lead_down());
    assert!(g[0].stuck().is_none(), "no fleet agent is the reason");

    // Every agent stopped: the task is stopped, not waiting on you.
    let agents = vec![
        member("o", Orchestrator, None, Status::Stopped, 10),
        member("f1", Specialist, Some("o"), Status::Stopped, 20),
    ];
    assert_eq!(groups(&agents)[0].status, Status::Stopped);

    // A solo agent is its own group, with no fleet; Talking counts as Working.
    let agents = vec![
        member("s", Solo, None, Status::Talking, 5),
        member("o", Orchestrator, None, Status::Queued, 10),
    ];
    let g = groups(&agents);
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].lead.id.0, "s");
    assert!(g[0].fleet.is_empty());
    assert_eq!(g[0].status, Status::Working);
    assert_eq!(g[1].lead.id.0, "o");
    assert!(g[1].fleet.is_empty());
}

#[test]
fn column_for_status_maps_like_column_for() {
    use fleet_client::schema::column_for_status;
    let schema = default_ui_schema();
    for s in Status::ALL {
        assert_eq!(
            column_for_status(&schema, s).map(|c| &c.id),
            column_for(&schema, &agent(s, false)).map(|c| &c.id),
            "{s:?}"
        );
    }
}

#[test]
fn an_ended_orchestrator_asks_for_a_restart() {
    use fleet_api::AgentKind::{Orchestrator, Solo, Specialist};
    use fleet_api::StopCause;
    use fleet_client::schema::groups;

    // Crashed or exited with no live fleet: the task is not known to be done, so it needs you.
    for cause in [StopCause::Crashed, StopCause::Exited] {
        let mut lead = member("o", Orchestrator, None, Status::Stopped, 10);
        lead.stop_cause = Some(cause);
        let mut done = member("f1", Specialist, Some("o"), Status::Stopped, 20);
        done.stop_cause = Some(StopCause::Finished);
        let agents = vec![lead, done];
        let g = groups(&agents);
        assert_eq!(g[0].status, Status::NeedsYou, "{cause:?}");
        assert!(g[0].lead_down(), "{cause:?}");
        assert!(g[0].can_restart(), "{cause:?}");
    }

    // Stopped by Stop task: the task is stopped, but it can still be restarted.
    let mut lead = member("o", Orchestrator, None, Status::Stopped, 10);
    lead.stop_cause = Some(StopCause::Stopped);
    let mut fleet = member("f1", Specialist, Some("o"), Status::Stopped, 20);
    fleet.stop_cause = Some(StopCause::Stopped);
    let agents = vec![lead, fleet];
    let g = groups(&agents);
    assert_eq!(g[0].status, Status::Stopped);
    assert!(!g[0].lead_down());
    assert!(g[0].can_restart());

    // A running orchestrator and a crashed solo agent have nothing to restart.
    let mut solo = member("s", Solo, None, Status::Stopped, 5);
    solo.stop_cause = Some(StopCause::Crashed);
    let agents = vec![solo, member("o", Orchestrator, None, Status::Working, 10)];
    let g = groups(&agents);
    assert_eq!(g[0].status, Status::Stopped);
    assert!(!g[0].lead_down() && !g[0].can_restart());
    assert!(!g[1].lead_down() && !g[1].can_restart());
}
