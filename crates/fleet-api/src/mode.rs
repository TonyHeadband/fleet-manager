use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use ts_rs::TS;
use utoipa::ToSchema;

/// The `claude --permission-mode` values an agent may run in. `plan` and `bypassPermissions` are not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub enum PermissionMode {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "dontAsk")]
    DontAsk,
}

impl PermissionMode {
    pub fn as_flag(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::AcceptEdits => "acceptEdits",
            Self::Manual => "manual",
            Self::DontAsk => "dontAsk",
        }
    }

    /// Position in the loosest-to-strictest order a step may move along; `dontAsk` is outside it.
    pub fn rank(&self) -> Option<u8> {
        match self {
            Self::Auto => Some(0),
            Self::AcceptEdits => Some(1),
            Self::Manual => Some(2),
            Self::DontAsk => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeError(pub String);

impl fmt::Display for ModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "mode {} is not allowed (auto, acceptEdits, manual, dontAsk)",
            self.0
        )
    }
}

impl std::error::Error for ModeError {}

impl FromStr for PermissionMode {
    type Err = ModeError;

    fn from_str(s: &str) -> Result<Self, ModeError> {
        match s {
            "auto" => Ok(Self::Auto),
            "acceptEdits" => Ok(Self::AcceptEdits),
            "manual" => Ok(Self::Manual),
            "dontAsk" => Ok(Self::DontAsk),
            other => Err(ModeError(other.to_string())),
        }
    }
}

/// The `claude --effort` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_flag(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffortError(pub String);

impl fmt::Display for EffortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "effort {} is not allowed (low, medium, high, xhigh, max)",
            self.0
        )
    }
}

impl std::error::Error for EffortError {}

impl FromStr for Effort {
    type Err = EffortError;

    fn from_str(s: &str) -> Result<Self, EffortError> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::Xhigh),
            "max" => Ok(Self::Max),
            other => Err(EffortError(other.to_string())),
        }
    }
}
