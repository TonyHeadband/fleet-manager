//! `~/.config/fleet-manager/config.toml`: where the daemon listens and where projects live. The file is optional.
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub listen: SocketAddr,
    pub state_dir: PathBuf,
    pub branch_prefix: String,
    /// Program and flags used to start Claude Code. The daemon adds only `--settings` and the prompt.
    pub claude_command: Vec<String>,
    /// Parent of every project folder.
    pub projects_root: PathBuf,
    /// v0.1 `[[repos]]`; read once to seed the remembered-repo list, not consulted afterwards.
    pub seed_repos: Vec<Repo>,
    pub telemetry: Telemetry,
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
    #[error("listen address {0} is not loopback; v1 has no auth and only binds 127.0.0.1 or ::1")]
    NonLoopback(SocketAddr),
    #[error("repo name {0:?} is listed twice")]
    DuplicateRepo(String),
    #[error("repo name {0:?} must be letters, digits, '.', '_' or '-' and not start with '.' or '-' (it becomes a directory name)")]
    BadRepoName(String),
    #[error("branch_prefix {0:?} is not a safe git ref component")]
    BadBranchPrefix(String),
    #[error("claude_command must name a program")]
    EmptyClaudeCommand,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Raw {
    listen: Option<SocketAddr>,
    state_dir: Option<String>,
    branch_prefix: Option<String>,
    claude_command: Option<Vec<String>>,
    projects_root: Option<String>,
    #[serde(default)]
    repos: Vec<RawRepo>,
    #[serde(default)]
    telemetry: Telemetry,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawRepo {
    name: String,
    path: String,
}

pub fn default_path(home: &Path) -> PathBuf {
    home.join(".config/fleet-manager/config.toml")
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
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

    let listen = raw.listen.unwrap_or_else(|| "127.0.0.1:7878".parse().expect("static address"));
    if !listen.ip().is_loopback() {
        return Err(ConfigError::NonLoopback(listen));
    }
    let branch_prefix = raw.branch_prefix.unwrap_or_else(|| "fleet".to_string());
    if !is_safe_ref_prefix(&branch_prefix) {
        return Err(ConfigError::BadBranchPrefix(branch_prefix));
    }
    let claude_command = raw.claude_command.unwrap_or_else(|| vec!["claude".to_string()]);
    if claude_command.first().is_none_or(|p| p.trim().is_empty()) {
        return Err(ConfigError::EmptyClaudeCommand);
    }
    let state_dir = match raw.state_dir {
        Some(s) => expand(&s, home),
        None => home.join(".local/share/fleet-manager"),
    };

    let projects_root = match raw.projects_root {
        Some(s) => expand(&s, home),
        None => home.join("fleet-projects"),
    };

    let mut repos: Vec<Repo> = Vec::new();
    for r in raw.repos {
        if !is_safe_dir_name(&r.name) {
            return Err(ConfigError::BadRepoName(r.name));
        }
        if repos.iter().any(|x| x.name == r.name) {
            return Err(ConfigError::DuplicateRepo(r.name));
        }
        let path = expand(&r.path, home);
        // Only a seed for the remembered list: a clone deleted since must not keep the daemon from starting.
        if !path.is_dir() {
            eprintln!("fleet-daemon: warning: [[repos]] {:?}: {} is not a directory; skipped", r.name, path.display());
            continue;
        }
        repos.push(Repo { name: r.name, path });
    }
    Ok(Config { listen, state_dir, branch_prefix, claude_command, projects_root, seed_repos: repos, telemetry: raw.telemetry })
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
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Conservative subset of `git check-ref-format` rules, so the prefix is safe in a branch name and a path.
fn is_safe_ref_prefix(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(['-', '/', '.'])
        && !s.ends_with(['/', '.'])
        && !s.contains("..")
        && !s.contains("//")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}
