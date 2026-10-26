//! `~/.config/fleet-manager/config.toml`: where the daemon listens and where tasks live. The file is optional.
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub listen: SocketAddr,
    pub state_dir: PathBuf,
    pub branch_prefix: String,
    /// Program and flags used to start Claude Code. The daemon adds only `--settings` and the prompt.
    pub claude_command: Vec<String>,
    /// The tmux program: `tmux` from PATH unless the config names another (an older system tmux, say).
    pub tmux_command: String,
    /// Parent of every task folder.
    pub tasks_root: PathBuf,
    /// Parent of every persona folder (`<name>/claude`, `<name>/gh`): each persona's own Claude and GitHub login.
    pub personas_dir: PathBuf,
    /// The GitHub CLI program: `gh` from PATH unless the config names another.
    pub gh_command: String,
    /// Fleet-wide agent settings (`agent/` beside the config file). Not a key: it moves with the file.
    pub agent_config_dir: PathBuf,
    /// User orchestrator types, specialists and skills (`fleet/` beside the config file), over the built-ins.
    pub fleet_dir: PathBuf,
    /// How long the test gate may run before `open_pr` gives up on it.
    pub test_timeout_secs: u64,
    pub telemetry: Telemetry,
    /// How many v0.1 `[[repos]]` entries the file still has. They are no longer read (repos belong to products), but
    /// a file with them still loads; loading runs before the logger exists, so the caller says so.
    pub legacy_repos: usize,
}

/// `[telemetry]`. Reserved: the endpoint is read so a config written for it keeps loading, but nothing exports yet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Telemetry {
    pub otlp_endpoint: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("invalid config {0}: {1}")]
    Parse(PathBuf, toml::de::Error),
    #[error("{old} was renamed to {new}; rename it in {}", path.display())]
    RenamedKey {
        old: &'static str,
        new: &'static str,
        path: PathBuf,
    },
    #[error("listen address {0} is not loopback; v1 has no auth and only binds 127.0.0.1 or ::1")]
    NonLoopback(SocketAddr),
    #[error("branch_prefix {0:?} is not a safe git ref component")]
    BadBranchPrefix(String),
    #[error("tmux_command must name a program")]
    EmptyTmuxCommand,
    #[error("claude_command must name a program")]
    EmptyClaudeCommand,
    #[error("gh_command must name a program")]
    EmptyGhCommand,
    #[error("test_timeout_secs must be more than 0")]
    ZeroTestTimeout,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Raw {
    listen: Option<SocketAddr>,
    state_dir: Option<String>,
    branch_prefix: Option<String>,
    claude_command: Option<Vec<String>>,
    tmux_command: Option<String>,
    tasks_root: Option<String>,
    /// Only to refuse it with a pointer to the new name.
    projects_root: Option<String>,
    personas_dir: Option<String>,
    gh_command: Option<String>,
    test_timeout_secs: Option<u64>,
    /// Only counted, see `Config::legacy_repos`.
    #[serde(default)]
    repos: Vec<toml::Value>,
    #[serde(default)]
    telemetry: Telemetry,
}

pub fn default_path(home: &Path) -> PathBuf {
    home.join(".config/fleet-manager/config.toml")
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    load_with_home(path, &home_dir())
}

pub fn load_with_home(path: &Path, home: &Path) -> Result<Config, ConfigError> {
    // No file is a normal first run; only NotFound means defaults, so a permissions problem still surfaces.
    let raw: Raw = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Raw::default(),
        Err(e) => return Err(ConfigError::Io(path.to_path_buf(), e)),
    };
    if raw.projects_root.is_some() {
        return Err(ConfigError::RenamedKey {
            old: "projects_root",
            new: "tasks_root",
            path: path.to_path_buf(),
        });
    }

    let listen = raw
        .listen
        .unwrap_or_else(|| "127.0.0.1:7878".parse().expect("static address"));
    if !listen.ip().is_loopback() {
        return Err(ConfigError::NonLoopback(listen));
    }
    let branch_prefix = raw.branch_prefix.unwrap_or_else(|| "fleet".to_string());
    if !is_safe_ref_prefix(&branch_prefix) {
        return Err(ConfigError::BadBranchPrefix(branch_prefix));
    }
    let claude_command = raw
        .claude_command
        .unwrap_or_else(|| vec!["claude".to_string()]);
    if claude_command.first().is_none_or(|p| p.trim().is_empty()) {
        return Err(ConfigError::EmptyClaudeCommand);
    }
    let tmux_command = match raw.tmux_command {
        Some(t) if t.trim().is_empty() => return Err(ConfigError::EmptyTmuxCommand),
        Some(t) => expand(&t, home).to_string_lossy().into_owned(),
        None => "tmux".to_string(),
    };
    let state_dir = match raw.state_dir {
        Some(s) => expand(&s, home),
        None => home.join(".local/share/fleet-manager"),
    };

    let tasks_root = match raw.tasks_root {
        Some(s) => expand(&s, home),
        None => home.join("fleet-tasks"),
    };

    let personas_dir = match raw.personas_dir {
        Some(s) => expand(&s, home),
        None => home.join(".local/share/fleet-manager/personas"),
    };
    let gh_command = match raw.gh_command {
        Some(g) if g.trim().is_empty() => return Err(ConfigError::EmptyGhCommand),
        Some(g) => expand(&g, home).to_string_lossy().into_owned(),
        None => "gh".to_string(),
    };
    let agent_config_dir = path.parent().unwrap_or(Path::new("")).join("agent");
    let fleet_dir = path.parent().unwrap_or(Path::new("")).join("fleet");
    let test_timeout_secs = raw.test_timeout_secs.unwrap_or(1800);
    if test_timeout_secs == 0 {
        return Err(ConfigError::ZeroTestTimeout);
    }

    Ok(Config {
        listen,
        state_dir,
        branch_prefix,
        claude_command,
        tmux_command,
        tasks_root,
        personas_dir,
        gh_command,
        agent_config_dir,
        fleet_dir,
        test_timeout_secs,
        telemetry: raw.telemetry,
        legacy_repos: raw.repos.len(),
    })
}

pub(crate) fn expand(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

pub(crate) fn is_safe_dir_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(['.', '-'])
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Conservative subset of `git check-ref-format` rules, so the prefix is safe in a branch name and a path.
fn is_safe_ref_prefix(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(['-', '/', '.'])
        && !s.ends_with(['/', '.'])
        && !s.contains("..")
        && !s.contains("//")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}
