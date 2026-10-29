use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;
use utoipa::ToSchema;

/// How a persona's agents authenticate to GitHub: the `gh` CLI's own login, or a personal access token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GithubAuth {
    Gh,
    Pat,
}

/// The result of checking one login.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct LoginState {
    pub ok: bool,
    pub account: Option<String>,
    /// One line: why the login is not ok.
    pub detail: Option<String>,
    /// The tool problem behind it, when there is one: the CLI is missing or will not start (logging in cannot help;
    /// its fix can), or it says the persona is logged out. None when the check failed some other way, and from an
    /// older daemon.
    #[serde(default)]
    pub tool: Option<crate::ToolError>,
}

/// A Claude and GitHub identity that tasks run under. `claude` and `github` are None until checked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub host: String,
    pub commit_name: String,
    pub commit_email: String,
    pub github_auth: GithubAuth,
    pub claude_dir: String,
    pub gh_dir: String,
    pub ready: bool,
    pub claude: Option<LoginState>,
    pub github: Option<LoginState>,
    pub tasks: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct PersonaList {
    pub personas: Vec<Persona>,
    pub claude_command: Vec<String>,
    pub gh_command: String,
    pub tasks_root: String,
    /// Where `claude_command[0]` is on the daemon's PATH, absolute; None when it is not found there (or from an
    /// older daemon). A login terminal runs this, so it needs no PATH of its own.
    #[serde(default)]
    pub claude_path: Option<String>,
    /// Where `gh_command` is on the daemon's PATH, absolute; None as for `claude_path`.
    #[serde(default)]
    pub gh_path: Option<String>,
}

impl PersonaList {
    /// The Claude command a login runs: `claude_command` with its program where the daemon found it.
    pub fn claude_login_command(&self) -> Vec<String> {
        let mut argv = self.claude_command.clone();
        if let (Some(first), Some(path)) = (argv.first_mut(), &self.claude_path) {
            *first = path.clone();
        }
        argv
    }

    /// The gh a login runs: where the daemon found it, else `gh_command` as configured.
    pub fn gh_login_program(&self) -> &str {
        self.gh_path.as_deref().unwrap_or(&self.gh_command)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct NewPersona {
    pub name: String,
    pub host: String,
    pub commit_name: String,
    pub commit_email: String,
    pub github_auth: GithubAuth,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct PersonaUpdate {
    pub name: Option<String>,
    pub commit_name: Option<String>,
    pub commit_email: Option<String>,
    pub github_auth: Option<GithubAuth>,
}

/// The permissions and environment written into an agent's Claude settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AgentSettings {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// What every agent gets: the instructions text and the settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AgentConfig {
    pub instructions: String,
    pub settings: AgentSettings,
    /// Set by GET when settings.json is unreadable or invalid (then `settings` is the default); ignored on PUT.
    #[serde(default)]
    pub settings_error: Option<String>,
}

/// A save of the agent config: only the parts given are written, so one tab never rewrites the other's file.
/// At least one must be set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AgentConfigUpdate {
    pub instructions: Option<String>,
    pub settings: Option<AgentSettings>,
}

/// Variables that would log a CLI in without the persona's own folder, so an ambient one must never make a
/// persona look ready, nor reach an agent or the daemon's own git.
pub const AMBIENT_TOKENS: [&str; 12] = [
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];

/// Variables through which an ambient git setup could still answer for a persona: `GIT_CONFIG_PARAMETERS` adds
/// config (a credential helper) after the persona's own, and `SSH_ASKPASS` would pop the desktop's password prompt
/// once the persona's helper has nothing. `SSH_AUTH_SOCK` would hand an agent the user's own ssh keys. Stripped
/// from the daemon's git and from every agent, with the tokens.
pub const AMBIENT_GIT: [&str; 3] = ["GIT_CONFIG_PARAMETERS", "SSH_ASKPASS", "SSH_AUTH_SOCK"];

/// Everything stripped from the daemon's git, from an agent's process and from a login: the one list all use.
pub fn ambient_vars() -> impl Iterator<Item = &'static str> {
    AMBIENT_TOKENS.into_iter().chain(AMBIENT_GIT)
}
