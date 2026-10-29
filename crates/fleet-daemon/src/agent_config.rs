//! What every agent gets whatever its persona: instructions and allow/deny/env settings, kept as two files in
//! `Config.agent_config_dir` (`instructions.md`, `settings.json`). Personas share nothing with `~/.claude`, so this
//! is the one place a user sets them for all agents.
use crate::agents::MCP_HOLD_ENV;
use crate::hooks_cfg::ALLOWED_TOOLS;
use crate::personas::ambient_vars;
use fleet_api::{AgentConfig, AgentConfigUpdate, AgentSettings};
use serde::Deserialize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const INSTRUCTIONS: &str = "instructions.md";
const SETTINGS: &str = "settings.json";

#[derive(Debug, thiserror::Error)]
pub enum AgentConfigError {
    #[error("{0:?} is not a permission rule (a tool name, optionally followed by (...))")]
    BadRule(String),
    #[error("{0:?} would deny a fleet tool, which every agent needs to ask for repos")]
    DeniesFleetTool(String),
    #[error("{0:?} is not a valid environment variable name")]
    BadEnvName(String),
    #[error("{0} is set by the daemon and cannot be overridden here")]
    ReservedEnv(String),
    #[error("nothing to save: give the instructions, the settings or both")]
    Empty,
    #[error("cannot read {SETTINGS}: {0}")]
    Parse(serde_json::Error),
    #[error("cannot write the agent config: {0}")]
    Encode(serde_json::Error),
    #[error("cannot access the agent config: {0}")]
    Io(#[from] std::io::Error),
}

/// A file may omit any field; the settings type itself requires all of them.
// Unknown keys fail: a typo'd "denny" would otherwise silently drop the rule.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsFile {
    allow: Vec<String>,
    deny: Vec<String>,
    env: std::collections::BTreeMap<String, String>,
}

fn read_instructions(dir: &Path) -> Result<String, AgentConfigError> {
    match std::fs::read_to_string(dir.join(INSTRUCTIONS)) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

fn read_settings(dir: &Path) -> Result<AgentSettings, AgentConfigError> {
    match std::fs::read_to_string(dir.join(SETTINGS)) {
        Ok(t) => {
            let f: SettingsFile = serde_json::from_str(&t).map_err(AgentConfigError::Parse)?;
            Ok(AgentSettings {
                allow: f.allow,
                deny: f.deny,
                env: f.env,
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AgentSettings::default()),
        Err(e) => Err(e.into()),
    }
}

/// Reads the config; a missing file is the default. Not validated. A file that does not parse is an error naming it.
pub fn load(dir: &Path) -> Result<AgentConfig, AgentConfigError> {
    Ok(AgentConfig {
        instructions: read_instructions(dir)?,
        settings: read_settings(dir)?,
        settings_error: None,
    })
}

/// What the GUI shows: a settings.json that is unreadable or invalid does not hide the instructions. It comes back
/// as default settings plus the message, and saving valid ones repairs the file. A broken instructions.md is still
/// an error.
pub fn load_lenient(dir: &Path) -> Result<AgentConfig, AgentConfigError> {
    let instructions = read_instructions(dir)?;
    let checked = read_settings(dir).and_then(|settings| {
        validate_settings(&settings)?;
        Ok(settings)
    });
    let (settings, settings_error) = match checked {
        Ok(s) => (s, None),
        Err(AgentConfigError::Io(e)) => (
            AgentSettings::default(),
            Some(format!("cannot read {SETTINGS}: {e}")),
        ),
        Err(e) => (AgentSettings::default(), Some(e.to_string())),
    };
    Ok(AgentConfig {
        instructions,
        settings,
        settings_error,
    })
}

/// Reads and validates: what spawn and attach use, so a hand-edited bad file stops them before anything is made.
pub fn load_valid(dir: &Path) -> Result<AgentConfig, AgentConfigError> {
    let c = load(dir)?;
    validate(&c)?;
    Ok(c)
}

/// Validates what is given, then writes only that, each file through a temp file and a rename so a reader never
/// sees half of one. Both together are not atomic: a crash between the renames leaves new instructions with old
/// settings, and both files are valid on their own, so that is accepted over a lock or a directory swap.
pub fn save(dir: &Path, u: &AgentConfigUpdate) -> Result<(), AgentConfigError> {
    if u.instructions.is_none() && u.settings.is_none() {
        return Err(AgentConfigError::Empty);
    }
    let settings = match &u.settings {
        Some(s) => {
            validate_settings(s)?;
            Some(serde_json::to_string_pretty(s).map_err(AgentConfigError::Encode)?)
        }
        None => None,
    };
    std::fs::create_dir_all(dir)?;
    if let Some(i) = &u.instructions {
        write_atomic(dir, INSTRUCTIONS, i.as_bytes())?;
    }
    if let Some(s) = settings {
        write_atomic(dir, SETTINGS, s.as_bytes())?;
    }
    Ok(())
}

fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    // Unique per call so two concurrent saves never write the same temp file.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dir.join(name)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub fn validate(c: &AgentConfig) -> Result<(), AgentConfigError> {
    validate_settings(&c.settings)
}

pub fn validate_settings(s: &AgentSettings) -> Result<(), AgentConfigError> {
    for rule in s.allow.iter().chain(&s.deny) {
        if !valid_rule(rule) {
            return Err(AgentConfigError::BadRule(rule.clone()));
        }
    }
    // A prefix denies too (`mcp__fleet` matches the whole server), and without these tools an agent cannot ask for a repo.
    // The name part only: `mcp__fleet(*)` denies as much as `mcp__fleet` does.
    for rule in &s.deny {
        let name = rule.split('(').next().unwrap_or(rule);
        if ALLOWED_TOOLS.iter().any(|t| t.starts_with(name)) {
            return Err(AgentConfigError::DeniesFleetTool(rule.clone()));
        }
    }
    for key in s.env.keys() {
        if !valid_env_name(key) {
            return Err(AgentConfigError::BadEnvName(key.clone()));
        }
        if reserved_env(key) {
            return Err(AgentConfigError::ReservedEnv(key.clone()));
        }
    }
    Ok(())
}

/// `^[A-Za-z_][A-Za-z0-9_-]*(\(.*\))?$`
fn valid_rule(rule: &str) -> bool {
    let name_end = rule.find('(').unwrap_or(rule.len());
    let (name, args) = rule.split_at(name_end);
    let mut chars = name.chars();
    let name_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    // `.` does not match a newline in the pattern.
    let args_ok =
        args.is_empty() || (args.len() >= 2 && args.ends_with(')') && !args.contains('\n'));
    name_ok && args_ok
}

/// `^[A-Za-z_][A-Za-z0-9_]*$`
pub(crate) fn valid_env_name(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whatever the daemon sets for an agent or strips from it: a user env must never bring that back.
pub(crate) fn reserved_env(key: &str) -> bool {
    key == "CLAUDE_CONFIG_DIR"
        || key == "FLEET_TOKEN"
        || key.starts_with("GH_")
        || key.starts_with("GIT_")
        || MCP_HOLD_ENV.iter().any(|(k, _)| *k == key)
        || ambient_vars().any(|k| k == key)
}
