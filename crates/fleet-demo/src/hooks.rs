//! Builds the hook payloads Claude Code would send and delivers them to the daemon's `/internal/hooks/<id>`.
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

const SESSION_START: &str = include_str!("../../../fixtures/hooks/SessionStart-1.json");
const USER_PROMPT_SUBMIT: &str = include_str!("../../../fixtures/hooks/UserPromptSubmit-1.json");
const PRE_TOOL_USE: &str = include_str!("../../../fixtures/hooks/PreToolUse-1.json");
const POST_TOOL_USE: &str = include_str!("../../../fixtures/hooks/PostToolUse-1.json");
const PERMISSION_REQUEST: &str = include_str!("../../../fixtures/hooks/PermissionRequest-1.json");
const NOTIFICATION: &str = include_str!("../../../fixtures/hooks/Notification-1.json");
const STOP: &str = include_str!("../../../fixtures/hooks/Stop-1.json");
const SESSION_END: &str = include_str!("../../../fixtures/hooks/SessionEnd-1.json");

#[derive(Clone, Debug)]
pub struct HookTarget {
    pub url: String,
    pub token: String,
}

/// Reads the URL from the settings the daemon wrote, so the demo posts exactly where a real CLI would.
pub fn target_from_settings(settings: &Path, token: String) -> Result<HookTarget, String> {
    let text = std::fs::read_to_string(settings)
        .map_err(|e| format!("cannot read {}: {e}", settings.display()))?;
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not JSON: {e}", settings.display()))?;
    let url = json["hooks"]["PreToolUse"][0]["hooks"][0]["url"]
        .as_str()
        .ok_or_else(|| {
            format!(
                "{} has no hooks.PreToolUse[0].hooks[0].url",
                settings.display()
            )
        })?;
    Ok(HookTarget {
        url: url.to_string(),
        token,
    })
}

#[derive(Clone, Debug)]
pub enum Hook {
    SessionStart,
    UserPromptSubmit { prompt: String },
    PreToolUse { tool: String, input: Value },
    PostToolUse { tool: String, input: Value },
    PermissionRequest { tool: String, input: Value },
    Question { text: String },
    Stop,
    SessionEnd,
}

/// Starts from a recorded real payload so fields the reducer may grow to read stay realistic.
pub fn payload(h: &Hook, session_id: &str, cwd: &Path) -> Value {
    let (fixture, event) = match h {
        Hook::SessionStart => (SESSION_START, "SessionStart"),
        Hook::UserPromptSubmit { .. } => (USER_PROMPT_SUBMIT, "UserPromptSubmit"),
        Hook::PreToolUse { .. } => (PRE_TOOL_USE, "PreToolUse"),
        Hook::PostToolUse { .. } => (POST_TOOL_USE, "PostToolUse"),
        Hook::PermissionRequest { .. } => (PERMISSION_REQUEST, "PermissionRequest"),
        Hook::Question { .. } => (NOTIFICATION, "Notification"),
        Hook::Stop => (STOP, "Stop"),
        Hook::SessionEnd => (SESSION_END, "SessionEnd"),
    };
    let mut v: Value = serde_json::from_str(fixture).expect("embedded fixture is valid JSON");
    v["session_id"] = json!(session_id);
    v["cwd"] = json!(cwd);
    v["transcript_path"] = json!(cwd.join(".demo-transcript.jsonl"));
    v["hook_event_name"] = json!(event);
    match h {
        Hook::UserPromptSubmit { prompt } => v["prompt"] = json!(prompt),
        Hook::PreToolUse { tool, input }
        | Hook::PostToolUse { tool, input }
        | Hook::PermissionRequest { tool, input } => {
            v["tool_name"] = json!(tool);
            v["tool_input"] = input.clone();
        }
        Hook::Question { text } => {
            v["notification_type"] = json!("elicitation_dialog");
            v["message"] = json!(text);
        }
        Hook::SessionStart | Hook::Stop | Hook::SessionEnd => {}
    }
    v
}

pub trait Sink {
    fn send(&mut self, body: Value);
}

impl Sink for Vec<Value> {
    fn send(&mut self, body: Value) {
        self.push(body);
    }
}

pub struct HttpSink {
    target: HookTarget,
    agent: ureq::Agent,
    warned: bool,
}

impl HttpSink {
    pub fn new(target: HookTarget) -> Self {
        // The daemon is local; a hung one must not stall the scripted timeline for long.
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            target,
            agent,
            warned: false,
        }
    }
}

impl Sink for HttpSink {
    fn send(&mut self, body: Value) {
        let res = self
            .agent
            .post(&self.target.url)
            .header("X-Fleet-Token", &self.target.token)
            .send_json(&body);
        let err = match res {
            Ok(r) if r.status().is_success() => return,
            Ok(r) => format!("HTTP {}", r.status()),
            Err(e) => e.to_string(),
        };
        // Once only: a dead daemon would otherwise bury the log in one line per event.
        if !self.warned {
            self.warned = true;
            eprintln!("fleet-demo: hook post to {} failed: {err}", self.target.url);
        }
    }
}
