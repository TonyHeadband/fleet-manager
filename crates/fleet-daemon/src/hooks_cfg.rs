//! Generates the per-agent `--settings` file that makes Claude Code report hook events to the daemon.
use fleet_api::AgentId;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("agent id {0:?} is not safe to embed in a hook command (letters, digits, '-' and '_' only)")]
    BadAgentId(String),
    #[error("base url {0:?} must be http://127.0.0.1:<port> or http://[::1]:<port>")]
    BadBaseUrl(String),
    #[error("cannot write settings: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot serialise settings: {0}")]
    Json(#[from] serde_json::Error),
}

/// Events delivered by http hook handlers.
const HTTP_EVENTS: [&str; 10] = [
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionDenied",
    "SubagentStart",
    "Notification",
    "Stop",
    "StopFailure",
];

/// These two events only support command handlers, so they pipe the payload to the same URL with curl.
const COMMAND_EVENTS: [&str; 2] = ["SessionStart", "SessionEnd"];

fn valid_agent_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn valid_base_url(url: &str) -> bool {
    let port = url.strip_prefix("http://127.0.0.1:").or_else(|| url.strip_prefix("http://[::1]:"));
    matches!(port, Some(p) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Writes `<dir>/settings.json` and returns its path. The agent id and base url are validated because
/// they end up inside a shell command for the two command-only events.
pub fn write_settings(dir: &Path, base_url: &str, agent: &AgentId) -> Result<PathBuf, SettingsError> {
    if !valid_agent_id(&agent.0) {
        return Err(SettingsError::BadAgentId(agent.0.clone()));
    }
    if !valid_base_url(base_url) {
        return Err(SettingsError::BadBaseUrl(base_url.to_string()));
    }
    let url = format!("{base_url}/internal/hooks/{}", agent.0);

    let mut hooks = serde_json::Map::new();
    for ev in HTTP_EVENTS {
        hooks.insert(
            ev.into(),
            json!([{ "matcher": "", "hooks": [{
                "type": "http",
                "url": url,
                "headers": { "X-Fleet-Token": "$FLEET_TOKEN" },
                "allowedEnvVars": ["FLEET_TOKEN"],
            }]}]),
        );
    }
    // `|| true`: a daemon that is down must never make Claude Code report a hook error.
    let command = format!(
        "curl -s -m 5 -X POST -H 'content-type: application/json' -H \"X-Fleet-Token: $FLEET_TOKEN\" --data-binary @- '{url}' || true"
    );
    for ev in COMMAND_EVENTS {
        hooks.insert(ev.into(), json!([{ "matcher": "", "hooks": [{ "type": "command", "command": command }]}]));
    }

    std::fs::create_dir_all(dir)?;
    let path = dir.join("settings.json");
    std::fs::write(&path, serde_json::to_string_pretty(&json!({ "hooks": Value::Object(hooks) }))?)?;
    Ok(path)
}
