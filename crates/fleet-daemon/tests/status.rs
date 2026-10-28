use fleet_api::{BlockedReason, Status};
use fleet_daemon::status::{parse_hook, reduce, AgentState, HookEvent, NotificationKind};
use serde_json::json;
use std::path::PathBuf;

fn state(status: Status, activity: &str, paused: bool) -> AgentState {
    AgentState {
        status,
        activity: activity.into(),
        paused,
        blocked: None,
    }
}

fn fixture(name: &str) -> serde_json::Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hooks")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn after(prev: &AgentState, ev: &HookEvent) -> AgentState {
    reduce(prev, ev).unwrap_or_else(|| prev.clone())
}

#[test]
fn maps_every_event_in_the_spec_table() {
    let idle = state(Status::Queued, "", false);
    let cases: Vec<(HookEvent, Status)> = vec![
        (HookEvent::SessionStart, Status::Working),
        (HookEvent::UserPromptSubmit, Status::Working),
        (
            HookEvent::PreToolUse {
                tool: "Bash".into(),
            },
            Status::Working,
        ),
        (HookEvent::PostToolUse, Status::Working),
        (HookEvent::PostToolUseFailure, Status::Working),
        (HookEvent::PermissionDenied, Status::Working),
        (HookEvent::SubagentStart, Status::Working),
        (
            HookEvent::PermissionRequest {
                tool: "Bash".into(),
            },
            Status::NeedsYou,
        ),
        (
            HookEvent::Notification {
                kind: NotificationKind::PermissionPrompt,
            },
            Status::NeedsYou,
        ),
        (
            HookEvent::Notification {
                kind: NotificationKind::Elicitation,
            },
            Status::NeedsYou,
        ),
        (
            HookEvent::Notification {
                kind: NotificationKind::AgentNeedsInput,
            },
            Status::NeedsYou,
        ),
        (HookEvent::Stop, Status::Review),
        (HookEvent::StopFailure, Status::NeedsYou),
        (
            HookEvent::Notification {
                kind: NotificationKind::IdlePrompt,
            },
            Status::Review,
        ),
        (HookEvent::SessionEnd, Status::Stopped),
    ];
    for (ev, want) in cases {
        let got = reduce(&idle, &ev)
            .unwrap_or_else(|| panic!("{ev:?} produced no change"))
            .status;
        assert_eq!(got, want, "{ev:?}");
    }
}

#[test]
fn blocked_reason_follows_the_event_that_caused_it() {
    let working = state(Status::Working, "thinking", false);
    let cases = [
        (
            HookEvent::PermissionRequest {
                tool: "Bash".into(),
            },
            BlockedReason::Permission,
        ),
        (
            HookEvent::Notification {
                kind: NotificationKind::PermissionPrompt,
            },
            BlockedReason::Permission,
        ),
        (
            HookEvent::Notification {
                kind: NotificationKind::Elicitation,
            },
            BlockedReason::Question,
        ),
        (
            HookEvent::Notification {
                kind: NotificationKind::AgentNeedsInput,
            },
            BlockedReason::Question,
        ),
        (HookEvent::StopFailure, BlockedReason::Error),
    ];
    for (ev, want) in cases {
        assert_eq!(reduce(&working, &ev).unwrap().blocked, Some(want), "{ev:?}");
    }
}

#[test]
fn blocked_reason_clears_when_the_agent_moves_on() {
    let blocked = AgentState {
        status: Status::NeedsYou,
        activity: "x".into(),
        paused: false,
        blocked: Some(BlockedReason::Permission),
    };
    for ev in [
        HookEvent::PostToolUse,
        HookEvent::UserPromptSubmit,
        HookEvent::Stop,
        HookEvent::SessionEnd,
    ] {
        assert_eq!(reduce(&blocked, &ev).unwrap().blocked, None, "{ev:?}");
    }
}

#[test]
fn activity_names_the_tool() {
    let s = state(Status::Working, "thinking", false);
    assert_eq!(
        reduce(
            &s,
            &HookEvent::PermissionRequest {
                tool: "Bash".into()
            }
        )
        .unwrap()
        .activity,
        "wants to run Bash"
    );
    assert_eq!(
        reduce(
            &s,
            &HookEvent::PreToolUse {
                tool: "Edit".into()
            }
        )
        .unwrap()
        .activity,
        "running Edit"
    );
}

#[test]
fn permission_resolved_by_tool_events() {
    let needs = state(Status::NeedsYou, "wants to run Bash", false);
    for ev in [
        HookEvent::PostToolUse,
        HookEvent::PostToolUseFailure,
        HookEvent::PermissionDenied,
    ] {
        assert_eq!(
            reduce(&needs, &ev).unwrap().status,
            Status::Working,
            "{ev:?}"
        );
    }
}

#[test]
fn permission_notification_after_request_changes_nothing() {
    let needs = state(Status::NeedsYou, "wants to run Bash", false);
    let ev = HookEvent::Notification {
        kind: NotificationKind::PermissionPrompt,
    };
    assert_eq!(reduce(&needs, &ev), None);
}

