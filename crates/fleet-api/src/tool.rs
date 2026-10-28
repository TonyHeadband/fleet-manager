//! The tools the app needs on the daemon's machine, and the one error family for their problems.
use serde::{Deserialize, Serialize};
use std::fmt;
use ts_rs::TS;
use utoipa::ToSchema;

/// A tool the daemon checks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ToolId {
    Git,
    GitBash,
    Claude,
    Gh,
}

impl ToolId {
    /// The name shown to people.
    pub fn name(self) -> &'static str {
        match self {
            ToolId::Git => "Git",
            ToolId::GitBash => "Git Bash",
            ToolId::Claude => "Claude Code",
            ToolId::Gh => "GitHub CLI (gh)",
        }
    }
}

/// How to fix a missing or broken tool on the daemon's machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ToolFix {
    /// The exact command to run on the daemon's machine; none when no package manager has one.
    pub install: Option<String>,
    /// Where to download or read about it.
    pub link: String,
    /// The package manager `install` uses, e.g. `winget`.
    pub manager: Option<String>,
}

/// Why a tool cannot be used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[ts(export)]
pub enum ToolError {
    NotInstalled {
        tool: ToolId,
        fix: ToolFix,
    },
    TooOld {
        tool: ToolId,
        found: String,
        needed: String,
        fix: ToolFix,
    },
    NotLoggedIn {
        tool: ToolId,
        persona: Option<String>,
    },
    Broken {
        tool: ToolId,
        detail: String,
        fix: ToolFix,
    },
}

impl ToolError {
    pub fn tool(&self) -> ToolId {
        match self {
            ToolError::NotInstalled { tool, .. }
            | ToolError::TooOld { tool, .. }
            | ToolError::NotLoggedIn { tool, .. }
            | ToolError::Broken { tool, .. } => *tool,
        }
    }

    /// The fix, when the problem has one (logging in is not an install).
    pub fn fix(&self) -> Option<&ToolFix> {
        match self {
            ToolError::NotInstalled { fix, .. }
            | ToolError::TooOld { fix, .. }
            | ToolError::Broken { fix, .. } => Some(fix),
            ToolError::NotLoggedIn { .. } => None,
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.tool().name();
        match self {
            ToolError::NotInstalled { .. } => write!(f, "{name} is not installed"),
            ToolError::TooOld { found, needed, .. } => {
                write!(f, "{name} {found} is too old: {needed} or newer is needed")
            }
            ToolError::NotLoggedIn { persona: None, .. } => write!(f, "{name} is not logged in"),
            ToolError::NotLoggedIn {
                persona: Some(p), ..
            } => write!(f, "{name} is not logged in for {p}"),
            ToolError::Broken { detail, .. } => {
                write!(f, "{name} is installed but does not work: {detail}")
            }
        }
    }
}

impl std::error::Error for ToolError {}

/// One tool's state on the daemon's machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ToolStatus {
    pub tool: ToolId,
    pub required: bool,
    pub version: Option<String>,
    pub error: Option<ToolError>,
}

/// Body of `GET /api/tools`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ToolList {
    pub tools: Vec<ToolStatus>,
    /// Unix seconds of the check these results come from.
    pub checked_at: i64,
}

impl ToolList {
    /// Required tools that have a problem.
    pub fn missing_required(&self) -> impl Iterator<Item = &ToolStatus> {
        self.tools
            .iter()
            .filter(|t| t.required && t.error.is_some())
    }
}

/// Body of `GET /api/version`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct VersionInfo {
    pub version: String,
}

/// The daemon's operating system; picks the install command table and the client's install argv.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Os {
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}
