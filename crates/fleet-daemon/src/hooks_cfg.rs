//! Generates the per-agent `--settings` file that makes Claude Code report hook events to the daemon, and the
//! `--mcp-config` file that connects it to the daemon's MCP server.
use crate::repo_rules::RepoRules;
use fleet_api::{AgentId, AgentSettings};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(
        "agent id {0:?} is not safe to embed in a hook command (letters, digits, '-' and '_' only)"
    )]
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

/// The fleet MCP tools only ask; granting still needs the owner. Without this Claude Code shows a permission
/// prompt before every call, which an unattended agent would sit at instead of reaching the daemon.
pub const ALLOWED_TOOLS: [&str; 2] = ["mcp__fleet__list_repos", "mcp__fleet__request_repo"];

/// These two events only support command handlers, so they pipe the payload to the same URL with curl.
const COMMAND_EVENTS: [&str; 2] = ["SessionStart", "SessionEnd"];

fn valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn valid_base_url(url: &str) -> bool {
    let port = url
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| url.strip_prefix("http://[::1]:"));
    matches!(port, Some(p) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn check(base_url: &str, agent: &AgentId) -> Result<(), SettingsError> {
    if !valid_agent_id(&agent.0) {
        return Err(SettingsError::BadAgentId(agent.0.clone()));
    }
    if !valid_base_url(base_url) {
        return Err(SettingsError::BadBaseUrl(base_url.to_string()));
    }
    Ok(())
}

/// Writes `<dir>/settings.json` and returns its path. The agent id and base url are validated because
/// they end up inside a shell command for the two command-only events.
/// `repo_rules` are the project's repos' deny and ask rules, merged in because Claude does not read them itself
/// (see `repo_rules`); written again whenever a repo joins the project. `fleet` is the fleet-wide config: its allow
/// rules extend the fleet tools', its deny rules follow the repos', and its env is the agent's.
pub fn write_settings(
    dir: &Path,
    base_url: &str,
    agent: &AgentId,
    repo_rules: &RepoRules,
    fleet: &AgentSettings,
) -> Result<PathBuf, SettingsError> {
    check(base_url, agent)?;
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
        hooks.insert(
            ev.into(),
            json!([{ "matcher": "", "hooks": [{ "type": "command", "command": command }]}]),
        );
    }

    let mut permissions = serde_json::Map::new();
    let allow: Vec<&str> = ALLOWED_TOOLS
        .into_iter()
        .chain(fleet.allow.iter().map(String::as_str))
        .collect();
    permissions.insert("allow".into(), json!(allow));
    let deny: Vec<&String> = repo_rules.deny.iter().chain(&fleet.deny).collect();
    for (key, rules) in [("deny", deny), ("ask", repo_rules.ask.iter().collect())] {
        if !rules.is_empty() {
            permissions.insert(key.into(), json!(rules));
        }
    }
    let mut settings = json!({
        "hooks": Value::Object(hooks),
        "permissions": Value::Object(permissions),
    });
    if !fleet.env.is_empty() {
        settings["env"] = json!(fleet.env);
    }

    std::fs::create_dir_all(dir)?;
    let path = dir.join("settings.json");
    std::fs::write(&path, serde_json::to_string_pretty(&settings)?)?;
    Ok(path)
}

/// Writes `<dir>/mcp.json`, which points Claude Code at `/internal/mcp/<id>` as the http server "fleet". The file
/// holds the token itself, so it is created 0600 and never readable by anyone else, not even briefly.
pub fn write_mcp_config(
    dir: &Path,
    base_url: &str,
    agent: &AgentId,
    token: &str,
) -> Result<PathBuf, SettingsError> {
    check(base_url, agent)?;
    let config = json!({ "mcpServers": { "fleet": {
        "type": "http",
        "url": format!("{base_url}/internal/mcp/{}", agent.0),
        "headers": { "X-Fleet-Token": token },
    }}});
    std::fs::create_dir_all(dir)?;
    let path = dir.join("mcp.json");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut file = opts.open(&path)?;
    // `mode` only applies to a file it creates; a left-over one keeps its old bits otherwise.
    #[cfg(unix)]
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    std::io::Write::write_all(&mut file, serde_json::to_string_pretty(&config)?.as_bytes())?;
    Ok(path)
}