#[test]
fn session_end_stops_with_exited() {
    let next = reduce(&state(Status::Working, "x", false), &HookEvent::SessionEnd).unwrap();
    assert_eq!(next.status, Status::Stopped);
    assert_eq!(next.activity, "agent exited");
}

#[test]
fn nothing_moves_a_stopped_agent() {
    let dead = state(Status::Stopped, "agent exited", false);
    for ev in [
        HookEvent::SessionStart,
        HookEvent::UserPromptSubmit,
        HookEvent::Stop,
        HookEvent::PostToolUse,
    ] {
        assert_eq!(reduce(&dead, &ev), None, "{ev:?}");
    }
}

#[test]
fn duplicate_event_is_no_change() {
    let ev = HookEvent::PreToolUse {
        tool: "Bash".into(),
    };
    let once = reduce(&state(Status::Working, "thinking", false), &ev).unwrap();
    assert_eq!(reduce(&once, &ev), None);
}

#[test]
fn paused_survives_stop_and_clears_on_next_prompt() {
    let paused = state(Status::Working, "thinking", true);
    let stopped = reduce(&paused, &HookEvent::Stop).unwrap();
    assert_eq!((stopped.status, stopped.paused), (Status::Review, true));
    let resumed = reduce(&stopped, &HookEvent::UserPromptSubmit).unwrap();
    assert_eq!((resumed.status, resumed.paused), (Status::Working, false));
}

#[test]
fn session_end_for_clear_is_not_the_end_of_the_agent() {
    // /clear emits SessionEnd{reason: clear} then SessionStart{source: clear}; the terminal lives on.
    let ev = parse_hook(&json!({"hook_event_name": "SessionEnd", "reason": "clear"})).unwrap();
    assert_eq!(reduce(&state(Status::Review, "idle", false), &ev), None);
    let start = parse_hook(&json!({"hook_event_name": "SessionStart", "source": "clear"})).unwrap();
    let after = after(&state(Status::Review, "idle", false), &ev);
    assert_eq!(
        after.status,
        Status::Review,
        "the clear's SessionEnd must not kill the card"
    );
    assert_eq!(reduce(&after, &start).unwrap().status, Status::Working);
}

#[test]
fn session_end_for_any_other_reason_still_ends_the_agent() {
    for reason in [
        "prompt_input_exit",
        "other",
        "logout",
        "bypass_permissions_disabled",
    ] {
        let ev = parse_hook(&json!({"hook_event_name": "SessionEnd", "reason": reason})).unwrap();
        assert_eq!(
            reduce(&state(Status::Working, "x", false), &ev)
                .unwrap()
                .status,
            Status::Stopped,
            "{reason}"
        );
    }
    let no_reason = parse_hook(&json!({"hook_event_name": "SessionEnd"})).unwrap();
    assert_eq!(
        reduce(&state(Status::Working, "x", false), &no_reason)
            .unwrap()
            .status,
        Status::Stopped
    );
}

#[test]
fn unknown_event_is_other_and_ignored() {
    let ev = parse_hook(&json!({"hook_event_name": "BrandNewEvent"})).unwrap();
    assert_eq!(ev, HookEvent::Other("BrandNewEvent".into()));
    assert_eq!(reduce(&state(Status::Working, "x", false), &ev), None);
}

#[test]
fn unknown_notification_type_is_other() {
    let ev = parse_hook(
        &json!({"hook_event_name": "Notification", "notification_type": "plugin_notification"}),
    )
    .unwrap();
    assert_eq!(
        ev,
        HookEvent::Notification {
            kind: NotificationKind::Other("plugin_notification".into())
        }
    );
    assert_eq!(reduce(&state(Status::Working, "x", false), &ev), None);
}

#[test]
fn body_without_event_name_is_an_error() {
    assert!(parse_hook(&json!({"session_id": "x"})).is_err());
    assert!(parse_hook(&json!("not an object")).is_err());
}

#[test]
fn parses_every_fixture() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hooks");
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let ev = parse_hook(&v).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        assert!(
            !matches!(ev, HookEvent::Other(_)),
            "{path:?} parsed as Other"
        );
        n += 1;
    }
    assert!(n >= 10, "only {n} fixtures");
}

#[test]
fn replaying_the_recorded_session_walks_the_expected_statuses() {
    let order = [
        ("SessionStart-1.json", Status::Working),
        ("UserPromptSubmit-1.json", Status::Working),
        ("PreToolUse-1.json", Status::Working),
        ("PermissionRequest-1.json", Status::NeedsYou),
        ("Notification-1.json", Status::NeedsYou),
        ("PostToolUse-1.json", Status::Working),
        ("Stop-1.json", Status::Review),
        ("SessionEnd-3.json", Status::Stopped),
    ];
    let mut s = state(Status::Queued, "", false);
    for (file, want) in order {
        s = after(&s, &parse_hook(&fixture(file)).unwrap());
        assert_eq!(s.status, want, "after {file}");
    }
}
