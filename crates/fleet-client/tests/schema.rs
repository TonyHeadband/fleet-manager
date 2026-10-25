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
        task: "fix the tests".into(),
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
        "review"
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
    assert_eq!(ids(&schema, &agent(Status::Stopped, false)), ["stop"]);
    assert_eq!(ids(&schema, &agent(Status::Stopped, true)), ["stop"]);
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
        field_value(&a, "task"),
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
