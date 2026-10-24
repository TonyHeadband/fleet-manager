//! `~/.config/fleet-board/config.toml`: which repos the board manages and where the daemon listens.
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
    pub repos: Vec<Repo>,
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
    #[error("repo path {0} is not a directory")]
    NotADirectory(PathBuf),
    #[error("branch_prefix {0:?} is not a safe git ref component")]
    BadBranchPrefix(String),
    #[error("claude_command must name a program")]
    EmptyClaudeCommand,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    listen: Option<SocketAddr>,
    state_dir: Option<String>,
    branch_prefix: Option<String>,
    claude_command: Option<Vec<String>>,
    #[serde(default)]
    repos: Vec<RawRepo>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepo {
    name: String,
    path: String,
}

pub fn default_path(home: &Path) -> PathBuf {
    home.join(".config/fleet-board/config.toml")
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    load_with_home(path, &home_dir())
}

pub fn load_with_home(path: &Path, home: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
    let raw: Raw = toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?;

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
        None => home.join(".local/share/fleet-board"),
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
        if !path.is_dir() {
            return Err(ConfigError::NotADirectory(path));
        }
        repos.push(Repo { name: r.name, path });
    }
    Ok(Config { listen, state_dir, branch_prefix, claude_command, repos })
}

fn expand(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

fn is_safe_dir_name(s: &str) -> bool {
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
