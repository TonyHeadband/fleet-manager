//! Pure mapping from Claude Code hook payloads to an agent's board status.
use fleet_api::{BlockedReason, Status};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationKind {
    PermissionPrompt,
    IdlePrompt,
    Elicitation,
    AgentNeedsInput,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse { tool: String },
    PermissionRequest { tool: String },
    PostToolUse,
    PostToolUseFailure,
    PermissionDenied,
    SubagentStart,
    Notification { kind: NotificationKind },
    Stop,
    StopFailure,
    SessionEnd,
    Other(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("hook body is not a JSON object")]
    NotAnObject,
    #[error("hook body has no hook_event_name")]
    MissingEventName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentState {
    pub status: Status,
    pub activity: String,
    pub paused: bool,
    pub blocked: Option<BlockedReason>,
}

pub fn parse_hook(body: &Value) -> Result<HookEvent, ParseError> {
    let obj = body.as_object().ok_or(ParseError::NotAnObject)?;
    let name = obj
        .get("hook_event_name")
        .and_then(Value::as_str)
        .ok_or(ParseError::MissingEventName)?;
    let tool = || {
        obj.get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string()
    };
    Ok(match name {
        "SessionStart" => HookEvent::SessionStart,
        "UserPromptSubmit" => HookEvent::UserPromptSubmit,
        "PreToolUse" => HookEvent::PreToolUse { tool: tool() },
        "PermissionRequest" => HookEvent::PermissionRequest { tool: tool() },
        "PostToolUse" => HookEvent::PostToolUse,
        "PostToolUseFailure" => HookEvent::PostToolUseFailure,
        "PermissionDenied" => HookEvent::PermissionDenied,
        "SubagentStart" => HookEvent::SubagentStart,
        "Stop" => HookEvent::Stop,
        "StopFailure" => HookEvent::StopFailure,
        // /clear ends one Claude session and starts the next in the same tmux session; only a real exit is Stopped.
        "SessionEnd" if obj.get("reason").and_then(Value::as_str) == Some("clear") => {
            HookEvent::Other("SessionEnd:clear".into())
        }
        "SessionEnd" => HookEvent::SessionEnd,
        "Notification" => {
            let kind = match obj
                .get("notification_type")
                .and_then(Value::as_str)
                .unwrap_or("")
            {
                "permission_prompt" => NotificationKind::PermissionPrompt,
                "idle_prompt" => NotificationKind::IdlePrompt,
                "elicitation_dialog" | "elicitation_url_dialog" => NotificationKind::Elicitation,
                "agent_needs_input" => NotificationKind::AgentNeedsInput,
                other => NotificationKind::Other(other.to_string()),
            };
            HookEvent::Notification { kind }
        }
        other => HookEvent::Other(other.to_string()),
    })
}

/// Returns the new state, or `None` when the event changes nothing (duplicates, unknown events,
/// anything after the session ended).
pub fn reduce(prev: &AgentState, ev: &HookEvent) -> Option<AgentState> {
    if prev.status == Status::Stopped {
        return None;
    }
    let (status, activity, blocked) = match ev {
        HookEvent::SessionStart => (Status::Working, "starting".to_string(), None),
        HookEvent::UserPromptSubmit => (Status::Working, "thinking".to_string(), None),
        HookEvent::PreToolUse { tool } => (Status::Working, format!("running {tool}"), None),
        HookEvent::PostToolUse | HookEvent::PostToolUseFailure | HookEvent::PermissionDenied => {
            (Status::Working, "thinking".to_string(), None)
        }
        HookEvent::SubagentStart => (Status::Working, "running a subagent".to_string(), None),
        HookEvent::PermissionRequest { tool } => (
            Status::NeedsYou,
            format!("wants to run {tool}"),
            Some(BlockedReason::Permission),
        ),
        HookEvent::Notification { kind } => match kind {
            // PermissionRequest already said it, with the tool name; the late notification adds nothing.
            NotificationKind::PermissionPrompt if prev.status == Status::NeedsYou => return None,
            NotificationKind::PermissionPrompt => (
                Status::NeedsYou,
                "waiting for permission".to_string(),
                Some(BlockedReason::Permission),
            ),
            NotificationKind::Elicitation => (
                Status::NeedsYou,
                "asks a question".to_string(),
                Some(BlockedReason::Question),
            ),
            NotificationKind::AgentNeedsInput => (
                Status::NeedsYou,
                "needs input".to_string(),
                Some(BlockedReason::Question),
            ),
            NotificationKind::IdlePrompt => (Status::Review, "idle".to_string(), None),
            NotificationKind::Other(_) => return None,
        },
        HookEvent::Stop => (Status::Review, "finished".to_string(), None),
        // A failed turn leaves the agent idle until its owner acts, so it is a blocked card, not a finished one.
        HookEvent::StopFailure => (
            Status::NeedsYou,
            "stopped with an error".to_string(),
            Some(BlockedReason::Error),
        ),
        HookEvent::SessionEnd => (Status::Stopped, "agent exited".to_string(), None),
        HookEvent::Other(_) => return None,
    };
    // An ended session has nothing left to pause, so a Stopped card never shows the badge.
    let paused = prev.paused && !matches!(ev, HookEvent::UserPromptSubmit | HookEvent::SessionEnd);
    let next = AgentState {
        status,
        activity,
        paused,
        blocked,
    };
    (next != *prev).then_some(next)
}
