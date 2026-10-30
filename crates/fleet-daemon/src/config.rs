//! The daemon's config file (`config.toml` in `fleet_fs::dirs::config_dir`): where the daemon listens and where tasks live. The file is optional.
use fleet_api::{Effort, PermissionMode};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub listen: SocketAddr,
    pub state_dir: PathBuf,
    pub branch_prefix: String,
    /// Program and flags used to start Claude Code. The daemon adds `--settings`, `--permission-mode`, `--effort` and
    /// the prompt, so the command itself may set neither of the last two.
    pub claude_command: Vec<String>,
    /// The permission mode and effort an agent runs with unless its definition, the product or its step says otherwise.
    pub default_mode: PermissionMode,
    pub default_effort: Effort,
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
    /// Fleet-wide caps the orchestrator tools check before they act.
    pub limits: LimitValues,
    /// Most products that may have orchestrated tasks working at once; None is no cap. Global only: no product
    /// resolves it.
    pub max_active_products: Option<u32>,
    /// How the orchestrator's pull requests are opened unless a product or the repo says otherwise.
    pub pr: PrSettings,
    pub telemetry: Telemetry,
    /// How many v0.1 `[[repos]]` entries the file still has. They are no longer read (repos belong to products), but
    /// a file with them still loads; loading runs before the logger exists, so the caller says so.
    pub legacy_repos: usize,
}

/// Top-level `max_*` keys. Read once at startup, so a change applies after the daemon restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitValues {
    /// Most breakdown versions one task may accumulate before a new breakdown is refused.
    pub max_breakdown_versions: u32,
    /// Most steps one breakdown may declare.
    pub max_steps: u32,
    /// Most specialists one orchestrator may have running at once.
    pub max_parallel_specialists: u32,
    /// Most times one task may try to open its pull request before `open_pr` refuses.
    pub max_pr_attempts: u32,
    /// Most orchestrated tasks one product may have working at once; None is no cap.
    pub max_concurrent_tasks: Option<u32>,
}

/// `pr_*` keys: the global defaults for a pull request's style, attribution line, draft state and labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrSettings {
    /// Name of a `pr_style` definition.
    pub style: String,
    /// One line added to the body; empty adds nothing.
    pub attribution: String,
    pub draft: bool,
    pub labels: Vec<String>,
    /// Where the gate's repo env builds.
    pub gate_build: GateBuild,
}

impl Default for PrSettings {
    fn default() -> Self {
        Self {
            style: "plain".into(),
            attribution: String::new(),
            draft: false,
            labels: Vec::new(),
            gate_build: GateBuild::default(),
        }
    }
}

/// `pr_gate_build`: whether the gate builds in the task's own folder, where the agents built, or in a fresh one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GateBuild {
    /// `{task}/` paths of the repo env resolve as they do for the agents, so the gate reuses their build.
    #[default]
    Shared,
    /// `{task}/` paths resolve under `<task folder>/gate-build`, emptied before and after the run.
    Clean,
}

impl std::str::FromStr for GateBuild {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "shared" => Ok(Self::Shared),
            "clean" => Ok(Self::Clean),
            _ => Err("must be shared or clean".into()),
        }
    }
}

