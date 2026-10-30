use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// How a PR's version note is counted.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS,
)]
#[ts(export)]
pub enum Versioning {
    /// breaking → major, feature → minor, fix → patch.
    #[serde(rename = "semver")]
    Semver,
    /// Before 1.0: breaking → minor, feature and fix → patch.
    #[serde(rename = "semver-pre1.0")]
    SemverPre1,
    /// No version note.
    #[default]
    #[serde(rename = "none")]
    None,
}

/// What a pull request an orchestrator opens must look like: a built-in style or the repo's own, resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct PrRules {
    /// The style's name.
    #[serde(default)]
    pub style: String,
    /// Regex the PR title must match; None accepts any title.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub title: Option<String>,
    /// Regex every commit subject on the task branch must match; None accepts any subject. Absent from rules frozen
    /// before commits were checked.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub commits: Option<String>,
    /// Headings the PR body must have, in order.
    #[serde(default)]
    pub sections: Vec<String>,
    #[serde(default)]
    pub versioning: Versioning,
    /// Free-form guidance shown to the orchestrator; not checked.
    #[serde(default)]
    pub prose: String,
    /// The file release-note lines go to; None means the PR carries none.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub release_notes: Option<String>,
    /// Markers that make a change breaking.
    #[serde(default)]
    pub breaking: Vec<String>,
    /// Trailer lines the PR body ends with.
    #[serde(default)]
    pub attribution: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub labels: Vec<String>,
}
