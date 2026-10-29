use fleet_api::{Agent, Effort, PermissionMode, Step};
use serde_json::json;

#[test]
fn modes_parse_as_the_cli_spells_them() {
    assert_eq!("acceptEdits".parse(), Ok(PermissionMode::AcceptEdits));
    assert_eq!("dontAsk".parse(), Ok(PermissionMode::DontAsk));
    for mode in [
        PermissionMode::Auto,
        PermissionMode::AcceptEdits,
        PermissionMode::Manual,
        PermissionMode::DontAsk,
    ] {
        assert_eq!(mode.as_flag().parse(), Ok(mode));
    }
}

#[test]
fn modes_rank_from_loosest_to_strictest_with_dont_ask_outside() {
    assert_eq!(PermissionMode::Auto.rank(), Some(0));
    assert_eq!(PermissionMode::AcceptEdits.rank(), Some(1));
    assert_eq!(PermissionMode::Manual.rank(), Some(2));
    assert_eq!(PermissionMode::DontAsk.rank(), None);
}

#[test]
fn plan_and_bypass_are_refused() {
    for m in ["plan", "bypassPermissions"] {
        assert_eq!(
            m.parse::<PermissionMode>().unwrap_err().to_string(),
            format!("mode {m} is not allowed (auto, acceptEdits, manual, dontAsk)")
        );
    }
}

#[test]
fn efforts_parse() {
    for (text, effort) in [
        ("low", Effort::Low),
        ("medium", Effort::Medium),
        ("high", Effort::High),
        ("xhigh", Effort::Xhigh),
        ("max", Effort::Max),
    ] {
        assert_eq!(text.parse(), Ok(effort));
        assert_eq!(effort.as_flag(), text);
    }
    assert!("huge".parse::<Effort>().is_err());
}

#[test]
fn values_serialize_as_the_cli_spells_them() {
    assert_eq!(json!(PermissionMode::AcceptEdits), json!("acceptEdits"));
    assert_eq!(json!(PermissionMode::DontAsk), json!("dontAsk"));
    assert_eq!(json!(Effort::Xhigh), json!("xhigh"));
}

#[test]
fn old_records_without_the_fields_still_read() {
    let step: Step = serde_json::from_value(json!({
        "n": 1, "title": "t", "specialist": "rust", "brief": "b", "state": "pending"
    }))
    .unwrap();
    assert_eq!((step.mode, step.effort), (None, None));

    let mut agent = serde_json::to_value(sample_agent()).unwrap();
    agent.as_object_mut().unwrap().remove("mode");
    agent.as_object_mut().unwrap().remove("effort");
    let agent: Agent = serde_json::from_value(agent).unwrap();
    assert_eq!((agent.mode, agent.effort), (None, None));
}

fn sample_agent() -> Agent {
    serde_json::from_value(json!({
        "id": "a1", "goal": "g", "workdir": "/w", "workspaces": [], "session": "s",
        "status": "working", "activity": "", "status_since": 0, "has_changes": false,
        "paused": false, "owner": "local", "blocked_reason": null
    }))
    .unwrap()
}