impl Default for LimitValues {
    fn default() -> Self {
        Self {
            max_breakdown_versions: 8,
            max_steps: 20,
            max_parallel_specialists: 3,
            max_pr_attempts: 3,
            max_concurrent_tasks: None,
        }
    }
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
    #[error("claude_command must name a program")]
    EmptyClaudeCommand,
    #[error("claude_command must not set {0}; use default_mode / default_effort")]
    ClaudeFlag(&'static str),
    #[error("{0}: {1}")]
    BadValue(&'static str, String),
    #[error("gh_command must name a program")]
    EmptyGhCommand,
    #[error("test_timeout_secs must be more than 0")]
    ZeroTestTimeout,
    #[error("{}", zero_limit(.0))]
    ZeroLimit(&'static str),
}

/// A cap that is off unless set says how to turn it off, since 0 does not.
fn zero_limit(key: &str) -> String {
    match key {
        "max_concurrent_tasks" | "max_active_products" => {
            format!("{key} must be more than 0; leave it out for no cap")
        }
        _ => format!("{key} must be more than 0"),
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Raw {
    listen: Option<SocketAddr>,
    state_dir: Option<String>,
    branch_prefix: Option<String>,
    claude_command: Option<Vec<String>>,
    default_mode: Option<String>,
    default_effort: Option<String>,
    tasks_root: Option<String>,
    /// Only to refuse it with a pointer to the new name.
    projects_root: Option<String>,
    personas_dir: Option<String>,
    gh_command: Option<String>,
    test_timeout_secs: Option<u64>,
    max_breakdown_versions: Option<u32>,
    max_steps: Option<u32>,
    max_parallel_specialists: Option<u32>,
    max_pr_attempts: Option<u32>,
    max_concurrent_tasks: Option<u32>,
    max_active_products: Option<u32>,
    pr_style: Option<String>,
    pr_attribution: Option<String>,
    pr_draft: Option<bool>,
    pr_labels: Option<Vec<String>>,
    pr_gate_build: Option<String>,
    /// Only counted, see `Config::legacy_repos`.
    #[serde(default)]
    repos: Vec<toml::Value>,
    #[serde(default)]
    telemetry: Telemetry,
}

pub fn default_path(home: &Path) -> PathBuf {
    fleet_fs::dirs::config_dir(home).join("config.toml")
}

pub fn home_dir() -> PathBuf {
    fleet_fs::dirs::home_dir()
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
    for flag in ["--permission-mode", "--effort"] {
        let sets_it = claude_command.iter().any(|a| {
            a == flag
                || a.strip_prefix(flag)
                    .is_some_and(|rest| rest.starts_with('='))
        });
        if sets_it {
            return Err(ConfigError::ClaudeFlag(flag));
        }
    }
    let default_mode = match raw.default_mode {
        Some(m) => m.parse().map_err(|e: fleet_api::ModeError| {
            ConfigError::BadValue("default_mode", e.to_string())
        })?,
        None => PermissionMode::Auto,
    };
    let default_effort = match raw.default_effort {
        Some(m) => m.parse().map_err(|e: fleet_api::EffortError| {
            ConfigError::BadValue("default_effort", e.to_string())
        })?,
        None => Effort::Medium,
    };
    let state_dir = match raw.state_dir {
        Some(s) => expand(&s, home),
        None => fleet_fs::dirs::data_dir(home),
    };

    let tasks_root = match raw.tasks_root {
        Some(s) => expand(&s, home),
        None => home.join("fleet-tasks"),
    };

    let personas_dir = match raw.personas_dir {
        Some(s) => expand(&s, home),
        None => fleet_fs::dirs::data_dir(home).join("personas"),
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
    let defaults = LimitValues::default();
    let limits = LimitValues {
        max_breakdown_versions: positive_limit(
            "max_breakdown_versions",
            raw.max_breakdown_versions,
            defaults.max_breakdown_versions,
        )?,
        max_steps: positive_limit("max_steps", raw.max_steps, defaults.max_steps)?,
        max_parallel_specialists: positive_limit(
            "max_parallel_specialists",
            raw.max_parallel_specialists,
            defaults.max_parallel_specialists,
        )?,
        max_pr_attempts: positive_limit(
            "max_pr_attempts",
            raw.max_pr_attempts,
            defaults.max_pr_attempts,
        )?,
        max_concurrent_tasks: optional_limit("max_concurrent_tasks", raw.max_concurrent_tasks)?,
    };
    let max_active_products = optional_limit("max_active_products", raw.max_active_products)?;
    let pr_defaults = PrSettings::default();
    let pr = PrSettings {
        style: raw.pr_style.unwrap_or(pr_defaults.style),
        attribution: raw.pr_attribution.unwrap_or(pr_defaults.attribution),
        draft: raw.pr_draft.unwrap_or(pr_defaults.draft),
        labels: raw.pr_labels.unwrap_or(pr_defaults.labels),
        gate_build: match raw.pr_gate_build {
            Some(v) => v
                .parse()
                .map_err(|e: String| ConfigError::BadValue("pr_gate_build", e))?,
            None => pr_defaults.gate_build,
        },
    };

    Ok(Config {
        listen,
        state_dir,
        branch_prefix,
        claude_command,
        default_mode,
        default_effort,
        tasks_root,
        personas_dir,
        gh_command,
        agent_config_dir,
        fleet_dir,
        test_timeout_secs,
        limits,
        max_active_products,
        pr,
        telemetry: raw.telemetry,
        legacy_repos: raw.repos.len(),
    })
}

/// A limit of 0 would refuse every breakdown or spawn, which is a misconfiguration, not a way to turn a cap off.
fn positive_limit(key: &'static str, value: Option<u32>, default: u32) -> Result<u32, ConfigError> {
    match value.unwrap_or(default) {
        0 => Err(ConfigError::ZeroLimit(key)),
        n => Ok(n),
    }
}

/// `positive_limit` for a cap that is off unless set: 0 is still a misconfiguration, not a way to say "no cap".
fn optional_limit(key: &'static str, value: Option<u32>) -> Result<Option<u32>, ConfigError> {
    match value {
        Some(0) => Err(ConfigError::ZeroLimit(key)),
        n => Ok(n),
    }
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
